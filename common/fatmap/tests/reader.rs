//! Phase 2 of the `embedded_sdmmc` replacement: `fatmap::{vol, dir, sfn}`
//! against `embedded_sdmmc` 0.9.0 (the oracle, see `tests/oracle.rs`) on
//! every fixture and built image: listings with long names, clusters and
//! sizes, look-ups, whole-file reads in many chunk sizes, reads after
//! seeks, short-name spelling. Where the two differ on purpose, a
//! `differs_*` test says how and why.

mod common;

use common::*;
use fatmap::dir::{find, walk, DirEntry, File};
use fatmap::sfn::Sfn;
use fatmap::vol::{Block, BlockRead, Error, Volume};

impl BlockRead for Card {
    fn read_block(&mut self, lba: u32, buf: &mut [u8; 512]) -> Result<(), ()> {
        if lba >= self.total {
            return Err(());
        }
        *buf = self.get(lba);
        Ok(())
    }
}

struct Mounted {
    card: Card,
    vol: Volume,
    blk: Block,
}

fn mount(card: Card) -> Result<Mounted, Error> {
    let mut card = card;
    let mut blk = Block::new();
    let vol = Volume::mount(&mut card, &mut blk)?;
    Ok(Mounted { card, vol, blk })
}

impl Mounted {
    /// The directory cluster at `path` (by exact 8.3 name), 0 = root.
    fn cd(&mut self, path: &[Name]) -> Result<u32, Error> {
        let mut d = 0;
        for n in path {
            let e = find(&self.vol, &mut self.card, &mut self.blk, d, n)?;
            assert!(e.is_dir());
            d = e.first_cluster;
        }
        Ok(d)
    }

    fn list(&mut self, path: &[Name]) -> Result<Vec<Seen>, Error> {
        let d = self.cd(path)?;
        let mut out = vec![];
        let mut lfn = [0u8; 765];
        walk(&self.vol, &mut self.card, &mut self.blk, d, &mut lfn, &mut |e: &DirEntry, long| {
            out.push(Seen { base: cut(&e.name[..8]), ext: cut(&e.name[8..]), attr: e.attr, size: e.size, cluster: e.first_cluster, long: long.map(|l| l.to_vec()) });
            false
        })?;
        Ok(out)
    }

    /// Open `name` (spelt by `Sfn::from_bytes`) in `path` and run the plan.
    fn read(&mut self, path: &[Name], name: &str, plan: &[Op]) -> Result<(u32, Vec<u8>), Error> {
        let d = self.cd(path)?;
        let s = Sfn::from_bytes(name.as_bytes()).ok_or(Error::NotFound)?;
        let e = find(&self.vol, &mut self.card, &mut self.blk, d, &s.0)?;
        let mut f = File::open(&e);
        let mut out = vec![];
        for op in plan {
            match *op {
                Op::Seek(o) => f.seek(o)?,
                Op::Read(n) => {
                    let mut buf = vec![0u8; n];
                    let got = f.read(&self.vol, &mut self.card, &mut self.blk, &mut buf)?;
                    assert!(got == n || f.pos() == f.len(), "short read before the end");
                    out.extend_from_slice(&buf[..got]);
                }
            }
        }
        Ok((f.len(), out))
    }
}

/// `embedded_sdmmc`'s listing without the volume label (which `walk`
/// leaves out, `differs_label`).
fn sd_list_no_label(card: &Card, path: &[Name]) -> Vec<Seen> {
    sd_list(card, path).unwrap().into_iter().filter(|s| s.attr & 0x08 == 0).collect()
}

fn dotted(base: &[u8], ext: &[u8]) -> String {
    let b = String::from_utf8_lossy(base);
    if ext.is_empty() { b.into() } else { format!("{b}.{}", String::from_utf8_lossy(ext)) }
}

/// A deterministic mix of seeks and reads over a file of `len` bytes.
fn seek_plan(len: u32, seed: u32) -> Vec<Op> {
    let mut s = seed.wrapping_mul(2654435761) | 1;
    let mut rnd = move || {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        s
    };
    let mut v = vec![Op::Seek(len), Op::Read(1), Op::Seek(len.saturating_sub(1)), Op::Read(10)];
    for _ in 0..12 {
        v.push(Op::Seek(rnd() % (len + 1)));
        v.push(Op::Read((rnd() % 3000) as usize));
    }
    v.push(Op::Seek(0));
    v.push(Op::Read(600));
    v
}

