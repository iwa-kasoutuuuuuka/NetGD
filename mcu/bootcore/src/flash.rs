//! The flash as the boot logic sees it.
//!
//! A hardware implementation (stage S3/S4) reads flash through its memory
//! map and gets every command as an [`FcrCmd`] from [`crate::fcr`]. The EWP
//! is **one blocking method**: the hardware implementation is a single
//! routine that runs from RAM (the flash cannot be read, and so no
//! instruction can be fetched from it, while a program command runs on that
//! plane; design 1-1): fill the latch with 32-bit writes, write FCR, poll
//! FSR with a timeout, and return the FLOCKE/FCMDE flags of the same FSR read
//! that saw FRDY (they clear on read). There is no polling loop in this
//! crate, so no bootcore code runs from flash while a command is busy. The
//! host model in `tests/common` checks that `read` is never entered while
//! an `ewp` is in progress and cuts power inside `ewp`.
//!
//! **Wait states** (design 2-2 K6; datasheet errata 46.1.1.3, 46.2.1.2): a
//! flash command runs with FWS = 6, everything else with FWS = 4. The portable
//! code owns that: it calls [`Flash::set_fws`] (`FWS_WRITE` before
//! `ewp`/`slb`, `FWS_RUN` right after, also on error), so the host model can
//! count every EWP or SLB at the wrong FWS, and every flash read (and so the
//! hand-off) at FWS = 6, as violations.
//!
//! **Watchdog** (design 2-3 W1/W3): the resident never writes `WDT_MR`. While
//! it recovers it rewinds the counter (`WDT_CR` = `0xA5000001`); the
//! implementations of [`Flash::ewp`] and of [`crate::fat::BlockRead::read_block`]
//! do that as their first act, so every SD sector and every flash page rewinds
//! it. The pure code in between (CRC over the flash, a FAT chain step) is
//! bounded: `tests/watchdog.rs` bounds the flash bytes read between two
//! rewinds in the model.

use crate::fcr::{FcrCmd, Plane};

pub const FLASH_BASE: u32 = 0x0008_0000;
pub const PLANE1_BASE: u32 = 0x0010_0000;
pub const PLANE_LEN: u32 = 0x0002_0000;
pub const PAGE: usize = 256;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FlashError {
    /// The command was refused (page outside what we may write, FCMDE).
    Refused,
    /// FLOCKE: the page is in a locked region.
    Locked,
    /// The controller or the supply failed.
    Failed,
    /// FRDY did not come up in time. The page content is unknown.
    Timeout,
}

pub trait Flash {
    /// Read flash bytes at an absolute address. Never called while `ewp`
    /// is running.
    fn read(&mut self, addr: u32, out: &mut [u8]);

    /// Set `EEFC_FMR.FWS` of every plane the part has ([`crate::part::FWS_RUN`]
    /// or [`crate::part::FWS_WRITE`]; FRDY interrupt off).
    fn set_fws(&mut self, fws: u8);

    /// Latch `data` into the page `cmd` names (an EWP command) and run it to
    /// completion, with a timeout (`Err(Timeout)`). FLOCKE gives
    /// `Err(Locked)`, FCMDE `Err(Refused)`. Called with FWS = 6; rewinds the
    /// watchdog first.
    fn ewp(&mut self, cmd: FcrCmd, data: &[u8; PAGE]) -> Result<(), FlashError>;

    /// Run SLB (`cmd` from [`FcrCmd::slb`]): write FCR, wait for FRDY with a
    /// timeout. Errors as for `ewp`. Called with FWS = 6. Flash may be read
    /// while it runs (datasheet 20.4.3.4) but the resident does not.
    fn slb(&mut self, cmd: FcrCmd) -> Result<(), FlashError>;

    /// GLB: bit n set = lock region n of `plane` is locked. A failure must
    /// be reported as all ones (everything locked).
    fn lock_bits(&mut self, plane: Plane) -> u32;
}

/// Plane and page index (relative to the plane, as FARG wants it) of a
/// page-aligned flash address.
pub fn locate(addr: u32) -> Option<(Plane, u32)> {
    if addr & (PAGE as u32 - 1) != 0 {
        return None;
    }
    if addr >= FLASH_BASE && addr < FLASH_BASE + PLANE_LEN {
        Some((Plane::P0, (addr - FLASH_BASE) / PAGE as u32))
    } else if addr >= PLANE1_BASE && addr < PLANE1_BASE + PLANE_LEN {
        Some((Plane::P1, (addr - PLANE1_BASE) / PAGE as u32))
    } else {
        None
    }
}
