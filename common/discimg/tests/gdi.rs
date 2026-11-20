//! Characterization tests for the GDI text parser: they pin what the firmware
//! parser did when it was extracted, quirks included. A test that looks like
//! it pins a bug does -- see the comments.

mod oracle;

use discimg::gdi::{parse, parse_vec};
use discimg::{Error, Full, TrackDesc, TrackSink, VecSink};

fn tr(number: u8, start_lba: u32, ctrl: u8, sector_size: u16, name: &str) -> TrackDesc {
    TrackDesc {
        number,
        start_lba,
        ctrl,
        sector_size,
        sector_count: 0,
        filename: name.try_into().unwrap(),
        file_offset: 0,
        session: 0,
    }
}

const GDI3: &str = "3\n\
1 0 4 2352 track01.bin 0\n\
2 450 0 2352 track02.raw 0\n\
3 45000 4 2352 track03.bin 0\n";

const GDI4: &str = "4\n\
1 0 4 2352 track01.bin 0\n\
2 600 0 2352 track02.raw 0\n\
3 45000 4 2352 track03.bin 0\n\
4 549150 0 2352 track04.raw 0\n";

#[test]
fn three_track_layout() {
    let t = parse_vec(GDI3.as_bytes()).unwrap();
    assert_eq!(
        t.as_slice(),
        &[
            tr(1, 0, 4, 2352, "track01.bin"),
            tr(2, 450, 0, 2352, "track02.raw"),
            tr(3, 45000, 4, 2352, "track03.bin"),
        ]
    );
}

#[test]
fn four_track_layout_from_the_module_doc() {
    let t = parse_vec(GDI4.as_bytes()).unwrap();
    assert_eq!(t.len(), 4);
    assert_eq!(t[1], tr(2, 600, 0, 2352, "track02.raw"));
    assert_eq!(t[3], tr(4, 549150, 0, 2352, "track04.raw"));
}

#[test]
fn crlf_and_no_trailing_newline() {
    let crlf = GDI3.replace('\n', "\r\n");
    assert_eq!(parse_vec(crlf.as_bytes()), parse_vec(GDI3.as_bytes()));
    let no_nl = GDI3.trim_end();
    assert_eq!(parse_vec(no_nl.as_bytes()), parse_vec(GDI3.as_bytes()));
}

#[test]
fn blank_lines_whitespace_and_tabs_between_tracks_are_ignored() {
    let txt = "3\r\n\r\n  1\t0   4 2352\ttrack01.bin 0  \r\n   \r\n2 450 0 2352 track02.raw 0\n\n3 45000 4 2352 track03.bin 0\n\n";
    assert_eq!(parse_vec(txt.as_bytes()), parse_vec(GDI3.as_bytes()));
}

#[test]
fn pregap_column_is_ignored_and_optional() {
    let txt = "1\n1 0 4 2352 track01.bin\n";
    assert_eq!(parse_vec(txt.as_bytes()).unwrap().len(), 1);
    let txt = "1\n1 0 4 2352 track01.bin 150 trailing junk\n";
    assert_eq!(parse_vec(txt.as_bytes()).unwrap()[0].file_offset, 0);
}

#[test]
fn declared_track_count_is_not_checked() {
    // The header number must parse but is otherwise unused.
    let txt = "99\n1 0 4 2352 track01.bin 0\n";
    assert_eq!(parse_vec(txt.as_bytes()).unwrap().len(), 1);
    let txt = "1\n1 0 4 2352 a.bin 0\n2 450 0 2352 b.bin 0\n";
    assert_eq!(parse_vec(txt.as_bytes()).unwrap().len(), 2);
}

#[test]
fn quoted_filename_without_spaces_loses_its_quotes() {
    let txt = "1\n1 0 4 2352 \"track01.bin\" 0\n";
    assert_eq!(parse_vec(txt.as_bytes()).unwrap()[0].filename.as_str(), "track01.bin");
}

