//! The two supported parts. Same values and semantics as `Part` in
//! `mcu/fwupdater/src/logic.rs` (SAM3U datasheet Atmel-6430G table 8-3 for
//! the flash geometry, `docs/design/fw-bootloader.md` section 1).

pub const SRAM0_BASE: u32 = 0x2000_0000;

// --- clocks and flash wait states (docs/design/resident-app-contract.md
// 2-2, N3) -------------------------------------------------------------
//
// The values the resident leaves the chip in at hand-off, and the values the
// application builds when it has to (`mcu/embassy-atsam3/src/clocks.rs`).
// One definition for both, so that the application's "already configured"
// test keeps recognising what the resident hands over. Dependency-free
// numbers only; the register code stays in the two users.

/// The main crystal.
pub const XTAL_HZ: u32 = 12_000_000;
/// `CKGR_MOR.MOSCXTST`: crystal start-up time, in units of 8 slow-clock cycles.
pub const MOSC_XTST: u8 = 8;
/// PLLA multiplier as a factor; the `CKGR_PLLAR.MULA` field holds `PLLA_MUL - 1`.
pub const PLLA_MUL: u32 = 8;
/// `CKGR_PLLAR.DIVA` (1 = divide by 1).
pub const PLLA_DIV: u32 = 1;
/// `CKGR_PLLAR.PLLACOUNT`: slow-clock cycles before LOCKA.
pub const PLLA_COUNT: u8 = 0x3F;
/// MCK at hand-off: PLLA, prescaler 1 (`MCKR.CSS` = PLLA, `PRES` = 0).
pub const MCK_HZ: u32 = XTAL_HZ / PLLA_DIV * PLLA_MUL;
/// `EEFC_FMR.FWS` while running (and at hand-off), on both planes.
pub const FWS_RUN: u8 = 4;
/// `EEFC_FMR.FWS` while a flash command (EWP, SLB) runs (datasheet errata
/// 46.1.1.3 / 46.2.1.2). Always restored to [`FWS_RUN`] before anything is
/// read from flash again.
pub const FWS_WRITE: u8 = 6;
/// `EEFC_FMR` value at hand-off: FWS = 4, no FRDY interrupt.
pub const EEFC_FMR_RUN: u32 = (FWS_RUN as u32) << 8;

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
