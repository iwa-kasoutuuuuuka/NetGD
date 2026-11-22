//! Pin census for a GDEMU 6.0 board (ATSAM3U2E) whose flash has been erased.
//!
//! `docs/research/gdemu60-bringup.md` §2-4. After ERASE nothing on the board
//! is known to be safe to drive: the MCU pin map of the 6.0 board is assumed
//! from v5.5 and only the SWD pins are confirmed (LOG R116). The census is
//! the first firmware to run. It **never drives a line**: every PIO line
//! stays an input (the state the SAM3U leaves it in after reset) and the only
//! thing it changes is the internal pull-up (on, as after reset; then off;
//! then on again). From the input levels it puts each line in a [`Class`]:
//!
//! | class       | pull-up on | pull-up off | what it usually means |
//! |-------------|------------|-------------|------------------------|
//! | `Low`       | always 0   | always 0    | something holds the line low: GND, a pull-down, an output driving low (an unconfigured FPGA's CONF_DONE) |
//! | `High`      | always 1   | always 1    | an external pull-up, an output driving high, 3.3 V -- **or a floating line still holding the charge the pull-up left** (see below) |
//! | `Floating`  | always 1   | 0 or both   | nothing that holds it: an open pin, a high-impedance input on the other side |
//! | `Toggling`  | many edges in either phase | | a clock or a data line that is busy |
//! | `Unstable`  | both levels, few edges | | changed while being sampled (a button pressed, a slow signal) |
//! | `Odd`       | always 0   | always 1    | changed between the phases; sample again |
//!
//! Limits (by design, since nothing is driven):
//! * The SAM3U has a pull-up but no pull-down, so `High` cannot be told from
//!   a floating line that keeps its charge after the pull-up is switched off.
//!   `Low` and `Floating` are the reliable answers.
//! * A line can only be named by what is on it, not by where it goes. Which
//!   FPGA ball or connector pin a line reaches is the pin-finder's job
//!   (`fpga/pinfind`) or a continuity test's.
//!
//! The [`Watch`] half reports lines that change later, while the owner
//! presses the button or inserts and removes the SD card: that finds the
//! button and the card-detect line without driving anything either.
//!
//! [`V55_ROLES`] lists what each line is on v5.5 (`docs/PCB.md`,
//! `embassy-atsam3`) and, for the few lines where a passive reading means
//! something, which classes agree with the v5.5 assumption on a 6.0 board
//! whose FPGA is not configured. A mismatch there is a reason to stop before
//! any firmware drives that line (`docs/research/gdemu60-bringup.md` §3).

#![no_std]

/// PIOA, PIOB, PIOC.
pub const PORTS: usize = 3;
/// 32 lines per port. All 96 are bonded out on the 144-pin SAM3U.
pub const NPINS: usize = PORTS * 32;
/// A line with at least this many edges over both phases is `Toggling`.
/// Fewer edges with both levels seen is `Unstable`.
pub const TOGGLE_MIN: u32 = 8;

/// Pin index 0..96: PIOA 0..31, PIOB 32..63, PIOC 64..95.
pub const fn pin_index(port: usize, bit: usize) -> usize {
    port * 32 + bit
}

/// `'A'`, `'B'` or `'C'`.
pub const fn port_char(pin: usize) -> char {
    (b'A' + (pin / 32) as u8) as char
}

/// The line number within its port.
pub const fn pin_bit(pin: usize) -> u8 {
    (pin % 32) as u8
}

fn bit_set(m: &[u32; PORTS], pin: usize) -> bool {
    m[pin / 32] & (1 << (pin % 32)) != 0
}

/// What a passive reading says about one line.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Class {
    High = 0,
    Low = 1,
    Floating = 2,
    Toggling = 3,
    Unstable = 4,
    Odd = 5,
}

impl Class {
    pub const ALL: [Class; 6] = [
        Class::High,
        Class::Low,
        Class::Floating,
        Class::Toggling,
        Class::Unstable,
        Class::Odd,
    ];

