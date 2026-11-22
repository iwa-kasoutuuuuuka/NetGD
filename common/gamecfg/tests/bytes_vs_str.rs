//! The byte-level parsers against the `&str` ones they replaced.
//!
//! `Meta::parse`, `Ini`'s line parser and `resolve` used to work on `&str`
//! (`lines`, `split_once`, `trim_matches`, `chars`, `from_utf8`); they now work
//! on bytes so the firmware links none of that. The old code is copied here
//! verbatim as the reference, and random files built from the characters
//! that matter (delimiters, whitespace of every kind, quotes, comments,
//! multi-byte UTF-8, Unicode spaces, invalid bytes, long values) must parse
//! the same.

use gamecfg::ini::Ini;
use gamecfg::{Kind, Meta};
use heapless::String as HString;

// ---- the previous implementation -----------------------------------------

fn old_trim(s: &str) -> &str {
    let b = s.as_bytes();
    let ws = |c: u8| c == b' ' || (b'\t'..=b'\r').contains(&c);
    let mut start = 0;
    let mut end = b.len();
    while start < end && ws(b[start]) {
        start += 1;
    }
    while end > start && ws(b[end - 1]) {
        end -= 1;
    }
    s.get(start..end).unwrap_or(s)
}

fn old_strip_comment(line: &str) -> &str {
    let mut in_quotes = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            '#' if !in_quotes => return &line[..i],
            _ => {}
        }
    }
    line
}

fn old_unquote(v: &str) -> Option<&str> {
    let v = old_trim(v);
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        Some(&v[1..v.len() - 1])
    } else if v.is_empty() || v.starts_with('"') {
        None
    } else {
        Some(v)
    }
}

fn old_fit<const N: usize>(s: &str) -> Option<HString<N>> {
    let mut out: HString<N> = HString::new();
    for c in s.chars() {
        if out.push(c).is_err() {
            break;
        }
    }
    Some(out)
}

fn old_meta_parse(text: &str) -> Meta {
    let mut out = Meta::default();
    for line in text.lines() {
        let line = old_trim(old_strip_comment(line));
        if line.is_empty() || line.starts_with('[') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = old_trim(key);
        let value = old_trim(value);
        match key {
            "title" => out.title = old_unquote(value).and_then(old_fit),
            "boot" => out.boot = old_unquote(value).and_then(old_fit),
            "next" | "next_disk" | "next_disc" => out.next = old_unquote(value).and_then(old_fit),
            "type" => {
                if let Some(v) = old_unquote(value) {
                    out.kind = match v {
                        "elf" | "bin" | "prog" => Kind::Elf,
                        _ => Kind::Disc,
                    };
                }
            }
            _ => {}
        }
    }
    out
}

/// What the firmware did with the raw file.
fn old_meta_from_bytes(data: &[u8]) -> Meta {
    old_meta_parse(core::str::from_utf8(data).unwrap_or(""))
}

fn old_num(v: &str) -> Option<i32> {
    let v = old_trim(old_trim(v).trim_matches('"'));
    let (neg, digits) = match v.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, v.strip_prefix('+').unwrap_or(v)),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) || digits.len() > 9 {
        return None;
    }
    let n: i32 = digits.parse().ok()?;
    Some(if neg { -n } else { n })
}

