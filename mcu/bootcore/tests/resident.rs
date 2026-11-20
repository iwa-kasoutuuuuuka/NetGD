//! A reference run of the resident (`tests/common/sim.rs`) against the host
//! models: the lock procedure (design 4-1, B7), the FWS rule (K6, B4), GPBR0
//! and GPBR1 (G1, G2, B6, X6), the watchdog rewind interval (W3, B5).

mod common;

use bootcore::app::{verify_flash_app, APP_BASE};
use bootcore::boot::*;
use bootcore::fcr::Plane;
use bootcore::fat::NAME_APP;
use bootcore::flash::{Flash, PLANE1_BASE};
use bootcore::update::{lock_verdict, sd_recover_attempt, sd_recover_ex, RecoverError};
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
            assert_eq!(lock_step(Some(glb), dev), want, "glb={glb:#x} dev={dev}");
        }
        assert_eq!(lock_report(Some(glb)), glb & 1 != 0);
        assert!(!lock_unknown(Some(glb)));
    }
    // B12: a GLB that failed is not "locked": no SLB is tried, and the report
    // says unknown, not locked
    for dev in [false, true] {
        assert_eq!(lock_step(None, dev), LockStep::Skip);
    }
    assert!(!lock_report(None));
    assert!(lock_unknown(None));
}

// ------------------------------------------------------------------ B12: a GLB that fails

#[test]
fn a_failed_glb_is_reported_unknown_and_never_locked() {
    for part in PARTS {
        let app = app_image(part, 2048, 1);
        // whatever the lock bit really is
        for really_locked in [false, true] {
            let mut b = board(part, false, Some(&app), None);
            b.flash.lock[0] = really_locked as u32;
            b.flash.glb_fail[0] = u32::MAX;
            let Boot::Jump(r) = b.boot() else { panic!() };
            assert!(r.lock_unknown && !r.locked && !r.slb, "{really_locked}");
            assert_ne!(b.gpbr1 & REPORT_LOCK_UNKNOWN, 0);
            assert_eq!(b.gpbr1 & REPORT_LOCKED, 0);
            assert_eq!(b.flash.slbs, 0, "no SLB without knowing the lock bit");
            assert_eq!(r.result, RESULT_NONE, "the board still boots");
            // the GLB works again: the next boot reports the truth and locks if it has to
            b.flash.glb_fail[0] = 0;
            let Boot::Jump(r) = b.boot() else { panic!() };
            assert!(!r.lock_unknown && r.locked);
            assert_eq!(r.slb, !really_locked);
            assert_eq!(b.flash.slb_while_locked, 0);
        }
        // only the GLB after the SLB fails: SLB was issued, the outcome is unknown
        let mut m = Model::new(part);
        m.lock[0] = 0;
        m.set_fws(FWS_RUN);
        let mut lf = FailNthGlb(m, 1);
        let out = lock_resident(&mut lf, part, false);
        assert_eq!(out, LockOutcome { locked: false, slb: true, unknown: true });
        assert_eq!(lf.0.lock[0] & 1, 1);
    }
}

/// Fails the n-th GLB (0-based) of plane 0.
struct FailNthGlb(Model, u32);

impl Flash for FailNthGlb {
    fn read(&mut self, a: u32, o: &mut [u8]) {
        self.0.read(a, o)
    }
    fn set_fws(&mut self, f: u8) {
        self.0.set_fws(f)
    }
    fn ewp(&mut self, c: bootcore::fcr::FcrCmd, d: &[u8; 256]) -> Result<(), bootcore::flash::FlashError> {
        self.0.ewp(c, d)
    }
    fn slb(&mut self, c: bootcore::fcr::FcrCmd) -> Result<(), bootcore::flash::FlashError> {
        self.0.slb(c)
    }
    fn lock_bits(&mut self, p: Plane) -> Option<u32> {
        let n = self.0.glbs[0];
        let r = self.0.lock_bits(p);
        if p == Plane::P0 && n == self.1 {
            None
        } else {
            r
        }
    }
}

