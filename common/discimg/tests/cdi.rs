//! Characterization tests for the CDI descriptor parser, on synthetic images
//! built per the layout documented in the firmware's cdi.rs.

mod oracle;

use discimg::cdi::{parse, parse_slice, sector_size};
use discimg::{Error, Full, TrackDesc, TrackSink};

struct T {
    name_len: usize,
    pregap: u32,
    sector_count: u32,
    start_lba: u32,
    total_length: u32,
    read_mode: u32,
    control: u32,
}

/// Build a CDI file: `data_len` bytes of fake track data, then the descriptor
/// for `sessions` (each a list of tracks), a terminator, and the length word.
fn build(data_len: usize, sessions: &[Vec<T>]) -> Vec<u8> {
    let mut f = vec![0xAAu8; data_len];
    let desc_start = f.len();
    f.push(sessions.len() as u8);
    for s in sessions {
        f.extend_from_slice(&[0, s.len() as u8, (s.len() >> 8) as u8, 0, 0, 0, 0]);
        for (i, t) in s.iter().enumerate() {
            // Track header: 25 + name + 31. The name length is byte 24.
            let mut h = vec![0x11u8; 25];
            h[24] = t.name_len as u8;
            f.extend_from_slice(&h);
            f.extend(core::iter::repeat(b'N').take(t.name_len));
            f.extend_from_slice(&[0x22u8; 31]);
            // Track data: 180 bytes, 188 for the last of the session.
            let mut d = vec![0u8; if i + 1 == s.len() { 188 } else { 180 }];
            d[2..6].copy_from_slice(&t.pregap.to_le_bytes());
            d[6..10].copy_from_slice(&t.sector_count.to_le_bytes());
            d[32..36].copy_from_slice(&t.start_lba.to_le_bytes());
            d[36..40].copy_from_slice(&t.total_length.to_le_bytes());
            d[56..60].copy_from_slice(&t.read_mode.to_le_bytes());
            d[60..64].copy_from_slice(&t.control.to_le_bytes());
            f.extend_from_slice(&d);
        }
    }
    // Terminator: empty session header, a track header, some disc info.
    f.extend_from_slice(&[0u8; 7]);
    f.extend_from_slice(&[0u8; 56]);
    f.extend_from_slice(&[0x33u8; 40]);
    f.extend_from_slice(&0x8000_0006u32.to_le_bytes());
    let len = (f.len() - desc_start + 4) as u32;
    f.extend_from_slice(&len.to_le_bytes());
    f
}

/// A typical homebrew MIL-CD: session 1 = two audio tracks, session 2 = one
/// Mode 2 data track; filenames of 32, 56 and 7 bytes.
fn mil_cd() -> Vec<u8> {
    build(
        1_000_000,
        &[
            vec![
                T { name_len: 32, pregap: 150, sector_count: 3000, start_lba: 0, total_length: 3150, read_mode: 2, control: 0 },
                T { name_len: 56, pregap: 0, sector_count: 7000, start_lba: 3150, total_length: 7000, read_mode: 2, control: 0 },
            ],
            vec![
                T { name_len: 7, pregap: 150, sector_count: 20000, start_lba: 45000, total_length: 20150, read_mode: 1, control: 0x14 },
            ],
        ],
    )
}

fn mil_cd_expected() -> Vec<(u8, u32, u8, u16, u32, u32)> {
    // (number, start_lba, ctrl, sector_size, sector_count, file_offset)
    // start_lba is the first DATA sector's LBA = descriptor start_lba + pregap - 150:
    // track 2 (descriptor 3150, pregap 0) follows track 1's 3000 data sectors at 3000.
    // base1 = 0                      offset1 = 150*2352                   = 352_800
    // base2 = 3150*2352 = 7_408_800  offset2 = base2 (no pregap)          = 7_408_800
    // base3 = base2 + 7000*2352      = 23_872_800; offset3 = base3 + 150*2336 = 24_223_200
    vec![
        (1, 0, 0, 2352, 3000, 352_800),
        (2, 3000, 0, 2352, 7000, 7_408_800),
        (3, 45000, 4, 2336, 20000, 24_223_200),
    ]
}

