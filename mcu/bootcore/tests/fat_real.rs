//! The FAT32 reader against cards formatted and filled by real tools.
//!
//! `tests/common/mod.rs` builds its own FAT32 images, so a misreading of the
//! format that the builder shares with the reader would pass `tests/fat.rs`.
//! The fixtures in `tests/fixtures/` were made by dosfstools `mkfs.fat`,
//! util-linux `sfdisk` (the MBR) and mtools `mcopy`/`mdel`/`mmd` (the files),
//! by `tests/fixtures/gen_fat_real.py` (it lists what each one is). They are
//! stored sparsely: `NAME.sparse` holds only the non-zero sectors, and file
//! data that is exactly a block of an `lcg(seed, size)` file is stored as a
//! (seed, size, block) record instead of bytes; `NAME.txt` says what the
//! reader must do with it.
//!
//! `NAME.txt` keys: `desc`, `mount` (`ok`, an error name, or `REFUSE` = any
//! error), `file=NAME11:seed:size:expect` (expect: `ok` = mount + find + a
//! full read byte-identical to `lcg(seed, size)`, or the error that the
//! reader gives today), `ideal=ok` (what a correct reader would do where
//! today's behaviour is a refusal of something a real card can have: a known
//! gap, see `former_gaps_*`), `note`.
//!
//! Not covered: a Windows-formatted card (no Windows formatter was usable
//! without administrator rights when this was written).

mod common;

use bootcore::fat::*;
use common::lcg;
use std::collections::HashMap;
use std::path::PathBuf;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures")
}

fn u32at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// A card whose sectors are the stored ones; every other sector reads as
/// zeros, up to the card's size (32 GB-class cards are a few dozen sectors).
struct Sparse {
    total: u32,
    sectors: HashMap<u32, [u8; 512]>,
}

impl Sparse {
    fn load(name: &str) -> Sparse {
        let b = std::fs::read(dir().join(format!("{name}.sparse"))).unwrap_or_else(|e| panic!("{name}.sparse: {e}"));
        assert_eq!(&b[..4], b"SPRS", "{name}: magic");
        let total = u32at(&b, 4);
        let mut sectors = HashMap::new();
        let mut p = 8;
        while p < b.len() {
            match u32at(&b, p) {
                0 => {
                    let (lba, n) = (u32at(&b, p + 4), u32at(&b, p + 8) as usize);
                    p += 12;
                    for i in 0..n {
                        let mut s = [0u8; 512];
                        s.copy_from_slice(&b[p + i * 512..p + (i + 1) * 512]);
                        sectors.insert(lba + i as u32, s);
                    }
                    p += n * 512;
                }
                1 => {
                    let (lba, n, seed, size, k0) =
                        (u32at(&b, p + 4), u32at(&b, p + 8), u32at(&b, p + 12), u32at(&b, p + 16) as usize, u32at(&b, p + 20) as usize);
                    p += 24;
                    let mut data = lcg(seed, size);
                    data.resize(size.div_ceil(512) * 512, 0);
                    for i in 0..n as usize {
                        let mut s = [0u8; 512];
                        s.copy_from_slice(&data[(k0 + i) * 512..(k0 + i + 1) * 512]);
                        sectors.insert(lba + i as u32, s);
                    }
                }
                t => panic!("{name}: record type {t}"),
            }
        }
        assert_eq!(p, b.len(), "{name}: trailing bytes");
        Sparse { total, sectors }
    }
}

impl BlockRead for Sparse {
    fn read_block(&mut self, lba: u32, buf: &mut [u8; 512]) -> Result<(), ()> {
        if lba >= self.total {
            return Err(());
        }
        *buf = self.sectors.get(&lba).copied().unwrap_or([0u8; 512]);
        Ok(())
    }
}

struct Meta {
    desc: String,
    mount: String,
    /// (name11, seed, size, expect)
    files: Vec<([u8; 11], u32, usize, String)>,
    ideal_ok: bool,
}

fn meta(name: &str) -> Meta {
    let t = std::fs::read_to_string(dir().join(format!("{name}.txt"))).unwrap_or_else(|e| panic!("{name}.txt: {e}"));
    let mut m = Meta { desc: String::new(), mount: "ok".into(), files: vec![], ideal_ok: false };
    for l in t.lines() {
        let (k, v) = l.split_once('=').unwrap();
        match k {
            "desc" => m.desc = v.into(),
            "mount" => m.mount = v.into(),
            "file" => {
                let f: Vec<&str> = v.split(':').collect();
                let mut n = [0u8; 11];
                n.copy_from_slice(f[0].as_bytes());
                m.files.push((n, f[1].parse().unwrap(), f[2].parse().unwrap(), f[3].into()));
            }
            "ideal" => m.ideal_ok = v == "ok",
            "note" => {}
            _ => panic!("{name}: unknown key {k}"),
        }
    }
    m
}

