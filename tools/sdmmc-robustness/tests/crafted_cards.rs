//! Crafted cards that made embedded-sdmmc 0.9.0 panic or hang, through the
//! patched (vendored) copy the firmware builds with. Found by review of the
//! LFN fix; each test fails (panics, or hits its deadline) on the crates.io
//! release. See mcu/netgd/vendor/README.md.
//!
//! * `lfn_push_fuzz`, `dir_entry_fuzz`: random long-name pieces and entries;
//! * `zero_blocks_per_cluster`: BPB byte 13 = 0 divided by zero at mount;
//! * `root_cluster_top_bits`, `subdir_cluster_top_bits`: a start cluster
//!   with bits above 28 set reads (aliased) and then panicked in
//!   `next_cluster`;
//! * `cyclic_directory_chain*`: a FAT loop in a directory never returned;
//!   now the walk ends after the largest legal directory size.

mod common;

use common::*;
use embedded_sdmmc as sd;
use std::panic::catch_unwind;
use std::sync::mpsc;
use std::time::Duration;

struct D<'a>(&'a Card);
impl sd::BlockDevice for D<'_> {
    type Error = ();
    fn read(&self, blocks: &mut [sd::Block], start: sd::BlockIdx) -> Result<(), ()> {
        for (i, b) in blocks.iter_mut().enumerate() {
            let lba = start.0.wrapping_add(i as u32);
            if lba >= self.0.total {
                return Err(()); // like a real card
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

struct C;
impl sd::TimeSource for C {
    fn get_timestamp(&self) -> sd::Timestamp {
        sd::Timestamp { year_since_1970: 0, zero_indexed_month: 0, zero_indexed_day: 0, hours: 0, minutes: 0, seconds: 0 }
    }
}

/// Mount and list the root as `gdi::walk_to` does: the entry count or the
/// error.
fn list(card: &Card) -> Result<usize, String> {
    let vm = sd::VolumeManager::new(D(card), C);
    let vol = vm.open_volume(sd::VolumeIdx(0)).map_err(|e| format!("{e:?}"))?;
    let root = vol.open_root_dir().map_err(|e| format!("{e:?}"))?;
    let mut store = [0u8; 384];
    let mut lfn = sd::LfnBuffer::new(&mut store);
    let mut n = 0;
    root.iterate_dir_lfn(&mut lfn, |_e, long| {
        if let Some(l) = long {
            assert!(std::str::from_utf8(l.as_bytes()).is_ok());
        }
        n += 1;
    })
    .map_err(|e| format!("{e:?}"))?;
    Ok(n)
}

/// `list` on another thread with a deadline, so a regression to the hang
/// fails the test instead of stalling the suite.
fn list_within(card: Card, secs: u64) -> Option<Result<usize, String>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(catch_unwind(|| list(&card)).unwrap_or_else(|_| Err("PANIC".into())));
    });
    rx.recv_timeout(Duration::from_secs(secs)).ok()
}

struct Rng(u64);
impl Rng {
    fn n(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 20) as u32
    }
}

#[test]
fn lfn_push_fuzz() {
    let mut r = Rng(0x1234_5678_9abc_def1);
    let alpha = [0x41u16, 0xD83D, 0xDE00, 0x0000, 0xFFFF, 0xDBFF, 0xDC00, 0x20AC, 0xFFFE];
    for size in [0usize, 1, 2, 3, 4, 5, 39, 384] {
        let mut store = vec![0u8; size];
        let mut lfn = sd::LfnBuffer::new(&mut store);
        for _ in 0..200_000 {
            let mut b = [0u16; 13];
            let mode = r.n() % 4;
            for u in b.iter_mut() {
                *u = match mode {
                    0 => alpha[(r.n() % 9) as usize],
                    1 => alpha[(r.n() % 3) as usize],
                    2 => [0xD83D, 0xDE00][(r.n() % 2) as usize],
                    _ => r.n() as u16,
                };
            }
            if r.n() % 7 == 0 {
                lfn.clear();
            }
            lfn.push(&b);
            assert!(std::str::from_utf8(lfn.as_str().as_bytes()).is_ok());
        }
    }
}

#[test]
fn dir_entry_fuzz() {
    let base = build(FAT32, vec![file(sfn("A", "TXT"), vec![1])]);
    let root_lba = *base.sectors.iter().find(|(_, s)| &s[..11] == b"A       TXT").unwrap().0;
    let mut r = Rng(0xfeed_beef_1234_5677);
    let alpha = [0x41u16, 0xD83D, 0xDE00, 0x0000, 0xFFFF];
    for _ in 0..100_000 {
        let mut c = base.clone();
        let s = c.sector_mut(root_lba);
        for e in 0..16 {
            let o = e * 32;
            if r.n() % 8 < 5 {
                s[o] = [0x41, 0x42, 0x43, 0x01, 0x02, 0x03, 0x14, 0x54, 0x53, 0x5F, 0x7F, 0x01, 0x40, 0x60, 0xE5, 0x13][(r.n() % 16) as usize];
                s[o + 11] = 0x0F;
                s[o + 13] = (r.n() % 3) as u8;
                for k in [1usize, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30] {
                    let u = if r.n() % 5 == 0 { r.n() as u16 } else { alpha[(r.n() % 5) as usize] };
                    s[o + k..o + k + 2].copy_from_slice(&u.to_le_bytes());
                }
            } else {
                for k in 0..32 {
                    s[o + k] = r.n() as u8;
                }
                if s[o] == 0 {
                    s[o] = 0x41;
                }
            }
        }
        let _ = list(&c);
    }
}

