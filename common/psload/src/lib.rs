//! FPGA passive-serial (PS) configuration, as pure logic.
//!
//! The firmware owns the GPIO and the time base; this crate owns *what to do
//! with them*: the sequence, the per-FPGA timing and bit order (`PsProfile`),
//! the timeouts that turn a dead FPGA into an error instead of a hang, and
//! the retry policy. Pins and time come in through two small traits, so the
//! same code runs on the MCU and against a simulated FPGA in `tests/`.
//!
//! Design: `docs/research/ag16k-config.md` section 6.
//!
//! The loop in [`load_once`] is, for [`CYCLONE2`], the loader the firmware
//! shipped before this crate existed (`cyclone.rs`, R32): same order of pin
//! writes, same busy-wait between them. `tests/equivalence.rs` holds a
//! literal port of the old loop and compares the two waveforms.

#![no_std]

/// The five lines the loader drives or watches. Levels are logical: `true` is
/// electrically high. Reads take `&mut self` so a simulator can advance with
/// them.
pub trait PsPins {
    fn set_nconfig(&mut self, high: bool);
    fn set_dclk(&mut self, high: bool);
    fn set_data(&mut self, high: bool);
    fn nstatus_high(&mut self) -> bool;
    fn conf_done_high(&mut self) -> bool;
}

/// Time. Two scales on purpose: the per-bit waits are a handful of CPU
/// cycles (`nops`), the protocol waits are microseconds and up.
pub trait PsClock {
    /// Spin for `n` no-ops (one CPU cycle each on the MCU).
    fn nops(&mut self, n: u32);
    /// Busy-wait at least `us` microseconds. Must be accurate below the
    /// system tick (the firmware's tick is 1 ms).
    fn delay_us(&mut self, us: u32);
    /// A monotonic microsecond counter, for timeouts only (it may be as
    /// coarse as 1 ms). Must advance between calls in a polling loop.
    fn now_us(&mut self) -> u64;
}

/// Where the configuration bytes come from. A plain iterator, plus a way to
/// ask whether it stopped because the data ran out or because it broke (the
/// LZMA decoder stops on a corrupt stream and cannot say so through
/// `Iterator`).
pub trait PsSource: Iterator<Item = u8> {
    /// True if the source ended early because of an internal failure.
    fn failed(&self) -> bool {
        false
    }
    /// Bytes produced so far (for the error report only).
    fn produced(&self) -> u32 {
        0
    }
}

impl<I: PsSource + ?Sized> PsSource for &mut I {
    fn failed(&self) -> bool {
        (**self).failed()
    }
    fn produced(&self) -> u32 {
        (**self).produced()
    }
}

/// Which bit of each byte goes on DATA0 first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum BitOrder {
    /// Bit 0 first. Altera `.rbf` files, as clocked into a Cyclone.
    LsbFirst,
    /// Bit 7 first. AGM `.bin` files (they are the bit-reversed `.rbf`).
    MsbFirst,
}

/// Everything that differs between FPGAs. All times are microseconds unless
/// a name says otherwise; `0` means "do not wait" (or "do not check") as
/// noted per field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PsProfile {
    pub name: &'static str,
    pub bit_order: BitOrder,
    /// How long nCONFIG is held low to (re)start configuration.
    pub ncfg_low_us: u32,
    /// After nCONFIG falls, how long to wait for nSTATUS to be seen low
    /// before going on. 0 = do not look. Timing out is not an error: it is
    /// only a confirmation that the FPGA took the reset, and an FPGA that
    /// never answers is caught by the nSTATUS-high wait next.
    pub nstatus_low_wait_us: u32,
    /// After nCONFIG rises, the longest wait for nSTATUS to go high.
    /// Exceeding it is `PsError::NstatusStuckLow`.
    pub nstatus_high_timeout_us: u32,
    /// nSTATUS high to the first DCLK. 0 = none.
    pub post_nstatus_us: u32,
    /// No-ops after DCLK falls / after DATA is set / after DCLK rises (the
    /// last one is the whole DCLK-high time, so it is a single count here).
    pub nops_clk_low: u32,
    pub nops_data: u32,
    pub nops_clk_high: u32,
    /// DCLKs (DATA = 0) sent after CONF_DONE rises, before returning.
    pub extra_clocks: u32,
    /// When the data is exhausted and CONF_DONE is still low: how many
    /// zero-bits to send before giving up (`PsError::ConfDoneTimeout`).
    pub pad_cap_clocks: u32,
    /// Also watch nSTATUS while sending the trailing zero clocks, so a
    /// stream the FPGA rejected at its CRC check fails at once as
    /// `NstatusDropped` instead of after `pad_cap_clocks`. Off for Cyclone
    /// II, whose padding loop has never looked.
    pub check_nstatus_in_padding: bool,
    /// Retries after a failed attempt (so attempts = 1 + retries).
    pub retries: u8,
    /// Leave DCLK and DATA0 low on exit (they are user I/O afterwards on
    /// the AG16K). False = leave them as the last bit left them.
    pub park_low: bool,
}

