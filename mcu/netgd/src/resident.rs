//! What the application owes the resident boot region (design:
//! `docs/design/fw-bootloader.md` 2-5; format: `tools/fwimage/FORMAT.md`
//! "Format 3"). Compiled only with the cargo feature `resident`, together
//! with `link/*_resident.x`, which places the application at `APP_BASE`.
//!
//! * `.app_desc`: the 32-byte "NGAP" descriptor at `APP_BASE + 0x100`. Length
//!   and CRC are left 0 here ("not sealed"); `fwimage seal`/`build` fill them.
//! * `.image_crc`: the 4-byte trailer, last in the image; the linker script
//!   pads so that it ends on a 512-byte boundary. Left 0; `fwimage` fills it.
//! * `.app_ax`: the 32-byte "NGAX" extension block at `APP_BASE + 0x120`
//!   (`min_gen`, and on a 4E the `bs_stamp` that `fwimage` fills in).
//! * `.bs_crc` (4E): the 4-byte CRC that ends the bitstream image in plane 1.
//! * VTOR, written before anything else runs (see [`vtor_pre_init`]).
//! * The hand-off contract (`docs/design/resident-app-contract.md`): the
//!   resident's report and information block, and the recovery request.

use core::arch::global_asm;

/// First byte of the application; where the vector table is linked.
pub const APP_BASE: u32 = 0x0008_2000;
const _: () = assert!(APP_BASE == bootcore::app::APP_BASE);

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

// ---------------------------------------------------------------------------
// The hand-off contract (docs/design/resident-app-contract.md, generation 1).
//
// The wire formats (GPBR1 report, information block, "NGAX" fields) live in
// `bootcore`, shared with the resident and `fwimage`; what is here is the
// application's side of them. They are frozen with the resident: change one
// and the app no longer understands the resident already locked into a board.
// ---------------------------------------------------------------------------

use bootcore::boot::{self, Report, GPBR_RECOVER_MAGIC};
use bootcore::info::{self, Info, IFACE_GEN, INFO_ADDR, INFO_F_DEV, INFO_LEN};

/// The lowest resident interface generation this application works with,
/// published in the "NGAX" block as `min_gen` (contract 3-5). Below it the app
/// still starts; it only leaves out what needs the newer resident.
pub const MIN_GEN: u8 = 1;

/// `SYS_GPBR0`: the app's request to the resident (contract 3-1).
const SYS_GPBR0: *mut u32 = 0x400E_1290 as *mut u32;
/// `SYS_GPBR1`: the resident's report to the app (contract 3-2).
const SYS_GPBR1: *const u32 = 0x400E_1294 as *const u32;
/// `RSTC_CR`, and the key | PROCRST | PERRST word (contract X7: never
/// `SYSRESETREQ`, whose effect on the peripherals is not documented).
const RSTC_CR: *mut u32 = 0x400E_1200 as *mut u32;
const RSTC_PROC_PERI_RESET: u32 = 0xA500_0005;

/// The resident's report for this boot, if there was a resident (GPBR1 holds
/// the mark; after power-up or a probe start it holds anything).
pub fn boot_report() -> Option<Report> {
    Report::parse(unsafe { core::ptr::read_volatile(SYS_GPBR1) })
}

/// Contract rule X6: may this boot ask for a recovery? Not when there is no
/// resident to answer (no report: the request would reset the chip into the
/// same application, forever), and not when this boot already is the result
/// of such a request (a recovery that did not fix the problem would loop),
/// or follows one that a reset cut short (the resident found its "in
/// progress" word in GPBR0 and reports `REQ` with result 10).
/// All are `bootcore::boot::request_allowed`.
pub fn request_allowed(report: Option<Report>) -> bool {
    boot::request_allowed(report)
}

/// Why [`request_recovery`] returned.
#[derive(Clone, Copy, Debug, PartialEq, Eq, defmt::Format)]
pub struct RequestRefused;

