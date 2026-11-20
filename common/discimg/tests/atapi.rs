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
    // HDA: track 3 only. Lead-out = 45000 + 504150 + 150 = 549300 = 0x0861B4.
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
fn toc_hda_leadout_is_padded_to_the_pressed_capacity_but_sda_is_not() {
    // A trimmed dump: track 3 file is only 1000 sectors.
    let mut g = gd3();
    g[2].sector_count = 1000;
    let hda = toc(&g, false, 45000, 1);
    assert_eq!(&hda[404..408], &[0x41, 0x08, 0x61, 0xB4]);
    // Same trimming on the SDA side is reported as-is.
    g[1].sector_count = 100;
    let sda = toc(&g, false, 45000, 0);
    assert_eq!(&sda[404..408], &[0x01, 0x00, 0x02, 0xBC]); // 450+100+150 = 700
}

#[test]
fn toc_unknown_area_numbers_behave_like_hda_but_without_padding() {
    // Anything but 0 selects tracks >= boot_area_lba; only area == 1 pads the
    // lead-out out to the pressed-disc capacity.
    let mut g = gd3();
    g[2].sector_count = 1000;
    let a2 = toc(&g, false, 45000, 2);
    assert_eq!(&a2[8..12], &[0x41, 0x00, 0xB0, 0x5E]);
    assert_eq!(&a2[404..408], &[0x41, 0x00, 0xB4, 0x46]); // 45000+1000+150 = 46150
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
    // Area 1 of a CDI is empty.
    assert_eq!(toc(&m, true, 45000, 1), [0xFF; 408]);
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
    assert_eq!(leadout_lba(&gd3(), false), 549150);
    assert_eq!(leadout_lba(&mil(), true), 65000);
    // An (impossible) empty CDI falls back to the GD-ROM value.
    assert_eq!(leadout_lba(&[] as &[TrackDesc], true), 549150);
}

#[test]
fn req_ses_gdrom() {
    let g = gd3();
    // session count 1, lead-out 549150 + 150 = 549300 = 0x0861B4
    assert_eq!(req_ses_reply(&g, false, 45000, 1, 0), Some([0x01, 0x00, 0x01, 0x08, 0x61, 0xB4]));
    assert_eq!(req_ses_reply(&g, false, 45000, 1, 1), Some([0x01, 0x00, 0x01, 0x00, 0x00, 0x96]));
    // GD-ROM reports one session only.
    assert_eq!(req_ses_reply(&g, false, 45000, 1, 2), None);
    assert_eq!(req_ses_reply(&g, false, 45000, 1, 3), None);
}

#[test]
fn req_ses_mil_cd() {
    let m = mil();
    assert_eq!(req_ses_reply(&m, true, 45000, 2, 0), Some([0x01, 0x00, 0x02, 0x00, 0xFE, 0x7E]));
    assert_eq!(req_ses_reply(&m, true, 45000, 2, 1), Some([0x01, 0x00, 0x01, 0x00, 0x00, 0x96]));
    assert_eq!(req_ses_reply(&m, true, 45000, 2, 2), Some([0x01, 0x00, 0x03, 0x00, 0xB0, 0x5E]));
    assert_eq!(req_ses_reply(&m, true, 45000, 2, 3), None);
}

#[test]
fn req_ses_two_track_cdi_session_2_is_the_data_track() {
    // This used to pin ABRT (the firmware fell back to boot_area_lba = 45000
    // for a CDI with fewer than 3 tracks, so the data track at 11700 was not
    // "at or above" it). The firmware now passes the start of the last
    // session's first track (11700); with a stale 45000 the lookup still
    // finds nothing, which is why the caller must not guess.
    let c = vec![t(1, 0, 0, 11000), t(2, 11700, 4, 20000)];
    assert_eq!(req_ses_reply(&c, true, 45000, 2, 2), None);
    // boot_area_lba = the data track's start finds it.
    assert_eq!(req_ses_reply(&c, true, 11700, 2, 2), Some([0x01, 0x00, 0x02, 0x00, 0x2E, 0x4A]));
}

#[test]
fn req_ses_no_sda_track() {
    let g = vec![t(3, 45000, 4, 100)];
    assert_eq!(req_ses_reply(&g, false, 45000, 1, 1), None);
    // Session 0 does not need any track.
    assert_eq!(req_ses_reply(&[] as &[TrackDesc], false, 45000, 1, 0), Some([1, 0, 1, 0x08, 0x61, 0xB4]));
}

#[test]
fn head_position_idle_is_fad_150_in_track_1() {
    let g = gd3();
    let (fad, tr) = head_position(&g, AudioState::Standby, 12345);
    assert_eq!(fad, 150);
    assert_eq!(tr.unwrap().number, 1);
    // Playing at FAD 1000 -> LBA 850 -> track 2 (starts at 450).
    let (fad, tr) = head_position(&g, AudioState::Playing, 1000);
    assert_eq!((fad, tr.unwrap().number), (1000, 2));
    // Before the first track (FAD < 150 saturates to LBA 0), and past the end.
    let (_, tr) = head_position(&g, AudioState::Paused, 10);
    assert_eq!(tr.unwrap().number, 1);
    let (_, tr) = head_position(&g, AudioState::Paused, 600_000);
    assert_eq!(tr.unwrap().number, 3);
    // No tracks.
    assert!(head_position(&[] as &[TrackDesc], AudioState::Playing, 5).1.is_none());
}

