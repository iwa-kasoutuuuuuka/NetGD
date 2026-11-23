//! `psload::guarded` against a small model of the five lines, recording the
//! order of everything the loader does to them.

use psload::guarded::*;
use psload::*;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum E {
    Mark,
    Assert,
    Release,
    Disarm,
    Enable,
    Refused,
    ReleaseData { keep_dclk: bool },
    Clock,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Fpga {
    Good,
    LowFromStart,
    StuckHigh,
    StuckLow,
    /// nSTATUS drops once this many clocks went in.
    DropAfterBits(u32),
}

struct M {
    fpga: Fpga,
    /// DCLKs until CONF_DONE; `None` = never.
    done_after: Option<u32>,
    done_at_start: bool,
    refuse: bool,
    t_us: u64,
    ncfg_low: bool,
    seen_reset: bool,
    outputs: bool,
    dclk: bool,
    clocks: u32,
    ev: Vec<E>,
}

impl M {
    fn new(fpga: Fpga, done_after: Option<u32>) -> Self {
        M {
            fpga,
            done_after,
            done_at_start: false,
            refuse: false,
            t_us: 0,
            ncfg_low: false,
            seen_reset: false,
            outputs: false,
            dclk: false,
            clocks: 0,
            ev: vec![],
        }
    }
    fn count(&self, e: E) -> usize {
        self.ev.iter().filter(|x| **x == e).count()
    }
    fn pos(&self, e: E) -> usize {
        self.ev.iter().position(|x| *x == e).unwrap_or_else(|| panic!("{e:?} missing: {:?}", self.ev))
    }
    /// The last two events that are not clocks: how the run ended.
    fn ending(&self) -> Vec<E> {
        let v: Vec<E> = self.ev.iter().copied().filter(|e| *e != E::Clock).collect();
        v[v.len().saturating_sub(2)..].to_vec()
    }
}

#[derive(Clone)]
struct H(Rc<RefCell<M>>);

impl PsPins for H {
    fn set_nconfig(&mut self, _: bool) {
        panic!("the guarded path must use assert/release");
    }
    fn set_dclk(&mut self, high: bool) {
        let mut m = self.0.borrow_mut();
        assert!(m.outputs, "DCLK moved while not an output");
        if high && !m.dclk {
            m.clocks += 1;
            m.ev.push(E::Clock);
        }
        m.dclk = high;
    }
    fn set_data(&mut self, _: bool) {
        assert!(self.0.borrow().outputs, "DATA moved while not an output");
    }
    fn nstatus_high(&mut self) -> bool {
        let mut m = self.0.borrow_mut();
        m.t_us += 1;
        match m.fpga {
            Fpga::LowFromStart => false,
            Fpga::StuckHigh => true,
            Fpga::StuckLow => !m.seen_reset,
            Fpga::Good => !m.ncfg_low,
            Fpga::DropAfterBits(n) => !m.ncfg_low && m.clocks < n,
        }
    }
    fn conf_done_high(&mut self) -> bool {
        let m = self.0.borrow();
        m.done_at_start || m.done_after.map_or(false, |n| m.clocks >= n)
    }
}

impl GuardedPsPins for H {
    fn mark_run_started(&mut self) {
        self.0.borrow_mut().ev.push(E::Mark);
    }
    fn assert_nconfig(&mut self) {
        let mut m = self.0.borrow_mut();
        m.ncfg_low = true;
        m.seen_reset = true;
        m.ev.push(E::Assert);
    }
    fn release_nconfig(&mut self) {
        let mut m = self.0.borrow_mut();
        m.ncfg_low = false;
        m.ev.push(E::Release);
    }
    fn disarm_nconfig(&mut self) {
        let mut m = self.0.borrow_mut();
        m.ncfg_low = false;
        m.ev.push(E::Disarm);
    }
    fn enable_data_outputs(&mut self) -> bool {
        let mut m = self.0.borrow_mut();
        if m.refuse {
            m.ev.push(E::Refused);
            return false;
        }
        m.outputs = true;
        m.ev.push(E::Enable);
        true
    }
    fn release_data_outputs(&mut self, keep_dclk: bool) {
        let mut m = self.0.borrow_mut();
        m.outputs = false;
        m.ev.push(E::ReleaseData { keep_dclk });
    }
}

impl PsClock for H {
    fn nops(&mut self, _: u32) {}
    fn delay_us(&mut self, us: u32) {
        self.0.borrow_mut().t_us += us as u64;
    }
    fn now_us(&mut self) -> u64 {
        panic!("the guarded path must not depend on the system tick")
    }
}

struct Src {
    data: Vec<u8>,
    at: usize,
    fail: bool,
}
impl Iterator for Src {
    type Item = u8;
    fn next(&mut self) -> Option<u8> {
        let b = self.data.get(self.at).copied();
        self.at += 1;
        b
    }
}
impl PsSource for Src {
    fn failed(&self) -> bool {
        self.fail
    }
    fn produced(&self) -> u32 {
        self.data.len() as u32
    }
}
fn src(n: usize) -> Src {
    Src { data: vec![0xA5; n], at: 0, fail: false }
}

const LEN: usize = 16;
const ALL_BITS: u32 = LEN as u32 * 8;

fn run_with(
    m: M,
    keep_dclk: bool,
    mk: impl FnMut() -> Src,
) -> (Result<LoadStats, GuardedError>, M) {
    let h = H(Rc::new(RefCell::new(m)));
    let (mut pins, mut clk) = (h.clone(), h.clone());
    let r = guarded_load(&AG16K, &mut pins, &mut clk, LEN, keep_dclk, mk);
    drop((pins, clk));
    let m = Rc::try_unwrap(h.0).ok().expect("sole owner").into_inner();
    (r, m)
}

fn load(m: M) -> (Result<LoadStats, GuardedError>, M) {
    run_with(m, false, || src(LEN))
}

/// The worst the handshake may take on this profile.
fn handshake_bound() -> u64 {
    let p = AG16K;
    (p.nstatus_low_wait_us + p.ncfg_low_us + p.nstatus_high_timeout_us) as u64 * 2 + 1_000
}

fn assert_no_data_activity(m: &M) {
    assert_eq!(m.count(E::Enable), 0);
    assert_eq!(m.clocks, 0);
    assert!(!m.outputs);
}

const REL: E = E::ReleaseData { keep_dclk: false };

#[test]
fn good_load_orders_every_step() {
    let (r, m) = load(M::new(Fpga::Good, Some(ALL_BITS)));
    assert_eq!(r.unwrap().bytes, LEN as u32);
    let (mark, a, rel, en, clk) = (m.pos(E::Mark), m.pos(E::Assert), m.pos(E::Release), m.pos(E::Enable), m.pos(E::Clock));
    assert!(mark < a && a < rel && rel < en && en < clk, "{:?}", m.ev);
    assert_eq!(m.count(E::Assert), 1, "one reset only");
    assert_eq!(*m.ev.last().unwrap(), REL);
    assert!(!m.outputs && !m.ncfg_low);
}

#[test]
fn keep_dclk_applies_to_success_only() {
    let (r, m) = run_with(M::new(Fpga::Good, Some(ALL_BITS)), true, || src(LEN));
    r.unwrap();
    assert_eq!(*m.ev.last().unwrap(), E::ReleaseData { keep_dclk: true });

    let (r, m) = run_with(M::new(Fpga::Good, None), true, || src(LEN));
    r.unwrap_err();
    assert_eq!(m.count(E::ReleaseData { keep_dclk: true }), 0);
}

#[test]
fn nstatus_low_before_reset_drives_nothing() {
    let (r, m) = load(M::new(Fpga::LowFromStart, Some(0)));
    assert_eq!(r, Err(GuardedError::NstatusLowBeforeReset));
    assert_eq!(m.count(E::Assert), 0, "nCONFIG must not move");
    assert_no_data_activity(&m);
    assert_eq!(m.ending(), [REL, E::Disarm]);
}

#[test]
fn nstatus_stuck_high_releases_nconfig_and_never_clocks() {
    let (r, m) = load(M::new(Fpga::StuckHigh, Some(ALL_BITS)));
    assert_eq!(r, Err(GuardedError::NstatusStuckHigh));
    assert_eq!(m.count(E::Assert), 1);
    assert_no_data_activity(&m);
    assert!(!m.ncfg_low, "an unproven line is not left sunk");
    assert_eq!(m.ending(), [REL, E::Disarm]);
    assert!(m.t_us <= handshake_bound(), "{} us", m.t_us);
    assert!(m.t_us >= AG16K.nstatus_low_wait_us as u64);
}

#[test]
fn nstatus_stuck_low_holds_reset_and_never_clocks() {
    let (r, m) = load(M::new(Fpga::StuckLow, Some(ALL_BITS)));
    assert_eq!(r, Err(GuardedError::NstatusStuckLow));
    assert_no_data_activity(&m);
    assert!(m.ncfg_low);
    assert_eq!(m.ending(), [REL, E::Assert]);
    assert!(m.t_us <= handshake_bound(), "{} us", m.t_us);
    assert!(m.t_us >= AG16K.nstatus_high_timeout_us as u64);
}

#[test]
fn conf_done_high_after_reset_never_clocks() {
    let mut m = M::new(Fpga::Good, Some(ALL_BITS));
    m.done_at_start = true;
    let (r, m) = load(m);
    assert_eq!(r, Err(GuardedError::ConfDoneHighAfterReset));
    assert_no_data_activity(&m);
    assert!(m.ncfg_low);
}

#[test]
fn refused_outputs_hold_reset() {
    let mut m = M::new(Fpga::Good, Some(ALL_BITS));
    m.refuse = true;
    let (r, m) = load(m);
    assert_eq!(r, Err(GuardedError::OutputsRefused));
    assert_no_data_activity(&m);
    assert!(m.ncfg_low);
}

#[test]
fn conf_done_timeout_releases_data_then_holds_reset() {
    let (r, m) = load(M::new(Fpga::Good, None));
    assert_eq!(r, Err(GuardedError::Load(PsError::ConfDoneTimeout { padding: AG16K.pad_cap_clocks })));
    assert_eq!(m.clocks, ALL_BITS + AG16K.pad_cap_clocks, "bounded by the pad cap");
    assert!(!m.outputs && m.ncfg_low);
    assert_eq!(m.ending(), [REL, E::Assert]);
    assert_eq!(m.count(E::Enable), 1, "no retry");
}

#[test]
fn nstatus_drop_mid_stream_releases_then_holds_reset() {
    let (r, m) = load(M::new(Fpga::DropAfterBits(40), None));
    assert!(matches!(r, Err(GuardedError::Load(PsError::NstatusDropped { .. }))));
    assert!(!m.outputs && m.ncfg_low);
    assert_eq!(m.ending(), [REL, E::Assert]);
}

#[test]
fn broken_source_is_a_decode_error() {
    let (r, m) = run_with(M::new(Fpga::Good, Some(ALL_BITS)), false, || Src { fail: true, ..src(LEN) });
    assert_eq!(r, Err(GuardedError::Load(PsError::Decode { produced: LEN as u32 })));
    assert!(!m.outputs && m.ncfg_low);
}

#[test]
fn early_done_and_short_transfer_fail() {
    let (r, m) = load(M::new(Fpga::Good, Some(16)));
    assert_eq!(r, Err(GuardedError::EarlyDone { sent: 2, want: LEN as u32 }));
    assert!(!m.outputs && m.ncfg_low);

    let (r, m) = run_with(M::new(Fpga::Good, Some(8 * 8)), false, || src(8));
    assert_eq!(r, Err(GuardedError::ShortTransfer { sent: 8, want: LEN as u32 }));
    assert!(!m.outputs && m.ncfg_low);
}

#[test]
fn a_failure_builds_one_source_and_one_reset() {
    let mut made = 0;
    let (_, m) = run_with(M::new(Fpga::Good, None), false, || {
        made += 1;
        src(LEN)
    });
    assert_eq!(made, 1);
    // The handshake's assert, and the final hold.
    assert_eq!(m.count(E::Assert), 2);
    assert_eq!(m.count(E::Release), 1);
}

#[test]
fn fault_release_order() {
    let h = H(Rc::new(RefCell::new(M::new(Fpga::Good, None))));
    fault_release(&mut h.clone(), true);
    assert_eq!(h.0.borrow().ev, [REL, E::Assert]);

    let h = H(Rc::new(RefCell::new(M::new(Fpga::Good, None))));
    fault_release(&mut h.clone(), false);
    assert_eq!(h.0.borrow().ev, [REL], "unarmed nCONFIG is not touched");
}

#[test]
fn final_nconfig_table() {
    use GuardedError::*;
    for e in [NstatusLowBeforeReset, NstatusStuckHigh] {
        assert_eq!(e.final_nconfig(), NconfigFinal::Released);
    }
    for e in [
        NstatusStuckLow,
        ConfDoneHighAfterReset,
        OutputsRefused,
        Load(PsError::ConfDoneTimeout { padding: 0 }),
        EarlyDone { sent: 0, want: 1 },
        ShortTransfer { sent: 0, want: 1 },
    ] {
        assert_eq!(e.final_nconfig(), NconfigFinal::HeldLow);
    }
}

fn cksum(d: &[u8]) -> u32 {
    let mut c = Cksum::new();
    d.iter().for_each(|b| c.update(*b));
    c.finish()
}

#[test]
fn cksum_matches_posix() {
    // `printf 123456789 | cksum`
    assert_eq!(cksum(b"123456789"), 930_766_865);
}

#[test]
fn precheck_paths() {
    let good = cksum(&[0xA5; LEN]);
    let mut made = 0;
    let mut mk = || {
        made += 1;
        src(LEN)
    };
    assert_eq!(precheck(true, true, LEN, good, &mut mk), Err(PrecheckError::AlreadyRan));
    assert_eq!(precheck(false, false, LEN, good, &mut mk), Err(PrecheckError::NoBitstream));
    assert_eq!(precheck(false, true, LEN, good, &mut mk), Ok(()));
    assert_eq!(
        precheck(false, true, LEN + 1, good, &mut mk),
        Err(PrecheckError::Length { got: LEN as u32, want: LEN as u32 + 1 })
    );
    assert_eq!(
        precheck(false, true, LEN, good ^ 1, &mut mk),
        Err(PrecheckError::Cksum { got: good, want: good ^ 1 })
    );
    assert_eq!(made, 3, "the refusals before the decode build no source");
    let mut bad = || Src { fail: true, ..src(LEN) };
    assert_eq!(precheck(false, true, LEN, good, &mut bad), Err(PrecheckError::Decode { produced: LEN as u32 }));
}
