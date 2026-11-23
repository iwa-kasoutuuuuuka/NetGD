//! `pinfind` feature: the MCU side of the AG16K pin-finder (GDEMU 6.0).
//!
//! `docs/design/g1-ag16k-port.md` 2-4 method 2, `fpga/pinfind/README_ja.md`.
//! The pin-finder bitstream (fpga/pinfind) drives one FPGA ball at a time and
//! sends that ball's index on it. This firmware loads it over the normal
//! passive-serial loader, turns every MCU pin it can into an input with
//! pull-up (the state the SAM3U leaves them in after reset), and listens: an
//! index arriving on PB9 means "ball <index> is wired to PB9". Results go out
//! over RTT (defmt), in lines `tools/pinfind/pins2asf.py` reads:
//!
//! ```text
//! PF new pin=PB9 idx=37              first frame of ball 37 on PB9
//! PF hit pin=PB9 idx=37 n=12         summary (cumulative, every 10 s and at the end)
//! PF act pin=PA21 edges=1234         toggled, but never a frame
//! PF clk c=3 cnt=96 unit=1536 div=10 win=6 hz=48000000   clock probe 3
//! PF unit=1536                       tick length in CPU cycles (oscillator speed)
//! PF diff port=B changed=0x00000200 before=.. after=..   a pin changed across configuration
//! PF end                             FPGA held in reset (nCONFIG low)
//! ```
//!
//! Nothing here is used by the normal firmware; the feature is off by default.
//! Not run on hardware.
//!
//! Safety, MCU side (LOG R143; order in `psload::guarded`):
//! * Until every software check has passed (GPBR3, bitstream present, full
//!   decode, length, CRC) PA23, PA24 and PA25 are inputs. `McuPins::new`,
//!   which would drive them, is not called in this build.
//! * PA25 (nCONFIG) only ever sinks: its output latch stays 0 and it moves by
//!   OER / ODR, so the MCU cannot drive it high whatever the FPGA's I/O
//!   voltage. Released, a pull-up raises it (nCONFIG is a plain input of the
//!   FPGA, and the internal pull-up is on).
//! * DCLK / DATA0 become outputs only after nSTATUS has followed nCONFIG down
//!   and back up and CONF_DONE reads low; afterwards, or on any failure, they
//!   go back to input + pull-up (the reset state), before nCONFIG is touched.
//!   With a `ball:PIN_G1` clock PA24 stays an output after a good load.
//! * Final state of nCONFIG on failure: held low once PB2 has followed PA25
//!   or a clock may have gone out (user I/O off, nothing half-configured can
//!   drive). Left an input if the handshake never proved PA25 is nCONFIG: no
//!   clock was sent, and an unproven line is not sunk until power-off.
//! * A failed load is not retried; a SysTick hook ends the load or the run at
//!   its time limit; panic, HardFault and the default handler release the
//!   lines the same way.
//! * One run per power-up: a flag in GPBR3 (kept across resets, cleared when
//!   the backup domain loses power), set before the first drive, makes a
//!   second boot refuse. Clear it by removing power, or over SWD:
//!   `probe-rs write --chip ATSAM3U2E b32 0x400E129C 0`.
//! * Not covered: a debugger halting the core; and an MCU reset while the
//!   FPGA is configured. The refused boot may not drive PA25, so the FPGA
//!   stays configured: the log says so when CONF_DONE reads high. Power off.

use crate::cyclone;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use cortex_m::peripheral::DWT;
use defmt::{error, info, warn};
use embassy_atsam3::pac;
use embassy_time::Instant;
use pinfind::{Decoder, Frame, Tally};
use psload::guarded::{self, GuardedError, GuardedPsPins, PrecheckError};
use psload::{PsPins, PsSource};

mod params {
    include!(concat!(env!("OUT_DIR"), "/pinfind_params.rs"));
}

/// Lines left alone: the PS loader's five (PA23 DATA0, PA24 DCLK, PA25
/// nCONFIG; PB1 CONF_DONE, PB2 nSTATUS -- `cyclone::pinmap`). The
/// configuration balls are never scanned, so nothing would arrive there.
const SKIP: [u32; 3] = [(1 << 23) | (1 << 24) | (1 << 25), (1 << 1) | (1 << 2), 0];
/// Watched lines per port.
const MASK: [u32; 3] = [!SKIP[0], !SKIP[1], !SKIP[2]];
const NPINS: usize = 96;
const NCONFIG_BIT: u32 = 1 << 25; // PA25
const DATA0_BIT: u32 = 1 << 23; // PA23
const DCLK_BIT: u32 = 1 << 24; // PA24
const CONF_DONE_BIT: u32 = 1 << 1; // PB1
const NSTATUS_BIT: u32 = 1 << 2; // PB2

