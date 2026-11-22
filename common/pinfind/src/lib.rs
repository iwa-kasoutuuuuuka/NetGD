//! Decoder for the frames the AG16K pin-finder bitstream sends (`fpga/pinfind`).
//!
//! The pin-finder turns one FPGA ball at a time into an output and sends that
//! ball's index on it. Its clock is an on-chip oscillator (or a ring
//! oscillator, or whatever clock ball is tried) whose frequency is not known
//! to better than an order of magnitude, so the code carries its own time
//! base: every symbol is a whole number of *ticks* `T`, and the receiver only
//! ever compares pulse widths with each other.
//!
//! # Line code (format 1)
//!
//! "Active" is the level the FPGA drives low; "idle" is high (driven high in
//! push-pull mode, released to the receiver's pull-up in open-drain mode).
//!
//! ```text
//!  idle | SYNC: active 8T | idle 1T | bit: active 1T (0) or 3T (1), idle 1T | ... | idle >= 4T
//! ```
//!
//! Bits are sent MSB first, in whole bytes:
//!
//! * `idx`, `!idx` -- a 2-byte frame: ball index `idx` (0..=255) and its
//!   complement.
//! * optionally followed by the clock-probe section: `hdr`, then
//!   `count_hi, count_lo` per probed clock ball, then `csum` = the low 8 bits
//!   of the sum of every byte before it. `hdr` = `div_bits << 4 | win_bits`:
//!   each probed ball clocks a `div_bits`-bit counter in its own domain, and
//!   `count` is how many times that counter's top bit toggled during a window
//!   of `2^win_bits` ticks (see [`clock_hz`]).
//!
//! The receiver measures `T` over SYNC and the 1T idle after it (a SYNC is an
//! active pulse 4..=16 times longer than the idle pulse after it; `T` is the
//! pair's length / 9) and keeps tracking it over each bit + idle pair, so a clock that is
//! anywhere from a few hundred kHz to a few hundred MHz, and drifts, decodes
//! the same. Thresholds: active < 2T is a 0, 2T..5T is a 1, idle >= 3T ends
//! the frame. Anything else drops the frame.
//!
//! Time is any free-running `u32` counter (CPU cycles on the MCU, sample
//! numbers for a logic-analyser capture); only differences are used, with
//! wrapping arithmetic.
//!
//! Design: `docs/design/g1-ag16k-port.md` 2-4 method 2, `fpga/pinfind/README_ja.md`.

#![no_std]

/// Ticks in the active part of SYNC.
pub const SYNC_ACTIVE: u32 = 8;
/// Ticks of an active pulse for a 0 bit / a 1 bit.
pub const BIT0_ACTIVE: u32 = 1;
pub const BIT1_ACTIVE: u32 = 3;
/// Ticks of idle after SYNC and after every bit.
pub const IDLE: u32 = 1;
/// Ticks of idle that end a frame (including the 1T after the last bit).
pub const GAP: u32 = 4;

/// Most clock balls a frame can report. AG16KDDF256 has 11 dedicated clock
/// inputs.
pub const MAX_CLOCKS: usize = 12;
/// Largest frame: idx, !idx, hdr, 2 per clock, csum.
pub const MAX_FRAME_BYTES: usize = 4 + 2 * MAX_CLOCKS;

/// A frame that passed the length and check-byte tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    bytes: [u8; MAX_FRAME_BYTES],
    len: u8,
    /// The tick length `T` as the receiver last estimated it, in time units.
    pub unit: u32,
}

impl Frame {
    /// The ball index.
    pub fn index(&self) -> u8 {
        self.bytes[0]
    }

    /// The raw bytes, `idx` first.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }

    /// The clock-probe section, if the frame has one.
    pub fn clocks(&self) -> Option<ClockReport> {
        if self.len < 4 {
            return None;
        }
        let hdr = self.bytes[2];
        let n = (self.len as usize - 4) / 2;
        let mut counts = [0u16; MAX_CLOCKS];
        for (i, c) in counts.iter_mut().enumerate().take(n) {
            *c = u16::from_be_bytes([self.bytes[3 + 2 * i], self.bytes[4 + 2 * i]]);
        }
        Some(ClockReport { div_bits: hdr >> 4, win_bits: hdr & 0x0f, n: n as u8, counts })
    }

    /// Check a byte string as a frame. `None` if the length is not one the
    /// pin-finder sends, `!idx` does not match or the sum is wrong.
    pub fn from_bytes(b: &[u8], unit: u32) -> Option<Frame> {
        let len = b.len();
        if len < 2 || len > MAX_FRAME_BYTES || len == 3 || (len > 2 && len % 2 != 0) {
            return None;
        }
        if b[1] != !b[0] {
            return None;
        }
        if len > 2 {
            let sum = b[..len - 1].iter().fold(0u8, |a, &x| a.wrapping_add(x));
            if sum != b[len - 1] {
                return None;
            }
        }
        let mut bytes = [0u8; MAX_FRAME_BYTES];
        bytes[..len].copy_from_slice(b);
        Some(Frame { bytes, len: len as u8, unit })
    }
}

/// The clock-probe section of a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockReport {
    pub div_bits: u8,
    pub win_bits: u8,
    pub n: u8,
    pub counts: [u16; MAX_CLOCKS],
}

impl ClockReport {
    /// Frequency of probed clock `i` in Hz, given the frame's tick length
    /// `unit` in time units and the time base `time_hz` (time units per
    /// second). 0 for a silent ball. See [`clock_hz`].
    pub fn hz(&self, i: usize, unit: u32, time_hz: u32) -> u64 {
        clock_hz(self.counts[i], unit, time_hz, self.div_bits, self.win_bits)
    }
}