fn old_ini_line(out: &mut Ini, mut line: &[u8], first: bool) {
    use gamecfg::ini::*;
    let clamp = |v: i32, lo: i32, hi: i32| v.max(lo).min(hi);
    if first {
        if let Some(rest) = line.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
            line = rest;
        }
    }
    let code = match line.iter().position(|&b| b == b'#' || b == b';') {
        Some(i) => &line[..i],
        None => line,
    };
    if !code.is_ascii() {
        return;
    }
    let Ok(line) = core::str::from_utf8(code) else { return };
    let line = old_trim(line);
    if line.is_empty() || line.starts_with('[') {
        return;
    }
    let Some((key, value)) = line.split_once('=') else {
        return;
    };
    let (key, value) = (old_trim(key), old_trim(value));
    if key.eq_ignore_ascii_case("open_time") {
        if let Some(v) = old_num(value) {
            out.open_time = clamp(v, TIME_MIN as i32, TIME_MAX as i32) as u16;
            out.open_time_set = true;
        }
    } else if key.eq_ignore_ascii_case("detect_time") {
        if let Some(v) = old_num(value) {
            out.detect_time = clamp(v, TIME_MIN as i32, TIME_MAX as i32) as u16;
            out.detect_time_set = true;
        }
    } else if key.eq_ignore_ascii_case("image_tests") {
        if let Some(v) = old_num(value) {
            out.image_tests = v != 0;
        }
    } else if key.eq_ignore_ascii_case("high_speed") {
        if let Some(v) = old_num(value) {
            out.high_speed = v != 0;
        }
    } else if key.eq_ignore_ascii_case("reset_goto") {
        if let Some(v) = old_num(value) {
            out.reset_goto = if (0..=RESET_GOTO_MAX as i32).contains(&v) { v as u16 } else { 0 };
        }
    } else if key.eq_ignore_ascii_case("read_limit") {
        if let Some(v) = old_num(value) {
            out.read_limit = match v {
                0 => 0,
                v if v < 0 => -1,
                v => clamp(v, READ_LIMIT_MIN as i32, READ_LIMIT_MAX as i32) as i16,
            };
        }
    } else {
        out.unknown = out.unknown.saturating_add(1);
    }
}

fn old_ini_parse_bytes(data: &[u8]) -> Ini {
    let mut out = Ini::new();
    let mut first = true;
    for line in data.split(|&b| b == b'\r' || b == b'\n') {
        old_ini_line(&mut out, line, first);
        first = false;
    }
    out
}

fn old_resolve<'a>(names: impl Iterator<Item = &'a str>, want: &str) -> Option<usize> {
    let want = old_trim(want).trim_end_matches('/');
    names.enumerate().find_map(|(i, n)| {
        (n.len() == want.len()
            && n.chars().zip(want.chars()).all(|(a, b)| a.eq_ignore_ascii_case(&b)))
        .then_some(i)
    })
}

// ---- generators -----------------------------------------------------------

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.next() as usize % xs.len()]
    }
}

/// Pieces a random file is built from. Byte strings, so invalid UTF-8 can
/// be among them.
const PIECES: &[&[u8]] = &[
    b"title", b"next", b"next_disk", b"next_disc", b"boot", b"type", b"Title", b"elf", b"bin",
    b"prog", b"disc", b"open_time", b"DETECT_TIME", b"image_tests", b"high_speed", b"reset_goto",
    b"read_limit", b"foo", b"=", b"=", b"\"", b"\"", b"#", b";", b"[", b"]", b" ", b" ", b"\t",
    b"\x0b", b"\x0c", b"\r", b"\n", b"\r\n", b"0", b"1", b"42", b"700", b"123456789",
    b"1234567890", b"-", b"+", b"/", b"Disc 2", b"Skies of Arcadia", "é".as_bytes(),
    "日本語".as_bytes(), "\u{3000}".as_bytes(), "\u{a0}".as_bytes(), "\u{1F4BF}".as_bytes(),
    b"\xff", b"\x80", b"\xe3\x81", b"\xEF\xBB\xBF",
];

fn random_file(r: &mut Rng) -> Vec<u8> {
    let mut v = Vec::new();
    let n = r.next() % 40;
    for _ in 0..n {
        v.extend_from_slice(r.pick(PIECES));
    }
    v
}

/// A long value with multi-byte chars, to cross each `fit` limit.
fn long_value(r: &mut Rng) -> Vec<u8> {
    let mut v = Vec::new();
    let n = r.next() % 120;
    for _ in 0..n {
        v.extend_from_slice(r.pick(&[&b"a"[..], b"Z", b" ", "é".as_bytes(), "日".as_bytes(), "\u{1F4BF}".as_bytes()]));
    }
    v
}

