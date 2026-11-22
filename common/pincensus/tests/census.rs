use pincensus::*;

/// Builds a pull-up-on and a pull-up-off phase from per-line level
/// sequences (one entry per sample, the same length in each phase).
fn phases(lines: &[(usize, &[bool], &[bool])], n: usize) -> (Phase, Phase) {
    let mut on = Phase::new();
    let mut off = Phase::new();
    for i in 0..n {
        let mut a = [0u32; PORTS];
        let mut b = [0u32; PORTS];
        for &(pin, son, soff) in lines {
            if son[i % son.len()] {
                a[pin / 32] |= 1 << (pin % 32);
            }
            if soff[i % soff.len()] {
                b[pin / 32] |= 1 << (pin % 32);
            }
        }
        on.sample(a);
        off.sample(b);
    }
    (on, off)
}

const H: &[bool] = &[true];
const L: &[bool] = &[false];

#[test]
fn pin_naming() {
    assert_eq!(pin_index(1, 18), 50);
    assert_eq!(port_char(50), 'B');
    assert_eq!(pin_bit(50), 18);
    assert_eq!(port_char(0), 'A');
    assert_eq!(port_char(95), 'C');
    assert_eq!(pin_bit(95), 31);
}

#[test]
fn classes() {
    let pin_hi = pin_index(0, 25);
    let pin_lo = pin_index(1, 1);
    let pin_fl = pin_index(0, 23);
    let pin_fl_mixed = pin_index(0, 24);
    let pin_odd = pin_index(2, 3);
    let pin_unst = pin_index(0, 20);
    let pin_clk = pin_index(0, 21);
    let unstable: &[bool] = &[true, true, true, false, false, false, false, false];
    let clock: &[bool] = &[true, false];
    let (on, off) = phases(
        &[
            (pin_hi, H, H),
            (pin_lo, L, L),
            (pin_fl, H, L),
            (pin_fl_mixed, H, unstable),
            (pin_odd, L, H),
            (pin_unst, unstable, H),
            (pin_clk, clock, clock),
        ],
        8,
    );
    assert_eq!(classify(&on, &off, pin_hi), Class::High);
    assert_eq!(classify(&on, &off, pin_lo), Class::Low);
    assert_eq!(classify(&on, &off, pin_fl), Class::Floating);
    assert_eq!(classify(&on, &off, pin_fl_mixed), Class::Floating);
    assert_eq!(classify(&on, &off, pin_odd), Class::Odd);
    assert_eq!(classify(&on, &off, pin_unst), Class::Unstable);
    assert_eq!(classify(&on, &off, pin_clk), Class::Toggling);
    // Every other line was 0 in both phases.
    assert_eq!(classify(&on, &off, pin_index(2, 31)), Class::Low);
}

#[test]
fn edges_and_toggle_threshold() {
    let p = pin_index(1, 9);
    // 4 edges in each phase: 8 = TOGGLE_MIN -> toggling.
    let s1: &[bool] = &[true, false, true, false, true, true, true, true];
    let s2: &[bool] = &[false, true, false, true, false, false, false, false];
    let (on, off) = phases(&[(p, s1, s2)], 8);
    assert_eq!(on.edges(p), 4);
    assert_eq!(off.edges(p), 4);
    assert_eq!(on.edges(p) + off.edges(p), TOGGLE_MIN);
    assert_eq!(classify(&on, &off, p), Class::Toggling);

    let s3: &[bool] = &[true, false, true, true, true, true, true, true];
    let (on, off) = phases(&[(p, s3, H)], 8);
    assert_eq!(on.edges(p), 2);
    assert_eq!(classify(&on, &off, p), Class::Unstable);
}

#[test]
fn first_sample_is_not_an_edge() {
    let mut ph = Phase::new();
    ph.sample([u32::MAX; PORTS]);
    for pin in 0..NPINS {
        assert_eq!(ph.edges(pin), 0);
        assert_eq!(ph.seen(pin), Seen::OnlyHigh);
    }
    assert_eq!(ph.samples(), 1);
    assert_eq!(Phase::new().seen(0), Seen::Nothing);
}

#[test]
fn edge_counter_saturates() {
    let mut ph = Phase::new();
    // Not 2^32 samples; just check the arithmetic path with many edges.
    for i in 0..10_000u32 {
        ph.sample([if i % 2 == 0 { 1 } else { 0 }, 0, 0]);
    }
    assert_eq!(ph.edges(0), 9_999);
    assert_eq!(ph.edges(1), 0);
}

#[test]
fn masks_cover_every_line_once() {
    let unstable: &[bool] = &[true, false, false, false];
    let (on, off) = phases(
        &[(3, H, H), (40, H, L), (70, unstable, H), (90, &[true, false], &[false, true])],
        16,
    );
    let m = class_masks(&on, &off);
    for pin in 0..NPINS {
        let n = Class::ALL
            .iter()
            .filter(|&&c| m[c as usize][pin / 32] & (1 << (pin % 32)) != 0)
            .count();
        assert_eq!(n, 1, "pin {pin}");
        let c = classify(&on, &off, pin);
        assert!(m[c as usize][pin / 32] & (1 << (pin % 32)) != 0);
    }
    assert_eq!(m[Class::High as usize], [1 << 3, 0, 0]);
    assert_eq!(m[Class::Floating as usize], [0, 1 << 8, 0]);
    assert_eq!(m[Class::Toggling as usize], [0, 0, 1 << 26]);
}