/// Wait this long after boot, every PS line an input, before loading: time to attach the logic analyser and the external
/// pull-ups to a powered board (README_ja.md section 5).
const PRE_LOAD_WAIT_S: u64 = 10;
/// The configuration of the FPGA may take this long (estimated 1-3 s,
/// ag16k-config.md 4-3); past it the tick hook drives nCONFIG low.
const LOAD_LIMIT_MS: u32 = 30_000;
/// Stop scanning after this long whatever happens.
const RUN_LIMIT_S: u64 = 120;
/// Stop this long after the last frame on any pin.
const QUIET_END_S: u64 = 30;
/// Stage 0: keep the (output-free) design configured this long, for the
/// current reading, then nCONFIG low.
const STAGE0_HOLD_S: u64 = 20;
/// The tick hook's own limit lies this far beyond the loop's.
const HOOK_MARGIN_MS: u32 = 3_000;
/// Summary period (not with a `ball:` clock: the loop is the clock).
const REPORT_S: u64 = 10;
/// A watched line that went low after the start and stays low this long
/// (no frame holds a line low for more than 8 ticks) ends the run.
const STUCK_LOW_MS: u32 = 500;
const STUCK_LOW_CYCLES: u32 = STUCK_LOW_MS * (bootcore::part::MCK_HZ / 1000);

/// GPBR3: "a pin-finder run started since the backup domain powered up".
/// GPBR0 / GPBR1 belong to the resident (bootcore::boot).
const GPBR3: *mut u32 = 0x400E_129C as *mut u32;
const RAN_MAGIC: u32 = u32::from_le_bytes(*b"PFRN");

fn pio(port: usize) -> &'static pac::pioa::RegisterBlock {
    unsafe {
        match port {
            0 => &*pac::PIOA::PTR,
            1 => &*pac::PIOB::PTR,
            _ => &*pac::PIOC::PTR,
        }
    }
}

/// May the fault paths sink nCONFIG? False until the handshake first drives
/// it, and again once a failed handshake has left it unproven.
static NCONFIG_ARMED: AtomicBool = AtomicBool::new(false);

/// The five PS lines by register, from any context. A zero-sized handle, so
/// the panic and fault handlers can use it too.
pub struct RegPins;

impl PsPins for RegPins {
    #[inline(always)]
    fn set_nconfig(&mut self, high: bool) {
        if high {
            self.release_nconfig()
        } else {
            self.assert_nconfig()
        }
    }
    /// The output latch only: with the line released this drives nothing.
    #[inline(always)]
    fn set_dclk(&mut self, high: bool) {
        let a = pio(0);
        unsafe {
            if high {
                a.sodr.write_with_zero(|w| w.bits(DCLK_BIT));
            } else {
                a.codr.write_with_zero(|w| w.bits(DCLK_BIT));
            }
        }
    }
    #[inline(always)]
    fn set_data(&mut self, high: bool) {
        let a = pio(0);
        unsafe {
            if high {
                a.sodr.write_with_zero(|w| w.bits(DATA0_BIT));
            } else {
                a.codr.write_with_zero(|w| w.bits(DATA0_BIT));
            }
        }
    }
    /// Reads low once the deadline has tripped, which ends the stream at the
    /// next byte.
    #[inline(always)]
    fn nstatus_high(&mut self) -> bool {
        !TRIPPED.load(Ordering::Relaxed) && pio(1).pdsr.read().bits() & NSTATUS_BIT != 0
    }
    #[inline(always)]
    fn conf_done_high(&mut self) -> bool {
        pio(1).pdsr.read().bits() & CONF_DONE_BIT != 0
    }
}

