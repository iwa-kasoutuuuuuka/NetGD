//! A read-only FAT32 volume (FAT16 with the `fat16` feature): finding it on
//! the card and following cluster chains.
//!
//! This is the base of the reader meant to replace `embedded_sdmmc` in the
//! firmware (`dir` for listings and look-ups, `dir::File` for reading). The
//! rules are those of the firmware's own `fat::probe`/`fat_next`, so that the
//! two FAT readers on the card agree:
//!
//! * the volume is at LBA 0 if LBA 0 is a BPB (a "superfloppy"), else at MBR
//!   entry 0, whatever its type byte says. (`embedded_sdmmc` wants entry 0 and
//!   a FAT type byte, so it refuses superfloppies; see `tests/oracle.rs`.)
//! * FAT32 is told by the BPB's shape (no fixed root, no 16-bit FAT size),
//!   not by the cluster count, so small FAT32 images are FAT32.
//! * only FAT 0 is read (BPB 0x28 is ignored), as both current readers do.
//! * a chain that leaves the volume, hits a free / reserved / bad entry, or
//!   (for directories) loops, is an error and never a block number.
//!
//! Every call takes the device and a [`Block`] buffer from the caller: no
//! statics, no allocation, one 512-byte buffer for everything.

/// One 512-byte block read, as the firmware's `hsmci::read_block` (CMD17).
#[allow(clippy::result_unit_err)]
pub trait BlockRead {
    fn read_block(&mut self, lba: u32, buf: &mut [u8; 512]) -> Result<(), ()>;
}

impl<F: FnMut(u32, &mut [u8; 512]) -> Result<(), ()>> BlockRead for F {
    fn read_block(&mut self, lba: u32, buf: &mut [u8; 512]) -> Result<(), ()> {
        self(lba, buf)
    }
}

/// Why a request failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The card read failed.
    Io,
    /// No FAT volume this reader handles.
    NoVolume,
    /// A damaged cluster chain or a file shorter than its size.
    BadChain,
    /// No such entry.
    NotFound,
    /// A seek past the end of the file.
    Offset,
}

/// A block buffer that remembers which block it holds, so a run of small
/// reads from one block reads the card once. Call [`Block::invalidate`]
/// after using `data` for anything else or after the card was written.
pub struct Block {
    pub data: [u8; 512],
    lba: u32,
}

impl Default for Block {
    fn default() -> Self {
        Self::new()
    }
}

impl Block {
    pub const fn new() -> Self {
        Block { data: [0; 512], lba: u32::MAX }
    }

    pub fn invalidate(&mut self) {
        self.lba = u32::MAX;
    }

    /// Make the buffer hold block `lba`.
    pub fn load(&mut self, dev: &mut dyn BlockRead, lba: u32) -> Result<&[u8; 512], Error> {
        if self.lba != lba {
            self.lba = u32::MAX;
            dev.read_block(lba, &mut self.data).map_err(|_| Error::Io)?;
            self.lba = lba;
        }
        Ok(&self.data)
    }
}

pub(crate) fn rd16(b: &[u8], off: usize) -> u32 {
    u16::from_le_bytes([b[off], b[off + 1]]) as u32
}

pub(crate) fn rd32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn looks_like_bpb(s: &[u8; 512]) -> bool {
    (s[0] == 0xEB || s[0] == 0xE9) && rd16(s, 0x0B) == 512 && s[0x0D] != 0
}

fn signed(s: &[u8; 512]) -> bool {
    s[510] == 0x55 && s[511] == 0xAA
}

/// A mounted volume; everything in 512-byte blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Volume {
    fat_lba: u32,
    sectors_per_fat: u32,
    data_lba: u32,
    blocks_per_cluster: u32,
    /// FAT32: the root directory's first cluster. FAT16: 0.
    root_cluster: u32,
    /// Valid cluster numbers are `2..clusters + 2`.
    clusters: u32,
    /// FAT16: the fixed root directory area.
    #[cfg(feature = "fat16")]
    root_lba: u32,
    #[cfg(feature = "fat16")]
    root_blocks: u32,
}

