//! A reference run of the resident (`tests/common/sim.rs`) against the host
//! models: the lock procedure (design 4-1, B7), the FWS rule (K6, B4), GPBR0
//! and GPBR1 (G1, G2, B6, X6), the watchdog rewind interval (W3, B5).

mod common;

use bootcore::app::{verify_flash_app, APP_BASE};
use bootcore::boot::*;
use bootcore::fcr::Plane;
use bootcore::flash::{Flash, PLANE1_BASE};
use bootcore::part::{Part, FWS_RUN, FWS_WRITE};
use common::sim::*;
use common::*;

const PARTS: [Part; 2] = [Part::Sam3u2e, Part::Sam3u4e];

fn board(part: Part, dev: bool, installed: Option<&[u8]>, card: Option<&[u8]>) -> Board {
    let mut b = Board::new(part, dev);
    if let Some(a) = installed {
        b.install_app(a);
    }
    if let Some(c) = card {
        b.set_card(c);
    }
    b
}

fn app_valid(b: &mut Board) -> bool {
    verify_flash_app(&mut b.flash, b.part, b.dev).is_ok()
}

fn installed_is(b: &Board, img: &[u8]) -> bool {
    b.flash.peek(APP_BASE, img.len()) == img
}

// ------------------------------------------------------------------ B7: lock procedure

#[test]
fn lock_step_and_report_tables() {
    for glb in [0u32, 1, 2, 3, 0x8000_0000, 0xFFFF_FFFE, 0xFFFF_FFFF, 0x0000_FFFE] {
        for dev in [false, true] {
            let want = if dev || glb & 1 != 0 { LockStep::Skip } else { LockStep::Slb };
            assert_eq!(lock_step(glb, dev), want, "glb={glb:#x} dev={dev}");
        }
        assert_eq!(lock_report(glb), glb & 1 != 0);
    }
    // a GLB that failed reads as all ones: reported locked, no SLB tried
    assert_eq!(lock_step(u32::MAX, false), LockStep::Skip);
    assert!(lock_report(u32::MAX));
}

#[test]
fn first_boot_locks_and_later_boots_do_not_touch_the_lock() {
    for part in PARTS {
        let app = app_image(part, 2048, 1);
        let mut b = board(part, false, Some(&app), None);
        b.flash.lock[0] = 0; // factory fresh: region 0 not locked
        let Boot::Jump(r) = b.boot() else { panic!() };
        assert!(r.slb && r.locked && !r.wrote);
        assert_eq!(r.result, RESULT_NONE);
        assert_eq!(b.flash.slbs, 1);
        assert_eq!(b.flash.lock[0] & 1, 1);
        for _ in 0..3 {
            let Boot::Jump(r) = b.boot() else { panic!() };
            assert!(!r.slb && r.locked);
        }
        assert_eq!(b.flash.slbs, 1);
        assert_eq!(b.flash.slb_while_locked, 0);
        assert_eq!(b.flash.fws_violations, 0);
        assert_eq!(b.flash.fws, FWS_RUN);
    }
}

#[test]
fn a_development_resident_never_locks() {
    let part = Part::Sam3u4e;
    let app = app_image(part, 2048, 1);
    let mut b = board(part, true, Some(&app), None);
    b.flash.lock[0] = 0;
    let Boot::Jump(r) = b.boot() else { panic!() };
    assert!(!r.slb && !r.locked);
    assert_eq!(b.flash.slbs, 0);
    // and it reports a lock that is there
    b.flash.lock[0] = 1;
    let Boot::Jump(r) = b.boot() else { panic!() };
    assert!(!r.slb && r.locked);
}

