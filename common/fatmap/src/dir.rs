//! Directory listing with long names, look-up by 8.3 name, and a file
//! reader with seek -- the parts of `embedded_sdmmc` the firmware uses.
//!
//! [`walk`] hands every entry of a directory to a callback together with
//! its long name, assembled as UTF-8 into a buffer the caller lends. The
//! callback says when to stop, so a look-up is a walk that stops at the
//! match ([`find`]) and there is one copy of the directory loop. The walk
//! and the callback are not generic (`dyn`), so every caller shares that one
//! copy in flash.
//!
//! # What a listing contains
//!
//! Files, directories and `.` / `..`, in directory order. Not: deleted
//! entries, long-name pieces, and the volume label. Names are the raw 11
//! bytes (the 8.3 case flags in byte 12 are ignored, so names stay upper
//! case, and a leading 0x05 stays 0x05), as `embedded_sdmmc` gives them.
//!
//! # Long names
//!
//! A run of pieces is used only if its ordinals count down to 1 without a
//! gap, every piece is well formed (type 0, cluster 0, ordinal 1..=20) and
//! carries the checksum of the 8.3 entry that follows. A deleted entry or a
//! malformed piece breaks the run. This is what [`crate::lfn::LfnMatcher`]
//! and Windows do; `embedded_sdmmc` differs in corner cases (and can panic on lone surrogates), see
//! `tests/reader.rs` (`differs_*`). Each piece is cut at its first 0x0000;
//! surrogate pairs (also split over two pieces) become one character; a
//! lone surrogate becomes U+FFFD. If the name does not fit the buffer, the
//! entry is reported without a long name.

use crate::vol::{rd16, rd32, Block, BlockRead, Error, Volume};

/// Code-unit offsets of the 13 characters inside an LFN piece.
const CHAR_OFFSETS: [u8; 13] = [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];

/// One directory entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirEntry {
    /// The on-disk 8.3 name (the alias, when there is a long name).
    pub name: [u8; 11],
    pub attr: u8,
    /// 0 for an empty file, and in `..` for the root.
    pub first_cluster: u32,
    pub size: u32,
}

impl DirEntry {
    pub fn is_dir(&self) -> bool {
        self.attr & 0x10 != 0
    }
}

/// Long-name assembly state. The name is written backwards from the end of
/// the caller's buffer (pieces come last first), so it needs no UTF-16
/// buffer of its own.
struct Lfn {
    /// Ordinal of the piece accepted last; 0: no run.
    expect: u8,
    csum: u8,
    /// The name starts here in the buffer.
    free: usize,
    /// No overflow so far.
    fits: bool,
    /// A low surrogate waiting for its high half (0: none).
    low: u16,
}

impl Lfn {
    fn emit(&mut self, out: &mut [u8], cp: u32) {
        let mut tmp = [0u8; 4];
        let n = if cp < 0x80 {
            tmp[0] = cp as u8;
            1
        } else if cp < 0x800 {
            tmp[0] = 0xC0 | (cp >> 6) as u8;
            tmp[1] = 0x80 | (cp & 0x3F) as u8;
            2
        } else if cp < 0x1_0000 {
            tmp[0] = 0xE0 | (cp >> 12) as u8;
            tmp[1] = 0x80 | ((cp >> 6) & 0x3F) as u8;
            tmp[2] = 0x80 | (cp & 0x3F) as u8;
            3
        } else {
            tmp[0] = 0xF0 | (cp >> 18) as u8;
            tmp[1] = 0x80 | ((cp >> 12) & 0x3F) as u8;
            tmp[2] = 0x80 | ((cp >> 6) & 0x3F) as u8;
            tmp[3] = 0x80 | (cp & 0x3F) as u8;
            4
        };
        if !self.fits || self.free < n {
            self.fits = false;
            return;
        }
        self.free -= n;
        out[self.free..self.free + n].copy_from_slice(&tmp[..n]);
    }

    /// Take one piece (`e[11]` says it is one). `out` is the name buffer.
    fn piece(&mut self, e: &[u8], out: &mut [u8]) {
        let ord = e[0];
        let seq = ord & 0x3F;
        if e[12] != 0 || e[26] != 0 || e[27] != 0 || seq == 0 || seq > 20 {
            self.expect = 0;
            return;
        }
        if ord & 0x40 != 0 {
            self.csum = e[13];
            self.free = out.len();
            self.fits = true;
            self.low = 0;
        } else if self.expect == 0 || seq != self.expect - 1 || e[13] != self.csum {
            self.expect = 0;
            return;
        }
        self.expect = seq;
        let mut units = [0u16; 13];
        let mut n = 0;
        for &off in CHAR_OFFSETS.iter() {
            let u = rd16(e, off as usize) as u16;
            if u == 0 {
                break;
            }
            units[n] = u;
            n += 1;
        }
        for &u in units[..n].iter().rev() {
            if self.low != 0 {
                let low = core::mem::take(&mut self.low);
                if (0xD800..0xDC00).contains(&u) {
                    let cp = 0x1_0000 + (((u as u32) & 0x3FF) << 10) + ((low as u32) & 0x3FF);
                    self.emit(out, cp);
                    continue;
                }
                self.emit(out, 0xFFFD);
            }
            if (0xDC00..0xE000).contains(&u) {
                self.low = u;
            } else if (0xD800..0xDC00).contains(&u) {
                self.emit(out, 0xFFFD);
            } else {
                self.emit(out, u as u32);
            }
        }
    }
}

