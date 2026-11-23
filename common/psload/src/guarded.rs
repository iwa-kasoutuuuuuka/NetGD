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
//! 3. Only after the handshake do DCLK and DATA0 become outputs.
//! 4. Every exit releases DCLK and DATA0 *first*, then settles nCONFIG
//!    ([`GuardedError::final_nconfig`]).

use crate::{stream, LoadStats, PsClock, PsError, PsPins, PsProfile, PsSource};

/// Polling step of the handshake. The bounds are counted in delays, not read
/// from the system tick, so they hold even if the tick has stopped.
const POLL_STEP_US: u32 = 10;

/// The line control a guarded load needs on top of [`PsPins`].
pub trait GuardedPsPins: PsPins {
    /// Record that a run started. Called before the first drive of any line.
    fn mark_run_started(&mut self);
    /// Sink nCONFIG low. Never drives it high.
    fn assert_nconfig(&mut self);
    /// Stop sinking nCONFIG: input, raised by the pull-up.
    fn release_nconfig(&mut self);
    /// As `release_nconfig`, and no later fault path may sink it again.
    fn disarm_nconfig(&mut self);
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
    /// nSTATUS followed nCONFIG down but did not rise after the release.
    NstatusStuckLow,
    /// CONF_DONE read high after the reset: clocking would be meaningless.
    ConfDoneHighAfterReset,
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
            GuardedError::NstatusLowBeforeReset | GuardedError::NstatusStuckHigh => NconfigFinal::Released,
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

fn wait_nstatus<P: PsPins, C: PsClock>(pins: &mut P, clk: &mut C, want_high: bool, timeout_us: u32) -> bool {
    for _ in 0..=timeout_us / POLL_STEP_US {
        if pins.nstatus_high() == want_high {
            return true;
        }
        clk.delay_us(POLL_STEP_US);
    }
    pins.nstatus_high() == want_high
}

/// nCONFIG low, nSTATUS must follow; nCONFIG released, nSTATUS must rise.
/// Both waits are bounded by the profile. Touches nCONFIG only.
pub fn handshake<P: GuardedPsPins, C: PsClock>(p: &PsProfile, pins: &mut P, clk: &mut C) -> Result<(), GuardedError> {
    if !pins.nstatus_high() {
        return Err(GuardedError::NstatusLowBeforeReset);
    }
    pins.assert_nconfig();
    if !wait_nstatus(pins, clk, false, p.nstatus_low_wait_us) {
        return Err(GuardedError::NstatusStuckHigh);
    }
    clk.delay_us(p.ncfg_low_us);
    pins.release_nconfig();
    if !wait_nstatus(pins, clk, true, p.nstatus_high_timeout_us) {
        return Err(GuardedError::NstatusStuckLow);
    }
    Ok(())
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
