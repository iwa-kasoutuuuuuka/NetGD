//! The long-name panic in embedded-sdmmc 0.9.0 (mcu/netgd/vendor/README.md) and the
//! vendored fix, exercised the way the firmware's `gdi::walk_to` calls the
//! crate: `iterate_dir_lfn` with an `LfnBuffer` over a 4 * MAX_NAME_CHARS
//! (384-byte) store, the name taken through `gamecfg::fit::<96>` as
//! `gdi::entry_name` does.
//!
//! * the two crashing names: the patched crate lists them (with U+FFFD for
//!   the lone surrogate), the crates.io release panics;
//! * everything else -- every bootcore fixture and the built scenario card --
//!   lists identically with and without the patch.

mod common;

use common::*;
use embedded_sdmmc_upstream as up;

const MAX_NAME_CHARS: usize = 96; // gamecfg::MAX_NEXT, gdi::MAX_NAME_CHARS

/// One listed entry: raw 8.3 base/ext and the name `entry_name` would use.
type Row = (Vec<u8>, Vec<u8>, String);

fn entry_name(base: &[u8], ext: &[u8], long: Option<&str>) -> String {
    match long {
        Some(l) if !l.is_empty() => gamecfg::fit::<MAX_NAME_CHARS>(l.as_bytes()).as_str().to_string(),
        _ => {
            let mut s: String = base.iter().map(|&b| b as char).collect();
            if !ext.is_empty() {
                s.push('.');
                s.extend(ext.iter().map(|&b| b as char));
            }
            s
        }
    }
}

/// The root listing through the patched (vendored) crate.
fn list_patched(card: &Card) -> Vec<Row> {
    use embedded_sdmmc as sd;
    struct D<'a>(&'a Card);
    impl sd::BlockDevice for D<'_> {
        type Error = ();
        fn read(&self, blocks: &mut [sd::Block], start: sd::BlockIdx) -> Result<(), ()> {
            for (i, b) in blocks.iter_mut().enumerate() {
                b.contents = self.0.get(start.0 + i as u32);
            }
            Ok(())
        }
        fn write(&self, _: &[sd::Block], _: sd::BlockIdx) -> Result<(), ()> {
            Err(())
        }
        fn num_blocks(&self) -> Result<sd::BlockCount, ()> {
            Ok(sd::BlockCount(self.0.total))
        }
    }
    struct C;
    impl sd::TimeSource for C {
        fn get_timestamp(&self) -> sd::Timestamp {
            sd::Timestamp { year_since_1970: 0, zero_indexed_month: 0, zero_indexed_day: 0, hours: 0, minutes: 0, seconds: 0 }
        }
    }
    let vm = sd::VolumeManager::new(D(card), C);
    let vol = vm.open_volume(sd::VolumeIdx(0)).unwrap();
    let root = vol.open_root_dir().unwrap();
    let mut store = [0u8; 4 * MAX_NAME_CHARS];
    let mut lfn = sd::LfnBuffer::new(&mut store);
    let mut out = vec![];
    root.iterate_dir_lfn(&mut lfn, |e, long| {
        out.push((e.name.base_name().to_vec(), e.name.extension().to_vec(), entry_name(e.name.base_name(), e.name.extension(), long)));
    })
    .unwrap();
    out
}

/// The same through the crates.io release.
fn list_upstream(card: &Card) -> Vec<Row> {
    struct D<'a>(&'a Card);
    impl up::BlockDevice for D<'_> {
        type Error = ();
        fn read(&self, blocks: &mut [up::Block], start: up::BlockIdx) -> Result<(), ()> {
            for (i, b) in blocks.iter_mut().enumerate() {
                b.contents = self.0.get(start.0 + i as u32);
            }
            Ok(())
        }
        fn write(&self, _: &[up::Block], _: up::BlockIdx) -> Result<(), ()> {
            Err(())
        }
        fn num_blocks(&self) -> Result<up::BlockCount, ()> {
            Ok(up::BlockCount(self.0.total))
        }
    }
    struct C;
    impl up::TimeSource for C {
        fn get_timestamp(&self) -> up::Timestamp {
            up::Timestamp { year_since_1970: 0, zero_indexed_month: 0, zero_indexed_day: 0, hours: 0, minutes: 0, seconds: 0 }
        }
    }
    let vm = up::VolumeManager::new(D(card), C);
    let vol = vm.open_volume(up::VolumeIdx(0)).unwrap();
    let root = vol.open_root_dir().unwrap();
    let mut store = [0u8; 4 * MAX_NAME_CHARS];
    let mut lfn = up::LfnBuffer::new(&mut store);
    let mut out = vec![];
    root.iterate_dir_lfn(&mut lfn, |e, long| {
        out.push((e.name.base_name().to_vec(), e.name.extension().to_vec(), entry_name(e.name.base_name(), e.name.extension(), long)));
    })
    .unwrap();
    out
}