#[test]
fn req_stat() {
    let g = gd3();
    let (fad, tr) = head_position(&g, AudioState::Standby, 777);
    assert_eq!(
        req_stat_reply(AudioState::Standby, 0, fad, tr),
        [0x02, 0x00, 0x41, 0x01, 0x01, 0x00, 0x00, 0x96, 0x00, 0x00]
    );
    let (fad, tr) = head_position(&g, AudioState::Playing, 1000);
    assert_eq!(
        req_stat_reply(AudioState::Playing, 3, fad, tr),
        [0x03, 0x03, 0x01, 0x02, 0x01, 0x00, 0x03, 0xE8, 0x00, 0x00]
    );
    for (s, code) in [
        (AudioState::Playing, 3),
        (AudioState::Paused, 1),
        (AudioState::Finished, 1),
        (AudioState::Error, 9),
        (AudioState::Standby, 2),
    ] {
        assert_eq!(req_stat_reply::<TrackDesc>(s, 0, 0, None)[0], code);
    }
    // No track under the head: CTRL/ADR 0x41, track 1; FAD is full 24-bit BE.
    assert_eq!(
        req_stat_reply::<TrackDesc>(AudioState::Paused, 9, 0x01_02_03, None),
        [0x01, 0x09, 0x41, 0x01, 0x01, 0x01, 0x02, 0x03, 0x00, 0x00]
    );
}

#[test]
fn get_scd_format_1_is_14_bytes() {
    let g = gd3();
    let (fad, tr) = head_position(&g, AudioState::Playing, 1000);
    let mut r = [0xEEu8; 100];
    assert_eq!(get_scd_reply(1, AudioState::Playing, fad, tr, &mut r), 14);
    // Q: ctrl/adr 01, track 02, index 01, rel 00:05:25 (400 = 1000-600), 00, abs 00:13:25.
    assert_eq!(
        &r[..14],
        &[0x00, 0x11, 0x00, 0x0E, 0x01, 0x02, 0x01, 0x00, 0x05, 0x25, 0x00, 0x00, 0x13, 0x25]
    );
    // The rest of the buffer is zeroed, not left alone.
    assert!(r[14..].iter().all(|&b| b == 0));
}

#[test]
fn get_scd_idle_and_trackless() {
    let g = gd3();
    let (fad, tr) = head_position(&g, AudioState::Standby, 0);
    let mut r = [0u8; 100];
    assert_eq!(get_scd_reply(1, AudioState::Standby, fad, tr, &mut r), 14);
    // Track 1, data; relative 0; absolute 150 = 00:02:00.
    assert_eq!(
        &r[..14],
        &[0x00, 0x15, 0x00, 0x0E, 0x41, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00]
    );
    // No track: ctrl/adr 0x41, track and relative time left zero.
    assert_eq!(get_scd_reply::<TrackDesc>(1, AudioState::Paused, 150, None, &mut r), 14);
    assert_eq!(
        &r[..14],
        &[0x00, 0x12, 0x00, 0x0E, 0x41, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00]
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
    let (fad, tr) = head_position(&g, AudioState::Playing, 1000);
    let mut r = [0u8; 100];
    assert_eq!(get_scd_reply(0, AudioState::Playing, fad, tr, &mut r), 100);
    assert_eq!(&r[..4], &[0x00, 0x11, 0x00, 100]);
    // Same Q as the format-1 case, then the inverted CRC of Q[0..10].
    let q10 = [0x01, 0x02, 0x01, 0x00, 0x05, 0x25, 0x00, 0x00, 0x13, 0x25];
    let crc = !xmodem(&q10);
    let mut q = q10.to_vec();
    q.extend_from_slice(&crc.to_be_bytes());
    let mut want = Vec::new();
    for b in &q {
        for bit in 0..8 {
            want.push(if (b >> (7 - bit)) & 1 != 0 { 0x7F } else { 0x3F });
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
            let mut o = [0u8; 408];
            og.build_toc(area, &mut o);
            assert_eq!(toc(&new, is_cdi, boot, area), o);
        }
        for s in 0..5u8 {
            // The oracle is the ORIGINAL code, which is wrong for CDI sessions
            // 1 and 2 (it keyed both on `boot_area_lba` alone, so a CDI whose
            // data track sits below it, or a single-session CDI, got ABRT).
            // CDI session 0 and all GDI replies must still match; the CDI
            // cases are pinned in tests/cdi_sessions.rs.
            if !is_cdi || s == 0 {
                assert_eq!(req_ses_reply(&new, is_cdi, boot, sessions, s), oracle::build_req_ses_reply(Some(&og), s));
            }
        }
        assert_eq!(leadout_lba(&new, is_cdi), og.leadout_lba());

        for &(ns, os) in &states {
            let fad = if rnd(4) == 0 { rnd(400) as u32 } else { rnd(700000) as u32 };
            let repeat = rnd(256) as u8;
            let snap = Snapshot { status: os, repeat, fad };
            let (hf, ht) = head_position(&new, ns, fad);
            let (of, ot) = oracle::head_position(&snap, Some(&og));
            assert_eq!(hf, of);
            assert_eq!(ht.map(|t| t.number), ot.map(|t| t.number));
            assert_eq!(
                req_stat_reply(ns, repeat, hf, ht),
                oracle::build_req_stat_reply(&snap, Some(&og))
            );
            for format in [0u8, 1, 2] {
                let mut a = [0xCCu8; 100];
                let mut b = [0xDDu8; 100];
                let la = get_scd_reply(format, ns, hf, ht, &mut a);
                let lb = oracle::build_get_scd_reply(format, &snap, Some(&og), &mut b);
                assert_eq!(la, lb);
                assert_eq!(a, b);
            }
        }
    }
}