fn summary(t: &[TrackDesc]) -> Vec<(u8, u32, u8, u16, u32, u32)> {
    t.iter()
        .map(|t| (t.number, t.start_lba, t.ctrl, t.sector_size, t.sector_count, t.file_offset))
        .collect()
}

#[test]
fn two_session_mil_cd_with_variable_length_filenames() {
    let (sessions, tracks) = parse_slice(&mil_cd()).unwrap();
    assert_eq!(sessions, 2);
    assert_eq!(summary(&tracks), mil_cd_expected());
    assert!(tracks.iter().all(|t| t.filename.is_empty()));
}

#[test]
fn control_is_masked_to_four_bits() {
    // 0x14 -> 4 (the ADR nibble is dropped).
    let (_, t) = parse_slice(&mil_cd()).unwrap();
    assert_eq!(t[2].ctrl, 4);
}

#[test]
fn read_mode_to_sector_size() {
    assert_eq!(sector_size(0), Some(2048));
    assert_eq!(sector_size(1), Some(2336));
    assert_eq!(sector_size(2), Some(2352));
    assert_eq!(sector_size(3), None);
    assert_eq!(sector_size(u32::MAX), None);
}

#[test]
fn single_session_cooked_data_track() {
    let f = build(
        4096,
        &[vec![T { name_len: 32, pregap: 0, sector_count: 2, start_lba: 200, total_length: 2, read_mode: 0, control: 4 }]],
    );
    let (s, t) = parse_slice(&f).unwrap();
    assert_eq!(s, 1);
    assert_eq!(summary(&t), vec![(1, 50, 4, 2048, 2, 0)]);
}

#[test]
fn mixed_name_lengths_do_not_misalign_later_tracks() {
    // 0 and 255-byte names (255 is the u8 maximum) around a normal one.
    let mk = |n| T { name_len: n, pregap: 0, sector_count: 10, start_lba: n as u32 + 150, total_length: 10, read_mode: 2, control: 0 };
    let f = build(100, &[vec![mk(0), mk(255), mk(1), mk(56)]]);
    let (_, t) = parse_slice(&f).unwrap();
    assert_eq!(t.iter().map(|t| t.start_lba).collect::<Vec<_>>(), vec![0, 255, 1, 56]);
    assert_eq!(t.iter().map(|t| t.file_offset).collect::<Vec<_>>(), vec![0, 23520, 47040, 70560]);
}

#[test]
fn start_lba_is_the_first_data_sector_not_the_pregap() {
    // mkdcdisc layout (image 05 of tools/mksdcard): three tracks, pregap 150 each, descriptor
    // start_lba = running sum of pregap+data+postgap. With pregap 150 the descriptor value is
    // already the data LBA (FAD 150 + LBA), so nothing moves.
    let t = |start, count, mode| T { name_len: 32, pregap: 150, sector_count: count, start_lba: start, total_length: count + 150, read_mode: mode, control: 0 };
    let f = build(10_000, &[vec![t(0, 302, 2), t(452, 402, 2)], vec![t(12254, 1102, 1)]]);
    let (_, tr) = parse_slice(&f).unwrap();
    assert_eq!(tr.iter().map(|t| t.start_lba).collect::<Vec<_>>(), vec![0, 452, 12254]);
    assert_eq!(tr.iter().map(|t| t.file_offset).collect::<Vec<_>>(), vec![352_800, 1_415_904, 2_711_808]);
    // File offsets: track 2's data sits (452 - 302 - ... ) sectors after track 1's data; consecutive
    // data LBAs differ by pregap + count exactly as the file offsets do.
    assert_eq!(tr[1].file_offset - tr[0].file_offset, (150 + 302) * 2352);
}

#[test]
fn shorter_pregap_moves_the_data_lba_earlier() {
    // pregap 0 on track 2: its data begins where its (zero-length) pregap does, FAD start_lba,
    // i.e. LBA start_lba - 150. Flycast: track FAD = start_lba + pregap_length.
    let f = build(
        10_000,
        &[vec![
            T { name_len: 1, pregap: 150, sector_count: 1000, start_lba: 0, total_length: 1150, read_mode: 2, control: 0 },
            T { name_len: 1, pregap: 0, sector_count: 500, start_lba: 1150, total_length: 500, read_mode: 2, control: 0 },
        ]],
    );
    let (_, t) = parse_slice(&f).unwrap();
    assert_eq!(t[0].start_lba, 0);
    assert_eq!(t[1].start_lba, 1000); // directly after track 1's 1000 data sectors
    assert_eq!(t[1].file_offset, 1150 * 2352);
}

