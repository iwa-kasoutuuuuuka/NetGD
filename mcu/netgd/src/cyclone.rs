//! FPGA passive-serial loader, MCU side.
//!
//! The sequence, the per-FPGA timing and the retry policy live in the
//! host-tested `psload` crate. This file is what only the MCU can supply: the
//! five GPIO lines, a cycle-accurate delay, and the board's choice of
//! profile. (The file keeps its old name; `docs/design/g1-ag16k-port.md`
//! 4-3 proposes `fpga_loader.rs` once the rest of the board split lands.)
//!
//! Board selection is a cargo feature (`board-v55` | `board-v60`), see
//! `Cargo.toml`.

use cortex_m::asm;
use embassy_atsam3::{
    gpio::{Input, Level, Output, Pull},
    peripherals, Peri,
};
use embassy_time::Instant;
use psload::{LoadFailure, LoadStats, PsClock, PsError, PsPins, PsProfile, PsSource};

#[cfg(all(feature = "board-v55", feature = "board-v60"))]
compile_error!(
    "`board-v55` and `board-v60` are mutually exclusive; \
     build the 6.0 board with `--no-default-features --features board-v60`"
);
#[cfg(not(any(feature = "board-v55", feature = "board-v60")))]
compile_error!("select a board: `--features board-v55` (default) or `board-v60`");

/// The FPGA this firmware image configures, and how.
#[cfg(feature = "board-v55")]
pub const PROFILE: PsProfile = psload::CYCLONE2;
#[cfg(all(feature = "board-v60", not(feature = "ps-msb-first")))]
pub const PROFILE: PsProfile = psload::AG16K;
#[cfg(all(feature = "board-v60", feature = "ps-msb-first"))]
pub const PROFILE: PsProfile = psload::AG16K.with_bit_order(psload::BitOrder::MsbFirst);

/// Where the five configuration lines are.
///
/// v5.5: read off the PCB and in use since the first boards.
///
/// v6.0: **UNCONFIRMED.** These are the v5.5 pins, assumed unchanged because
/// the schematic of the 6.0 board is not available. The owner's continuity
/// test (`docs/research/user-tasks.html`, task 2) must confirm each one --
/// DATA0, DCLK, nCONFIG, nSTATUS, CONF_DONE -- before this loader is
/// trusted. Until then a 6.0 build that cannot configure the FPGA is as
/// likely a wrong pin as a wrong bitstream.
///
/// The aliases are what `McuPins::new` is typed against, so a mismatch with
/// `UserPeripherals` (which fixes the pin types in `embassy-atsam3`) is a
/// compile error, not a silent miswire. Moving a line to another pin means
/// changing the alias here *and* the field's type there.
pub mod pinmap {
    use super::peripherals;
    pub type Data0 = peripherals::PA23;
    pub type Dclk = peripherals::PA24;
    pub type Nconfig = peripherals::PA25;
    pub type ConfDone = peripherals::PB1;
    pub type Nstatus = peripherals::PB2;

    // The names, for anyone reading the log or the source; nothing indexes
    // by them (the types above are what the compiler checks).
    #[allow(dead_code)]
    pub const DATA0: &str = "PA23";
    #[allow(dead_code)]
    pub const DCLK: &str = "PA24";
    #[allow(dead_code)]
    pub const NCONFIG: &str = "PA25";
    #[allow(dead_code)]
    pub const CONF_DONE: &str = "PB1";
    #[allow(dead_code)]
    pub const NSTATUS: &str = "PB2";
}

/// The loader's five lines. nCONFIG, DCLK and DATA0 start low (as the old
/// `PSLoader::new` left them); nSTATUS and CONF_DONE have the internal
/// pull-up on, which is harmless with an external one and the only safety
/// net without.
pub struct McuPins {
    nconfig: Output<'static, pinmap::Nconfig>,
    dclk: Output<'static, pinmap::Dclk>,
    data: Output<'static, pinmap::Data0>,
    nstatus: Input<'static, pinmap::Nstatus>,
    done: Input<'static, pinmap::ConfDone>,
}

