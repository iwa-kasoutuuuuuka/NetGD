//! The two supported parts. Same values and semantics as `Part` in
//! `mcu/fwupdater/src/logic.rs` (SAM3U datasheet Atmel-6430G table 8-3 for
//! the flash geometry, `docs/design/fw-bootloader.md` section 1).

pub const SRAM0_BASE: u32 = 0x2000_0000;

pub const CIDR_SAM3U2E: u32 = 0x281A_0761;
pub const CIDR_SAM3U4E: u32 = 0x2810_0961;
/// CHIPID_CIDR[4:0] is the silicon revision; ignored on compare.
pub const CIDR_VERSION_MASK: u32 = 0x1F;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    Sam3u2e,
    Sam3u4e,
}

impl Part {
    pub fn from_cidr(cidr: u32) -> Option<Part> {
        let c = cidr & !CIDR_VERSION_MASK;
        if c == CIDR_SAM3U2E & !CIDR_VERSION_MASK {
            Some(Part::Sam3u2e)
        } else if c == CIDR_SAM3U4E & !CIDR_VERSION_MASK {
            Some(Part::Sam3u4e)
        } else {
            None
        }
    }

    /// One past the last byte of SRAM0.
    pub fn sram0_end(self) -> u32 {
        match self {
            Part::Sam3u2e => SRAM0_BASE + 0x4000,
            Part::Sam3u4e => SRAM0_BASE + 0x8000,
        }
    }

    /// Only the 4E has the second flash plane (EEFC1, 0x0010_0000).
    pub fn has_plane1(self) -> bool {
        self == Part::Sam3u4e
    }
}
