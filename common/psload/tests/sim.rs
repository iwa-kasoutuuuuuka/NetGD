mod common;

use common::*;
use psload::*;

fn run(
    p: &PsProfile,
    w: &Shared,
    mut make: impl FnMut() -> VecSource,
) -> (Result<LoadStats, LoadFailure>, Vec<(u8, PsError)>, u32) {
    let mut pins = SimPins(w.clone());
    let mut clk = SimClock(w.clone());
    let mut fails = Vec::new();
    let mut made = 0u32;
    let r = load_with_retries(
        p,
        &mut pins,
        &mut clk,
        || {
            made += 1;
            make()
        },
        |n, e| fails.push((n, *e)),
    );
    (r, fails, made)
}

#[test]
fn ag16k_success_first_try() {
    let data = payload(2000, 1);
    let w = world(Cfg { done_after_clocks: 7, init_clocks: 2, ..Cfg::new(data.clone()) });
    let (r, fails, made) = run(&AG16K, &w, || VecSource::new(data.clone()));
    let s = r.expect("load");
    assert!(fails.is_empty());
    assert_eq!(made, 1);
    assert_eq!((s.bytes, s.attempts, s.padding, s.early_done), (2000, 1, 7, false));
    let w = w.borrow();
    assert!(w.done && w.user_mode);
    assert_eq!(w.rx, data);
    // 64 extra clocks after CONF_DONE, exactly, then DCLK/DATA0 parked low.
    assert_eq!(w.clocks_after_done, 64);
    assert!(!w.dclk && !w.data);
    // Settle times were honoured: nCONFIG low >= 1 ms before it rose.
    assert!(w.trace.iter().any(|e| *e == Ev::Delay(1_000)));
    assert!(w.trace.iter().any(|e| *e == Ev::Delay(50)));
}

#[test]
fn ag16k_stops_when_conf_done_rises_before_the_stream_ends() {
    let need = payload(300, 2);
    let mut longer = need.clone();
    longer.extend(payload(100, 3)); // trailing bytes the FPGA does not need
    let w = world(Cfg::new(need.clone()));
    let (r, _, _) = run(&AG16K, &w, || VecSource::new(longer.clone()));
    let s = r.unwrap();
    assert!(s.early_done);
    assert_eq!(s.bytes, 300);
    assert_eq!(w.borrow().clocks_after_done, 64);
}

#[test]
fn nstatus_never_rises_times_out_then_retries_then_errors() {
    for p in [AG16K, CYCLONE2] {
        let data = payload(64, 4);
        let w = world(Cfg { nstatus_never: true, ..Cfg::new(data.clone()) });
        let (r, fails, made) = run(&p, &w, || VecSource::new(data.clone()));
        let f = r.unwrap_err();
        assert_eq!(f.error, PsError::NstatusStuckLow, "{}", p.name);
        assert_eq!(f.attempts as u32, 1 + p.retries as u32);
        assert_eq!(made, 4);
        assert_eq!(fails.len(), 4);
        let w = w.borrow();
        // nCONFIG was pulsed once per attempt, and the whole thing is
        // bounded: 4 attempts of at most (cap + the low phase + slack).
        assert_eq!(w.attempts, 4);
        let per = p.nstatus_high_timeout_us as u64 + p.ncfg_low_us as u64 + p.nstatus_low_wait_us as u64;
        assert!(w.t_ns / 1000 <= 4 * (per + 100), "{} took {} us", p.name, w.t_ns / 1000);
        assert!(w.t_ns / 1000 >= 4 * p.nstatus_high_timeout_us as u64);
        assert_eq!(w.rx.len(), 0, "no data may be clocked while nSTATUS is low");
    }
}

#[test]
fn slow_power_on_reset_within_the_cap_still_works() {
    let data = payload(100, 5);
    let w = world(Cfg { nstatus_rise_us: 300_000, ..Cfg::new(data.clone()) });
    let (r, fails, _) = run(&AG16K, &w, || VecSource::new(data.clone()));
    assert!(r.is_ok());
    assert!(fails.is_empty());
}

#[test]
fn conf_done_never_rises_after_the_full_stream() {
    for p in [AG16K, CYCLONE2] {
        let data = payload(128, 6);
        let w = world(Cfg { done_never: true, ..Cfg::new(data.clone()) });
        let (r, fails, made) = run(&p, &w, || VecSource::new(data.clone()));
        let f = r.unwrap_err();
        assert_eq!(f.error, PsError::ConfDoneTimeout { padding: p.pad_cap_clocks }, "{}", p.name);
        assert_eq!(made as u8, 1 + p.retries);
        assert_eq!(fails.len() as u8, 1 + p.retries);
        // Whole stream went in every time, padding stopped at the cap.
        assert_eq!(w.borrow().padding_seen, p.pad_cap_clocks);
    }
}