    /// This class as a bit of an [`Role::expect`] set.
    pub const fn bit(self) -> u8 {
        1 << (self as u8)
    }

    /// Short name for the log (what `docs/research/gdemu60-bringup.md` uses).
    pub const fn label(self) -> &'static str {
        match self {
            Class::High => "high",
            Class::Low => "low",
            Class::Floating => "float",
            Class::Toggling => "toggle",
            Class::Unstable => "unstable",
            Class::Odd => "odd",
        }
    }
}

/// The levels seen on one line during one phase.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Seen {
    /// No sample yet.
    Nothing,
    OnlyHigh,
    OnlyLow,
    Both,
}

/// Samples of all 96 lines taken with one pull-up setting.
#[derive(Clone)]
pub struct Phase {
    seen_hi: [u32; PORTS],
    seen_lo: [u32; PORTS],
    prev: [u32; PORTS],
    edges: [u32; NPINS],
    samples: u32,
}

impl Default for Phase {
    fn default() -> Self {
        Self::new()
    }
}

impl Phase {
    pub const fn new() -> Self {
        Self {
            seen_hi: [0; PORTS],
            seen_lo: [0; PORTS],
            prev: [0; PORTS],
            edges: [0; NPINS],
            samples: 0,
        }
    }

    /// One reading of the three `PIO_PDSR` registers.
    pub fn sample(&mut self, pdsr: [u32; PORTS]) {
        if self.samples != 0 {
            for port in 0..PORTS {
                let mut ch = pdsr[port] ^ self.prev[port];
                while ch != 0 {
                    let b = ch.trailing_zeros() as usize;
                    let e = &mut self.edges[pin_index(port, b)];
                    *e = e.saturating_add(1);
                    ch &= ch - 1;
                }
            }
        }
        for port in 0..PORTS {
            self.seen_hi[port] |= pdsr[port];
            self.seen_lo[port] |= !pdsr[port];
        }
        self.prev = pdsr;
        self.samples = self.samples.saturating_add(1);
    }

    pub fn samples(&self) -> u32 {
        self.samples
    }

    pub fn edges(&self, pin: usize) -> u32 {
        self.edges[pin]
    }

    pub fn seen(&self, pin: usize) -> Seen {
        if self.samples == 0 {
            return Seen::Nothing;
        }
        match (bit_set(&self.seen_hi, pin), bit_set(&self.seen_lo, pin)) {
            (true, false) => Seen::OnlyHigh,
            (false, true) => Seen::OnlyLow,
            _ => Seen::Both,
        }
    }

    /// The last reading.
    pub fn last(&self) -> [u32; PORTS] {
        self.prev
    }
}

/// Classify one line from the pull-up-on and pull-up-off phases.
pub fn classify(pu_on: &Phase, pu_off: &Phase, pin: usize) -> Class {
    let edges = pu_on.edges(pin).saturating_add(pu_off.edges(pin));
    if edges >= TOGGLE_MIN {
        return Class::Toggling;
    }
    match (pu_on.seen(pin), pu_off.seen(pin)) {
        (Seen::OnlyHigh, Seen::OnlyHigh) => Class::High,
        (Seen::OnlyLow, Seen::OnlyLow) => Class::Low,
        (Seen::OnlyHigh, Seen::OnlyLow | Seen::Both) => Class::Floating,
        (Seen::OnlyLow, Seen::OnlyHigh) => Class::Odd,
        _ => Class::Unstable,
    }
}

/// Every line's class as one mask per class (index = `Class as usize`),
/// three words each.
pub fn class_masks(pu_on: &Phase, pu_off: &Phase) -> [[u32; PORTS]; 6] {
    let mut m = [[0u32; PORTS]; 6];
    for pin in 0..NPINS {
        let c = classify(pu_on, pu_off, pin);
        m[c as usize][pin / 32] |= 1 << (pin % 32);
    }
    m
}

