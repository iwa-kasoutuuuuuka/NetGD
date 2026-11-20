//! Standalone flash writer for in-field firmware updates.
//!
//! This is not part of the firmware binary. It is built as its own program,
//! linked to run from the base of SRAM1 (see `updater.x`), emitted as a raw
//! binary, and embedded in the firmware image as a byte blob. When a firmware
//! update starts, `opengdemu`'s `fwupdate` shim copies the blob to
//! `0x2008_0000` and branches to offset 0 — [`_start`].
//!
//! # Why a separate binary
//!
//! While the EEFC is programming, its flash plane cannot be read, so an
//! instruction fetched from the plane being written faults. The SAM3U2E has a
//! single plane, so "run from the other bank" is not an option. (The SAM3U
//! datasheet does document a ROM IAP function, section 22.4.4, entry point in
//! the ROM's NMI vector; it is not used here -- driving the EEFC directly
//! from RAM keeps the lock-bit, FSR and wait-state handling visible in this
//! file.) The flash writer therefore executes from RAM.
//!
//! The obvious way to get that — `#[link_section = ".data"]` on each function
//! — works, but only as long as nothing in the module gives the compiler a
//! reason to emit a call to a helper that is still in flash. A `[0u8; N]`
//! initialiser becoming `__aeabi_memclr4` is enough, the linker will happily
//! route it through a veneer, and the failure shows up as a hard fault
//! mid-update rather than a build error.
//!
//! Building it as its own program removes the hazard at the root: this
//! program links its own copy of `compiler_builtins`, so its `memcpy` and
//! friends are inside the blob at RAM addresses. Nothing it can call lives in
//! flash, because flash is not in its address space at all. Ordinary Rust is
//! fine in here.
//!
//! The caller's stack (in SRAM0) is borrowed as-is; SRAM0 is untouched by
//! everything below, which is why the link address is in SRAM1.
//!
//! [`_start`] never returns. The only ways out are a host-commanded reset or
//! the SWD probe.
//!
//! # Protocol
//!
//! Every step is vendor opcode `0xF9` on the disk device, selected by the
//! FEATURES byte. Getting here at all took a 56-bit key (see the firmware's
//! `fwupdate` shim); from this point the sequence is, per image:
//!
//! ```text
//!   F9 / feat 55                header, one 512-byte block (see `logic.rs`)
//!   F9 / feat 56, count = n     n 512-byte payload blocks (count 0 = 256)
//!   F9 / feat 57                verify and commit
//! ```
//!
//! and, once every image is verified:
//!
//! ```text
//!   F9 / feat 58                reset -- refused unless an image was verified
//!                               and nothing is left half-written
//!   F9 / feat 59 + FORCE_KEY    reset regardless (48-bit key in the LBA regs)
//! ```
//!
//! A full 4E update is two images (code to plane 0, bitstream to plane 1); a
//! 2E update is one, because code and bitstream share its only plane.
//!
//! # What keeps a bad update from bricking the board
//!
//! * The header names the part it is for (CHIPID_CIDR, silicon revision
//!   ignored); a mismatch is `BadTarget`. On a part without plane 1 a plane-1
//!   destination is `BadDest`. A code image must start at the plane base, so
//!   a partial overwrite of the code plane cannot be requested.
//! * The first 256 bytes (the vector table) are held in RAM, not written. For
//!   a code image they are checked against *this* part's SRAM before the
//!   first page is erased (`BadVector`).
//! * Pages 1..N are programmed as they arrive. Page 0 is written last, at
//!   verify, after the CRC of the whole image (RAM page 0 + flash pages 1..N)
//!   matched the header; then the whole flash image is read back against the
//!   CRC once more.
//! * Reset is only honoured after a successful verify.
//! * FWS=6 for the whole run (SAM3U errata 46.1.1.3), stale EEFC error flags
//!   are cleared before every command, lock bits over the destination are
//!   read first (`Locked`), and the IDE write FIFO is drained after a
//!   failure so the retry starts clean.
//!
//! Remaining unrecoverable window: from the first page-1.. erase until page 0
//! has been rewritten, the old vector table is still in flash but points at
//! partly overwritten code. A power cut or a host that disappears in that
//! window leaves a board that needs SWD. Closing it needs a resident boot
//! region (LOG R37, countermeasure B).
//!
//! Results come back in the ERROR register with the ERR status bit, since
//! there is no logging in here: see [`Status`].