impl GuardedPsPins for RegPins {
    fn mark_run_started(&mut self) {
        unsafe { core::ptr::write_volatile(GPBR3, RAN_MAGIC) };
        cortex_m::asm::dsb();
    }
    #[inline(always)]
    fn assert_nconfig(&mut self) {
        NCONFIG_ARMED.store(true, Ordering::Relaxed);
        let a = pio(0);
        unsafe {
            a.codr.write_with_zero(|w| w.bits(NCONFIG_BIT));
            a.oer.write_with_zero(|w| w.bits(NCONFIG_BIT));
        }
    }
    /// Masked so the deadline hook cannot sink it between the check and the
    /// release; after a trip nCONFIG stays low.
    fn release_nconfig(&mut self) {
        cortex_m::interrupt::free(|_| {
            if !TRIPPED.load(Ordering::Relaxed) {
                unsafe { pio(0).odr.write_with_zero(|w| w.bits(NCONFIG_BIT)) };
            }
        });
    }
    fn disarm_nconfig(&mut self) {
        NCONFIG_ARMED.store(false, Ordering::Relaxed);
        unsafe { pio(0).odr.write_with_zero(|w| w.bits(NCONFIG_BIT)) };
    }
    /// Masked for the same reason: a trip either comes first (refused) or
    /// finds the lines enabled and releases them.
    fn enable_data_outputs(&mut self) -> bool {
        cortex_m::interrupt::free(|_| {
            if TRIPPED.load(Ordering::Relaxed) {
                return false;
            }
            let a = pio(0);
            unsafe {
                a.codr.write_with_zero(|w| w.bits(DCLK_BIT | DATA0_BIT));
                a.oer.write_with_zero(|w| w.bits(DCLK_BIT | DATA0_BIT));
            }
            true
        })
    }
    #[inline(always)]
    fn release_data_outputs(&mut self, keep_dclk: bool) {
        let bits = if keep_dclk { DATA0_BIT } else { DATA0_BIT | DCLK_BIT };
        let a = pio(0);
        unsafe {
            a.odr.write_with_zero(|w| w.bits(bits));
            a.puer.write_with_zero(|w| w.bits(bits));
        }
    }
}

/// Every exit: DCLK / DATA0 to inputs first, then nCONFIG low if it is armed.
/// From any context; idempotent.
#[inline(always)]
pub fn fpga_off() {
    guarded::fault_release(&mut RegPins, NCONFIG_ARMED.load(Ordering::Relaxed));
}

/// The five PS lines to PIO inputs with pull-up, nothing driven, the output
/// latches 0. First thing in the run: also covers what a boot ROM or an
/// earlier firmware left behind across a warm reset.
fn ps_lines_to_input() {
    const A: u32 = NCONFIG_BIT | DCLK_BIT | DATA0_BIT;
    const B: u32 = NSTATUS_BIT | CONF_DONE_BIT;
    for (port, m) in [(0, A), (1, B)] {
        let p = pio(port);
        unsafe {
            p.odr.write_with_zero(|w| w.bits(m));
            p.codr.write_with_zero(|w| w.bits(m));
            p.idr.write_with_zero(|w| w.bits(m));
            p.mddr.write_with_zero(|w| w.bits(m));
            p.ifdr.write_with_zero(|w| w.bits(m));
            p.puer.write_with_zero(|w| w.bits(m));
            p.per.write_with_zero(|w| w.bits(m));
        }
    }
}

/// Every watched line to PIO input with pull-up, no glitch filter, no
/// interrupt, no multi-drive. Call before the FPGA is loaded. (With
/// `no-board-io` nothing has set them up; this also covers whatever the
/// boot ROM or a resident left behind.)
pub fn prepare_pins() {
    ps_lines_to_input();
    for (port, &m) in MASK.iter().enumerate() {
        let p = pio(port);
        unsafe {
            p.idr.write_with_zero(|w| w.bits(m));
            p.mddr.write_with_zero(|w| w.bits(m));
            p.ifdr.write_with_zero(|w| w.bits(m));
            p.odr.write_with_zero(|w| w.bits(m));
            p.puer.write_with_zero(|w| w.bits(m));
            p.per.write_with_zero(|w| w.bits(m));
        }
    }
}

// ---- independent deadline (SysTick hook) -------------------------------

static TICKS: AtomicU32 = AtomicU32::new(0);
static DEADLINE: AtomicU32 = AtomicU32::new(u32::MAX);
static TRIPPED: AtomicBool = AtomicBool::new(false);