#[test]
fn lock_verdict_table() {
    for valid in [false, true] {
        assert_eq!(lock_verdict(Some(0), 0xFFFE, valid), Ok(()));
        assert_eq!(lock_verdict(Some(1), 0xFFFE, valid), Ok(()), "the resident's own lock is not in the mask");
        assert_eq!(lock_verdict(Some(2), 0xFFFE, valid), Err(RecoverError::Locked));
        assert_eq!(lock_verdict(Some(u32::MAX), 0x0002, valid), Err(RecoverError::Locked));
    }
    // unknown: a valid application is not put at risk, an invalid one is rewritten anyway
    assert_eq!(lock_verdict(None, 0xFFFE, true), Err(RecoverError::LockUnknown));
    assert_eq!(lock_verdict(None, 0xFFFE, false), Ok(()));
    assert_eq!(RecoverError::LockUnknown.result_code(), RESULT_LOCKED);
}

/// A GLB that fails does not refuse recovery for ever: with no valid
/// application the recovery goes ahead (FLOCKE guards a page that is really
/// locked); with a valid one this attempt is refused and the application is
/// untouched, and the attempt after the GLB works again succeeds.
#[test]
fn a_failed_glb_does_not_refuse_recovery_for_ever() {
    for part in PARTS {
        let old = app_image(part, 4096, 1);
        let new = app_image(part, 6144, 2);
        let mut file = image(0, part, &new);
        if part.has_plane1() {
            file.extend_from_slice(&image(1, part, &bitstream(2048, 3)));
        }
        let planes: &[usize] = if part.has_plane1() { &[0, 1] } else { &[0] };
        for &pl in planes {
            // no valid application: recovers although GLB never answers
            let mut b = board(part, false, None, Some(&file));
            b.flash.glb_fail[pl] = u32::MAX;
            let Boot::Jump(r) = b.boot() else { panic!("{part:?} plane {pl}") };
            assert_eq!((r.result, r.wrote), (RESULT_RECOVERED, true));
            assert_eq!(r.lock_unknown, pl == 0);
            assert!(installed_is(&b, &new));
            assert_eq!(b.flash.fws_violations, 0);

            // a valid application and a request: refused this time, old app runs untouched
            let mut b = board(part, false, Some(&old), Some(&file));
            b.flash.glb_fail[pl] = u32::MAX;
            b.gpbr0 = GPBR_RECOVER_MAGIC;
            let Boot::Jump(r) = b.boot() else { panic!() };
            assert_eq!(r.result, RESULT_LOCKED);
            assert!(!r.wrote && r.trigger.request);
            assert_eq!(b.flash.ops, 0, "nothing was written");
            assert!(installed_is(&b, &old));
            // GLB answers again: the same request now goes through
            b.flash.glb_fail[pl] = 0;
            b.reset();
            b.gpbr0 = GPBR_RECOVER_MAGIC;
            let Boot::Jump(r) = b.boot() else { panic!() };
            assert_eq!((r.result, r.wrote), (RESULT_RECOVERED, true));
            assert!(!r.lock_unknown && r.locked);
            assert!(installed_is(&b, &new));
        }
        // no valid application, GLB dead, and the application's region really is
        // locked: FLOCKE stops it, nothing else can be done, and it is not worse
        // than before (no application either way)
        let mut b = board(part, false, None, Some(&file));
        b.flash.glb_fail[0] = u32::MAX;
        b.flash.lock[0] |= 1 << 1;
        assert_eq!(b.boot(), Boot::Waiting(RESULT_LOCKED));
        assert!(b.flash.flocke >= 1);
        assert_eq!(&b.flash.peek(bootcore::flash::FLASH_BASE, 0x2000)[..], &lcg(99, 0x2000)[..], "the resident is untouched");
    }
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
        assert_eq!(b.flash.fws, Some(FWS_RUN));
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
    m.set_fws(FWS_RUN);
    let out = lock_resident(&mut m, part, false);
    assert_eq!(out, LockOutcome { locked: true, slb: true, unknown: false });
    let out = lock_resident(&mut m, part, false);
    assert_eq!(out, LockOutcome { locked: true, slb: false, unknown: false });
    assert_eq!(m.slbs, 1);
    assert_eq!(m.fws, Some(FWS_RUN));
    assert_eq!(m.lock_bits(Plane::P0).unwrap() & 1, 1);
}

