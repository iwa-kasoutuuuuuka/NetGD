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
    f.extend_from_slice(&0x8000_0006u32.to_le_bytes());
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
        session: 0,
    };
    assert_eq!(gdi_boot_area_lba(&[mk(1, 0), mk(2, 450), mk(3, 45000)]), 45000);
    // Non-standard HDA start is honoured.
    assert_eq!(gdi_boot_area_lba(&[mk(1, 0), mk(2, 450), mk(3, 50000)]), 50000);
    // Two-track GDI (no HDA): unchanged fallback.
    assert_eq!(gdi_boot_area_lba(&[mk(1, 0), mk(2, 450)]), 45000);
}

// ---- 3+ sessions: REQ_SES(n) is session n's own first track -------------

#[test]
fn three_sessions_middle_session_is_its_own_first_track() {
    // s1: audio (data LBA 0). s2: data, descriptor start 11700 (pregap 150).
    // s3: data, descriptor start 40000 (pregap 150) = the boot session.
    let d3: T = (150, 5000, 40000, 5150, 1, 4);
    let (n, bt, boot, t) = load(&[vec![AUDIO], vec![DATA2], vec![d3]]);
    assert_eq!((n, bt, boot), (3, 3, 40000));
    assert_eq!(t.iter().map(|x| x.session).collect::<Vec<_>>(), [1, 2, 3]);
    assert_eq!(req_ses_reply(&t, true, boot, n, 1), Some([1, 0, 1, 0x00, 0x00, 0x96]));
    // session 2 = track 2 at FAD 11850 (0x2E4A), NOT the boot track
    assert_eq!(req_ses_reply(&t, true, boot, n, 2), Some([1, 0, 2, 0x00, 0x2E, 0x4A]));
    // session 3 = track 3 at FAD 40150 = 0x9CD6
    assert_eq!(req_ses_reply(&t, true, boot, n, 3), Some([1, 0, 3, 0x00, 0x9C, 0xD6]));
    assert_eq!(req_ses_reply(&t, true, boot, n, 4), None);
    // the boot area is still the first track of the last session
    assert_eq!(boot, t[2].start_lba);
}

#[test]
fn middle_session_with_several_tracks_reports_its_first() {
    let a: T = (150, 1000, 11700, 1150, 2, 0);
    let b: T = (150, 1000, 12850, 1150, 2, 0);
    let c: T = (150, 3000, 30000, 3150, 1, 4);
    let (n, bt, boot, t) = load(&[vec![AUDIO], vec![a, b], vec![c]]);
    assert_eq!((n, bt), (3, 4));
    assert_eq!(t.iter().map(|x| x.session).collect::<Vec<_>>(), [1, 2, 2, 3]);
    // session 2 = track 2 (not track 3 and not the boot track 4), FAD 11850
    assert_eq!(req_ses_reply(&t, true, boot, n, 2), Some([1, 0, 2, 0x00, 0x2E, 0x4A]));
    assert_eq!(req_ses_reply(&t, true, boot, n, 3), Some([1, 0, 4, 0x00, 0x75, 0xC6])); // 30000+150
}

#[test]
fn tracks_without_session_numbers_keep_the_boot_fallback() {
    // A firmware track type that does not implement `session()` (default 0).
    let d3: T = (150, 5000, 40000, 5150, 1, 4);
    let (n, _, boot, mut t) = load(&[vec![AUDIO], vec![DATA2], vec![d3]]);
    for x in t.iter_mut() {
        x.session = 0;
    }
    assert_eq!(req_ses_reply(&t, true, boot, n, 2), Some([1, 0, 3, 0x00, 0x9C, 0xD6]));
}

// ---- Track FAD = descriptor start + pregap (Flycast `StartFAD`) ----------

#[test]
fn track_fad_is_start_plus_pregap_with_odd_pregaps() {
    // Track 1: pregap 150, data 0..1000. Track 2: pregap 0 (DiscJuggler
    // style), descriptor start 1150: Flycast StartFAD = 1150 + 0 = 1150, so
    // LBA 1000 (right after track 1's data). Track 3 in session 2: pregap 300,
    // descriptor start 5000: StartFAD = 5300 -> LBA 5150.
    let t1: T = (150, 1000, 0, 1150, 2, 0);
    let t2: T = (0, 500, 1150, 500, 2, 0);
    let t3: T = (300, 2000, 5000, 2300, 1, 4);
    let (n, bt, boot, t) = load(&[vec![t1, t2], vec![t3]]);
    assert_eq!((n, bt), (2, 3));
    assert_eq!([t[0].start_lba, t[1].start_lba, t[2].start_lba], [0, 1000, 5150]);
    assert_eq!(boot, 5150);
    // Reported FADs are Flycast's StartFAD values.
    assert_eq!(req_ses_reply(&t, true, boot, n, 1), Some([1, 0, 1, 0x00, 0x00, 0x96]));
    assert_eq!(req_ses_reply(&t, true, boot, n, 2), Some([1, 0, 3, 0x00, 0x14, 0xB4])); // 5300
    let mut toc = [0u8; 408];
    build_toc(&t[..], true, boot, 0, &mut toc);
    assert_eq!(&toc[4..8], &[0x01, 0x00, 0x04, 0x7E]); // track 2: FAD 1150
    assert_eq!(&toc[8..12], &[0x41, 0x00, 0x14, 0xB4]); // track 3: FAD 5300
}

// ---- Which track IP.BIN is looked for in (`ip_bin_candidates`) ----------