#[test]
fn nstatus_drops_mid_stream_then_the_retry_succeeds() {
    let data = payload(500, 7);
    let w = world(Cfg { drop_at_byte: Some(123), drop_attempts: 1, ..Cfg::new(data.clone()) });
    let (r, fails, made) = run(&AG16K, &w, || VecSource::new(data.clone()));
    let s = r.unwrap();
    assert_eq!(s.attempts, 2);
    assert_eq!(made, 2);
    // The byte count is how many had gone in when the next one was refused.
    assert_eq!(fails, vec![(1, PsError::NstatusDropped { byte: 124 })]);
    assert_eq!(w.borrow().rx, data);
    assert_eq!(w.borrow().attempts, 2);
}

#[test]
fn nstatus_drops_every_time_gives_up_after_the_retries() {
    for p in [AG16K, CYCLONE2] {
        let data = payload(500, 8);
        let w = world(Cfg { drop_at_byte: Some(10), drop_attempts: 99, ..Cfg::new(data.clone()) });
        let (r, _, made) = run(&p, &w, || VecSource::new(data.clone()));
        let f = r.unwrap_err();
        assert!(matches!(f.error, PsError::NstatusDropped { .. }));
        assert_eq!(made as u8, 1 + p.retries);
    }
}

#[test]
fn bit_order_mismatch_is_detected_by_the_simulator_and_fixed_by_the_profile() {
    let data = payload(256, 9);
    // The FPGA wants MSB first; the AG16K profile sends LSB first.
    let cfg = Cfg { order: BitOrder::MsbFirst, ..Cfg::new(data.clone()) };
    let w = world(cfg.clone());
    let (r, fails, _) = run(&AG16K, &w, || VecSource::new(data.clone()));
    assert!(r.is_err());
    assert!(w.borrow().bit_order_mismatch_seen);
    // The FPGA's CRC check fails at the end of the stream; the AG16K profile
    // sees nSTATUS drop in its padding loop and fails fast, not at the cap.
    assert!(fails.iter().all(|(_, e)| *e == PsError::NstatusDropped { byte: 256 }), "{fails:?}");
    assert!(w.borrow().padding_seen < 4);

    let w = world(cfg);
    let msb = AG16K.with_bit_order(BitOrder::MsbFirst);
    let (r, fails, _) = run(&msb, &w, || VecSource::new(data.clone()));
    assert!(r.is_ok());
    assert!(fails.is_empty());
    assert!(!w.borrow().bit_order_mismatch_seen);
    assert_eq!(w.borrow().rx, data);
}

#[test]
fn cyclone2_padding_loop_does_not_look_at_nstatus_so_a_crc_error_waits_for_the_cap() {
    let data = payload(64, 10);
    let w = world(Cfg { order: BitOrder::MsbFirst, ..Cfg::new(data.clone()) });
    let (r, _, _) = run(&CYCLONE2, &w, || VecSource::new(data.clone()));
    assert_eq!(r.unwrap_err().error, PsError::ConfDoneTimeout { padding: CYCLONE2.pad_cap_clocks });
}

#[test]
fn a_broken_source_is_a_decode_error_and_is_not_retried() {
    let data = payload(200, 11);
    let w = world(Cfg::new(data.clone()));
    let (r, fails, made) = run(&AG16K, &w, || {
        let mut s = VecSource::new(data.clone());
        s.broken_at = Some(50);
        s
    });
    let f = r.unwrap_err();
    assert_eq!(f.error, PsError::Decode { produced: 50 });
    assert_eq!(f.attempts, 1);
    assert_eq!(made, 1);
    assert_eq!(fails.len(), 1);
    assert!(!PsError::Decode { produced: 0 }.retryable());
    assert!(!PsError::NoBitstream.retryable());
}

#[test]
fn every_error_has_its_own_blink_code() {
    let all = [
        PsError::NstatusStuckLow,
        PsError::NstatusDropped { byte: 0 },
        PsError::ConfDoneTimeout { padding: 0 },
        PsError::Decode { produced: 0 },
        PsError::NoBitstream,
    ];
    let mut codes: Vec<u8> = all.iter().map(|e| e.code()).collect();
    codes.sort();
    codes.dedup();
    assert_eq!(codes, vec![1, 2, 3, 4, 5]);
}

#[test]
fn profiles_hold_the_documented_values() {
    // CYCLONE2 is the old loader's wire behaviour.
    let c = CYCLONE2;
    assert_eq!(c.bit_order, BitOrder::LsbFirst);
    assert_eq!((c.nops_clk_low, c.nops_data, c.nops_clk_high), (5, 5, 10));
    assert_eq!((c.extra_clocks, c.nstatus_low_wait_us, c.post_nstatus_us), (0, 0, 0));
    assert!(!c.park_low && !c.check_nstatus_in_padding);
    // ag16k-config.md section 6 table.
    let a = AG16K;
    assert_eq!(a.bit_order, BitOrder::LsbFirst);
    assert_eq!((a.ncfg_low_us, a.nstatus_low_wait_us, a.nstatus_high_timeout_us), (1_000, 10_000, 500_000));
    assert_eq!((a.post_nstatus_us, a.extra_clocks, a.pad_cap_clocks, a.retries), (50, 64, 4_096, 3));
    assert_eq!((a.nops_clk_low, a.nops_data, a.nops_clk_high), (0, 0, 0));
    assert!(a.park_low);
}