#[test]
fn slb_that_turns_into_clb_or_does_nothing_is_retried_next_boot() {
    for part in PARTS {
        for fault in [SlbFault::BecomesClb, SlbFault::Ignored] {
            let app = app_image(part, 2048, 1);
            let mut b = board(part, false, Some(&app), None);
            b.flash.lock[0] = 0;
            b.flash.slb_fault = Some(fault);
            // the boot goes on, the report says what is true
            let Boot::Jump(r) = b.boot() else { panic!() };
            assert!(r.slb && !r.locked, "{fault:?}");
            assert_eq!(b.flash.lock[0] & 1, 0);
            // the next boot sees an unlocked region and issues SLB again
            let Boot::Jump(r) = b.boot() else { panic!() };
            assert!(r.slb && r.locked);
            assert_eq!(b.flash.slbs, 2);
            // and now it stays
            let Boot::Jump(r) = b.boot() else { panic!() };
            assert!(!r.slb && r.locked);
            assert_eq!(b.flash.slbs, 2);
            assert_eq!(b.flash.slb_while_locked, 0, "SLB is never issued to a locked region");
            assert_eq!(b.flash.fws_violations, 0);
        }
    }
}

#[test]
fn power_cut_during_slb_recovers_on_the_next_boot() {
    for part in PARTS {
        for bit_set in [false, true] {
            let app = app_image(part, 2048, 1);
            let mut b = board(part, false, Some(&app), None);
            b.flash.lock[0] = 0;
            b.flash.slb_fault = Some(SlbFault::PowerCut { bit_set });
            assert_eq!(b.boot(), Boot::PowerLost);
            b.reset();
            let Boot::Jump(r) = b.boot() else { panic!() };
            assert!(r.locked, "bit_set={bit_set}");
            // the cut happened inside SLB: if the bit survived, nothing more to do; if not, SLB again
            assert_eq!(r.slb, !bit_set);
            assert_eq!(b.flash.slbs, if bit_set { 1 } else { 2 });
            assert_eq!(b.flash.slb_while_locked, 0);
            assert!(app_valid(&mut b), "the cut did not touch the application");
            assert_eq!(b.flash.fws_violations, 0);
        }
    }
}

#[test]
fn the_slb_word_is_slb_0_on_eefc0_and_only_on_an_unlocked_region() {
    // the model asserts the word (0x5A000008) and the register block in `slb`
    let part = Part::Sam3u4e;
    let mut m = Model::new(part);
    m.lock[0] = 0;
    let out = lock_resident(&mut m, part, false);
    assert_eq!(out, LockOutcome { locked: true, slb: true });
    let out = lock_resident(&mut m, part, false);
    assert_eq!(out, LockOutcome { locked: true, slb: false });
    assert_eq!(m.slbs, 1);
    assert_eq!(m.fws, FWS_RUN);
    assert_eq!(m.lock_bits(Plane::P0) & 1, 1);
}

// ------------------------------------------------------------------ B4: FWS

#[test]
fn the_model_counts_wrong_fws_as_violations() {
    let part = Part::Sam3u2e;
    let mut m = Model::new(part);
    let cmd = bootcore::fcr::FcrCmd::ewp(part, Plane::P0, 40).unwrap();
    // EWP at FWS 4
    m.ewp(cmd, &[0u8; 256]).unwrap();
    assert_eq!(m.fws_violations, 1);
    // EWP at FWS 6 is right
    m.set_fws(FWS_WRITE);
    m.ewp(cmd, &[0u8; 256]).unwrap();
    assert_eq!(m.fws_violations, 1);
    // a read, or the hand-off, at FWS 6
    let mut x = [0u8; 4];
    m.read(APP_BASE, &mut x);
    assert_eq!(m.fws_violations, 2);
    m.set_fws(FWS_RUN);
    m.read(APP_BASE, &mut x);
    assert_eq!(m.fws_violations, 2);
    // SLB at FWS 4
    m.lock[0] = 0;
    m.slb(bootcore::fcr::FcrCmd::slb(part, true).unwrap()).unwrap();
    assert_eq!(m.fws_violations, 3);
}