// ------------------------------------------------------------------ B4: FWS

#[test]
fn the_model_counts_wrong_fws_as_violations() {
    let part = Part::Sam3u2e;
    let mut m = Model::new(part);
    let cmd = bootcore::fcr::FcrCmd::ewp(part, Plane::P0, 40).unwrap();
    // B2: a read before FWS was ever set (the reset value 0)
    let mut x = [0u8; 4];
    assert_eq!(m.fws, None);
    m.read(APP_BASE, &mut x);
    assert_eq!(m.fws_violations, 1);
    m.set_fws(FWS_RUN);
    m.read(APP_BASE, &mut x);
    assert_eq!(m.fws_violations, 1);
    // ... and again after a reset
    m.power_up();
    assert_eq!(m.fws, None);
    m.read(APP_BASE, &mut x);
    assert_eq!(m.fws_violations, 2);
    m.fws_violations = 0;
    m.set_fws(FWS_RUN);
    // EWP at FWS 4
    m.ewp(cmd, &[0u8; 256]).unwrap();
    assert_eq!(m.fws_violations, 1);
    // EWP at FWS 6 is right
    m.set_fws(FWS_WRITE);
    m.ewp(cmd, &[0u8; 256]).unwrap();
    assert_eq!(m.fws_violations, 1);
    // a read, or the hand-off, at FWS 6
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

/// B2: `sd_recover` sets FWS itself before it reads flash. Straight after a
/// reset nobody has set EEFC1's FWS on a 4E (the resident's start-up sets
/// plane 0 for its own code; the model has one FWS for all planes and starts
/// "unset"), and plane 1 read at FWS 0 would look different from the file.
#[test]
fn sd_recover_sets_fws_before_its_first_flash_read() {
    for part in PARTS {
        let app = app_image(part, 4096, 1);
        let bs = bitstream(2048, 5);
        let mut file = image(0, part, &app);
        if part.has_plane1() {
            file.extend_from_slice(&image(1, part, &bs));
        }
        // already installed: only reads (the path of the finding)
        let mut m = Model::new(part);
        m.poke(APP_BASE, &app);
        if part.has_plane1() {
            m.poke(PLANE1_BASE, &bs);
        }
        assert_eq!(m.fws, None, "the model starts with FWS unset");
        let mut buf = [0u8; 512];
        let r = sd_recover_ex(&mut Card::new(card_with(&NAME_APP, &file)), &mut m, part, &mut buf, &NAME_APP);
        assert_eq!(r.map(|r| r.wrote), Ok(false));
        assert_eq!(m.ops, 0);
        assert_eq!(m.fws_violations, 0, "{part:?}: flash was read before FWS was set");
        assert_eq!(m.fws, Some(FWS_RUN));
        // blank flash: reads and writes
        let mut m = Model::new(part);
        let a = sd_recover_attempt(&mut Card::new(card_with(&NAME_APP, &file)), &mut m, part, &mut buf, &NAME_APP);
        assert_eq!(a.result.map(|r| r.wrote), Ok(true));
        assert!(a.wrote);
        assert_eq!(m.fws_violations, 0);
        // a card that cannot be read: FWS is set all the same (the caller reads flash next)
        let mut m = Model::new(part);
        let mut card = Card::new(card_with(&NAME_APP, &file));
        card.fail_at = Some(0);
        let a = sd_recover_attempt(&mut card, &mut m, part, &mut buf, &NAME_APP);
        assert!(a.result.is_err() && !a.wrote);
        assert_eq!(m.fws, Some(FWS_RUN));
    }
}

/// B3: a stamped bitstream's header CRC is the same for every bitstream (the
/// CRC residue), so "already installed" and the check after writing also
/// compare the last word of plane 1 with the file's.
#[test]
fn a_different_stamped_bitstream_of_the_same_length_is_not_already_installed() {
    let part = Part::Sam3u4e;
    let stamped = |seed: u32| {
        let mut b = bitstream(4096, seed);
        let c = bootcore::crc::crc32(&b[..4092]);
        put32(&mut b, 4092, c);
        b
    };
    let (old_bs, new_bs) = (stamped(1), stamped(2));
    // both have the same payload CRC: the header alone cannot tell them apart
    assert_eq!(bootcore::crc::crc32(&old_bs), bootcore::crc::crc32(&new_bs));
    assert_eq!(bootcore::crc::crc32(&old_bs), 0x2144_DF1C);
    let app = app_image(part, 4096, 1);
    let mut file = image(0, part, &app);
    file.extend_from_slice(&image(1, part, &new_bs));
    let mut b = board(part, false, Some(&app), Some(&file));
    b.flash.poke(PLANE1_BASE, &old_bs);
    b.button = true;
    let Boot::Jump(r) = b.boot() else { panic!() };
    assert_eq!((r.result, r.wrote), (RESULT_RECOVERED, true), "the old bitstream was taken for the file's");
    assert_eq!(b.flash.peek(PLANE1_BASE, 4096), &new_bs[..]);
    // and now it is installed: the button costs no write
    let ops = b.flash.log.len();
    b.reset();
    let Boot::Jump(r) = b.boot() else { panic!() };
    assert_eq!((r.result, r.wrote), (RESULT_RECOVERED, false));
    assert_eq!(b.flash.ops, 0);
    assert!(ops > 0);

    // plane 1 self-consistent but wrong in its last word only after writing
    // cannot happen through EWP read-back; the final check is the same
    // function, exercised here by a plane 1 whose body matches and whose
    // stamp does not
    let mut wrong = new_bs.clone();
    put32(&mut wrong, 4092, 0x1234_5678);
    let mut b = board(part, false, Some(&app), Some(&file));
    b.flash.poke(PLANE1_BASE, &wrong);
    b.button = true;
    let Boot::Jump(r) = b.boot() else { panic!() };
    assert!(r.wrote);
    assert_eq!(b.flash.peek(PLANE1_BASE, 4096), &new_bs[..]);
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
            if name == "timeout" {
                // B5: an EWP timeout is a reset from inside the RAM routine,
                // not a return; see `an_ewp_timeout_is_a_reset_...` below
                assert_eq!(r, Boot::PowerLost, "{name}");
                assert_eq!(b.flash.timeout_resets, 1);
                b.reset();
                let r = b.boot();
                assert!(matches!(r, Boot::Jump(_)), "{name}: {r:?}");
            } else {
                assert!(matches!(r, Boot::Jump(_) | Boot::Waiting(_)), "{name}: {r:?}");
            }
            assert_eq!(b.flash.fws, Some(FWS_RUN), "{name}: FWS at the end");
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
    // no hand-off yet: the request is "in progress", not standing and not forgotten
    assert_eq!(b.gpbr0, GPBR_RECOVER_BUSY);
    // a card arrives while the resident polls: it recovers, reports REQ, clears GPBR0
    let app = app_image(part, 2048, 4);
    b.set_card(&image(0, part, &app));
    let Boot::Jump(r) = b.poll() else { panic!() };
    assert!(r.trigger.request && !r.trigger.button);
    assert_eq!((r.result, r.wrote), (RESULT_RECOVERED, true));
    assert!(installed_is(&b, &app));
    assert_eq!(b.gpbr0, 0);

    // the same, but a reset comes while it waits: the next boot still has no
    // application, recovers, and still reports REQ
    let mut b = board(part, false, None, None);
    b.gpbr0 = GPBR_RECOVER_MAGIC;
    assert_eq!(b.boot(), Boot::Waiting(RESULT_NO_CARD));
    b.reset();
    assert_eq!(b.boot(), Boot::Waiting(RESULT_NO_CARD));
    assert_eq!(b.gpbr0, GPBR_RECOVER_BUSY);
    b.set_card(&image(0, part, &app));
    b.reset();
    let Boot::Jump(r) = b.boot() else { panic!() };
    assert!(r.trigger.request);
    assert_eq!((r.result, r.wrote), (RESULT_RECOVERED, true));
    assert_eq!(b.gpbr0, 0);
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
        assert_ne!(b.gpbr0, GPBR_RECOVER_MAGIC, "read {n}");
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
            // keeps polling the card, and a healthy card finishes the job
            Boot::Waiting(code) => {
                assert_eq!(b.gpbr0, GPBR_RECOVER_BUSY, "read {n}: in progress until the hand-off");
                assert!((RESULT_NO_FILE..=RESULT_VERIFY).contains(&code), "read {n}: {code}");
                assert!(!app_valid(&mut b));
                b.card_fail_at = None;
                let Boot::Jump(r) = b.poll() else { panic!("read {n}") };
                assert!(r.trigger.request && r.result == RESULT_RECOVERED);
                assert!(installed_is(&b, &new), "read {n}");
            }
            other => panic!("read {n}: {other:?}"),
        }
        assert_eq!(b.gpbr0, 0, "read {n}: cleared at the hand-off");
        // and the application, which still wants a recovery, does not ask again
        assert!(!b.app_asks_for_recovery(), "read {n}");
    }
}

