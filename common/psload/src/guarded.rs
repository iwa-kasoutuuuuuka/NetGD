//! A one-shot PS load that proves the FPGA answers before it drives anything
//! but nCONFIG. Used by the `pinfind` firmware on the GDEMU 6.0, whose pin map
//! is only partly confirmed; the normal firmware keeps [`crate::load_with_retries`].
//!
//! Order, and what each failure leaves behind:
//!
//! 1. [`precheck`]: run marker, bitstream present, full decode, length, CRC.
//!    No line is an output yet.
//! 2. [`guarded_load`] records the run (so a reset cannot start a second one),
//!    then does the nCONFIG/nSTATUS handshake. Only nCONFIG moves, and only
//!    by sinking it low: released, the line is an input held up by a pull-up.
//! 3. Only after the handshake, and only if both read high in every sample
//!    with their pull-ups on, do DCLK and DATA0 become outputs.
//! 4. Every exit releases DCLK and DATA0 *first*, then settles nCONFIG
//!    ([`GuardedError::final_nconfig`]).

use crate::{stream, LoadStats, PsClock, PsError, PsPins, PsProfile, PsSource};

/// Polling step of the handshake. The bounds are counted in delays, not read
/// from the system tick, so they hold even if the tick has stopped.
const POLL_STEP_US: u32 = 10;

/// Reads of DATA0 / DCLK before they may become outputs, `POLL_STEP_US` apart.
pub const DATA_LINE_SAMPLES: u32 = 8;

/// DATA0 and DCLK as inputs with their pull-ups on. A field is true only if
/// that line read high in every sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct DataLineLevels {
    pub data0_high: bool,
    pub dclk_high: bool,
}

impl DataLineLevels {
    /// A line that something holds low must not be driven against it.
    pub const fn safe_to_enable(&self) -> bool {
        self.data0_high && self.dclk_high
    }
}

/// The line control a guarded load needs on top of [`PsPins`].
pub trait GuardedPsPins: PsPins {
    /// Record that a run started. Called before the first drive of any line.
    fn mark_run_started(&mut self);
    /// Has the deadline passed? Unlike `nstatus_high()`, which reads low after
    /// a trip, this tells a trip apart from a real low. Read it *after* a
    /// sample of nSTATUS: a trip that lands between the two is then seen.
    fn tripped(&mut self) -> bool;
    /// Sink nCONFIG low. Never drives it high.
    fn assert_nconfig(&mut self);
    /// Stop sinking nCONFIG: input, raised by the pull-up.
    fn release_nconfig(&mut self);
    /// As `release_nconfig`, and no later fault path may sink it again.
    fn disarm_nconfig(&mut self);
    /// DATA0 as an input, right now.
    fn data0_high(&mut self) -> bool;
    /// DCLK as an input, right now.
    fn dclk_high(&mut self) -> bool;
    /// The check before enabling passed, with these levels (for the log).
    fn data_lines_passed(&mut self, levels: DataLineLevels);
    /// Make DCLK and DATA0 outputs, both low. `false` = refused (the deadline
    /// has passed) and neither became an output.
    fn enable_data_outputs(&mut self) -> bool;
    /// DCLK and DATA0 back to inputs. `keep_dclk` leaves DCLK an output (a
    /// design clocked from that ball, after a successful load only).
    fn release_data_outputs(&mut self, keep_dclk: bool);
}

