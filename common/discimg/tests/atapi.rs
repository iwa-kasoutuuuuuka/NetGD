//! Characterization tests for the ATAPI reply builders: exact bytes, derived
//! by hand from the original firmware code, plus a sweep against a verbatim
//! copy of that code (tests/oracle).

mod oracle;

use discimg::atapi::*;
use discimg::TrackDesc;

fn t(number: u8, start_lba: u32, ctrl: u8, sector_count: u32) -> TrackDesc {
    TrackDesc {
        number,
        start_lba,
        ctrl,
        sector_size: 2352,
        sector_count,
        filename: Default::default(),
        file_offset: 0,
        session: 0,
        mode: discimg::track_mode(ctrl, None, 2352),
    }
}

/// 3-track GD-ROM: audio SDA track pair would be 1-2, data HDA track 3.
fn gd3() -> Vec<TrackDesc> {
    vec![t(1, 0, 4, 450), t(2, 450, 0, 10950), t(3, 45000, 4, 504_150)]
}

/// MIL-CD: two audio tracks, then the data track as session 2.
fn mil() -> Vec<TrackDesc> {
    vec![t(1, 0, 0, 3000), t(2, 3150, 0, 7000), t(3, 45000, 4, 20000)]
}

fn toc(tracks: &[TrackDesc], is_cdi: bool, boot: u32, area: u8) -> [u8; 408] {
    let mut out = [0u8; 408];
    build_toc(tracks, is_cdi, boot, area, &mut out);
    out
}

/// Expected TOC: FF fill, with the given `(offset, bytes)` patches.
fn expect(patches: &[(usize, [u8; 4])]) -> [u8; 408] {
    let mut e = [0xFFu8; 408];
    for (at, b) in patches {
        e[*at..*at + 4].copy_from_slice(b);
    }
    e
}

#[test]
fn bcd_and_msf() {
    assert_eq!(bcd(0), 0x00);
    assert_eq!(bcd(9), 0x09);
    assert_eq!(bcd(10), 0x10);
    assert_eq!(bcd(59), 0x59);
    assert_eq!(bcd(99), 0x99);
    // Out of range: not clamped; tens digit >= 10 spills into hex nibbles.
    assert_eq!(bcd(100), 0xA0);
    assert_eq!(bcd(123), 0xC3);
    let mut m = [0xEEu8; 4];
    put_msf(&mut m, 0);
    assert_eq!(m, [0, 0, 0, 0xEE]);
    put_msf(&mut m, 150); // 2 s
    assert_eq!(&m[..3], &[0x00, 0x02, 0x00]);
    put_msf(&mut m, 4500 + 75 * 3 + 7); // 1:03:07
    assert_eq!(&m[..3], &[0x01, 0x03, 0x07]);
    put_msf(&mut m, 74 * 4500 + 59 * 75 + 74); // 74:59:74
    assert_eq!(&m[..3], &[0x74, 0x59, 0x74]);
}

#[test]
fn toc_gdrom_sda_and_hda() {
    let g = gd3();
    // SDA: tracks 1, 2. Lead-out = end of track 2 + 150 = 11550 = 0x002D1E.
    assert_eq!(
        toc(&g, false, 45000, 0),
        expect(&[
            (0, [0x41, 0x00, 0x00, 0x96]),
            (4, [0x01, 0x00, 0x02, 0x58]),
            (396, [0x41, 0x01, 0x00, 0x00]),
            (400, [0x01, 0x02, 0x00, 0x00]),
            (404, [0x01, 0x00, 0x2D, 0x1E]),
        ])
    );
    // HDA: track 3 only. Lead-out = end of track 3 = 45000 + 504150 + 150 =
    // 549300 = 0x0861B4 (a full-size track, so the same as the old padding).
    assert_eq!(
        toc(&g, false, 45000, 1),
        expect(&[
            (8, [0x41, 0x00, 0xB0, 0x5E]),
            (396, [0x41, 0x03, 0x00, 0x00]),
            (400, [0x41, 0x03, 0x00, 0x00]),
            (404, [0x41, 0x08, 0x61, 0xB4]),
        ])
    );
}

#[test]
fn toc_hda_leadout_is_the_end_of_the_last_track_like_the_sda() {
    // A trimmed dump: track 3 file is only 1000 sectors. Stock GDEMU 6.0
    // reports the real end (D1); this used to be padded to 549300.
    let mut g = gd3();
    g[2].sector_count = 1000;
    let hda = toc(&g, false, 45000, 1);
    assert_eq!(&hda[404..408], &[0x41, 0x00, 0xB4, 0x46]); // 45000+1000+150 = 46150
    // Same trimming on the SDA side is reported as-is.
    g[1].sector_count = 100;
    let sda = toc(&g, false, 45000, 0);
    assert_eq!(&sda[404..408], &[0x01, 0x00, 0x02, 0xBC]); // 450+100+150 = 700
}

#[test]
fn toc_unknown_area_numbers_behave_like_hda() {
    // Anything but 0 selects tracks >= boot_area_lba (area 1 no longer pads
    // the lead-out, D1, so the two are identical).
    let mut g = gd3();
    g[2].sector_count = 1000;
    let a2 = toc(&g, false, 45000, 2);
    assert_eq!(&a2[8..12], &[0x41, 0x00, 0xB0, 0x5E]);
    assert_eq!(&a2[404..408], &[0x41, 0x00, 0xB4, 0x46]); // 45000+1000+150 = 46150
    assert_eq!(a2, toc(&g, false, 45000, 1));
}

#[test]
fn toc_mil_cd_has_everything_in_area_0_and_an_exact_leadout() {
    let m = mil();
    assert_eq!(
        toc(&m, true, 45000, 0),
        expect(&[
            (0, [0x01, 0x00, 0x00, 0x96]),
            (4, [0x01, 0x00, 0x0C, 0xE4]), // 3300
            (8, [0x41, 0x00, 0xB0, 0x5E]),
            (396, [0x01, 0x01, 0x00, 0x00]),
            (400, [0x41, 0x03, 0x00, 0x00]),
            (404, [0x41, 0x00, 0xFE, 0x7E]), // 45000+20000+150 = 65150
        ])
    );
    // Area 1 of a CDI is the same TOC (stock GDEMU 6.0, R117: GET_TOC_1 of the
    // dcload-ip CDI repeats area 0, lead-out included).
    assert_eq!(toc(&m, true, 45000, 1), toc(&m, true, 45000, 0));
    assert_eq!(toc(&m, true, 45000, 2), toc(&m, true, 45000, 0));
}

#[test]
fn toc_empty_or_missing_area_is_all_ff() {
    assert_eq!(toc(&[] as &[TrackDesc], false, 45000, 0), [0xFF; 408]);
    // Only SDA tracks, ask for the HDA.
    let g = vec![t(1, 0, 4, 100)];
    assert_eq!(toc(&g, false, 45000, 1), [0xFF; 408]);
}

#[test]
fn toc_skips_out_of_range_track_numbers_and_keeps_first_ctrl_quirk() {
    // Track numbers 0 and > 99 are dropped. The first-track entry takes its
    // CTRL from the first track *in the whole table* with that number, which
    // can differ from the one placed in the area.
    let g = vec![t(0, 0, 4, 10), t(100, 10, 4, 10), t(1, 20, 4, 10), t(1, 50000, 0, 10)];
    let a1 = toc(&g, false, 45000, 1);
    assert_eq!(&a1[0..4], &[0x01, 0x00, 0xC3, 0xE6]); // dup number 1 at 50000: slot 0 rewritten
    assert_eq!(&a1[396..400], &[0x41, 0x01, 0x00, 0x00]); // ctrl 4 from the *first* track 1
    assert_eq!(&a1[400..404], &[0x01, 0x01, 0x00, 0x00]);
}

#[test]
fn leadout() {
    // GDI and CDI alike: the end of the last track (stock GDEMU 6.0, D1).
    assert_eq!(leadout_lba(&gd3(), false), 549150);
    let mut g = gd3();
    g[2].sector_count = 20000;
    assert_eq!(leadout_lba(&g, false), 65000);
    assert_eq!(leadout_lba(&mil(), true), 65000);
    // An (impossible) empty table falls back to the GD-ROM capacity.
    assert_eq!(leadout_lba(&[] as &[TrackDesc], true), 549150);
    assert_eq!(leadout_lba(&[] as &[TrackDesc], false), 549150);
}

