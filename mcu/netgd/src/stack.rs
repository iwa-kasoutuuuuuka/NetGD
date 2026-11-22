//! Stack high-water mark on the board (docs/PLAN.md A-1; cargo feature
//! `stack-hw`). The scan and the reply layout are `common/stackmark`
//! (host-tested); this is the part that touches memory.
//!
//! * [`paint`] fills the free part of the stack with `stackmark::PATTERN`,
//!   once, from `main` right after `embassy_atsam3::init` (so it runs at
//!   the PLL clock, not the 4 MHz reset clock, and before any task exists).
//!   Region: `__sheap` (cortex-m-rt's end of `.uninit`, i.e. above every
//!   static, `defmt_rtt`'s buffer included) up to [`GUARD`] bytes below the
//!   current MSP. Everything below MSP is dead at that point: nothing has
//!   been spawned, and an interrupt frame that lands there is gone again
//!   before this code resumes. Nothing above `__sheap` survives a reset or
//!   is handed over by the resident (resident-app-contract.md 2-5 M1: SRAM0
//!   is "undefined" to the app; the hand-off is GPBR and flash only).
//! * [`report`] scans upward from `__sheap` for the first word that lost the
//!   pattern. Read-only; called by `0xE0` sub-command 9 (`ide.rs`).
//!
//! The firmware has one stack (MSP): thread mode, embassy's tasks and every
//! interrupt handler all run on it, so one mark covers them all.

use core::ptr::{addr_of, read_volatile, write_volatile};
use core::sync::atomic::{AtomicU32, Ordering};

extern "C" {
    /// cortex-m-rt `link.x`: the first byte after `.uninit` (4-aligned).
    static __sheap: u32;
    /// cortex-m-rt `link.x`: the initial MSP, the end of SRAM0 (8-aligned).
    static _stack_start: u32;
}

/// Bytes left unpainted below the MSP that [`paint`] sees: its own return
/// path needs none (it is a leaf apart from the inlined MSP read), but a few
/// words of margin cost nothing.
const GUARD: usize = 32;

/// `_stack_start` minus the first address [`paint`] did not paint. Zero
/// until it has run.
static UNPAINTED: AtomicU32 = AtomicU32::new(0);

fn region() -> (usize, usize) {
    (addr_of!(__sheap) as usize, addr_of!(_stack_start) as usize)
}

/// Paint the free stack. Call once, from `main`, before anything is spawned.
#[inline(never)]
pub fn paint() {
    let (lo, top) = region();
    let sp = cortex_m::register::msp::read() as usize;
    let hi = sp.saturating_sub(GUARD) & !3;
    let mut p = lo;
    while p < hi {
        // SAFETY: [lo, hi) is SRAM0 between the last static and the live
        // stack, owned by no one (see the module comment).
        unsafe { write_volatile(p as *mut u32, stackmark::PATTERN) };
        p += 4;
    }
    UNPAINTED.store((top - p) as u32, Ordering::Relaxed);
}

/// The high-water mark since the last reset.
pub fn report() -> stackmark::Report {
    let (lo, top) = region();
    let reserved = (top - lo) as u32;
    let n = stackmark::untouched_words(reserved as usize / 4, |i| {
        // SAFETY: inside SRAM0; a plain read of memory the stack may own.
        unsafe { read_volatile((lo + 4 * i) as *const u32) }
    });
    stackmark::Report::new(reserved, n, UNPAINTED.load(Ordering::Relaxed))
}