/// Where nCONFIG ends up after a failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum NconfigFinal {
    /// Input, not driven: the line is not proven to be nCONFIG, or was never
    /// touched.
    Released,
    /// Sunk low: the FPGA is held in reset with its user I/O off.
    HeldLow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PrecheckError {
    AlreadyRan,
    NoBitstream,
    Decode { produced: u32 },
    Length { got: u32, want: u32 },
    Cksum { got: u32, want: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum GuardedError {
    /// nSTATUS was already low before nCONFIG was touched, so its falling
    /// would prove nothing. Nothing was driven.
    NstatusLowBeforeReset,
    /// nCONFIG low did not bring nSTATUS low.
    NstatusStuckHigh,
    /// nSTATUS fell, then read high again while nCONFIG was still sunk. A
    /// real FPGA holds it low for the whole reset, so nothing shows PA25
    /// controls PB2 (a coupled line would do this).
    NstatusRoseWhileLow,
    /// The deadline had passed before nCONFIG was touched. Nothing was driven.
    DeadlineBeforeReset,
    /// The deadline passed after nCONFIG was sunk, during the handshake. PB2
    /// is not shown to have moved; the deadline hook has left nCONFIG low.
    DeadlineTripped,
    /// nSTATUS followed nCONFIG down but did not rise after the release.
    NstatusStuckLow,
    /// CONF_DONE read high after the reset: clocking would be meaningless.
    ConfDoneHighAfterReset,
    /// DATA0 or DCLK did not read high in every sample with its pull-up on:
    /// something may be holding it. Neither was enabled.
    DataLinesNotHigh(DataLineLevels),
    /// The deadline passed before DCLK / DATA0 were enabled.
    OutputsRefused,
    /// The stream itself failed.
    Load(PsError),
    /// CONF_DONE rose before the whole bitstream was sent.
    EarlyDone { sent: u32, want: u32 },
    /// The source ended short of the expected length.
    ShortTransfer { sent: u32, want: u32 },
}

impl GuardedError {
    /// The safe state of nCONFIG after this failure.
    ///
    /// Held low whenever PB2 has followed PA25 (so PA25 is shown to be
    /// nCONFIG) or any clock may have gone out: in reset the FPGA's user I/O
    /// are off, so a partial configuration cannot drive anything. Released
    /// when the handshake gave no such proof: no DCLK was sent, so nothing
    /// can be half-configured by this run, and there is no case for sinking
    /// a line of unproven function until power-off.
    pub const fn final_nconfig(&self) -> NconfigFinal {
        match self {
            GuardedError::NstatusLowBeforeReset
            | GuardedError::NstatusStuckHigh
            | GuardedError::NstatusRoseWhileLow
            | GuardedError::DeadlineBeforeReset => NconfigFinal::Released,
            _ => NconfigFinal::HeldLow,
        }
    }
}

/// POSIX `cksum` CRC (polynomial 0x04C11DB7, MSB first, length appended,
/// complemented), the value `build.rs` checks the `.rbf` against.
#[derive(Clone, Copy)]
pub struct Cksum {
    crc: u32,
    len: u32,
}

impl Cksum {
    pub const fn new() -> Self {
        Cksum { crc: 0, len: 0 }
    }

    #[inline]
    fn step(crc: u32, b: u8) -> u32 {
        let mut c = crc ^ ((b as u32) << 24);
        for _ in 0..8 {
            c = if c & 0x8000_0000 != 0 { (c << 1) ^ 0x04C1_1DB7 } else { c << 1 };
        }
        c
    }

    #[inline]
    pub fn update(&mut self, b: u8) {
        self.crc = Self::step(self.crc, b);
        self.len = self.len.wrapping_add(1);
    }

    pub fn finish(&self) -> u32 {
        let mut crc = self.crc;
        let mut n = self.len;
        while n != 0 {
            crc = Self::step(crc, n as u8);
            n >>= 8;
        }
        !crc
    }
}

impl Default for Cksum {
    fn default() -> Self {
        Self::new()
    }
}

/// Every check that needs no line to move. Decodes the whole bitstream once.
pub fn precheck<S, F>(
    already_ran: bool,
    present: bool,
    raw_len: usize,
    want_cksum: u32,
    make_source: &mut F,
) -> Result<(), PrecheckError>
where
    S: PsSource,
    F: FnMut() -> S,
{
    if already_ran {
        return Err(PrecheckError::AlreadyRan);
    }
    if !present {
        return Err(PrecheckError::NoBitstream);
    }
    let mut src = make_source();
    let mut sum = Cksum::new();
    let mut n: u32 = 0;
    for b in &mut src {
        sum.update(b);
        n = n.wrapping_add(1);
    }
    if src.failed() {
        return Err(PrecheckError::Decode { produced: src.produced() });
    }
    if n as usize != raw_len {
        return Err(PrecheckError::Length { got: n, want: raw_len as u32 });
    }
    let got = sum.finish();
    if got != want_cksum {
        return Err(PrecheckError::Cksum { got, want: want_cksum });
    }
    Ok(())
}

enum Wait {
    Seen,
    Timeout,
    Tripped,
}

/// Samples nSTATUS first and the deadline after, so a trip that makes
/// `nstatus_high()` read low is never taken for a real low.
fn wait_nstatus<P: GuardedPsPins, C: PsClock>(pins: &mut P, clk: &mut C, want_high: bool, timeout_us: u32) -> Wait {
    for _ in 0..=timeout_us / POLL_STEP_US {
        let high = pins.nstatus_high();
        if pins.tripped() {
            return Wait::Tripped;
        }
        if high == want_high {
            return Wait::Seen;
        }
        clk.delay_us(POLL_STEP_US);
    }
    let high = pins.nstatus_high();
    if pins.tripped() {
        Wait::Tripped
    } else if high == want_high {
        Wait::Seen
    } else {
        Wait::Timeout
    }
}

/// nSTATUS samples spread over the nCONFIG-low period.
const LOW_SAMPLES: u32 = 4;

/// nCONFIG low, nSTATUS must follow and stay low for the whole low period;
/// nCONFIG released, nSTATUS must rise. Every wait is bounded by the profile.
/// A deadline trip is never taken for nSTATUS moving. Touches nCONFIG only.
pub fn handshake<P: GuardedPsPins, C: PsClock>(p: &PsProfile, pins: &mut P, clk: &mut C) -> Result<(), GuardedError> {
    let high = pins.nstatus_high();
    if pins.tripped() {
        return Err(GuardedError::DeadlineBeforeReset);
    }
    if !high {
        return Err(GuardedError::NstatusLowBeforeReset);
    }
    pins.assert_nconfig();
    match wait_nstatus(pins, clk, false, p.nstatus_low_wait_us) {
        Wait::Seen => {}
        Wait::Timeout => return Err(GuardedError::NstatusStuckHigh),
        Wait::Tripped => return Err(GuardedError::DeadlineTripped),
    }
    // The low period, in slices; nSTATUS is sampled after each, the last one
    // after the whole `ncfg_low_us`.
    let slice = p.ncfg_low_us / LOW_SAMPLES;
    for i in 0..LOW_SAMPLES {
        let us = if i + 1 == LOW_SAMPLES { p.ncfg_low_us - slice * (LOW_SAMPLES - 1) } else { slice };
        clk.delay_us(us);
        let high = pins.nstatus_high();
        if pins.tripped() {
            return Err(GuardedError::DeadlineTripped);
        }
        if high {
            return Err(GuardedError::NstatusRoseWhileLow);
        }
    }
    pins.release_nconfig();
    match wait_nstatus(pins, clk, true, p.nstatus_high_timeout_us) {
        Wait::Seen => Ok(()),
        Wait::Timeout => Err(GuardedError::NstatusStuckLow),
        Wait::Tripped => Err(GuardedError::DeadlineTripped),
    }
}

/// Sample DATA0 and DCLK [`DATA_LINE_SAMPLES`] times. Reads only.
pub fn sample_data_lines<P: GuardedPsPins, C: PsClock>(pins: &mut P, clk: &mut C) -> DataLineLevels {
    let mut levels = DataLineLevels { data0_high: true, dclk_high: true };
    for i in 0..DATA_LINE_SAMPLES {
        if i != 0 {
            clk.delay_us(POLL_STEP_US);
        }
        levels.data0_high &= pins.data0_high();
        levels.dclk_high &= pins.dclk_high();
    }
    levels
}

fn attempt<P, C, S, F>(
    p: &PsProfile,
    pins: &mut P,
    clk: &mut C,
    raw_len: usize,
    make_source: &mut F,
) -> Result<LoadStats, GuardedError>
where
    P: GuardedPsPins,
    C: PsClock,
    S: PsSource,
    F: FnMut() -> S,
{
    handshake(p, pins, clk)?;
    if pins.conf_done_high() {
        return Err(GuardedError::ConfDoneHighAfterReset);
    }
    let levels = sample_data_lines(pins, clk);
    if !levels.safe_to_enable() {
        return Err(GuardedError::DataLinesNotHigh(levels));
    }
    pins.data_lines_passed(levels);
    if !pins.enable_data_outputs() {
        return Err(GuardedError::OutputsRefused);
    }
    if p.post_nstatus_us != 0 {
        clk.delay_us(p.post_nstatus_us);
    }
    let mut src = make_source();
    let r = stream(p, pins, clk, &mut src);
    if src.failed() {
        return Err(GuardedError::Load(PsError::Decode { produced: src.produced() }));
    }
    let s = r.map_err(GuardedError::Load)?;
    let want = raw_len as u32;
    if s.early_done {
        return Err(GuardedError::EarlyDone { sent: s.bytes, want });
    }
    if s.bytes != want {
        return Err(GuardedError::ShortTransfer { sent: s.bytes, want });
    }
    Ok(s)
}

/// One load, never retried. On return DCLK and DATA0 are inputs (DCLK stays
/// an output only for `keep_dclk` after success), and on failure nCONFIG is
/// in the state [`GuardedError::final_nconfig`] names.
pub fn guarded_load<P, C, S, F>(
    p: &PsProfile,
    pins: &mut P,
    clk: &mut C,
    raw_len: usize,
    keep_dclk: bool,
    mut make_source: F,
) -> Result<LoadStats, GuardedError>
where
    P: GuardedPsPins,
    C: PsClock,
    S: PsSource,
    F: FnMut() -> S,
{
    pins.mark_run_started();
    match attempt(p, pins, clk, raw_len, &mut make_source) {
        Ok(s) => {
            pins.release_data_outputs(keep_dclk);
            Ok(s)
        }
        Err(e) => {
            pins.release_data_outputs(false);
            match e.final_nconfig() {
                NconfigFinal::HeldLow => pins.assert_nconfig(),
                NconfigFinal::Released => pins.disarm_nconfig(),
            }
            Err(e)
        }
    }
}

/// For panic / fault / deadline paths: stop driving DCLK and DATA0 first, then
/// hold the FPGA in reset if nCONFIG is armed. An unarmed nCONFIG stays an
/// input. Idempotent.
#[inline(always)]
pub fn fault_release<P: GuardedPsPins>(pins: &mut P, nconfig_armed: bool) {
    pins.release_data_outputs(false);
    if nconfig_armed {
        pins.assert_nconfig();
    }
}