/// Every way a recovery can end leaves FWS at 4 and made no flash command at the wrong FWS.
#[test]
fn recovery_keeps_to_the_fws_rule_on_every_ending() {
    for part in PARTS {
        let old = app_image(part, 4096, 1);
        let new = app_image(part, 4096, 2);
        let file = image(0, part, &new);
        // success, same image, no card, bad file, locked, flash timeout, flash error mid-way
        let mut cases: Vec<(&str, Board)> = vec![];
        let mut b = board(part, false, Some(&old), Some(&file));
        b.gpbr0 = GPBR_RECOVER_MAGIC;
        cases.push(("success", b));
        let mut b = board(part, false, Some(&new), Some(&file));
        b.gpbr0 = GPBR_RECOVER_MAGIC;
        cases.push(("same", b));
        let mut b = board(part, false, Some(&old), None);
        b.gpbr0 = GPBR_RECOVER_MAGIC;
        cases.push(("no card", b));
        let mut bad = file.clone();
        bad[600] ^= 1;
        let mut b = board(part, false, Some(&old), Some(&bad));
        b.gpbr0 = GPBR_RECOVER_MAGIC;
        cases.push(("bad file", b));
        let mut b = board(part, false, Some(&old), Some(&file));
        b.gpbr0 = GPBR_RECOVER_MAGIC;
        b.flash.lock[0] |= 1 << 1;
        cases.push(("locked", b));
        let mut b = board(part, false, Some(&old), Some(&file));
        b.gpbr0 = GPBR_RECOVER_MAGIC;
        b.flash.timeout = Some(Cut { op: 3, outcome: Outcome::Garbage });
        cases.push(("timeout", b));
        let mut b = board(part, false, Some(&old), Some(&file));
        b.gpbr0 = GPBR_RECOVER_MAGIC;
        b.flash.stuck = Some(APP_BASE + 0x1000);
        cases.push(("stuck bit", b));
        for (name, mut b) in cases {
            let r = b.boot();
            assert!(matches!(r, Boot::Jump(_) | Boot::Waiting(_)), "{name}: {r:?}");
            assert_eq!(b.flash.fws, FWS_RUN, "{name}: FWS at the end");
            assert_eq!(b.flash.fws_violations, 0, "{name}");
            assert_eq!(b.flash.violations, 0, "{name}");
        }
    }
}

// ------------------------------------------------------------------ B6: GPBR

#[test]
fn request_for_the_installed_image_clears_gpbr0_writes_nothing_and_reports_req() {
    for part in PARTS {
        let app = app_image(part, 4096, 1);
        let mut b = board(part, false, Some(&app), Some(&image(0, part, &app)));
        b.gpbr0 = GPBR_RECOVER_MAGIC;
        let Boot::Jump(r) = b.boot() else { panic!() };
        assert_eq!(b.gpbr0, 0);
        assert!(r.trigger.request && !r.trigger.button);
        assert_eq!(r.result, RESULT_RECOVERED);
        assert!(!r.wrote);
        assert_eq!(b.flash.ops, 0);
        assert!(!request_allowed(Report::parse(b.gpbr1)));
        // the next reset is an ordinary boot and asks nothing of the card
        b.reset();
        let Boot::Jump(r) = b.boot() else { panic!() };
        assert_eq!(r.result, RESULT_NONE);
        assert!(!r.trigger.request);
        assert!(request_allowed(Report::parse(b.gpbr1)));
    }
}

#[test]
fn request_with_a_new_image_writes_it_and_reports_wrote() {
    for part in PARTS {
        let old = app_image(part, 4096, 1);
        let new = app_image(part, 6144, 2);
        let mut b = board(part, false, Some(&old), Some(&image(0, part, &new)));
        b.gpbr0 = GPBR_RECOVER_MAGIC;
        let Boot::Jump(r) = b.boot() else { panic!() };
        assert_eq!(b.gpbr0, 0);
        assert!(r.trigger.request && r.wrote && r.result == RESULT_RECOVERED);
        assert!(installed_is(&b, &new));
        assert_eq!(b.flash.fws_violations, 0);
    }
}

