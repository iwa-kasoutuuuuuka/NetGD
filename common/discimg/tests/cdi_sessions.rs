//! CDI session handling end to end: descriptor -> `parse_info` ->
//! `cdi_boot_area_lba` -> REQ_SES / TOC. The boot area of a CDI is the first
//! track of the LAST session (Flycast `Disc::GetSessionInfo` reports one
//! first-track/StartFAD per session; the BIOS boots the last), not "track >= 3".

use discimg::atapi::*;
use discimg::cdi::parse_info;
use discimg::{TrackDesc, VecSink};

/// (pregap, sector_count, start_lba, total_length, read_mode, control)
type T = (u32, u32, u32, u32, u32, u32);

fn build(sessions: &[Vec<T>]) -> Vec<u8> {
    let mut f = vec![0xAAu8; 1000];
    let desc_start = f.len();
    f.push(sessions.len() as u8);
    for s in sessions {
        f.extend_from_slice(&[0, s.len() as u8, 0, 0, 0, 0, 0]);
        for (i, t) in s.iter().enumerate() {
            let mut h = vec![0x11u8; 25];
            h[24] = 4;
            f.extend_from_slice(&h);
            f.extend_from_slice(b"NAME");
            f.extend_from_slice(&[0x22u8; 31]);
            let mut d = vec![0u8; if i + 1 == s.len() { 188 } else { 180 }];
            d[2..6].copy_from_slice(&t.0.to_le_bytes());
            d[6..10].copy_from_slice(&t.1.to_le_bytes());
            d[32..36].copy_from_slice(&t.2.to_le_bytes());
            d[36..40].copy_from_slice(&t.3.to_le_bytes());
            d[56..60].copy_from_slice(&t.4.to_le_bytes());
            d[60..64].copy_from_slice(&t.5.to_le_bytes());
            f.extend_from_slice(&d);
        }
    }
    f.extend_from_slice(&[0u8; 7]);
    f.extend_from_slice(&[0u8; 56]);
    f.extend_from_slice(&[0x33u8; 40]);
    let len = (f.len() - desc_start + 4) as u32;
    f.extend_from_slice(&len.to_le_bytes());
    f
}

fn load(sessions: &[Vec<T>]) -> (u8, u8, u32, Vec<TrackDesc>) {
    let f = build(sessions);
    let mut sink = VecSink::new();
    let info = parse_info(
        f.len() as u32,
        |off, buf| {
            buf.copy_from_slice(f.get(off as usize..off as usize + buf.len()).ok_or(())?);
            Ok(())
        },
        &mut sink,
    )
    .unwrap();
    let tracks: Vec<TrackDesc> = sink.0.iter().cloned().collect();
    let boot = cdi_boot_area_lba(&tracks[..], info.boot_track);
    (info.sessions, info.boot_track, boot, tracks)
}

// Audio track: 11000 data sectors, 150 pregap, starts at descriptor LBA 0 -> data LBA 0.
const AUDIO: T = (150, 11000, 0, 11150, 2, 0);
// Data track of the second session, descriptor start 11700 (the common
// homebrew layout) -> data LBA 11700 + 150 - 150 = 11700.
const DATA2: T = (150, 20000, 11700, 20150, 1, 4);

#[test]
fn one_data_track_boots_from_track_1() {
    let (n, bt, boot, t) = load(&[vec![(150, 5000, 0, 5150, 1, 4)]]);
    assert_eq!((n, bt, boot), (1, 1, 0));
    // The old code fell back to 45000 here (and prefetched IP.BIN from there).
    assert_eq!(req_ses_reply(&t, true, boot, n, 0), Some([1, 0, 1, 0x00, 0x14, 0x1E])); // 5000+150 = 5150
}

