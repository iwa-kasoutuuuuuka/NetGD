//! VFAT long-name matching against hand-built directories.

use fatmap::lfn::*;

const OFFS: [usize; 13] = [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];

/// The 11-byte alias for `"BASE.EXT"`-style input, space padded and upper case.
fn alias(base: &str, ext: &str) -> [u8; 11] {
    let mut a = [b' '; 11];
    a[..base.len()].copy_from_slice(base.as_bytes());
    a[8..8 + ext.len()].copy_from_slice(ext.as_bytes());
    a
}

/// A short (8.3) directory entry.
fn short_entry(name: &[u8; 11], attr: u8, cluster: u32, size: u32) -> [u8; 32] {
    let mut e = [0u8; 32];
    e[..11].copy_from_slice(name);
    e[11] = attr;
    e[20..22].copy_from_slice(&((cluster >> 16) as u16).to_le_bytes());
    e[26..28].copy_from_slice(&(cluster as u16).to_le_bytes());
    e[28..32].copy_from_slice(&size.to_le_bytes());
    e
}

/// The long-name pieces for `units`, in on-disk order (last piece first),
/// carrying `csum`.
fn lfn_pieces(units: &[u16], csum: u8) -> Vec<[u8; 32]> {
    let n = (units.len() + 12) / 13;
    let mut out = Vec::new();
    for seq in (1..=n).rev() {
        let mut e = [0u8; 32];
        e[0] = seq as u8 | if seq == n { 0x40 } else { 0 };
        e[11] = 0x0F;
        e[13] = csum;
        for (k, &off) in OFFS.iter().enumerate() {
            let idx = (seq - 1) * 13 + k;
            let c = if idx < units.len() {
                units[idx]
            } else if idx == units.len() {
                0x0000
            } else {
                0xFFFF
            };
            e[off..off + 2].copy_from_slice(&c.to_le_bytes());
        }
        out.push(e);
    }
    out
}