/// Compare every directory reachable from the root, and every file in
/// them (whole reads in several chunk sizes, then seeks), against the
/// oracle. `sd` is the card as `embedded_sdmmc` can open it; `mine` may be
/// the same card or the superfloppy original.
fn compare_all(sd: &Card, mine: Card, what: &str) -> usize {
    let mut m = mount(mine).unwrap_or_else(|e| panic!("{what}: mount {e:?}"));
    let mut todo: Vec<Vec<Name>> = vec![vec![]];
    let mut files = 0;
    while let Some(path) = todo.pop() {
        let ours = m.list(&path).unwrap();
        let theirs = sd_list_no_label(sd, &path);
        assert_eq!(ours, theirs, "{what}: listing of {path:?}");
        for s in &ours {
            let mut n = [b' '; 11];
            n[..s.base.len()].copy_from_slice(&s.base);
            n[8..8 + s.ext.len()].copy_from_slice(&s.ext);
            if s.attr & 0x10 != 0 {
                if s.base != b"." && s.base != b".." {
                    let mut p = path.clone();
                    p.push(n);
                    todo.push(p);
                }
                continue;
            }
            let name = dotted(&s.base, &s.ext);
            // Names that `create_from_str` cannot spell cannot be opened by
            // name through either API.
            if sd_sfn(&name) != Some(n) {
                continue;
            }
            files += 1;
            let mut plans = vec![chunks(s.size, 4096), chunks(s.size, 513), seek_plan(s.size, s.size ^ s.cluster)];
            if s.size <= 20_000 {
                plans.push(chunks(s.size, 1));
            }
            plans.push(chunks(s.size, 100_000));
            for plan in plans {
                let ours = m.read(&path, &name, &plan);
                let theirs = sd_read(sd, &path, &name, &plan);
                match (ours, theirs) {
                    (Ok(a), Ok(b)) => assert!(a == b, "{what}: {path:?} {name}: contents differ"),
                    (Err(_), Err(_)) => {}
                    (a, b) => panic!("{what}: {path:?} {name}: ours {:?}, theirs {:?}", a.map(|x| x.0), b.map(|x| x.0)),
                }
            }
        }
    }
    files
}

// ------------------------------------------------------------ fixtures

/// Every real-tool fixture `embedded_sdmmc` can open: same listings and
/// bytes. Superfloppies are compared as they are on our side (we mount
/// them) and behind an MBR on the oracle's.
#[test]
fn fixtures_match_sdmmc() {
    let mut total = 0;
    let mut cards = 0;
    for f in fixtures() {
        if ["mbr_entry1", "sector4096", "fat_active1_stale0"].contains(&f.as_str()) {
            continue;
        }
        let raw = load_fixture(&f);
        let sd = load_fixture_for_sdmmc(&f);
        total += compare_all(&sd, raw.clone(), &f);
        if is_superfloppy(&raw) {
            compare_all(&sd, sd.clone(), &format!("{f} behind MBR"));
        }
        cards += 1;
    }
    assert_eq!(cards, 31);
    assert!(total > 200, "{total} files compared");
}

/// The fixtures neither reader opens, and the stale-FAT one both misread
/// the same way (FAT 0 only): ours refuses to read the files too.
#[test]
fn fixtures_refused_alike() {
    assert_eq!(mount(load_fixture("sector4096")).err(), Some(Error::NoVolume));
    assert_eq!(mount(load_fixture("mbr_entry1")).err(), Some(Error::NoVolume));
    let mut m = mount(load_fixture("fat_active1_stale0")).unwrap();
    for n in ["NETGD.OGF", "NETGD.BIT"] {
        assert!(m.read(&[], n, &chunks(10000, 4096)).is_err());
        assert!(sd_read(&load_fixture_for_sdmmc("fat_active1_stale0"), &[], n, &chunks(10000, 4096)).is_err());
    }
}

// ------------------------------------------------------------ built images

#[test]
fn built_fat32_matches_sdmmc() {
    let c = build(FAT32, scenario());
    assert!(compare_all(&c, c.clone(), "fat32") >= 70);
}

