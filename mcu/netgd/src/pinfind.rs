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
//! Safety, MCU side (LOG R143):
//! * The MCU drives nothing but nCONFIG, and DCLK / DATA0 only while
//!   configuring. Right after a successful configuration DATA0 (PA23) and,
//!   unless the design is clocked from G1 (`--clock ball:PIN_G1`), DCLK (PA24)
//!   go back to input + pull-up. Every other PIO line is an input with pull-up
//!   from the start (`prepare_pins`, and `init` sets up no SMC / UART / PCK0 /
//!   HSMCI pins: embassy-atsam3 `no-board-io`). No LED.
//! * The FPGA is never left running: a failed or short load drives nCONFIG
//!   low and stops (no retry); a SysTick hook drives nCONFIG low by itself
//!   when the time limit passes, whatever the main loop is doing; the panic,
//!   HardFault and default handlers drive nCONFIG low first; an MCU line held
//!   low far longer than any frame can (a stopped clock) ends the run.
//! * One run per power-up: a flag in GPBR3 (kept across resets, cleared when
//!   the backup domain loses power) makes a second boot refuse to load the
//!   FPGA. Clear it by removing power, or over SWD:
//!   `probe-rs write --chip ATSAM3U2E b32 0x400E129C 0`.
//! * Not covered: a debugger halting the core (nothing runs, nCONFIG stays
//!   as it is; with a `ball:` clock the FPGA stops mid-frame) -- do not halt
//!   while the FPGA is configured.

use crate::cyclone;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use cortex_m::peripheral::DWT;
use defmt::{error, info, warn};
use embassy_atsam3::pac;
use embassy_time::Instant;
use pinfind::{Decoder, Frame, Tally};
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

/// Wait this long after boot, FPGA unconfigured (nCONFIG low, its user I/O
/// off), before loading: time to attach the logic analyser and the external
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

/// nCONFIG (PA25) low, by register, from any context: the FPGA goes back
/// into configuration with its user I/O off. Idempotent.
#[inline(always)]
pub fn fpga_off() {
    let a = pio(0);
    unsafe {
        a.codr.write_with_zero(|w| w.bits(NCONFIG_BIT));
        a.oer.write_with_zero(|w| w.bits(NCONFIG_BIT));
        a.per.write_with_zero(|w| w.bits(NCONFIG_BIT));
    }
}

/// Every watched line to PIO input with pull-up, no glitch filter, no
/// interrupt, no multi-drive. Call before the FPGA is loaded. (With
/// `no-board-io` nothing has set them up; this also covers whatever the
/// boot ROM or a resident left behind.)
pub fn prepare_pins() {
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

/// PA23 (and PA24 unless it clocks the design) back to input + pull-up,
/// right after configuration: they are user I/O of the FPGA from now on
/// (ag16k-config.md C9).
fn release_ps_data_lines() {
    let bits = if params::TOGGLE_DCLK { DATA0_BIT } else { DATA0_BIT | DCLK_BIT };
    let a = pio(0);
    unsafe {
        a.odr.write_with_zero(|w| w.bits(bits));
        a.puer.write_with_zero(|w| w.bits(bits));
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
        Some(l) => error!("PF panic at {=str}:{=u32}; FPGA held in reset", l.file(), l.line()),
        None => error!("PF panic; FPGA held in reset"),
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

/// nCONFIG low and stop for good (RTT stays readable).
fn halt(why: &str) -> ! {
    fpga_off();
    error!("PF stop: {=str}. FPGA held in reset (nCONFIG low). Power off.", why);
    loop {
        fpga_off();
        cortex_m::asm::wfi();
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

/// The whole pin-finder run. Called by `main_task` right after the PS pins
/// are set up; never returns. `make_source` starts the bitstream (as for
/// `cyclone::load`).
pub fn main<S: PsSource>(
    fpga: &mut cyclone::McuPins,
    present: bool,
    raw_len: usize,
    make_source: impl FnMut() -> S,
) -> ! {
    fpga.set_nconfig(false);
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

    // One run per power-up.
    unsafe {
        if core::ptr::read_volatile(GPBR3) == RAN_MAGIC {
            halt("a pin-finder run already started since power-up (GPBR3); remove power, or clear it with `probe-rs write --chip ATSAM3U2E b32 0x400E129C 0`");
        }
    }
    if !present {
        halt("no bitstream in this firmware (fpga/pinfind/output/pinfind_ag16k.rbf was missing at build time)");
    }

    info!("PF waiting {} s with the FPGA in reset before loading (attach probes now)", PRE_LOAD_WAIT_S);
    let w0 = Instant::now();
    while (Instant::now() - w0).as_secs() < PRE_LOAD_WAIT_S {
        fpga.set_nconfig(false);
    }

    // From here on a reset must not lead to a second run.
    unsafe { core::ptr::write_volatile(GPBR3, RAN_MAGIC) };
    embassy_atsam3::systick::set_tick_hook(on_tick);
    set_deadline_ms(LOAD_LIMIT_MS);

    let t0 = Instant::now();
    cortex_m::asm::delay(bootcore::part::MCK_HZ / 500); // 2 ms: inputs settle
    let before = pdsr_all();

    let stats = match cyclone::load_once_no_retry(fpga, make_source) {
        Ok(s) => s,
        Err(f) => {
            error!("PF FPGA load failed: {} (after {} attempt)", f.error, f.attempts);
            halt("load failed")
        }
    };
    if TRIPPED.load(Ordering::Relaxed) {
        halt("load took longer than the limit");
    }
    if stats.early_done || stats.bytes as usize != raw_len {
        error!(
            "PF CONF_DONE early or short transfer: {} of {} bytes, early_done={}",
            stats.bytes, raw_len, stats.early_done
        );
        halt("configuration did not take the whole bitstream");
    }
    release_ps_data_lines();
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
    run(fpga)
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
fn run(fpga: &mut cyclone::McuPins) -> ! {
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
    fpga.set_nconfig(false);
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