/// What a line is on v5.5, and which classes agree with that on a 6.0 board
/// with an unconfigured FPGA and an MCU that drives nothing.
#[derive(Clone, Copy, Debug)]
pub struct Role {
    pub pin: u8,
    pub name: &'static str,
    /// Allowed classes (`Class::bit`), or 0 when a passive reading says
    /// nothing either way.
    pub expect: u8,
}

const HI: u8 = Class::High.bit();
const LO: u8 = Class::Low.bit();
const FL: u8 = Class::Floating.bit();

const fn a(bit: u8) -> u8 {
    bit
}
const fn b(bit: u8) -> u8 {
    32 + bit
}
const fn c(bit: u8) -> u8 {
    64 + bit
}

/// The v5.5 lines (`docs/PCB.md`, `embassy-atsam3/src/{lib,smc,hsmci,uart}.rs`,
/// `cyclone::pinmap`), in pin order.
///
/// Expectations, all assumptions to be checked, never facts:
/// * CONF_DONE (PB1): **low** -- the FPGA holds it low until it is
///   configured (the Cyclone convention; AGM's documents do not say,
///   `docs/research/ag16k-config.md` §1-4).
/// * nSTATUS (PB2): **high** -- released after the FPGA's power-on reset,
///   pulled up on the board (10 kΩ per the AGM guide).
/// * nCONFIG (PA25): high (external pull-up) or floating (none); **never
///   low** -- low would mean something else holds it, and then the loader's
///   "nCONFIG low = FPGA off" is not true on this board.
/// * DATA0 / DCLK (PA23 / PA24): inputs of the FPGA during configuration:
///   high or floating, never low.
/// * Button (PA20): a switch to GND, not pressed: high or floating.
pub const V55_ROLES: &[Role] = &[
    Role { pin: a(1), name: "SD card detect", expect: 0 },
    Role { pin: a(3), name: "HSMCI MCCK (SD CLK)", expect: 0 },
    Role { pin: a(4), name: "HSMCI MCCDA (SD CMD)", expect: 0 },
    Role { pin: a(5), name: "HSMCI MCDA0", expect: 0 },
    Role { pin: a(6), name: "HSMCI MCDA1", expect: 0 },
    Role { pin: a(7), name: "HSMCI MCDA2", expect: 0 },
    Role { pin: a(8), name: "HSMCI MCDA3", expect: 0 },
    Role { pin: a(11), name: "UART0 URXD", expect: 0 },
    Role { pin: a(12), name: "UART0 UTXD", expect: 0 },
    Role { pin: a(20), name: "button", expect: HI | FL },
    Role { pin: a(21), name: "PCK0 -> FPGA 48 MHz", expect: 0 },
    Role { pin: a(22), name: "FPGA MCU_IRQn", expect: 0 },
    Role { pin: a(23), name: "PS DATA0", expect: HI | FL },
    Role { pin: a(24), name: "PS DCLK", expect: HI | FL },
    Role { pin: a(25), name: "PS nCONFIG", expect: HI | FL },
    Role { pin: a(28), name: "48 MHz (docs/PCB.md)", expect: 0 },
    Role { pin: a(30), name: "FPGA MCU_RSTn", expect: 0 },
    Role { pin: b(1), name: "PS CONF_DONE", expect: LO },
    Role { pin: b(2), name: "PS nSTATUS", expect: HI },
    Role { pin: b(6), name: "SMC D15", expect: 0 },
    Role { pin: b(7), name: "SMC A0/NBS0", expect: 0 },
    Role { pin: b(8), name: "SMC A1", expect: 0 },
    Role { pin: b(9), name: "SMC D0", expect: 0 },
    Role { pin: b(10), name: "SMC D1", expect: 0 },
    Role { pin: b(11), name: "SMC D2", expect: 0 },
    Role { pin: b(12), name: "SMC D3", expect: 0 },
    Role { pin: b(13), name: "SMC D4", expect: 0 },
    Role { pin: b(14), name: "SMC D5", expect: 0 },
    Role { pin: b(15), name: "SMC D6", expect: 0 },
    Role { pin: b(16), name: "SMC D7", expect: 0 },
    Role { pin: b(18), name: "LED", expect: 0 },
    Role { pin: b(19), name: "SMC NRD", expect: 0 },
    Role { pin: b(23), name: "SMC NWE", expect: 0 },
    Role { pin: b(25), name: "SMC D8", expect: 0 },
    Role { pin: b(26), name: "SMC D9", expect: 0 },
    Role { pin: b(27), name: "SMC D10", expect: 0 },
    Role { pin: b(28), name: "SMC D11", expect: 0 },
    Role { pin: b(29), name: "SMC D12", expect: 0 },
    Role { pin: b(30), name: "SMC D13", expect: 0 },
    Role { pin: b(31), name: "SMC D14", expect: 0 },
    Role { pin: c(0), name: "SMC A2", expect: 0 },
    Role { pin: c(1), name: "SMC A3", expect: 0 },
    Role { pin: c(2), name: "SMC A4", expect: 0 },
    Role { pin: c(3), name: "SMC A5", expect: 0 },
    Role { pin: c(4), name: "SMC A6", expect: 0 },
    Role { pin: c(5), name: "SMC A7", expect: 0 },
    Role { pin: c(6), name: "SMC A8", expect: 0 },
    Role { pin: c(15), name: "SMC NBS1", expect: 0 },
    Role { pin: c(17), name: "SMC NCS3", expect: 0 },
];