#[test]
fn start_lba_before_the_first_fad_skips_the_sectors_below_lba_zero() {
    // pregap 0, start_lba 0: the data begins at FAD 0, 150 sectors before LBA 0. Those are
    // skipped (file offset advances, count shrinks) rather than pinning the LBA to 0 with the
    // offset unchanged, which would serve every sector 150 sectors early.
    let one = |pregap, count, start| T { name_len: 1, pregap, sector_count: count, start_lba: start, total_length: count, read_mode: 2, control: 0 };
    let f = build(100, &[vec![one(0, 1000, 0)]]);
    let t = parse_slice(&f).unwrap().1;
    assert_eq!((t[0].start_lba, t[0].sector_count, t[0].file_offset), (0, 850, 150 * 2352));
    // With a pregap: first data sector at FAD 100 (pregap 100, start 0), 50 below LBA 0.
    let f = build(100, &[vec![one(100, 1000, 0)]]);
    let t = parse_slice(&f).unwrap().1;
    assert_eq!((t[0].start_lba, t[0].sector_count, t[0].file_offset), (0, 950, (100 + 50) * 2352));
    // Exactly at FAD 150 nothing moves.
    let f = build(100, &[vec![one(100, 1000, 50)]]);
    let t = parse_slice(&f).unwrap().1;
    assert_eq!((t[0].start_lba, t[0].sector_count, t[0].file_offset), (0, 1000, 100 * 2352));
    // A track wholly below LBA 0 has nothing to map.
    let f = build(100, &[vec![one(0, 150, 0)]]);
    assert_eq!(parse_slice(&f), Err(Error::BadCdi));
    let f = build(100, &[vec![one(0, 1, 0)]]);
    assert_eq!(parse_slice(&f), Err(Error::BadCdi));
}

#[test]
fn track_numbers_run_across_sessions() {
    let (_, t) = parse_slice(&mil_cd()).unwrap();
    assert_eq!(t.iter().map(|t| t.number).collect::<Vec<_>>(), vec![1, 2, 3]);
}

#[test]
fn errors() {
    let good = mil_cd();
    // Too short for the length word + anything.
    assert_eq!(parse_slice(&good[..7]), Err(Error::BadCdi));
    // Descriptor length below 16 / past the start of the file.
    let mut f = good.clone();
    let n = f.len();
    f[n - 4..].copy_from_slice(&15u32.to_le_bytes());
    assert_eq!(parse_slice(&f), Err(Error::BadCdi));
    f[n - 4..].copy_from_slice(&(n as u32 + 1).to_le_bytes());
    assert_eq!(parse_slice(&f), Err(Error::BadCdi));
    // Unknown read mode.
    let bad = build(
        64,
        &[vec![T { name_len: 1, pregap: 0, sector_count: 1, start_lba: 0, total_length: 1, read_mode: 3, control: 0 }]],
    );
    assert_eq!(parse_slice(&bad), Err(Error::BadCdi));
    // pregap * sector size overflows.
    let bad = build(
        64,
        &[vec![T { name_len: 1, pregap: 0x4000_0000, sector_count: 1, start_lba: 0, total_length: 1, read_mode: 2, control: 0 }]],
    );
    assert_eq!(parse_slice(&bad), Err(Error::BadCdi));
    // running base overflows on the second track.
    let t = |total| T { name_len: 1, pregap: 0, sector_count: 1, start_lba: 200, total_length: total, read_mode: 2, control: 0 };
    let bad = build(64, &[vec![t(0x2000_0000), t(0x2000_0000)]]);
    assert_eq!(parse_slice(&bad), Err(Error::BadCdi));
    // No sessions / no tracks at all.
    assert_eq!(parse_slice(&build(64, &[])), Err(Error::BadCdi));
    assert_eq!(parse_slice(&build(64, &[vec![]])), Err(Error::BadCdi));
}