#[test]
fn built_fat32_fragmented_matches_sdmmc() {
    let c = build(Fmt { frag: true, clusters: 140_000, ..FAT32 }, scenario());
    compare_all(&c, c.clone(), "fat32 frag");
}

#[test]
fn built_fat32_big_clusters_match_sdmmc() {
    for spc in [2, 8, 64] {
        let c = build(Fmt { spc, clusters: 65_600, frag: spc == 8, ..FAT32 }, scenario());
        compare_all(&c, c.clone(), &format!("fat32 spc {spc}"));
    }
}

#[test]
fn built_superfloppy_mounts() {
    let c = build(Fmt { mbr: None, ..FAT32 }, scenario());
    let sd = behind_mbr(&c);
    compare_all(&sd, c, "superfloppy");
}

#[cfg(feature = "fat16")]
#[test]
fn built_fat16_matches_sdmmc() {
    for fmt in [FAT16, Fmt { frag: true, ..FAT16 }, Fmt { spc: 4, clusters: 20_000, ..FAT16 }] {
        let c = build(fmt, scenario());
        compare_all(&c, c.clone(), "fat16");
    }
}

#[cfg(not(feature = "fat16"))]
#[test]
fn fat16_refused_without_feature() {
    assert_eq!(mount(build(FAT16, scenario())).err(), Some(Error::NoVolume));
}

/// A FAT32 volume below 65,525 clusters: `embedded_sdmmc` takes it for
/// FAT16 and cannot list it; we read it as the FAT32 it is (as the firmware's
/// `fat.rs` does already).
#[test]
fn differs_small_fat32() {
    let c = build(Fmt { clusters: 20_000, ..FAT32 }, scenario());
    assert!(sd_list(&c, &[]).map_or(true, |l| l.iter().all(|s| s.long.is_none())));
    let mut m = mount(c).unwrap();
    let l = m.list(&[]).unwrap();
    assert!(l.iter().any(|s| s.long.as_deref() == Some(&b"Track 01.bin"[..])));
    let (_, d) = m.read(&[], "TRACK0~1.BIN", &chunks(70000, 4096)).unwrap();
    assert_eq!(d, lcg(2, 70000));
}

// ------------------------------------------------------------ differences

/// 248..=255-unit names (20 pieces): we assemble them, `embedded_sdmmc`
/// drops the long name. 255 units is a legal FAT long name.
#[test]
fn differs_20_piece_name() {
    let c = build(FAT32, vec![lfile(&long_name(255), sfn("ABCDEF~1", "BIN"), lcg(1, 10)), lfile(&long_name(248), sfn("ABCDEF~2", "BIN"), lcg(1, 10))]);
    let mut m = mount(c.clone()).unwrap();
    let l = m.list(&[]).unwrap();
    assert_eq!(l[0].long.as_deref(), Some(long_name(255).as_bytes()));
    assert_eq!(l[1].long.as_deref(), Some(long_name(248).as_bytes()));
    assert!(sd_list(&c, &[]).unwrap().iter().all(|s| s.long.is_none()));
}

/// A deleted entry between the pieces and the 8.3 entry orphans the run
/// for us (and for Windows and `LfnMatcher`); `embedded_sdmmc` skips
/// deleted entries before its LFN state machine and keeps the name.
#[test]
fn differs_deleted_entry_inside_run() {
    let s = sfn("TRACK0~1", "BIN");
    let mut raw = lfn_pieces(&utf16("Track 01.bin"), checksum(&s));
    let mut del = short_entry(&sfn("GONE", "TXT"), 0x20, 0, 0, 0);
    del[0] = 0xE5;
    raw.insert(1, del);
    raw.push(short_entry(&s, 0x20, 0, 0, 0));
    let c = build(FAT32, vec![Item::Raw(raw)]);
    let mut m = mount(c).unwrap();
    assert_eq!(m.list(&[]).unwrap()[0].long, None);
}

/// The volume label is not listed by us; `embedded_sdmmc` lists it as an
/// entry with attribute 0x08. Every firmware caller filters it out anyway
/// (they want directories or a `.GDI`/`.CDI`/`.INI` extension).
#[test]
fn differs_label() {
    let c = build(FAT32, vec![Item::Raw(vec![label_entry()]), file(sfn("A", "TXT"), vec![1])]);
    let mut m = mount(c.clone()).unwrap();
    assert_eq!(m.list(&[]).unwrap().len(), 1);
    assert_eq!(sd_list(&c, &[]).unwrap().len(), 2);
    assert_eq!(m.read(&[], "NETGD", &[]).err(), Some(Error::NotFound));
}