#[test]
fn quoted_filename_with_spaces_is_one_name() {
    // Some dumpers quote names that contain blanks.
    let txt = "1\n1 0 4 2352 \"Track 01.bin\" 0\n";
    assert_eq!(parse_vec(txt.as_bytes()).unwrap()[0], tr(1, 0, 4, 2352, "Track 01.bin"));
    // Several blanks, a tab, no pregap column.
    let t = parse_vec(b"1\n1 0 4 2352 \"a  b\tc.bin\"\n").unwrap();
    assert_eq!(t[0].filename.as_str(), "a  b\tc.bin");
    // Blanks inside the quotes are kept, even at the ends.
    let t = parse_vec(b"1\n1 0 4 2352 \" x.bin \" 0\n").unwrap();
    assert_eq!(t[0].filename.as_str(), " x.bin ");
    // Extra trailing fields (quoted too) are ignored.
    let t = parse_vec(b"1\n1 0 4 2352 \"T 1.bin\" 150 junk \"more junk\"\n").unwrap();
    assert_eq!(t[0].filename.as_str(), "T 1.bin");
}

#[test]
fn unterminated_and_empty_quoted_names() {
    // No closing quote: the name runs to the end of the line (CR trimmed).
    let t = parse_vec(b"1\r\n1 0 4 2352 \"Track 01.bin\r\n").unwrap();
    assert_eq!(t[0].filename.as_str(), "Track 01.bin");
    // An empty name is passed on as is; the sink decides what to do with it.
    let t = parse_vec(b"1\n1 0 4 2352 \"\" 0\n").unwrap();
    assert_eq!(t[0].filename.as_str(), "");
}

#[test]
fn quoted_name_over_the_length_limit_fails() {
    let long = format!("1\n1 0 4 2352 \"{} x\" 0\n", "a".repeat(discimg::MAX_NAME - 1));
    assert_eq!(parse_vec(long.as_bytes()), Err(Error::NameTooLong));
    // Exactly at the limit is fine.
    let ok = format!("1\n1 0 4 2352 \"{}\" 0\n", "a".repeat(discimg::MAX_NAME));
    assert_eq!(parse_vec(ok.as_bytes()).unwrap()[0].filename.len(), discimg::MAX_NAME);
    // A byte above 0x7F is widened to a two-byte char: 65 of them do not fit.
    let mut raw = b"1\n1 0 4 2352 \"".to_vec();
    raw.extend(std::iter::repeat(0xE9u8).take(65));
    raw.extend_from_slice(b"\" 0\n");
    assert_eq!(parse_vec(&raw), Err(Error::NameTooLong));
}

#[test]
fn redump_style_long_names_parse() {
    let gdi = "3\n\
        1 0 4 2352 \"Crazy Taxi (Europe) (En,Fr,De,Es) (Track 01).bin\" 0\n\
        2 600 0 2352 \"Crazy Taxi (Europe) (En,Fr,De,Es) (Track 02).raw\" 0\n\
        3 45000 4 2352 \"Crazy Taxi (Europe) (En,Fr,De,Es) (Track 03).bin\" 0\n";
    let t = parse_vec(gdi.as_bytes()).unwrap();
    assert_eq!(t.len(), 3);
    assert_eq!(t[0].filename.as_str(), "Crazy Taxi (Europe) (En,Fr,De,Es) (Track 01).bin");
    assert_eq!(t[1].filename.as_str(), "Crazy Taxi (Europe) (En,Fr,De,Es) (Track 02).raw");
    assert!(t[0].filename.len() > 32);
}

#[test]
fn bom_trailing_blanks_and_mixed_line_endings() {
    let mut txt = b"\xEF\xBB\xBF3 \t\r\n".to_vec();
    txt.extend_from_slice(b"1 0 4 2352 \"track01.bin\" 0 \t\r\n2 450 0 2352 track02.raw 0\n");
    txt.extend_from_slice(b"3 45000 4 2352 \"track03.bin\" 0\r\n\r\n");
    assert_eq!(parse_vec(&txt), parse_vec(GDI3.as_bytes()));
}

#[test]
fn name_case_is_preserved() {
    let t = parse_vec(b"1\n1 0 4 2352 TRACK01.BIN 0\n").unwrap();
    assert_eq!(t[0].filename.as_str(), "TRACK01.BIN");
}

#[test]
fn numbers_stop_at_the_first_non_digit_and_are_range_checked() {
    // "4abc" parses as 4: parse_u32 stops at a non-digit once it has one.
    let t = parse_vec(b"1\n1 0 4abc 2352 t.bin 0\n").unwrap();
    assert_eq!(t[0].ctrl, 4);
    // R89 C6: track number, ctrl and sector size are no longer cast down
    // without a range check (257 -> 1, 260 -> 4, 65537 -> 1 used to parse).
    assert_eq!(parse_vec(b"1\n257 0 260 65537 t.bin 0\n"), Err(Error::Parse));
}