/// Ask the resident for an SD recovery and reset (contract 3-1). Returns only
/// when the request is not allowed ([`request_allowed`]); otherwise it never
/// returns. Interrupts stay off from the GPBR0 write until the reset takes
/// effect (X5).
#[allow(dead_code)] // the only caller is the 4E bitstream-stamp check
pub fn request_recovery() -> Result<core::convert::Infallible, RequestRefused> {
    if !request_allowed(boot_report()) {
        return Err(RequestRefused);
    }
    cortex_m::interrupt::disable();
    unsafe {
        core::ptr::write_volatile(SYS_GPBR0, GPBR_RECOVER_MAGIC);
        core::ptr::write_volatile(RSTC_CR, RSTC_PROC_PERI_RESET);
    }
    loop {
        cortex_m::asm::wfi();
    }
}

/// The resident's information block, read as ordinary flash (no EEFC).
pub fn resident_info() -> Option<Info> {
    let p = INFO_ADDR as *const u32;
    let mut b = [0u8; INFO_LEN];
    for i in 0..INFO_LEN / 4 {
        let w = unsafe { core::ptr::read_volatile(p.add(i)) };
        b[4 * i..4 * i + 4].copy_from_slice(&w.to_le_bytes());
    }
    info::resident_info(&b)
}

/// The information block's first two words' interesting half: the interface
/// generation in bits 0..8 and the flags in bits 8..16, or 0 without an
/// information block. Goes into the OGDM version reply's reserved bytes 6
/// and 7.
///
/// Not built on [`resident_info`] and kept to a few instructions: on a
/// SAM3U2E the image has no room to spare (see [`bootloader_version_word`]).
pub fn ogdm_gen_flags() -> u16 {
    let p = INFO_ADDR as *const u32;
    let (magic, w1) = unsafe { (core::ptr::read_volatile(p), core::ptr::read_volatile(p.add(1))) };
    if magic != info::INFO_MAGIC {
        return 0;
    }
    w1 as u16
}

/// The first word of the `52 00` reply: the resident's version in the packed
/// BCD GDEMU uses (`discimg::gdemu::version_word`), or 0 -- "0.0.0", what the
/// reply always said -- without an information block.
///
/// **Not built for a SAM3U2E**, and there the word stays 0: a 2E image of the
/// resident layout has about 100 bytes free before the 0x1E000 limit, and this
/// is 80 of them. The BCD is `x + x / 10 * 6` per part, right for parts below
/// 100 (a bigger one spills into its neighbour: that is what saves the bytes).
#[cfg(not(feature = "sam3u2e"))]
pub fn bootloader_version_word() -> u32 {
    let p = INFO_ADDR as *const u32;
    let (magic, v) = unsafe { (core::ptr::read_volatile(p), core::ptr::read_volatile(p.add(2))) };
    if magic != info::INFO_MAGIC {
        return 0;
    }
    let mut word = 0u32;
    let mut s = 16;
    loop {
        let x = (v >> s) & 0xFF;
        word = word << 8 | (x + x / 10 * 6);
        if s == 0 {
            return word << 8;
        }
        s -= 8;
    }
}

/// True when the build discards `defmt` output (`DEFMT_LOG=off`). Then
/// [`log_boot`] has nothing to do, and its reads of GPBR and flash would be
/// code for nothing: that matters on the SAM3U2E, which has no room.
const LOG_OFF: bool = match option_env!("DEFMT_LOG") {
    Some(s) => {
        let b = s.as_bytes();
        b.len() == 3 && b[0] == b'o' && b[1] == b'f' && b[2] == b'f'
    }
    None => false,
};