const HI: u16 = 0xD83D;
const LO: u16 = 0xDE00;

fn a(n: usize) -> Vec<u16> {
    (0..n).map(|i| b'a' as u16 + (i % 26) as u16).collect()
}

/// A card whose root holds one file with the long name `units`.
fn card_with(units: Vec<u16>) -> Card {
    build(FAT32, vec![Item::File { long: Some(units), short: sfn("CRASH~1", "BIN"), nt: 0, data: vec![1, 2, 3] }])
}

/// Case 6 of fatmap's `surrogates_match_sdmmc`: a lone low surrogate opens
/// the second piece, after a full first piece.
#[test]
fn lone_surrogate_after_full_piece() {
    let card = card_with([a(13), vec![LO], a(3)].concat());
    let l = list_patched(&card);
    assert_eq!(l.len(), 1);
    assert_eq!(l[0].2, "abcdefghijklm\u{FFFD}abc");
    assert!(std::panic::catch_unwind(|| list_upstream(&card)).is_err(), "crates.io 0.9.0 no longer panics?");
}

/// Case 8: a lone high surrogate alone in the third piece, a lone low at the
/// end of the full second one.
#[test]
fn lone_high_alone_in_last_piece() {
    let card = card_with([a(25), vec![LO, HI]].concat());
    let l = list_patched(&card);
    assert_eq!(l[0].2, "abcdefghijklmnopqrstuvwxy\u{FFFD}\u{FFFD}");
    assert!(std::panic::catch_unwind(|| list_upstream(&card)).is_err());
}

/// The crashing names next to ordinary files and folders: the walk goes on
/// past them and finds what follows, as `walk_to` needs.
#[test]
fn walk_continues_past_the_bad_names() {
    let items = vec![
        lfile("Before.txt", sfn("BEFORE", "TXT"), vec![1]),
        Item::File { long: Some([a(13), vec![LO], a(3)].concat()), short: sfn("CRASH~1", "BIN"), nt: 0, data: vec![] },
        Item::File { long: Some([a(25), vec![LO, HI]].concat()), short: sfn("CRASH~2", "BIN"), nt: 0, data: vec![] },
        ldir("Game Folder 01", sfn("GAMEFO~1", ""), vec![file(sfn("DISC", "GDI"), vec![b'1'])]),
    ];
    let l = list_patched(&build(FAT32, items));
    let names: Vec<&str> = l.iter().map(|r| r.2.as_str()).collect();
    assert_eq!(names, ["Before.txt", "abcdefghijklm\u{FFFD}abc", "abcdefghijklmnopqrstuvwxy\u{FFFD}\u{FFFD}", "Game Folder 01"]);
}

/// Every other surrogate placement, and the full scenario card: identical
/// with and without the patch (the patch only adds room for a 14th char).
#[test]
fn patch_changes_nothing_else() {
    let cases: Vec<Vec<u16>> = vec![
        [a(12), vec![HI, LO], a(3)].concat(),
        [a(11), vec![HI, LO], a(3)].concat(),
        [a(13), vec![HI, LO]].concat(),
        [a(5), vec![LO], a(3)].concat(),
        [a(5), vec![HI], a(3)].concat(),
        [a(12), vec![HI], a(3)].concat(),
        [a(5), vec![HI, HI, LO, LO], a(20)].concat(),
        vec![HI, LO, HI, LO],
        [vec![HI], a(4)].concat(),
        [vec![LO], a(4)].concat(),
    ];
    for (i, u) in cases.into_iter().enumerate() {
        let c = card_with(u);
        assert_eq!(list_patched(&c), list_upstream(&c), "case {i}");
    }
    let c = build(FAT32, scenario());
    assert_eq!(list_patched(&c), list_upstream(&c));
}

/// The real-tool fixtures (mcu/bootcore/tests/fixtures) that embedded-sdmmc
/// opens: identical listings with and without the patch.
#[test]
fn fixtures_list_alike() {
    let mut n = 0;
    for f in fixtures() {
        if ["mbr_entry1", "sector4096"].contains(&f.as_str()) {
            continue;
        }
        let c = load_fixture_for_sdmmc(&f);
        assert_eq!(list_patched(&c), list_upstream(&c), "{f}");
        n += 1;
    }
    assert_eq!(n, 32);
}