#![no_std]
#![no_main]

use core::mem::MaybeUninit;
use core::ptr::{read_volatile, write_volatile};

use fwupdater_logic as logic;
use logic::{Header, Part, Reject, KIND_CODE, PAGE, PLANE0_BASE, PLANE1_BASE};

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    // Nothing to report to and nowhere to go: a panic here means a bug in
    // this file. Stop, and leave the host's command un-answered so it times
    // out rather than believing the update succeeded.
    loop {
        core::hint::spin_loop();
    }
}

const CMD_FW: u8 = 0xF9;

const FEAT_HEADER: u8 = 0x55;
const FEAT_DATA: u8 = 0x56;
const FEAT_VERIFY: u8 = 0x57;
const FEAT_RESET: u8 = 0x58;
const FEAT_FORCE_RESET: u8 = 0x59;

/// 48-bit key, in the LBA registers of the forced reset: "OGFRST".
const FORCE_KEY: u64 = 0x4F47_4652_5354;

/// Reported in the ERROR register alongside the ERR status bit.
///
/// The encoding is forced by the hardware: the FPGA does not store a whole
/// ERROR byte. It keeps an ABRT flag and a 4-bit sense key, and rebuilds the
/// host-visible register as `[_, _, ABRT, _, sense_key]` (see `decode.error`
/// in `hw/ide_device.py`), so bits 0, 1 and 3 read back as zero no matter
/// what is written. A dense 0x10..0x16 numbering silently collapsed —
/// 0x13 and 0x11 both arrived as 0x10, 0x16 as 0x14.
///
/// So: ABRT set on every failure, and the reason in the sense-key nibble.
/// Plain 0x04 (ABRT, nibble 0) is what the *normal* firmware returns when it
/// refuses to arm, which keeps "refused to start" distinct from every way an
/// update can fail once it has started.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Ok = 0x00,
    BadHeader = 0x14,
    BadDest = 0x24,
    BadLength = 0x34,
    NoHeader = 0x44,
    Overrun = 0x54,
    FlashError = 0x64,
    CrcMismatch = 0x74,
    /// Header is for a different part (or this part is not a SAM3U2E/4E).
    BadTarget = 0x84,
    /// A lock region under the destination is locked.
    Locked = 0x94,
    /// Initial SP or a handler of the new vector table is not plausible.
    BadVector = 0xA4,
    /// Reset requested before an image was verified (or with one half-written).
    NotVerified = 0xB4,
}

impl From<Reject> for Status {
    fn from(r: Reject) -> Status {
        match r {
            Reject::BadHeader => Status::BadHeader,
            Reject::BadTarget => Status::BadTarget,
            Reject::BadDest => Status::BadDest,
            Reject::BadLength => Status::BadLength,
            Reject::BadVector => Status::BadVector,
        }
    }
}

// --- EEFC ---------------------------------------------------------------

const EFC0: *mut u32 = 0x400E_0800 as *mut u32;
/// Exists on the SAM3U4E only. Never touched on any other part.
const EFC1: *mut u32 = 0x400E_0A00 as *mut u32;
const EEFC_FMR: usize = 0; // 0x00 / 4
const EEFC_FCR: usize = 1; // 0x04 / 4
const EEFC_FSR: usize = 2; // 0x08 / 4
const EEFC_FRR: usize = 3; // 0x0C / 4
const FSR_FRDY: u32 = 1 << 0;
/// FCMDE | FLOCKE: bad command, or the target lock region is locked.
const FSR_ERRORS: u32 = (1 << 1) | (1 << 2);

/// Erase page and write page, in one command. SAM3U still has EWP (SAM4
/// dropped it for a separate erase), which is why there is no erase pass
/// below — but also why a page must be filled completely before committing.
const FCMD_EWP: u32 = 0x03;
/// Get Lock Bit: the lock bits of the plane come back in EEFC_FRR.
const FCMD_GLB: u32 = 0x0A;
const FKEY: u32 = 0x5A;

/// Wait states during programming. Errata 46.1.1.3: with fewer, the data
/// written may not be the data expected. Set once at entry; the board resets
/// at the end, which restores the firmware's own setting.
const PROGRAMMING_FWS: u32 = 6;