fn u16s(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// Long-name run plus its alias entry.
fn file(long: &str, a: &[u8; 11]) -> Vec<[u8; 32]> {
    let mut v = lfn_pieces(&u16s(long), short_checksum(a));
    v.push(short_entry(a, 0x20, 5, 1000));
    v
}

/// Feed a whole directory (with a terminating 0x00 entry) and return, per
/// real entry, whether the long name matched.
fn run<const N: usize>(m: &mut LfnMatcher<N>, dir: &[[u8; 32]]) -> Vec<bool> {
    let mut hits = Vec::new();
    for e in dir {
        match m.feed(e) {
            Feed::End => break,
            Feed::Other => {}
            Feed::Entry { long } => hits.push(long),
        }
    }
    hits
}

fn matcher(name: &str) -> LfnMatcher<255> {
    LfnMatcher::from_str(name).unwrap()
}

#[test]
fn checksum_vectors() {
    // Computed with an independent implementation of the spec's algorithm.
    assert_eq!(short_checksum(b"TRACK0~1BIN"), 0x23);
    assert_eq!(short_checksum(b"FOO     BAR"), 0x53);
    assert_eq!(short_checksum(b"README  TXT"), 0x73);
    assert_eq!(short_checksum(b"           "), 0xF7);
}

#[test]
fn matches_a_name_with_spaces() {
    let a = alias("TRACK0~1", "BIN");
    let dir = file("Track 01.bin", &a);
    assert_eq!(run(&mut matcher("Track 01.bin"), &dir), vec![true]);
}

#[test]
fn compares_ascii_case_insensitively_both_ways() {
    let a = alias("TRACK0~1", "BIN");
    let dir = file("Track 01.bin", &a);
    assert_eq!(run(&mut matcher("TRACK 01.BIN"), &dir), vec![true]);
    assert_eq!(run(&mut matcher("track 01.Bin"), &dir), vec![true]);
    let dir = file("TRACK 01.BIN", &a);
    assert_eq!(run(&mut matcher("track 01.bin"), &dir), vec![true]);
}

#[test]
fn different_names_do_not_match() {
    let a = alias("TRACK0~1", "BIN");
    let dir = file("Track 01.bin", &a);
    for other in ["Track 02.bin", "Track 01.bi", "Track 01.binx", "Track01.bin", "rack 01.bin", "T"] {
        assert_eq!(run(&mut matcher(other), &dir), vec![false], "{other}");
    }
}

#[test]
fn prefix_and_extension_of_the_name_do_not_match() {
    // 13-character boundary cases: target is a prefix / extension of the
    // stored name and the lengths straddle a piece.
    let a = alias("LONGFI~1", "BIN");
    let stored = "abcdefghijklm.bin"; // 17 chars, 2 pieces
    let dir = file(stored, &a);
    assert_eq!(run(&mut matcher(stored), &dir), vec![true]);
    assert_eq!(run(&mut matcher("abcdefghijklm"), &dir), vec![false]);
    assert_eq!(run(&mut matcher("abcdefghijklm.bin2"), &dir), vec![false]);
    assert_eq!(run(&mut matcher("abcdefghijkl"), &dir), vec![false]);
}

#[test]
fn checksum_mismatch_is_an_orphan() {
    let a = alias("TRACK0~1", "BIN");
    let mut dir = lfn_pieces(&u16s("Track 01.bin"), short_checksum(&a) ^ 1);
    dir.push(short_entry(&a, 0x20, 5, 10));
    assert_eq!(run(&mut matcher("Track 01.bin"), &dir), vec![false]);
}

#[test]
fn checksum_must_agree_across_pieces() {
    let a = alias("LONGFI~1", "BIN");
    let name = "abcdefghijklmnopqrstuvwxyz.bin"; // 30 chars, 3 pieces
    let mut dir = lfn_pieces(&u16s(name), short_checksum(&a));
    dir[1][13] ^= 0x10; // middle piece
    dir.push(short_entry(&a, 0x20, 5, 10));
    assert_eq!(run(&mut matcher(name), &dir), vec![false]);
}

#[test]
fn deleted_alias_leaves_no_trace_for_the_next_entry() {
    let a = alias("TRACK0~1", "BIN");
    let mut dir = lfn_pieces(&u16s("Track 01.bin"), short_checksum(&a));
    let mut dead = short_entry(&a, 0x20, 5, 10);
    dead[0] = 0xE5;
    dir.push(dead);
    // The next file has the same alias but no long name of its own.
    dir.push(short_entry(&a, 0x20, 9, 10));
    assert_eq!(run(&mut matcher("Track 01.bin"), &dir), vec![false]);
}

#[test]
fn deleted_piece_breaks_the_run() {
    let a = alias("LONGFI~1", "BIN");
    let name = "abcdefghijklmnopqrstuvwxyz.bin";
    for victim in 0..3 {
        let mut dir = lfn_pieces(&u16s(name), short_checksum(&a));
        dir[victim][0] = 0xE5;
        dir.push(short_entry(&a, 0x20, 5, 10));
        assert_eq!(run(&mut matcher(name), &dir), vec![false], "piece {victim}");
    }
}

#[test]
fn missing_piece_and_wrong_order_are_rejected() {
    let a = alias("LONGFI~1", "BIN");
    let name = "abcdefghijklmnopqrstuvwxyz.bin"; // 3 pieces
    let full = lfn_pieces(&u16s(name), short_checksum(&a));
    // gap: 3, 1
    let mut dir = vec![full[0], full[2]];
    dir.push(short_entry(&a, 0x20, 5, 10));
    assert_eq!(run(&mut matcher(name), &dir), vec![false]);
    // swapped: 3, 1, 2
    let mut dir = vec![full[0], full[2], full[1]];
    dir.push(short_entry(&a, 0x20, 5, 10));
    assert_eq!(run(&mut matcher(name), &dir), vec![false]);
    // a run that starts without the 0x40 flag (its head was overwritten)
    let mut dir = vec![full[1], full[2]];
    dir.push(short_entry(&a, 0x20, 5, 10));
    assert_eq!(run(&mut matcher(name), &dir), vec![false]);
    // a run that stops before ordinal 1
    let mut dir = vec![full[0], full[1]];
    dir.push(short_entry(&a, 0x20, 5, 10));
    assert_eq!(run(&mut matcher(name), &dir), vec![false]);
}

#[test]
fn orphan_run_does_not_poison_the_next_file() {
    let a1 = alias("TRACK0~1", "BIN");
    let a2 = alias("TRACK0~2", "BIN");
    // An orphan run for the sought name (its alias is gone), then a real file
    // with a different long name, then the real one.
    let mut dir = lfn_pieces(&u16s("Track 01.bin"), short_checksum(&a1));
    dir.extend(file("Track 02.bin", &a2));
    dir.extend(file("Track 01.bin", &a1));
    assert_eq!(run(&mut matcher("Track 01.bin"), &dir), vec![false, true]);
}

#[test]
fn run_followed_by_end_of_directory_is_not_a_match() {
    let a = alias("TRACK0~1", "BIN");
    let mut dir = lfn_pieces(&u16s("Track 01.bin"), short_checksum(&a));
    dir.push([0u8; 32]);
    dir.push(short_entry(&a, 0x20, 5, 10)); // beyond the end marker
    assert_eq!(run(&mut matcher("Track 01.bin"), &dir), Vec::<bool>::new());
}

#[test]
fn only_the_entry_right_after_the_run_matches() {
    let a = alias("TRACK0~1", "BIN");
    let b = alias("OTHER   ", "BIN");
    let mut dir = file("Track 01.bin", &a);
    dir.push(short_entry(&b, 0x20, 6, 10));
    assert_eq!(run(&mut matcher("Track 01.bin"), &dir), vec![true, false]);
}

#[test]
fn picks_the_right_file_among_many() {
    let mut dir = Vec::new();
    dir.push(short_entry(b"MYDISC     ", 0x08, 0, 0)); // volume label
    dir.push(short_entry(b"DISC    GDI", 0x20, 3, 100)); // plain 8.3 file
    for i in 1..=9u8 {
        let a = alias(&format!("TRACK0~{i}"), "BIN");
        dir.extend(file(&format!("Track 0{i}.bin"), &a));
    }
    let hits = run(&mut matcher("Track 07.bin"), &dir);
    assert_eq!(hits.len(), 10);
    let which: Vec<usize> = hits.iter().enumerate().filter(|(_, &h)| h).map(|(i, _)| i).collect();
    assert_eq!(which, vec![1 + 6]);
}

#[test]
fn directories_and_dot_entries_pass_through() {
    let a = alias("SUBDIR~1", "");
    let mut dir = vec![short_entry(b".          ", 0x10, 2, 0), short_entry(b"..         ", 0x10, 0, 0)];
    dir.extend(file("My Sub Dir", &a));
    assert_eq!(run(&mut matcher("my sub dir"), &dir), vec![false, false, true]);
}

#[test]
fn volume_label_is_not_an_entry() {
    let mut m = matcher("x");
    assert_eq!(m.feed(&short_entry(b"LABEL      ", 0x08, 0, 0)), Feed::Other);
}

#[test]
fn bad_type_or_cluster_field_is_not_a_piece() {
    let a = alias("TRACK0~1", "BIN");
    for (idx, val) in [(12usize, 1u8), (26, 1), (27, 1)] {
        let mut dir = lfn_pieces(&u16s("Track 01.bin"), short_checksum(&a));
        dir[0][idx] = val;
        dir.push(short_entry(&a, 0x20, 5, 10));
        assert_eq!(run(&mut matcher("Track 01.bin"), &dir), vec![false], "byte {idx}");
    }
}

#[test]
fn ordinal_zero_and_above_twenty_are_rejected() {
    let a = alias("TRACK0~1", "BIN");
    for ord in [0x40u8, 0x40 | 21] {
        let mut dir = lfn_pieces(&u16s("Track 01.bin"), short_checksum(&a));
        dir[0][0] = ord;
        dir.push(short_entry(&a, 0x20, 5, 10));
        assert_eq!(run(&mut matcher("Track 01.bin"), &dir), vec![false]);
    }
}

#[test]
fn full_length_names() {
    // 255 characters is 19 pieces of 13 plus 8: the last piece is a terminator
    // and padding.
    let a = alias("LONGLO~1", "BIN");
    let long: String = (0..255).map(|i| (b'a' + (i % 26) as u8) as char).collect();
    assert_eq!(long.len(), 255);
    let dir = file(&long, &a);
    assert_eq!(dir.len(), 20 + 1);
    assert_eq!(run(&mut matcher(&long), &dir), vec![true]);
    // One character different at either end or in the middle.
    for pos in [0usize, 127, 254] {
        let mut s = long.clone().into_bytes();
        s[pos] = b'#';
        let s = String::from_utf8(s).unwrap();
        assert_eq!(run(&mut matcher(&s), &dir), vec![false], "pos {pos}");
    }
    // 254 and 256 characters do not match a 255-character name.
    assert_eq!(run(&mut matcher(&long[..254]), &dir), vec![false]);
    // (256 characters cannot exist on a FAT volume; the matcher refuses it.)
    assert!(LfnMatcher::<255>::from_str(&format!("{long}z")).is_none());
}

#[test]
fn exact_multiples_of_a_piece_have_no_terminator() {
    // 13, 26 and 247 (= 19 * 13) characters fill their last piece completely.
    let a = alias("EXACT~1", "BIN");
    for len in [13usize, 26, 247] {
        let long: String = (0..len).map(|i| (b'A' + (i % 26) as u8) as char).collect();
        let dir = file(&long, &a);
        assert_eq!(dir.len(), len / 13 + 1);
        assert_eq!(run(&mut matcher(&long), &dir), vec![true], "len {len}");
        // One longer must not match a name that fills its last piece exactly.
        assert_eq!(run(&mut matcher(&format!("{long}x")), &dir), vec![false], "len {len}+1");
        assert_eq!(run(&mut matcher(&long[..len - 1]), &dir), vec![false], "len {len}-1");
    }
}

#[test]
fn names_longer_than_the_matcher_are_refused() {
    assert!(LfnMatcher::<32>::from_str(&"a".repeat(33)).is_none());
    assert!(LfnMatcher::<32>::from_str(&"a".repeat(32)).is_some());
    assert!(LfnMatcher::<32>::from_str("").is_none());
    assert!(LfnMatcher::<300>::from_str(&"a".repeat(256)).is_none());
}

#[test]
fn run_split_across_sectors_and_clusters() {
    // The caller reads a directory 16 entries per sector and moves to a new
    // cluster at arbitrary points; the matcher sees one continuous stream.
    // Slide the run across every position of a 16-entry sector.
    let a = alias("LONGFI~1", "BIN");
    let name = "abcdefghijklmnopqrstuvwxyz0123456789.bin"; // 40 chars, 4 pieces
    for lead in 0..40 {
        let mut dir: Vec<[u8; 32]> =
            (0..lead).map(|i| short_entry(&alias(&format!("F{i:07}"), "DAT"), 0x20, 9, 1)).collect();
        dir.extend(file(name, &a));
        dir.push(short_entry(b"AFTER      ", 0x20, 7, 1));
        while dir.len() % 16 != 0 {
            dir.push([0u8; 32]);
        }
        let mut m = matcher(name);
        let mut hits = Vec::new();
        'outer: for sector in dir.chunks(16) {
            // (A new cluster would start here every 8 sectors; nothing about
            // the matcher depends on it.)
            for e in sector {
                match m.feed(e) {
                    Feed::End => break 'outer,
                    Feed::Other => {}
                    Feed::Entry { long } => hits.push(long),
                }
            }
        }
        let n_hits = hits.iter().filter(|&&h| h).count();
        assert_eq!(n_hits, 1, "lead {lead}");
        assert!(hits[lead], "lead {lead}");
    }
}

