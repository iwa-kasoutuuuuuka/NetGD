//! The resident's information block: the last 16 bytes of lock region 0
//! (design 3-3, `tools/fwimage/FORMAT.md`). The application reads it with
//! plain loads (no EEFC) to learn which resident it runs behind.
//!
//! ```text
//!   +0   magic "NGRS"
//!   +4   byte 0  interface generation (IFACE_GEN, 1)
//!        byte 1  flags (INFO_F_*)
//!        byte 2-3 layout >> 8 (0x0020 for the 8 KB region)
//!   +8   resident version  major << 16 | minor << 8 | patch
//!   +12  CRC-32 of the resident region 0x80000..0x81FFB, the last four bytes
//!        of the region (the same rule as an application's trailer, so
//!        `app::check_trailer` accepts the whole 8192 bytes)
//! ```
//!
//! Unused bytes of the region are 0xFF. `fwimage seal-resident` fills the
//! CRC. The gen is raised only when design sections 2 and 3 change in a way
//! that is not compatible (3-4); the version moves with every build.

use crate::app::{check_trailer, LAYOUT};
use crate::rd32;

/// First byte of the resident region.
pub const RESIDENT_BASE: u32 = 0x0008_0000;
/// Length of the resident region (= [`LAYOUT`]).
pub const RESIDENT_LEN: usize = LAYOUT as usize;
/// Where the block sits.
pub const INFO_ADDR: u32 = RESIDENT_BASE + LAYOUT - INFO_LEN as u32;
pub const INFO_LEN: usize = 16;
/// `"NGRS"` read as a little-endian word.
pub const INFO_MAGIC: u32 = u32::from_le_bytes(*b"NGRS");
/// The interface generation this crate and `docs/design/resident-app-contract.md`
/// describe.
pub const IFACE_GEN: u8 = 1;

/// Flags (byte 1 of word 1).
/// A development build (no SLB, accepts unsealed apps, may write `WDT_MR`).
pub const INFO_F_DEV: u8 = 1 << 0;
/// This build can lock itself (issues SLB).
pub const INFO_F_SELF_LOCK: u8 = 1 << 1;
/// This build reads the front button.
pub const INFO_F_BUTTON: u8 = 1 << 2;
/// This build drives an LED.
pub const INFO_F_LED: u8 = 1 << 3;
/// This build writes `WDT_MR` (development only).
pub const INFO_F_WDT_MR: u8 = 1 << 4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Info {
    pub gen: u8,
    pub flags: u8,
    /// `layout >> 8` as stored.
    pub layout_shr8: u16,
    pub version: u32,
    pub crc: u32,
}

impl Info {
    pub fn dev(&self) -> bool {
        self.flags & INFO_F_DEV != 0
    }

    pub fn layout(&self) -> u32 {
        (self.layout_shr8 as u32) << 8
    }
}

/// `None` (version unknown) when the magic is wrong or the generation is 0.
pub fn resident_info(b: &[u8; INFO_LEN]) -> Option<Info> {
    if rd32(b, 0) != INFO_MAGIC {
        return None;
    }
    let w = rd32(b, 4);
    let gen = w as u8;
    if gen == 0 {
        return None;
    }
    Some(Info {
        gen,
        flags: (w >> 8) as u8,
        layout_shr8: (w >> 16) as u16,
        version: rd32(b, 8),
        crc: rd32(b, 12),
    })
}

/// The 16 bytes (tools and tests).
pub fn pack_info(gen: u8, flags: u8, layout: u32, version: u32, crc: u32) -> [u8; INFO_LEN] {
    let w1 = gen as u32 | (flags as u32) << 8 | (layout >> 8) << 16;
    let mut b = [0u8; INFO_LEN];
    for (i, v) in [INFO_MAGIC, w1, version, crc].iter().enumerate() {
        b[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
    }
    b
}

/// `major << 16 | minor << 8 | patch`.
#[inline]
pub const fn version(major: u8, minor: u8, patch: u8) -> u32 {
    (major as u32) << 16 | (minor as u32) << 8 | patch as u32
}

/// Is `region` (the 8192 bytes at [`RESIDENT_BASE`]) intact: right length and
/// its last four bytes are the CRC-32 of the rest? About 1 ms; the application
/// does it only on request (`netgd-fwupd`), not at boot.
pub fn check_region(region: &[u8]) -> bool {
    region.len() == RESIDENT_LEN && check_trailer(region)
}
