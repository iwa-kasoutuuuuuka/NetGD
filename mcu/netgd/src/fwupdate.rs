//! Handover to the standalone firmware updater.
//!
//! The flash writer itself is not in this binary. It is a separate program
//! (`mcu/fwupdater`) linked to run from the base of SRAM1 and embedded here
//! as a flat blob by `build.rs`; this module only recognises the command
//! sequence that starts an update, copies the blob into RAM, and branches to
//! it. See that crate's module docs for the wire protocol and for why the
//! writer cannot execute from flash.
//!
//! Everything in *this* file still runs from flash, which is fine: the copy
//! and the branch both happen before a single page is erased.
//!
//! # Getting in
//!
//! Two steps, both vendor opcode [`CMD_FW`] on the disk device, each
//! carrying a 48-bit key in the LBA registers on top of the FEATURES
//! selector:
//!
//! ```text
//!   F9 / feat 4F + MAGIC_ARM     arm    -- recoverable, drive keeps working
//!   F9 / feat 53 + MAGIC_ENTER   enter  -- one-way
//! ```
//!
//! 56 bits of key across two commands is not something a BIOS, a WinCE
//! driver, or a stray disk write arrives at by accident, which is the point:
//! nothing should be able to wander into a firmware update.
//!
//! # Once inside
//!
//! The updater refuses images for another part (CHIPID_CIDR in the header),
//! checks the new vector table against this part's SRAM before erasing
//! anything, writes page 0 last, and only resets after a verified image; see
//! `mcu/fwupdater/src/main.rs` and `tools/fwimage/FORMAT.md`. It keeps a
//! 256-byte copy of the new page 0 on the caller's stack.

use defmt::warn;

/// Vendor-specific ATA opcode. ATA-5 leaves 0xF0–0xFF to the vendor.
pub const CMD_FW: u8 = 0xF9;

#[cfg_attr(feature = "resident", allow(dead_code))]
pub const FEAT_ARM: u8 = 0x4F;
#[cfg_attr(feature = "resident", allow(dead_code))]
pub const FEAT_ENTER: u8 = 0x53;

/// 48-bit key carried in the LBA registers of the arm command.
#[cfg_attr(feature = "resident", allow(dead_code))]
pub const MAGIC_ARM: u64 = 0x4F47_4455_5041;
/// ...and of the enter command. A different value, so a replayed arm cannot
/// become an enter.
#[cfg_attr(feature = "resident", allow(dead_code))]
pub const MAGIC_ENTER: u64 = 0x4744_454D_5521;

/// The updater, built by `build.rs` from `mcu/fwupdater` and linked to run at
/// [`LOAD_ADDR`]. Not part of a `resident` build: the format-2 updater writes from
/// 0x80000 and would erase the resident (docs/design/resident-app-contract.md N5,
/// owner decision E9), so `build.rs` does not even build it there. S5 brings a
/// format-3 updater back.
#[cfg(not(feature = "resident"))]
static UPDATER: [u8; include_bytes!(concat!(env!("OUT_DIR"), "/fwupdater.bin")).len()] =
    *include_bytes!(concat!(env!("OUT_DIR"), "/fwupdater.bin"));

/// Where the blob must land. Fixed, because it is linked for this address —
/// see `mcu/fwupdater/updater.x`.
///
/// This is the base of SRAM1, which holds only `ide::CD_READ_BUF`. Overwriting
/// it is deliberate: staged sectors are meaningless once an update starts, and
/// SRAM0 has to stay intact because the updater borrows the caller's stack.
#[cfg(not(feature = "resident"))]
const LOAD_ADDR: usize = 0x2008_0000;

/// SRAM1 is 16 KB, and the blob gets all of it.
#[cfg(not(feature = "resident"))]
const SRAM1_LEN: usize = 16 * 1024;
#[cfg(not(feature = "resident"))]
const _: () = assert!(UPDATER.len() <= SRAM1_LEN);

/// Copy the updater into RAM and branch to it. Never returns.
///
/// # Safety
///
/// The caller must have stopped every other user of the MCU first — the
/// executor's other tasks and anything driven by an interrupt — because the
/// updater masks interrupts and then erases the flash their handlers live in.
#[cfg(not(feature = "resident"))]
pub unsafe fn run(base: *mut u16) -> ! {
    core::ptr::copy_nonoverlapping(UPDATER.as_ptr(), LOAD_ADDR as *mut u8, UPDATER.len());
    // The Cortex-M3 has no unified cache to worry about, but the write buffer
    // still has to drain and the pipeline be refilled before those bytes are
    // fetched as instructions.
    cortex_m::asm::dsb();
    cortex_m::asm::isb();

    // Thumb state: the low bit of a branch target selects the instruction
    // set, and `_start` sits at offset 0 of the blob.
    let entry: extern "C" fn(*mut u16) -> ! = core::mem::transmute(LOAD_ADDR | 1);
    entry(base)
}

/// Set by the arm step, required by the enter step. Cleared by anything else
/// on the vendor opcode, so a half-finished sequence does not linger.
#[cfg(not(feature = "resident"))]
static mut ARMED: bool = false;

#[cfg(feature = "resident")]
pub fn handle(features: u8, key: u64, label: &'static str) -> bool {
    // The embedded format-2 updater is not in this build (N5), so no step can
    // be accepted: it would erase the resident. S5 replaces this with a
    // format-3 updater.
    let _ = key;
    warn!("{}: firmware update refused in the resident layout (step {:#04x})", label, features);
    false
}

/// Handle [`CMD_FW`]. Called from the IDE command dispatch for the disk
/// device; `key` is the 48-bit LBA field.
///
/// Returns `true` if the command was accepted, `false` if it should be
/// aborted. The enter step does not return at all.
#[cfg(not(feature = "resident"))]
pub fn handle(features: u8, key: u64, label: &'static str) -> bool {
    let armed = unsafe { ARMED };
    match features {
        FEAT_ARM if key == MAGIC_ARM => {
            unsafe { ARMED = true };
            warn!("{}: firmware update armed ({} byte updater)", label, UPDATER.len());
            true
        }
        other => {
            unsafe { ARMED = false };
            warn!("{}: firmware update step {:#04x} rejected (armed={})", label, other, armed);
            false
        }
    }
}

/// Whether the enter step should be honoured. Split out from [`handle`] so
/// the caller can do its own teardown before the one-way branch.
#[cfg(not(feature = "resident"))]
pub fn should_enter(features: u8, key: u64) -> bool {
    features == FEAT_ENTER && key == MAGIC_ENTER && unsafe { ARMED }
}