fn read_all(card: &mut Sparse, vol: &Volume, ent: &DirEnt) -> Result<Vec<u8>, FatError> {
    let mut r = FileReader::new(ent);
    let mut buf = [0u8; 512];
    let mut out = vec![];
    loop {
        let n = r.read_next(vol, card, &mut buf)?;
        if n == 0 {
            return Ok(out);
        }
        out.extend_from_slice(&buf[..n]);
    }
}

/// Mount + find + full read; `Ok(bytes)` or the error name.
fn fetch(card: &mut Sparse, vol: &Volume, name: &[u8; 11]) -> Result<Vec<u8>, String> {
    let mut buf = [0u8; 512];
    let ent = vol.find(card, &mut buf, name).map_err(|e| format!("{e:?}"))?;
    let data = read_all(card, vol, &ent).map_err(|e| format!("{e:?}"))?;
    assert_eq!(data.len(), ent.size as usize, "short read without an error");
    Ok(data)
}

/// Check one fixture; `ideal` checks what a correct reader would do instead
/// of what this one does today. Returns the mismatches.
fn check(name: &str, ideal: bool) -> Vec<String> {
    let m = meta(name);
    let mut card = Sparse::load(name);
    let mut buf = [0u8; 512];
    let mut bad = vec![];
    let mount_expect = if ideal { "ok" } else { m.mount.as_str() };
    let vol = match Volume::mount(&mut card, &mut buf) {
        Ok(v) => {
            if mount_expect != "ok" && mount_expect != "REFUSE" {
                bad.push(format!("{name}: mounted, expected {mount_expect}"));
            }
            Some(v)
        }
        Err(e) => {
            if mount_expect != "REFUSE" && mount_expect != format!("{e:?}") {
                bad.push(format!("{name}: mount gave {e:?}, expected {mount_expect}"));
            }
            None
        }
    };
    if mount_expect == "REFUSE" {
        // refused at mount, or at the latest at the lookup; never the wrong data
        if let Some(v) = vol {
            if fetch(&mut card, &v, b"NETGD   OGF").is_ok() {
                bad.push(format!("{name}: a 4096-byte-sector volume was read"));
            }
        }
        return bad;
    }
    for (n, seed, size, expect) in &m.files {
        let expect = if ideal { "ok" } else { expect.as_str() };
        let got = match &vol {
            Some(v) => fetch(&mut card, v, n),
            None => continue,
        };
        let label = String::from_utf8_lossy(n).to_string();
        match (got, expect) {
            (Ok(d), "ok") => {
                if d != lcg(*seed, *size) {
                    bad.push(format!("{name}: {label} read {} bytes, content differs", d.len()));
                }
            }
            (Ok(d), e) => bad.push(format!("{name}: {label} read {} bytes, expected the error {e}", d.len())),
            (Err(g), e) => {
                if g != e {
                    bad.push(format!("{name}: {label} gave {g}, expected {e}"));
                }
            }
        }
    }
    bad
}

fn all_names() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir())
        .unwrap()
        .filter_map(|e| {
            let n = e.unwrap().file_name().into_string().unwrap();
            n.strip_suffix(".txt").map(|s| s.to_string())
        })
        .collect();
    v.sort();
    v
}

macro_rules! fixtures {
    ($($name:ident),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                let bad = check(stringify!($name), false);
                assert!(bad.is_empty(), "{}: {:#?}", meta(stringify!($name)).desc, bad);
            }
        )*
        const LISTED: &[&str] = &[$(stringify!($name)),*];
    };
}

fixtures!(
    sf_spc1,
    sf_spc2,
    sf_spc4,
    sf_spc8,
    sf_spc16,
    sf_spc32,
    sf_spc64,
    mbr_c_2048,
    mbr_b_2048,
    mbr_c_63,
    mbr_b_12345,
    mbr_c_8192,
    mbr_entry1,
    sd32g_sda_32k,
    sd32g_16k_far,
    sd32gib_2048_far,
    fat_nomirror_active0,
    fat_active1_identical,
    fat_active1_stale0,
    fat_single,
    fat_res2,
    fat_top_f,
    fat_top_a,
    fat_top_5,
    frag_spc1,
    frag_spc2,
    frag_spc8,
    names_lfn_decoys,
    names_lowercase,
    subdir_only,
    subdir_and_root,
    card_mksdcard_4k,
    card_mksdcard_512,
    sector4096,
);