/// A. Boot sector byte 13 (blocks per cluster) = 0: mount fails cleanly.
#[test]
fn zero_blocks_per_cluster() {
    let mut c = build(FAT32, vec![file(sfn("A", "TXT"), vec![1])]);
    c.sector_mut(2048)[13] = 0;
    let r = catch_unwind(|| list(&c));
    assert!(matches!(r, Ok(Err(_))), "{:?}", r.map_err(|_| "PANIC"));
}

fn big_cluster_card(items: Vec<Item>) -> Card {
    build(Fmt { fat16: false, spc: 4, clusters: 65600, mbr: Some((2048, 0x0C)), frag: false }, items)
}

/// B. BPB root cluster 0x40000002 (aliases cluster 2 with 4 blocks per
/// cluster): no panic.
#[test]
fn root_cluster_top_bits() {
    let items: Vec<Item> = (0..70).map(|i| file(sfn(&format!("F{i:03}"), "BIN"), vec![])).collect();
    let mut c = big_cluster_card(items);
    assert_eq!(list(&c), Ok(70));
    c.sector_mut(2048)[0x2C..0x30].copy_from_slice(&0x4000_0002u32.to_le_bytes());
    // The first cluster reads aliased (cluster 2, 64 entries); the walk ends
    // at `next_cluster`, which refuses the cluster instead of panicking.
    let r = catch_unwind(|| list(&c));
    assert!(matches!(r, Ok(Ok(64))), "{:?}", r.map_err(|_| "PANIC"));
}

/// B. A folder whose entry's cluster-high word is 0x4000: listing it and
/// looking a file up in it do not panic.
#[test]
fn subdir_cluster_top_bits() {
    let inner: Vec<Item> = (0..70).map(|i| file(sfn(&format!("F{i:03}"), "BIN"), vec![])).collect();
    let mut c = big_cluster_card(vec![dir(sfn("GAME", ""), inner)]);
    let lba = *c.sectors.iter().find(|(_, s)| &s[..11] == b"GAME       ").unwrap().0;
    c.sector_mut(lba)[20..22].copy_from_slice(&0x4000u16.to_le_bytes());
    let listed = catch_unwind(|| {
        let vm = sd::VolumeManager::new(D(&c), C);
        let vol = vm.open_volume(sd::VolumeIdx(0)).unwrap();
        let mut d = vol.open_root_dir().unwrap();
        d.change_dir("GAME").unwrap();
        let mut n = 0;
        let e = d.iterate_dir(|_| n += 1).is_ok();
        (e, n)
    });
    // Same: the aliased first cluster is listed, then the walk ends.
    assert!(matches!(listed, Ok((true, 64))), "{:?}", listed.map_err(|_| "PANIC"));
    let found = catch_unwind(|| {
        let vm = sd::VolumeManager::new(D(&c), C);
        let vol = vm.open_volume(sd::VolumeIdx(0)).unwrap();
        let mut d = vol.open_root_dir().unwrap();
        d.change_dir("GAME").unwrap();
        let e = d.open_file_in_dir("NOPE.GDI", sd::Mode::ReadOnly).is_err();
        e
    });
    assert!(matches!(found, Ok(true)), "{:?}", found.map_err(|_| "PANIC"));
}

/// C. The root's first cluster points back at itself in the FAT (and the
/// cluster is full, so no end marker stops the walk): returns within the
/// deadline.
#[test]
fn cyclic_directory_chain() {
    let fmt = Fmt { fat16: false, spc: 1, clusters: 65600, mbr: Some((2048, 0x0C)), frag: false };
    let items: Vec<Item> = (0..40).map(|i| file(sfn(&format!("F{i:03}"), "BIN"), vec![])).collect();
    let mut c = build(fmt, items);
    let fat = 2048 + 32;
    c.sector_mut(fat)[8..12].copy_from_slice(&2u32.to_le_bytes()); // FAT[2] = 2
    match list_within(c, 10) {
        // The walk ends after 2 MiB of directory (4096 one-block clusters of
        // 16 entries), like any other broken chain: no error, no hang.
        Some(Ok(n)) => assert_eq!(n, 4096 * 16),
        Some(Err(e)) => panic!("{e}"),
        None => panic!("cyclic chain: no return within 10 s"),
    }
}

/// The same on a FAT16 subdirectory, with a 4-cluster cycle, for both the
/// listing and the look-up.
#[test]
fn cyclic_directory_chain_fat16() {
    let inner: Vec<Item> = (0..62).map(|i| file(sfn(&format!("F{i:03}"), "BIN"), vec![])).collect();
    let mut c = build(FAT16, vec![dir(sfn("GAME", ""), inner)]);
    // GAME fills clusters 2..=5 exactly (64 entries, no end marker; the FAT16
    // root is fixed); the 4th points back at the 1st.
    let fat = 63 + 4;
    c.sector_mut(fat)[2 * 5..2 * 5 + 2].copy_from_slice(&2u16.to_le_bytes());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let r = catch_unwind(|| {
            let vm = sd::VolumeManager::new(D(&c), C);
            let vol = vm.open_volume(sd::VolumeIdx(0)).unwrap();
            let mut d = vol.open_root_dir().unwrap();
            d.change_dir("GAME").unwrap();
            let a = d.iterate_dir(|_| {}).is_ok();
            let b = d.open_file_in_dir("NOPE.GDI", sd::Mode::ReadOnly).is_err();
            (a, b)
        });
        let _ = tx.send(r.ok());
    });
    match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Some((true, true))) => {}
        Ok(r) => panic!("{r:?}"),
        Err(_) => panic!("FAT16 cyclic chain: no return within 10 s"),
    }
}
