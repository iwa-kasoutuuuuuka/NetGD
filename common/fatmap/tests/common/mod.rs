//! Shared by the reader tests: sparse cards (the `mcu/bootcore` fixtures and
//! images built here), a small FAT image builder, and `embedded_sdmmc` 0.9 --
//! the crate the firmware uses today -- wrapped as an oracle.
//!
//! The builder is deliberately plain (sequential allocation, optional
//! fragmentation, literal directory entries for the odd cases) so that a
//! misreading it shared with the reader would show up as a disagreement with
//! `embedded_sdmmc` or with the real-tool fixtures.
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::PathBuf;

use embedded_sdmmc as sd;

// ---------------------------------------------------------------- cards

/// A card: the stored sectors; every other sector up to `total` reads zero.
#[derive(Clone)]
pub struct Card {
    pub total: u32,
    pub sectors: HashMap<u32, [u8; 512]>,
}

impl Card {
    pub fn new(total: u32) -> Card {
        Card { total, sectors: HashMap::new() }
    }

    pub fn get(&self, lba: u32) -> [u8; 512] {
        self.sectors.get(&lba).copied().unwrap_or([0u8; 512])
    }

    pub fn sector_mut(&mut self, lba: u32) -> &mut [u8; 512] {
        assert!(lba < self.total, "write past the card: {lba}");
        self.sectors.entry(lba).or_insert([0u8; 512])
    }

    pub fn write(&mut self, byte_off: u64, data: &[u8]) {
        for (i, &b) in data.iter().enumerate() {
            let o = byte_off + i as u64;
            self.sector_mut((o / 512) as u32)[(o % 512) as usize] = b;
        }
    }
}

pub fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mcu/bootcore/tests/fixtures")
}

/// Fixture names (without extension), sorted.
pub fn fixtures() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(fixture_dir())
        .expect("mcu/bootcore/tests/fixtures")
        .filter_map(|e| {
            let n = e.ok()?.file_name().into_string().ok()?;
            n.strip_suffix(".sparse").map(str::to_owned)
        })
        .collect();
    v.sort();
    v
}

/// The byte stream the fixture generator fills files with
/// (`mcu/bootcore/tests/common/mod.rs`, `gen_fat_real.py`).
pub fn lcg(seed: u32, n: usize) -> Vec<u8> {
    let mut s = seed.wrapping_mul(2654435761).wrapping_add(12345);
    (0..n)
        .map(|_| {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            (s >> 24) as u8
        })
        .collect()
}

fn u32at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// Load `NAME.sparse` (format: `mcu/bootcore/tests/fat_real.rs`).
pub fn load_fixture(name: &str) -> Card {
    let b = std::fs::read(fixture_dir().join(format!("{name}.sparse"))).unwrap();
    assert_eq!(&b[..4], b"SPRS");
    let mut card = Card::new(u32at(&b, 4));
    let mut p = 8;
    while p < b.len() {
        match u32at(&b, p) {
            0 => {
                let (lba, n) = (u32at(&b, p + 4), u32at(&b, p + 8) as usize);
                p += 12;
                for i in 0..n {
                    let mut s = [0u8; 512];
                    s.copy_from_slice(&b[p + i * 512..p + (i + 1) * 512]);
                    card.sectors.insert(lba + i as u32, s);
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
                    card.sectors.insert(lba + i as u32, s);
                }
            }
            t => panic!("{name}: record type {t}"),
        }
    }
    card
}

/// The same volume moved behind an MBR (entry 0 at LBA 2048, type 0x0C).
/// `embedded_sdmmc` only opens partitioned cards; most fixtures are
/// superfloppies, so this is how the oracle gets to see their contents.
pub fn behind_mbr(card: &Card) -> Card {
    const AT: u32 = 2048;
    let mut out = Card::new(card.total + AT);
    for (&lba, s) in &card.sectors {
        out.sectors.insert(lba + AT, *s);
    }
    let m = out.sector_mut(0);
    m[0x1BE + 4] = 0x0C;
    m[0x1BE + 8..0x1BE + 12].copy_from_slice(&AT.to_le_bytes());
    m[0x1BE + 12..0x1BE + 16].copy_from_slice(&card.total.to_le_bytes());
    m[510] = 0x55;
    m[511] = 0xAA;
    out
}

/// Whether LBA 0 is a BPB rather than an MBR (the test as `fatmap::vol`
/// and the firmware's `fat::probe` make it).
pub fn is_superfloppy(card: &Card) -> bool {
    let s = card.get(0);
    (s[0] == 0xEB || s[0] == 0xE9) && u16::from_le_bytes([s[11], s[12]]) == 512 && s[13] != 0
}

