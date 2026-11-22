//! Phase 0 of the `embedded_sdmmc` replacement: pin down what
//! `embedded_sdmmc` 0.9.0 -- the version the firmware links -- makes of real
//! and built FAT images, so that `tests/reader.rs` can hold `fatmap`'s reader
//! to the same answers.
//!
//! Two kinds of images:
//! * the `mcu/bootcore/tests/fixtures` cards (made by mkfs.fat / sfdisk /
//!   mtools, see `gen_fat_real.py`), whose `.txt` says which files they hold;
//! * cards built by `tests/common` from a model (`scenario()`), so the
//!   expected listing is known independently of either reader.
//!
//! Where `embedded_sdmmc` departs from the FAT spec (or from what Windows
//! does) the test says so by name (`sdmmc_quirk_*`); those are the cases where
//! the replacement is allowed to differ.

mod common;

use common::*;

/// Fixture `.txt`: `file=NAME11:seed:size:expect` lines.
fn fixture_files(name: &str) -> Vec<(Name, u32, usize, bool)> {
    let t = std::fs::read_to_string(fixture_dir().join(format!("{name}.txt"))).unwrap();
    t.lines()
        .filter_map(|l| l.strip_prefix("file="))
        .map(|v| {
            let f: Vec<&str> = v.split(':').collect();
            let mut n = [0u8; 11];
            n.copy_from_slice(f[0].as_bytes());
            (n, f[1].parse().unwrap(), f[2].parse().unwrap(), f[3] == "ok")
        })
        .collect()
}

fn dotted(n: &Name) -> String {
    let b = String::from_utf8_lossy(&cut(&n[..8])).to_string();
    let e = String::from_utf8_lossy(&cut(&n[8..])).to_string();
    if e.is_empty() { b } else { format!("{b}.{e}") }
}

/// Fixtures `embedded_sdmmc` refuses as they are, and after a superfloppy
/// is moved behind an MBR (`behind_mbr`).
fn sdmmc_refusals(load: fn(&str) -> Card) -> Vec<String> {
    fixtures().into_iter().filter(|f| sd_list(&load(f), &[]).is_err()).collect()
}

/// `embedded_sdmmc` opens only MBR entry 0 with a FAT type byte: every
/// superfloppy (LBA 0 is the BPB; `fat::probe` and `fatmap::vol` take those)
/// is refused, as are the volume in MBR entry 1 and 4096-byte sectors.
#[test]
fn fixtures_sdmmc_mounts() {
    let raw = sdmmc_refusals(load_fixture);
    let superfloppies: Vec<String> = fixtures().into_iter().filter(|f| is_superfloppy(&load_fixture(f))).collect();
    let mut expect = superfloppies.clone();
    expect.extend(["mbr_entry1".to_string(), "sector4096".to_string()]);
    expect.sort();
    assert_eq!(raw, expect);
    assert_eq!(superfloppies.len(), 23);
    assert_eq!(sdmmc_refusals(load_fixture_for_sdmmc), SDMMC_REFUSES);
}

/// Fixtures `embedded_sdmmc` refuses even behind an MBR.
pub const SDMMC_REFUSES: &[&str] = &["mbr_entry1", "sector4096"];

/// On every fixture it opens, `embedded_sdmmc` reads each listed file back
/// byte for byte (the fixture generator's ground truth), and does not find
/// the ones that are not there -- except on `SDMMC_STALE_FAT`.
#[test]
fn fixtures_sdmmc_reads_listed_files() {
    let mut bad = vec![];
    for f in fixtures() {
        if SDMMC_REFUSES.contains(&f.as_str()) {
            continue;
        }
        let card = load_fixture_for_sdmmc(&f);
        for (n, seed, size, ok) in fixture_files(&f) {
            let got = sd_read(&card, &[], &dotted(&n), &chunks(size as u32, 4096));
            let ok = ok && f != SDMMC_STALE_FAT;
            match got {
                Ok((len, d)) if ok => {
                    if len as usize != size || d != lcg(seed, size) {
                        bad.push(format!("{f}: {} differs", dotted(&n)));
                    }
                }
                Ok(_) => bad.push(format!("{f}: {} read, expected an error", dotted(&n))),
                Err(e) if ok => bad.push(format!("{f}: {}: {e}", dotted(&n))),
                Err(_) => {}
            }
        }
    }
    assert!(bad.is_empty(), "{bad:#?}");
}