#[test]
fn req_ses_gdrom() {
    let g = gd3();
    // session count 1, lead-out 549150 + 150 = 549300 = 0x0861B4
    assert_eq!(req_ses_reply(&g, false, 45000, 1, 0, DRIVE_STANDBY), Some([0x02, 0x00, 0x01, 0x08, 0x61, 0xB4]));
    assert_eq!(req_ses_reply(&g, false, 45000, 1, 1, DRIVE_STANDBY), Some([0x02, 0x00, 0x01, 0x00, 0x00, 0x96]));
    // GD-ROM reports one session only.
    assert_eq!(req_ses_reply(&g, false, 45000, 1, 2, DRIVE_STANDBY), None);
    assert_eq!(req_ses_reply(&g, false, 45000, 1, 3, DRIVE_STANDBY), None);
}

#[test]
fn req_ses_mil_cd() {
    let m = mil();
    assert_eq!(req_ses_reply(&m, true, 45000, 2, 0, DRIVE_STANDBY), Some([0x02, 0x00, 0x02, 0x00, 0xFE, 0x7E]));
    assert_eq!(req_ses_reply(&m, true, 45000, 2, 1, DRIVE_STANDBY), Some([0x02, 0x00, 0x01, 0x00, 0x00, 0x96]));
    assert_eq!(req_ses_reply(&m, true, 45000, 2, 2, DRIVE_STANDBY), Some([0x02, 0x00, 0x03, 0x00, 0xB0, 0x5E]));
    assert_eq!(req_ses_reply(&m, true, 45000, 2, 3, DRIVE_STANDBY), None);
}

#[test]
fn req_ses_two_track_cdi_session_2_is_the_data_track() {
    // This used to pin ABRT (the firmware fell back to boot_area_lba = 45000
    // for a CDI with fewer than 3 tracks, so the data track at 11700 was not
    // "at or above" it). The firmware now passes the start of the last
    // session's first track (11700); with a stale 45000 the lookup still
    // finds nothing, which is why the caller must not guess.
    let c = vec![t(1, 0, 0, 11000), t(2, 11700, 4, 20000)];
    assert_eq!(req_ses_reply(&c, true, 45000, 2, 2, DRIVE_STANDBY), None);
    // boot_area_lba = the data track's start finds it.
    assert_eq!(req_ses_reply(&c, true, 11700, 2, 2, DRIVE_STANDBY), Some([0x02, 0x00, 0x02, 0x00, 0x2E, 0x4A]));
}

#[test]
fn req_ses_no_sda_track() {
    let g = vec![t(3, 45000, 4, 100)];
    assert_eq!(req_ses_reply(&g, false, 45000, 1, 1, DRIVE_STANDBY), None);
    // Session 0 does not need any track.
    assert_eq!(req_ses_reply(&[] as &[TrackDesc], false, 45000, 1, 0, DRIVE_STANDBY), Some([2, 0, 1, 0x08, 0x61, 0xB4]));
}

#[test]
fn head_position_idle_is_fad_150_in_track_1() {
    let g = gd3();
    let (fad, tr) = head_position(&g, AudioState::Standby, 12345, 150);
    assert_eq!(fad, 150);
    assert_eq!(tr.unwrap().number, 1);
    // Playing at FAD 1000 -> LBA 850 -> track 2 (starts at 450).
    let (fad, tr) = head_position(&g, AudioState::Playing, 1000, 150);
    assert_eq!((fad, tr.unwrap().number), (1000, 2));
    // Before the first track (FAD < 150 saturates to LBA 0), and past the end.
    let (_, tr) = head_position(&g, AudioState::Paused, 10, 150);
    assert_eq!(tr.unwrap().number, 1);
    let (_, tr) = head_position(&g, AudioState::Paused, 600_000, 150);
    assert_eq!(tr.unwrap().number, 3);
    // No tracks.
    assert!(head_position(&[] as &[TrackDesc], AudioState::Playing, 5, 150).1.is_none());
}

#[test]
fn req_stat() {
    let g = gd3();
    let (fad, tr) = head_position(&g, AudioState::Standby, 777, 150);
    assert_eq!(
        req_stat_reply(AudioState::Standby, DRIVE_STANDBY, DISC_FORMAT_GDROM, 0, fad, tr),
        [0x02, 0x8F, 0x41, 0x01, 0x01, 0x00, 0x00, 0x96, 0x00, 0x00]
    );
    let (fad, tr) = head_position(&g, AudioState::Playing, 1000, 150);
    assert_eq!(
        req_stat_reply(AudioState::Playing, DRIVE_STANDBY, DISC_FORMAT_CDROM_XA, 3, fad, tr),
        [0x03, 0x23, 0x01, 0x02, 0x01, 0x00, 0x03, 0xE8, 0x00, 0x00]
    );
    for (s, code) in [
        (AudioState::Playing, 3),
        (AudioState::Paused, 1),
        (AudioState::Finished, 1),
        (AudioState::Error, 9),
        (AudioState::Standby, 2),
    ] {
        assert_eq!(req_stat_reply::<TrackDesc>(s, DRIVE_STANDBY, 0, 0, 0, None)[0], code);
        assert_eq!(drive_status(s, DRIVE_STANDBY), code);
    }
    // No track under the head: CTRL/ADR 0x41, track 1; FAD is full 24-bit BE.
    // Paused: the repeat nibble is F whatever the count (stock, D3).
    assert_eq!(
        req_stat_reply::<TrackDesc>(AudioState::Paused, DRIVE_STANDBY, 0, 9, 0x01_02_03, None),
        [0x01, 0x0F, 0x41, 0x01, 0x01, 0x01, 0x02, 0x03, 0x00, 0x00]
    );
    // Byte 1: disc format high nibble, repeat count low while playing (the
    // firmware's repeat is 4 bits; anything above is masked so it cannot
    // spill into the format).
    assert_eq!(req_stat_reply::<TrackDesc>(AudioState::Playing, DRIVE_STANDBY, DISC_FORMAT_CDROM_XA, 0x13, 0, None)[1], 0x23);
    // Not playing: F (stock GDEMU 6.0 sends 2f for a CDI and 8f for a GDI in
    // standby and pause, records 02-07; there is no stock record of playback).
    for s in [AudioState::Standby, AudioState::Paused, AudioState::Finished, AudioState::Error] {
        for rep in [0u8, 3, 0x0F] {
            assert_eq!(req_stat_reply::<TrackDesc>(s, DRIVE_STANDBY, DISC_FORMAT_GDROM, rep, 0, None)[1], 0x8F);
            assert_eq!(req_stat_reply::<TrackDesc>(s, DRIVE_STANDBY, DISC_FORMAT_CDROM_XA, rep, 0, None)[1], 0x2F);
        }
    }
}

#[test]
fn get_scd_format_1_is_14_bytes() {
    let g = gd3();
    let (fad, tr) = head_position(&g, AudioState::Playing, 1000, 150);
    let mut r = [0xEEu8; 100];
    assert_eq!(get_scd_reply(1, AudioState::Playing, fad, tr, &mut r), 14);
    // Q: ctrl/adr 01, track 02, index 01, rel FAD 400 (1000-600) = 0x000190, 00,
    // abs FAD 1000 = 0x0003E8 -- binary, not BCD MSF (D-2; stock GDEMU 6.0, R117).
    assert_eq!(
        &r[..14],
        &[0x00, 0x11, 0x00, 0x0E, 0x01, 0x02, 0x01, 0x00, 0x01, 0x90, 0x00, 0x00, 0x03, 0xE8]
    );
    // The rest of the buffer is zeroed, not left alone.
    assert!(r[14..].iter().all(|&b| b == 0));
}

#[test]
fn get_scd_idle_and_trackless() {
    let g = gd3();
    let (fad, tr) = head_position(&g, AudioState::Standby, 0, 150);
    let mut r = [0u8; 100];
    assert_eq!(get_scd_reply(1, AudioState::Standby, fad, tr, &mut r), 14);
    // Track 1, data; relative 0; absolute FAD 150 = 0x000096.
    assert_eq!(
        &r[..14],
        &[0x00, 0x15, 0x00, 0x0E, 0x41, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x96]
    );
    // No track: ctrl/adr 0x41, track and relative time left zero.
    assert_eq!(get_scd_reply::<TrackDesc>(1, AudioState::Paused, 150, None, &mut r), 14);
    assert_eq!(
        &r[..14],
        &[0x00, 0x12, 0x00, 0x0E, 0x41, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x96]
    );
    for (s, code) in [
        (AudioState::Playing, 0x11),
        (AudioState::Paused, 0x12),
        (AudioState::Finished, 0x13),
        (AudioState::Error, 0x14),
        (AudioState::Standby, 0x15),
    ] {
        get_scd_reply::<TrackDesc>(1, s, 150, None, &mut r);
        assert_eq!(r[1], code);
    }
}

