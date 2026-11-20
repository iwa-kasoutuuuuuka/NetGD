//! CYCLONE2 must put the same waveform on the wire as the loader it
//! replaced. `old_load` below is that loader (`mcu/netgd/src/cyclone.rs`
//! at 665ac01d), ported line for line onto the same pin/clock traits; the
//! new `load_once` runs against an identical simulated FPGA and the two
//! event traces are compared.
//!
//! Compared: every level change on nCONFIG / DCLK / DATA0, in order, with
//! the no-op counts and the delays between them. Not compared: the no-ops of
//! the old nSTATUS polling loop (the new loop polls the clock instead; both
//! only spin until the FPGA answers), and writes that do not change a level.
//! The old `Timer::after(1 ms)` is modelled as `delay_us(1000)`.

mod common;

use common::*;
use psload::*;

/// The old `PSLoader::load_from`, minus the panic (an error return instead).
fn old_load<P: PsPins, C: PsClock>(
    pins: &mut P,
    clk: &mut C,
    bitstream: impl Iterator<Item = u8>,
) -> Result<(), &'static str> {
    fn delay<C: PsClock>(clk: &mut C) {
        clk.nops(5); // for _ in 0..5 { asm::nop() }
    }
    fn send_bit<P: PsPins, C: PsClock>(pins: &mut P, clk: &mut C, bit: bool) {
        pins.set_dclk(false);
        delay(clk);
        pins.set_data(bit);
        delay(clk);
        pins.set_dclk(true);
        delay(clk);
        delay(clk);
    }
    fn send_byte<P: PsPins, C: PsClock>(pins: &mut P, clk: &mut C, byte: u8) {
        for i in 0..8 {
            send_bit(pins, clk, (byte & (1 << i)) != 0);
        }
    }

    // `new()` left all three outputs low; the sim starts that way.
    clk.delay_us(1000); // Timer::after(Duration::from_millis(1))
    pins.set_nconfig(true);
    while !pins.nstatus_high() {
        clk.nops(1);
    }
    for byte in bitstream {
        if pins.conf_done_high() {
            return Ok(());
        }
        if !pins.nstatus_high() {
            return Err("FPGA Upload Error");
        }
        send_byte(pins, clk, byte);
    }
    while !pins.conf_done_high() {
        send_bit(pins, clk, false);
    }
    Ok(())
}

/// Drop the polling no-ops: everything before the first DCLK event that is a
/// `Nops`. (Delays and pin changes before it are kept.)
fn normalized(w: &Shared) -> Vec<Ev> {
    let t = w.borrow().trace.clone();
    let first_dclk = t.iter().position(|e| matches!(e, Ev::Pin(Pin::Dclk, _))).unwrap_or(t.len());
    t.into_iter()
        .enumerate()
        .filter(|(i, e)| !(*i < first_dclk && matches!(e, Ev::Nops(_))))
        .map(|(_, e)| e)
        .collect()
}

fn compare(cfg: Cfg, stream: Vec<u8>) {
    let wo = world(cfg.clone());
    let r = old_load(&mut SimPins(wo.clone()), &mut SimClock(wo.clone()), stream.iter().copied());
    assert!(r.is_ok());

    let wn = world(cfg);
    let r = load_once(&CYCLONE2, &mut SimPins(wn.clone()), &mut SimClock(wn.clone()), stream.iter().copied());
    assert!(r.is_ok(), "{r:?}");

    let (a, b) = (normalized(&wo), normalized(&wn));
    assert!(a.len() > 100, "trace too short to mean anything: {}", a.len());
    assert_eq!(a, b);
    // The FPGA saw the same thing, and ended in the same state.
    let (wo, wn) = (wo.borrow(), wn.borrow());
    assert_eq!(wo.rx, wn.rx);
    assert_eq!((wo.dclk, wo.data, wo.nconfig), (wn.dclk, wn.data, wn.nconfig));
    assert_eq!((wo.done, wo.clocks_after_done), (wn.done, wn.clocks_after_done));
    // No trailing clocks, DCLK left as the last bit left it (the old exit state).
    assert_eq!(wn.clocks_after_done, 0);
}

#[test]
fn short_stream_done_after_the_last_bit() {
    let data = payload(40, 21);
    compare(Cfg::new(data.clone()), data);
}

#[test]
fn short_stream_with_padding_clocks_before_done() {
    let data = payload(40, 22);
    compare(Cfg { done_after_clocks: 23, ..Cfg::new(data.clone()) }, data);
}

#[test]
fn conf_done_rises_before_the_source_is_exhausted() {
    let need = payload(40, 23);
    let mut stream = need.clone();
    stream.extend(payload(50, 24));
    compare(Cfg::new(need), stream);
}

#[test]
fn slow_nstatus_does_not_change_the_data_phase() {
    let data = payload(64, 25);
    compare(Cfg { nstatus_rise_us: 230, done_after_clocks: 5, ..Cfg::new(data.clone()) }, data);
}

#[test]
fn ends_with_dclk_high_and_data_as_last_bit() {
    // Spelled out, since it is the visible difference from AG16K's parked exit.
    let data = vec![0x01u8, 0x80];
    let w = world(Cfg::new(data.clone()));
    load_once(&CYCLONE2, &mut SimPins(w.clone()), &mut SimClock(w.clone()), data.iter().copied()).unwrap();
    let w = w.borrow();
    assert!(w.dclk, "DCLK high at exit, as before");
    assert!(w.data, "last bit sent was bit 7 of 0x80, DATA stays high");
}