/// A long name that does not fit the caller's buffer: we report none (the
/// caller falls back to the 8.3 name); `embedded_sdmmc` reports `""`, which
/// the firmware's `entry_name` treats the same way.
#[test]
fn differs_overflow() {
    let c = build(FAT32, vec![lfile("Track 01.bin", sfn("TRACK0~1", "BIN"), vec![1])]);
    let mut m = mount(c).unwrap();
    let mut small = [0u8; 11];
    let mut got = vec![];
    walk(&m.vol, &mut m.card, &mut m.blk, 0, &mut small, &mut |_, l| {
        got.push(l.map(|x| x.to_vec()));
        false
    })
    .unwrap();
    assert_eq!(got, [None]);
    let mut exact = [0u8; 12];
    let mut got = vec![];
    walk(&m.vol, &mut m.card, &mut m.blk, 0, &mut exact, &mut |_, l| {
        got.push(l.map(|x| x.to_vec()));
        false
    })
    .unwrap();
    assert_eq!(got, [Some(b"Track 01.bin".to_vec())]);
}

/// Malformed pieces (type byte or cluster field not zero) break the run
/// for us; `embedded_sdmmc` does not check them.
#[test]
fn differs_malformed_piece() {
    let s = sfn("TRACK0~1", "BIN");
    let mut raw = lfn_pieces(&utf16("Track 01.bin"), checksum(&s));
    raw[0][26] = 7;
    raw.push(short_entry(&s, 0x20, 0, 0, 0));
    let c = build(FAT32, vec![Item::Raw(raw)]);
    assert_eq!(mount(c).unwrap().list(&[]).unwrap()[0].long, None);
}

// ------------------------------------------------------------ long names

/// Surrogates in every position, compared with the oracle. The one case
/// that differs: a lone *high* surrogate as the very first unit, which
/// `embedded_sdmmc` drops and we turn into U+FFFD. Two cases make
/// `embedded_sdmmc` panic (see the end).
#[test]
fn surrogates_match_sdmmc() {
    let (hi, lo) = (0xD83Du16, 0xDE00u16);
    let a = |n: usize| -> Vec<u16> { (0..n).map(|i| b'a' as u16 + i as u16 % 26).collect() };
    let cases: Vec<Vec<u16>> = vec![
        [a(12), vec![hi, lo], a(3)].concat(),
        [a(11), vec![hi, lo], a(3)].concat(),
        [a(13), vec![hi, lo]].concat(),
        [a(5), vec![lo], a(3)].concat(),
        [a(5), vec![hi], a(3)].concat(),
        [a(12), vec![hi], a(3)].concat(),
        [a(13), vec![lo], a(3)].concat(),
        [a(5), vec![hi, hi, lo, lo], a(20)].concat(),
        [a(25), vec![lo, hi]].concat(),
        vec![hi, lo, hi, lo],
    ];
    let mut panics = vec![];
    for (i, u) in cases.iter().enumerate() {
        let c = build(FAT32, vec![Item::File { long: Some(u.clone()), short: sfn(&format!("SUR{i}"), "BIN"), nt: 0, data: vec![i as u8] }]);
        let ours = mount(c.clone()).unwrap().list(&[]).unwrap();
        match std::panic::catch_unwind(|| sd_list(&c, &[]).unwrap()) {
            Ok(theirs) => assert_eq!(ours, theirs, "case {i}"),
            Err(_) => panics.push(i),
        }
    }
    // `embedded_sdmmc` panics ("Vec was full?!") when a lone surrogate opens
    // a piece that follows a full one: its 13-char buffer gets 14 chars.
    assert_eq!(panics, [6, 8]);
    // A lone surrogate as the name's first unit.
    let first = |u: u16| {
        let c = build(FAT32, vec![Item::File { long: Some([vec![u], a(4)].concat()), short: sfn("X", ""), nt: 0, data: vec![] }]);
        (mount(c.clone()).unwrap().list(&[]).unwrap()[0].long.clone().unwrap(), sd_list(&c, &[]).unwrap()[0].long.clone().unwrap())
    };
    assert_eq!(first(lo), (b"abcd".to_vec(), b"abcd".to_vec()));
    // The one difference: a lone high surrogate first, which
    // `embedded_sdmmc` drops and we turn into U+FFFD like any other.
    assert_eq!(first(hi), ("\u{FFFD}abcd".as_bytes().to_vec(), b"abcd".to_vec()));
}

