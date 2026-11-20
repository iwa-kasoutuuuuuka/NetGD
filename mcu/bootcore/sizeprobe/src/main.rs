//! Code-size probe: bootcore instantiated with plausible hardware
//! implementations (memory-mapped flash + EEFC, an external SD block read),
//! one `#[inline(never)]` entry point per job so `llvm-nm --size-sort` shows
//! what each costs. Nothing here has ever run on a board; the register code
//! is only as detailed as needed to be the right size.
#![no_std]
#![no_main]

use bootcore::app::{verify_flash_app, APP_BASE};
use bootcore::boot::{decide, Action};
use bootcore::fat::{BlockRead, NAME_APP};
use bootcore::fcr::{fcr_word_for, Cmd, Plane};
use bootcore::flash::{Flash, FlashError, Poll};
use bootcore::part::Part;
use bootcore::update::sd_recover;
use core::ptr::{read_volatile, write_volatile};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

const EEFC0: usize = 0x400E_0800;
const EEFC1: usize = 0x400E_0A00;

fn efc(p: Plane) -> *mut u32 {
    (if p == Plane::P0 { EEFC0 } else { EEFC1 }) as *mut u32
}

struct Hw;

impl Flash for Hw {
    fn read(&mut self, addr: u32, out: &mut [u8]) {
        for (i, b) in out.iter_mut().enumerate() {
            *b = unsafe { read_volatile((addr as usize + i) as *const u8) };
        }
    }

    // On the board this function and `ewp_poll` run from RAM.
    fn ewp_start(&mut self, plane: Plane, page: u32, data: &[u8; 256]) -> Result<(), FlashError> {
        let w = fcr_word_for(plane, Cmd::Ewp, page).ok_or(FlashError::Refused)?;
        let base = (if plane == Plane::P0 { 0x0008_0000usize } else { 0x0010_0000 }) + page as usize * 256;
        for (i, c) in data.chunks_exact(4).enumerate() {
            let v = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
            unsafe { write_volatile((base as *mut u32).add(i), v) };
        }
        unsafe { write_volatile(efc(plane).add(1), w) };
        Ok(())
    }

    fn ewp_poll(&mut self, ) -> Poll {
        // The probe does not know which plane; the board code keeps it.
        let fsr = unsafe { read_volatile(efc(Plane::P0).add(2)) };
        if fsr & 1 == 0 {
            Poll::Busy
        } else if fsr & 4 != 0 {
            Poll::Done(Err(FlashError::Locked))
        } else if fsr & 2 != 0 {
            Poll::Done(Err(FlashError::Refused))
        } else {
            Poll::Done(Ok(()))
        }
    }

    fn lock_bits(&mut self, plane: Plane) -> u32 {
        if let Some(w) = fcr_word_for(plane, Cmd::Glb, 0) {
            unsafe {
                write_volatile(efc(plane).add(1), w);
                while read_volatile(efc(plane).add(2)) & 1 == 0 {}
                read_volatile(efc(plane).add(3))
            }
        } else {
            !0
        }
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
    let ok = verify_flash_app(&mut Hw, part_of(part), allow_unsealed != 0).is_ok();
    (decide(ok, button != 0, gpbr) == Action::RunApp) as u32
}

/// SD recovery: FAT32, validation, the three passes, EWP.
#[no_mangle]
#[inline(never)]
pub extern "C" fn probe_recover(part: u32, buf: *mut [u8; 512]) -> u32 {
    let buf = unsafe { &mut *buf };
    match sd_recover(&mut Sd, &mut Hw, part_of(part), buf, &NAME_APP) {
        Ok(l) => l,
        Err(_) => 0,
    }
}

#[no_mangle]
#[inline(never)]
pub extern "C" fn probe_crc(p: *const u8, n: usize) -> u32 {
    bootcore::crc::crc32(unsafe { core::slice::from_raw_parts(p, n) })
}

#[no_mangle]
#[inline(never)]
pub extern "C" fn probe_fcr(plane: u32, cmd: u32, arg: u32) -> u32 {
    let c = match cmd {
        0 => Cmd::Ewp,
        1 => Cmd::Glb,
        _ => Cmd::Slb,
    };
    fcr_word_for(if plane == 0 { Plane::P0 } else { Plane::P1 }, c, arg).unwrap_or(0)
}

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    let mut buf = [0u8; 512];
    let mut x = probe_boot(0, 0, 0, 0);
    x ^= probe_recover(x, &mut buf);
    x ^= probe_crc(buf.as_ptr(), 512);
    x ^= probe_fcr(x, x, x);
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