/// CHIPID_CIDR. The flash writer takes the part from here, not from a build
/// option, so an image cannot be steered to the wrong part by the firmware
/// that happened to launch it.
const CHIPID_CIDR: *const u32 = 0x400E_0740 as *const u32;

// --- FPGA register file (mirrors the constants in the firmware's ide.rs) --

const REG_HW_INFO: usize = 0;
const REG_STATUS: usize = 1;
const REG_ERROR: usize = 3;
const REG_CMD_FEAT: usize = 4;
const REG_SECTOR_COUNT: usize = 5;
const REG_LBA_LOW_MID: usize = 6;
const REG_LBA_HIGH_LOWP: usize = 7;
const REG_LBA_MIDP_HIGHP: usize = 8;
const REG_DATA_FIFO: usize = 9;
const REG_WRITE_FIFO_LEVEL: usize = 11;
const REG_DMA_MODE: usize = 19;

const HW_CMD_PEND: u16 = 0x0002;

const STATUS_BSY: u8 = 0x80;
const STATUS_DRDY: u8 = 0x40;
const STATUS_DSC: u8 = 0x10;
const STATUS_DRQ: u8 = 0x08;
const STATUS_ERR: u8 = 0x01;
const STATUS_IDLE: u8 = STATUS_DRDY | STATUS_DSC;
const ERR_ABRT: u8 = 0x04;

/// Words per 512-byte block, matching the host's PIO sector size.
const WORDS_PER_BLOCK: u16 = 256;
const WORDS_PER_PAGE: usize = (PAGE / 4) as usize;

/// The FPGA's register window, as a handle so the register pokes read
/// somewhat like the firmware's `IdeDevice`.
#[derive(Clone, Copy)]
struct Fpga(*mut u16);

impl Fpga {
    #[inline]
    fn read(self, reg: usize) -> u16 {
        unsafe { read_volatile(self.0.add(reg)) }
    }

    #[inline]
    fn write(self, reg: usize, val: u16) {
        unsafe { write_volatile(self.0.add(reg), val) }
    }

    /// Post a status and assert INTRQ in the same FPGA clock (bit 8). Split
    /// writes lose interrupts on hosts that poll Alt Status.
    fn set_status_intrq(self, status: u8) {
        self.write(REG_STATUS, status as u16 | 0x0100);
    }

    fn command_pending(self) -> bool {
        self.read(REG_HW_INFO) & HW_CMD_PEND != 0
    }

    /// Clear COMMAND_PEND so the FPGA stops asserting MCU_IRQ and accepts
    /// the next command.
    fn ack_command(self) {
        self.write(REG_HW_INFO, 0);
    }

    fn pop16(self) -> u16 {
        self.read(REG_DATA_FIFO)
    }

    fn pop32(self) -> u32 {
        let lo = self.pop16() as u32;
        let hi = self.pop16() as u32;
        lo | (hi << 16)
    }

    /// The 48-bit LBA of the pending command, assembled the way the
    /// firmware's `CommandSnapshot::lba48` does.
    fn lba48(self) -> u64 {
        let low_mid = self.read(REG_LBA_LOW_MID) as u64;
        let high_lowp = self.read(REG_LBA_HIGH_LOWP) as u64;
        let midp_highp = self.read(REG_LBA_MIDP_HIGHP) as u64;
        (midp_highp << 32) | ((high_lowp >> 8) << 24) | ((high_lowp & 0xFF) << 16) | low_mid
    }

    /// Throw away whatever the host left in the write FIFO. After a failed
    /// block the rest of it (and possibly a whole further block) is still
    /// there, and the next header would otherwise be parsed out of the
    /// middle of stale payload.
    fn discard_fifo(self) {
        // BSY first, so the host stops feeding it while we empty it.
        self.close_block();
        // Bounded: a full FIFO is 512 words.
        for _ in 0..1024 {
            if self.read(REG_WRITE_FIFO_LEVEL) == 0 {
                break;
            }
            let _ = self.pop16();
        }
    }