/// B6: a reset at every EWP of a requested recovery (a power cut with the
/// backup domain alive, a watchdog reset, an EWP timeout). GPBR0 holds "in
/// progress" across it, so the next boot knows: it does not start another
/// recovery on that ground, reports REQ, and the application does not ask
/// again. The board gets to a valid application without looping.
#[test]
fn reset_during_a_requested_recovery_converges_and_is_not_requested_again() {
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
    let (mut skipped, mut redone) = (0, 0);
    for op in 0..total {
        for outcome in outcomes {
            for timeout in [false, true] {
                let mut b = board(part, false, Some(&old), Some(&file));
                b.gpbr0 = GPBR_RECOVER_MAGIC;
                let cut = Some(Cut { op, outcome });
                if timeout {
                    b.flash.timeout = cut;
                } else {
                    b.flash.cut = cut;
                }
                assert_eq!(b.boot(), Boot::PowerLost, "op {op} {outcome:?}");
                assert_eq!(b.gpbr0, GPBR_RECOVER_BUSY, "in progress before the first write (op {op})");
                b.reset();
                let attempts = b.attempts;
                let Boot::Jump(r) = b.boot() else { panic!("op {op} {outcome:?}") };
                assert!(r.trigger.request, "op {op}: the interrupted request is reported");
                assert_eq!(b.gpbr0, 0, "op {op}");
                assert!(app_valid(&mut b), "op {op} {outcome:?}");
                if r.result == RESULT_INTERRUPTED {
                    // the application was still (or already) valid: no second recovery, the card was not read
                    assert_eq!(b.attempts, attempts, "op {op}");
                    assert!(!r.wrote);
                    assert_eq!(b.flash.ops, 0);
                    assert!(installed_is(&b, &old) || installed_is(&b, &new), "op {op} {outcome:?}");
                    skipped += 1;
                } else {
                    // no valid application: there is nothing else to do but recover
                    assert_eq!((r.result, r.wrote), (RESULT_RECOVERED, true), "op {op} {outcome:?}");
                    assert!(installed_is(&b, &new), "op {op} {outcome:?}");
                    redone += 1;
                }
                // the application wants a recovery (that is why it asked): it may not ask again now
                assert!(!b.app_asks_for_recovery(), "op {op} {outcome:?}");
                // an ordinary reset later is an ordinary boot
                b.reset();
                let Boot::Jump(r) = b.boot() else { panic!() };
                assert_eq!(r.result, RESULT_NONE);
                assert!(!r.trigger.request);
                assert_eq!(b.flash.fws_violations, 0, "op {op} {outcome:?}");
                runs += 1;
            }
        }
    }
    assert_eq!(runs, total * outcomes.len() * 2);
    assert!(skipped > 0 && redone > 0, "{skipped} {redone}");
}