fn checksum(name: &[u8]) -> u8 {
    name[..11].iter().fold(0u8, |s, &b| s.rotate_right(1).wrapping_add(b))
}

/// Call `f` with every entry of the directory starting at `dir` (a
/// directory's first cluster; 0 or [`Volume::root`] for the root) and its
/// long name, if any, as UTF-8 in `lfn`. Pass an empty `lfn` to skip long
/// names (255 UTF-16 units need at most 765 bytes). `f` returns `true` to
/// stop; the entry it stopped at is returned. `Ok(None)`: the directory
/// ended.
pub fn walk(
    vol: &Volume,
    dev: &mut dyn BlockRead,
    blk: &mut Block,
    dir: u32,
    lfn: &mut [u8],
    f: &mut dyn FnMut(&DirEntry, Option<&[u8]>) -> bool,
) -> Result<Option<DirEntry>, Error> {
    let mut st = Lfn { expect: 0, csum: 0, free: 0, fits: false, low: 0 };
    let fixed = dir == 0 && vol.is_fat16();
    let mut cluster = if dir == 0 { vol.root() } else { dir };
    let mut hops = 0u32;
    loop {
        let (lba, blocks) = if fixed {
            vol.fixed_root()
        } else {
            // A directory has at most `clusters` clusters; more is a loop.
            hops += 1;
            if hops > vol.clusters() {
                return Err(Error::BadChain);
            }
            (vol.cluster_lba(cluster).ok_or(Error::BadChain)?, vol.blocks_per_cluster())
        };
        for b in 0..blocks {
            let data = blk.load(dev, lba + b)?;
            for e in data.as_chunks::<32>().0 {
                let attr = e[11];
                if e[0] == 0 {
                    return Ok(None);
                }
                if e[0] == 0xE5 {
                    st.expect = 0;
                    continue;
                }
                if attr & 0x0F == 0x0F {
                    st.piece(e, lfn);
                    continue;
                }
                let long = st.expect == 1 && st.fits && st.csum == checksum(e);
                st.expect = 0;
                if attr & 0x08 != 0 {
                    continue; // volume label
                }
                let mut name = [0u8; 11];
                name.copy_from_slice(&e[..11]);
                let ent = DirEntry { name, attr, first_cluster: (rd16(e, 0x14) << 16) | rd16(e, 0x1A), size: rd32(e, 0x1C) };
                if f(&ent, if long { Some(&lfn[st.free..]) } else { None }) {
                    return Ok(Some(ent));
                }
            }
        }
        if fixed {
            return Ok(None);
        }
        match vol.next_cluster(dev, blk, cluster)? {
            Some(c) => cluster = c,
            None => return Ok(None),
        }
    }
}

/// The entry named exactly `name` (all 11 bytes, as `embedded_sdmmc`
/// compares them) in the directory at `dir`.
pub fn find(vol: &Volume, dev: &mut dyn BlockRead, blk: &mut Block, dir: u32, name: &[u8; 11]) -> Result<DirEntry, Error> {
    walk(vol, dev, blk, dir, &mut [], &mut |e, _| e.name == *name)?.ok_or(Error::NotFound)
}

/// A file being read: position and a cursor into its cluster chain.
/// Reading forward follows the chain; a seek backwards restarts it from the
/// first cluster on the next read.
#[derive(Clone, Copy, Debug)]
pub struct File {
    first: u32,
    size: u32,
    pos: u32,
    /// Cluster holding cluster-index `index` of the file.
    cluster: u32,
    index: u32,
}

impl File {
    pub fn open(e: &DirEntry) -> File {
        File { first: e.first_cluster, size: e.size, pos: 0, cluster: e.first_cluster, index: 0 }
    }

    pub fn len(&self) -> u32 {
        self.size
    }

    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    pub fn pos(&self) -> u32 {
        self.pos
    }

    /// Move to `off`; up to the length is allowed, beyond is
    /// [`Error::Offset`] (as `embedded_sdmmc`'s `seek_from_start`).
    pub fn seek(&mut self, off: u32) -> Result<(), Error> {
        if off > self.size {
            return Err(Error::Offset);
        }
        self.pos = off;
        Ok(())
    }

    /// Read up to `out.len()` bytes; fewer only at the end of the file.
    pub fn read(&mut self, vol: &Volume, dev: &mut dyn BlockRead, blk: &mut Block, out: &mut [u8]) -> Result<usize, Error> {
        let cb = vol.blocks_per_cluster() * 512;
        let mut n = 0;
        while n < out.len() && self.pos < self.size {
            let idx = self.pos / cb;
            if idx < self.index {
                self.cluster = self.first;
                self.index = 0;
            }
            while self.index < idx {
                self.cluster = vol.next_cluster(dev, blk, self.cluster)?.ok_or(Error::BadChain)?;
                self.index += 1;
            }
            let in_cluster = self.pos % cb;
            let lba = vol.cluster_lba(self.cluster).ok_or(Error::BadChain)? + in_cluster / 512;
            let data = blk.load(dev, lba)?;
            let off = (in_cluster % 512) as usize;
            let take = (512 - off).min(out.len() - n).min((self.size - self.pos) as usize);
            out[n..n + take].copy_from_slice(&data[off..off + take]);
            n += take;
            self.pos += take as u32;
        }
        Ok(n)
    }
}