/// Cyclone II (EP2C5), GDEMU v5.5 and the v5.15b clones.
///
/// The wire behaviour is exactly the loader this replaced: LSB first, DCLK
/// low 5 no-ops, DATA 5 no-ops, DCLK high 10 no-ops, no wait for nSTATUS
/// low, no wait after nSTATUS high, no extra clocks, DCLK/DATA left where
/// the last bit put them. What is new is only what happens when the FPGA
/// does not answer: caps on the two waits that used to be unbounded, and a
/// `Result` where there used to be a `panic!`. `ncfg_low_us` is 1 ms as
/// before (it was `Timer::after(1 ms)`, which on a 1 kHz tick is 1-2 ms).
pub const CYCLONE2: PsProfile = PsProfile {
    name: "cyclone2",
    bit_order: BitOrder::LsbFirst,
    ncfg_low_us: 1_000,
    nstatus_low_wait_us: 0,
    nstatus_high_timeout_us: 500_000,
    post_nstatus_us: 0,
    nops_clk_low: 5,
    nops_data: 5,
    nops_clk_high: 10,
    extra_clocks: 0,
    pad_cap_clocks: 16_384,
    check_nstatus_in_padding: false,
    retries: 3,
    park_low: false,
};

/// AGM AG16KDDF256. The datasheets give no PS timing, so these are Cyclone
/// IV E values with wide margins (ag16k-config.md 1-4 and 6). LSB-first
/// assumes the file is the `.rbf`; if the board never reaches CONF_DONE with
/// a correct file, `bit_order` is the first thing to flip.
pub const AG16K: PsProfile = PsProfile {
    name: "ag16k",
    bit_order: BitOrder::LsbFirst,
    ncfg_low_us: 1_000,
    nstatus_low_wait_us: 10_000,
    nstatus_high_timeout_us: 500_000,
    post_nstatus_us: 50,
    nops_clk_low: 0,
    nops_data: 0,
    nops_clk_high: 0,
    extra_clocks: 64,
    pad_cap_clocks: 4_096,
    check_nstatus_in_padding: true,
    retries: 3,
    park_low: true,
};