/// B6, the loop of the finding: the application asks, the recovery hangs
/// before it writes anything (say in the card's initialisation), the watchdog
/// resets after 16 s, the application starts and would ask again. It must not.
#[test]
fn a_recovery_that_hangs_is_requested_once_not_for_ever() {
    for part in PARTS {
        let app = app_image(part, 4096, 1);
        let mut b = board(part, false, Some(&app), None);
        assert!(matches!(b.boot(), Boot::Jump(_)));
        for round in 0..3 {
            // the application asks (allowed after an ordinary boot)
            assert!(b.app_asks_for_recovery(), "round {round}");
            assert_eq!(b.gpbr0, GPBR_RECOVER_MAGIC);
            // the resident starts, marks the request "in progress", and hangs: the harness
            // plays the part up to the hang, then the watchdog resets
            let action = decide(true, false, b.gpbr0);
            assert_eq!(action, Action::Recover { request: Request::New });
            b.gpbr0 = Request::New.gpbr0_before_recovery().unwrap();
            b.reset();
            // the boot after the watchdog reset
            let Boot::Jump(r) = b.boot() else { panic!() };
            assert_eq!(r.result, RESULT_INTERRUPTED);
            assert!(r.trigger.request && !r.wrote);
            assert_eq!(b.gpbr0, 0);
            assert_eq!(b.attempts, 0, "the card is not touched again");
            // the application still wants its recovery, and is refused: the loop ends here
            assert!(!b.app_asks_for_recovery(), "round {round}");
            // only a later, ordinary reset (or power cycle) lets it ask once more
            b.reset();
            let Boot::Jump(r) = b.boot() else { panic!() };
            assert_eq!(r.result, RESULT_NONE);
        }
        assert_eq!(b.flash.ops, 0);
    }
}