#[test]
fn audio_then_data_in_two_sessions() {
    let (n, bt, boot, t) = load(&[vec![AUDIO], vec![DATA2]]);
    assert_eq!((n, bt, boot), (2, 2, 11700));
    assert_eq!(t[1].start_lba, 11700);
    // session 0: count 2, lead-out 11700+20000+150 = 31850 = 0x7C6A
    assert_eq!(req_ses_reply(&t, true, boot, n, 0), Some([1, 0, 2, 0x00, 0x7C, 0x6A]));
    // session 1: track 1 at FAD 150
    assert_eq!(req_ses_reply(&t, true, boot, n, 1), Some([1, 0, 1, 0x00, 0x00, 0x96]));
    // session 2: track 2 at FAD 11850 = 0x2E4A (was ABRT with boot_area_lba = 45000)
    assert_eq!(req_ses_reply(&t, true, boot, n, 2), Some([1, 0, 2, 0x00, 0x2E, 0x4A]));
    assert_eq!(req_ses_reply(&t, true, boot, n, 3), None);
}

#[test]
fn data_then_data_in_two_sessions() {
    let d1: T = (150, 3000, 0, 3150, 1, 4);
    let (n, bt, boot, t) = load(&[vec![d1], vec![DATA2]]);
    assert_eq!((n, bt, boot), (2, 2, 11700));
    // Session 1 is track 1 even though it is data and below the boot area.
    assert_eq!(req_ses_reply(&t, true, boot, n, 1), Some([1, 0, 1, 0x00, 0x00, 0x96]));
    assert_eq!(req_ses_reply(&t, true, boot, n, 2), Some([1, 0, 2, 0x00, 0x2E, 0x4A]));
}

#[test]
fn mil_cd_three_tracks_gd_shaped_second_session_at_45000() {
    let a2: T = (0, 7000, 11150, 7000, 2, 0);
    let d: T = (150, 20000, 45000, 20150, 1, 4);
    let (n, bt, boot, t) = load(&[vec![AUDIO, a2], vec![d]]);
    assert_eq!((n, bt, boot), (2, 3, 45000));
    assert_eq!(req_ses_reply(&t, true, boot, n, 2), Some([1, 0, 3, 0x00, 0xB0, 0x5E]));
}

#[test]
fn three_tracks_in_one_session_boot_from_the_session_start() {
    let a2: T = (150, 1000, 11150, 1150, 2, 0);
    let d: T = (150, 2000, 12300, 2150, 1, 4);
    let (n, bt, boot, t) = load(&[vec![AUDIO, a2, d]]);
    assert_eq!((n, bt, boot), (1, 1, 0));
    // One session: no session 2.
    assert_eq!(req_ses_reply(&t, true, boot, n, 2), None);
    assert!(req_ses_reply(&t, true, boot, n, 1).is_some());
}

#[test]
fn cdi_toc_lists_every_track_in_area_0() {
    let (_, _, boot, t) = load(&[vec![AUDIO], vec![DATA2]]);
    let mut out = [0u8; 408];
    build_toc(&t[..], true, boot, 0, &mut out);
    assert_eq!(&out[0..4], &[0x01, 0x00, 0x00, 0x96]);
    assert_eq!(&out[4..8], &[0x41, 0x00, 0x2E, 0x4A]);
    assert_eq!(&out[396..400], &[0x01, 0x01, 0x00, 0x00]);
    assert_eq!(&out[400..404], &[0x41, 0x02, 0x00, 0x00]);
}

#[test]
fn gdi_boot_area_keeps_track_3_and_the_45000_fallback() {
    let mk = |n, lba| TrackDesc {
        number: n,
        start_lba: lba,
        ctrl: 4,
        sector_size: 2352,
        sector_count: 1,
        filename: Default::default(),
        file_offset: 0,
    };
    assert_eq!(gdi_boot_area_lba(&[mk(1, 0), mk(2, 450), mk(3, 45000)]), 45000);
    // Non-standard HDA start is honoured.
    assert_eq!(gdi_boot_area_lba(&[mk(1, 0), mk(2, 450), mk(3, 50000)]), 50000);
    // Two-track GDI (no HDA): unchanged fallback.
    assert_eq!(gdi_boot_area_lba(&[mk(1, 0), mk(2, 450)]), 45000);
}
