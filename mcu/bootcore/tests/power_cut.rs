//! Exhaustive power-cut sweep of the SD recovery.
//!
//! For every EWP the recovery issues, and for a set of outcomes of that very
//! EWP being cut short (not started, erased, programmed up to byte k for
//! every k, erase stopped at byte k for every k, arbitrary garbage), the
//! flash is taken as it is after the cut and "booted":
//!
//! * `verify_flash_app` (the boot check) must say either "invalid" (so the
//!   resident goes to recovery mode) or "valid", and then the bytes it
//!   accepted must be exactly the old image or exactly the new image, and
//!   the bitstream plane must match whichever image it is: never a jump to a
//!   corrupt application;
//! * stronger, from the design's ordering: after the very first EWP the
//!   application is invalid, until the final one;
//! * the resident's 8 KB never changes and no EWP ever targets it;
//! * and recovery started again from that state (with the card still in)
//!   completes, ending with exactly the new image.

mod common;

use bootcore::app::{verify_flash_app, APP_BASE};
use bootcore::fat::NAME_APP;
use bootcore::flash::PLANE1_BASE;
use bootcore::part::Part;
use bootcore::update::sd_recover;
use common::*;

#[derive(Clone)]
struct Scenario {
    part: Part,
    old_app: Option<Vec<u8>>,
    old_bs: Option<Vec<u8>>,
    new_app: Vec<u8>,
    new_bs: Option<Vec<u8>>,
    card: std::sync::Arc<Vec<u8>>,
}

impl Scenario {
    fn new(part: Part, old_app: Option<Vec<u8>>, old_bs: Option<Vec<u8>>, new_app: Vec<u8>, new_bs: Option<Vec<u8>>, layout: Layout) -> Self {
        let mut file = image(0, part, &new_app);
        if let Some(b) = &new_bs {
            file.extend(image(1, part, b));
        }
        let mut f = Fat::new(8192 * 2, 1);
        f.add_file(b"JUNK    TXT", &lcg(1, 700), Layout::Stride3, true);
        f.add_file(&NAME_APP, &file, layout, true);
        Scenario { part, old_app, old_bs, new_app, new_bs, card: std::sync::Arc::new(with_mbr(&f.img, 0x0C)) }
    }

    fn base(&self) -> Model {
        let mut m = Model::new(self.part);
        // junk everywhere the app or the bitstream may go
        m.poke(APP_BASE, &lcg(77, 0x1E000));
        if self.part.has_plane1() {
            m.poke(PLANE1_BASE, &lcg(78, 0x2_0000));
        }
        if let Some(a) = &self.old_app {
            m.poke(APP_BASE, a);
        }
        if let Some(b) = &self.old_bs {
            m.poke(PLANE1_BASE, b);
        }
        m
    }

    fn recover(&self, m: &mut Model) -> Result<u32, bootcore::update::RecoverError> {
        let mut card = Card::shared(self.card.clone());
        let mut buf = [0u8; 512];
        sd_recover(&mut card, m, self.part, &mut buf, &NAME_APP)
    }

