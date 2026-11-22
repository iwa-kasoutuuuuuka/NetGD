//! `census` feature: a passive pin census for a GDEMU 6.0 board after ERASE.
//!
//! `docs/research/gdemu60-bringup.md` §2-4; the logic is `common/pincensus`
//! (host-tested). The first firmware to put on an erased 6.0 board, before
//! the pin-finder and before NetGD: those drive nCONFIG / DCLK / DATA0 (and
//! NetGD the SMC bus, the LED, HSMCI, UART, PCK0) on pins that are assumed
//! from v5.5, and this one checks those assumptions without driving anything.
//!
//! What it does, over and over (attach at any time):
//! 1. logs what the PIO controllers look like at entry (PSR, OSR, PUSR,
//!    ABSR: after a reset every line is a PIO input with its pull-up on);
//! 2. makes sure every line is a PIO input -- output disabled, no
//!    multi-drive, no glitch filter, no interrupt -- and leaves the pull-up
//!    on (the reset state);
//! 3. samples all 96 lines with the pull-up on, then with the pull-up off,
//!    then switches the pull-up back on;
//! 4. classifies each line (`pincensus::classify`) and logs it with its
//!    v5.5 role, and whether the five PS lines and the button read the way
//!    the v5.5 assumption predicts for an unconfigured FPGA;
//! 5. watches for changes for `WATCH_S` seconds: press the button, insert and
//!    remove the SD card then.
//!
//! RTT lines (all start with `PC `):
//!
//! ```text
//! PC census pass=1 (passive: no line is ever driven)
//! PC entry port=A psr=.. osr=.. pusr=.. absr=..
//! PC rate on=.. off=.. samples per phase, ... ms each
//! PC pin=PB1 class=low on=low off=low edges=0/0 v55=PS CONF_DONE agrees=yes
//! PC mask class=low A=0x.. B=0x.. C=0x..
//! PC ps-check agree=6 disagree=0 (PA23 PA24 PA25 PB1 PB2 PA20)
//! PC watch 30 s: press the button now, insert / remove the SD card now
//! PC change t_ms=1234 pin=PA20 level=0 v55=button
//! PC pass end
//! ```
//!
//! "Passive" is about the pins. Besides PIO, this firmware (with the chip
//! init it runs after) also writes PMC (peripheral clocks, the main crystal
//! oscillator start-up and the MCK switch to it), EFC (flash wait states for
//! the faster MCK), WDT and SysTick, and starts the 12 MHz XIN/XOUT
//! oscillator: a missing or different crystal stalls it silently, with no
//! RTT output (`docs/research/gdemu60-bringup.md` §2-1 N18, §3-1).
//!
//! Safety: the only PIO registers written are ODR, IDR, MDDR, IFDR, PER
//! (input, plain) and PUER / PUDR (the pull-up). Never OER, SODR, CODR, PDR,
//! ABSR, MDER. `init` sets up no SMC / UART / PCK0 / HSMCI pins
//! (embassy-atsam3 `no-board-io`), and no LED is used. The FPGA is never
//! configured. Not run on hardware.

use defmt::{info, warn};
use embassy_atsam3::pac;
use embassy_time::Instant;
use pincensus::{agrees, class_masks, classify, pin_bit, pins, port_char, role, Class, Phase, Watch, NPINS, PORTS};

/// Settle after a pull-up change before sampling.
const SETTLE_MS: u64 = 200;
/// Sampling time per phase.
const PHASE_MS: u64 = 500;
/// Watch for changes this long after each census pass.
const WATCH_S: u64 = 30;
/// Reports per line in one watch.
const WATCH_LIMIT: u8 = 4;
/// Pause between log lines, so the 1 KiB RTT buffer is not overrun while a
/// host is attached (defmt-rtt drops what does not fit).
const LINE_GAP_MS: u64 = 4;

const ALL: u32 = 0xFFFF_FFFF;

fn pio(port: usize) -> &'static pac::pioa::RegisterBlock {
    unsafe {
        match port {
            0 => &*pac::PIOA::PTR,
            1 => &*pac::PIOB::PTR,
            _ => &*pac::PIOC::PTR,
        }
    }
}

fn read_all() -> [u32; PORTS] {
    [pio(0).pdsr.read().bits(), pio(1).pdsr.read().bits(), pio(2).pdsr.read().bits()]
}

fn spin_ms(ms: u64) {
    let t = Instant::now();
    while (Instant::now() - t).as_millis() < ms {}
}

/// Every line a plain PIO input. Output disable first, so nothing is driven
/// even for a moment when a line goes from a peripheral back to the PIO.
fn all_inputs() {
    for port in 0..PORTS {
        let p = pio(port);
        unsafe {
            p.odr.write_with_zero(|w| w.bits(ALL));
            p.idr.write_with_zero(|w| w.bits(ALL));
            p.mddr.write_with_zero(|w| w.bits(ALL));
            p.ifdr.write_with_zero(|w| w.bits(ALL));
            p.per.write_with_zero(|w| w.bits(ALL));
        }
    }
}

