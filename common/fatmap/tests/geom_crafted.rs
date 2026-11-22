//! Crafted boot sectors and FAT values against `fatmap::geom` (the
//! firmware's `fat::probe` arithmetic): the failure classes a review found
//! in `embedded_sdmmc` 0.9.0 -- nonsense geometry, cluster numbers past
//! FAT32's 28 bits or outside the volume, and looping directory chains --
//! must be refused or bounded, never a wrong block number or a walk whose
//! length grows with the card (the firmware has no watchdog while it reads
//! the card).

use fatmap::extent::{walk, Walk};
use fatmap::geom::{parse_fat32, BpbError, Geometry};

/// A FAT32 boot sector (as in `tests/geom.rs`).
fn bpb(spc: u8, reserved: u16, fats: u8, spf: u32, total: u32, root: u32) -> [u8; 512] {
    let mut s = [0u8; 512];
    s[0] = 0xEB;
    s[0x0B..0x0D].copy_from_slice(&512u16.to_le_bytes());
    s[0x0D] = spc;
    s[0x0E..0x10].copy_from_slice(&reserved.to_le_bytes());
    s[0x10] = fats;
    s[0x20..0x24].copy_from_slice(&total.to_le_bytes());
    s[0x24..0x28].copy_from_slice(&spf.to_le_bytes());
    s[0x2C..0x30].copy_from_slice(&root.to_le_bytes());
    s[510] = 0x55;
    s[511] = 0xAA;
    s
}

/// Zero reserved sectors put FAT 0 on top of the boot sector.
#[test]
fn reserved_zero_is_refused() {
    assert_eq!(parse_fat32(&bpb(8, 0, 2, 8000, 32 + 16000 + 8_000_000, 2), 0), Err(BpbError::BadBpb));
}

/// A volume whose metadata or data area ends past block 2^32 - 1: the
/// firmware's `base + meta` wrapped (release builds have no overflow
/// checks), giving block numbers at the start of the card.
#[test]
fn volume_past_lba_space_is_refused() {
    // The metadata alone crosses 2^32.
    let s = bpb(8, 32, 2, 8000, 32 + 16000 + 8 * 1000, 2);
    assert_eq!(parse_fat32(&s, 0xFFFF_FF00), Err(BpbError::BadBpb));
    // The data area crosses it.
    let s = bpb(1, 32, 1, 0x0040_0000, 0x2000_0000, 2);
    assert!(parse_fat32(&s, 0).is_ok());
    assert_eq!(parse_fat32(&s, 0xF000_0000), Err(BpbError::BadBpb));
    // A volume may use every block up to and including 0xFFFFFFFF: its
    // exclusive end is then exactly 2^32. One block further does not fit.
    let total = 32 + 1000 + 100_000;
    let s = bpb(1, 32, 1, 1000, total, 2);
    assert!(parse_fat32(&s, u32::MAX - total).is_ok()); // last block 0xFFFFFFFE
    assert!(parse_fat32(&s, u32::MAX - total + 1).is_ok()); // last block 0xFFFFFFFF
    assert_eq!(parse_fat32(&s, u32::MAX - total + 2), Err(BpbError::BadBpb));
}

/// A BPB claiming more clusters than FAT32's 28-bit numbers allow (a huge
/// FAT on a huge volume). Capped at 0x0FFFFFF5, so a root or an entry with
/// any of the top four bits set is outside the volume instead of a cluster
/// that aliases a real one, and the bad mark 0x0FFFFFF7 is never a link.
#[test]
fn cluster_count_is_capped_at_28_bits() {
    let s = bpb(1, 32, 1, 0x0200_0000, u32::MAX, 2);
    let g = parse_fat32(&s, 0).unwrap();
    assert!(g.clusters <= 0x0FFF_FFF5, "{:#x}", g.clusters);
    assert_eq!(g.link(0x0FFF_FFF7), Err(()));
    assert_eq!(g.link(0x0FFF_FFF6), Ok(Some(0x0FFF_FFF6)));
    assert_eq!(g.cluster_lba(0x1000_0003), None);
    assert_eq!(g.start(0x1000_0003, 100), None);
    let s = bpb(1, 32, 1, 0x0200_0000, u32::MAX, 0x1000_0002);
    assert_eq!(parse_fat32(&s, 0), Err(BpbError::RootOutside));
}

/// A file whose first cluster is outside the volume is not walked: a file
/// of one cluster never consults the FAT, so before this the walk emitted
/// an extent for whatever block `data_lba + (c - 2) * spc` was.
#[test]
fn out_of_volume_first_cluster_is_refused() {
    let g = parse_fat32(&bpb(8, 32, 2, 8000, 32 + 16000 + 8 * 1000, 2), 0).unwrap();
    assert_eq!(g.clusters, 1000);
    assert!(g.start(1001, 4096).is_some());
    for c in [1002u32, 5000, 0x0FFF_FFF0, 0x1000_0003, u32::MAX] {
        assert_eq!(g.start(c, 4096), None, "{c:#x}");
    }
    // What the old cursor led to: an extent past the end of the volume.
    let cur = fatmap::extent::Cursor::start(5000, 4096).unwrap();
    let mut out = vec![];
    let r = walk(&g.cluster_geom(), cur, |_| Err(()), |_, e| {
        out.push(e);
        true
    });
    assert_eq!(r, Walk::Done);
    assert!(out[0].lba >= g.data_lba + g.clusters * 8, "the hole this closes");
}

/// A directory chain that loops stops after 65536 entries (4096 blocks,
/// the FAT limit for a directory), not after as many clusters as the
/// volume has: with 2M clusters of 64 blocks that was 128M block reads.
#[test]
fn looping_directory_is_bounded_by_the_directory_limit() {
    let geom = |spc: u8, clusters: u32| -> Geometry {
        let spf = clusters.div_ceil(128) + 1;
        parse_fat32(&bpb(spc, 32, 1, spf, 32 + spf + clusters * spc as u32, 2), 0).unwrap()
    };
    for (spc, want) in [(1u8, 4096u32), (8, 512), (64, 64), (128, 32)] {
        assert_eq!(geom(spc, 2_000_000).dir_max_hops(), want, "spc {spc}");
    }
    // A small volume: its cluster count is the tighter bound.
    assert_eq!(geom(1, 100).dir_max_hops(), 100);
    // The firmware's `lookup` loop over a chain 2 -> 3 -> 2, counted the
    // same way, with a hard stop so a regression cannot hang the suite.
    let g = geom(64, 2_000_000);
    let fat = |c: u32| g.link(if c == 2 { 3 } else { 2 });
    let (mut cluster, mut hops, mut blocks) = (g.root_cluster, 0u32, 0u64);
    let ended = loop {
        hops += 1;
        if hops > g.dir_max_hops() {
            break true;
        }
        if blocks > 10_000_000 {
            break false;
        }
        blocks += g.blocks_per_cluster as u64;
        cluster = fat(cluster).unwrap().unwrap();
    };
    assert!(ended && blocks <= 4096, "{blocks} blocks read");
}
