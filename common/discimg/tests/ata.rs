use discimg::ata::{advertise_mwdma, runs, MWDMA_CYCLE_NS, WORD49_DMA, WORD49_LBA, WORD63_MWDMA2};

fn collect(lba: u32, count: u32, max: u32) -> Vec<(u32, u32)> {
    runs(lba, count, max).collect()
}

#[test]
fn small_counts_are_one_run() {
    assert_eq!(collect(100, 1, 32768), vec![(100, 1)]);
    assert_eq!(collect(100, 255, 32768), vec![(100, 255)]);
    assert_eq!(collect(100, 256, 32768), vec![(100, 256)]);
}

#[test]
fn zero_count_is_empty() {
    assert!(collect(5, 0, 32768).is_empty());
}

#[test]
fn count_65535_is_one_run_at_the_hsmci_limit() {
    assert_eq!(collect(7, 65535, 65535), vec![(7, 65535)]);
}

#[test]
fn count_65536_splits_in_two_with_continuous_lba() {
    assert_eq!(collect(1000, 65536, 32768), vec![(1000, 32768), (33768, 32768)]);
}

#[test]
fn count_65536_at_the_hsmci_limit_splits_65535_plus_1() {
    assert_eq!(collect(0, 65536, 65535), vec![(0, 65535), (65535, 1)]);
}

#[test]
fn max_run_boundaries() {
    assert_eq!(collect(0, 9, 10), vec![(0, 9)]);
    assert_eq!(collect(0, 10, 10), vec![(0, 10)]);
    assert_eq!(collect(0, 11, 10), vec![(0, 10), (10, 1)]);
    assert_eq!(collect(0, 20, 10), vec![(0, 10), (10, 10)]);
    assert_eq!(collect(0, 21, 10), vec![(0, 10), (10, 10), (20, 1)]);
}

#[test]
fn max_run_zero_is_treated_as_one() {
    assert_eq!(collect(3, 3, 0), vec![(3, 1), (4, 1), (5, 1)]);
}

#[test]
fn runs_are_contiguous_and_cover_everything() {
    for &(lba, count, max) in &[(0u32, 65536u32, 32768u32), (123, 65536, 1000), (9, 70000, 65535), (1, 1, 1)] {
        let mut next = lba;
        let mut total = 0u32;
        for (first, n) in runs(lba, count, max) {
            assert_eq!(first, next);
            assert!(n >= 1 && n <= max);
            next = first + n;
            total += n;
        }
        assert_eq!(total, count);
    }
}

#[test]
fn identify_dma_words() {
    let mut w = [0u16; 256];
    w[49] = WORD49_LBA;
    w[0] = 0x0040;
    advertise_mwdma(&mut w);
    assert_eq!(w[49], 0x0300);
    assert_eq!(w[63], 0x0407);
    assert_eq!(WORD49_DMA, 0x0100);
    assert_eq!(WORD63_MWDMA2 & 0x4, 0x4, "KOS tests bit 2");
    assert_eq!(w[0], 0x0040, "other words untouched");
    assert_eq!(w[64], 0);
    assert_eq!(MWDMA_CYCLE_NS, 120);
    assert_eq!(w[65], 0x0078, "min MWDMA cycle time");
    assert_eq!(w[66], 0x0078, "recommended MWDMA cycle time");
    assert_eq!(w[67], 0);
}
