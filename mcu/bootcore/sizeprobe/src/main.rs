//! Code-size probe: bootcore instantiated with plausible hardware
//! implementations (memory-mapped flash + EEFC, an external SD block read),
//! one `#[inline(never)]` entry point per job so `llvm-nm --size-sort` shows
//! what each costs. Nothing here has ever run on a board; the register code
//! is only as detailed as needed to be the right size.
#![no_std]
#![no_main]

use bootcore::app::{verify_flash_app, APP_BASE};
use bootcore::boot::{decide, lock_resident, report, Action, LockOutcome, Request, Trigger};
use bootcore::fat::{BlockRead, NAME_APP};
use bootcore::fcr::{Cmd, FcrCmd, Plane};
use bootcore::flash::{Flash, FlashError, PAGE};
use bootcore::part::Part;
use bootcore::update::sd_recover_attempt;
use core::ptr::{read_volatile, write_volatile};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

/// Hardware flash of one part. The register block and the page address come
/// out of the `FcrCmd`; only `lock_bits` needs the part to build a GLB.
struct Hw(Part);

impl Flash for Hw {
    fn set_fws(&mut self, fws: u8) {
        unsafe {
            write_volatile(bootcore::fcr::EEFC0_BASE as *mut u32, (fws as u32) << 8);
            if self.0.has_plane1() {
                write_volatile(bootcore::fcr::EEFC1_BASE as *mut u32, (fws as u32) << 8);
            }
        }
    }

    // On the board: RAM routine, FSR read, FCR write, poll FSR with a
    // timeout. Flash stays readable during SLB, so a timeout may return.
    fn slb(&mut self, cmd: FcrCmd) -> Result<(), FlashError> {
        match run(cmd) {
            Some(fsr) => flags(fsr),
            None => Err(FlashError::Timeout),
        }
    }

    fn read(&mut self, addr: u32, out: &mut [u8]) {
        for (i, b) in out.iter_mut().enumerate() {
            *b = unsafe { read_volatile((addr as usize + i) as *const u8) };
        }
    }

    // On the board this whole function is one RAM routine: latch with 32-bit
    // writes, FSR read, FCR, poll FSR with a timeout, FLOCKE/FCMDE from the
    // same read that saw FRDY. **On a timeout it does not return** (the
    // caller is in the plane that is still busy): it resets the chip.
    fn ewp(&mut self, cmd: FcrCmd, data: &[u8; PAGE]) -> Result<(), FlashError> {
        let latch = cmd.page_addr() as *mut u32;
        for (i, c) in data.chunks_exact(4).enumerate() {
            let v = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
            unsafe { write_volatile(latch.add(i), v) };
        }
        match run(cmd) {
            Some(fsr) => flags(fsr),
            None => reset_from_ram(),
        }
    }

    // FSR is read before the FCR write (in `run`), and FCMDE of the read
    // that saw FRDY means FRR holds nothing: `None`, not "all locked".
    fn lock_bits(&mut self, plane: Plane) -> Option<u32> {
        let c = FcrCmd::glb(self.0, plane)?;
        let fsr = run(c)?;
        if fsr & FSR_FCMDE != 0 {
            return None;
        }
        Some(unsafe { read_volatile((c.eefc_base() + 0xC) as *const u32) })
    }
}

const FSR_FRDY: u32 = 1;
const FSR_FCMDE: u32 = 2;
const FSR_FLOCKE: u32 = 4;

/// One command: read FSR (FCMDE and FLOCKE clear on read, so what is seen
/// afterwards is this command's), write FCR, wait for FRDY. The FSR value of
/// the read that saw FRDY, or `None` on a timeout. One copy, as on the board
/// (the resident has one RAM routine that writes FCR).
#[inline(never)]
fn run(cmd: FcrCmd) -> Option<u32> {
    unsafe {
        let _ = read_volatile(cmd.fsr_addr() as *const u32);
        write_volatile(cmd.fcr_addr() as *mut u32, cmd.word());
    }
    let mut n = 0u32;
    loop {
        let fsr = unsafe { read_volatile(cmd.fsr_addr() as *const u32) };
        if fsr & FSR_FRDY != 0 {
            return Some(fsr);
        }
        n += 1;
        if n > 4_000_000 {
            return None;
        }
    }
}

#[inline(always)]
fn flags(fsr: u32) -> Result<(), FlashError> {
    if fsr & FSR_FLOCKE != 0 {
        Err(FlashError::Locked)
    } else if fsr & FSR_FCMDE != 0 {
        Err(FlashError::Refused)
    } else {
        Ok(())
    }
}