/// BPB 0x28 says FAT 1 is the live one and FAT 0 is stale: `embedded_sdmmc`
/// (and the firmware's `fat.rs`) read FAT 0 and hit an unterminated chain.
/// `bootcore` honours 0x28; the replacement keeps reading FAT 0, like the
/// code it replaces.
pub const SDMMC_STALE_FAT: &str = "fat_active1_stale0";

/// The listing a correct reader gives of `items` (in directory order, with
/// `.`/`..` for subdirectories), long names as UTF-8. Clusters are left out
/// (0): they depend on allocation.
fn model(items: &[Item], sub: bool) -> Vec<Seen> {
    let mut v = vec![];
    if sub {
        for d in [".", ".."] {
            v.push(Seen { base: d.as_bytes().to_vec(), ext: vec![], attr: 0x10, size: 0, cluster: 0, long: None });
        }
    }
    for it in items {
        match it {
            Item::File { long, short, data, .. } => v.push(Seen {
                base: cut(&short[..8]),
                ext: cut(&short[8..]),
                attr: 0x20,
                size: data.len() as u32,
                cluster: 0,
                long: long.as_ref().map(|l| String::from_utf16(l).unwrap().into_bytes()),
            }),
            Item::Dir { long, short, .. } => v.push(Seen {
                base: cut(&short[..8]),
                ext: cut(&short[8..]),
                attr: 0x10,
                size: 0,
                cluster: 0,
                long: long.as_ref().map(|l| String::from_utf16(l).unwrap().into_bytes()),
            }),
            Item::Raw(es) => {
                for e in es {
                    if e[0] == 0xE5 || e[11] & 0x0F == 0x0F {
                        continue;
                    }
                    let mut n = [0u8; 11];
                    n.copy_from_slice(&e[..11]);
                    v.push(Seen { base: cut(&n[..8]), ext: cut(&n[8..]), attr: e[11], size: 0, cluster: 0, long: None });
                }
            }
        }
    }
    v
}

fn no_clusters(mut v: Vec<Seen>) -> Vec<Seen> {
    v.iter_mut().for_each(|s| s.cluster = 0);
    v
}

/// Every directory of `items` with its path, depth first.
fn dirs<'a>(items: &'a [Item], path: Vec<Name>, out: &mut Vec<(Vec<Name>, &'a [Item])>) {
    out.push((path.clone(), items));
    for it in items {
        if let Item::Dir { short, items, .. } = it {
            let mut p = path.clone();
            p.push(*short);
            dirs(items, p, out);
        }
    }
}

fn check_model(fmt: Fmt) {
    let items = scenario();
    let card = build(fmt, scenario());
    let mut all = vec![];
    dirs(&items, vec![], &mut all);
    for (path, its) in all {
        let got = no_clusters(sd_list(&card, &path).unwrap());
        assert_eq!(got, model(its, !path.is_empty()), "listing of {path:?}");
        for it in its {
            if let Item::File { short, data, .. } = it {
                let (len, d) = sd_read(&card, &path, &dotted(short), &chunks(data.len() as u32, 1000)).unwrap();
                assert_eq!(len as usize, data.len());
                assert!(d == *data, "{} differs", dotted(short));
            }
        }
    }
}

#[test]
fn built_fat32_matches_model() {
    check_model(FAT32);
}

#[test]
fn built_fat32_fragmented_matches_model() {
    check_model(Fmt { frag: true, clusters: 140_000, ..FAT32 });
}

#[test]
fn built_fat32_8k_clusters_matches_model() {
    check_model(Fmt { spc: 16, clusters: 65_600, ..FAT32 });
}

#[test]
fn built_fat16_matches_model() {
    check_model(FAT16);
}

/// Seeks: `embedded_sdmmc` accepts any offset up to the length (inclusive)
/// and refuses one past it.
#[test]
fn sdmmc_seek_rules() {
    let card = build(FAT32, scenario());
    let data = lcg(2, 70000);
    let (_, d) = sd_read(&card, &[], "TRACK0~1.BIN", &[Op::Seek(69990), Op::Read(100), Op::Seek(5), Op::Read(3)]).unwrap();
    assert_eq!(d, [&data[69990..], &data[5..8]].concat());
    let (_, d) = sd_read(&card, &[], "TRACK0~1.BIN", &[Op::Seek(70000), Op::Read(10)]).unwrap();
    assert!(d.is_empty());
    assert!(sd_read(&card, &[], "TRACK0~1.BIN", &[Op::Seek(70001)]).is_err());
    assert!(sd_read(&card, &[], "NOPE.BIN", &[]).is_err());
}