/// CRC-16/XMODEM, written independently (byte-wise) of the firmware's
/// bit-at-a-time loop.
fn xmodem(data: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

#[test]
fn the_independent_crc_matches_the_standard_check_value() {
    assert_eq!(xmodem(b"123456789"), 0x31C3);
}

#[test]
fn get_scd_format_0_is_100_bytes_with_bit_expanded_q_and_inverted_crc() {
    let g = gd3();
    let (fad, tr) = head_position(&g, AudioState::Playing, 1000, 150);
    let mut r = [0u8; 100];
    assert_eq!(get_scd_reply(0, AudioState::Playing, fad, tr, &mut r), 100);
    assert_eq!(&r[..4], &[0x00, 0x11, 0x00, 100]);
    // Raw Q (MSF in BCD, as on the disc): rel 00:05:25, abs 00:13:25, then the
    // inverted CRC of Q[0..10]. Each bit is one byte: Q is bit 6 (0x40), P and
    // R..W are 0 (stock GDEMU 6.0, R117; this used to be 0x7F / 0x3F).
    let q10 = [0x01, 0x02, 0x01, 0x00, 0x05, 0x25, 0x00, 0x00, 0x13, 0x25];
    let crc = !xmodem(&q10);
    let mut q = q10.to_vec();
    q.extend_from_slice(&crc.to_be_bytes());
    let mut want = Vec::new();
    for b in &q {
        for bit in 0..8 {
            want.push(if (b >> (7 - bit)) & 1 != 0 { 0x40 } else { 0x00 });
        }
    }
    assert_eq!(&r[4..], &want[..]);
    // Golden value, so a change to the CRC shows up even if both sides agree.
    assert_eq!(crc, GOLDEN_CRC);
}

const GOLDEN_CRC: u16 = 0x5B0B;

#[test]
fn sweep_matches_the_original_code() {
    use oracle::{Gdi, Snapshot, Status};
    let mut seed: u64 = 0x1234_5678_9abc_def0;
    let mut rnd = move |n: u64| -> u64 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) % n
    };
    let states = [
        (AudioState::Playing, Status::Playing),
        (AudioState::Paused, Status::Paused),
        (AudioState::Finished, Status::Finished),
        (AudioState::Error, Status::Error),
        (AudioState::Standby, Status::Standby),
    ];
    for _ in 0..3000 {
        let n = rnd(9) as usize;
        let is_cdi = rnd(2) == 1;
        let mut new = Vec::new();
        let mut old = heapless::Vec::<oracle::Track, 99>::new();
        let mut lba = 0u32;
        for i in 0..n {
            lba += rnd(60000) as u32;
            let number = if rnd(8) == 0 { rnd(120) as u8 } else { (i + 1) as u8 };
            let ctrl = rnd(16) as u8;
            let sc = rnd(600000) as u32;
            new.push(t(number, lba, ctrl, sc));
            old.push(oracle::Track {
                number, start_lba: lba, ctrl, sector_size: 2352, sector_count: sc, filename: Default::default(),
            }).unwrap();
        }
        let boot = if rnd(2) == 0 { 45000 } else { rnd(100000) as u32 };
        let sessions = rnd(3) as u8;
        let og = Gdi { tracks: old, boot_area_lba: boot, is_cdi, sessions };

        for area in 0..3u8 {
            // A CDI answers every area with its one TOC (R117); the original
            // code gave area 0 only and FF for the rest.
            // A GDI's area 1 lead-out is no longer padded to 549300 (D1):
            // the original's area 2 is the same selection unpadded.
            let mut o = [0u8; 408];
            og.build_toc(if is_cdi { 0 } else if area == 1 { 2 } else { area }, &mut o);
            assert_eq!(toc(&new, is_cdi, boot, area), o);
        }
        for s in 0..5u8 {
            // The oracle is the ORIGINAL code, which is wrong for CDI sessions
            // 1 and 2 (it keyed both on `boot_area_lba` alone, so a CDI whose
            // data track sits below it, or a single-session CDI, got ABRT).
            // CDI session 0 and all GDI replies must still match; the CDI
            // cases are pinned in tests/cdi_sessions.rs.
            if !is_cdi || s == 0 {
                // Byte 0 is now the drive status (R117); the original code
                // always put 0x01 there.
                let st = rnd(16) as u8;
                let want = oracle::build_req_ses_reply(Some(&og), s).map(|mut r| {
                    r[0] = st;
                    // Session 0 of a GDI: lead-out = end of the last track
                    // (D1), as the original already did for a CDI.
                    if s == 0 && !is_cdi {
                        let lo = end_of_last(&og).wrapping_add(150);
                        r[3..6].copy_from_slice(&lo.to_be_bytes()[1..]);
                    }
                    r
                });
                assert_eq!(req_ses_reply(&new, is_cdi, boot, sessions, s, st), want);
            }
        }
        assert_eq!(leadout_lba(&new, is_cdi), end_of_last(&og));

        for &(ns, os) in &states {
            let fad = if rnd(4) == 0 { rnd(400) as u32 } else { rnd(700000) as u32 };
            let repeat = rnd(256) as u8;
            let snap = Snapshot { status: os, repeat, fad };
            let (hf, ht) = head_position(&new, ns, fad, 150);
            let (of, ot) = oracle::head_position(&snap, Some(&og));
            assert_eq!(hf, of);
            assert_eq!(ht.map(|t| t.number), ot.map(|t| t.number));
            // Byte 1 now carries the disc format in its high nibble (D-3,
            // R117); the original code put the raw repeat count there.
            let fmt = [0u8, DISC_FORMAT_CDROM_XA, DISC_FORMAT_GDROM][rnd(3) as usize];
            let mut want = oracle::build_req_stat_reply(&snap, Some(&og));
            // Repeat nibble F unless playing (D3).
            let rep = if ns == AudioState::Playing { want[1] & 0x0F } else { 0x0F };
            want[1] = fmt | rep;
            assert_eq!(req_stat_reply(ns, DRIVE_STANDBY, fmt, repeat, hf, ht), want);
            for format in [0u8, 1, 2] {
                let mut a = [0xCCu8; 100];
                let mut b = [0xDDu8; 100];
                let la = get_scd_reply(format, ns, hf, ht, &mut a);
                let lb = oracle::build_get_scd_reply(format, &snap, Some(&og), &mut b);
                assert_eq!(la, lb);
                let mut want = b;
                if format == 1 {
                    // Positions are binary FADs now (D-2, R117); the rest of
                    // the reply is the original's.
                    let rel = match ht {
                        Some(t) => hf.saturating_sub(t.start_lba.wrapping_add(150)),
                        None => 0,
                    };
                    want[7..10].copy_from_slice(&rel.to_be_bytes()[1..]);
                    want[11..14].copy_from_slice(&hf.to_be_bytes()[1..]);
                } else {
                    // Bit-expanded Q: 1 -> 0x40, 0 -> 0x00 instead of 0x7F / 0x3F.
                    for x in &mut want[4..] {
                        *x = if *x == 0x7F { 0x40 } else { 0x00 };
                    }
                }
                assert_eq!(a, want);
            }
        }
    }
}

/// The original code's CDI lead-out rule, which now applies to a GDI too:
/// end of the last track, 549150 for an empty table.
fn end_of_last(g: &oracle::Gdi) -> u32 {
    g.tracks.last().map(|t| t.start_lba + t.sector_count).unwrap_or(549150)
}

#[test]
fn req_ses_byte_0_is_the_drive_status() {
    // Stock GDEMU 6.0 answered 0x02 (standby) right after REQ_STAT said
    // standby (R117). The older g1bios_replay capture saw 0x01, presumably
    // with the drive paused by the BIOS's CD_SEEK -- same rule.
    let g = gd3();
    for st in [DRIVE_PAUSE, DRIVE_STANDBY, DRIVE_PLAY] {
        assert_eq!(req_ses_reply(&g, false, 45000, 1, 1, st).unwrap()[0], st);
    }
}

#[test]
fn req_mode_is_a_window_into_the_stock_32_byte_block() {
    // Stock GDEMU 6.0 (R117): REQ_MODE (0, 32).
    let stock = hex("0000000000b419000008534520202020202052657620362e3432393930333136");
    assert_eq!(&REQ_MODE_BLOCK[..], &stock[..]);
    assert_eq!(req_mode_reply(0, 32), Some(&stock[..]));
    assert_eq!(req_mode_reply(0, 10), Some(&stock[..10]));
    assert_eq!(req_mode_reply(18, 8), Some(&b"Rev 6.42"[..]));
    assert_eq!(req_mode_reply(10, 8), Some(&b"SE      "[..]));
    // Outside the block, or empty: not answered (the caller aborts as before).
    assert_eq!(req_mode_reply(0, 33), None);
    assert_eq!(req_mode_reply(31, 2), None);
    assert_eq!(req_mode_reply(32, 1), None);
    assert_eq!(req_mode_reply(5, 0), None);
    assert_eq!(req_mode_reply(255, 255), None);
}