    /// End a command: drop flow control, post the result, release the latch.
    fn finish(self, status: Status) {
        if status != Status::Ok {
            self.discard_fifo();
        }
        self.write(REG_DMA_MODE, 0);
        let err = status as u8;
        self.write(REG_ERROR, err as u16);
        self.set_status_intrq(if err == 0 {
            STATUS_IDLE
        } else {
            STATUS_IDLE | STATUS_ERR
        });
        self.ack_command();
    }

    /// Arm the FPGA's PIO write backpressure and raise DRQ for the first
    /// block. Bit 2 of REG_DMA_MODE makes the FPGA drop the host-visible DRQ
    /// whenever the write FIFO is full — polling from here is far too slow to
    /// do that in software, and the host would overrun the FIFO.
    fn open_data_out(self) {
        self.write(REG_DMA_MODE, 4);
        self.write(REG_ERROR, 0);
        self.set_status_intrq(STATUS_DRDY | STATUS_DSC | STATUS_DRQ);
        self.ack_command();
    }

    /// Spin until a whole block is in the write FIFO. False if the host gave
    /// up and issued another command instead.
    fn await_block(self) -> bool {
        loop {
            if self.read(REG_WRITE_FIFO_LEVEL) >= WORDS_PER_BLOCK {
                return true;
            }
            if self.command_pending() {
                return false;
            }
        }
    }

    /// DRQ down and BSY up. The host must not start the next block while a
    /// page programs: the FIFO is 512 words and an EWP takes milliseconds.
    fn close_block(self) {
        self.write(REG_STATUS, (STATUS_BSY | STATUS_DRDY | STATUS_DSC) as u16);
    }
}

/// State of the image being received.
#[derive(Default)]
struct Target {
    /// A header was accepted and nothing has failed since.
    active: bool,
    /// Something failed mid-image: only a new header can continue.
    failed: bool,
    kind: u32,
    /// First flash address of the image.
    base: u32,
    /// Total bytes in the image.
    len: u32,
    /// Bytes received so far (page 0 counts once it is held in RAM).
    off: u32,
    crc: u32,
    /// Page 0 has been programmed (verify did it).
    committed: bool,
}

/// What outlives a single image: whether a reset would boot something
/// half-written.
#[derive(Default)]
struct Session {
    /// Per kind: flash was modified and the image has not verified since.
    dirty: [bool; 2],
    /// At least one image has verified.
    verified_any: bool,
}

/// Blob entry point. The firmware branches here with the FPGA register
/// window in r0, having already stopped everything else on the MCU.
#[no_mangle]
#[link_section = ".entry"]
pub extern "C" fn _start(base: *mut u16) -> ! {
    // The vector table lives in the plane about to be erased.
    unsafe { core::arch::asm!("cpsid i", options(nomem, nostack)) };

    let fpga = Fpga(base);
    let part = Part::from_cidr(unsafe { read_volatile(CHIPID_CIDR) });

    // Errata 46.1.1.3. EFC1 only where it exists.
    unsafe {
        write_volatile(EFC0.add(EEFC_FMR), PROGRAMMING_FWS << 8);
        if part == Some(Part::Sam3u4e) {
            write_volatile(EFC1.add(EEFC_FMR), PROGRAMMING_FWS << 8);
        }
    }

    let mut t = Target::default();
    let mut s = Session::default();
    // Page 0 of the image being received, parked in RAM until verify. On the
    // borrowed stack rather than in the blob, so it costs the blob nothing.
    let mut page0 = MaybeUninit::<[u32; WORDS_PER_PAGE]>::uninit();
    let page0 = unsafe { &mut *page0.as_mut_ptr() };

    // Drop any flow control the interrupted command left armed, then report
    // ready so the host can start the sequence.
    fpga.write(REG_DMA_MODE, 0);
    fpga.write(REG_ERROR, 0);
    fpga.set_status_intrq(STATUS_IDLE);
    fpga.ack_command();

    loop {
        while !fpga.command_pending() {}

        let cmd_feat = fpga.read(REG_CMD_FEAT);
        let count = fpga.read(REG_SECTOR_COUNT) & 0xFF;

        if (cmd_feat & 0xFF) as u8 != CMD_FW {
            fpga.write(REG_DMA_MODE, 0);
            fpga.write(REG_ERROR, ERR_ABRT as u16);
            fpga.set_status_intrq(STATUS_IDLE | STATUS_ERR);
            fpga.ack_command();
            continue;
        }

        match (cmd_feat >> 8) as u8 {
            FEAT_HEADER => {
                let r = recv_header(fpga, part, &mut t);
                fpga.finish(r);
            }
            FEAT_DATA => {
                // Sector count 0 means 256, as everywhere else in ATA.
                let n = if count == 0 { 256 } else { count as u32 };
                let r = recv_data(fpga, part, &mut t, &mut s, page0, n);
                fpga.finish(r);
            }
            FEAT_VERIFY => {
                let r = verify(&mut t, &mut s, page0);
                fpga.finish(r);
            }
            FEAT_RESET => {
                if s.verified_any && !s.dirty[0] && !s.dirty[1] {
                    fpga.finish(Status::Ok);
                    reset();
                }
                fpga.finish(Status::NotVerified);
            }
            FEAT_FORCE_RESET => {
                if fpga.lba48() == FORCE_KEY {
                    fpga.finish(Status::Ok);
                    reset();
                }
                fpga.finish(Status::NotVerified);
            }
            _ => {
                fpga.write(REG_DMA_MODE, 0);
                fpga.write(REG_ERROR, ERR_ABRT as u16);
                fpga.set_status_intrq(STATUS_IDLE | STATUS_ERR);
                fpga.ack_command();
            }
        }
    }
}