#[test]
fn truncated_descriptor_is_a_read_error() {
    // Claims two sessions but the descriptor is cut mid-way: the next read
    // runs past EOF, which is a reader failure rather than BadCdi.
    let f = build(
        64,
        &[vec![T { name_len: 1, pregap: 0, sector_count: 1, start_lba: 200, total_length: 1, read_mode: 2, control: 0 }]],
    );
    let mut g = f.clone();
    // Bump the session count so the walk wants a second session header past
    // the real one; with the terminator (7+56+40 bytes) in place this reads
    // the terminator as a session with 0 tracks, so cut the file instead.
    let ds = 64;
    g[ds] = 200;
    let r = parse_slice(&g);
    assert_eq!(r, Err(Error::Read));
}

struct Cap {
    cap: usize,
    n: usize,
    hdr: Option<(u32, u8)>,
    cleared: usize,
}
impl TrackSink for Cap {
    type Error = ();
    fn clear(&mut self) { self.n = 0; self.cleared += 1; }
    fn push(&mut self, _: &TrackDesc) -> Result<(), ()> {
        if self.n == self.cap { return Err(()); }
        self.n += 1;
        Ok(())
    }
    fn cdi_header(&mut self, l: u32, s: u8) { self.hdr = Some((l, s)); }
}

#[test]
fn sink_sees_header_then_clear_and_its_errors_pass_through() {
    let f = mil_cd();
    let rd = |off: u32, buf: &mut [u8]| -> Result<(), ()> {
        let o = off as usize;
        buf.copy_from_slice(f.get(o..o + buf.len()).ok_or(())?);
        Ok(())
    };
    let mut c = Cap { cap: 99, n: 0, hdr: None, cleared: 0 };
    assert_eq!(parse(f.len() as u32, rd, &mut c), Ok(2));
    assert_eq!((c.n, c.cleared), (3, 1));
    let (dl, s) = c.hdr.unwrap();
    assert_eq!(s, 2);
    // desc_len counts back from EOF and includes the length word itself.
    assert_eq!(f.len() - dl as usize, 1_000_000);
    let mut c = Cap { cap: 2, n: 0, hdr: None, cleared: 0 };
    assert_eq!(parse(f.len() as u32, rd, &mut c), Err(Error::Sink(())));
    // Failure before the header (length word) leaves the sink untouched.
    let mut c = Cap { cap: 99, n: 7, hdr: None, cleared: 0 };
    assert_eq!(parse(4, rd, &mut c), Err(Error::BadCdi));
    assert_eq!((c.n, c.cleared), (7, 0));
}

#[test]
fn too_many_tracks_is_the_sinks_call() {
    // 100 tracks into the 99-entry VecSink.
    let t = |i: u32| T { name_len: 1, pregap: 0, sector_count: 1, start_lba: 200 + i, total_length: 1, read_mode: 2, control: 0 };
    let f = build(64, &[(0..100).map(t).collect()]);
    assert_eq!(parse_slice(&f), Err(Error::Sink(Full)));
    let f = build(64, &[(0..99).map(t).collect()]);
    assert_eq!(parse_slice(&f).unwrap().1.len(), 99);
}

#[test]
fn agrees_with_the_original_walk() {
    let mut images = vec![mil_cd()];
    images.push(build(
        4096,
        &[vec![T { name_len: 32, pregap: 0, sector_count: 2, start_lba: 200, total_length: 2, read_mode: 0, control: 4 }]],
    ));
    images.push(build(10, &[]));
    images.push(vec![0u8; 7]);
    images.push(vec![0xFF; 100]);
    let mut cut = mil_cd();
    cut[1_000_000] = 9;
    images.push(cut);
    for f in &images {
        let new = parse_slice(f);
        let old = oracle::cdi_parse(f);
        match (new, old) {
            (Ok((s, t)), Ok((os, ot, offs))) => {
                assert_eq!(s, os);
                assert_eq!(t.len(), ot.len());
                for ((a, b), off) in t.iter().zip(ot.iter()).zip(offs.iter()) {
                    // INTENTIONAL DIFFERENCE (R28/D-5): the old walk reported the descriptor's
                    // start_lba (the PREGAP start) as the data LBA; the new parser reports
                    // start_lba + pregap - 150 (saturating). Everything else must be identical;
                    // start_lba is checked against the old value mapped through that rule, with
                    // the pregap re-read by an independent reader (`pregap_adjust`). They are
                    // equal wherever the pregap is 150, so only mil_cd()'s track 2 differs.
                    assert_eq!(
                        (a.number, a.ctrl, a.sector_size, a.sector_count, a.file_offset),
                        (b.number, b.ctrl, b.sector_size, b.sector_count, *off)
                    );
                    assert_eq!(a.start_lba, pregap_adjust(f, a.number, b.start_lba));
                }
            }
            (Err(Error::BadCdi), Err("BadCdi")) => {}
            (Err(Error::Read), Err("Read")) => {}
            (n, o) => panic!("mismatch: {:?} vs {:?}", n, o),
        }
    }
}