/// Quirk: a long name of 248..=255 units (20 pieces) is not assembled --
/// `embedded_sdmmc` only takes a first piece with ordinal 2..=19.
#[test]
fn sdmmc_quirk_20_piece_name_dropped() {
    let card = build(FAT32, vec![lfile(&long_name(255), sfn("ABCDEF~1", "BIN"), lcg(1, 10)), lfile(&long_name(247), sfn("ABCDEF~2", "BIN"), lcg(1, 10))]);
    let l = sd_list(&card, &[]).unwrap();
    assert_eq!(l[0].long, None);
    assert_eq!(l[1].long.as_deref(), Some(long_name(247).as_bytes()));
}

/// Quirk: a deleted entry between a long name's pieces and its 8.3 entry
/// does not break the run (deleted entries are skipped before the LFN state
/// machine sees them). Windows / the FAT spec treat the run as orphaned.
#[test]
fn sdmmc_quirk_deleted_entry_inside_run() {
    let s = sfn("TRACK0~1", "BIN");
    let mut raw = lfn_pieces(&utf16("Track 01.bin"), checksum(&s));
    let mut del = short_entry(&sfn("GONE", "TXT"), 0x20, 0, 0, 0);
    del[0] = 0xE5;
    raw.push(del);
    raw.push(short_entry(&s, 0x20, 0, 0, 0));
    let card = build(FAT32, vec![Item::Raw(raw)]);
    assert_eq!(sd_list(&card, &[]).unwrap()[0].long.as_deref(), Some(&b"Track 01.bin"[..]));
}

/// Quirk: the volume label is listed like a file (attribute 0x08).
#[test]
fn sdmmc_quirk_label_listed() {
    let card = build(FAT32, vec![Item::Raw(vec![label_entry()])]);
    let l = sd_list(&card, &[]).unwrap();
    assert_eq!(l.len(), 1);
    assert_eq!(l[0].attr, 0x08);
}

/// The 8.3 case flags (NT byte 0x0C) are ignored: names come back upper case.
#[test]
fn sdmmc_ignores_case_flags() {
    let card = build(FAT32, vec![Item::File { long: None, short: sfn("LOWER", "TXT"), nt: 0x18, data: vec![1] }]);
    assert_eq!(sd_list(&card, &[]).unwrap()[0].base, b"LOWER");
}

/// `ShortFileName::create_from_str` on the names the firmware asks for.
#[test]
fn sdmmc_short_names() {
    assert_eq!(sd_sfn("gdemu.ini"), Some(sfn("GDEMU", "INI")));
    assert_eq!(sd_sfn("01"), Some(sfn("01", "")));
    assert_eq!(sd_sfn("ABCDEFGH.TXT"), Some(sfn("ABCDEFGH", "TXT")));
    assert_eq!(sd_sfn("ABCDEFGHI"), None);
    assert_eq!(sd_sfn("A.TEXT"), None);
    assert_eq!(sd_sfn("Track 01.bin"), None);
    assert_eq!(sd_sfn(".hidden"), None);
    assert_eq!(sd_sfn("a.b.c"), None);
    // Quirk: a second dot right after the first is swallowed.
    assert_eq!(sd_sfn("A..B"), Some(sfn("A", "B")));
    // Latin-1 is stored as the byte, not upper-cased.
    assert_eq!(sd_sfn("é.txt"), Some(*b"\xE9       TXT"));
    assert_eq!(sd_sfn("日本"), None);
}

/// The mksdcard fixture (mtools-written long names, folders 01..07) as
/// `embedded_sdmmc` lists it: the real-tool long names the reader is
/// compared on in `tests/reader.rs`.
#[test]
fn fixture_mksdcard_long_names_seen() {
    for f in ["card_mksdcard_4k", "card_mksdcard_512"] {
        let card = load_fixture_for_sdmmc(f);
        let root = sd_list(&card, &[]).unwrap();
        let longs = root.iter().filter(|s| s.long.is_some()).count();
        let folders = root.iter().filter(|s| s.attr & 0x10 != 0 && s.base.len() == 2).count();
        assert!(longs >= 40, "{f}: {longs} long names");
        assert_eq!(folders, 7, "{f}");
    }
}
