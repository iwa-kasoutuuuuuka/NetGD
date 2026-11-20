//! Cluster chains to block runs.
//!
//! A file on FAT32 is a linked list of clusters. The firmware wants it as a
//! short list of contiguous runs of 512-byte blocks (`Extent`), so that a
//! file offset becomes an SD block with integer arithmetic and a multi-block
//! read can cover a whole run.
//!
//! [`walk`] turns a chain into runs, calling back for each; the caller
//! decides where they go. Two users:
//!
//! * the **eager** map: every run of a track file into a shared pool at image
//!   load, found again by [`locate`];
//! * the **window** ([`Window`]): when the pool is full, or a file is too
//!   fragmented to be worth a permanent map, a small cache of the runs
//!   around the offset being read, refilled by walking the chain on demand.
//!   That replaces the filesystem fallback, which re-opened the file and
//!   walked the chain from its start for every 2 KB chunk.
//!
//! The FAT lookup is passed in as a closure so this crate never touches the
//! card.

/// A contiguous run of blocks on the card.
///
/// No file offset is stored: runs of one file are laid end to end, so the
/// offset of a run is the sum of the lengths before it. That makes an entry 8
/// bytes instead of 12, which matters on a part with 16 KB of RAM.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Extent {
    /// First SD block of the run.
    pub lba: u32,
    /// Length in 512-byte blocks.
    pub blocks: u32,
}

impl Extent {
    pub const EMPTY: Extent = Extent { lba: 0, blocks: 0 };

    fn bytes(&self) -> u64 {
        self.blocks as u64 * 512
    }
}

/// What the cluster arithmetic needs to know about the volume.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClusterGeom {
    /// First block of cluster 2.
    pub data_lba: u32,
    pub blocks_per_cluster: u32,
}

impl ClusterGeom {
    /// First block of `cluster`; `None` for a cluster number below 2 or one
    /// whose block number does not fit a `u32` (a damaged FAT value).
    pub fn checked_cluster_lba(&self, cluster: u32) -> Option<u32> {
        self.data_lba
            .checked_add(cluster.checked_sub(2)?.checked_mul(self.blocks_per_cluster)?)
    }

    /// [`checked_cluster_lba`](Self::checked_cluster_lba) for a cluster the
    /// caller knows is valid (tests); wraps rather than panics otherwise.
    pub fn cluster_lba(&self, cluster: u32) -> u32 {
        self.data_lba
            .wrapping_add(cluster.wrapping_sub(2).wrapping_mul(self.blocks_per_cluster))
    }
}

/// Where a walk starts (or resumes): the first cluster of a run, the file
/// offset that cluster begins at, and how many blocks of the file remain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Cursor {
    pub cluster: u32,
    pub file_off: u32,
    pub blocks_left: u32,
}

impl Cursor {
    /// The start of a file of `size` bytes whose first cluster is
    /// `first_cluster`. `None` for an empty file or a cluster number below 2
    /// (an empty file's 0, or a `..` entry's).
    pub fn start(first_cluster: u32, size: u32) -> Option<Cursor> {
        if first_cluster < 2 || size == 0 {
            return None;
        }
        Some(Cursor { cluster: first_cluster, file_off: 0, blocks_left: blocks_of(size) })
    }
}

/// Blocks a file of `size` bytes occupies (the last may be partial).
pub fn blocks_of(size: u32) -> u32 {
    ((size as u64 + 511) / 512) as u32
}

/// How a walk ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Walk {
    /// The whole remaining file was emitted.
    Done,
    /// `emit` refused a run (its pool is full). Resume from the cursor, which
    /// is the start of the refused run: nothing has been lost, the runs
    /// before it were emitted and that one was not.
    More(Cursor),
    /// The chain is broken: ends before the file's size, leaves the FAT, or
    /// points at a reserved or bad cluster.
    Broken,
}

/// Follow the chain from `cur`, emitting each contiguous run as
/// `(file_off, extent)`.
///
/// `next(c)` is the FAT entry of cluster `c`: `Ok(Some(n))` for a link,
/// `Ok(None)` at end of chain, `Err` for a bad entry or a failed read.
/// `emit` returns `false` to stop (and have the run offered again on the next
/// walk).
///
/// The walk is bounded by the file's size, not by the chain: every cluster
/// visited uses up blocks, so a looping chain cannot make it run longer than
/// the file is.
pub fn walk(
    g: &ClusterGeom,
    cur: Cursor,
    mut next: impl FnMut(u32) -> Result<Option<u32>, ()>,
    mut emit: impl FnMut(u32, Extent) -> bool,
) -> Walk {
    walk_dyn(g, cur, &mut next, &mut emit)
}