/// Runs every millisecond in the SysTick exception, whatever the main loop
/// does (unless interrupts are masked for that long, which nothing here does).
fn on_tick() {
    let t = TICKS.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    if t >= DEADLINE.load(Ordering::Relaxed) {
        fpga_off();
        TRIPPED.store(true, Ordering::Relaxed);
    }
}

fn set_deadline_ms(ms: u32) {
    DEADLINE.store(TICKS.load(Ordering::Relaxed).saturating_add(ms), Ordering::Relaxed);
}

// ---- fault paths -------------------------------------------------------

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    fpga_off();
    cortex_m::interrupt::disable();
    match info.location() {
        Some(l) => error!("PF panic at {=str}:{=u32}; PS lines released", l.file(), l.line()),
        None => error!("PF panic; PS lines released"),
    }
    loop {
        fpga_off();
        cortex_m::asm::wfi();
    }
}

#[cortex_m_rt::exception]
unsafe fn HardFault(_ef: &cortex_m_rt::ExceptionFrame) -> ! {
    fpga_off();
    loop {
        cortex_m::asm::wfi();
    }
}

/// Release the lines and stop for good (RTT stays readable).
fn halt(why: &str) -> ! {
    fpga_off();
    if NCONFIG_ARMED.load(Ordering::Relaxed) {
        error!("PF stop: {=str}. DCLK/DATA0 inputs, FPGA held in reset (nCONFIG low). Power off.", why);
    } else {
        error!("PF stop: {=str}. PA23/PA24/PA25 all inputs, nothing driven. Power off.", why);
    }
    loop {
        fpga_off();
        cortex_m::asm::wfi();
    }
}

fn report_precheck(e: PrecheckError) -> ! {
    match e {
        PrecheckError::AlreadyRan => {
            if RegPins.conf_done_high() {
                error!("PF precheck: CONF_DONE (PB1) reads high: the FPGA may still be configured from the earlier run. Remove power now.");
            }
            halt("precheck: a pin-finder run already started since power-up (GPBR3); remove power, or clear it with `probe-rs write --chip ATSAM3U2E b32 0x400E129C 0`")
        }
        PrecheckError::NoBitstream => {
            halt("precheck: no bitstream in this firmware (fpga/pinfind/output/pinfind_ag16k.rbf was missing at build time)")
        }
        PrecheckError::Decode { produced } => {
            error!("PF precheck: bitstream does not decode (stopped after {} bytes)", produced);
            halt("precheck: corrupt bitstream")
        }
        PrecheckError::Length { got, want } => {
            error!("PF precheck: bitstream decodes to {} bytes, expected {}", got, want);
            halt("precheck: bitstream length")
        }
        PrecheckError::Cksum { got, want } => {
            error!("PF precheck: bitstream cksum {=u32} != {=u32}", got, want);
            halt("precheck: bitstream checksum")
        }
    }
}

fn report_load(e: GuardedError) -> ! {
    if TRIPPED.load(Ordering::Relaxed) {
        error!("PF load: time limit ({} ms) passed", LOAD_LIMIT_MS);
    }
    match e {
        GuardedError::NstatusLowBeforeReset => {
            error!("PF handshake FAILED: nSTATUS (PB2) already low before nCONFIG moved; PA25 not driven");
            halt("handshake: nSTATUS low at start")
        }
        GuardedError::NstatusStuckHigh => {
            error!("PF handshake FAILED: nSTATUS (PB2) did not fall with PA25 low; PA25 released, PA23/PA24 never enabled");
            halt("handshake: nSTATUS stuck high")
        }
        GuardedError::NstatusStuckLow => {
            error!("PF handshake FAILED: nSTATUS (PB2) did not rise after PA25 release; PA23/PA24 never enabled");
            halt("handshake: nSTATUS stuck low")
        }
        GuardedError::ConfDoneHighAfterReset => {
            error!("PF handshake FAILED: CONF_DONE (PB1) high after reset; PA23/PA24 never enabled");
            halt("handshake: CONF_DONE high")
        }
        GuardedError::OutputsRefused => halt("load: deadline passed before DCLK/DATA0 were enabled"),
        GuardedError::Load(f) => {
            error!("PF stream FAILED: {}", f);
            halt("load failed")
        }
        GuardedError::EarlyDone { sent, want } => {
            error!("PF stream: CONF_DONE early, {} of {} bytes", sent, want);
            halt("configuration did not take the whole bitstream")
        }
        GuardedError::ShortTransfer { sent, want } => {
            error!("PF stream: short transfer, {} of {} bytes", sent, want);
            halt("configuration did not take the whole bitstream")
        }
    }
}