/// Long names as `LfnMatcher` finds them: every name the walk assembles is
/// one the matcher recognises at the same entry (the two must not disagree
/// about which runs are valid).
#[test]
fn walk_agrees_with_lfn_matcher() {
    use fatmap::lfn::{Feed, LfnMatcher};
    for f in ["card_mksdcard_4k", "names_lfn_decoys"] {
        let mut m = mount(load_fixture(f)).unwrap();
        for s in m.list(&[]).unwrap() {
            let Some(long) = s.long else { continue };
            let name = String::from_utf8(long).unwrap();
            let mut lm = LfnMatcher::<255>::from_str(&name).unwrap();
            // Feed the root directory raw and see where the matcher hits.
            let mut hit = None;
            let mut c = m.vol.root();
            'outer: loop {
                let lba = m.vol.cluster_lba(c).unwrap();
                for b in 0..m.vol.blocks_per_cluster() {
                    let blk = m.card.get(lba + b);
                    for e in blk.chunks(32) {
                        match lm.feed(e) {
                            Feed::End => break 'outer,
                            Feed::Entry { long: true } => {
                                hit = Some(e[..11].to_vec());
                                break 'outer;
                            }
                            _ => {}
                        }
                    }
                }
                match m.vol.next_cluster(&mut m.card, &mut m.blk, c).unwrap() {
                    Some(n) => c = n,
                    None => break,
                }
            }
            let mut want = s.base.clone();
            want.resize(8, b' ');
            want.extend(&s.ext);
            want.resize(11, b' ');
            assert_eq!(hit, Some(want), "{f}: {name}");
        }
    }
}

// ------------------------------------------------------------ 8.3 names

/// `Sfn::from_bytes` spells every name like `create_from_str`, over a
/// generated set of names (all characters of interest in every position,
/// dots, lengths around 8 and 3, Latin-1, beyond Latin-1, invalid UTF-8).
#[test]
fn sfn_matches_create_from_str() {
    let alphabet: Vec<&str> = vec!["a", "Z", "0", "~", ".", " ", "+", "[", "é", "日", "\u{7f}", "\u{1}", "-", "_", "!", "ÿ"];
    let mut names: Vec<String> = vec![String::new(), ".".into(), "..".into(), "...".into()];
    for a in &alphabet {
        for b in &alphabet {
            for c in &alphabet {
                names.push(format!("{a}{b}{c}"));
                names.push(format!("{a}bcdefg{b}.{c}x"));
                names.push(format!("abcdefgh{a}{b}{c}"));
                names.push(format!("ab{a}.t{b}{c}"));
            }
        }
    }
    for base in 0..=10 {
        for ext in 0..=4 {
            names.push(format!("{}.{}", "B".repeat(base), "e".repeat(ext)));
            names.push(format!("{}..{}", "B".repeat(base), "e".repeat(ext)));
        }
    }
    let mut n = 0;
    for name in &names {
        let ours = Sfn::from_bytes(name.as_bytes()).map(|s| s.0);
        let theirs = if name.is_empty() || name == "." {
            Some(*b".          ")
        } else if name == ".." {
            Some(*b"..         ")
        } else {
            sd_sfn(name)
        };
        assert_eq!(ours, theirs, "{name:?}");
        n += 1;
    }
    assert!(n > 16_000);
    // Invalid UTF-8 (create_from_str only takes a str): refused.
    assert_eq!(Sfn::from_bytes(b"A\xFF.TXT"), None);
    assert_eq!(Sfn::from_bytes(b"\xC3"), None);
    let s = Sfn::from_bytes(b"gdemu.ini").unwrap();
    assert_eq!((s.base(), s.ext()), (&b"GDEMU"[..], &b"INI"[..]));
}

// ------------------------------------------------------------ look-ups, files