#[test]
fn non_ascii_names() {
    let a = alias("CAFE~1", "BIN");
    let dir = file("Café ñ.bin", &a);
    assert_eq!(run(&mut matcher("Café ñ.bin"), &dir), vec![true]);
    // ASCII letters fold, accented ones do not (documented limitation).
    assert_eq!(run(&mut matcher("CAFé Ñ.BIN"), &dir), vec![false]);
    assert_eq!(run(&mut matcher("CAFé ñ.BIN"), &dir), vec![true]);
    // A character outside the BMP is a surrogate pair on disk.
    let dir = file("disc \u{1F4BF}.bin", &a);
    assert_eq!(run(&mut matcher("disc \u{1F4BF}.bin"), &dir), vec![true]);
    assert_eq!(run(&mut matcher("disc \u{1F4C0}.bin"), &dir), vec![false]);
}

#[test]
fn gdi_names_arrive_as_widened_utf8() {
    // `é` written as UTF-8 in the GDI is two bytes, widened to two chars.
    let widened: String = "Café.bin".bytes().map(|b| b as char).collect();
    assert_ne!(widened, "Café.bin");
    let a = alias("CAFE~1", "BIN");
    let dir = file("Café.bin", &a);
    let mut m = LfnMatcher::<32>::from_gdi_name(&widened).unwrap();
    assert_eq!(run(&mut m, &dir), vec![true]);
    // Latin-1 bytes that are not valid UTF-8 are taken as Latin-1.
    let latin1: String = [b'C', b'a', b'f', 0xE9, b'.', b'b', b'i', b'n'].iter().map(|&b| b as char).collect();
    let mut m = LfnMatcher::<32>::from_gdi_name(&latin1).unwrap();
    assert_eq!(run(&mut m, &dir), vec![true]);
    // Plain ASCII is untouched.
    let mut m = LfnMatcher::<32>::from_gdi_name("Track 01.bin").unwrap();
    let dir = file("track 01.BIN", &alias("TRACK0~1", "BIN"));
    assert_eq!(run(&mut m, &dir), vec![true]);
}