/// The old walk's `start_lba` mapped through the new rule (`+ pregap - 150`, saturating at 0),
/// re-reading the pregap of track `number` from the descriptor with an independent reader.
fn pregap_adjust(file: &[u8], number: u8, old_start: u32) -> u32 {
    let rd = |at: usize| u32::from_le_bytes([file[at], file[at + 1], file[at + 2], file[at + 3]]);
    let desc_start = file.len() - rd(file.len() - 4) as usize;
    let mut cur = desc_start + 1;
    let mut n = 0u8;
    for _ in 0..file[desc_start] {
        let tc = u16::from_le_bytes([file[cur + 1], file[cur + 2]]);
        cur += 7;
        for ti in 0..tc {
            let td = cur + 25 + file[cur + 24] as usize + 31;
            n += 1;
            if n == number {
                return old_start.saturating_add(rd(td + 2)).saturating_sub(150);
            }
            cur = td + if ti + 1 == tc { 188 } else { 180 };
        }
    }
    unreachable!()
}

// ---- R89 C3: trailer version word ------------------------------------------

/// The same descriptor re-stamped with another trailer: the version word and
/// the offset (v3: from the start of the file, v3.5: from the end).
fn with_trailer(good: &[u8], version: u32, offset: impl Fn(usize, usize) -> u32) -> Vec<u8> {
    let n = good.len();
    let desc_len = u32::from_le_bytes(good[n - 4..].try_into().unwrap()) as usize;
    let desc_start = n - desc_len;
    let mut f = good.to_vec();
    f[n - 8..n - 4].copy_from_slice(&version.to_le_bytes());
    f[n - 4..].copy_from_slice(&offset(desc_start, n).to_le_bytes());
    f
}

#[test]
fn v35_offset_counts_from_the_end() {
    let good = mil_cd();
    let f = with_trailer(&good, 0x8000_0006, |start, n| (n - start) as u32);
    assert_eq!(f, good);
    assert_eq!(parse_slice(&f).unwrap().0, 2);
}

#[test]
fn v3_offset_counts_from_the_start() {
    let v35 = parse_slice(&mil_cd()).unwrap();
    let f = with_trailer(&mil_cd(), 0x8000_0005, |start, _| start as u32);
    assert_eq!(parse_slice(&f).unwrap(), v35);
    // The v3.5 reading of the same bytes is a different (wrong) place.
    let g = with_trailer(&mil_cd(), 0x8000_0006, |start, _| start as u32);
    assert!(parse_slice(&g).is_err());
    // Offsets that leave no room for a descriptor.
    let n = f.len() as u32;
    for bad in [0, n - 15, n, n + 1] {
        let f = with_trailer(&mil_cd(), 0x8000_0005, |_, _| bad);
        let r = parse_slice(&f);
        assert_eq!(r, Err(Error::BadCdi), "v3 offset {}", bad);
    }
}

#[test]
fn v2_and_unknown_versions_are_bad_cdi() {
    for v in [0x8000_0004u32, 0x8000_0003, 0x8000_0007, 0, 0xFFFF_FFFF] {
        let f = with_trailer(&mil_cd(), v, |start, n| (n - start) as u32);
        assert_eq!(parse_slice(&f), Err(Error::BadCdi), "version {:#x}", v);
    }
}