#[test]
fn roles_are_unique_sorted_and_named() {
    let mut last = None;
    for r in V55_ROLES {
        assert!((r.pin as usize) < NPINS);
        assert!(!r.name.is_empty());
        if let Some(l) = last {
            assert!(r.pin > l, "roles must be in pin order without duplicates");
        }
        last = Some(r.pin);
        assert_eq!(role(r.pin as usize).unwrap().name, r.name);
        // Expect sets only use real classes.
        assert_eq!(r.expect & !0x3F, 0);
    }
    assert!(role(pin_index(2, 31)).is_none());
}

#[test]
fn roles_match_the_firmware_pin_map() {
    // cyclone::pinmap and embassy-atsam3 UserPeripherals (v5.5 assumption).
    let name = |p| role(p).unwrap().name;
    assert_eq!(name(pin_index(0, 23)), "PS DATA0");
    assert_eq!(name(pin_index(0, 24)), "PS DCLK");
    assert_eq!(name(pin_index(0, 25)), "PS nCONFIG");
    assert_eq!(name(pin_index(1, 1)), "PS CONF_DONE");
    assert_eq!(name(pin_index(1, 2)), "PS nSTATUS");
    assert_eq!(name(pin_index(1, 18)), "LED");
    assert_eq!(name(pin_index(0, 20)), "button");
    assert_eq!(name(pin_index(0, 22)), "FPGA MCU_IRQn");
    // smc.rs: PIOB_MASK and PIOC_MASK.
    let smc_b: u32 = (1 << 6) | (1 << 7) | (1 << 8) | (0xFF << 9) | (1 << 19) | (1 << 23) | (0x7F << 25);
    let smc_c: u32 = 0x7F | (1 << 15) | (1 << 17);
    for bit in 0..32 {
        let pb = pin_index(1, bit);
        let pc = pin_index(2, bit);
        if smc_b & (1 << bit) != 0 {
            assert!(name(pb).starts_with("SMC"), "PB{bit}");
        }
        if smc_c & (1 << bit) != 0 {
            assert!(name(pc).starts_with("SMC"), "PC{bit}");
        }
    }
    // hsmci.rs: PA3..PA8.
    for bit in 3..=8 {
        assert!(name(pin_index(0, bit)).starts_with("HSMCI"));
    }
}

#[test]
fn ps_expectations() {
    let conf_done = role(pin_index(1, 1)).unwrap();
    assert_eq!(agrees(conf_done, Class::Low), Some(true));
    assert_eq!(agrees(conf_done, Class::High), Some(false));
    assert_eq!(agrees(conf_done, Class::Floating), Some(false));
    let nconfig = role(pin_index(0, 25)).unwrap();
    assert_eq!(agrees(nconfig, Class::High), Some(true));
    assert_eq!(agrees(nconfig, Class::Floating), Some(true));
    assert_eq!(agrees(nconfig, Class::Low), Some(false));
    assert_eq!(agrees(nconfig, Class::Toggling), Some(false));
    let nstatus = role(pin_index(1, 2)).unwrap();
    assert_eq!(agrees(nstatus, Class::High), Some(true));
    assert_eq!(agrees(nstatus, Class::Low), Some(false));
    let smc = role(pin_index(1, 9)).unwrap();
    assert_eq!(agrees(smc, Class::Low), None);
}

#[test]
fn watch_reports_changes_up_to_the_limit() {
    let button = pin_index(0, 20);
    let clock = pin_index(0, 21);
    let mut w = Watch::new([1 << 21, 0, 0], 2);
    // First reading only sets the baseline.
    assert_eq!(w.update([u32::MAX, 0, 0]), [0, 0, 0]);
    // Press: PA20 low; the ignored clock line changes too.
    let r = w.update([!(1 << 20) & !(1 << 21), 0, 0]);
    assert_eq!(r, [1 << 20, 0, 0]);
    assert_eq!(pins(r).collect::<Vec<_>>(), vec![button]);
    // Release: reported again (2 of 2).
    assert_eq!(w.update([u32::MAX, 0, 0]), [1 << 20, 0, 0]);
    // Third change: over the limit, silent.
    assert_eq!(w.update([!(1 << 20), 0, 0]), [0, 0, 0]);
    assert_eq!(w.reported(button), 2);
    assert_eq!(w.reported(clock), 0);
    // A card-detect line on another port still reports.
    let r = w.update([!(1 << 20), 0, 1 << 5]);
    assert_eq!(pins(r).collect::<Vec<_>>(), vec![pin_index(2, 5)]);
}

#[test]
fn pins_iterates_in_order() {
    let v: Vec<usize> = pins([1 | (1 << 31), 1 << 4, 1 << 31]).collect();
    assert_eq!(v, vec![0, 31, 36, 95]);
    assert_eq!(pins([0; PORTS]).count(), 0);
}

#[test]
fn labels_are_distinct() {
    for (i, a) in Class::ALL.iter().enumerate() {
        for b in &Class::ALL[i + 1..] {
            assert_ne!(a.label(), b.label());
            assert_ne!(a.bit(), b.bit());
        }
    }
}