/// B6: "in progress" plus a held button still recovers (the user asks), and
/// REQ is reported; GPBR0 is cleared at the hand-off.
#[test]
fn interrupted_request_with_the_button_held_recovers_and_reports_req() {
    let part = Part::Sam3u2e;
    let old = app_image(part, 2048, 1);
    let new = app_image(part, 2048, 2);
    let mut b = board(part, false, Some(&old), Some(&image(0, part, &new)));
    b.gpbr0 = GPBR_RECOVER_BUSY;
    b.button = true;
    let Boot::Jump(r) = b.boot() else { panic!() };
    assert!(r.trigger.request && r.trigger.button);
    assert_eq!((r.result, r.wrote), (RESULT_RECOVERED, true));
    assert_eq!(b.gpbr0, 0);
    assert!(installed_is(&b, &new));
}

// ------------------------------------------------------------------ B5: an EWP timeout is a reset

#[test]
fn an_ewp_timeout_is_a_reset_and_the_next_boot_goes_on() {
    for part in PARTS {
        let old = app_image(part, 2048, 1);
        let new = app_image(part, 4096, 2);
        let file = image(0, part, &new);
        let mut probe = board(part, false, Some(&old), Some(&file));
        probe.button = true;
        assert!(matches!(probe.boot(), Boot::Jump(_)));
        let total = probe.flash.ops;
        for op in 0..total {
            let mut b = board(part, false, Some(&old), Some(&file));
            b.button = true;
            b.flash.timeout = Some(Cut { op, outcome: Outcome::Garbage });
            // the EWP does not come back: nothing after it runs, in particular no flash read
            assert_eq!(b.boot(), Boot::PowerLost, "op {op}");
            assert_eq!(b.flash.timeout_resets, 1);
            assert_eq!(b.flash.ops, op + 1, "op {op}: no command after the timeout");
            b.reset();
            let Boot::Jump(r) = b.boot() else { panic!("op {op}") };
            assert_eq!(r.result, RESULT_RECOVERED, "op {op}");
            assert!(installed_is(&b, &new), "op {op}");
            assert_eq!(b.flash.fws_violations, 0);
            assert_eq!(b.flash.violations, 0);
        }
    }
}

