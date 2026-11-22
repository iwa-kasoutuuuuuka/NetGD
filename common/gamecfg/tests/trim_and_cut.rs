//! The shared trim helper's exact byte set, the GAME.CFG 256-byte cut, and
//! the firmware's `configured_name` line scan before/after it used `trim_bytes`.
//!
//! Behaviour differences pinned here (input -> before -> after):
//! * GDEMU.CFG `image=foo<VT>` -> "foo<VT>" -> "foo"
//! * `<VT>image=foo` -> key does not match, no image -> "foo"
//! * `image<VT>=foo` -> key "image<VT>" does not match, no image -> "foo"
//! * `image=<VT>foo` -> "<VT>foo" -> "foo"
//! * `image=<VT>` -> "<VT>" -> "" (empty name)
//! * `<VT># comment` -> not a comment, no '=' ends the search -> skipped
//! * a `<VT>`-only line before `image=` -> search ended (None) -> skipped
//! * GAME.CFG > 256 B -> cut back to the last LF within the first 256 bytes
//!   (all 256 kept when byte 257 is LF; no LF at all: first 256 bytes as
//!   before). Latin-1 0xE9 at byte 255 -> all defaults -> the lines before
//!   its line parse; `next = <katakana>2` straddling 256 -> defaults -> that
//!   line dropped, earlier lines parse; a bare value cut short
//!   (`next = Disc 2 long`) -> truncated value -> line dropped.

use gamecfg::{complete_lines, trim_bytes, Meta};

#[test]
fn trim_bytes_byte_set_is_exactly_the_six_ascii_whitespace() {
    for b in 0u8..=255 {
        let ws = matches!(b, b' ' | 0x09 | 0x0A | 0x0B | 0x0C | 0x0D);
        let v = [b, b'x', b];
        assert_eq!(trim_bytes(&v), if ws { &b"x"[..] } else { &v[..] }, "byte {b:#x}");
        let one = [b];
        assert_eq!(trim_bytes(&one), if ws { &b""[..] } else { &one[..] }, "byte {b:#x}");
    }
    assert_eq!(trim_bytes(b""), b"");
    assert_eq!(trim_bytes(b" \t\r\n a b \x0B\x0C"), b"a b");
}

/// The firmware's `configured_name` scan, parameterised on the trim used.
fn image_name(buf: &[u8], trim: fn(&[u8]) -> &[u8]) -> Option<Vec<u8>> {
    for line in buf.split(|c| *c == b'\n') {
        let line = trim(line);
        if line.is_empty() || line[0] == b'#' {
            continue;
        }
        let eq = line.iter().position(|c| *c == b'=')?;
        let key = trim(&line[..eq]);
        let val = trim(&line[eq + 1..]);
        if !key.eq_ignore_ascii_case(b"image") {
            continue;
        }
        return Some(val.to_vec());
    }
    None
}

/// Previous helper: `u8::is_ascii_whitespace` (no VT).
fn old_trim_ascii(mut s: &[u8]) -> &[u8] {
    while let Some((f, rest)) = s.split_first() {
        if f.is_ascii_whitespace() { s = rest } else { break }
    }
    while let Some((l, rest)) = s.split_last() {
        if l.is_ascii_whitespace() { s = rest } else { break }
    }
    s
}

#[test]
fn gdemu_cfg_default_cards_are_unchanged() {
    let cases: &[&[u8]] = &[
        b"image=Game One\n",
        b"image=Game One",
        b"# comment\nimage = Game One\r\n",
        b"\r\n\r\n  IMAGE\t=\t Disc 2 \t\r\n",
        b"other=1\nimage=x\n",
        b"junk line\nimage=x\n",
        b"",
        b"# only comment\n",
        b"image=a b  c\r\nimage=second\r\n",
        "image=caf\u{e9}\n".as_bytes(),
    ];
    for c in cases {
        assert_eq!(image_name(c, trim_bytes), image_name(c, old_trim_ascii), "{:?}", c);
    }
    assert_eq!(image_name(b"image = Game One \r\n", trim_bytes).unwrap(), b"Game One");
}

#[test]
fn gdemu_cfg_vt_differences() {
    let b4 = |s: &[u8]| image_name(s, old_trim_ascii);
    let af = |s: &[u8]| image_name(s, trim_bytes);
    assert_eq!(b4(b"image=foo\x0B\n").unwrap(), b"foo\x0B");
    assert_eq!(af(b"image=foo\x0B\n").unwrap(), b"foo");
    assert_eq!(b4(b"\x0Bimage=foo\n"), None);
    assert_eq!(af(b"\x0Bimage=foo\n").unwrap(), b"foo");
    assert_eq!(b4(b"image\x0B=foo\n"), None);
    assert_eq!(af(b"image\x0B=foo\n").unwrap(), b"foo");
    assert_eq!(b4(b"image=\x0Bfoo\n").unwrap(), b"\x0Bfoo");
    assert_eq!(af(b"image=\x0Bfoo\n").unwrap(), b"foo");
    assert_eq!(b4(b"image=\x0B\n").unwrap(), b"\x0B");
    assert_eq!(af(b"image=\x0B\n").unwrap(), b"");
    assert_eq!(b4(b"\x0B# comment\nimage=foo\n"), None);
    assert_eq!(af(b"\x0B# comment\nimage=foo\n").unwrap(), b"foo");
    assert_eq!(b4(b"\x0B\nimage=foo\n"), None);
    assert_eq!(af(b"\x0B\nimage=foo\n").unwrap(), b"foo");
}