#[test]
fn find_is_exact_and_misses_cleanly() {
    let c = build(FAT32, scenario());
    let mut m = mount(c.clone()).unwrap();
    for name in ["NOPE.TXT", "GDEMU.IN", "TRACK0~1.BI", "DELETE~1.BIN"] {
        assert_eq!(m.read(&[], name, &[]).err(), Some(Error::NotFound), "{name}");
        assert!(sd_read(&c, &[], name, &[]).is_err());
    }
    // In a subdirectory, and a 3-level path.
    let (len, d) = m.read(&[sfn("01", "")], "GAME.CFG", &chunks(40, 7)).unwrap();
    assert_eq!((len, d), (40, lcg(15, 40)));
    let p = [sfn("LONGFO~1", ""), sfn("SUB", ""), sfn("THIRDL~1", "")];
    let (_, d) = m.read(&p, "DEEPFI~1.CDI", &chunks(9000, 1000)).unwrap();
    assert_eq!(d, lcg(16, 9000));
    assert_eq!(m.cd(&[sfn("NOPE", "")]).err(), Some(Error::NotFound));
    // `..` of a top-level folder is cluster 0, the root.
    let d01 = m.cd(&[sfn("01", "")]).unwrap();
    let up = find(&m.vol, &mut m.card, &mut m.blk, d01, &sfn("..", "")).unwrap();
    assert_eq!(up.first_cluster, 0);
    let root = m.list(&[]).unwrap();
    let mut via_dotdot = vec![];
    walk(&m.vol, &mut m.card, &mut m.blk, up.first_cluster, &mut [0u8; 765], &mut |e, l| {
        via_dotdot.push((e.name, l.map(|x| x.to_vec())));
        false
    })
    .unwrap();
    assert_eq!(via_dotdot.len(), root.len());
}

#[test]
fn seek_rules_match_sdmmc() {
    let c = build(FAT32, scenario());
    let mut m = mount(c.clone()).unwrap();
    for plan in [vec![Op::Seek(70000), Op::Read(10)], vec![Op::Seek(70001)], vec![Op::Read(10), Op::Seek(3), Op::Read(1)]] {
        let ours = m.read(&[], "TRACK0~1.BIN", &plan);
        let theirs = sd_read(&c, &[], "TRACK0~1.BIN", &plan);
        assert_eq!(ours.is_ok(), theirs.is_ok(), "{plan:?}");
        if let (Ok(a), Ok(b)) = (ours, theirs) {
            assert_eq!(a, b);
        }
    }
    // Empty file: length 0, reads nothing, no cluster needed.
    assert_eq!(m.read(&[], "EMPTY.BIN", &[Op::Read(10)]).unwrap(), (0, vec![]));
}

/// The block buffer is reused: a run of small reads inside one block reads
/// the card once.
#[test]
fn small_reads_hit_the_block_buffer() {
    struct Count<'a>(&'a mut Card, u32);
    impl BlockRead for Count<'_> {
        fn read_block(&mut self, lba: u32, buf: &mut [u8; 512]) -> Result<(), ()> {
            self.1 += 1;
            self.0.read_block(lba, buf)
        }
    }
    let mut c = build(FAT32, scenario());
    let mut blk = Block::new();
    let vol = Volume::mount(&mut c, &mut blk).unwrap();
    let e = find(&vol, &mut c, &mut blk, 0, &sfn("PLAIN", "TXT")).unwrap();
    let mut f = File::open(&e);
    let mut dev = Count(&mut c, 0);
    let mut b = [0u8; 4];
    for _ in 0..128 {
        f.read(&vol, &mut dev, &mut blk, &mut b).unwrap();
    }
    assert_eq!(dev.1, 1);
}

// ------------------------------------------------------------ damage