fn mk(number: u8, start_lba: u32, ctrl: u8) -> TrackDesc {
    TrackDesc {
        number,
        start_lba,
        ctrl,
        sector_size: 2352,
        sector_count: 1,
        filename: Default::default(),
        file_offset: 0,
        session: 0,
    }
}

fn order(tracks: &[TrackDesc], boot: u32) -> Vec<u8> {
    ip_bin_candidates(tracks, boot).map(|t| t.number).collect()
}

/// What the firmware's `read_ip_header` did before: data tracks starting at
/// LBA 11700 or later first, then the rest. Kept here to pin down where the
/// two agree and where they do not.
fn old_order(tracks: &[TrackDesc]) -> Vec<u8> {
    let data = |t: &&TrackDesc| t.ctrl & 4 != 0;
    tracks
        .iter()
        .filter(data)
        .filter(|t| t.start_lba >= 11700)
        .chain(tracks.iter().filter(data).filter(|t| t.start_lba < 11700))
        .map(|t| t.number)
        .collect()
}

#[test]
fn ip_bin_gdi_is_track_3_as_before() {
    let t = [mk(1, 0, 4), mk(2, 450, 0), mk(3, 45000, 4), mk(4, 60000, 0), mk(5, 70000, 4)];
    let boot = gdi_boot_area_lba(&t);
    assert_eq!(order(&t, boot), [3, 5, 1]);
    assert_eq!(order(&t, boot), old_order(&t));
    // The first candidate is the track the boot prefetch reads.
    assert_eq!(ip_bin_candidates(&t, boot).next().unwrap().start_lba, boot);

    // Two tracks, no high-density area: boot area is the 45000 fallback,
    // nothing is up there, the data track below is the only candidate.
    let t = [mk(1, 0, 4), mk(2, 450, 0)];
    assert_eq!(order(&t, gdi_boot_area_lba(&t)), [1]);
    assert_eq!(old_order(&t), [1]);
}

#[test]
fn ip_bin_cdi_cases_that_did_not_change() {
    // One data track at 0.
    let (_, _, boot, t) = load(&[vec![(150, 5000, 0, 5150, 1, 4)]]);
    assert_eq!((boot, order(&t, boot), old_order(&t)), (0, vec![1], vec![1]));

    // Audio + data at 11700.
    let (_, _, boot, t) = load(&[vec![AUDIO], vec![DATA2]]);
    assert_eq!((boot, order(&t, boot), old_order(&t)), (11700, vec![2], vec![2]));

    // Audio + data whose descriptor start is 11702 (302-sector first session).
    let (_, _, boot, t) =
        load(&[vec![(150, 302, 0, 452, 2, 0)], vec![(150, 20000, 11702, 20150, 1, 4)]]);
    assert_eq!((boot, order(&t, boot), old_order(&t)), (11702, vec![2], vec![2]));

    // Data + data: the second session first, the first as a fallback.
    let (_, _, boot, t) = load(&[vec![(150, 3000, 0, 3150, 1, 4)], vec![DATA2]]);
    assert_eq!((boot, order(&t, boot), old_order(&t)), (11700, vec![2, 1], vec![2, 1]));

    // Three-track MIL-CD, data at 45000.
    let a2: T = (0, 7000, 11150, 7000, 2, 0);
    let (_, _, boot, t) = load(&[vec![AUDIO, a2], vec![(150, 20000, 45000, 20150, 1, 4)]]);
    assert_eq!((boot, order(&t, boot), old_order(&t)), (45000, vec![3], vec![3]));

    // One session, audio audio data: the boot area is the session start (0)
    // and the only data track is the candidate either way.
    let a2: T = (150, 1000, 11150, 1150, 2, 0);
    let (_, _, boot, t) = load(&[vec![AUDIO, a2, (150, 2000, 12300, 2150, 1, 4)]]);
    assert_eq!((boot, order(&t, boot), old_order(&t)), (0, vec![3], vec![3]));
}

#[test]
fn ip_bin_three_sessions_prefers_the_last_not_the_first_above_11700() {
    let d2: T = (150, 5000, 11700, 5150, 1, 4);
    let d3: T = (150, 20000, 28250, 20150, 1, 4);
    let (n, bt, boot, t) = load(&[vec![AUDIO], vec![d2], vec![d3]]);
    assert_eq!((n, bt, boot), (3, 3, 28250));
    assert_eq!(order(&t, boot), [3, 2]);
    // The old rule took the middle session's track if it was signed, while
    // the BIOS (and the boot prefetch) go to the last.
    assert_eq!(old_order(&t), [2, 3]);
}

#[test]
fn ip_bin_second_session_below_11700_is_still_the_boot_track() {
    // A first session shorter than the 300 sectors DiscJuggler pads to puts
    // the second below 11700: 100 + 11400 = 11500.
    let d1: T = (150, 100, 0, 250, 1, 4);
    let d2: T = (150, 20000, 11500, 20150, 1, 4);
    let (_, bt, boot, t) = load(&[vec![d1], vec![d2]]);
    assert_eq!((bt, boot), (2, 11500));
    assert_eq!(order(&t, boot), [2, 1]);
    // The old rule saw no track at 11700 or later and started with track 1.
    assert_eq!(old_order(&t), [1, 2]);
}

#[test]
fn ip_bin_never_offers_an_audio_track() {
    // A boot area that starts with audio (not bootable, but not a reason to
    // hand back PCM as a header).
    let t = [mk(1, 0, 0), mk(2, 11700, 0), mk(3, 20000, 4)];
    assert_eq!(order(&t, 11700), [3]);
    assert!(order(&[mk(1, 0, 0)], 0).is_empty());
    assert!(order(&[], 0).is_empty());
}