// ---- PIO snapshots -----------------------------------------------------

#[inline(always)]
fn pdsr_all() -> [u32; 3] {
    [pio(0).pdsr.read().bits(), pio(1).pdsr.read().bits(), pio(2).pdsr.read().bits()]
}

/// Report watched lines that differ between two snapshots. Returns how many.
fn report_diff(what: &str, a: &[u32; 3], b: &[u32; 3]) -> u32 {
    let mut n = 0;
    for port in 0..3 {
        let d = (a[port] ^ b[port]) & MASK[port];
        if d != 0 {
            warn!(
                "PF diff {=str} port={} changed={=u32:#010x} before={=u32:#010x} after={=u32:#010x}",
                what,
                port_char(port * 32),
                d,
                a[port],
                b[port]
            );
            n += d.count_ones();
        }
    }
    n
}

/// The whole pin-finder run. Called by `main_task` before anything has made a
/// PS line an output; never returns. `make_source` starts the bitstream (as for
/// `cyclone::load`).
pub fn main<S: PsSource>(
    present: bool,
    raw_len: usize,
    mut make_source: impl FnMut() -> S,
) -> ! {
    if params::STAGE0 {
        info!("PF STAGE 0 (no outputs driven) bitstream sha256={} ({} bytes raw)", params::RBF_SHA256, raw_len);
    } else {
        info!("PF STAGE 1-3 (DRIVES balls; NOT stage 0) bitstream sha256={} ({} bytes raw)", params::RBF_SHA256, raw_len);
    }
    info!(
        "PF pin-finder: stage0={} toggle_dclk={} ball_clock={} loops={} frames={} prescale={} drive={}",
        params::STAGE0,
        params::TOGGLE_DCLK,
        params::BALL_CLOCK,
        params::MAX_LOOPS,
        params::FRAMES,
        params::PRESCALE,
        params::N_DRIVE
    );

    // One run per power-up, and a bitstream that decodes to what was built.
    // Every PS line is still an input.
    let already_ran = unsafe { core::ptr::read_volatile(GPBR3) } == RAN_MAGIC;
    if let Err(e) = guarded::precheck(already_ran, present, raw_len, params::RBF_CKSUM, &mut make_source) {
        report_precheck(e);
    }
    info!("PF precheck passed ({} bytes, cksum {=u32})", raw_len, params::RBF_CKSUM);

    info!("PF waiting {} s, PS lines all inputs, before loading (attach probes now)", PRE_LOAD_WAIT_S);
    let w0 = Instant::now();
    while (Instant::now() - w0).as_secs() < PRE_LOAD_WAIT_S {}

    embassy_atsam3::systick::set_tick_hook(on_tick);
    set_deadline_ms(LOAD_LIMIT_MS);

    let t0 = Instant::now();
    cortex_m::asm::delay(bootcore::part::MCK_HZ / 500); // 2 ms: inputs settle
    let before = pdsr_all();

    info!("PF handshake: PA25 low, expecting PB2 to follow");
    let mut clk = cyclone::McuClock;
    let stats = match guarded::guarded_load(
        &cyclone::PROFILE,
        &mut RegPins,
        &mut clk,
        raw_len,
        params::TOGGLE_DCLK,
        make_source,
    ) {
        Ok(s) => s,
        Err(e) => report_load(e),
    };
    if TRIPPED.load(Ordering::Relaxed) {
        halt("load took longer than the limit");
    }
    info!(
        "PF FPGA configured ({} ms, {} bytes, padding {}); PA23{} back to input",
        (Instant::now() - t0).as_millis(),
        stats.bytes,
        stats.padding,
        if params::TOGGLE_DCLK { "" } else { "/PA24" }
    );

    if params::STAGE0 {
        set_deadline_ms(STAGE0_HOLD_S as u32 * 1000 + HOOK_MARGIN_MS);
    } else {
        set_deadline_ms(RUN_LIMIT_S as u32 * 1000 + HOOK_MARGIN_MS);
    }
    cortex_m::asm::delay(bootcore::part::MCK_HZ / 1000); // 1 ms
    let after = pdsr_all();
    let n = report_diff("across-configuration", &before, &after);
    info!("PF across configuration: {} watched line(s) changed (PS lines not counted)", n);

    if params::STAGE0 {
        stage0(&after);
    }
    run(&mut RegPins)
}