impl Volume {
    /// Find the volume (see the module notes for the rules).
    pub fn mount(dev: &mut dyn BlockRead, blk: &mut Block) -> Result<Volume, Error> {
        let s = blk.load(dev, 0)?;
        if !signed(s) {
            return Err(Error::NoVolume);
        }
        let base = if looks_like_bpb(s) {
            0
        } else {
            let start = rd32(s, 0x1BE + 8);
            if start == 0 {
                return Err(Error::NoVolume);
            }
            let s = blk.load(dev, start)?;
            if !looks_like_bpb(s) || !signed(s) {
                return Err(Error::NoVolume);
            }
            start
        };
        let s = &blk.data;
        let spc = s[0x0D] as u32;
        let total16 = rd16(s, 0x13);
        let total = if total16 != 0 { total16 } else { rd32(s, 0x20) };
        let reserved = rd16(s, 0x0E);
        let num_fats = s[0x10] as u32;
        let root_entries = rd16(s, 0x11);
        let spf16 = rd16(s, 0x16);
        let spf32 = rd32(s, 0x24);
        let root_cluster = rd32(s, 0x2C);
        if num_fats == 0 {
            return Err(Error::NoVolume);
        }
        #[cfg(feature = "fat16")]
        if root_entries != 0 && spf16 != 0 {
            let root_blocks = (root_entries * 32).div_ceil(512);
            let root_off = reserved + num_fats * spf16;
            let meta = root_off + root_blocks;
            if total <= meta {
                return Err(Error::NoVolume);
            }
            let clusters = ((total - meta) / spc).min(spf16 * 256 - 2);
            // FAT12 below, and FAT32 territory above: not a FAT16.
            if !(4085..65525).contains(&clusters) {
                return Err(Error::NoVolume);
            }
            return Ok(Volume {
                fat_lba: base + reserved,
                sectors_per_fat: spf16,
                data_lba: base + meta,
                blocks_per_cluster: spc,
                root_cluster: 0,
                clusters,
                root_lba: base + root_off,
                root_blocks,
            });
        }
        if root_entries != 0 || spf16 != 0 || spf32 == 0 || root_cluster < 2 {
            return Err(Error::NoVolume);
        }
        // What the volume holds after the metadata, and no more than one FAT
        // has entries for (a BPB that claims more is damaged).
        let meta = num_fats
            .checked_mul(spf32)
            .and_then(|f| f.checked_add(reserved))
            .ok_or(Error::NoVolume)?;
        if total <= meta {
            return Err(Error::NoVolume);
        }
        let clusters = ((total - meta) / spc).min(spf32.saturating_mul(128).saturating_sub(2));
        if root_cluster - 2 >= clusters {
            return Err(Error::NoVolume);
        }
        Ok(Volume {
            fat_lba: base + reserved,
            sectors_per_fat: spf32,
            data_lba: base + meta,
            blocks_per_cluster: spc,
            root_cluster,
            clusters,
            #[cfg(feature = "fat16")]
            root_lba: 0,
            #[cfg(feature = "fat16")]
            root_blocks: 0,
        })
    }

    #[inline]
    pub(crate) fn is_fat16(&self) -> bool {
        #[cfg(feature = "fat16")]
        return self.root_blocks != 0;
        #[cfg(not(feature = "fat16"))]
        false
    }

    /// FAT16's fixed root directory: (first block, blocks).
    #[cfg(feature = "fat16")]
    pub(crate) fn fixed_root(&self) -> (u32, u32) {
        (self.root_lba, self.root_blocks)
    }
    #[cfg(not(feature = "fat16"))]
    pub(crate) fn fixed_root(&self) -> (u32, u32) {
        (0, 0)
    }

    pub fn blocks_per_cluster(&self) -> u32 {
        self.blocks_per_cluster
    }

    /// Data clusters on the volume.
    pub fn clusters(&self) -> u32 {
        self.clusters
    }

    /// The root directory as a directory cluster for `dir::walk`: the
    /// FAT32 root cluster, or 0 (also accepted for FAT32, as `..` stores it).
    pub fn root(&self) -> u32 {
        self.root_cluster
    }

    /// First block of `cluster`; `None` outside the volume.
    pub fn cluster_lba(&self, cluster: u32) -> Option<u32> {
        if cluster < 2 || cluster - 2 >= self.clusters {
            return None;
        }
        (cluster - 2).checked_mul(self.blocks_per_cluster)?.checked_add(self.data_lba)
    }

    /// The cluster after `cluster` in its chain; `None` at the end.
    pub fn next_cluster(&self, dev: &mut dyn BlockRead, blk: &mut Block, cluster: u32) -> Result<Option<u32>, Error> {
        if cluster < 2 || cluster - 2 >= self.clusters {
            return Err(Error::BadChain);
        }
        let (per, eoc) = if self.is_fat16() { (256, 0xFFF8) } else { (128, 0x0FFF_FFF8) };
        let sector = cluster / per;
        if sector >= self.sectors_per_fat {
            return Err(Error::BadChain);
        }
        let s = blk.load(dev, self.fat_lba + sector)?;
        let i = (cluster % per) as usize;
        let v = if self.is_fat16() { rd16(s, i * 2) } else { rd32(s, i * 4) & 0x0FFF_FFFF };
        if v >= eoc {
            Ok(None)
        } else if v < 2 || v - 2 >= self.clusters {
            // Free, reserved, bad or past the end: damaged.
            Err(Error::BadChain)
        } else {
            Ok(Some(v))
        }
    }
}