#[test]
fn padding_with_zero_is_tolerated() {
    let a = alias("TRACK0~1", "BIN");
    let mut dir = file("Track 1.bin", &a); // 11 chars: terminator, then 1 pad
    // Some writers pad with 0x0000 instead of 0xFFFF.
    dir[0][OFFS[12]] = 0;
    dir[0][OFFS[12] + 1] = 0;
    assert_eq!(run(&mut matcher("Track 1.bin"), &dir), vec![true]);
}

#[test]
fn garbage_after_the_terminator_is_rejected() {
    let a = alias("TRACK0~1", "BIN");
    let mut dir = file("Track 1.bin", &a);
    dir[0][OFFS[12]] = b'x';
    dir[0][OFFS[12] + 1] = 0;
    assert_eq!(run(&mut matcher("Track 1.bin"), &dir), vec![false]);
}

#[test]
fn matcher_is_reusable_after_reset() {
    let a = alias("TRACK0~1", "BIN");
    let dir = file("Track 01.bin", &a);
    let mut m = matcher("Track 01.bin");
    assert_eq!(run(&mut m, &dir), vec![true]);
    m.reset();
    assert_eq!(run(&mut m, &dir), vec![true]);
    // A truncated run left over from an earlier directory must not complete
    // with the next one's alias.
    let mut m = matcher("Track 01.bin");
    let _ = run(&mut m, &lfn_pieces(&u16s("Track 01.bin"), short_checksum(&a)));
    m.reset();
    assert_eq!(m.feed(&short_entry(&a, 0x20, 5, 10)), Feed::Entry { long: false });
}