#[test]
fn request_that_fails_still_clears_gpbr0_and_starts_the_old_app() {
    for part in PARTS {
        let old = app_image(part, 4096, 1);
        let new = app_image(part, 4096, 2);
        let file = image(0, part, &new);
        let mut bad = file.clone();
        bad[700] ^= 1;
        // (card, expected result)
        let cases: Vec<(Option<Vec<u8>>, u8)> = vec![
            (None, RESULT_NO_CARD),
            (Some(bad), RESULT_CRC),
            (Some(image(0, Part::Sam3u2e, &app_image(Part::Sam3u2e, 4096, 3))), if part == Part::Sam3u2e { RESULT_RECOVERED } else { RESULT_HEADER }),
        ];
        for (card, want) in cases {
            let mut b = board(part, false, Some(&old), card.as_deref());
            b.gpbr0 = GPBR_RECOVER_MAGIC;
            let Boot::Jump(r) = b.boot() else { panic!() };
            assert_eq!(b.gpbr0, 0);
            assert_eq!(r.result, want);
            assert!(r.trigger.request);
            assert!(!request_allowed(Some(r)));
            if want != RESULT_RECOVERED {
                assert!(installed_is(&b, &old), "the old app is untouched");
            }
        }
    }
}

#[test]
fn request_with_no_app_and_no_card_clears_gpbr0_and_waits() {
    let part = Part::Sam3u2e;
    let mut b = board(part, false, None, None);
    b.gpbr0 = GPBR_RECOVER_MAGIC;
    assert_eq!(b.boot(), Boot::Waiting(RESULT_NO_CARD));
    assert_eq!(b.gpbr0, 0);
    // a card arrives; the next boot (the resident keeps polling, or a reset) recovers without any request
    let app = app_image(part, 2048, 4);
    b.set_card(&image(0, part, &app));
    let Boot::Jump(r) = b.boot() else { panic!() };
    assert!(!r.trigger.request && !r.trigger.button);
    assert_eq!((r.result, r.wrote), (RESULT_RECOVERED, true));
    assert!(installed_is(&b, &app));
}

#[test]
fn anything_but_the_magic_leaves_gpbr0_alone_and_triggers_nothing() {
    let part = Part::Sam3u2e;
    let app = app_image(part, 2048, 1);
    for g in [0u32, 0xFFFF_FFFF, 0x5A5A_A5A5, GPBR_RECOVER_MAGIC ^ 1, GPBR_RECOVER_MAGIC.rotate_left(8), 0x4EB1_0001] {
        let mut b = board(part, false, Some(&app), Some(&image(0, part, &app)));
        b.gpbr0 = g;
        let Boot::Jump(r) = b.boot() else { panic!() };
        assert_eq!(b.gpbr0, g, "GPBR0 is not the resident's to change");
        assert_eq!(r.result, RESULT_NONE);
        assert!(!r.trigger.request && !r.trigger.button);
        assert_eq!(b.flash.ops, 0);
    }
}

#[test]
fn gpbr1_is_rewritten_on_every_boot() {
    let part = Part::Sam3u2e;
    let app = app_image(part, 2048, 1);
    let mut b = board(part, false, Some(&app), Some(&image(0, part, &app)));
    b.gpbr0 = GPBR_RECOVER_MAGIC;
    b.boot();
    let first = b.gpbr1;
    assert_ne!(first & REPORT_REQ, 0);
    b.boot();
    assert_ne!(b.gpbr1, first);
    assert_eq!(b.gpbr1 & REPORT_REQ, 0, "no trace of the previous boot's request");
}

#[test]
fn button_boot_reports_btn_and_does_not_rewrite() {
    for part in PARTS {
        let app = app_image(part, 4096, 1);
        let mut b = board(part, false, Some(&app), Some(&image(0, part, &app)));
        b.button = true;
        for _ in 0..3 {
            let Boot::Jump(r) = b.boot() else { panic!() };
            assert!(r.trigger.button && !r.trigger.request);
            assert_eq!((r.result, r.wrote), (RESULT_RECOVERED, false));
            // pressing the button is not a request: the app may still ask once
            assert!(request_allowed(Some(r)));
        }
        assert_eq!(b.flash.ops, 0);
    }
}