/// Stage 0: the design has no outputs. Hold it configured for the current
/// reading, and report any watched line that changes meanwhile.
fn stage0(after: &[u32; 3]) -> ! {
    info!("PF stage 0: holding the output-free design for {} s (read the supply current now)", STAGE0_HOLD_S);
    let start = Instant::now();
    let mut last = *after;
    let mut changes = 0u32;
    while (Instant::now() - start).as_secs() < STAGE0_HOLD_S {
        if TRIPPED.load(Ordering::Relaxed) {
            break;
        }
        let s = pdsr_all();
        if (s[0] ^ last[0]) & MASK[0] | (s[1] ^ last[1]) & MASK[1] | (s[2] ^ last[2]) & MASK[2] != 0 {
            changes += report_diff("during-stage0", &last, &s);
            last = s;
        }
    }
    fpga_off();
    info!(
        "PF end (stage 0): {} change(s) while configured. FPGA held in reset (nCONFIG low). Power off.",
        changes
    );
    loop {
        fpga_off();
        cortex_m::asm::wfi();
    }
}

#[inline(always)]
fn sample() -> [u32; 3] {
    [
        pio(0).pdsr.read().bits() & MASK[0],
        pio(1).pdsr.read().bits() & MASK[1],
        pio(2).pdsr.read().bits() & MASK[2],
    ]
}

struct Pin {
    dec: Decoder,
    tally: Tally,
    edges: u32,
    /// Cycle count of the last falling edge.
    low_since: u32,
}

const PIN0: Pin = Pin { dec: Decoder::new(false), tally: Tally::new(), edges: 0, low_since: 0 };

/// ~90 bytes a line, in SRAM0. Uninitialised (.bss, not .data: the initial
/// value is not all zeros and would cost 8.5 KB of flash) and filled in by
/// `run`. Only the active-low decoder runs here (an inverter between the
/// FPGA and the MCU is not expected; such a line shows up as `PF act`); the
/// logic-analyser path tries both polarities.
static mut PINS: core::mem::MaybeUninit<[Pin; NPINS]> = core::mem::MaybeUninit::uninit();

fn port_char(i: usize) -> char {
    (b'A' + (i / 32) as u8) as char
}

fn report(pins: &[Pin; NPINS], clock: &Option<Frame>) {
    for (i, p) in pins.iter().enumerate() {
        for &(idx, n) in p.tally.hits() {
            info!("PF hit pin=P{}{} idx={} n={}", port_char(i), i % 32, idx, n);
        }
        if p.tally.hits().is_empty() && p.edges > 0 {
            info!("PF act pin=P{}{} edges={}", port_char(i), i % 32, p.edges);
        }
    }
    if let Some(f) = clock {
        info!("PF unit={}", f.unit);
        if let Some(c) = f.clocks() {
            for k in 0..c.n as usize {
                let hz = c.hz(k, f.unit, bootcore::part::MCK_HZ);
                info!(
                    "PF clk c={} cnt={} unit={} div={} win={} hz={}",
                    k, c.counts[k], f.unit, c.div_bits, c.win_bits, hz as u32
                );
            }
        }
    }
}