/// Log what the resident said about this boot and what it is (contract 6-1,
/// 3-3, 3-4). Changes no behaviour.
pub fn log_boot() {
    if LOG_OFF {
        return;
    }
    match resident_info() {
        Some(i) => {
            defmt::info!(
                "resident: gen {} v{}.{}.{} flags {:#04x} crc {:#010x}",
                i.gen, (i.version >> 16) as u8, (i.version >> 8) as u8, i.version as u8, i.flags, i.crc
            );
            if i.gen < MIN_GEN {
                defmt::warn!("resident generation {} is older than the {} this application needs", i.gen, MIN_GEN);
            } else if i.gen > IFACE_GEN {
                defmt::info!("resident generation {} is newer than this application's {}; assuming generation 1 still holds", i.gen, IFACE_GEN);
            }
            if i.flags & INFO_F_DEV != 0 {
                defmt::warn!("resident is a DEVELOPMENT build (unsealed apps accepted, region not locked)");
            }
        }
        None => defmt::info!("resident: no information block (version unknown)"),
    }
    match boot_report() {
        Some(r) => {
            defmt::info!(
                "boot report: result {} button {} request {} wrote {} locked {} slb {} unsealed {} lock_unknown {}",
                r.result, r.trigger.button, r.trigger.request, r.wrote, r.locked, r.slb, r.unsealed, r.lock_unknown
            );
            if r.lock_unknown {
                // the resident's GLB failed: `locked` says nothing (contract 3-2)
                defmt::warn!("resident region: lock state unknown (the lock bits could not be read)");
            } else if !r.locked {
                defmt::warn!("resident region is not locked");
            }
            if r.result == boot::RESULT_INTERRUPTED {
                defmt::warn!("a requested recovery was interrupted by a reset; not asking again in this run");
            } else if r.result >= 2 {
                defmt::warn!("the resident's recovery failed (code {}); this is the previous application", r.result);
            }
        }
        None => defmt::info!("boot report: none (not started by the resident)"),
    }
}

/// 4E: does the bitstream in plane 1 belong to this code? (contract 4-3, V3)
///
/// `bs_stamp` in the "NGAX" block is the CRC-32 `fwimage` computed over the
/// bitstream image without its last four bytes; those four bytes (`.bs_crc`)
/// hold the same value. Comparing the two words costs nothing at boot and
/// catches code that was updated without its bitstream or the other way
/// round -- the case the FPGA's own CRC cannot see, since an old bitstream is
/// a valid one. A stamp of 0 (an unsealed or probe-flashed image) checks
/// nothing.
#[cfg(not(feature = "sam3u2e"))]
pub fn bitstream_stamp_ok() -> bool {
    extern "C" {
        /// Address of `.bs_crc` (link/sam3u4e_resident.x).
        static __bs_crc: u32;
    }
    let stamp = unsafe { core::ptr::read_volatile((APP_BASE + bootcore::app::AX_OFF + 12) as *const u32) };
    if stamp == 0 || BS_LEN == 0 {
        return true;
    }
    unsafe { core::ptr::read_volatile(core::ptr::addr_of!(__bs_crc)) == stamp }
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
    // The extension block (contract 3-5): fields the resident never reads,
    // because the descriptor's version is frozen at 1. 32 bytes at +0x120.
    ".section .app_ax,\"a\",%progbits",
    ".balign 4",
    ".ascii \"NGAX\"",
    ".word 32",             // block length
    ".word {min_gen}",      // lowest resident generation this app needs
    ".word 0",              // bs_stamp: CRC-32 of the bitstream image (fwimage; 4E)
    ".word 0",              // bs_iface: expected FPGA interface word (0 = unchecked)
    ".word 0",              // reserved
    ".word 0",              // reserved
    ".word 0",              // reserved (the trailer covers this block: no CRC of its own)
    ".previous",
    ".section .image_crc,\"a\",%progbits",
    ".balign 4",
    ".word 0",              // CRC-32 of the whole image before it (fwimage)
    ".previous",
    len = const BS_LEN,
    raw = const BS_RAW_LEN,
    kv = const KIND_VERSION,
    min_gen = const MIN_GEN as u32,
);

// 4E (contract 4-3, V3): the bitstream image ends with its own CRC-32, which
// `fwimage` fills in and copies into `bs_stamp` above. The linker script pads
// plane 1 so that this word ends the image on a 512-byte boundary.
#[cfg(not(feature = "sam3u2e"))]
global_asm!(
    ".section .bs_crc,\"a\",%progbits",
    ".balign 4",
    ".word 0",              // CRC-32 of plane 1 before it (fwimage)
    ".previous",
);