/// [`walk`] without the generics, so the firmware, which walks from two
/// places, carries one copy of the loop.
#[inline(never)]
pub fn walk_dyn(
    g: &ClusterGeom,
    cur: Cursor,
    next: &mut dyn FnMut(u32) -> Result<Option<u32>, ()>,
    emit: &mut dyn FnMut(u32, Extent) -> bool,
) -> Walk {
    let mut cluster = cur.cluster;
    let mut left = cur.blocks_left;
    let mut off = cur.file_off;
    let cluster_bytes = g.blocks_per_cluster.wrapping_mul(512);
    if left == 0 || cluster < 2 {
        return Walk::Done;
    }
    loop {
        let run_cluster = cluster;
        let run_left = left;
        let run_off = off;
        let Some(run_lba) = g.checked_cluster_lba(cluster) else {
            return Walk::Broken;
        };
        let mut blocks = 0u32;
        let following = loop {
            let n = g.blocks_per_cluster.min(left);
            blocks += n;
            left -= n;
            off = off.wrapping_add(cluster_bytes);
            if left == 0 {
                break None;
            }
            match next(cluster) {
                Ok(Some(c)) if c >= 2 => {
                    if cluster.checked_add(1) == Some(c) {
                        cluster = c;
                    } else {
                        break Some(c);
                    }
                }
                _ => return Walk::Broken,
            }
        };
        // The run must end inside the 32-bit block space.
        if run_lba.checked_add(blocks).is_none() {
            return Walk::Broken;
        }
        if !emit(run_off, Extent { lba: run_lba, blocks }) {
            return Walk::More(Cursor {
                cluster: run_cluster,
                file_off: run_off,
                blocks_left: run_left,
            });
        }
        match following {
            None => return Walk::Done,
            Some(c) => cluster = c,
        }
    }
}

/// Resolve `byte_off` within runs laid end to end from file offset
/// `base_off`: `(sd_block, offset within that block, contiguous blocks
/// available from there)`.
pub fn locate(extents: &[Extent], base_off: u32, byte_off: u32) -> Option<(u32, u32, u32)> {
    let target = byte_off as u64;
    let mut off = base_off as u64;
    if target < off {
        return None;
    }
    for e in extents {
        let len = e.bytes();
        if target < off + len {
            let rel = ((target - off) / 512) as u32;
            return Some((e.lba + rel, byte_off % 512, e.blocks - rel));
        }
        off += len;
    }
    None
}

/// A cache of the `N` runs around the offset last read from one file.
///
/// Reading a file that is not in the permanent map goes through here. A read
/// inside the window costs nothing. A read just past it continues the chain
/// walk from where the window ended, so a file streamed from start to end is
/// walked once in total. A read before the window, or in another file, walks
/// again from the file's first cluster: the price of a backward seek in an
/// unmapped file is its chain, one FAT sector read per 128 clusters (about 4
/// MB at 32 KB clusters), which is what the old fallback paid for *every*
/// chunk.
pub struct Window<const N: usize> {
    exts: [Extent; N],
    n: usize,
    /// File offset of `exts[0]`.
    base_off: u32,
    /// Which file the window holds; meaningful only when `valid`.
    key: u8,
    valid: bool,
    /// Where the walk stopped, or `None` if it reached the file's end.
    resume: Option<Cursor>,
}

impl<const N: usize> Window<N> {
    pub const fn new() -> Self {
        Self {
            exts: [Extent::EMPTY; N],
            n: 0,
            base_off: 0,
            key: 0,
            valid: false,
            resume: None,
        }
    }

    /// Drop the cache (a different card or image).
    pub fn invalidate(&mut self) {
        self.valid = false;
        self.n = 0;
        self.resume = None;
    }

    fn end_off(&self) -> u64 {
        self.base_off as u64 + self.exts[..self.n].iter().map(Extent::bytes).sum::<u64>()
    }

    /// `(sd_block, offset in block, contiguous blocks)` for `byte_off` of the
    /// file `key`, whose chain starts at `first_cluster` and whose length is
    /// `size` bytes. `next` is the FAT lookup (see [`walk`]).
    ///
    /// `None` if `byte_off` is past the file or the chain is broken.
    pub fn lookup(
        &mut self,
        key: u8,
        g: &ClusterGeom,
        first_cluster: u32,
        size: u32,
        byte_off: u32,
        next: impl FnMut(u32) -> Result<Option<u32>, ()>,
    ) -> Option<(u32, u32, u32)> {
        if (byte_off as u64) >= blocks_of(size) as u64 * 512 {
            return None;
        }
        let mut from = None;
        if self.valid && self.key == key {
            if let Some(hit) = locate(&self.exts[..self.n], self.base_off, byte_off) {
                return Some(hit);
            }
            if byte_off as u64 >= self.end_off() {
                from = self.resume;
            }
        }
        let cur = match from {
            Some(c) => c,
            None => Cursor::start(first_cluster, size)?,
        };
        self.fill(key, g, cur, byte_off, next);
        if !self.valid {
            return None;
        }
        locate(&self.exts[..self.n], self.base_off, byte_off)
    }

    /// Replace the window with the runs from `cur` on, skipping those that
    /// end at or before `target`.
    fn fill(
        &mut self,
        key: u8,
        g: &ClusterGeom,
        cur: Cursor,
        target: u32,
        next: impl FnMut(u32) -> Result<Option<u32>, ()>,
    ) {
        self.n = 0;
        self.valid = false;
        self.key = key;
        self.resume = None;
        let (exts, n_slot, base) = (&mut self.exts, &mut self.n, &mut self.base_off);
        let r = walk(g, cur, next, |off, e| {
            if off as u64 + e.bytes() <= target as u64 {
                return true; // wholly before what was asked for
            }
            if *n_slot == N {
                return false;
            }
            if *n_slot == 0 {
                *base = off;
            }
            exts[*n_slot] = e;
            *n_slot += 1;
            true
        });
        match r {
            Walk::Done => self.valid = true,
            Walk::More(c) => {
                self.valid = true;
                self.resume = Some(c);
            }
            Walk::Broken => self.n = 0,
        }
    }
}