impl McuPins {
    pub fn new(
        nconfig: Peri<'static, pinmap::Nconfig>,
        dclk: Peri<'static, pinmap::Dclk>,
        data: Peri<'static, pinmap::Data0>,
        nstatus: Peri<'static, pinmap::Nstatus>,
        done: Peri<'static, pinmap::ConfDone>,
    ) -> Self {
        Self {
            nconfig: Output::new(nconfig, Level::Low),
            dclk: Output::new(dclk, Level::Low),
            data: Output::new(data, Level::Low),
            nstatus: Input::new(nstatus, Pull::Up),
            done: Input::new(done, Pull::Up),
        }
    }
}

impl PsPins for McuPins {
    #[inline(always)]
    fn set_nconfig(&mut self, high: bool) {
        self.nconfig.set_level(high.into());
    }
    #[inline(always)]
    fn set_dclk(&mut self, high: bool) {
        self.dclk.set_level(high.into());
    }
    #[inline(always)]
    fn set_data(&mut self, high: bool) {
        self.data.set_level(high.into());
    }
    #[inline(always)]
    fn nstatus_high(&mut self) -> bool {
        self.nstatus.is_high()
    }
    #[inline(always)]
    fn conf_done_high(&mut self) -> bool {
        self.done.is_high()
    }
}

/// MCK in MHz: `bootcore::part::MCK_HZ`, 12 MHz * 8 (what `main` configures).
const CYCLES_PER_US: u32 = bootcore::part::MCK_HZ / 1_000_000;

pub struct McuClock;

impl PsClock for McuClock {
    #[inline(always)]
    fn nops(&mut self, n: u32) {
        for _ in 0..n {
            asm::nop();
        }
    }
    /// A cycle count, not a `Timer`: the system tick is 1 kHz, far too coarse
    /// for the microsecond waits, and nothing else runs while the FPGA is
    /// being loaded (the old `Timer::after(1 ms)` never yielded to anything
    /// either).
    #[inline(always)]
    fn delay_us(&mut self, us: u32) {
        asm::delay(us.saturating_mul(CYCLES_PER_US));
    }
    /// 1 ms resolution; only the 500 ms-class timeouts use it.
    #[inline(always)]
    fn now_us(&mut self) -> u64 {
        Instant::now().as_micros()
    }
}

/// The LZMA decoder as the loader's byte source.
pub struct LzSource<'a>(pub lzma::Decoder<'a>);

impl Iterator for LzSource<'_> {
    type Item = u8;
    #[inline(always)]
    fn next(&mut self) -> Option<u8> {
        self.0.next()
    }
}

impl PsSource for LzSource<'_> {
    fn failed(&self) -> bool {
        self.0.error().is_some()
    }
    fn produced(&self) -> u32 {
        self.0.produced() as u32
    }
}

/// Configure the FPGA, with the profile's retries. `make_source` is called
/// once per attempt and must start the bitstream over (see
/// `psload::load_with_retries`).
///
/// Everything is inlined into the caller so that `PROFILE` is a compile-time
/// constant inside the per-bit loop: the no-op counts must be immediates, or
/// the bit-bang slows down and so does the FPGA's time-to-ready.
#[inline(always)]
pub fn load<S: PsSource>(
    pins: &mut McuPins,
    make_source: impl FnMut() -> S,
) -> Result<LoadStats, LoadFailure> {
    let mut clk = McuClock;
    psload::load_with_retries(&PROFILE, pins, &mut clk, make_source, |n, e| {
        defmt::warn!("FPGA load attempt {} failed: {}", n, e);
    })
}

/// What the LED says when the FPGA could not be configured: `code` short
/// blinks (`PsError::code`), then a pause, three times over.
pub async fn blink_code(led: &mut Output<'static, peripherals::PB18>, error: &PsError) {
    use embassy_time::{Duration, Timer};
    for _ in 0..3 {
        for _ in 0..error.code() {
            led.set_low();
            Timer::after(Duration::from_millis(200)).await;
            led.set_high();
            Timer::after(Duration::from_millis(200)).await;
        }
        Timer::after(Duration::from_millis(2000)).await;
    }
}