/// Damaged chains end in an error, never a hang or a panic: a directory
/// whose chain loops, a file whose chain loops or ends early, a cluster
/// outside the volume, a read error.
#[test]
fn damage_is_an_error() {
    let fmt = Fmt { spc: 1, ..FAT32 };
    let c = build(fmt, scenario());
    let mut m = mount(c.clone()).unwrap();
    let many = m.cd(&[sfn("MANY", "")]).unwrap();
    let fat = |m: &Mounted, cl: u32| -> u64 {
        let s = m.card.get(0x800);
        let reserved = u16::from_le_bytes([s[14], s[15]]) as u64;
        (0x800 + reserved) * 512 + cl as u64 * 4
    };
    // Directory loop: the 3rd cluster of MANY points back at the first.
    let mut cl = many;
    for _ in 0..2 {
        cl = m.vol.next_cluster(&mut m.card, &mut m.blk, cl).unwrap().unwrap();
    }
    let off = fat(&m, cl);
    m.card.write(off, &many.to_le_bytes());
    m.blk.invalidate();
    assert_eq!(m.list(&[sfn("MANY", "")]).err(), Some(Error::BadChain));
    // File chain that loops on itself: the read stops at the size.
    let mut m = mount(c.clone()).unwrap();
    let e = find(&m.vol, &mut m.card, &mut m.blk, 0, &sfn("TRACK0~1", "BIN")).unwrap();
    let off = fat(&m, e.first_cluster);
    m.card.write(off, &e.first_cluster.to_le_bytes());
    m.blk.invalidate();
    let (_, d) = m.read(&[], "TRACK0~1.BIN", &chunks(70000, 4096)).unwrap();
    assert_eq!(d.len(), 70000);
    // Chain ends early, free entry, out of range: errors.
    for v in [0x0FFF_FFFFu32, 0, 1, 0x0FFF_FFF7, 0x0FFF_FFF0] {
        let mut m = mount(c.clone()).unwrap();
        let off = fat(&m, e.first_cluster);
        m.card.write(off, &v.to_le_bytes());
        assert_eq!(m.read(&[], "TRACK0~1.BIN", &chunks(70000, 4096)).err(), Some(Error::BadChain), "{v:x}");
    }
    // A directory entry pointing outside the volume.
    let mut bad = c.clone();
    let m = mount(bad.clone()).unwrap();
    let root = m.vol.cluster_lba(m.vol.root()).unwrap();
    for b in 0..8 {
        let mut s = bad.get(root + b);
        for k in 0..16 {
            if &s[k * 32..k * 32 + 11] == b"PLAIN   TXT" {
                s[k * 32 + 0x14..k * 32 + 0x16].copy_from_slice(&0x7FFFu16.to_le_bytes());
            }
        }
        bad.sectors.insert(root + b, s);
    }
    let mut m = mount(bad).unwrap();
    assert_eq!(m.read(&[], "PLAIN.TXT", &[Op::Read(10)]).err(), Some(Error::BadChain));
    // Read errors: a card that ends where GDEMU.INI starts.
    let mut short = c.clone();
    let e = find(&m.vol, &mut m.card, &mut m.blk, 0, &sfn("GDEMU", "INI")).unwrap();
    short.total = m.vol.cluster_lba(e.first_cluster).unwrap();
    let mut m = mount(short).unwrap();
    assert_eq!(m.read(&[], "GDEMU.INI", &[Op::Read(10)]).err(), Some(Error::Io));
}

/// Random damage to the metadata: every walk and read finishes (with data
/// or an error), none panics.
#[test]
fn random_damage_terminates() {
    let clean = build(Fmt { spc: 1, frag: true, clusters: 140_000, ..FAT32 }, scenario());
    let meta: Vec<u32> = {
        let mut v: Vec<u32> = clean.sectors.keys().copied().filter(|&l| l < 0x800 + 32 + 2 * 1100 + 400).collect();
        v.sort();
        v
    };
    let mut s = 0x1234_5678u32;
    let mut rnd = move || {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        s
    };
    for _ in 0..300 {
        let mut c = clean.clone();
        for _ in 0..8 {
            let lba = meta[rnd() as usize % meta.len()];
            let i = rnd() as usize % 512;
            c.sector_mut(lba)[i] = rnd() as u8;
        }
        let Ok(mut m) = mount(c) else { continue };
        let mut todo = vec![0u32];
        let mut seen = 0;
        while let Some(d) = todo.pop() {
            seen += 1;
            if seen > 50 {
                break;
            }
            let mut found = vec![];
            let _ = walk(&m.vol, &mut m.card, &mut m.blk, d, &mut [0u8; 765], &mut |e, _| {
                found.push(*e);
                false
            });
            for e in found {
                if e.is_dir() && e.name[0] != b'.' {
                    todo.push(e.first_cluster);
                } else if !e.is_dir() {
                    let mut f = File::open(&e);
                    let mut b = vec![0u8; 8192];
                    for _ in 0..4 {
                        if f.read(&m.vol, &mut m.card, &mut m.blk, &mut b).unwrap_or(0) == 0 {
                            break;
                        }
                    }
                }
            }
        }
    }
}