/// A new fixture must be listed above (and so run), and every listed one
/// must exist.
#[test]
fn every_fixture_is_listed() {
    let mut have = all_names();
    let mut listed: Vec<String> = LISTED.iter().map(|s| s.to_string()).collect();
    have.sort();
    listed.sort();
    assert_eq!(have, listed);
    let total: u64 = std::fs::read_dir(dir())
        .unwrap()
        .map(|e| e.unwrap().metadata().unwrap().len())
        .sum();
    assert!(total < 260_000, "fixtures grew to {total} bytes; keep them small");
}

/// The fixtures really have the shapes they claim (so a regeneration that
/// silently loses them is noticed).
#[test]
fn fixtures_have_their_shapes() {
    // 32 GB-class cards: more than 60 million sectors, the volume far from LBA 0
    for n in ["sd32g_sda_32k", "sd32g_16k_far", "sd32gib_2048_far"] {
        let c = Sparse::load(n);
        assert!(c.total > 60_000_000, "{n}: {} sectors", c.total);
    }
    // MBR types and starts
    for (n, ty, start) in [("mbr_c_2048", 0x0C, 2048), ("mbr_b_2048", 0x0B, 2048), ("mbr_c_63", 0x0C, 63), ("mbr_b_12345", 0x0B, 12345), ("mbr_c_8192", 0x0C, 8192)] {
        let c = Sparse::load(n);
        let mbr = c.sectors[&0];
        assert_eq!(mbr[0x1BE + 4], ty, "{n}");
        assert_eq!(u32at(&mbr, 0x1BE + 8), start, "{n}");
        assert_eq!(&mbr[510..], &[0x55, 0xAA]);
    }
    // superfloppy: LBA 0 is a BPB with the jump
    let c = Sparse::load("sf_spc4");
    assert!(c.sectors[&0][0] == 0xEB || c.sectors[&0][0] == 0xE9);
    // partition in entry 1 only
    let c = Sparse::load("mbr_entry1");
    let mbr = c.sectors[&0];
    assert_eq!(mbr[0x1BE + 4], 0x83);
    assert_eq!(mbr[0x1CE + 4], 0x0C);
    // active-FAT flag
    let c = Sparse::load("fat_active1_stale0");
    assert_eq!(c.sectors[&0][0x28], 0x81);
    // top nibble really set in a FAT entry of the chain
    let c = Sparse::load("fat_top_f");
    let res = u32::from(u16::from_le_bytes([c.sectors[&0][0x0E], c.sectors[&0][0x0F]]));
    let fat0 = &c.sectors[&res];
    assert!((0..128).any(|i| u32at(fat0, i * 4) >> 28 == 0xF && u32at(fat0, i * 4) & 0x0FFF_FFFF != 0));
    // the 4 KiB fixture: bytes/sector in the BPB
    let c = Sparse::load("sector4096");
    assert_eq!(u16::from_le_bytes([c.sectors[&0][0x0B], c.sectors[&0][0x0C]]), 4096);
}

/// Former gaps (cards the reader used to refuse although they are legal
/// FAT32; the fixtures carry `ideal=ok`): they must read correctly now.
#[test]
fn former_gaps_read_correctly() {
    let mut bad = vec![];
    let mut n = 0;
    for name in all_names() {
        if meta(&name).ideal_ok {
            n += 1;
            bad.extend(check(&name, true));
        }
    }
    assert!(n > 0);
    assert!(bad.is_empty(), "{bad:#?}");
}

/// The former gaps are exactly these two (a fixture marked `ideal=ok` whose
/// `expect` is not `ok` would be an open gap: `check(.., false)` pins what
/// the reader does today, `check(.., true)` what it should do).
#[test]
fn former_gaps_are_the_documented_ones() {
    let gaps: Vec<String> = all_names().into_iter().filter(|n| meta(n).ideal_ok).collect();
    assert_eq!(gaps, ["fat_active1_stale0", "mbr_entry1"]);
}

/// A protective-MBR entry 0 is refused even when a FAT32 entry follows; an
/// active-FAT number past the last FAT falls back to FAT 0.
#[test]
fn gpt_in_entry_0_and_bad_active_fat() {
    let mut c = Sparse::load("mbr_entry1");
    c.sectors.get_mut(&0).unwrap()[0x1BE + 4] = 0xEE;
    let mut buf = [0u8; 512];
    assert_eq!(Volume::mount(&mut c, &mut buf), Err(FatError::Gpt));

    let mut c = Sparse::load("fat_active1_identical");
    c.sectors.get_mut(&0).unwrap()[0x28] = 0x85; // active FAT 5 of 2: ignored
    let v = Volume::mount(&mut c, &mut buf).unwrap();
    assert_eq!(fetch(&mut c, &v, b"NETGD   OGF").unwrap(), lcg(7, 6000));
}