/// Listen until the run ends, then hold the FPGA in reset. Never returns.
fn run(fpga: &mut RegPins) -> ! {
    // CPU cycle counter as the time base (96 MHz, wraps every ~44 s; only
    // differences are used).
    let mut cp = unsafe { cortex_m::Peripherals::steal() };
    cp.DCB.enable_trace();
    cp.DWT.enable_cycle_counter();

    // One element at a time, so the 8.5 KB array is never built on the stack.
    let pins: &mut [Pin; NPINS] = unsafe {
        let base = core::ptr::addr_of_mut!(PINS) as *mut Pin;
        for i in 0..NPINS {
            base.add(i).write(PIN0);
        }
        (*core::ptr::addr_of_mut!(PINS)).assume_init_mut()
    };
    let mut prev = sample();
    // Lines low at the start are someone else's (not a stuck scan output).
    let low_at_start = [!prev[0] & MASK[0], !prev[1] & MASK[1], !prev[2] & MASK[2]];
    let t0 = DWT::cycle_count();
    for (i, p) in pins.iter_mut().enumerate() {
        let high = prev[i / 32] >> (i % 32) & 1 != 0;
        p.dec.edge(t0, high); // the starting level
        p.low_since = t0;
    }
    info!(
        "PF start: watching {} lines (PS lines excluded), limit {} s, quiet end {} s, stuck-low {} ms",
        MASK.iter().map(|m| m.count_ones()).sum::<u32>(),
        RUN_LIMIT_S,
        QUIET_END_S,
        STUCK_LOW_MS
    );

    let start = Instant::now();
    let mut last_report = Instant::now();
    let mut last_frame: Option<Instant> = None;
    let mut clock: Option<Frame> = None;
    let mut dclk = false;
    let mut iter: u32 = 0;
    let why: &str;

    loop {
        if params::TOGGLE_DCLK {
            dclk = !dclk;
            fpga.set_dclk(dclk);
        }
        let now = DWT::cycle_count();
        let s = sample();
        let diff = [s[0] ^ prev[0], s[1] ^ prev[1], s[2] ^ prev[2]];
        if diff[0] | diff[1] | diff[2] != 0 {
            for (port, &d) in diff.iter().enumerate() {
                let mut bits = d;
                while bits != 0 {
                    let b = bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    let i = port * 32 + b;
                    let high = s[port] >> b & 1 != 0;
                    let p = &mut pins[i];
                    p.edges = p.edges.saturating_add(1);
                    if !high {
                        p.low_since = now;
                    }
                    if let Some(f) = p.dec.edge(now, high) {
                        got_frame(i, p, f, &mut clock, &mut last_frame);
                    }
                }
            }
            prev = s;
        }

        iter = iter.wrapping_add(1);
        if iter % 1024 == 0 {
            let now = DWT::cycle_count();
            let mut stuck = None;
            for i in 0..NPINS {
                let p = &mut pins[i];
                if p.dec.in_frame() {
                    if let Some(f) = p.dec.poll(now) {
                        got_frame(i, p, f, &mut clock, &mut last_frame);
                    }
                }
                let (port, b) = (i / 32, i % 32);
                let low = prev[port] >> b & 1 == 0 && MASK[port] >> b & 1 != 0;
                if low && low_at_start[port] >> b & 1 == 0 && now.wrapping_sub(p.low_since) >= STUCK_LOW_CYCLES {
                    stuck = Some(i);
                }
            }
            if let Some(i) = stuck {
                fpga_off();
                error!("PF stuck-low pin=P{}{} for >= {} ms: a stopped scan? FPGA put in reset", port_char(i), i % 32, STUCK_LOW_MS);
                why = "stuck-low line";
                break;
            }
            if TRIPPED.load(Ordering::Relaxed) {
                why = "time limit (tick hook)";
                break;
            }
            if iter % (64 * 1024) == 0 {
                let t = Instant::now();
                let quiet = last_frame.map(|l| (t - l).as_secs() >= QUIET_END_S).unwrap_or(false);
                let limit = (t - start).as_secs() >= RUN_LIMIT_S;
                // With a `ball:` clock the loop is the FPGA's clock: no long
                // summary while it runs (LOG R143 item 4).
                if !params::BALL_CLOCK && !quiet && !limit && (t - last_report).as_secs() >= REPORT_S {
                    report(pins, &clock);
                    last_report = t;
                }
                if quiet || limit {
                    why = if quiet { "quiet" } else { "time limit" };
                    break;
                }
            }
        }
    }

    // FPGA back into configuration: user I/O off. Then the summary.
    fpga_off();
    report(pins, &clock);
    info!(
        "PF end ({=str}, {} s): FPGA held in reset (nCONFIG low). Power off, then flash the normal firmware.",
        why,
        (Instant::now() - start).as_secs()
    );
    loop {
        fpga_off();
        cortex_m::asm::wfi();
    }
}

#[inline(never)]
fn got_frame(i: usize, p: &mut Pin, f: Frame, clock: &mut Option<Frame>, last: &mut Option<Instant>) {
    *last = Some(Instant::now());
    if p.tally.add(f.index()) {
        info!("PF new pin=P{}{} idx={} unit={}", port_char(i), i % 32, f.index(), f.unit);
    }
    if f.clocks().is_some() {
        *clock = Some(f);
    }
}
