//! Pure logic of the resident boot region ("B-SD", design option I-B of
//! `docs/design/fw-bootloader.md`). Everything here is register-free:
//! the hardware (HSMCI, EEFC, GPBR, PIO) is reached only through the
//! [`fat::BlockRead`] and [`flash::Flash`] traits, so the whole of it runs in
//! host tests, including a flash model with lock bits and exhaustive power
//! cuts.
//!
//! Code-size rules (the resident must fit 8 KB with startup and HSMCI code):
//! no panics on non-test paths (no indexing that can fail, no `unwrap`, no
//! `expect`, no overflow-checked arithmetic that can trap), no formatting, no
//! floats, no heap, no statics that are written. Memory is one 512-byte
//! buffer supplied by the caller plus small stack arrays.
//!
//! Modules:
//! * [`crc`]    CRC-32 (zlib), 4-bit table
//! * [`part`]   SAM3U2E / SAM3U4E constants, clock and FWS constants
//! * [`app`]    "NGAP" descriptor, vector table and image check (in flash)
//! * [`fcr`]    the one and only producer of `EEFC_FCR` words
//! * [`flash`]  the flash trait and its error type
//! * [`info`]   the resident's information block at 0x81FF0 ("NGRS")
//! * [`fat`]    read-only FAT32 over 512-byte block reads
//! * [`ogfw`]   OGFW format 3 header and streaming image check
//! * [`update`] the SD recovery state machine
//! * [`boot`]   the boot decision, the GPBR1 boot report, the lock procedure

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod app;
pub mod boot;
pub mod crc;
pub mod fat;
pub mod fcr;
pub mod flash;
pub mod info;
pub mod ogfw;
pub mod part;
pub mod update;

/// Little-endian word at `off`, 0 if out of range (no panic path).
#[inline]
pub(crate) fn rd32(b: &[u8], off: usize) -> u32 {
    match b.get(off..off + 4) {
        Some(&[a, b, c, d]) => u32::from_le_bytes([a, b, c, d]),
        _ => 0,
    }
}

#[inline]
pub(crate) fn rd16(b: &[u8], off: usize) -> u32 {
    match b.get(off..off + 2) {
        Some(&[a, b]) => u16::from_le_bytes([a, b]) as u32,
        _ => 0,
    }
}