/// The fixture as the oracle can open it: superfloppies moved behind an MBR.
pub fn load_fixture_for_sdmmc(name: &str) -> Card {
    let c = load_fixture(name);
    if is_superfloppy(&c) { behind_mbr(&c) } else { c }
}

// ---------------------------------------------------------------- builder

/// 8.3 name from `"BASE"`, `"EXT"` (bytes as given, space padded).
pub fn sfn(base: &str, ext: &str) -> [u8; 11] {
    let mut a = [b' '; 11];
    a[..base.len()].copy_from_slice(base.as_bytes());
    a[8..8 + ext.len()].copy_from_slice(ext.as_bytes());
    a
}

pub fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

pub fn checksum(name: &[u8; 11]) -> u8 {
    name.iter().fold(0u8, |s, &b| s.rotate_right(1).wrapping_add(b))
}

const OFFS: [usize; 13] = [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];

/// Long-name pieces for `units`, on-disk order (last piece first).
pub fn lfn_pieces(units: &[u16], csum: u8) -> Vec<[u8; 32]> {
    let n = units.len().div_ceil(13);
    let mut out = Vec::new();
    for seq in (1..=n).rev() {
        let mut e = [0u8; 32];
        e[0] = seq as u8 | if seq == n { 0x40 } else { 0 };
        e[11] = 0x0F;
        e[13] = csum;
        for (k, &off) in OFFS.iter().enumerate() {
            let idx = (seq - 1) * 13 + k;
            let c = match idx.cmp(&units.len()) {
                std::cmp::Ordering::Less => units[idx],
                std::cmp::Ordering::Equal => 0,
                std::cmp::Ordering::Greater => 0xFFFF,
            };
            e[off..off + 2].copy_from_slice(&c.to_le_bytes());
        }
        out.push(e);
    }
    out
}

pub fn short_entry(name: &[u8; 11], attr: u8, nt: u8, cluster: u32, size: u32) -> [u8; 32] {
    let mut e = [0u8; 32];
    e[..11].copy_from_slice(name);
    e[11] = attr;
    e[12] = nt;
    e[20..22].copy_from_slice(&((cluster >> 16) as u16).to_le_bytes());
    e[26..28].copy_from_slice(&(cluster as u16).to_le_bytes());
    e[28..32].copy_from_slice(&size.to_le_bytes());
    e
}

pub enum Item {
    File { long: Option<Vec<u16>>, short: [u8; 11], nt: u8, data: Vec<u8> },
    Dir { long: Option<Vec<u16>>, short: [u8; 11], items: Vec<Item> },
    /// Literal 32-byte entries (deleted entries, orphans, labels ...).
    Raw(Vec<[u8; 32]>),
}

pub fn file(short: [u8; 11], data: Vec<u8>) -> Item {
    Item::File { long: None, short, nt: 0, data }
}

pub fn lfile(long: &str, short: [u8; 11], data: Vec<u8>) -> Item {
    Item::File { long: Some(utf16(long)), short, nt: 0, data }
}

pub fn dir(short: [u8; 11], items: Vec<Item>) -> Item {
    Item::Dir { long: None, short, items }
}

pub fn ldir(long: &str, short: [u8; 11], items: Vec<Item>) -> Item {
    Item::Dir { long: Some(utf16(long)), short, items }
}

#[derive(Clone, Copy)]
pub struct Fmt {
    pub fat16: bool,
    pub spc: u8,
    pub clusters: u32,
    /// MBR entry 0 at this LBA with this type; `None` = no MBR.
    pub mbr: Option<(u32, u8)>,
    /// Allocate every other cluster, so no chain is contiguous.
    pub frag: bool,
}

/// Smallest FAT32 that `embedded_sdmmc` also takes for FAT32 (it decides
/// the type by the cluster count, >= 65525).
pub const FAT32: Fmt = Fmt { fat16: false, spc: 1, clusters: 65600, mbr: Some((2048, 0x0C)), frag: false };
pub const FAT16: Fmt = Fmt { fat16: true, spc: 1, clusters: 8000, mbr: Some((63, 0x06)), frag: false };

struct Builder {
    card: Card,
    fmt: Fmt,
    base: u32,
    fat_lba: u32,
    spf: u32,
    root_lba: u32,
    data_lba: u32,
    next: u32,
}