// ------------------------------------------------------------------ B8: bounded writing attempts

/// No valid application, and a card that passes validation and then fails
/// while the body is written: every attempt erases the head and rewrites pages.
/// One boot makes at most MAX_WRITING_ATTEMPTS such attempts.
#[test]
fn writing_attempts_are_bounded_per_boot() {
    for part in PARTS {
        let new = app_image(part, 16 * 1024, 2);
        let file = image(0, part, &new);
        // find a read index inside pass 2 (after the head was erased)
        let mut probe = board(part, false, None, Some(&file));
        let fail_at = (0..2000)
            .find(|&n| {
                let mut b = board(part, false, None, Some(&file));
                b.card_fail_at = Some(n);
                b.boot() == Boot::Waiting(RESULT_READ) && b.flash.ops > 10
            })
            .expect("a read that fails while the body is written");
        assert!(matches!(probe.boot(), Boot::Jump(_)));

        let mut b = board(part, false, None, Some(&file));
        b.card_fail_at = Some(fail_at);
        let mut last = b.boot();
        let per_attempt = b.flash.ops;
        let mut polls = 0;
        while let Boot::Waiting(code) = last {
            assert_eq!(code, RESULT_READ);
            last = b.poll();
            polls += 1;
            assert!(polls < 20, "the resident never gives up");
        }
        assert_eq!(last, Boot::Halted(RESULT_READ));
        assert_eq!(b.write_attempts, MAX_WRITING_ATTEMPTS);
        assert_eq!(b.attempts, MAX_WRITING_ATTEMPTS as u32);
        assert_eq!(b.flash.ops, per_attempt * MAX_WRITING_ATTEMPTS as usize, "no page beyond the budget");
        assert!(!app_valid(&mut b));
        assert_eq!(b.flash.fws_violations, 0);

        // a reset (or power cycle) starts a new count, and a good card finishes the job
        b.reset();
        b.card_fail_at = None;
        let Boot::Jump(r) = b.boot() else { panic!() };
        assert_eq!((r.result, r.wrote), (RESULT_RECOVERED, true));
        assert!(installed_is(&b, &new));
    }
}

/// Attempts that write nothing are free: no card, no file, a bad file can be
/// polled for as long as it takes, and a good card then still recovers.
#[test]
fn attempts_that_write_nothing_are_not_counted() {
    let part = Part::Sam3u2e;
    let new = app_image(part, 4096, 2);
    let mut bad = image(0, part, &new);
    bad[800] ^= 1;
    let mut b = board(part, false, None, None);
    assert_eq!(b.boot(), Boot::Waiting(RESULT_NO_CARD));
    for _ in 0..50 {
        assert_eq!(b.poll(), Boot::Waiting(RESULT_NO_CARD));
    }
    b.set_card(&bad);
    for _ in 0..50 {
        assert_eq!(b.poll(), Boot::Waiting(RESULT_CRC));
    }
    assert_eq!(b.write_attempts, 0);
    assert_eq!(b.flash.ops, 0);
    b.set_card(&image(0, part, &new));
    let Boot::Jump(r) = b.poll() else { panic!() };
    assert_eq!((r.result, r.wrote), (RESULT_RECOVERED, true));
    assert_eq!(b.write_attempts, 1);
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