/// An application that always asks for recovery (its FPGA never starts, and
/// recovery cannot help): it must reset once and then stay, whatever the card holds.
#[test]
fn no_second_request_after_a_req_report() {
    for part in PARTS {
        let old = app_image(part, 4096, 1);
        let new = app_image(part, 4096, 2);
        let mut bad = image(0, part, &new);
        bad[900] ^= 4;
        for (name, card) in [
            ("same image", Some(image(0, part, &old))),
            ("new image", Some(image(0, part, &new))),
            ("no card", None),
            ("bad file", Some(bad)),
        ] {
            let mut b = board(part, false, Some(&old), card.as_deref());
            let (last, boots) = b.run_app_that_wants_recovery(10);
            let Boot::Jump(r) = last else { panic!("{name}: {last:?}") };
            assert_eq!(boots, 2, "{name}: boot, recover once, stay");
            assert_eq!(b.app_resets, 1, "{name}");
            assert!(r.trigger.request, "{name}");
            assert_eq!(b.gpbr0, 0, "{name}");
            assert!(app_valid(&mut b), "{name}");
            assert_eq!(b.flash.fws_violations, 0, "{name}");
        }
        // a second power-on starts the story again (one more request, once)
        let mut b = board(part, false, Some(&old), None);
        b.run_app_that_wants_recovery(10);
        b.reset();
        let (_, boots) = b.run_app_that_wants_recovery(10);
        assert_eq!(boots, 2);
        assert_eq!(b.app_resets, 2);
    }
}

/// The card fails at every read of a requested recovery.
#[test]
fn request_with_a_failing_card_never_leaves_the_request_standing() {
    let part = Part::Sam3u4e;
    let old = app_image(part, 2048, 1);
    let new = app_image(part, 2048, 2);
    let file = image(0, part, &new);
    for n in 0..40 {
        let mut b = board(part, false, Some(&old), Some(&file));
        b.card_fail_at = Some(n);
        b.gpbr0 = GPBR_RECOVER_MAGIC;
        let first = b.boot();
        assert_eq!(b.gpbr0, 0, "read {n}");
        match first {
            Boot::Jump(r) => {
                assert!(r.trigger.request);
                assert!(app_valid(&mut b), "read {n}");
                assert!(
                    r.result == RESULT_RECOVERED || (RESULT_NO_FILE..=RESULT_VERIFY).contains(&r.result),
                    "read {n}: {}",
                    r.result
                );
            }
            // the card failed after the head was erased: no valid app, the resident
            // keeps polling the card, and a healthy card finishes the job without a request
            Boot::Waiting(code) => {
                assert!((RESULT_NO_FILE..=RESULT_VERIFY).contains(&code), "read {n}: {code}");
                assert!(!app_valid(&mut b));
                b.card_fail_at = None;
                let Boot::Jump(r) = b.boot() else { panic!("read {n}") };
                assert!(!r.trigger.request && r.result == RESULT_RECOVERED);
                assert!(installed_is(&b, &new), "read {n}");
            }
            Boot::PowerLost => panic!("read {n}"),
        }
    }
}

/// Power cut at every EWP of a requested recovery: the request was cleared
/// before anything was written, so after the cut there is no request, and the
/// board gets to a valid application within two boots, never looping.
#[test]
fn power_cut_during_a_requested_recovery_converges() {
    let part = Part::Sam3u4e;
    let old = app_image(part, 1024, 1);
    let new = app_image(part, 3072, 2);
    let file = image(0, part, &new);
    // how many EWPs a full recovery takes
    let mut probe = board(part, false, Some(&old), Some(&file));
    probe.gpbr0 = GPBR_RECOVER_MAGIC;
    assert!(matches!(probe.boot(), Boot::Jump(_)));
    let total = probe.flash.ops;
    assert!(total > 10, "{total}");
    let outcomes = [Outcome::Untouched, Outcome::Erased, Outcome::Prefix(100), Outcome::ErasePrefix(60), Outcome::Garbage];
    let mut runs = 0;
    for op in 0..total {
        for outcome in outcomes {
            let mut b = board(part, false, Some(&old), Some(&file));
            b.gpbr0 = GPBR_RECOVER_MAGIC;
            b.flash.cut = Some(Cut { op, outcome });
            assert_eq!(b.boot(), Boot::PowerLost, "op {op} {outcome:?}");
            assert_eq!(b.gpbr0, 0, "the request is gone before the first write (op {op})");
            b.reset();
            // up to two more boots until a valid application runs
            let mut jumped = None;
            for _ in 0..2 {
                match b.boot() {
                    Boot::Jump(r) => {
                        jumped = Some(r);
                        break;
                    }
                    other => panic!("op {op} {outcome:?}: {other:?}"),
                }
            }
            let r = jumped.unwrap_or_else(|| panic!("op {op} {outcome:?}: no jump"));
            assert!(!r.trigger.request, "op {op}");
            assert!(app_valid(&mut b), "op {op} {outcome:?}");
            // the app is either the old one (cut before the head was erased) or the new one
            assert!(installed_is(&b, &old) || installed_is(&b, &new), "op {op} {outcome:?}");
            assert_eq!(b.flash.fws_violations, 0, "op {op} {outcome:?}");
            runs += 1;
        }
    }
    assert_eq!(runs, total * outcomes.len());
}