impl Builder {
    fn cbytes(&self) -> usize {
        self.fmt.spc as usize * 512
    }

    fn alloc(&mut self, n: usize) -> Vec<u32> {
        let mut v = vec![];
        for _ in 0..n {
            assert!(self.next < self.fmt.clusters + 2, "image full");
            v.push(self.next);
            self.next += if self.fmt.frag { 2 } else { 1 };
        }
        for w in v.windows(2) {
            self.set_fat(w[0], w[1]);
        }
        if let Some(&l) = v.last() {
            self.set_fat(l, 0x0FFF_FFFF);
        }
        v
    }

    fn set_fat(&mut self, c: u32, v: u32) {
        for f in 0..2 {
            let lba = self.fat_lba + f * self.spf;
            if self.fmt.fat16 {
                let off = lba as u64 * 512 + c as u64 * 2;
                self.card.write(off, &(v as u16).to_le_bytes());
            } else {
                let off = lba as u64 * 512 + c as u64 * 4;
                self.card.write(off, &v.to_le_bytes());
            }
        }
    }

    fn cluster_off(&self, c: u32) -> u64 {
        (self.data_lba as u64 + (c as u64 - 2) * self.fmt.spc as u64) * 512
    }

    fn write_chain(&mut self, chain: &[u32], data: &[u8]) {
        let cb = self.cbytes();
        for (i, part) in data.chunks(cb).enumerate() {
            let off = self.cluster_off(chain[i]);
            self.card.write(off, part);
        }
    }

    fn entry_count(items: &[Item]) -> usize {
        items
            .iter()
            .map(|it| match it {
                Item::File { long, .. } | Item::Dir { long, .. } => 1 + long.as_ref().map_or(0, |l| l.len().div_ceil(13)),
                Item::Raw(v) => v.len(),
            })
            .sum()
    }

    /// Lay out `items` (allocating their clusters) and return the
    /// directory's bytes. `me` is this directory's first cluster (0: root).
    fn dir_bytes(&mut self, items: &[Item], me: u32, parent: u32, is_root: bool) -> Vec<u8> {
        let mut out: Vec<u8> = vec![];
        if !is_root {
            out.extend(short_entry(&sfn(".", ""), 0x10, 0, me, 0));
            out.extend(short_entry(&sfn("..", ""), 0x10, 0, parent, 0));
        }
        for it in items {
            match it {
                Item::Raw(v) => v.iter().for_each(|e| out.extend(e)),
                Item::File { long, short, nt, data } => {
                    let chain = self.alloc(data.len().div_ceil(self.cbytes()));
                    self.write_chain(&chain, data);
                    if let Some(l) = long {
                        lfn_pieces(l, checksum(short)).iter().for_each(|e| out.extend(e));
                    }
                    let first = chain.first().copied().unwrap_or(0);
                    out.extend(short_entry(short, 0x20, *nt, first, data.len() as u32));
                }
                Item::Dir { long, short, items } => {
                    let n = 2 + Self::entry_count(items);
                    let chain = self.alloc((n * 32).div_ceil(self.cbytes()));
                    let bytes = self.dir_bytes(items, chain[0], if is_root { 0 } else { me }, false);
                    self.write_chain(&chain, &bytes);
                    if let Some(l) = long {
                        lfn_pieces(l, checksum(short)).iter().for_each(|e| out.extend(e));
                    }
                    out.extend(short_entry(short, 0x10, 0, chain[0], 0));
                }
            }
        }
        out
    }
}