#[test]
fn complete_lines_cuts() {
    // byte N+1 is LF: all of head kept
    assert_eq!(complete_lines(b"ab\ncd\n"), b"ab\ncd");
    // cut mid line: back to the last LF (kept)
    assert_eq!(complete_lines(b"ab\ncdX"), b"ab\n");
    assert_eq!(complete_lines(b"ab\ncd\nef"), b"ab\ncd\n");
    // no LF at all: head as is
    assert_eq!(complete_lines(b"abcdX"), b"abcd");
    assert_eq!(complete_lines(b""), b"");
    assert_eq!(complete_lines(b"x"), b"");
}

/// What `read_game_cfg` does with a file: reads up to 257 bytes.
fn read_cfg(file: &[u8]) -> Meta {
    let buf = &file[..file.len().min(257)];
    let got = if buf.len() > 256 { complete_lines(buf).len() } else { buf.len() };
    Meta::parse_bytes(&buf[..got])
}

/// Before: 256-byte buffer, parsed as is.
fn old_read_cfg(file: &[u8]) -> Meta {
    Meta::parse_bytes(&file[..file.len().min(256)])
}

fn same(a: &Meta, b: &Meta) -> bool {
    a.title.as_deref() == b.title.as_deref()
        && a.next.as_deref() == b.next.as_deref()
        && a.boot.as_deref() == b.boot.as_deref()
        && a.kind == b.kind
}

/// `prefix`, then `#` filler (one comment line, no newline) up to `len` bytes.
fn padded(prefix: &[u8], len: usize) -> Vec<u8> {
    let mut v = prefix.to_vec();
    while v.len() < len {
        v.push(b'#');
    }
    v.truncate(len);
    v
}

const HEAD: &[u8] = b"title = \"Skies\"\nnext = \"B\"\n";

/// HEAD, a comment line of `#` up to byte `at`, a LF, then `tail`.
fn with_tail_at(at: usize, tail: &[u8]) -> Vec<u8> {
    let mut v = padded(HEAD, at);
    v.push(b'\n');
    v.extend_from_slice(tail);
    v
}

#[test]
fn game_cfg_up_to_256_bytes_unchanged() {
    let head = b"title = \"Skies\"\r\nnext = \"Disc 2\"\r\ntype = \"disc\"\r\n";
    for len in [0usize, 1, 10, head.len(), 100, 255, 256] {
        let f = padded(head, len);
        assert!(same(&read_cfg(&f), &old_read_cfg(&f)), "len {len}");
    }
    // Split character at the end of a file <= 256 B: still defaults.
    for n in [253usize, 254] {
        let mut f = padded(head, n);
        f.extend_from_slice(&"\u{3042}".as_bytes()[..2]);
        assert!(f.len() <= 256);
        assert!(read_cfg(&f).title.is_none() && old_read_cfg(&f).title.is_none());
    }
    // Invalid UTF-8 inside a file <= 256 B: defaults.
    let mut f = padded(head, 200);
    f[5] = 0xFF;
    assert!(read_cfg(&f).title.is_none());
}

#[test]
fn game_cfg_over_256_latin1_at_the_cut() {
    // 0xE9 at byte 255 (a lead-looking byte), then 'e': before defaults,
    // now the lines before its line parse.
    let mut f = padded(HEAD, 254);
    f.push(b'\n');
    f.push(0xE9);
    f.extend_from_slice(b"e\nboot = \"x\"\n");
    assert_eq!(f[255], 0xE9);
    assert!(f.len() > 257);
    assert!(old_read_cfg(&f).title.is_none());
    let n = read_cfg(&f);
    assert_eq!((n.title.as_deref(), n.next.as_deref()), (Some("Skies"), Some("B")));
    assert!(n.boot.is_none());
}

#[test]
fn game_cfg_over_256_katakana_bare_value() {
    let tail = "next = ディスク2\n".as_bytes();
    let mut splits = 0;
    for at in 235..255 {
        let f = with_tail_at(at, tail);
        if f.len() <= 256 || std::str::from_utf8(&f[..256]).is_ok() {
            continue;
        }
        splits += 1;
        assert!(old_read_cfg(&f).title.is_none(), "before: defaults (at {at})");
        let n = read_cfg(&f);
        assert_eq!((n.title.as_deref(), n.next.as_deref()), (Some("Skies"), Some("B")));
    }
    assert!(splits >= 2, "placements that split a character: {splits}");
}

#[test]
fn game_cfg_over_256_bare_value_cut_short() {
    let f = with_tail_at(256 - 14, b"next = Disc 2 long name\n");
    assert!(f.len() > 257);
    assert!(old_read_cfg(&f).next.as_deref().unwrap().starts_with("Disc"));
    assert_eq!(read_cfg(&f).next.as_deref(), Some("B"));
}

#[test]
fn game_cfg_over_256_unchanged_cases() {
    // Byte 257 is LF: the cut is on a line end, everything kept, as before.
    let mut f = padded(HEAD, 256);
    f.extend_from_slice(b"\ntitle = \"Z\"\n");
    assert_eq!(f[256], b'\n');
    assert!(same(&read_cfg(&f), &old_read_cfg(&f)));
    assert_eq!(read_cfg(&f).title.as_deref(), Some("Skies"));
    // No LF at all in a > 256 B file: the first 256 bytes as before.
    let f = padded(b"title = \"Skies\"", 400);
    assert!(same(&read_cfg(&f), &old_read_cfg(&f)));
    assert_eq!(read_cfg(&f).title.as_deref(), Some("Skies"));
    // Invalid UTF-8 earlier in a long file: defaults, as before.
    let mut f = with_tail_at(300, b"x\n");
    f[3] = 0xFF;
    assert!(read_cfg(&f).title.is_none() && old_read_cfg(&f).title.is_none());
}
