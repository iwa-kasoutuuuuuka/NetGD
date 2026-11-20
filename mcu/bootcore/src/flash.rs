//! The flash as the boot logic sees it.
//!
//! A hardware implementation (stage S3/S4) reads flash through its memory
//! map and builds every `EEFC_FCR` value with [`crate::fcr::fcr_word_for`];
//! the EWP routine must run from RAM, because the flash cannot be read while
//! a program command runs (design 1-1). The host model in `tests/common`
//! enforces exactly that: a `read` while a command is busy is counted as a
//! violation.

use crate::fcr::Plane;

pub const FLASH_BASE: u32 = 0x0008_0000;
pub const PLANE1_BASE: u32 = 0x0010_0000;
pub const PLANE_LEN: u32 = 0x0002_0000;
pub const PAGE: usize = 256;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FlashError {
    /// `fcr_word` refused the command (page outside what we may write).
    Refused,
    /// FLOCKE: the page is in a locked region.
    Locked,
    /// The controller or the supply failed.
    Failed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Poll {
    Busy,
    Done(Result<(), FlashError>),
}

pub trait Flash {
    /// Read flash bytes at an absolute address. Never called while a command
    /// is busy.
    fn read(&mut self, addr: u32, out: &mut [u8]);

    /// Latch `data` and start EWP for `page` of `plane`. Returns at once.
    fn ewp_start(&mut self, plane: Plane, page: u32, data: &[u8; PAGE]) -> Result<(), FlashError>;

    /// FRDY and, once ready, FLOCKE/FCMDE as a result.
    fn ewp_poll(&mut self) -> Poll;

    /// GLB: bit n set = lock region n of `plane` is locked.
    fn lock_bits(&mut self, plane: Plane) -> u32;

    /// Erase and write one page and wait for it.
    fn ewp(&mut self, plane: Plane, page: u32, data: &[u8; PAGE]) -> Result<(), FlashError> {
        self.ewp_start(plane, page, data)?;
        loop {
            if let Poll::Done(r) = self.ewp_poll() {
                return r;
            }
        }
    }
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