/// Build a formatted, populated card.
pub fn build(fmt: Fmt, root: Vec<Item>) -> Card {
    let base = fmt.mbr.map_or(0, |m| m.0);
    let reserved: u32 = if fmt.fat16 { 4 } else { 32 };
    let root_entries: u32 = if fmt.fat16 { 512 } else { 0 };
    let root_blocks = root_entries * 32 / 512;
    let per = if fmt.fat16 { 256 } else { 128 };
    let spf = (fmt.clusters + 2).div_ceil(per);
    let fat_lba = base + reserved;
    let root_lba = fat_lba + 2 * spf;
    let data_lba = root_lba + root_blocks;
    let vol_total = reserved + 2 * spf + root_blocks + fmt.clusters * fmt.spc as u32;
    let mut b = Builder {
        card: Card::new(base + vol_total),
        fmt,
        base,
        fat_lba,
        spf,
        root_lba,
        data_lba,
        next: 2,
    };
    // MBR
    if let Some((start, ty)) = fmt.mbr {
        let m = b.card.sector_mut(0);
        m[0x1BE + 4] = ty;
        m[0x1BE + 8..0x1BE + 12].copy_from_slice(&start.to_le_bytes());
        m[0x1BE + 12..0x1BE + 16].copy_from_slice(&vol_total.to_le_bytes());
        m[510] = 0x55;
        m[511] = 0xAA;
    }
    // BPB
    {
        let s = b.card.sector_mut(base);
        s[..3].copy_from_slice(&[0xEB, 0x58, 0x90]);
        s[3..11].copy_from_slice(b"FATMAPTS");
        s[11..13].copy_from_slice(&512u16.to_le_bytes());
        s[13] = fmt.spc;
        s[14..16].copy_from_slice(&(reserved as u16).to_le_bytes());
        s[16] = 2;
        s[17..19].copy_from_slice(&(root_entries as u16).to_le_bytes());
        s[21] = 0xF8;
        s[24] = 63;
        s[26] = 255;
        s[28..32].copy_from_slice(&base.to_le_bytes());
        if fmt.fat16 && vol_total < 0x10000 {
            s[19..21].copy_from_slice(&(vol_total as u16).to_le_bytes());
        } else {
            s[32..36].copy_from_slice(&vol_total.to_le_bytes());
        }
        if fmt.fat16 {
            s[22..24].copy_from_slice(&(spf as u16).to_le_bytes());
            s[0x24] = 0x80;
            s[0x26] = 0x29;
            s[0x2B..0x36].copy_from_slice(b"FATMAP     ");
            s[0x36..0x3E].copy_from_slice(b"FAT16   ");
        } else {
            s[0x24..0x28].copy_from_slice(&spf.to_le_bytes());
            s[0x2C..0x30].copy_from_slice(&2u32.to_le_bytes());
            s[0x30..0x32].copy_from_slice(&1u16.to_le_bytes());
            s[0x32..0x34].copy_from_slice(&6u16.to_le_bytes());
            s[0x40] = 0x80;
            s[0x42] = 0x29;
            s[0x47..0x52].copy_from_slice(b"FATMAP     ");
            s[0x52..0x5A].copy_from_slice(b"FAT32   ");
        }
        s[510] = 0x55;
        s[511] = 0xAA;
    }
    if !fmt.fat16 {
        let s = b.card.sector_mut(base + 1);
        s[0..4].copy_from_slice(&0x4161_5252u32.to_le_bytes());
        s[484..488].copy_from_slice(&0x6141_7272u32.to_le_bytes());
        s[488..496].copy_from_slice(&[0xFF; 8]);
        s[508..512].copy_from_slice(&0xAA55_0000u32.to_le_bytes());
    }
    b.set_fat(0, 0x0FFF_FFF8);
    b.set_fat(1, 0x0FFF_FFFF);
    if fmt.fat16 {
        let bytes = b.dir_bytes(&root, 0, 0, true);
        assert!(bytes.len() <= root_blocks as usize * 512, "FAT16 root full");
        let off = root_lba as u64 * 512;
        b.card.write(off, &bytes);
    } else {
        let n = Builder::entry_count(&root).max(1);
        let chain = b.alloc((n * 32).div_ceil(b.cbytes()));
        let bytes = b.dir_bytes(&root, 0, 0, true);
        // The root directory's own cluster number is in the BPB.
        let rc = chain[0];
        b.card.sector_mut(base)[0x2C..0x30].copy_from_slice(&rc.to_le_bytes());
        b.write_chain(&chain, &bytes);
    }
    let _ = (b.base, b.root_lba);
    b.card
}

// ---------------------------------------------------------------- oracle

/// One directory entry as seen by either reader, normalised for comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seen {
    /// Base and extension, cut at the first space (how `embedded_sdmmc`'s
    /// `base_name()` / `extension()` present them).
    pub base: Vec<u8>,
    pub ext: Vec<u8>,
    pub attr: u8,
    pub size: u32,
    /// First cluster; 0 for none and for the root (`..` of a top folder).
    pub cluster: u32,
    pub long: Option<Vec<u8>>,
}

pub fn cut(b: &[u8]) -> Vec<u8> {
    b.split(|&c| c == b' ').next().unwrap_or(&[]).to_vec()
}

struct Dev<'a>(&'a Card);