#[test]
fn errors() {
    let p = |s: &str| parse_vec(s.as_bytes());
    assert_eq!(p(""), Err(Error::Parse));
    // R89 C6: leading blank lines before the count are skipped, as the Python
    // tool does (they used to make the header empty, which is not a number).
    assert_eq!(p("\n3\n1 0 4 2352 a.bin 0\n").unwrap().len(), 1);
    assert_eq!(p("three\n"), Err(Error::Parse));
    assert_eq!(p("-1\n"), Err(Error::Parse));
    // Header alone is a valid, empty table.
    assert_eq!(p("0\n").unwrap().len(), 0);
    assert_eq!(p("1\n1 0 4 2352\n"), Err(Error::Parse)); // no filename
    assert_eq!(p("1\n1 0 4\n"), Err(Error::Parse));
    assert_eq!(p("1\nx 0 4 2352 a.bin 0\n"), Err(Error::Parse));
    assert_eq!(p("1\n1 0 4 2352 a.bin 0\n2 450 0\n"), Err(Error::Parse));
    // u32 overflow in a field.
    assert_eq!(p("1\n1 4294967296 4 2352 a.bin 0\n"), Err(Error::Parse));
    assert_eq!(p("1\n1 4294967295 4 2352 a.bin 0\n").unwrap()[0].start_lba, u32::MAX);
}

