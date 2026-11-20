//! Read-only FAT32 over 512-byte block reads, for the resident's SD
//! recovery. One caller-supplied 512-byte buffer is the only working memory
//! (plus a few words of cursor state): every call that reads the FAT or a
//! directory clobbers it, so the caller consumes each data chunk before
//! asking for the next.
//!
//! What it accepts, compared with `mcu/opengdemu/src/fat.rs` (the
//! application's `probe`, which delegates the type decision to
//! embedded-sdmmc):
//!
//! * LBA 0 must end in 0x55 0xAA (same).
//! * LBA 0 that already is a BPB (`EB`/`E9` jump, 512 bytes/sector, non-zero
//!   sectors/cluster) is used as a "super floppy" (same).
//! * Otherwise MBR partition entry 0 (same), **but only types 0x0B and
//!   0x0C** (FAT32 CHS/LBA; what embedded-sdmmc accepts). Type 0xEE (GPT
//!   protective MBR) is [`FatError::Gpt`]; every other type, and an `EXFAT`
//!   boot sector, is [`FatError::NotFat32`]. fat.rs itself does not look at
//!   the type byte; it looks at the BPB, as this code does too.
//! * BPB: FAT32 means `root_entries == 0`, `sectors_per_fat16 == 0`,
//!   `sectors_per_fat32 != 0` (same, so FAT12/16 are refused). Additionally
//!   refused here (fat.rs trusts them): sectors/cluster not a power of two
//!   or > 128, zero reserved sectors, more than 2 FATs, a root cluster or FAT
//!   too small for the volume, sizes that overflow 32 bits.
//! * The cluster count is **not** required to be >= 65525 (the Microsoft
//!   rule for telling FAT32 from FAT16); the structural test above decides,
//!   as in fat.rs. Only FAT 0 is read (the "only one FAT active" flag in
//!   BPB offset 0x28 is ignored, as in fat.rs).
//! * The file is found by exact 8.3 name in the **root directory**. Long
//!   name entries (attribute 0x0F), deleted entries, volume labels and
//!   directories are skipped. A short name hidden behind a long one is found
//!   (Windows writes the 8.3 entry right after its LFN entries).
//! * Chains: an entry below 2, the bad-cluster mark, or above the last
//!   cluster is [`FatError::BadChain`]; end of chain before the file's size
//!   is [`FatError::Truncated`]; a chain that comes back on itself is
//!   [`FatError::Loop`] (Brent cycle detection: 3 words of state, finds any
//!   cycle in a bounded number of steps).