fn same_meta(a: &Meta, b: &Meta) -> bool {
    a.title == b.title && a.next == b.next && a.boot == b.boot && a.kind == b.kind
}

// ---- the comparisons ------------------------------------------------------

#[test]
fn meta_parse_matches_the_str_parser() {
    let mut r = Rng(1);
    let mut set = 0;
    for i in 0..200_000 {
        let f = if i % 2 == 0 { random_file(&mut r) } else { structured_file(&mut r) };
        let (a, b) = (Meta::parse_bytes(&f), old_meta_from_bytes(&f));
        assert!(same_meta(&a, &b), "{:?}\nnew {a:?}\nold {b:?}", String::from_utf8_lossy(&f));
        if let Ok(s) = core::str::from_utf8(&f) {
            assert!(same_meta(&Meta::parse(s), &b), "{s:?}");
        }
        set += (b.title.is_some() || b.next.is_some() || b.boot.is_some() || b.kind == Kind::Elf) as u32;
    }
    // The generator must actually reach the assignments.
    assert!(set > 5_000, "only {set} files set anything");
}

/// Lines shaped like `key = value # comment`, each part drawn with noise.
fn structured_file(r: &mut Rng) -> Vec<u8> {
    const WS: &[&[u8]] = &[b"", b"", b" ", b"\t", b"  ", b"\x0b", b"\x0c", "\u{3000}".as_bytes()];
    const KEYS: &[&[u8]] = &[
        b"title", b"next", b"next_disk", b"boot", b"type", b"open_time", b"Detect_Time",
        b"reset_goto", b"read_limit", b"image_tests", b"high_speed", b"x", b"[sec]", b"",
    ];
    const VALS: &[&[u8]] = &[
        b"\"Disc 2\"", b"Disc 2", b"\"elf\"", b"elf", b"prog", b"\"\"", b"\"", b"700", b"\"700\"",
        b"-1", b"+12", b"\"\"5\"\"", b"1234567890", b"abc", "\"日本語 é\"".as_bytes(), b"a=b",
        b"\"a # b\"", b"\xff", b"\"\x82\xa0\"", b"",
    ];
    const ENDS: &[&[u8]] = &[b"\n", b"\r\n", b"\r", b"", b" # c\n", b" ; c\n", b"#\x82\xa0\r\n"];
    let mut v = Vec::new();
    if r.next() % 8 == 0 {
        v.extend_from_slice(b"\xEF\xBB\xBF");
    }
    for _ in 0..r.next() % 6 {
        for part in [r.pick(WS), r.pick(KEYS), r.pick(WS), &b"="[..], r.pick(WS), r.pick(VALS), r.pick(WS), r.pick(ENDS)] {
            v.extend_from_slice(part);
        }
    }
    v
}

#[test]
fn meta_long_values_truncate_alike() {
    let mut r = Rng(2);
    for _ in 0..20_000 {
        for key in ["title", "next", "boot"] {
            for quoted in [false, true] {
                let mut f = Vec::from(format!("{key} = ").as_bytes());
                if quoted {
                    f.push(b'"');
                }
                f.extend(long_value(&mut r));
                if quoted {
                    f.push(b'"');
                }
                f.extend_from_slice(b"  # c\r\n");
                let (a, b) = (Meta::parse_bytes(&f), old_meta_from_bytes(&f));
                assert!(same_meta(&a, &b), "{:?}", String::from_utf8_lossy(&f));
            }
        }
    }
}

