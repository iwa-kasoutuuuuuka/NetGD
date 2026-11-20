//! What the application owes the resident boot region (design:
//! `docs/design/fw-bootloader.md` 2-5; format: `tools/fwimage/FORMAT.md`
//! "Format 3"). Compiled only with the cargo feature `resident`, together
//! with `link/*_resident.x`, which places the application at `APP_BASE`.
//!
//! * `.app_desc`: the 32-byte "NGAP" descriptor at `APP_BASE + 0x100`. Length
//!   and CRC are left 0 here ("not sealed"); `fwimage seal`/`build` fill them.
//! * `.image_crc`: the 4-byte trailer, last in the image; the linker script
//!   pads so that it ends on a 512-byte boundary. Left 0; `fwimage` fills it.
//! * VTOR, written before anything else runs (see [`vtor_pre_init`]).

use core::arch::global_asm;

/// First byte of the application; where the vector table is linked.
pub const APP_BASE: u32 = 0x0008_2000;

/// `SCB->VTOR`. Not taken through `cortex_m::Peripherals`: this runs before
/// `main`, and `Peripherals::take` is spent by `embassy_atsam3::init` later.
const SCB_VTOR: *mut u32 = 0xE000_ED08 as *mut u32;

/// Point VTOR at the application's own vector table.
///
/// cortex-m-rt calls `__pre_init` first thing in `Reset`, before `.data` and
/// `.bss` are touched and before `main`: this is the earliest point the
/// application controls, earlier than anything in `main` or embassy's init
/// (nothing there has enabled an interrupt yet either way, but a fault taken
/// before VTOR is right would be fetched from the wrong table). The resident
/// writes the same value before it jumps; doing it here as well means the
/// application does not depend on that (a probe that starts the image
/// straight at `Reset`, or a resident that forgets). `TBLOFF` is in units of
/// 128 bytes, and 0x82000 is one. Needs no RAM and no stack.
#[cortex_m_rt::pre_init]
unsafe fn vtor_pre_init() {
    core::ptr::write_volatile(SCB_VTOR, APP_BASE);
}

/// Compressed bitstream size, as `main.rs` embeds it (same file).
const BS_LEN: u32 = include_bytes!(concat!(env!("OUT_DIR"), "/bitstream.lzma")).len() as u32;
const BS_RAW_LEN: u32 = crate::BITSTREAM_RAW_LEN as u32;
/// Bitstream kind in the top byte (1 = LZMA, 0 = none), app version below.
const KIND_VERSION: u32 = (((BS_LEN != 0) as u32) << 24) | app_version();

const fn num(s: &str) -> u32 {
    let b = s.as_bytes();
    let (mut i, mut n) = (0, 0u32);
    while i < b.len() {
        n = n * 10 + (b[i] - b'0') as u32;
        i += 1;
    }
    n
}

/// Crate version as `major << 16 | minor << 8 | patch` (24 bits).
const fn app_version() -> u32 {
    ((num(env!("CARGO_PKG_VERSION_MAJOR")) & 0xFF) << 16)
        | ((num(env!("CARGO_PKG_VERSION_MINOR")) & 0xFF) << 8)
        | (num(env!("CARGO_PKG_VERSION_PATCH")) & 0xFF)
}

// `__bitstream_addr` is defined by the linker script as the start of
// `.flash1`: 0x00100000 on a 4E, inside the image on a 2E. The word is 0 when
// the image carries no bitstream (board-v60 without a .rbf), as the format
// says for "SD / none". A relocation, not a Rust constant: a pointer cannot be
// turned into an integer in a static initializer.
global_asm!(
    ".section .app_desc,\"a\",%progbits",
    ".balign 4",
    ".ascii \"NGAP\"",
    ".word 1",              // descriptor version
    ".word 0",              // image length: 0 = not sealed (fwimage)
    ".if {len} != 0",
    ".word __bitstream_addr",
    ".else",
    ".word 0",
    ".endif",
    ".word {len}",          // bitstream length, compressed
    ".word {raw}",          // bitstream length, unpacked
    ".word {kv}",           // kind << 24 | app version
    ".word 0",              // CRC-32 of the 28 bytes before (fwimage)
    ".previous",
    ".section .image_crc,\"a\",%progbits",
    ".balign 4",
    ".word 0",              // CRC-32 of the whole image before it (fwimage)
    ".previous",
    len = const BS_LEN,
    raw = const BS_RAW_LEN,
    kv = const KIND_VERSION,
);