    /// What the boot check makes of this flash. Panics on a corrupt app.
    fn boot(&self, m: &Model) -> Boot {
        let mut m = m.clone();
        m.power_up();
        let part = self.part;
        match verify_flash_app(&mut m, part, false) {
            Err(_) => Boot::Invalid,
            Ok(len) => {
                let img = m.peek(APP_BASE, len as usize);
                if self.old_app.as_deref() == Some(img) {
                    if let Some(b) = &self.old_bs {
                        assert_eq!(m.peek(PLANE1_BASE, b.len()), &b[..], "old app valid with a damaged old bitstream");
                    }
                    Boot::Old
                } else if img == &self.new_app[..] {
                    if let Some(b) = &self.new_bs {
                        assert_eq!(m.peek(PLANE1_BASE, b.len()), &b[..], "new app valid with a damaged new bitstream");
                    }
                    Boot::New
                } else {
                    panic!("boot check accepted a corrupt application ({len} bytes)");
                }
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Boot {
    Old,
    New,
    Invalid,
}

fn outcomes(full: bool) -> Vec<Outcome> {
    let mut v = vec![Outcome::Untouched, Outcome::Erased, Outcome::Garbage];
    if full {
        for k in 1..=255 {
            v.push(Outcome::Prefix(k));
            v.push(Outcome::ErasePrefix(k));
        }
    } else {
        for k in [1, 7, 31, 32, 33, 128, 255] {
            v.push(Outcome::Prefix(k));
            v.push(Outcome::ErasePrefix(k));
        }
    }
    v
}

struct Stats {
    cases: usize,
    old: usize,
    new: usize,
    invalid: usize,
    ops: usize,
}

fn sweep(sc: &Scenario, full: bool) -> Stats {
    let mut st = Stats { cases: 0, old: 0, new: 0, invalid: 0, ops: 0 };
    let base = sc.base();
    let resident = base.peek(0x0008_0000, 0x2000).to_vec();

    // clean run: the number of EWPs and the final state
    let mut clean = base.clone();
    sc.recover(&mut clean).expect("clean run");
    assert_eq!(sc.boot(&clean), Boot::New);
    assert_eq!(clean.violations, 0);
    let n = clean.ops;
    st.ops = n;
    assert!(clean.log.iter().all(|&a| a >= APP_BASE));

    for op in 0..n {
        for o in outcomes(full) {
            let mut m = base.clone();
            m.cut = Some(Cut { op, outcome: o });
            let r = sc.recover(&mut m);
            assert!(r.is_err(), "op {op} {o:?}: the cut must abort the recovery");
            assert!(m.dead);
            assert_eq!(m.violations, 0);
            assert_eq!(m.peek(0x0008_0000, 0x2000), &resident[..], "resident modified");
            assert!(m.log.iter().all(|&a| a >= APP_BASE));

            let b = sc.boot(&m);
            match b {
                Boot::Old => {
                    assert_eq!(op, 0, "old app still valid after the second EWP ({o:?})");
                    st.old += 1;
                }
                Boot::New => {
                    assert_eq!(op, n - 1, "new app valid before its last EWP ({o:?})");
                    st.new += 1;
                }
                Boot::Invalid => st.invalid += 1,
            }
            if op >= 1 && op < n - 1 {
                assert_eq!(b, Boot::Invalid, "op {op} {o:?}");
            }

            // restart: the same card, the resulting flash
            let mut m2 = m.clone();
            m2.power_up();
            sc.recover(&mut m2).unwrap_or_else(|e| panic!("restart after op {op} {o:?}: {e:?}"));
            assert_eq!(sc.boot(&m2), Boot::New, "restart after op {op} {o:?}");
            assert_eq!(m2.violations, 0);
            st.cases += 1;
        }
    }
    st
}

#[test]
fn sweep_2e_blank_flash_every_byte() {
    let part = Part::Sam3u2e;
    let sc = Scenario::new(part, None, None, app_image(part, 8192, 21), None, Layout::Stride3);
    let s = sweep(&sc, true);
    println!("2E blank, 8 KB: {} EWPs, {} cases (old {}, new {}, invalid {})", s.ops, s.cases, s.old, s.new, s.invalid);
    assert_eq!(s.ops, 8192 / 256 + 2);
    assert!(s.cases >= 34 * 500);
    assert_eq!(s.old, 0);
}

#[test]
fn sweep_2e_over_a_valid_old_app_every_byte() {
    let part = Part::Sam3u2e;
    // new longer, same length, shorter than the old one
    for (old_len, new_len) in [(2048usize, 8192usize), (8192, 8192), (12288, 4096), (512, 512)] {
        let sc = Scenario::new(
            part,
            Some(app_image(part, old_len, 5)),
            None,
            app_image(part, new_len, 6),
            None,
            Layout::Reverse,
        );
        let s = sweep(&sc, true);
        println!("2E old {old_len} -> new {new_len}: {} EWPs, {} cases (old {}, new {}, invalid {})", s.ops, s.cases, s.old, s.new, s.invalid);
        assert!(s.old > 0, "the old app is valid for a cut before anything changed");
    }
}

#[test]
fn sweep_2e_over_an_invalid_old_app() {
    let part = Part::Sam3u2e;
    // an old image that has a good descriptor and vectors but a bad CRC
    // (what a stalled older updater leaves)
    let mut bad = app_image(part, 8192, 5);
    bad[5000] ^= 0xFF;
    let sc = Scenario::new(part, Some(bad), None, app_image(part, 8192, 8), None, Layout::Contig);
    assert_eq!(sc.boot(&sc.base()), Boot::Invalid);
    let s = sweep(&sc, false);
    println!("2E invalid old app: {} EWPs, {} cases", s.ops, s.cases);
    assert!(s.cases > 100);
    assert_eq!(s.old, 0);
}

#[test]
fn sweep_4e_app_and_bitstream() {
    let part = Part::Sam3u4e;
    let sc = Scenario::new(
        part,
        Some(app_image(part, 4096, 1)),
        Some(bitstream(0x3000, 1)),
        app_image(part, 4096, 2),
        Some(bitstream(0x4000, 2)),
        Layout::Stride3,
    );
    let s = sweep(&sc, true);
    println!("4E app 4 KB + bitstream 16 KB: {} EWPs, {} cases (old {}, new {}, invalid {})", s.ops, s.cases, s.old, s.new, s.invalid);
    assert_eq!(s.ops, 4096 / 256 + 0x4000 / 256 + 2);
    // a bitstream that is longer than before, then shorter
    for (obl, nbl) in [(0x1000usize, 0x6000usize), (0x8000, 0x1000)] {
        let sc = Scenario::new(
            part,
            Some(app_image(part, 8192, 3)),
            Some(bitstream(obl, 3)),
            app_image(part, 2048, 4),
            Some(bitstream(nbl, 4)),
            Layout::Contig,
        );
        let s = sweep(&sc, false);
        println!("4E bitstream {obl:#x} -> {nbl:#x}: {} EWPs, {} cases", s.ops, s.cases);
    }
}

#[test]
fn sweep_2e_largest_app() {
    let part = Part::Sam3u2e;
    let sc = Scenario::new(
        part,
        Some(app_image(part, 0x1E000, 5)),
        None,
        app_image(part, 0x1E000, 6),
        None,
        Layout::Stride3,
    );
    let s = sweep(&sc, false);
    println!("2E 120 KB -> 120 KB: {} EWPs, {} cases (old {}, new {}, invalid {})", s.ops, s.cases, s.old, s.new, s.invalid);
    assert_eq!(s.ops, 0x1E000 / 256 + 2);
}

/// A second power cut during the recovery that follows the first one, at
/// every EWP of the second run, for a sample of first cuts.
#[test]
fn nested_cuts() {
    let part = Part::Sam3u4e;
    let sc = Scenario::new(
        part,
        Some(app_image(part, 2048, 1)),
        Some(bitstream(0x1000, 1)),
        app_image(part, 3072, 2),
        Some(bitstream(0x1800, 2)),
        Layout::Stride3,
    );
    let base = sc.base();
    let mut clean = base.clone();
    sc.recover(&mut clean).unwrap();
    let n = clean.ops;
    let mut cases = 0;
    let firsts = [Outcome::Untouched, Outcome::Erased, Outcome::Prefix(100), Outcome::ErasePrefix(40), Outcome::Garbage];
    for op1 in (0..n).step_by(3).chain([n - 1]) {
        for o1 in firsts {
            let mut m = base.clone();
            m.cut = Some(Cut { op: op1, outcome: o1 });
            assert!(sc.recover(&mut m).is_err());
            for op2 in 0..n {
                for o2 in [Outcome::Untouched, Outcome::Garbage, Outcome::Prefix(200), Outcome::Erased] {
                    let mut m2 = m.clone();
                    m2.power_up();
                    m2.cut = Some(Cut { op: op2, outcome: o2 });
                    let r = sc.recover(&mut m2);
                    assert!(r.is_err());
                    let _ = sc.boot(&m2); // panics on a corrupt app
                    // and the third run finishes
                    let mut m3 = m2.clone();
                    m3.power_up();
                    sc.recover(&mut m3).unwrap();
                    assert_eq!(sc.boot(&m3), Boot::New);
                    cases += 1;
                }
            }
        }
    }
    println!("nested cuts: {cases} cases");
    assert!(cases > 1000);
}

/// The harness must be able to fail: a flash that holds a third, perfectly
/// valid image is "a corrupt application" as far as the sweep is concerned.
#[test]
#[should_panic(expected = "corrupt application")]
fn harness_flags_a_valid_but_foreign_image() {
    let part = Part::Sam3u2e;
    let sc = Scenario::new(part, Some(app_image(part, 4096, 1)), None, app_image(part, 4096, 2), None, Layout::Contig);
    let mut m = sc.base();
    m.poke(APP_BASE, &app_image(part, 4096, 3));
    sc.boot(&m);
}
