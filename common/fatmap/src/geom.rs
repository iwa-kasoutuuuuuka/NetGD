//! FAT32 volume geometry from a boot sector, with no I/O: the arithmetic
//! behind the firmware's `fat::probe`, its cluster checks and its directory
//! bound (`mcu/netgd/src/fat.rs`), kept here so that the host tests can
//! feed it crafted boot sectors (`tests/geom.rs`).

use crate::extent::{ClusterGeom, Cursor};

/// Volume geometry, everything in 512-byte blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Geometry {
    pub fat_lba: u32,
    pub sectors_per_fat: u32,
    pub data_lba: u32,
    pub blocks_per_cluster: u32,
    pub root_cluster: u32,
    /// Data clusters on the volume: valid cluster numbers are `2..clusters + 2`.
    /// Bounded by the volume's size and by what one FAT can hold.
    pub clusters: u32,
}

/// Why a boot sector is not a FAT32 volume this reader takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum BpbError {
    /// FAT12/16 or not FAT at all.
    NotFat32,
    /// FAT32-shaped but inconsistent.
    BadBpb,
    /// The root cluster is outside the data area.
    RootOutside,
}

fn rd16(b: &[u8], off: usize) -> u32 {
    u16::from_le_bytes([b[off], b[off + 1]]) as u32
}

fn rd32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

/// A jump, 512 bytes per sector and a non-zero cluster size: a boot sector
/// (used to tell a "superfloppy" from an MBR at LBA 0).
pub fn looks_like_bpb(s: &[u8; 512]) -> bool {
    (s[0] == 0xEB || s[0] == 0xE9) && rd16(s, 0x0B) == 512 && s[0x0D] != 0
}

/// The geometry of the FAT32 volume whose boot sector `sec` is at block
/// `base`. FAT32 is told by the BPB's shape (no fixed root, no 16-bit FAT
/// size), not by the cluster count.
pub fn parse_fat32(sec: &[u8; 512], base: u32) -> Result<Geometry, BpbError> {
    let blocks_per_cluster = sec[0x0D] as u32;
    let total16 = rd16(sec, 0x13);
    let total = if total16 != 0 { total16 } else { rd32(sec, 0x20) };
    let reserved = rd16(sec, 0x0E);
    let num_fats = sec[0x10] as u32;
    let root_entries = rd16(sec, 0x11);
    let spf16 = rd16(sec, 0x16);
    let spf32 = rd32(sec, 0x24);
    let root_cluster = rd32(sec, 0x2C);
    if root_entries != 0 || spf16 != 0 || spf32 == 0 || num_fats == 0 || root_cluster < 2 {
        return Err(BpbError::NotFat32);
    }
    // Cluster count: what the volume holds after the metadata, and no more
    // than the FAT has entries for (a BPB that claims more is damaged, and the
    // smaller number is the safe one).
    let meta = reserved
        .checked_add(num_fats.checked_mul(spf32).ok_or(BpbError::BadBpb)?)
        .ok_or(BpbError::BadBpb)?;
    if blocks_per_cluster == 0 || total <= meta {
        return Err(BpbError::BadBpb);
    }
    let clusters = ((total - meta) / blocks_per_cluster).min(spf32.saturating_mul(128).saturating_sub(2));
    if root_cluster - 2 >= clusters {
        return Err(BpbError::RootOutside);
    }
    Ok(Geometry {
        fat_lba: base.wrapping_add(reserved),
        sectors_per_fat: spf32,
        data_lba: base.wrapping_add(meta),
        blocks_per_cluster,
        root_cluster,
        clusters,
    })
}

impl Geometry {
    /// First block of `cluster`, `None` outside the volume (a damaged FAT
    /// value must not become a block number).
    pub fn cluster_lba(&self, cluster: u32) -> Option<u32> {
        if cluster < 2 || cluster - 2 >= self.clusters {
            return None;
        }
        self.cluster_geom().checked_cluster_lba(cluster)
    }

    pub fn cluster_geom(&self) -> ClusterGeom {
        ClusterGeom { data_lba: self.data_lba, blocks_per_cluster: self.blocks_per_cluster }
    }

    /// Where a walk of the file with this first cluster and size starts;
    /// `None` for an empty file or one that cannot be walked.
    pub fn start(&self, first_cluster: u32, size: u32) -> Option<Cursor> {
        Cursor::start(first_cluster, size)
    }

    /// A FAT entry's value (`raw`, top four bits included) as a link:
    /// `Ok(None)` at end of chain, `Err` for free, reserved, bad or past the
    /// end of the volume.
    pub fn link(&self, raw: u32) -> Result<Option<u32>, ()> {
        let v = raw & 0x0FFF_FFFF;
        if v >= 0x0FFF_FFF8 {
            Ok(None)
        } else if v < 2 || v - 2 >= self.clusters {
            Err(())
        } else {
            Ok(Some(v))
        }
    }

    /// Most clusters a directory walk visits before it calls the chain a
    /// loop.
    pub fn dir_max_hops(&self) -> u32 {
        self.clusters
    }
}