/// Frequency of a probed clock ball.
///
/// The ball clocks a `div_bits`-bit counter, whose top bit toggles once every
/// `2^(div_bits-1)` input cycles; `count` toggles were seen in `2^win_bits`
/// ticks of `unit` time units each, and the time base runs at `time_hz`.
/// A saturated count (`0xffff`) means "at least this much".
pub fn clock_hz(count: u16, unit: u32, time_hz: u32, div_bits: u8, win_bits: u8) -> u64 {
    if unit == 0 || div_bits == 0 {
        return 0;
    }
    let cycles = (count as u64) << (div_bits - 1);
    let window = (unit as u64) << win_bits; // in time units
    cycles * time_hz as u64 / window
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
enum State {
    /// Waiting for a SYNC.
    Hunt,
    /// Inside a frame, collecting bits.
    Bits,
}

/// Streaming decoder for one line.
///
/// Feed it every edge with [`edge`](Self::edge) and call [`poll`](Self::poll)
/// now and then (the last frame of a burst ends in idle that no edge closes).
#[derive(Clone, Copy, Debug)]
pub struct Decoder {
    /// Which electrical level is "active": `false` for the normal line code
    /// (active low), `true` for a line seen through an inverter.
    active_high: bool,
    state: State,
    /// Time and logical level ("active" = true) of the last edge; `None`
    /// before the first edge.
    last: Option<(u32, bool)>,
    /// Length of the active pulse that just ended, while it may be a SYNC.
    pending_sync: Option<u32>,
    /// Running estimate of T, in time units.
    unit: u32,
    /// Length and tick count of the last bit's active pulse, for tracking.
    last_bit: (u32, u32),
    nbits: u16,
    buf: [u8; MAX_FRAME_BYTES],
    /// Frames dropped after a SYNC (bad pulse, bad check byte).
    pub dropped: u32,
}

impl Decoder {
    pub const fn new(active_high: bool) -> Self {
        Decoder {
            active_high,
            state: State::Hunt,
            last: None,
            pending_sync: None,
            unit: 0,
            last_bit: (0, 0),
            nbits: 0,
            buf: [0; MAX_FRAME_BYTES],
            dropped: 0,
        }
    }

    /// True while a frame is being received; only then can [`poll`](Self::poll)
    /// return something.
    pub fn in_frame(&self) -> bool {
        self.state == State::Bits
    }

    fn drop_frame(&mut self) {
        if self.state == State::Bits {
            self.dropped += 1;
        }
        self.state = State::Hunt;
        self.nbits = 0;
    }

    fn finish(&mut self) -> Option<Frame> {
        self.state = State::Hunt;
        let n = self.nbits as usize;
        self.nbits = 0;
        let f = if n % 8 == 0 { Frame::from_bytes(&self.buf[..n / 8], self.unit) } else { None };
        if f.is_none() {
            self.dropped += 1;
        }
        f
    }

    fn push_bit(&mut self, bit: bool) {
        let n = self.nbits as usize;
        if n >= MAX_FRAME_BYTES * 8 {
            self.drop_frame();
            return;
        }
        let byte = &mut self.buf[n / 8];
        if n % 8 == 0 {
            *byte = 0;
        }
        *byte = (*byte << 1) | bit as u8;
        self.nbits += 1;
    }

    /// The line changed to electrical level `high` at time `now`. Returns a
    /// frame when this edge completed one.
    pub fn edge(&mut self, now: u32, high: bool) -> Option<Frame> {
        let active = high == self.active_high;
        let prev = self.last.replace((now, active));
        let (t0, was_active) = match prev {
            Some(p) => p,
            None => return None,
        };
        if was_active == active {
            // Not an edge (a repeated sample); keep the original start time.
            self.last = Some((t0, was_active));
            return None;
        }
        let len = now.wrapping_sub(t0);
        if was_active {
            self.end_of_active(len);
            None
        } else {
            self.end_of_idle(len)
        }
    }

    /// An active pulse of `len` time units just ended.
    fn end_of_active(&mut self, len: u32) {
        match self.state {
            State::Hunt => self.pending_sync = Some(len),
            State::Bits => {
                let t = self.unit;
                if len.saturating_mul(2) < t {
                    self.drop_frame(); // glitch
                    self.pending_sync = None;
                } else if len < 2 * t {
                    self.last_bit = (len, BIT0_ACTIVE);
                    self.push_bit(false);
                } else if len < 5 * t {
                    self.last_bit = (len, BIT1_ACTIVE);
                    self.push_bit(true);
                } else {
                    // Too long for a bit: maybe a new SYNC (the burst was cut
                    // and a new one started).
                    self.drop_frame();
                    self.pending_sync = Some(len);
                }
            }
        }
    }

    /// An idle pulse of `len` time units just ended.
    fn end_of_idle(&mut self, len: u32) -> Option<Frame> {
        match self.state {
            State::Hunt => {
                if let Some(s) = self.pending_sync.take() {
                    // 8T active + 1T idle. The ratio test is loose (an edge
                    // can be off by a fraction of a tick); T is taken over
                    // the whole 9T, where that error is spread thin.
                    if len > 0 && s >= 4 * len && s <= 16 * len {
                        let unit = (s as u64 + len as u64) / (SYNC_ACTIVE + IDLE) as u64;
                        if unit > 0 {
                            self.state = State::Bits;
                            self.unit = unit as u32;
                            self.nbits = 0;
                        }
                    }
                }
                None
            }
            State::Bits => {
                let t = self.unit;
                if len.saturating_mul(2) < t {
                    self.drop_frame();
                    None
                } else if len < 2 * t {
                    // Track slow drift: the last bit plus this idle is a
                    // whole number of ticks (2T or 4T), which jitters less
                    // than the 1T idle alone.
                    let (a, ticks) = self.last_bit;
                    let est = (a as u64 + len as u64) / (ticks + IDLE) as u64;
                    self.unit = ((3 * t as u64 + est) / 4).max(1) as u32;
                    None
                } else if len >= 3 * t {
                    self.finish()
                } else {
                    self.drop_frame();
                    None
                }
            }
        }
    }

    /// Close a frame whose final idle has lasted long enough. Call it
    /// periodically; it is cheap when [`in_frame`](Self::in_frame) is false.
    pub fn poll(&mut self, now: u32) -> Option<Frame> {
        if self.state != State::Bits {
            return None;
        }
        match self.last {
            Some((t0, false)) if now.wrapping_sub(t0) >= 3 * self.unit => self.finish(),
            _ => None,
        }
    }
}

/// What one line has received: up to four distinct indices with their
/// counts, so a stray decode or a second ball on the same net shows up
/// instead of being overwritten.
#[derive(Clone, Copy, Debug, Default)]
pub struct Tally {
    hits: [(u8, u16); 4],
    used: u8,
    /// Frames whose index did not fit in the four slots.
    pub overflow: u16,
}

impl Tally {
    pub const fn new() -> Self {
        Tally { hits: [(0, 0); 4], used: 0, overflow: 0 }
    }

    /// Count `idx`. Returns true the first time this index is seen.
    pub fn add(&mut self, idx: u8) -> bool {
        for h in &mut self.hits[..self.used as usize] {
            if h.0 == idx {
                h.1 = h.1.saturating_add(1);
                return false;
            }
        }
        if (self.used as usize) < self.hits.len() {
            self.hits[self.used as usize] = (idx, 1);
            self.used += 1;
            true
        } else {
            self.overflow = self.overflow.saturating_add(1);
            false
        }
    }

    /// `(index, count)` pairs, in the order first seen.
    pub fn hits(&self) -> &[(u8, u16)] {
        &self.hits[..self.used as usize]
    }

    /// The index with the most hits, if it has at least `min_share` percent
    /// of all hits on this line.
    pub fn dominant(&self, min_share: u32) -> Option<u8> {
        let total: u32 = self.hits().iter().map(|h| h.1 as u32).sum::<u32>() + self.overflow as u32;
        let best = self.hits().iter().max_by_key(|h| h.1)?;
        if total > 0 && best.1 as u32 * 100 >= min_share * total {
            Some(best.0)
        } else {
            None
        }
    }
}

/// Encoder for the line code, used by the tests and by anyone who wants to
/// generate a reference waveform. Yields `(active, ticks)` pulses, starting
/// with the SYNC and ending with the closing idle of [`GAP`] ticks.
pub fn encode(bytes: &[u8], mut out: impl FnMut(bool, u32)) {
    out(true, SYNC_ACTIVE);
    out(false, IDLE);
    let nbits = bytes.len() * 8;
    for i in 0..nbits {
        let bit = bytes[i / 8] >> (7 - i % 8) & 1 != 0;
        out(true, if bit { BIT1_ACTIVE } else { BIT0_ACTIVE });
        out(false, if i + 1 == nbits { GAP } else { IDLE });
    }
}

/// The bytes of a frame for ball `idx`, with an optional clock section.
/// Returns the length written into `buf`.
pub fn frame_bytes(idx: u8, clocks: Option<(u8, u8, &[u16])>, buf: &mut [u8; MAX_FRAME_BYTES]) -> usize {
    buf[0] = idx;
    buf[1] = !idx;
    let Some((div_bits, win_bits, counts)) = clocks else { return 2 };
    assert!(counts.len() <= MAX_CLOCKS);
    buf[2] = div_bits << 4 | (win_bits & 0x0f);
    for (i, c) in counts.iter().enumerate() {
        buf[3 + 2 * i..5 + 2 * i].copy_from_slice(&c.to_be_bytes());
    }
    let n = 3 + 2 * counts.len();
    buf[n] = buf[..n].iter().fold(0u8, |a, &x| a.wrapping_add(x));
    n + 1
}