#[test]
fn filename_length_limit_counts_utf8_bytes_of_widened_bytes() {
    let max = discimg::MAX_NAME;
    let ok = format!("1
1 0 4 2352 {} 0
", "a".repeat(max));
    let bad = format!("1
1 0 4 2352 {} 0
", "a".repeat(max + 1));
    assert_eq!(parse_vec(ok.as_bytes()).unwrap()[0].filename.len(), max);
    assert_eq!(parse_vec(bad.as_bytes()), Err(Error::NameTooLong));
    // Quotes do not count towards the limit.
    let quoted = format!("1
1 0 4 2352 \"{}\" 0
", "a".repeat(max));
    assert_eq!(parse_vec(quoted.as_bytes()).unwrap()[0].filename.len(), max);
    // A byte >= 0x80 is widened to a char and so takes two bytes: half fit, one more does not.
    let mut b16 = b"1
1 0 4 2352 ".to_vec();
    b16.extend(core::iter::repeat(0xE9u8).take(max / 2));
    b16.extend_from_slice(b" 0
");
    assert_eq!(parse_vec(&b16).unwrap()[0].filename.len(), max);
    let mut b17 = b"1
1 0 4 2352 ".to_vec();
    b17.extend(core::iter::repeat(0xE9u8).take(max / 2 + 1));
    b17.extend_from_slice(b" 0
");
    assert_eq!(parse_vec(&b17), Err(Error::NameTooLong));
}

#[test]
fn track_capacity_is_99() {
    let mut txt = String::from("99\n");
    for n in 1..=99 {
        txt.push_str(&format!("{} {} 0 2352 t{}.raw 0\n", n, n * 100, n));
    }
    assert_eq!(parse_vec(txt.as_bytes()).unwrap().len(), 99);
    txt.push_str("100 99999 0 2352 t100.raw 0\n");
    assert_eq!(parse_vec(txt.as_bytes()), Err(Error::Sink(Full)));
}

#[test]
fn sink_is_cleared_first_and_keeps_the_partial_table_on_failure() {
    let mut s = VecSink::new();
    parse(GDI4.as_bytes(), &mut s).unwrap();
    assert_eq!(s.0.len(), 4);
    // Fails on line 3; the tracks before it have already been pushed. The
    // firmware relies on a failed mount clearing the mounted image instead.
    let r = parse(b"3\n1 0 4 2352 a.bin 0\n2 450 0 2352 b.bin 0\n3 oops\n", &mut s);
    assert_eq!(r, Err(Error::Parse));
    assert_eq!(s.0.len(), 2);
    // Header failure still clears.
    assert_eq!(parse(b"nope", &mut s), Err(Error::Parse));
    assert_eq!(s.0.len(), 0);
}

struct Reject(u8);
impl TrackSink for Reject {
    type Error = &'static str;
    fn clear(&mut self) {}
    fn push(&mut self, t: &TrackDesc) -> Result<(), &'static str> {
        if t.number == self.0 { Err("rejected") } else { Ok(()) }
    }
}

#[test]
fn sink_errors_pass_through_in_line_order() {
    // The firmware's sink fails with BadFilename here; a later malformed line
    // must not mask it (the original parser converted each name as it went).
    let txt = b"3\n1 0 4 2352 a.bin 0\n2 450 0 2352 b.bin 0\n3 garbage\n";
    assert_eq!(parse(txt, &mut Reject(2)), Err(Error::Sink("rejected")));
    assert_eq!(parse(txt, &mut Reject(9)), Err(Error::Parse));
}

#[test]
fn agrees_with_the_original_parser_on_a_corpus() {
    let corpus: &[&[u8]] = &[
        GDI3.as_bytes(),
        GDI4.as_bytes(),
        b"",
        b"\n",
        b"1\n",
        b"1\r\n1 0 4 2352 \"a.bin\" 0\r\n",
        b"2\n  1 0 4 2352 a.bin\n\t2 5 0 2336 b.bin 0 x\n",
        b"1\n1 0 4 2352 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 0\n",
        b"1\n1 99999999999 4 2352 a.bin 0\n",
        b"1\n1 0 4 2352\n",
        b"abc\n",
        b"3\n1 0 4 2352 a.bin 0\n3 oops\n",
    ];
    for &c in corpus {
        let new = parse_vec(c);
        let old = oracle::parse(c);
        match (new, old) {
            (Ok(n), Ok(o)) => {
                assert_eq!(n.len(), o.len(), "{:?}", String::from_utf8_lossy(c));
                for (a, b) in n.iter().zip(o.iter()) {
                    assert_eq!(
                        (a.number, a.start_lba, a.ctrl, a.sector_size, a.filename.as_str()),
                        (b.number, b.start_lba, b.ctrl, b.sector_size, b.filename.as_str())
                    );
                }
            }
            (Err(Error::Parse), Err(oracle::GdiError::Parse)) => {}
            (Err(Error::NameTooLong), Err(oracle::GdiError::Parse)) => {}
            (Err(Error::Sink(Full)), Err(oracle::GdiError::TooManyTracks)) => {}
            (n, o) => panic!("mismatch on {:?}: {:?} vs {:?}", String::from_utf8_lossy(c), n, o),
        }
    }
}

// ---- R89 A5 / C6: field ranges, sector sizes, leading blank lines ---------

#[test]
fn sector_size_must_be_a_known_one() {
    for bad in ["0", "512", "2049", "2353", "65535"] {
        let g = format!("1\n1 0 4 {} t.bin 0\n", bad);
        assert_eq!(parse_vec(g.as_bytes()), Err(Error::Parse), "sector size {}", bad);
    }
    for ok in [2048u16, 2336, 2352] {
        let g = format!("1\n1 0 4 {} t.bin 0\n", ok);
        assert_eq!(parse_vec(g.as_bytes()).unwrap()[0].sector_size, ok);
    }
}

#[test]
fn fields_that_overflow_their_type_are_rejected_not_truncated() {
    // 65536 + 2352 truncates to 2352 under `as u16`.
    assert_eq!(parse_vec(b"1\n1 0 4 67888 t.bin 0\n"), Err(Error::Parse));
    // track number 257 truncates to 1 under `as u8`.
    assert_eq!(parse_vec(b"1\n257 0 4 2352 t.bin 0\n"), Err(Error::Parse));
    // control 260 truncates to 4.
    assert_eq!(parse_vec(b"1\n1 0 260 2352 t.bin 0\n"), Err(Error::Parse));
    assert!(parse_vec(b"1\n255 0 4 2352 t.bin 0\n").is_ok());
}

#[test]
fn leading_blank_lines_and_spaces_before_the_count() {
    let want = parse_vec(GDI3.as_bytes()).unwrap();
    let with = format!("\n  \r\n\t\n   {}", GDI3);
    assert_eq!(parse_vec(with.as_bytes()).unwrap(), want);
    // Only blanks: still no count.
    assert_eq!(parse_vec(b"\n  \n"), Err(Error::Parse));
}