impl PsProfile {
    pub const fn with_bit_order(mut self, o: BitOrder) -> Self {
        self.bit_order = o;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PsError {
    /// There is no bitstream in this firmware image (board with no FPGA
    /// file at build time). Never retried.
    NoBitstream,
    /// nSTATUS did not go high within the profile's limit after nCONFIG
    /// rose: no FPGA, no power, or wiring.
    NstatusStuckLow,
    /// nSTATUS went low while data was being clocked in (the FPGA rejected
    /// the stream). `byte` is how many bytes had been sent.
    NstatusDropped { byte: u32 },
    /// The whole stream and `padding` extra zero clocks went in and
    /// CONF_DONE never rose: wrong data, wrong bit order, or a short file.
    ConfDoneTimeout { padding: u32 },
    /// The source broke (corrupt compressed stream). Never retried: it would
    /// break at the same place.
    Decode { produced: u32 },
}

impl PsError {
    /// Worth another attempt with a fresh source?
    pub const fn retryable(&self) -> bool {
        matches!(
            self,
            PsError::NstatusStuckLow | PsError::NstatusDropped { .. } | PsError::ConfDoneTimeout { .. }
        )
    }

    /// Stable small number for a blink code, 1-based.
    pub const fn code(&self) -> u8 {
        match self {
            PsError::NstatusStuckLow => 1,
            PsError::NstatusDropped { .. } => 2,
            PsError::ConfDoneTimeout { .. } => 3,
            PsError::Decode { .. } => 4,
            PsError::NoBitstream => 5,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct LoadStats {
    /// Bytes taken from the source on the successful attempt.
    pub bytes: u32,
    /// Zero-bits sent after the data ran out, before CONF_DONE rose.
    pub padding: u32,
    /// 1 = first try worked.
    pub attempts: u8,
    /// CONF_DONE rose before the source was exhausted.
    pub early_done: bool,
}

#[inline(always)]
fn send_bit<P: PsPins, C: PsClock>(p: &PsProfile, pins: &mut P, clk: &mut C, bit: bool) {
    pins.set_dclk(false);
    clk.nops(p.nops_clk_low);
    pins.set_data(bit);
    clk.nops(p.nops_data);
    pins.set_dclk(true);
    clk.nops(p.nops_clk_high);
}

#[inline(always)]
fn send_byte<P: PsPins, C: PsClock>(p: &PsProfile, pins: &mut P, clk: &mut C, byte: u8) {
    match p.bit_order {
        BitOrder::LsbFirst => {
            for i in 0..8 {
                send_bit(p, pins, clk, byte & (1 << i) != 0);
            }
        }
        BitOrder::MsbFirst => {
            for i in (0..8).rev() {
                send_bit(p, pins, clk, byte & (1 << i) != 0);
            }
        }
    }
}

/// One configuration attempt: nCONFIG pulse, wait for nSTATUS, stream, pad,
/// trailing clocks. Every wait is bounded: by the caps in `p`, or by the
/// (finite) source.
///
/// `inline(always)`: the firmware calls this from one place with a constant
/// profile, and the per-bit no-op counts must fold into immediates there.
#[inline(always)]
pub fn load_once<P, C, I>(p: &PsProfile, pins: &mut P, clk: &mut C, src: I) -> Result<LoadStats, PsError>
where
    P: PsPins,
    C: PsClock,
    I: Iterator<Item = u8>,
{
    // (Re)start: nCONFIG low, DCLK and DATA0 low. On the first attempt the
    // MCU has already driven all three low, so these writes change nothing.
    pins.set_nconfig(false);
    pins.set_dclk(false);
    pins.set_data(false);
    clk.delay_us(p.ncfg_low_us);

    if p.nstatus_low_wait_us != 0 {
        let t0 = clk.now_us();
        while pins.nstatus_high() && clk.now_us().wrapping_sub(t0) < p.nstatus_low_wait_us as u64 {}
    }

    pins.set_nconfig(true);
    let t0 = clk.now_us();
    while !pins.nstatus_high() {
        if clk.now_us().wrapping_sub(t0) >= p.nstatus_high_timeout_us as u64 {
            return Err(PsError::NstatusStuckLow);
        }
    }
    if p.post_nstatus_us != 0 {
        clk.delay_us(p.post_nstatus_us);
    }

    let mut stats = LoadStats { attempts: 1, ..LoadStats::default() };
    for byte in src {
        if pins.conf_done_high() {
            stats.early_done = true;
            break;
        }
        if !pins.nstatus_high() {
            return Err(PsError::NstatusDropped { byte: stats.bytes });
        }
        send_byte(p, pins, clk, byte);
        stats.bytes += 1;
    }

    // Data exhausted: keep clocking zeros until CONF_DONE (the FPGA may need
    // a few clocks after the last data bit to finish).
    while !pins.conf_done_high() {
        if p.check_nstatus_in_padding && !pins.nstatus_high() {
            return Err(PsError::NstatusDropped { byte: stats.bytes });
        }
        if stats.padding >= p.pad_cap_clocks {
            return Err(PsError::ConfDoneTimeout { padding: stats.padding });
        }
        send_bit(p, pins, clk, false);
        stats.padding += 1;
    }

    for _ in 0..p.extra_clocks {
        send_bit(p, pins, clk, false);
    }
    if p.park_low {
        pins.set_dclk(false);
        pins.set_data(false);
    }
    Ok(stats)
}

/// What a failed load reports: the last error and how many attempts ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct LoadFailure {
    pub error: PsError,
    pub attempts: u8,
}

/// `load_once` with the profile's retry policy.
///
/// `make_source` builds a fresh source for each attempt (the bitstream is
/// read from the start every time; for the LZMA source that means a new
/// decoder over a re-borrowed scratch buffer). The source of attempt *n* is
/// dropped before `make_source` is called for attempt *n+1*, so a source
/// may borrow storage that the next one re-borrows.
///
/// `on_attempt_failed` is called after every failed attempt, including the
/// last (for logging).
#[inline(always)]
pub fn load_with_retries<P, C, S, F, L>(
    p: &PsProfile,
    pins: &mut P,
    clk: &mut C,
    mut make_source: F,
    mut on_attempt_failed: L,
) -> Result<LoadStats, LoadFailure>
where
    P: PsPins,
    C: PsClock,
    S: PsSource,
    F: FnMut() -> S,
    L: FnMut(u8, &PsError),
{
    let mut attempt: u8 = 0;
    loop {
        attempt += 1;
        let mut src = make_source();
        let r = load_once(p, pins, clk, &mut src);
        // A broken source overrides whatever the FPGA side said: the FPGA
        // could only have complained about the garbage it was fed.
        let r = if src.failed() { Err(PsError::Decode { produced: src.produced() }) } else { r };
        drop(src);
        match r {
            Ok(mut s) => {
                s.attempts = attempt;
                return Ok(s);
            }
            Err(e) => {
                on_attempt_failed(attempt, &e);
                if !e.retryable() || attempt > p.retries {
                    return Err(LoadFailure { error: e, attempts: attempt });
                }
            }
        }
    }
}
