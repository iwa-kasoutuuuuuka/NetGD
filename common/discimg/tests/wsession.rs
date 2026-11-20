use discimg::wsession::*;

fn begun(part: u32) -> WriteSession {
    let mut s = WriteSession::new();
    s.begin(b"NGWS", part).unwrap();
    s
}

#[test]
fn begin_needs_the_key() {
    let mut s = WriteSession::new();
    assert_eq!(s.begin(b"NGWX", 2048), Err(Refused::BadKey));
    assert_eq!(s.begin(b"NGW", 2048), Err(Refused::BadKey));
    assert!(!s.active());
    assert_eq!(s.begin(b"NGWS", 2048), Ok(()));
    assert!(s.active());
}

#[test]
fn end_without_begin_is_refused() {
    let mut s = WriteSession::new();
    assert_eq!(s.end(1), Err(Refused::NotActive));
    assert!(!s.active());
}

#[test]
fn end_selects_the_remount_and_closes() {
    for (sel, want) in [(0, Remount::None), (1, Remount::Default), (2, Remount::Named)] {
        let mut s = begun(100);
        assert_eq!(s.end(sel), Ok(want));
        assert!(!s.active());
        assert_eq!(s.end(sel), Err(Refused::NotActive));
    }
}

#[test]
fn bad_selector_keeps_the_session() {
    let mut s = begun(100);
    assert_eq!(s.end(3), Err(Refused::BadSelector));
    assert!(s.active());
    assert_eq!(s.end(255), Err(Refused::BadSelector));
    assert!(s.active());
}

#[test]
fn double_begin_restarts() {
    let mut s = begun(100);
    assert!(s.write_begin(100, 1));
    s.write_data(&[1, 2, 3]);
    s.write_end(true, 1);
    assert_eq!(s.begin(b"NGWS", 200), Ok(()));
    let st = s.status(false);
    assert_eq!(&st[8..12], &[0; 4]); // sectors cleared
    assert_eq!(&st[28..32], &200u32.to_le_bytes()); // boundary re-taken
    assert!(s.active());
    // a bad key on a running session changes nothing
    assert_eq!(s.begin(b"zzzz", 5), Err(Refused::BadKey));
    assert!(s.active());
}

#[test]
fn hard_reset_ends_the_session_once() {
    let mut s = begun(100);
    assert!(s.hard_reset());
    assert!(!s.active());
    assert!(!s.hard_reset()); // nothing to end the second time
    assert_eq!(s.end(1), Err(Refused::NotActive));
    // and the protection is gone with it
    assert!(!s.is_protected(0, 1));
}

#[test]
fn protected_range() {
    let mut s = begun(2048);
    assert!(s.is_protected(0, 1)); // the MBR
    assert!(s.is_protected(2047, 1)); // last gap sector
    assert!(s.is_protected(2040, 100)); // straddles the boundary
    assert!(!s.is_protected(2048, 1)); // first partition sector
    assert!(!s.is_protected(5000, 65536));
    assert!(!s.write_begin(0, 1));
    assert!(s.write_begin(2048, 8));
}

#[test]
fn no_protection_outside_a_session_or_when_unknown() {
    let s = WriteSession::new();
    assert!(!s.is_protected(0, 1));
    let s = begun(0);
    assert!(!s.is_protected(0, 1)); // 0 = unknown / superfloppy
}

#[test]
fn refused_write_is_a_failed_command_and_leaves_sectors() {
    let mut s = begun(2048);
    assert!(!s.write_begin(0, 1));
    let st = s.status(false);
    assert_eq!(st[4], FLAG_SESSION); // last not ok
    assert_eq!(&st[8..12], &[0; 4]);
    assert_eq!(&st[12..16], &1u32.to_le_bytes());
    assert_eq!(&st[16..20], &0u32.to_le_bytes());
    assert_eq!(&st[24..28], &FNV_INIT.to_le_bytes());
}

#[test]
fn fnv_known_vectors() {
    assert_eq!(fnv1a(FNV_INIT, b""), 0x811C9DC5);
    assert_eq!(fnv1a(FNV_INIT, b"a"), 0xE40C292C);
    assert_eq!(fnv1a(FNV_INIT, b"foobar"), 0xBF9CF968);
}

#[test]
fn checksum_spans_sectors_and_runs() {
    // F4 splits a 65536-sector command into runs; the checksum is still one
    // value over the whole command, however the bytes arrive.
    let mut data = [0u8; 512];
    let mut whole = FNV_INIT;
    let mut s = begun(2048);
    assert!(s.write_begin(4096, 3));
    for n in 0..3u8 {
        for (i, b) in data.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(7).wrapping_add(n);
        }
        whole = fnv1a(whole, &data);
        s.write_data(&data);
    }
    s.write_end(true, 3);
    let st = s.status(false);
    assert_eq!(&st[24..28], &whole.to_le_bytes());
    assert_eq!(&st[20..24], &3u32.to_le_bytes());
    // the same bytes in one piece give the same value
    let mut all = [0u8; 1536];
    for n in 0..3usize {
        for i in 0..512 {
            all[n * 512 + i] = (i as u8).wrapping_mul(7).wrapping_add(n as u8);
        }
    }
    assert_eq!(fnv1a(FNV_INIT, &all), whole);
}

#[test]
fn checksum_restarts_per_command_and_sectors_accumulate() {
    let mut s = begun(100);
    assert!(s.write_begin(100, 1));
    s.write_data(b"first");
    s.write_end(true, 1);
    assert!(s.write_begin(200, 2));
    s.write_data(b"second");
    s.write_end(true, 2);
    let st = s.status(true);
    assert_eq!(&st[24..28], &fnv1a(FNV_INIT, b"second").to_le_bytes());
    assert_eq!(&st[8..12], &3u32.to_le_bytes());
    assert_eq!(&st[16..20], &200u32.to_le_bytes());
    assert_eq!(st[4], FLAG_MEDIA | FLAG_SESSION | FLAG_LAST_OK);
}

#[test]
fn failed_command_is_counted_and_partial_sectors_kept() {
    let mut s = begun(100);
    assert!(s.write_begin(100, 8));
    s.write_end(false, 5);
    let st = s.status(false);
    assert_eq!(st[4], FLAG_SESSION);
    assert_eq!(&st[8..12], &5u32.to_le_bytes());
    assert_eq!(&st[12..16], &1u32.to_le_bytes());
}

#[test]
fn status_layout() {
    let st = begun(2048).status(false);
    assert_eq!(st.len(), 48);
    assert_eq!(&st[..4], b"NGST");
    assert_eq!(&st[5..8], &[0; 3]);
    assert_eq!(&st[28..32], &2048u32.to_le_bytes());
    assert_eq!(&st[32..], &[0u8; 16]);
}

#[test]
fn writes_work_outside_a_session_without_protection() {
    let mut s = WriteSession::new();
    assert!(s.write_begin(0, 1)); // ejected-host path: whole disk
    s.write_data(b"x");
    s.write_end(true, 1);
    assert_eq!(s.status(false)[4], FLAG_LAST_OK);
}

#[test]
fn partition_start_reading() {
    let mut sec = [0u8; 512];
    assert_eq!(partition_start(&sec, false), 0); // no signature
    sec[510] = 0x55;
    sec[511] = 0xAA;
    assert_eq!(partition_start(&sec, false), 0); // empty entry 0
    sec[0x1BE + 8..0x1BE + 12].copy_from_slice(&8192u32.to_le_bytes());
    assert_eq!(partition_start(&sec, false), 8192);
    assert_eq!(partition_start(&sec, true), 0); // superfloppy: LBA 0 is a BPB
}
