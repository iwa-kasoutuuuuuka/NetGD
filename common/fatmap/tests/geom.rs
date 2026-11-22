//! `fatmap::geom`: the firmware's FAT32 boot-sector arithmetic
//! (`mcu/netgd/src/fat.rs` `probe`) on built boot sectors.

use fatmap::geom::{looks_like_bpb, parse_fat32, BpbError, Geometry};

/// A FAT32 boot sector.
fn bpb(spc: u8, reserved: u16, fats: u8, spf: u32, total: u32, root: u32) -> [u8; 512] {
    let mut s = [0u8; 512];
    s[0] = 0xEB;
    s[1] = 0x58;
    s[2] = 0x90;
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

/// 8 GB-ish card: 8 blocks per cluster, 2 FATs.
fn good() -> [u8; 512] {
    bpb(8, 32, 2, 8000, 32 + 2 * 8000 + 8 * 1_000_000, 2)
}

#[test]
fn a_good_volume() {
    let s = good();
    assert!(looks_like_bpb(&s));
    let g = parse_fat32(&s, 2048).unwrap();
    assert_eq!(
        g,
        Geometry { fat_lba: 2048 + 32, sectors_per_fat: 8000, data_lba: 2048 + 32 + 16000, blocks_per_cluster: 8, root_cluster: 2, clusters: 1_000_000 }
    );
    // The FAT holds fewer entries than the data area has clusters: the FAT wins.
    let small_fat = parse_fat32(&bpb(8, 32, 2, 1000, 32 + 2 * 1000 + 8 * 1_000_000, 2), 0).unwrap();
    assert_eq!(small_fat.clusters, 1000 * 128 - 2);
}

#[test]
fn shapes_that_are_not_fat32() {
    let mut fat16 = good();
    fat16[0x11] = 2; // root entries
    assert_eq!(parse_fat32(&fat16, 0), Err(BpbError::NotFat32));
    let mut fat16b = good();
    fat16b[0x16] = 1; // 16-bit FAT size
    assert_eq!(parse_fat32(&fat16b, 0), Err(BpbError::NotFat32));
    assert_eq!(parse_fat32(&bpb(8, 32, 2, 0, 1 << 20, 2), 0), Err(BpbError::NotFat32));
    assert_eq!(parse_fat32(&bpb(8, 32, 0, 8000, 1 << 24, 2), 0), Err(BpbError::NotFat32));
    assert_eq!(parse_fat32(&bpb(8, 32, 2, 8000, 1 << 24, 1), 0), Err(BpbError::NotFat32));
    let mut no_jump = good();
    no_jump[0] = 0;
    assert!(!looks_like_bpb(&no_jump));
    let mut spc0 = good();
    spc0[0x0D] = 0;
    assert!(!looks_like_bpb(&spc0));
    let mut bps = good();
    bps[0x0B..0x0D].copy_from_slice(&1024u16.to_le_bytes());
    assert!(!looks_like_bpb(&bps));
}

#[test]
fn inconsistent_bpbs() {
    // Division by zero: sectors per cluster 0 is refused, not divided by.
    assert_eq!(parse_fat32(&bpb(0, 32, 2, 8000, 1 << 24, 2), 0), Err(BpbError::BadBpb));
    // FATs that overflow 32 bits.
    assert_eq!(parse_fat32(&bpb(8, 32, 2, 0xFFFF_FFF0, u32::MAX, 2), 0), Err(BpbError::BadBpb));
    assert_eq!(parse_fat32(&bpb(8, 0xFFFF, 1, 0xFFFF_FFF0, u32::MAX, 2), 0), Err(BpbError::BadBpb));
    // Nothing after the metadata.
    assert_eq!(parse_fat32(&bpb(8, 32, 2, 8000, 32 + 16000, 2), 0), Err(BpbError::BadBpb));
    // Root past the last cluster.
    assert_eq!(parse_fat32(&bpb(8, 32, 2, 8000, 32 + 16000 + 8 * 100, 102), 0), Err(BpbError::RootOutside));
    assert!(parse_fat32(&bpb(8, 32, 2, 8000, 32 + 16000 + 8 * 100, 101), 0).is_ok());
}

#[test]
fn clusters_and_links() {
    let g = parse_fat32(&good(), 0).unwrap();
    assert_eq!(g.cluster_lba(0), None);
    assert_eq!(g.cluster_lba(1), None);
    assert_eq!(g.cluster_lba(2), Some(g.data_lba));
    assert_eq!(g.cluster_lba(1_000_001), Some(g.data_lba + 8 * 999_999));
    assert_eq!(g.cluster_lba(1_000_002), None);
    assert_eq!(g.link(0x0FFF_FFFF), Ok(None));
    assert_eq!(g.link(0xFFFF_FFF8), Ok(None));
    assert_eq!(g.link(0x1000_0005), Ok(Some(5)), "top four bits are ignored in a FAT entry");
    for bad in [0, 1, 0x0FFF_FFF7, 1_000_002, 0x0FFF_FFF0] {
        assert_eq!(g.link(bad), Err(()), "{bad:#x}");
    }
    assert!(g.start(2, 1).is_some());
    assert!(g.start(0, 100).is_none());
    assert!(g.start(5, 0).is_none());
}