#[test]
fn display_83_formats_names() {
    let mut out = [0u8; 12];
    let n = display_83(b"TRACK0~1BIN", &mut out);
    assert_eq!(&out[..n], b"TRACK0~1.BIN");
    let n = display_83(b"README  TXT", &mut out);
    assert_eq!(&out[..n], b"README.TXT");
    let n = display_83(b"SUBDIR     ", &mut out);
    assert_eq!(&out[..n], b"SUBDIR");
    let n = display_83(b"A       B  ", &mut out);
    assert_eq!(&out[..n], b"A.B");
}

#[test]
fn gdi_name_decoding_edge_cases() {
    let widen = |bytes: &[u8]| -> String { bytes.iter().map(|&b| b as char).collect() };
    let mut out = [0u16; 16];
    // 2-, 3- and 4-byte sequences
    let n = utf16_from_gdi_name(&widen("é€\u{1F4BF}".as_bytes()), &mut out).unwrap();
    assert_eq!(&out[..n], &"é€\u{1F4BF}".encode_utf16().collect::<Vec<_>>()[..]);
    // invalid UTF-8 (lone continuation, overlong C0 80, surrogate ED A0 80,
    // truncated sequence) falls back to Latin-1, one unit per byte
    for bad in [&[0x80u8][..], &[0xC0, 0x80], &[0xED, 0xA0, 0x80], &[0xE2, 0x82], &[0xF5, 0x80, 0x80, 0x80]] {
        let n = utf16_from_gdi_name(&widen(bad), &mut out).unwrap();
        let want: Vec<u16> = bad.iter().map(|&b| b as u16).collect();
        assert_eq!(&out[..n], &want[..], "{bad:02x?}");
    }
    // too small an output
    let mut tiny = [0u16; 2];
    assert!(utf16_from_gdi_name("abc", &mut tiny).is_none());
    // a char above U+00FF cannot be a widened byte: taken as-is
    let n = utf16_from_gdi_name("a\u{4E2D}", &mut out).unwrap();
    assert_eq!(&out[..n], &[0x61, 0x4E2D]);
}