// ------------------------------------------------------------------ B5: watchdog

/// Bound on the flash bytes the pure code reads between two rewinds (SD
/// sectors and EWPs rewind). 256 KB is about 40 ms at the design's 150 ns/B
/// and 0.26 s at a pessimistic 1 us/B: far inside the 16 s of the reset
/// value of the watchdog and inside the 2 s that W3 allows.
const WDT_BOUND_BYTES: u64 = 256 * 1024;

#[test]
fn watchdog_is_rewound_often_enough_in_every_path() {
    for part in PARTS {
        let max_old = bootcore::app::APP_MAX_LEN as usize;
        let old = app_image(part, max_old, 1);
        let new = app_image(part, max_old, 2);
        let mk = |installed: Option<&[u8]>, card: &[u8], button: bool, req: bool| {
            let mut b = board(part, false, installed, Some(card));
            b.button = button;
            if req {
                b.gpbr0 = GPBR_RECOVER_MAGIC;
            }
            b
        };
        let file_new = image(0, part, &new);
        let mut file_both = image(0, part, &new);
        if part == Part::Sam3u4e {
            file_both.extend_from_slice(&image(1, part, &bitstream(0x20000, 7)));
        }
        let paths: Vec<(&str, Board)> = vec![
            ("normal start", mk(Some(&old), &file_new, false, false)),
            ("button, same image", mk(Some(&new), &file_new, true, false)),
            ("request, new image", mk(Some(&old), &file_new, false, true)),
            ("no app, whole file", mk(None, &file_both, false, false)),
            ("request, same image with bitstream", {
                let mut b = mk(Some(&new), &file_both, false, true);
                if part == Part::Sam3u4e {
                    b.flash.poke(PLANE1_BASE, &bitstream(0x20000, 7));
                }
                b
            }),
        ];
        for (name, mut b) in paths {
            let r = b.boot();
            assert!(matches!(r, Boot::Jump(_)), "{name}: {r:?}");
            let s = b.wdt.state();
            assert!(s.max <= WDT_BOUND_BYTES, "{name}: {} flash bytes between two rewinds", s.max);
            assert!(s.feeds >= 1);
            eprintln!("{part:?} {name}: longest stretch {} bytes, {} rewinds", s.max, s.feeds);
        }
    }
}

#[test]
fn recovery_rewinds_at_every_sector_and_every_page() {
    let part = Part::Sam3u2e;
    let old = app_image(part, 2048, 1);
    let new = app_image(part, 8192, 2);
    let mut b = board(part, false, Some(&old), Some(&image(0, part, &new)));
    b.gpbr0 = GPBR_RECOVER_MAGIC;
    assert!(matches!(b.boot(), Boot::Jump(_)));
    let s = b.wdt.state();
    // one rewind per EWP (8192 B = 32 pages + the head erases) plus the card reads
    assert!(s.feeds as usize >= b.flash.ops, "{} rewinds for {} EWPs", s.feeds, b.flash.ops);
    assert!(s.feeds > 100);
}