impl sd::BlockDevice for Dev<'_> {
    type Error = ();
    fn read(&self, blocks: &mut [sd::Block], start: sd::BlockIdx) -> Result<(), ()> {
        for (i, b) in blocks.iter_mut().enumerate() {
            let lba = start.0 + i as u32;
            if lba >= self.0.total {
                return Err(());
            }
            b.contents = self.0.get(lba);
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

struct Clock;

impl sd::TimeSource for Clock {
    fn get_timestamp(&self) -> sd::Timestamp {
        sd::Timestamp { year_since_1970: 0, zero_indexed_month: 0, zero_indexed_day: 0, hours: 0, minutes: 0, seconds: 0 }
    }
}

fn attr_of(a: sd::Attributes) -> u8 {
    (a.is_read_only() as u8)
        | (a.is_hidden() as u8) << 1
        | (a.is_system() as u8) << 2
        | (a.is_volume() as u8) << 3
        | (a.is_directory() as u8) << 4
        | (a.is_archive() as u8) << 5
}

fn cluster_of(c: sd::ClusterId) -> u32 {
    let s = format!("{c:?}");
    let inner = s.trim_start_matches("ClusterId(").trim_end_matches(')');
    u32::from_str_radix(inner, 16).unwrap_or(0)
}

/// Path element: the raw 11-byte name of a directory.
pub type Name = [u8; 11];

fn sd_err<E: std::fmt::Debug>(e: E) -> String {
    format!("{e:?}")
}

/// Open the volume and walk `path`, then run `f` on the directory.
fn sd_with<R>(
    card: &Card,
    path: &[Name],
    f: impl FnOnce(&sd::VolumeManager<Dev<'_>, Clock>, sd::RawDirectory) -> Result<R, String>,
) -> Result<R, String> {
    let vm = sd::VolumeManager::new(Dev(card), Clock);
    let vol = vm.open_raw_volume(sd::VolumeIdx(0)).map_err(sd_err)?;
    let mut d = vm.open_root_dir(vol).map_err(sd_err)?;
    for want in path {
        let mut hit = None;
        vm.iterate_dir(d, |e| {
            if hit.is_none() && e.attributes.is_directory() && cut(&want[..8]) == e.name.base_name() && cut(&want[8..]) == e.name.extension() {
                hit = Some(e.name.clone());
            }
        })
        .map_err(sd_err)?;
        let n = hit.ok_or("NotFound")?;
        let nd = vm.open_dir(d, &n).map_err(sd_err)?;
        vm.close_dir(d).map_err(sd_err)?;
        d = nd;
    }
    f(&vm, d)
}

/// `iterate_dir_lfn` of the directory at `path` (with a buffer big enough
/// for any long name).
pub fn sd_list(card: &Card, path: &[Name]) -> Result<Vec<Seen>, String> {
    sd_with(card, path, |vm, d| {
        let mut store = [0u8; 1024];
        let mut lfn = sd::LfnBuffer::new(&mut store);
        let mut out = vec![];
        vm.iterate_dir_lfn(d, &mut lfn, |e, long| {
            out.push(Seen {
                base: e.name.base_name().to_vec(),
                ext: e.name.extension().to_vec(),
                attr: attr_of(e.attributes),
                size: e.size,
                cluster: cluster_of(e.cluster),
                long: long.map(|s| s.as_bytes().to_vec()),
            })
        })
        .map_err(sd_err)?;
        Ok(out)
    })
}

/// One step of a read plan.
#[derive(Clone, Copy, Debug)]
pub enum Op {
    Read(usize),
    Seek(u32),
}

/// Open `name` (as `create_from_str` would spell it) in the directory at
/// `path` and run the plan: the bytes of each read, concatenated, or the
/// first error.
pub fn sd_read(card: &Card, path: &[Name], name: &str, plan: &[Op]) -> Result<(u32, Vec<u8>), String> {
    sd_with(card, path, |vm, d| {
        let f = vm.open_file_in_dir(d, name, sd::Mode::ReadOnly).map_err(sd_err)?;
        let len = vm.file_length(f).map_err(sd_err)?;
        let mut out = vec![];
        for op in plan {
            match *op {
                Op::Seek(o) => vm.file_seek_from_start(f, o).map_err(sd_err)?,
                Op::Read(n) => {
                    let mut buf = vec![0u8; n];
                    let got = vm.read(f, &mut buf).map_err(sd_err)?;
                    out.extend_from_slice(&buf[..got]);
                }
            }
        }
        vm.close_file(f).map_err(sd_err)?;
        Ok((len, out))
    })
}

/// `ShortFileName::create_from_str`, as raw bytes (`None` on any error).
pub fn sd_sfn(name: &str) -> Option<[u8; 11]> {
    let s = sd::ShortFileName::create_from_str(name).ok()?;
    // base_name()/extension() cut at a space, and create_from_str never
    // stores one, so this is the whole name.
    let mut a = [b' '; 11];
    a[..s.base_name().len()].copy_from_slice(s.base_name());
    a[8..8 + s.extension().len()].copy_from_slice(s.extension());
    Some(a)
}

/// A plan that reads everything in `chunk`-byte pieces.
pub fn chunks(len: u32, chunk: usize) -> Vec<Op> {
    vec![Op::Read(chunk); (len as usize).div_ceil(chunk) + 1]
}

// ---------------------------------------------------------------- scenario

/// A long name of exactly `n` UTF-16 units (ASCII).
pub fn long_name(n: usize) -> String {
    let mut s: String = (0..n - 4).map(|i| (b'a' + (i % 26) as u8) as char).collect();
    s.push_str(".bin");
    s
}

pub fn label_entry() -> [u8; 32] {
    short_entry(&sfn("NETGD", ""), 0x08, 0, 0, 0)
}

/// The NetGD-shaped card plus every directory-entry case both readers are
/// expected to agree on (see the `sdmmc_quirk_*` tests for the ones they do
/// not). Contents are `lcg(k, size)` with a distinct `k` per file.
pub fn scenario() -> Vec<Item> {
    let mut deleted = lfn_pieces(&utf16("Deleted long name.bin"), checksum(&sfn("DELETE~1", "BIN")));
    deleted.push(short_entry(&sfn("DELETE~1", "BIN"), 0x20, 0, 0, 0));
    for e in &mut deleted {
        e[0] = 0xE5;
    }
    // Pieces whose checksum is not the next entry's: an orphan run.
    let orphan = lfn_pieces(&utf16("Orphan name.txt"), 0x5A);
    let mut e05 = short_entry(&sfn("X5LEAD", "TXT"), 0x20, 0, 0, 0);
    e05[0] = 0x05;
    let many: Vec<Item> = (0..60)
        .map(|i| lfile(&format!("Long file number {i:02} of many.dat"), sfn(&format!("LONGFI~{}", i % 10), &format!("D{i:02}")), lcg(1000 + i, 100 + i as usize * 37)))
        .collect();
    vec![
        Item::Raw(vec![label_entry()]),
        file(sfn("GDEMU", "INI"), lcg(1, 300)),
        Item::Raw(deleted),
        lfile("Track 01.bin", sfn("TRACK0~1", "BIN"), lcg(2, 70000)),
        lfile("日本語のファイル名.txt", sfn("NIHONG~1", "TXT"), lcg(3, 513)),
        lfile("😀 smile.bin", sfn("SMILE~1", "BIN"), lcg(4, 1)),
        // The surrogate pair straddles the first two pieces (units 12, 13).
        lfile("abcdefghijkl😀.txt", sfn("ABCDEF~1", "TXT"), lcg(5, 2048)),
        lfile("ExactThirteen", sfn("EXACTT~1", ""), lcg(6, 512)),
        lfile("ExactlyTwentySixCharacter", sfn("EXACTL~1", ""), lcg(7, 511)),
        lfile(&long_name(100), sfn("ABCDEF~2", "BIN"), lcg(8, 4096)),
        lfile(&long_name(247), sfn("ABCDEF~3", "BIN"), lcg(9, 10)),
        Item::Raw(orphan),
        file(sfn("PLAIN", "TXT"), lcg(10, 5000)),
        Item::File { long: None, short: sfn("LOWER", "TXT"), nt: 0x18, data: lcg(11, 77) },
        Item::Raw(vec![e05]),
        file(sfn("EMPTY", "BIN"), vec![]),
        dir(
            sfn("01", ""),
            vec![
                file(sfn("DISC", "GDI"), lcg(12, 12288)),
                lfile("track01.bin", sfn("TRACK01", "BIN"), lcg(13, 2352 * 30)),
                lfile("Track 02.raw", sfn("TRACK0~1", "RAW"), lcg(14, 2352 * 75 + 7)),
                file(sfn("GAME", "CFG"), lcg(15, 40)),
            ],
        ),
        ldir(
            "Long Folder Name 長い",
            sfn("LONGFO~1", ""),
            vec![dir(sfn("SUB", ""), vec![ldir("Third level", sfn("THIRDL~1", ""), vec![lfile("deep file.cdi", sfn("DEEPFI~1", "CDI"), lcg(16, 9000))])])],
        ),
        dir(sfn("MANY", ""), many),
    ]
}