fn pull_ups(on: bool) {
    for port in 0..PORTS {
        let p = pio(port);
        unsafe {
            if on {
                p.puer.write_with_zero(|w| w.bits(ALL));
            } else {
                p.pudr.write_with_zero(|w| w.bits(ALL));
            }
        }
    }
}

fn log_entry_state() {
    for port in 0..PORTS {
        let p = pio(port);
        info!(
            "PC entry port={} psr={=u32:#010x} osr={=u32:#010x} pusr={=u32:#010x} absr={=u32:#010x}",
            port_char(port * 32),
            p.psr.read().bits(),
            p.osr.read().bits(),
            p.pusr.read().bits(),
            p.absr.read().bits()
        );
        spin_ms(LINE_GAP_MS);
    }
}

fn sample_phase(ph: &mut Phase) {
    let t = Instant::now();
    while (Instant::now() - t).as_millis() < PHASE_MS {
        ph.sample(read_all());
    }
}

fn seen_label(s: pincensus::Seen) -> &'static str {
    match s {
        pincensus::Seen::Nothing => "-",
        pincensus::Seen::OnlyHigh => "high",
        pincensus::Seen::OnlyLow => "low",
        pincensus::Seen::Both => "both",
    }
}

/// One census pass: classify and log. Returns the `Toggling` lines (the
/// watch ignores them).
fn pass(n: u32) -> [u32; PORTS] {
    info!("PC census pass={} (passive: no line is ever driven)", n);
    log_entry_state();
    all_inputs();

    let mut on = Phase::new();
    let mut off = Phase::new();
    pull_ups(true);
    spin_ms(SETTLE_MS);
    sample_phase(&mut on);
    pull_ups(false);
    spin_ms(SETTLE_MS);
    sample_phase(&mut off);
    pull_ups(true);
    info!(
        "PC rate on={} off={} samples per phase, {} ms each",
        on.samples(),
        off.samples(),
        PHASE_MS
    );

    let mut agree = 0u32;
    let mut disagree = 0u32;
    for pin in 0..NPINS {
        let c = classify(&on, &off, pin);
        let r = role(pin);
        let (name, verdict) = match r {
            Some(r) => (r.name, agrees(r, c)),
            None => ("-", None),
        };
        let v = match verdict {
            Some(true) => {
                agree += 1;
                "yes"
            }
            Some(false) => {
                disagree += 1;
                "NO"
            }
            None => "-",
        };
        info!(
            "PC pin=P{}{} class={=str} on={=str} off={=str} edges={}/{} v55={=str} agrees={=str}",
            port_char(pin),
            pin_bit(pin),
            c.label(),
            seen_label(on.seen(pin)),
            seen_label(off.seen(pin)),
            on.edges(pin),
            off.edges(pin),
            name,
            v
        );
        spin_ms(LINE_GAP_MS);
    }
    let masks = class_masks(&on, &off);
    for c in Class::ALL {
        let m = masks[c as usize];
        info!(
            "PC mask class={=str} A={=u32:#010x} B={=u32:#010x} C={=u32:#010x}",
            c.label(),
            m[0],
            m[1],
            m[2]
        );
        spin_ms(LINE_GAP_MS);
    }
    if disagree == 0 {
        info!("PC ps-check agree={} disagree=0 (PA23 PA24 PA25 PB1 PB2 PA20)", agree);
    } else {
        warn!(
            "PC ps-check agree={} disagree={}: the v5.5 PS / button pin map does NOT fit this board. Do not run the pin-finder or NetGD until this is understood.",
            agree,
            disagree
        );
    }
    masks[Class::Toggling as usize]
}

fn watch(ignore: [u32; PORTS]) {
    info!(
        "PC watch {} s: press the button now, insert / remove the SD card now",
        WATCH_S
    );
    let mut w = Watch::new(ignore, WATCH_LIMIT);
    let t0 = Instant::now();
    while (Instant::now() - t0).as_secs() < WATCH_S {
        let now = read_all();
        let rep = w.update(now);
        if rep != [0; PORTS] {
            let t_ms = (Instant::now() - t0).as_millis();
            for pin in pins(rep) {
                let level = (now[pin / 32] >> (pin % 32)) & 1;
                info!(
                    "PC change t_ms={} pin=P{}{} level={} v55={=str}",
                    t_ms,
                    port_char(pin),
                    pin_bit(pin),
                    level,
                    role(pin).map(|r| r.name).unwrap_or("-")
                );
            }
        }
    }
}

/// The whole census, forever. Never drives a line; never returns.
pub fn run() -> ! {
    info!("PC GDEMU 6.0 pin census (feature `census`): no line is ever driven, the FPGA is not configured");
    let mut n = 1u32;
    loop {
        let toggling = pass(n);
        watch(toggling);
        info!("PC pass end");
        n = n.wrapping_add(1);
    }
}