#[test]
fn req_error_reply_layout() {
    // Stock GDEMU 6.0 with nothing pending (R117): F0 then zeros.
    assert_eq!(req_error_reply(0, 0, 0), [0xF0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    // Sense key in the low nibble of byte 2 (SPI; gdprobe reads it there),
    // ASC/ASCQ in bytes 8/9 (D7).
    assert_eq!(req_error_reply(0x05, 0, 0), [0xF0, 0, 0x05, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(req_error_reply(0x16, 0, 0), [0xF0, 0, 0x06, 0, 0, 0, 0, 0, 0, 0]);
    // The other sense triples stock reports (records 03-07): becoming ready,
    // medium changed, illegal mode for this track.
    assert_eq!(req_error_reply(2, ASC_BECOMING_READY.0, ASC_BECOMING_READY.1), [0xF0, 0, 2, 0, 0, 0, 0, 0, 0x04, 0x01]);
    assert_eq!(req_error_reply(6, ASC_MEDIUM_CHANGED.0, ASC_MEDIUM_CHANGED.1), [0xF0, 0, 6, 0, 0, 0, 0, 0, 0x28, 0x00]);
    assert_eq!(req_error_reply(5, 0x64, 0x00), [0xF0, 0, 5, 0, 0, 0, 0, 0, 0x64, 0x00]);
}

#[test]
fn req_error_no_medium_is_the_stock_bytes() {
    // stock_20261121_04_img3-7.txt img=6 REQ_ERROR_W0 (TUR said NOT READY,
    // empty image slot): 2/3A/00.
    let (asc, ascq) = ASC_NO_MEDIUM;
    assert_eq!(&req_error_reply(2, asc, ascq)[..], &hex("f0000200000000003a00")[..]);
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

/// The dcload-ip CDI the stock GDEMU 6.0 had mounted in R117
/// (`docs/research/probe-records/stock_20261121_02.txt`, img=0): track 1 audio
/// at FAD 150 (session 1), track 2 data at FAD 11852 (session 2), lead-out
/// FAD 0x05017C. Only what the TOC shows matters; track 1's length is a guess.
fn dcload_cdi() -> Vec<TrackDesc> {
    let mut a = t(1, 0, 0, 11552);
    a.session = 1;
    let mut d = t(2, 11702, 4, 0x05017C - 150 - 11702);
    d.session = 2;
    d.sector_size = 2336;
    vec![a, d]
}

#[test]
fn the_stock_record_of_the_dcload_cdi() {
    let c = dcload_cdi();
    let boot = cdi_boot_area_lba(&c, 2);
    assert_eq!(boot, 11702);
    let (fad, tr) = head_position(&c, AudioState::Standby, 0, 150);

    // REQ_STAT: 02 2f 01 01 01 00 00 96 00 00. The low nibble of byte 1 (F) is
    // the repeat field, F whenever nothing plays (D3), whatever count the
    // player holds.
    let stat = req_stat_reply(AudioState::Standby, DRIVE_STANDBY, DISC_FORMAT_CDROM_XA, 0, fad, tr);
    assert_eq!(&stat[..], &hex("022f0101010000960000")[..]);

    // GET_TOC area 0 and 1: the same 408 bytes.
    let mut want = [0xFFu8; 408];
    want[..8].copy_from_slice(&hex("0100009641002e4c"));
    want[396..].copy_from_slice(&hex("01010000410200004105017c"));
    assert_eq!(toc(&c, true, boot, 0), want);
    assert_eq!(toc(&c, true, boot, 1), want);

    // REQ_SES 0..3.
    let st = drive_status(AudioState::Standby, DRIVE_STANDBY);
    assert_eq!(req_ses_reply(&c, true, boot, 2, 0, st).unwrap()[..], hex("02000205017c")[..]);
    assert_eq!(req_ses_reply(&c, true, boot, 2, 1, st).unwrap()[..], hex("020001000096")[..]);
    assert_eq!(req_ses_reply(&c, true, boot, 2, 2, st).unwrap()[..], hex("020002002e4c")[..]);
    assert_eq!(req_ses_reply(&c, true, boot, 2, 3, st), None); // stock: CHECK

    // GET_SCD format 0 (100 bytes, copied from the record) and format 1 (14).
    let mut r = [0u8; 100];
    assert_eq!(get_scd_reply(0, AudioState::Standby, fad, tr, &mut r), 100);
    assert_eq!(&r[..], &hex(STOCK_GET_SCD_0)[..]);
    assert_eq!(get_scd_reply(1, AudioState::Standby, fad, tr, &mut r), 14);
    assert_eq!(&r[..14], &hex("0015000e01010100000000000096")[..]);

    // REQ_ERROR with nothing pending.
    assert_eq!(&req_error_reply(0, 0, 0)[..], &hex("f0000000000000000000")[..]);
}

const STOCK_GET_SCD_0: &str = "00150064000000000000004000000000000000400000000000000040000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000004000000000000000000000400040400040000000400040000000";

/// GDI 03 of the stock probe card (`stock_20261121_04_img3-7.txt`, img=3):
/// data 0..599, audio 756..1355, data 45000..64999 (sector counts from the
/// track files, as the firmware and `examples/expect.rs` derive them).
fn stock_gdi03() -> Vec<TrackDesc> {
    vec![t(1, 0, 4, 600), t(2, 756, 0, 600), t(3, 45000, 4, 20000)]
}

#[test]
fn the_stock_record_of_gdi_03_leadout() {
    // D1: stock reports the end of the last track, not 549300.
    let g = stock_gdi03();
    let boot = gdi_boot_area_lba(&g);
    let a1 = toc(&g, false, boot, 1);
    // GET_TOC_1: ... 4100b05e ... 41030000 41030000 4100fe7e
    assert_eq!(&a1[8..12], &hex("4100b05e")[..]);
    assert_eq!(&a1[396..], &hex("41030000410300004100fe7e")[..]);
    // REQ_SES_0 (stock was paused, status 01): 01 00 01 00 fe 7e
    assert_eq!(req_ses_reply(&g, false, boot, 1, 0, DRIVE_PAUSE).unwrap()[..], hex("01000100fe7e")[..]);
    assert_eq!(leadout_lba(&g, false), 65000);
}

#[test]
fn the_stock_record_of_gdi_08_leadout() {
    // stock_20261121_06_gdi-variants.txt img=8: last track data 55900..65899.
    // GET_TOC_1 ends 41030000 41050000 41010202; REQ_SES_0 01 00 01 01 02 02.
    let g = vec![
        t(1, 0, 4, 600),
        t(2, 756, 0, 600),
        t(3, 45000, 4, 10000),
        t(4, 55150, 0, 600),
        t(5, 55900, 4, 10000),
    ];
    let boot = gdi_boot_area_lba(&g);
    let a1 = toc(&g, false, boot, 1);
    assert_eq!(&a1[396..], &hex("410300004105000041010202")[..]);
    assert_eq!(req_ses_reply(&g, false, boot, 1, 0, DRIVE_PAUSE).unwrap()[..], hex("010001010202")[..]);
}

#[test]
fn the_stock_record_of_gdi_req_stat_repeat_nibble() {
    // stock_20261121_04_img3-7.txt img=3 REQ_STAT: 01 8f ... (paused, GD-ROM).
    let stat = req_stat_reply::<TrackDesc>(AudioState::Paused, DRIVE_STANDBY, DISC_FORMAT_GDROM, 0, 0, None);
    assert_eq!(stat[1], 0x8F);
}

/// GET_SCD format 0 of GDI 03 after `52 82 03` (`stock_20261121_04_img3-7.txt`
/// img=3, also records 03/06/08): track 3, FAD 45150 = 00:10:00 absolute.
const STOCK_GDI03_GET_SCD_0: &str = "00150064004000000000004000000000000040400000000000000040000000000000000000000000000000000000000000000000000000000000000000000040000000000000000000004000000000000000000040004000404000400000404000404000";

#[test]
fn idle_status_and_head_after_power_on_and_after_a_switch() {
    // D2: no CD-DA started -> the idle status; CD-DA states keep theirs.
    assert_eq!(drive_status(AudioState::Standby, DRIVE_STANDBY), DRIVE_STANDBY);
    assert_eq!(drive_status(AudioState::Standby, DRIVE_PAUSE), DRIVE_PAUSE);
    for (s, code) in [(AudioState::Playing, 3), (AudioState::Paused, 1), (AudioState::Finished, 1), (AudioState::Error, 9)] {
        assert_eq!(drive_status(s, DRIVE_STANDBY), code);
        assert_eq!(drive_status(s, DRIVE_PAUSE), code);
    }
    // D4: the idle head. Power-on: FAD 150 for both formats (no stock record of
    // a GDI at power-on; NetGD keeps 150). After a switch: GDI = track 3's
    // start, CDI = 150; a GDI without a track 3 stays at 150.
    let g = stock_gdi03();
    assert_eq!(idle_fad(&g, false, false), 150);
    assert_eq!(idle_fad(&g, false, true), 45150);
    assert_eq!(idle_fad(&dcload_cdi(), true, true), 150);
    assert_eq!(idle_fad(&dcload_cdi(), true, false), 150);
    assert_eq!(idle_fad(&[t(1, 0, 4, 600), t(2, 756, 0, 600)], false, true), 150);
    // The idle position only applies while no CD-DA was started.
    assert_eq!(head_position(&g, AudioState::Paused, 1000, 45150).0, 1000);
}

#[test]
fn the_stock_record_of_gdi_03_after_a_switch() {
    // stock_20261121_04_img3-7.txt img=3 (and 03/06/08): paused, track 3, FAD 45150.
    let g = stock_gdi03();
    let boot = gdi_boot_area_lba(&g);
    let (fad, tr) = head_position(&g, AudioState::Standby, 0, idle_fad(&g, false, true));
    let stat = req_stat_reply(AudioState::Standby, DRIVE_PAUSE, DISC_FORMAT_GDROM, 0, fad, tr);
    assert_eq!(&stat[..], &hex("018f41030100b05e0000")[..]);
    let st = drive_status(AudioState::Standby, DRIVE_PAUSE);
    assert_eq!(req_ses_reply(&g, false, boot, 1, 0, st).unwrap()[..], hex("01000100fe7e")[..]);
    assert_eq!(req_ses_reply(&g, false, boot, 1, 1, st).unwrap()[..], hex("010001000096")[..]);
    // GET_SCD keeps audio status 0x15 (no CD-DA) while REQ_STAT says pause.
    let mut r = [0u8; 100];
    assert_eq!(get_scd_reply(1, AudioState::Standby, fad, tr, &mut r), 14);
    assert_eq!(&r[..14], &hex("0015000e4103010000000000b05e")[..]);
    assert_eq!(get_scd_reply(0, AudioState::Standby, fad, tr, &mut r), 100);
    assert_eq!(&r[..], &hex(STOCK_GDI03_GET_SCD_0)[..]);
}

#[test]
fn the_stock_record_of_a_cdi_after_a_switch() {
    // stock_20261121_04_img3-7.txt img=0 (after record 03's return to 2), and
    // img=4/5 (CDI 04/05): paused, track 1, FAD 150. The dcload CDI's tables.
    let c = dcload_cdi();
    let boot = cdi_boot_area_lba(&c, 2);
    let (fad, tr) = head_position(&c, AudioState::Standby, 0, idle_fad(&c, true, true));
    let stat = req_stat_reply(AudioState::Standby, DRIVE_PAUSE, DISC_FORMAT_CDROM_XA, 0, fad, tr);
    assert_eq!(&stat[..], &hex("012f0101010000960000")[..]);
    let st = drive_status(AudioState::Standby, DRIVE_PAUSE);
    assert_eq!(req_ses_reply(&c, true, boot, 2, 0, st).unwrap()[..], hex("01000205017c")[..]);
    assert_eq!(req_ses_reply(&c, true, boot, 2, 1, st).unwrap()[..], hex("010001000096")[..]);
    assert_eq!(req_ses_reply(&c, true, boot, 2, 2, st).unwrap()[..], hex("010002002e4c")[..]);
    let mut r = [0u8; 100];
    assert_eq!(get_scd_reply(1, AudioState::Standby, fad, tr, &mut r), 14);
    assert_eq!(&r[..14], &hex("0015000e01010100000000000096")[..]);
}

#[test]
fn identify_packet_is_the_stock_80_bytes() {
    // stock_20261122_08_identify.txt, probe=IDENTIFY_PACKET: 80 bytes, in the
    // order gdprobe stores them (each 16-bit data word little-endian) (D16).
    let want = hex(concat!(
        "20202000000000000000000000000000",
        "53452020202020202020202020202020",
        "43442d524f4d20445249564520202020",
        "362e3432202020202020202020202020",
        "00000000000000000000000000000000",
    ));
    assert_eq!(&IDENTIFY_PACKET_REPLY[..], &want[..]);
    assert_eq!(&IDENTIFY_PACKET_REPLY[16..18], b"SE");
    assert_eq!(&IDENTIFY_PACKET_REPLY[32..44], b"CD-ROM DRIVE");
    assert_eq!(&IDENTIFY_PACKET_REPLY[48..52], b"6.42");
}

// ---------------------------------------------------------------- D5 / D13 (stock_20261121_04_img3-7.txt)

#[test]
fn check_registers_are_the_stock_form() {
    // D5: every CHECK CONDITION of the stock drive reads status 0x11 (DSC |
    // ERR, DRDY clear) and error = sense key << 4.
    assert_eq!(CHECK_STATUS, 0x11);
    assert_eq!(check_error(SENSE_NOT_READY), 0x20); // TUR with no medium
    assert_eq!(check_error(SENSE_MEDIUM_ERROR), 0x30);
    assert_eq!(check_error(SENSE_ILLEGAL_REQUEST), 0x50); // REQ_SES_3, CD_READ type mismatch
    assert_eq!(check_error(SENSE_UNIT_ATTENTION), 0x60); // first TUR after a mount
    assert_eq!(check_error(0x16), 0x60, "only the key's nibble");
}

#[test]
fn swap_window_phases() {
    // Stock after `52 82 n` with no open_time / detect_time in GDEMU.INI
    // (`stock_20261122_11_ini-no-time-keys.txt`, LOG R135): 2/3A/00 from t=0,
    // 2/04/01 from ~506 ms, one 6/28/00 at ~755 ms, READY from the next poll.
    assert_eq!((SWAP_OPEN_MS, SWAP_DETECT_MS), (500, 250));
    let p = |t| swap_phase(t, SWAP_OPEN_MS, SWAP_DETECT_MS);
    assert_eq!(p(0), SwapPhase::Open);
    assert_eq!(p(499), SwapPhase::Open);
    assert_eq!(p(500), SwapPhase::Detect);
    assert_eq!(p(749), SwapPhase::Detect);
    assert_eq!(p(750), SwapPhase::Done);
    assert_eq!(p(u32::MAX), SwapPhase::Done);
    // With the INI keys set to 150 / 150 (R119) and 600 / 300 (R129) stock
    // used those values instead; `swap_phase` takes them as arguments.
    assert_eq!(swap_phase(149, 150, 150), SwapPhase::Open);
    assert_eq!(swap_phase(150, 150, 150), SwapPhase::Detect);
    assert_eq!(swap_phase(300, 150, 150), SwapPhase::Done);
    assert_eq!(swap_phase(899, 600, 300), SwapPhase::Detect);
    assert_eq!(swap_phase(900, 600, 300), SwapPhase::Done);
    // Zero-length phases are skipped.
    assert_eq!(swap_phase(0, 0, 150), SwapPhase::Detect);
    assert_eq!(swap_phase(0, 0, 0), SwapPhase::Done);
    assert_eq!(swap_phase(10, 5000, 5000), SwapPhase::Open);
    assert_eq!(swap_phase(9999, 5000, 5000), SwapPhase::Detect);
    assert_eq!(swap_phase(10_000, 5000, 5000), SwapPhase::Done);
}

#[test]
fn swap_window_sense_and_drive_status() {
    // The REQ_ERROR bytes each phase's NOT READY carries (records 04: img=3
    // SW_POLL, img=6/7 REQ_ERROR_W0).
    let (a, q) = not_ready_asc(SwapPhase::Open);
    assert_eq!(&req_error_reply(SENSE_NOT_READY, a, q)[..], &hex("f0000200000000003a00")[..]);
    let (a, q) = not_ready_asc(SwapPhase::Detect);
    assert_eq!(&req_error_reply(SENSE_NOT_READY, a, q)[..], &hex("f0000200000000000401")[..]);
    // After the window with no medium (empty folder): back to 2/3A/00.
    assert_eq!(not_ready_asc(SwapPhase::Done), ASC_NO_MEDIUM);
    // SECNR's drive status nibble (flycast gdromv3.h): open, then busy, then
    // paused (stock's REQ_STAT byte 0 after a switch, D2) or no disc. Stock's
    // register itself is not recorded by gdprobe.
    assert_eq!(swap_drive_status(SwapPhase::Open, true), DRIVE_OPEN);
    assert_eq!(swap_drive_status(SwapPhase::Detect, true), DRIVE_BUSY);
    assert_eq!(swap_drive_status(SwapPhase::Done, true), DRIVE_PAUSE);
    assert_eq!(swap_drive_status(SwapPhase::Done, false), DRIVE_NODISC);
    assert_eq!((DRIVE_BUSY, DRIVE_OPEN, DRIVE_NODISC), (0x00, 0x06, 0x07));
}

#[test]
fn device_reset_re_detects_like_the_stock_records() {
    // R155 / R159 / seq 17: every DEVICE RESET, with or without a window:
    // status 0x00, error 0x01, IRR 1 until the next command.
    assert_eq!((DEVICE_RESET_STATUS, DEVICE_RESET_ERROR, DEVICE_RESET_IRR), (0x00, 0x01, 0x01));
    // The re-detect lasts detect_time (seq 17: 150 -> ~150 ms; seq 18:
    // 400 -> ~401 ms), with no open phase.
    assert_eq!(device_reset_redetect_ms(150), 150);
    assert_eq!(device_reset_redetect_ms(400), 400);
    assert_eq!(device_reset_window(400), (0, 400));
    assert_eq!(swap_phase(399, 0, 400), SwapPhase::Detect);
    assert_eq!(swap_phase(400, 0, 400), SwapPhase::Done);
    // No open phase: 0x00 at once (never 0x06), the final byte at the end.
    let (o, d) = device_reset_window(150);
    let k = WindowKind::Redetect;
    let at = |t| window_secnr(k, swap_phase(t, o, d), true, DISC_FORMAT_CDROM_XA, DRIVE_PAUSE);
    assert_eq!(at(0), 0x00);
    assert_eq!(at(149), 0x00);
    assert_eq!(at(150), 0x21); // record 16: 601 -> 752 ms, 0x00 -> 0x21
    assert_eq!(window_secnr(k, SwapPhase::Done, true, DISC_FORMAT_GDROM, DRIVE_PAUSE), 0x81);
    assert_eq!(window_secnr(k, SwapPhase::Done, false, 0, DRIVE_PAUSE), 0x07);
    // The byte at rest keeps the idle status (standby before any switch).
    assert_eq!(window_secnr(k, SwapPhase::Done, true, DISC_FORMAT_CDROM_XA, 0x02), 0x22);
    // A switch's window is unchanged: open 0x06, busy 0x00, then paused.
    let s = WindowKind::Switch;
    assert_eq!(window_secnr(s, SwapPhase::Open, true, DISC_FORMAT_GDROM, 0x02), 0x06);
    assert_eq!(window_secnr(s, SwapPhase::Detect, true, DISC_FORMAT_GDROM, 0x02), 0x00);
    assert_eq!(window_secnr(s, SwapPhase::Done, true, DISC_FORMAT_GDROM, 0x02), 0x81);
    // NOT READY inside the re-detect is "becoming ready" (stock not probed).
    assert_eq!(not_ready_asc(swap_phase(0, o, d)), ASC_BECOMING_READY);
}

#[test]
fn same_folder_re_switch_is_a_no_op_only_for_the_mounted_folder() {
    // seq 17 RS_: 52 82 n for the folder already mounted, image or empty.
    assert!(same_folder_reswitch(Some(4), Some(4), true));
    assert!(same_folder_reswitch(Some(7), Some(7), true));
    // Another folder, nothing remembered, or ejected / write session: a mount.
    assert!(!same_folder_reswitch(Some(3), Some(4), true));
    assert!(!same_folder_reswitch(Some(4), None, true));
    assert!(!same_folder_reswitch(None, None, true));
    assert!(!same_folder_reswitch(Some(4), Some(4), false));
}

fn cdb(b: &[u8]) -> [u8; 12] {
    let mut c = [0u8; 12];
    c[..b.len()].copy_from_slice(b);
    c
}

#[test]
fn what_stock_answers_with_no_medium() {
    use NotReadyAnswer::*;
    // stock_20261121_04_img3-7.txt img=6 / img=7 (no medium), probe by probe.
    for (name, c, want) in [
        ("TUR", cdb(&[0x00]), NotReady),
        ("REQ_STAT", cdb(&[0x10, 0, 0, 0, 10]), NotReady),
        ("REQ_MODE_0_10", cdb(&[0x11, 0, 0, 0, 10]), Serve),
        ("REQ_MODE_18_8", cdb(&[0x11, 0, 18, 0, 8]), Serve),
        ("REQ_ERROR", cdb(&[0x13, 0, 0, 0, 10]), Serve),
        ("GET_TOC_0", cdb(&[0x14, 0, 0, 1, 0x98]), NotReady),
        ("GET_TOC_1", cdb(&[0x14, 1, 0, 1, 0x98]), NotReady),
        ("REQ_SES_0", cdb(&[0x15, 0, 0, 0, 6]), NotReady),
        ("REQ_SES_1", cdb(&[0x15, 0, 1, 0, 6]), IllegalRequest),
        ("REQ_SES_2", cdb(&[0x15, 0, 2, 0, 6]), IllegalRequest),
        ("REQ_SES_3", cdb(&[0x15, 0, 3, 0, 6]), IllegalRequest),
        ("GET_SCD_0", cdb(&[0x40, 0, 0, 0, 100]), NotReady),
        ("GET_SCD_1", cdb(&[0x40, 1, 0, 0, 14]), NotReady),
        ("GDEMU_VERSION", cdb(&[0x52, 0x00]), Serve),
        // Not in the record: the switch itself (it is what leaves the
        // state), REQUEST SENSE, NetGD's 0xE0 and START/STOP (the host's
        // eject / load) are served; reads and CD-DA play are NOT READY, as
        // NetGD already answered them while ejected.
        ("SW_CMD", cdb(&[0x52, 0x82, 7]), Serve),
        ("REQUEST_SENSE", cdb(&[0x03, 0, 0, 0, 18]), Serve),
        ("IMAGE_CTL", cdb(&[0xE0, 8]), Serve),
        ("START_STOP", cdb(&[0x1B, 0, 0, 0, 3]), Serve),
        ("CD_READ", cdb(&[0x30, 0x24, 0, 0, 0x96, 0, 0, 0, 0, 0, 1]), NotReady),
        ("CD_READ2", cdb(&[0x31]), NotReady),
        ("CD_PLAY", cdb(&[0x20, 1]), NotReady),
    ] {
        assert_eq!(not_ready_answer(&c), want, "{}", name);
    }
}

#[test]
fn cd_read_type_check_follows_the_stock_records() {
    use discimg::{TRACK_AUDIO, TRACK_MODE1, TRACK_MODE2};
    // Stock GDEMU 6.0, CDREAD_FLAGS (D8, D9, R144) -- by the track's mode:
    //   Mode 1 (GDI data, Mode 1 CDI 10): 0x20 ok, 0x24 ok,      0x28 5/64/00, 0x10 ok
    //   Mode 2 (mkdcdisc CDI 04 data):    0x20 ok, 0x24 5/64/00, 0x28 ok,      0x10 ok
    //   audio  (GDI 03, FAD 906):         0x20 ok, 0x24 5/64/00, 0x28 5/64/00, 0x10 ok
    for (flags, m1_bad, m2_bad, audio_bad) in [
        (0x20u8, false, false, false),
        (0x24, false, true, true),
        (0x28, true, false, true),
        (0x10, false, false, false),
    ] {
        assert_eq!(cd_read_type_mismatch(flags, TRACK_MODE1), m1_bad, "Mode 1 flags {:#x}", flags);
        assert_eq!(cd_read_type_mismatch(flags, TRACK_MODE2), m2_bad, "Mode 2 flags {:#x}", flags);
        assert_eq!(cd_read_type_mismatch(flags, TRACK_AUDIO), audio_bad, "audio flags {:#x}", flags);
        // Bit 0 (address type) does not change it.
        assert_eq!(cd_read_type_mismatch(flags | 1, TRACK_MODE2), m2_bad);
        assert_eq!(cd_read_type_mismatch(flags | 1, TRACK_AUDIO), audio_bad);
    }
    // Types stock was not asked about (CD-DA, Mode 2 formless, Form 2, ...): let through.
    for ty in [1u8, 3, 5, 6, 7] {
        for m in [TRACK_AUDIO, TRACK_MODE1, TRACK_MODE2] {
            assert!(!cd_read_type_mismatch(0x20 | (ty << 1), m));
        }
    }
    assert_eq!(ASC_ILLEGAL_MODE, (0x64, 0x00));
}

#[test]
fn cd_read_type_check_is_by_track_mode_not_image_format() {
    use discimg::{TrackInfo, TRACK_MODE1};
    // R144 g: image 10, a CDI whose data track is Mode 1, takes 0x24 and
    // refuses 0x28, like a GDI.
    let mut data = t(2, 11700, 4, 1000);
    data.mode = TRACK_MODE1;
    assert!(!cd_read_type_mismatch(0x24, data.mode()));
    assert!(cd_read_type_mismatch(0x28, data.mode()));
}

#[test]
fn audio_tracks_read_whole_pcm_sectors() {
    use discimg::{TRACK_AUDIO, TRACK_MODE1, TRACK_MODE2};
    // R144: GDI 03's audio track (FAD 906) answers 0x20 and 0x10 with the
    // 2,352-byte PCM sector as stored, seven sectors = 2,352 x 7 -- with or
    // without the raw-sectors feature (`raw_data`).
    for raw_data in [false, true] {
        for flags in [0x20u8, 0x10, 0x21, 0x11] {
            assert!(cd_read_raw_out(flags, TRACK_AUDIO, 2352, raw_data), "{:#x}", flags);
        }
        // An audio track not stored as 2,352-byte sectors cannot be sent whole.
        assert!(!cd_read_raw_out(0x20, TRACK_AUDIO, 2048, raw_data));
        assert!(!cd_read_raw_out(0x20, TRACK_AUDIO, 2336, raw_data));
        // Data tracks: 0x20 stays a 2,048-byte payload.
        for m in [TRACK_MODE1, TRACK_MODE2] {
            assert!(!cd_read_raw_out(0x20, m, 2352, raw_data));
            assert!(!cd_read_raw_out(0x24, m, 2352, raw_data));
        }
    }
    // Data 0x10 is whole only with raw-sectors (D10), from 2,352 or 2,336.
    for m in [TRACK_MODE1, TRACK_MODE2] {
        assert!(!cd_read_raw_out(0x10, m, 2352, false));
        assert!(cd_read_raw_out(0x10, m, 2352, true));
        assert!(cd_read_raw_out(0x10, m, 2336, true));
        assert!(!cd_read_raw_out(0x10, m, 2048, true));
    }
}

#[test]
fn audio_not_stored_whole_is_refused() {
    use discimg::{TRACK_AUDIO, TRACK_MODE1, TRACK_MODE2};
    assert!(!cd_read_unservable(TRACK_AUDIO, 2352));
    assert!(cd_read_unservable(TRACK_AUDIO, 2048));
    assert!(cd_read_unservable(TRACK_AUDIO, 2336));
    for m in [TRACK_MODE1, TRACK_MODE2] {
        for ssz in [2048u32, 2336, 2352] {
            assert!(!cd_read_unservable(m, ssz));
        }
    }
}

#[test]
fn ctrl_and_mode_pack_into_one_byte() {
    use discimg::{pack_ctrl_mode, packed_ctrl, packed_mode, TRACK_AUDIO, TRACK_MODE1, TRACK_MODE2};
    for ctrl in 0u8..16 {
        for m in [TRACK_AUDIO, TRACK_MODE1, TRACK_MODE2] {
            let p = pack_ctrl_mode(ctrl, m);
            assert_eq!((packed_ctrl(p), packed_mode(p)), (ctrl, m));
        }
    }
    // Only the control nibble is kept (every reader masks or shifts it anyway).
    assert_eq!(packed_ctrl(pack_ctrl_mode(0x14, TRACK_MODE2)), 4);
}

#[test]
fn disc_format_follows_the_track_modes() {
    use discimg::{TRACK_AUDIO, TRACK_MODE1, TRACK_MODE2};
    let mk = |modes: &[u8]| -> Vec<TrackDesc> {
        modes
            .iter()
            .enumerate()
            .map(|(i, &m)| {
                let mut x = t(i as u8 + 1, i as u32 * 1000, if m == TRACK_AUDIO { 0 } else { 4 }, 1000);
                x.mode = m;
                x
            })
            .collect()
    };
    // GDI 03: GD-ROM whatever its tracks (REQ_STAT `01 8f ...`, SECNR 0x81).
    assert_eq!(disc_format(&mk(&[TRACK_MODE1, TRACK_AUDIO, TRACK_MODE1]), false), DISC_FORMAT_GDROM);
    // CDI 04 (Mode 2 data): CD-ROM XA (`01 2f ...`, SECNR 0x21).
    assert_eq!(disc_format(&mk(&[TRACK_AUDIO, TRACK_MODE2]), true), DISC_FORMAT_CDROM_XA);
    // CDI 10 (Mode 1 data): CD-ROM (`01 1f 01 01 01 00 00 96`, SECNR 0x11).
    assert_eq!(disc_format(&mk(&[TRACK_AUDIO, TRACK_MODE1]), true), DISC_FORMAT_CDROM);
    // Any Mode 2 track makes it XA (Flycast `GuessDiscType`).
    assert_eq!(disc_format(&mk(&[TRACK_MODE1, TRACK_MODE2]), true), DISC_FORMAT_CDROM_XA);
    // CDI 11 (audio only): CD-ROM, not Flycast's CD-DA (R155: `01 1f ...`, SECNR 0x11).
    assert_eq!(disc_format(&mk(&[TRACK_AUDIO, TRACK_AUDIO]), true), DISC_FORMAT_CDROM);
    assert_eq!(disc_format(&mk(&[TRACK_AUDIO]), true), DISC_FORMAT_CDROM);
    assert_eq!(disc_format::<TrackDesc>(&[], true), DISC_FORMAT_CDROM);

    // R155's REQ_STAT of the audio-only CDI after a switch, byte for byte.
    let tracks = mk(&[TRACK_AUDIO, TRACK_AUDIO]);
    let fad = idle_fad(&tracks, true, true);
    let (fad, tr) = head_position(&tracks, AudioState::Standby, 0, fad);
    assert_eq!(
        req_stat_reply(AudioState::Standby, DRIVE_PAUSE, disc_format(&tracks, true), 0, fad, tr),
        [0x01, 0x1f, 0x01, 0x01, 0x01, 0x00, 0x00, 0x96, 0x00, 0x00]
    );

    // R144's REQ_STAT of the Mode 1 CDI after a switch, byte for byte.
    let tracks = mk(&[TRACK_AUDIO, TRACK_MODE1]);
    let fad = idle_fad(&tracks, true, true);
    let (fad, tr) = head_position(&tracks, AudioState::Standby, 0, fad);
    let fmt = disc_format(&tracks, true);
    assert_eq!(
        req_stat_reply(AudioState::Standby, DRIVE_PAUSE, fmt, 0, fad, tr),
        [0x01, 0x1f, 0x01, 0x01, 0x01, 0x00, 0x00, 0x96, 0x00, 0x00]
    );
}

#[test]
fn secnr_through_the_swap_window() {
    use SwapPhase::*;
    // R144 b (gdprobe secnr=): 0x06 (open), 0x00 (busy), format nibble 0 in
    // both, then 0xX1 with the disc's format by the UNIT ATTENTION.
    for fmt in [DISC_FORMAT_CDROM_XA, DISC_FORMAT_CDROM, DISC_FORMAT_GDROM] {
        assert_eq!(swap_secnr(Open, true, fmt), 0x06);
        assert_eq!(swap_secnr(Detect, true, fmt), 0x00);
        assert_eq!(swap_secnr(Done, true, fmt), fmt | 0x01);
        // Empty folder: 0x06, 0x00, then 0x07 (NO DISC), never a format.
        assert_eq!(swap_secnr(Open, false, fmt), 0x06);
        assert_eq!(swap_secnr(Detect, false, fmt), 0x00);
        assert_eq!(swap_secnr(Done, false, fmt), 0x07);
    }
    assert_eq!(swap_secnr(Done, true, DISC_FORMAT_CDROM_XA), 0x21);
    assert_eq!(swap_secnr(Done, true, DISC_FORMAT_CDROM), 0x11);
    assert_eq!(swap_secnr(Done, true, DISC_FORMAT_GDROM), 0x81);
}

#[test]
fn secnr_after_a_bus_reset() {
    // No medium: NO DISC, format 0, whatever the FPGA reloaded (0x81) or an
    // EJECT left behind (the old image's format | 1).
    for latched in [0x81, 0x21, 0x11, 0x01, 0x07] {
        for fmt in [DISC_FORMAT_GDROM, DISC_FORMAT_CDROM_XA, 0] {
            assert_eq!(reset_secnr(false, fmt, latched), DRIVE_NODISC);
        }
    }
    // A medium: its format, the latched status nibble kept.
    assert_eq!(reset_secnr(true, DISC_FORMAT_GDROM, 0x81), 0x81);
    assert_eq!(reset_secnr(true, DISC_FORMAT_CDROM_XA, 0x81), 0x21);
    assert_eq!(reset_secnr(true, DISC_FORMAT_CDROM, 0x81), 0x11);
    assert_eq!(reset_secnr(true, DISC_FORMAT_CDROM_XA, 0x02), 0x22);
}

#[test]
fn raw_sector_reads_follow_the_stock_records() {
    // D10: data select 0x1_ is a raw read; 0x20 and friends are not.
    assert!(cd_read_wants_raw(0x10));
    assert!(cd_read_wants_raw(0x11));
    for f in [0x00u8, 0x20, 0x24, 0x28, 0x30, 0xF0] {
        assert!(!cd_read_wants_raw(f), "{:#x}", f);
    }
    assert!(raw_from_stored(2352) && raw_from_stored(2336));
    assert!(!raw_from_stored(2048) && !raw_from_stored(2324));

    // CDI data at FAD 11852 (LBA 11702) -> MSF 02:38:02.
    let h = mode2_raw_header(11702);
    assert_eq!(h, [0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0, 0x02, 0x38, 0x02, 0x02]);
    // GD-ROM HDA start: LBA 45000 -> FAD 45150 = 10:02:00.
    assert_eq!(&mode2_raw_header(45000)[12..], &[0x10, 0x02, 0x00, 0x02]);

    let stored: Vec<u8> = (0..2336u32).map(|i| (i * 7) as u8).collect();
    let mut out = [0u8; 2352];
    raw_sector(&stored, 11702, &mut out);
    assert_eq!(out[..16], h[..]);
    assert_eq!(&out[16..], &stored[..]);
    let stored: Vec<u8> = (0..2352u32).map(|i| (i * 13) as u8).collect();
    raw_sector(&stored, 99, &mut out);
    assert_eq!(&out[..], &stored[..]);
}

#[test]
fn expand_raw_in_place_matches_a_copy_for_every_alignment() {
    // The firmware reads whole 512-byte blocks, so the first stored sector
    // starts `head` bytes in; the expansion then works inside one buffer.
    const BUF: usize = 14 * 1024;
    for sec in [2352usize, 2336] {
        for count in 1..=5usize {
            for head in [0usize, 1, 15, 16, 17, 100, 255, 256, 400, 510, 511] {
                if head + count * sec > BUF || count * 2352 > BUF {
                    continue;
                }
                let lba = 11702 + head as u32;
                let mut buf = vec![0xEEu8; BUF];
                let stored: Vec<Vec<u8>> = (0..count)
                    .map(|s| (0..sec).map(|i| (i as u32 * 31 + s as u32 * 7 + head as u32) as u8).collect())
                    .collect();
                for (s, st) in stored.iter().enumerate() {
                    buf[head + s * sec..head + (s + 1) * sec].copy_from_slice(st);
                }
                let n = expand_raw_in_place(&mut buf, head, sec, lba, count);
                assert_eq!(n, count * 2352);
                for (s, st) in stored.iter().enumerate() {
                    let mut want = [0u8; 2352];
                    raw_sector(st, lba + s as u32, &mut want);
                    assert_eq!(&buf[s * 2352..(s + 1) * 2352], &want[..], "sec {} count {} head {} s {}", sec, count, head, s);
                }
            }
        }
    }
}

/// R139: a sense read (REQUEST SENSE 0x03, REQ_ERROR 0x13) reports the sense
/// of the last CHECK. It must not advance or close the swap window: the TUR
/// that got 2/04/01 and the REQ_ERROR that follows it would otherwise see the
/// window close in between and report 6/28/00 (stock: 2/04/01, then 6/28/00 on
/// the next TUR).
#[test]
fn sense_reads_do_not_advance_the_swap_window() {
    assert!(!advances_swap_window(0x03));
    assert!(!advances_swap_window(0x13));
    for op in [0x00u8, 0x10, 0x11, 0x14, 0x15, 0x20, 0x30, 0x40, 0x52, 0xE0] {
        assert!(advances_swap_window(op), "opcode {:#04x}", op);
    }
}

/// R139: a sense read clears the UNIT ATTENTION only when it reported it
/// (key 6). After `52 00` has cleared the pending key, REQ_ERROR reports
/// nothing and the UA must stay for the next command's CHECK.
#[test]
fn sense_read_clears_ua_only_when_reported() {
    assert!(sense_read_clears_ua(SENSE_UNIT_ATTENTION));
    assert!(!sense_read_clears_ua(0));
    assert!(!sense_read_clears_ua(SENSE_NOT_READY));
}

/// mkdcdisc's postgap sector (`tools/mksdcard` FORM2_GAP): Mode 2 Form 2,
/// submode 0x20, zero data, a Form 2 EDC at the end.
fn form2_gap() -> Vec<u8> {
    let mut g = vec![0u8; 2336];
    g[..8].copy_from_slice(&[0, 0, 0x20, 0, 0, 0, 0x20, 0]);
    g[2332..].copy_from_slice(&[0x3F, 0x13, 0xB0, 0xBE]);
    g
}

#[test]
fn form2_reads_follow_the_stock_records() {
    use discimg::{TRACK_AUDIO, TRACK_MODE1, TRACK_MODE2};
    // D14: stock answers 0x20 on a CDI postgap sector (Form 2) with the
    // 2,324 bytes after the subheader, up to the Form 2 EDC (record 05,
    // HASH ssz=2324 crc=d0912a98 = FORM2_GAP[8:2332]).
    assert_eq!(FORM2_SECTOR_BYTES, 2324);
    assert_eq!(FORM2_DATA_OFFSET, 8);
    let g = form2_gap();
    assert!(stored_is_form2(&g));
    let mut f1 = vec![0u8; 2336];
    f1[..8].copy_from_slice(&[0, 0, 0x08, 0, 0, 0, 0x08, 0]);
    assert!(!stored_is_form2(&f1), "Form 1 data");
    assert!(!stored_is_form2(&[0u8; 2]), "too short");

    // Only data select 0x2 with no expected type, on a Mode 2 track stored
    // 2,336 bytes a sector (a CDI's data). 0x28 (Form 1 expected), raw
    // reads, Mode 1 and GDI tracks are not probed and keep their answers.
    assert!(cd_read_may_be_form2(0x20, TRACK_MODE2, 2336));
    assert!(cd_read_may_be_form2(0x21, TRACK_MODE2, 2336));
    for f in [0x24u8, 0x28, 0x10, 0x30, 0x00] {
        assert!(!cd_read_may_be_form2(f, TRACK_MODE2, 2336), "{:#x}", f);
    }
    assert!(!cd_read_may_be_form2(0x20, TRACK_MODE1, 2336));
    assert!(!cd_read_may_be_form2(0x20, TRACK_MODE2, 2352));
    assert!(!cd_read_may_be_form2(0x20, TRACK_MODE2, 2048));
    assert!(!cd_read_may_be_form2(0x20, TRACK_AUDIO, 2352));
}

#[test]
fn form2_compaction_matches_a_copy_for_every_alignment() {
    const BUF: usize = 14 * 1024;
    let gap = form2_gap();
    for count in 1..=5usize {
        for head in [0usize, 1, 8, 15, 16, 17, 100, 255, 256, 400, 510, 511] {
            if head + count * 2336 > BUF {
                continue;
            }
            for mixed in [false, true] {
                let mut buf = vec![0xEEu8; BUF];
                let stored: Vec<Vec<u8>> = (0..count)
                    .map(|s| {
                        let mut v = gap.clone();
                        for (i, b) in v[8..2332].iter_mut().enumerate() {
                            *b = (i as u32 * 29 + s as u32 * 5 + head as u32) as u8;
                        }
                        if mixed && s == count - 1 {
                            v[2] = 0x08;
                            v[6] = 0x08;
                        }
                        v
                    })
                    .collect();
                for (s, st) in stored.iter().enumerate() {
                    buf[head + s * 2336..head + (s + 1) * 2336].copy_from_slice(st);
                }
                assert_eq!(all_form2(&buf, head, count), !mixed, "count {} head {} mixed {}", count, head, mixed);
                if mixed {
                    continue;
                }
                let n = compact_in_place(&mut buf, head, 2336, count, FORM2_DATA_OFFSET, FORM2_SECTOR_BYTES);
                assert_eq!(n, count * 2324);
                for (s, st) in stored.iter().enumerate() {
                    assert_eq!(&buf[s * 2324..(s + 1) * 2324], &st[8..2332], "count {} head {} s {}", count, head, s);
                }
            }
        }
    }
}