/// Whatever reads 512-byte sectors (the HSMCI polling reader on the board).
pub trait BlockRead {
    fn read_block(&mut self, lba: u32, buf: &mut [u8; 512]) -> Result<(), ()>;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FatError {
    /// The block device failed.
    Read,
    /// LBA 0 (or the partition's first sector) lacks the 0x55AA signature.
    NoSignature,
    /// GPT protective MBR.
    Gpt,
    /// FAT12/16, exFAT, wrong partition type.
    NotFat32,
    /// A BPB that is FAT32-shaped but inconsistent.
    BadBpb,
    BadChain,
    Loop,
    /// Chain ends before the size in the directory entry is reached.
    Truncated,
    NotFound,
    /// Found, but empty (no data to flash).
    Empty,
}

/// The names the resident looks for (design D6).
pub const NAME_APP: [u8; 11] = *b"NETGD   OGF";
pub const NAME_BITSTREAM: [u8; 11] = *b"NETGD   BIT";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Volume {
    fat_lba: u32,
    fat_sectors: u32,
    data_lba: u32,
    spc: u32,
    root: u32,
    /// Highest valid cluster number (clusters + 1).
    max_cluster: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DirEnt {
    pub first_cluster: u32,
    pub size: u32,
}

const EOC: u32 = 0x0FFF_FFF8;
const BAD: u32 = 0x0FFF_FFF7;

fn rd16(b: &[u8; 512], o: usize) -> u32 {
    crate::rd16(b, o)
}

fn rd32(b: &[u8; 512], o: usize) -> u32 {
    crate::rd32(b, o)
}

fn looks_like_bpb(s: &[u8; 512]) -> bool {
    (s[0] == 0xEB || s[0] == 0xE9) && rd16(s, 0x0B) == 512 && s[0x0D] != 0
}

fn read<D: BlockRead>(dev: &mut D, lba: u32, buf: &mut [u8; 512]) -> Result<(), FatError> {
    dev.read_block(lba, buf).map_err(|_| FatError::Read)
}

/// Brent's cycle detection over cluster numbers.
#[derive(Clone, Copy)]
struct Guard {
    mark: u32,
    power: u32,
    steps: u32,
}

impl Guard {
    fn new(first: u32) -> Self {
        Self { mark: first, power: 1, steps: 0 }
    }

    /// Record that the walk moved to `c`; true if that closes a cycle.
    fn step(&mut self, c: u32) -> bool {
        if c == self.mark {
            return true;
        }
        self.steps += 1;
        if self.steps == self.power {
            self.mark = c;
            self.power = self.power.wrapping_shl(1);
            self.steps = 0;
        }
        false
    }
}

impl Volume {
    /// Locate and validate the first FAT32 volume. `buf` is scratch.
    pub fn mount<D: BlockRead>(dev: &mut D, buf: &mut [u8; 512]) -> Result<Volume, FatError> {
        read(dev, 0, buf)?;
        if buf[510] != 0x55 || buf[511] != 0xAA {
            return Err(FatError::NoSignature);
        }
        let base = if looks_like_bpb(buf) {
            0
        } else {
            let ty = buf[0x1BE + 4];
            if ty == 0xEE {
                return Err(FatError::Gpt);
            }
            if ty != 0x0B && ty != 0x0C {
                return Err(FatError::NotFat32);
            }
            let start = rd32(buf, 0x1BE + 8);
            if start == 0 {
                return Err(FatError::BadBpb);
            }
            read(dev, start, buf)?;
            if buf[510] != 0x55 || buf[511] != 0xAA {
                return Err(FatError::NoSignature);
            }
            if !looks_like_bpb(buf) {
                return Err(FatError::NotFat32);
            }
            start
        };
        // exFAT keeps a jump and "EXFAT   " but zeroes the BPB, so it fails
        // looks_like_bpb above; this catches a super-floppy exFAT whose
        // bytes happen to look like an MBR with a FAT32 type.
        if buf.get(3..11) == Some(&b"EXFAT   "[..]) {
            return Err(FatError::NotFat32);
        }

        let spc = buf[0x0D] as u32;
        if !spc.is_power_of_two() || spc > 128 {
            return Err(FatError::BadBpb);
        }
        let reserved = rd16(buf, 0x0E);
        let nfats = buf[0x10] as u32;
        if reserved == 0 || nfats == 0 || nfats > 2 {
            return Err(FatError::BadBpb);
        }
        if rd16(buf, 0x11) != 0 || rd16(buf, 0x16) != 0 {
            return Err(FatError::NotFat32);
        }
        let spf = rd32(buf, 0x24);
        if spf == 0 {
            return Err(FatError::NotFat32);
        }
        let root = rd32(buf, 0x2C);
        let tot16 = rd16(buf, 0x13);
        let total = if tot16 != 0 { tot16 } else { rd32(buf, 0x20) };

        let meta = match spf.checked_mul(nfats).and_then(|f| f.checked_add(reserved)) {
            Some(m) => m,
            None => return Err(FatError::BadBpb),
        };
        if total <= meta {
            return Err(FatError::BadBpb);
        }
        let clusters = (total - meta) / spc;
        // The FAT must have an entry for every cluster (and the two reserved).
        if clusters == 0 || (spf as u64) * 128 < clusters as u64 + 2 {
            return Err(FatError::BadBpb);
        }
        let max_cluster = clusters + 1;
        if root < 2 || root > max_cluster {
            return Err(FatError::BadBpb);
        }
        let fat_lba = base.checked_add(reserved);
        let data_lba = fat_lba.and_then(|l| l.checked_add(spf * nfats));
        let (fat_lba, data_lba) = match (fat_lba, data_lba) {
            (Some(f), Some(d)) if d.checked_add(clusters * spc).is_some() => (f, d),
            _ => return Err(FatError::BadBpb),
        };
        Ok(Volume { fat_lba, fat_sectors: spf, data_lba, spc, root, max_cluster })
    }

    #[inline]
    fn lba(&self, cluster: u32) -> u32 {
        self.data_lba + (cluster - 2) * self.spc
    }

    /// FAT entry of `c`: `None` at end of chain.
    fn next<D: BlockRead>(&self, dev: &mut D, buf: &mut [u8; 512], c: u32) -> Result<Option<u32>, FatError> {
        let sec = c >> 7;
        if sec >= self.fat_sectors {
            return Err(FatError::BadChain);
        }
        read(dev, self.fat_lba + sec, buf)?;
        let v = rd32(buf, ((c & 127) << 2) as usize) & 0x0FFF_FFFF;
        if v >= EOC {
            Ok(None)
        } else if v < 2 || v == BAD || v > self.max_cluster {
            Err(FatError::BadChain)
        } else {
            Ok(Some(v))
        }
    }

    /// Find `name` (11 bytes, space padded, upper case) in the root
    /// directory. `buf` is scratch and is clobbered.
    pub fn find<D: BlockRead>(&self, dev: &mut D, buf: &mut [u8; 512], name: &[u8; 11]) -> Result<DirEnt, FatError> {
        let mut c = self.root;
        let mut guard = Guard::new(c);
        loop {
            let base = self.lba(c);
            for s in 0..self.spc {
                read(dev, base + s, buf)?;
                for e in buf.chunks_exact(32) {
                    let first = e.first().copied().unwrap_or(0);
                    if first == 0x00 {
                        return Err(FatError::NotFound);
                    }
                    let attr = e.get(11).copied().unwrap_or(0);
                    // deleted, long-name part, volume label, directory
                    if first == 0xE5 || attr == 0x0F || attr & 0x18 != 0 {
                        continue;
                    }
                    if e.get(..11) != Some(&name[..]) {
                        continue;
                    }
                    let hi = crate::rd16(e, 20);
                    let lo = crate::rd16(e, 26);
                    let size = crate::rd32(e, 28);
                    let first_cluster = (hi << 16) | lo;
                    if size == 0 {
                        return Err(FatError::Empty);
                    }
                    // The chain must at least be able to hold `size` bytes.
                    let cap = (self.max_cluster - 1) as u64 * self.spc as u64 * 512;
                    if first_cluster < 2 || first_cluster > self.max_cluster || size as u64 > cap {
                        return Err(FatError::BadChain);
                    }
                    return Ok(DirEnt { first_cluster, size });
                }
            }
            match self.next(dev, buf, c)? {
                None => return Err(FatError::NotFound),
                Some(n) => {
                    if guard.step(n) {
                        return Err(FatError::Loop);
                    }
                    c = n;
                }
            }
        }
    }
}

/// Sequential reader of one file in 512-byte chunks.
pub struct FileReader {
    cluster: u32,
    /// Sectors of `cluster` already handed out.
    sec: u32,
    remaining: u32,
    guard: Guard,
}

impl FileReader {
    pub fn new(ent: &DirEnt) -> Self {
        Self { cluster: ent.first_cluster, sec: 0, remaining: ent.size, guard: Guard::new(ent.first_cluster) }
    }

    /// Bytes not yet handed out.
    pub fn remaining(&self) -> u32 {
        self.remaining
    }

    /// Read the next chunk into `buf` and return how many of its bytes are
    /// file data (512, except for the last chunk of a file whose size is not
    /// a multiple of 512); 0 at end of file. Clobbers `buf` also when it has
    /// to read the FAT, which is why the caller must be done with the
    /// previous chunk.
    pub fn read_next<D: BlockRead>(&mut self, vol: &Volume, dev: &mut D, buf: &mut [u8; 512]) -> Result<usize, FatError> {
        if self.remaining == 0 {
            return Ok(0);
        }
        if self.sec == vol.spc {
            match vol.next(dev, buf, self.cluster)? {
                None => return Err(FatError::Truncated),
                Some(n) => {
                    if self.guard.step(n) {
                        return Err(FatError::Loop);
                    }
                    self.cluster = n;
                    self.sec = 0;
                }
            }
        }
        read(dev, vol.lba(self.cluster) + self.sec, buf)?;
        self.sec += 1;
        let n = if self.remaining > 512 { 512 } else { self.remaining };
        self.remaining -= n;
        Ok(n as usize)
    }
}
