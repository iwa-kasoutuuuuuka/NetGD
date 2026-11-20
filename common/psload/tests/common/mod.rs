//! A simulated FPGA behind the `PsPins` / `PsClock` traits.
//!
//! It models just what the loader can observe of a real configuration
//! engine: nSTATUS answering nCONFIG after a delay, data sampled on DCLK
//! rising edges and assembled into bytes in a chosen bit order, a "CRC" (the
//! assembled payload must equal the expected one) checked when the payload
//! is complete, nSTATUS dropping on a bad stream, CONF_DONE rising some
//! clocks after the last bit, and a few clocks after CONF_DONE before user
//! mode. Time is virtual: every pin access, no-op and delay advances it.
#![allow(dead_code)]

use psload::{BitOrder, PsClock, PsPins};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pin {
    Nconfig,
    Dclk,
    Data,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ev {
    /// A write that changed the level.
    Pin(Pin, bool),
    Nops(u32),
    Delay(u32),
}

#[derive(Clone)]
pub struct Cfg {
    /// The payload the FPGA wants.
    pub expected: Vec<u8>,
    /// How the FPGA assembles the received bits into bytes.
    pub order: BitOrder,
    /// nCONFIG rising to nSTATUS rising.
    pub nstatus_rise_us: u64,
    pub nstatus_never: bool,
    /// While receiving this byte index, pull nSTATUS low (a rejected
    /// stream), but only on the first `drop_attempts` attempts.
    pub drop_at_byte: Option<usize>,
    pub drop_attempts: u32,
    /// DCLKs after the last payload bit before CONF_DONE rises.
    pub done_after_clocks: u32,
    pub done_never: bool,
    /// DCLKs after CONF_DONE before the FPGA enters user mode.
    pub init_clocks: u32,
}

impl Cfg {
    pub fn new(expected: Vec<u8>) -> Self {
        Cfg {
            expected,
            order: BitOrder::LsbFirst,
            nstatus_rise_us: 230,
            nstatus_never: false,
            drop_at_byte: None,
            drop_attempts: 0,
            done_after_clocks: 0,
            done_never: false,
            init_clocks: 0,
        }
    }
}

#[derive(Default)]
pub struct World {
    pub cfg: Option<Cfg>,
    pub t_ns: u64,
    pub nconfig: bool,
    pub dclk: bool,
    pub data: bool,
    pub nstatus_rise_at: Option<u64>,
    pub errored: bool,
    /// nCONFIG rising edges seen.
    pub attempts: u32,
    pub rx: Vec<u8>,
    cur: u8,
    nbits: u32,
    /// DCLKs seen after the payload was complete, before CONF_DONE.
    pub padding_seen: u32,
    pub payload_complete: bool,
    pub done: bool,
    pub clocks_after_done: u32,
    pub user_mode: bool,
    /// The assembled payload equalled the expected one with each byte's
    /// bits reversed: the loader is sending the other bit order.
    pub bit_order_mismatch_seen: bool,
    /// Sum of nSTATUS-low pulses seen on nCONFIG falling.
    pub resets: u32,
    pub trace: Vec<Ev>,
}

fn rev(b: u8) -> u8 {
    b.reverse_bits()
}

impl World {
    fn nstatus(&self) -> bool {
        !self.errored && self.nstatus_rise_at.map_or(false, |r| self.t_ns >= r)
    }

    fn push(&mut self, e: Ev) {
        match (&e, self.trace.last_mut()) {
            (Ev::Nops(n), Some(Ev::Nops(m))) => *m += n,
            _ => self.trace.push(e),
        }
    }

    fn write(&mut self, pin: Pin, level: bool) {
        self.t_ns += 10;
        let old = match pin {
            Pin::Nconfig => self.nconfig,
            Pin::Dclk => self.dclk,
            Pin::Data => self.data,
        };
        if old == level {
            return;
        }
        self.push(Ev::Pin(pin, level));
        match pin {
            Pin::Data => self.data = level,
            Pin::Nconfig => {
                self.nconfig = level;
                if !level {
                    // Reset: forget everything about this attempt.
                    self.resets += 1;
                    self.nstatus_rise_at = None;
                    self.errored = false;
                    self.rx.clear();
                    self.cur = 0;
                    self.nbits = 0;
                    self.padding_seen = 0;
                    self.payload_complete = false;
                    self.done = false;
                    self.clocks_after_done = 0;
                    self.user_mode = false;
                } else {
                    self.attempts += 1;
                    let cfg = self.cfg.as_ref().unwrap();
                    if !cfg.nstatus_never {
                        self.nstatus_rise_at = Some(self.t_ns + cfg.nstatus_rise_us * 1000);
                    }
                }
            }
            Pin::Dclk => {
                self.dclk = level;
                if level {
                    self.rising_edge();
                }
            }
        }
    }

    fn rising_edge(&mut self) {
        if self.done {
            self.clocks_after_done += 1;
            if self.clocks_after_done >= self.cfg.as_ref().unwrap().init_clocks {
                self.user_mode = true;
            }
            return;
        }
        if !self.nconfig || !self.nstatus() {
            return; // not configuring: the FPGA ignores DCLK
        }
        let cfg = self.cfg.clone().unwrap();
        if self.payload_complete {
            self.padding_seen += 1;
            if !cfg.done_never && self.padding_seen >= cfg.done_after_clocks {
                self.finish_done();
            }
            return;
        }
        // Data bit.
        match cfg.order {
            BitOrder::LsbFirst => self.cur |= (self.data as u8) << self.nbits,
            BitOrder::MsbFirst => self.cur |= (self.data as u8) << (7 - self.nbits),
        }
        self.nbits += 1;
        if self.nbits == 8 {
            self.rx.push(self.cur);
            self.cur = 0;
            self.nbits = 0;
            let idx = self.rx.len() - 1;
            if cfg.drop_at_byte == Some(idx) && self.attempts <= cfg.drop_attempts {
                self.errored = true;
                return;
            }
            if self.rx.len() == cfg.expected.len() {
                self.payload_complete = true;
                if self.rx == cfg.expected {
                    if !cfg.done_never && cfg.done_after_clocks == 0 {
                        self.finish_done();
                    }
                } else {
                    let reversed: Vec<u8> = self.rx.iter().map(|b| rev(*b)).collect();
                    if reversed == cfg.expected {
                        self.bit_order_mismatch_seen = true;
                    }
                    self.errored = true; // CRC error: nSTATUS drops
                }
            }
        }
    }

    fn finish_done(&mut self) {
        self.done = true;
        if self.cfg.as_ref().unwrap().init_clocks == 0 {
            self.user_mode = true;
        }
    }
}

pub type Shared = Rc<RefCell<World>>;

pub fn world(cfg: Cfg) -> Shared {
    Rc::new(RefCell::new(World { cfg: Some(cfg), ..World::default() }))
}

pub struct SimPins(pub Shared);
pub struct SimClock(pub Shared);

impl PsPins for SimPins {
    fn set_nconfig(&mut self, high: bool) {
        self.0.borrow_mut().write(Pin::Nconfig, high)
    }
    fn set_dclk(&mut self, high: bool) {
        self.0.borrow_mut().write(Pin::Dclk, high)
    }
    fn set_data(&mut self, high: bool) {
        self.0.borrow_mut().write(Pin::Data, high)
    }
    fn nstatus_high(&mut self) -> bool {
        let mut w = self.0.borrow_mut();
        w.t_ns += 10;
        w.nstatus()
    }
    fn conf_done_high(&mut self) -> bool {
        let mut w = self.0.borrow_mut();
        w.t_ns += 10;
        w.done
    }
}

impl PsClock for SimClock {
    fn nops(&mut self, n: u32) {
        if n == 0 {
            return;
        }
        let mut w = self.0.borrow_mut();
        w.t_ns += n as u64 * 10;
        w.push(Ev::Nops(n));
    }
    fn delay_us(&mut self, us: u32) {
        let mut w = self.0.borrow_mut();
        w.t_ns += us as u64 * 1000;
        w.push(Ev::Delay(us));
    }
    fn now_us(&mut self) -> u64 {
        let mut w = self.0.borrow_mut();
        w.t_ns += 1000;
        w.t_ns / 1000
    }
}

/// A payload that looks like a bitstream: not periodic, no byte that is its
/// own bit-reversal everywhere.
pub fn payload(n: usize, seed: u32) -> Vec<u8> {
    let mut x = seed.wrapping_mul(2654435761).wrapping_add(1);
    (0..n)
        .map(|_| {
            x = x.wrapping_mul(1664525).wrapping_add(1013904223);
            (x >> 24) as u8
        })
        .collect()
}

/// A `PsSource` over a byte vector that can be told to break.
pub struct VecSource {
    pub data: Vec<u8>,
    pub pos: usize,
    pub broken_at: Option<usize>,
    pub broke: bool,
}

impl VecSource {
    pub fn new(data: Vec<u8>) -> Self {
        VecSource { data, pos: 0, broken_at: None, broke: false }
    }
}

impl Iterator for VecSource {
    type Item = u8;
    fn next(&mut self) -> Option<u8> {
        if Some(self.pos) == self.broken_at {
            self.broke = true;
            return None;
        }
        let b = self.data.get(self.pos).copied();
        self.pos += 1;
        b
    }
}

impl psload::PsSource for VecSource {
    fn failed(&self) -> bool {
        self.broke
    }
    fn produced(&self) -> u32 {
        self.pos as u32
    }
}