fn reset() -> ! {
    // AIRCR: VECTKEY | SYSRESETREQ.
    unsafe { write_volatile(0xE000_ED0C as *mut u32, 0x05FA_0004) };
    loop {
        core::hint::spin_loop();
    }
}

/// Accept the 512-byte header block and set up `t` for the payload.
fn recv_header(fpga: Fpga, part: Option<Part>, t: &mut Target) -> Status {
    *t = Target::default();

    fpga.open_data_out();
    if !fpga.await_block() {
        return Status::Overrun;
    }
    fpga.close_block();

    let mut w = [0u32; logic::HEADER_WORDS];
    for x in w.iter_mut() {
        *x = fpga.pop32();
    }
    // Drain the rest of the block: the FIFO must be empty for the next one.
    for _ in (logic::HEADER_WORDS * 2)..WORDS_PER_BLOCK as usize {
        let _ = fpga.pop16();
    }

    let h: Header = match logic::parse_header(&w) {
        Ok(h) => h,
        Err(e) => return e.into(),
    };
    // Not one of the two supported parts: nothing is ever written.
    let Some(part) = part else {
        return Status::BadTarget;
    };
    if let Err(e) = logic::check_header(&h, part) {
        return e.into();
    }
    if locked(h.dest, h.len) {
        return Status::Locked;
    }

    t.active = true;
    t.kind = h.kind;
    t.base = h.dest;
    t.len = h.len;
    t.crc = h.crc;
    Status::Ok
}

/// Receive `n` 512-byte blocks and program each as two flash pages, except
/// page 0 of the image, which is held in RAM until verify.
fn recv_data(
    fpga: Fpga,
    part: Option<Part>,
    t: &mut Target,
    s: &mut Session,
    page0: &mut [u32; WORDS_PER_PAGE],
    n: u32,
) -> Status {
    if !t.active {
        return Status::NoHeader;
    }
    let Some(part) = part else {
        return Status::BadTarget;
    };
    if t.failed {
        // Half-written already: only a fresh header gets past this.
        return Status::FlashError;
    }
    if n.saturating_mul(512) > t.len - t.off {
        return Status::Overrun;
    }

    fpga.open_data_out();
    for block in 0..n {
        if !fpga.await_block() {
            t.failed = true;
            return Status::Overrun;
        }
        fpga.close_block();

        // Two pages per block, popped straight from the FIFO into the flash
        // latch buffer. There is no intermediate copy to get out of step.
        for _ in 0..2 {
            if t.off == 0 {
                for w in page0.iter_mut() {
                    *w = fpga.pop32();
                }
                if t.kind == KIND_CODE {
                    let v = [page0[0], page0[1], page0[2], page0[3]];
                    if let Err(e) = logic::check_vectors(&v, t.base, t.len, part) {
                        // Nothing has been erased or written for this image.
                        t.failed = true;
                        return e.into();
                    }
                }
            } else {
                // From here on flash is being changed: a reset now would
                // boot a mixture, until verify has run.
                s.dirty[t.kind as usize] = true;
                let addr = t.base + t.off;
                let latch = addr as *mut u32;
                for w in 0..WORDS_PER_PAGE {
                    unsafe { write_volatile(latch.add(w), fpga.pop32()) };
                }
                if !program_page(addr) {
                    t.failed = true;
                    return Status::FlashError;
                }
            }
            t.off += PAGE;
        }

        if block + 1 < n {
            // Ask for the next one.
            fpga.set_status_intrq(STATUS_DRDY | STATUS_DSC | STATUS_DRQ);
        }
    }

    fpga.write(REG_DMA_MODE, 0);
    Status::Ok
}