#[test]
fn meta_edge_cases() {
    let cases: &[&[u8]] = &[
        b"",
        b"\n\n\r\n",
        b"title=\"\"",
        b"title=\"",
        b"title=\"a",
        b"title=a\"",
        b"title = \"x\" \"y\"",
        b"title=\"a # b\" # c",
        b"title = x\rnext = y",
        b"next=Disc 2/",
        b"\ttitle\t=\tTabbed\t\r\n",
        b"\x0btitle\x0c= v\x0b",
        b"type = elf\ntype = \"x\"",
        b"type=\"\"",
        b"[title]\ntitle=t",
        b"=x\ntitle==y",
        "title = \u{3000}全角\u{3000}".as_bytes(),
        b"title = \xff\nnext = ok",
        b"title = ok\n# \x82\xa0\n",
        b"\xEF\xBB\xBFtitle = bom",
    ];
    for f in cases {
        let (a, b) = (Meta::parse_bytes(f), old_meta_from_bytes(f));
        assert!(same_meta(&a, &b), "{:?}\nnew {a:?}\nold {b:?}", String::from_utf8_lossy(f));
    }
}

#[test]
fn ini_matches_the_str_parser() {
    let mut r = Rng(3);
    let mut set = 0;
    for i in 0..200_000 {
        let f = if i % 2 == 0 { random_file(&mut r) } else { structured_file(&mut r) };
        let old = old_ini_parse_bytes(&f);
        assert_eq!(Ini::parse_bytes(&f), old, "{:?}", String::from_utf8_lossy(&f));
        set += (old != Ini { unknown: old.unknown, ..Ini::new() }) as u32;
    }
    assert!(set > 5_000, "only {set} files set a key");
    for f in [
        &b"open_time = \"\"\"700\"\"\""[..],
        b"open_time=\"-5\"",
        b"open_time=-+5",
        b"open_time=+-5",
        b"open_time=+",
        b"open_time=-",
        b"open_time=\" 700 \"",
        b"open_time=\"\t-700\t\"",
        b"open_time=123456789",
        b"open_time=1234567890",
        b"open_time=000000700",
        b"read_limit=-0",
        b"  =  \n[x\n x=1=2",
    ] {
        assert_eq!(Ini::parse_bytes(f), old_ini_parse_bytes(f), "{:?}", String::from_utf8_lossy(f));
    }
}

#[test]
fn resolve_matches_the_str_version() {
    let names: &[&str] = &[
        "LAUNCH", "Skies of Arcadia", "Skies of Arcadia Disc 2", "日本語", "Été", "été", "a/b",
        "x", "", "ab\u{e9}", "abé", "\u{3000}x",
    ];
    let wants: &[&str] = &[
        "launch", "LAUNCH/", " launch //", "\tSkies of arcadia disc 2\r", "日本語", "日本語/", "ÉTÉ",
        "Été", "été", "a/b", "a/b/", "X", "", "/", " ", "abé", "ABé", "\u{3000}x", "x\u{3000}", "Skies",
    ];
    for want in wants {
        assert_eq!(
            gamecfg::resolve(names.iter().copied(), want),
            old_resolve(names.iter().copied(), want),
            "{want:?}"
        );
        for (i, n) in names.iter().enumerate() {
            assert_eq!(
                gamecfg::name_matches(n.as_bytes(), want.as_bytes()),
                old_resolve(core::iter::once(*n), want) == Some(0),
                "{i} {n:?} {want:?}"
            );
        }
    }
}

#[test]
fn fit_matches_pushing_chars() {
    let mut r = Rng(4);
    for _ in 0..50_000 {
        let v = long_value(&mut r);
        let s = core::str::from_utf8(&v).unwrap();
        assert_eq!(gamecfg::fit::<32>(&v), old_fit::<32>(s).unwrap());
        assert_eq!(gamecfg::fit::<5>(&v), old_fit::<5>(s).unwrap());
        assert_eq!(gamecfg::fit::<96>(&v), old_fit::<96>(s).unwrap());
    }
    // Not UTF-8: empty rather than a broken String.
    assert_eq!(gamecfg::fit::<8>(b"ab\xffcd"), HString::<8>::new());
}