/// The v5.5 role of a line, if it has one.
pub fn role(pin: usize) -> Option<&'static Role> {
    V55_ROLES.iter().find(|r| r.pin as usize == pin)
}

/// `Some(true)` when the class agrees with the v5.5 assumption,
/// `Some(false)` when it does not, `None` when nothing is expected.
pub fn agrees(role: &Role, class: Class) -> Option<bool> {
    if role.expect == 0 {
        None
    } else {
        Some(role.expect & class.bit() != 0)
    }
}

/// Lines that change after the census, reported a few times each so a busy
/// line cannot flood the log.
pub struct Watch {
    prev: [u32; PORTS],
    ignore: [u32; PORTS],
    reports: [u8; NPINS],
    limit: u8,
    started: bool,
}

impl Watch {
    /// `ignore`: lines not to report (the `Toggling` ones); `limit`: reports
    /// per line.
    pub const fn new(ignore: [u32; PORTS], limit: u8) -> Self {
        Self {
            prev: [0; PORTS],
            ignore,
            reports: [0; NPINS],
            limit,
            started: false,
        }
    }

    /// One reading. Returns the lines to report now (changed, not ignored,
    /// under the limit); their new level is in `pdsr`.
    pub fn update(&mut self, pdsr: [u32; PORTS]) -> [u32; PORTS] {
        let mut out = [0u32; PORTS];
        if self.started {
            for port in 0..PORTS {
                let mut ch = (pdsr[port] ^ self.prev[port]) & !self.ignore[port];
                while ch != 0 {
                    let b = ch.trailing_zeros() as usize;
                    let r = &mut self.reports[pin_index(port, b)];
                    if *r < self.limit {
                        *r += 1;
                        out[port] |= 1 << b;
                    }
                    ch &= ch - 1;
                }
            }
        }
        self.prev = pdsr;
        self.started = true;
        out
    }

    /// How many times this line has been reported.
    pub fn reported(&self, pin: usize) -> u8 {
        self.reports[pin]
    }
}

/// The pin indices set in a three-word mask, in order.
pub fn pins(mask: [u32; PORTS]) -> impl Iterator<Item = usize> {
    (0..NPINS).filter(move |&p| bit_set(&mask, p))
}