/// Check the whole image, then write page 0 and check it again.
fn verify(t: &mut Target, s: &mut Session, page0: &[u32; WORDS_PER_PAGE]) -> Status {
    if !t.active {
        return Status::NoHeader;
    }
    if t.failed {
        return Status::FlashError;
    }
    if t.off != t.len {
        return Status::BadLength;
    }

    if !t.committed {
        // Page 0 still lives in RAM: CRC it from there, then pages 1..N from
        // flash. Only if the image as received is good does the old page 0
        // get replaced.
        let mut c = logic::CRC_INIT;
        for &w in page0.iter() {
            c = logic::crc_word(c, w);
        }
        c = crc_flash(c, t.base + PAGE, t.len - PAGE);
        if logic::crc_finish(c) != t.crc {
            return Status::CrcMismatch;
        }

        let latch = t.base as *mut u32;
        for (i, &w) in page0.iter().enumerate() {
            unsafe { write_volatile(latch.add(i), w) };
        }
        s.dirty[t.kind as usize] = true;
        if !program_page(t.base) {
            t.failed = true;
            return Status::FlashError;
        }
        t.committed = true;
    }

    // Everything as it now sits in flash.
    let c = crc_flash(logic::CRC_INIT, t.base, t.len);
    if logic::crc_finish(c) != t.crc {
        return Status::CrcMismatch;
    }
    s.dirty[t.kind as usize] = false;
    s.verified_any = true;
    Status::Ok
}

/// EEFC and plane base for a flash address.
fn efc_of(addr: u32) -> (*mut u32, u32) {
    if addr >= PLANE1_BASE {
        (EFC1, PLANE1_BASE)
    } else {
        (EFC0, PLANE0_BASE)
    }
}

/// Wait for FRDY, returning false if an error flag came up first.
fn wait_ready(efc: *mut u32) -> bool {
    loop {
        let fsr = unsafe { read_volatile(efc.add(EEFC_FSR)) };
        if fsr & FSR_ERRORS != 0 {
            return false;
        }
        if fsr & FSR_FRDY != 0 {
            return true;
        }
    }
}

/// Issue one command. Stale FCMDE/FLOCKE from earlier are cleared by a read
/// of FSR first, so an error seen afterwards is this command's.
fn command(efc: *mut u32, fcr: u32) -> bool {
    unsafe {
        let _ = read_volatile(efc.add(EEFC_FSR));
        write_volatile(efc.add(EEFC_FCR), (FKEY << 24) | fcr);
    }
    wait_ready(efc)
}

/// Commit the latch buffer to the page containing `addr`.
fn program_page(addr: u32) -> bool {
    let (efc, plane) = efc_of(addr);
    // FARG is the page index within the plane; each EEFC numbers from 0.
    let page = (addr - plane) / PAGE;
    command(efc, ((page & 0xFFFF) << 8) | FCMD_EWP)
}

/// Is any lock region under `[addr, addr + len)` locked? A failed lock-bit
/// read counts as locked.
fn locked(addr: u32, len: u32) -> bool {
    let (efc, _) = efc_of(addr);
    if !command(efc, FCMD_GLB) {
        return true;
    }
    let bits = unsafe { read_volatile(efc.add(EEFC_FRR)) };
    bits & logic::lock_mask(len) != 0
}

/// Continue a CRC over `len` bytes of flash at `addr`.
fn crc_flash(mut crc: u32, addr: u32, len: u32) -> u32 {
    let mut a = addr;
    let end = addr + len;
    while a < end {
        crc = logic::crc_word(crc, unsafe { read_volatile(a as *const u32) });
        a += 4;
    }
    crc
}