/// `RSTC_CR` = key | PERRST | PROCRST. On the board this is part of the RAM
/// routine: nothing here may be fetched from flash.
#[inline(always)]
fn reset_from_ram() -> ! {
    loop {
        unsafe { write_volatile(0x400E_1200 as *mut u32, 0xA500_0005) };
    }
}

/// Stand-in for the HSMCI polling reader (counted separately: not bootcore).
#[no_mangle]
#[inline(never)]
pub extern "C" fn sd_read_block(lba: u32, buf: *mut u8) -> i32 {
    unsafe {
        write_volatile(0x4000_0000 as *mut u32, lba);
        for i in 0..512 {
            *buf.add(i) = read_volatile(0x4000_0004 as *const u8);
        }
    }
    0
}

struct Sd;

impl BlockRead for Sd {
    fn read_block(&mut self, lba: u32, buf: &mut [u8; 512]) -> Result<(), ()> {
        if unsafe { sd_read_block(lba, buf.as_mut_ptr()) } == 0 {
            Ok(())
        } else {
            Err(())
        }
    }
}

fn part_of(p: u32) -> Part {
    if p == 0 {
        Part::Sam3u2e
    } else {
        Part::Sam3u4e
    }
}

/// Boot path: descriptor, vectors, CRC of the image, decision.
#[no_mangle]
#[inline(never)]
pub extern "C" fn probe_boot(part: u32, allow_unsealed: u32, button: u32, gpbr: u32) -> u32 {
    let ok = verify_flash_app(&mut Hw(part_of(part)), part_of(part), allow_unsealed != 0).is_ok();
    (decide(ok, button != 0, gpbr) == Action::RunApp { request: Request::None }) as u32
}

/// SD recovery: FAT32, validation, the three passes, EWP.
#[no_mangle]
#[inline(never)]
pub extern "C" fn probe_recover(part: u32, buf: *mut [u8; 512]) -> u32 {
    let buf = unsafe { &mut *buf };
    // the report word this boot would leave in GPBR1 (code size of the
    // report and of the error mapping included)
    let t = Trigger { button: part & 2 != 0, request: part & 4 != 0 };
    let p = part_of(part & 1);
    let lock = LockOutcome { locked: true, slb: false, unknown: part & 8 != 0 };
    let a = sd_recover_attempt(&mut Sd, &mut Hw(p), p, buf, &NAME_APP);
    let w = match a.result {
        Ok(r) => report(t, r.wrote, lock, false, 1) ^ r.len,
        Err(e) => report(t, a.wrote, lock, false, e.result_code()),
    };
    unsafe { write_volatile(0x400E_1294 as *mut u32, w) };
    w
}

/// Design 4-1: GLB, SLB if unlocked, GLB; the outcome for the report.
#[no_mangle]
#[inline(never)]
pub extern "C" fn probe_lock(part: u32, dev: u32) -> u32 {
    let o = lock_resident(&mut Hw(part_of(part)), part_of(part), dev != 0);
    o.locked as u32 | (o.slb as u32) << 1 | (o.unknown as u32) << 2
}

#[no_mangle]
#[inline(never)]
pub extern "C" fn probe_crc(p: *const u8, n: usize) -> u32 {
    bootcore::crc::crc32(unsafe { core::slice::from_raw_parts(p, n) })
}

#[no_mangle]
#[inline(never)]
pub extern "C" fn probe_fcr(part: u32, plane: u32, cmd: u32, arg: u32) -> u32 {
    let c = match cmd {
        0 => Cmd::Ewp,
        1 => Cmd::Glb,
        _ => Cmd::Slb,
    };
    let p = if plane == 0 { Plane::P0 } else { Plane::P1 };
    match FcrCmd::new(part_of(part), p, c, arg, true) {
        Some(c) => c.word() ^ c.fcr_addr(),
        None => 0,
    }
}

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    let mut buf = [0u8; 512];
    let mut x = probe_boot(0, 0, 0, 0);
    x ^= probe_recover(x, &mut buf);
    x ^= probe_lock(x, x);
    x ^= probe_crc(buf.as_ptr(), 512);
    x ^= probe_fcr(x, x, x, x);
    let _ = APP_BASE;
    loop {
        unsafe { write_volatile(0x2000_3000 as *mut u32, x) };
    }
}

#[repr(C)]
struct Vt {
    sp: u32,
    reset: extern "C" fn() -> !,
}

#[link_section = ".vector_table"]
#[used]
static VT: Vt = Vt { sp: 0x2000_4000, reset: Reset };
