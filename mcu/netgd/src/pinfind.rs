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
//! PF end                             FPGA held in reset (nCONFIG low)
//! ```
//!
//! Nothing here is used by the normal firmware; the feature is off by default.
//! Not run on hardware.
//!
//! Safety, MCU side: the MCU drives nothing but the three PS outputs it
//! always drives (nCONFIG, DCLK, DATA0). Every other PIO line is an input
//! with the internal pull-up, as after reset, so the MCU cannot fight the
//! FPGA on any pin. The run ends by itself (`RUN_LIMIT_S`, or `QUIET_END_S`
//! after the last frame): nCONFIG goes low, which puts the FPGA back into
//! configuration with its user I/O off.

use crate::cyclone;
use cortex_m::peripheral::DWT;
use defmt::info;
use embassy_atsam3::pac;
use embassy_time::Instant;
use pinfind::{Decoder, Frame, Tally};
use psload::PsPins;

/// Lines left alone: the PS loader's five (PA23 DATA0, PA24 DCLK, PA25
/// nCONFIG; PB1 CONF_DONE, PB2 nSTATUS -- `cyclone::pinmap`). The
/// configuration balls are never scanned, so nothing would arrive there.
const SKIP: [u32; 3] = [(1 << 23) | (1 << 24) | (1 << 25), (1 << 1) | (1 << 2), 0];
/// Watched lines per port.
const MASK: [u32; 3] = [!SKIP[0], !SKIP[1], !SKIP[2]];
const NPINS: usize = 96;

/// Stop after this long whatever happens: bounds the time any ball is driven
/// (the bitstream's own `max_loops` is the other bound).
const RUN_LIMIT_S: u64 = 300;
/// Stop this long after the last frame on any pin (the FPGA has finished its
/// loops, or stopped).
const QUIET_END_S: u64 = 30;
/// Summary period.
const REPORT_S: u64 = 10;

/// Toggle DCLK (PA24) in the sampling loop, so a bitstream built with
/// `gen.py --clock ball:PIN_G1` has a clock. Harmless otherwise: DCLK is a
/// configuration ball the scan never drives.
const TOGGLE_DCLK: bool = true;

fn pio(port: usize) -> &'static pac::pioa::RegisterBlock {
    unsafe {
        match port {
            0 => &*pac::PIOA::PTR,
            1 => &*pac::PIOB::PTR,
            _ => &*pac::PIOC::PTR,
        }
    }
}

/// Every watched line to PIO input with pull-up, no glitch filter, no
/// interrupt, no multi-drive. Undoes what `embassy_atsam3::init` set up (SMC
/// bus, UART, PCK0 on PA21, HSMCI) so none of it drives a line the FPGA may
/// be about to drive. Call before the FPGA is loaded.
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
}

const PIN0: Pin = Pin { dec: Decoder::new(false), tally: Tally::new(), edges: 0 };

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
pub fn run(fpga: &mut cyclone::McuPins) -> ! {
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
    let t0 = DWT::cycle_count();
    for (i, p) in pins.iter_mut().enumerate() {
        let high = prev[i / 32] >> (i % 32) & 1 != 0;
        p.dec.edge(t0, high); // the starting level
    }
    info!(
        "PF start: watching {} lines (PS lines excluded), limit {} s, quiet end {} s",
        MASK.iter().map(|m| m.count_ones()).sum::<u32>(),
        RUN_LIMIT_S,
        QUIET_END_S
    );

    let start = Instant::now();
    let mut last_report = Instant::now();
    let mut last_frame: Option<Instant> = None;
    let mut clock: Option<Frame> = None;
    let mut dclk = false;
    let mut iter: u32 = 0;

    loop {
        if TOGGLE_DCLK {
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
            for i in 0..NPINS {
                let p = &mut pins[i];
                if p.dec.in_frame() {
                    if let Some(f) = p.dec.poll(now) {
                        got_frame(i, p, f, &mut clock, &mut last_frame);
                    }
                }
            }
            if iter % (64 * 1024) == 0 {
                let t = Instant::now();
                let quiet = last_frame.map(|l| (t - l).as_secs() >= QUIET_END_S).unwrap_or(false);
                let limit = (t - start).as_secs() >= RUN_LIMIT_S;
                if quiet || limit || (t - last_report).as_secs() >= REPORT_S {
                    report(pins, &clock);
                    last_report = t;
                }
                if quiet || limit {
                    break;
                }
            }
        }
    }

    // FPGA back into configuration: user I/O off.
    fpga.set_nconfig(false);
    info!(
        "PF end ({} s): FPGA held in reset (nCONFIG low). Power off, then flash the normal firmware.",
        (Instant::now() - start).as_secs()
    );
    loop {
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
