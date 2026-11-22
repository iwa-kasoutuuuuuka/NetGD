//! The shared trim helper's exact byte set, the GAME.CFG 256-byte cut, and
//! the firmware's `configured_name` line scan before/after it used `trim_bytes`.
//!
//! Behaviour differences pinned here (input -> before -> after):
//! * GDEMU.CFG, VT (0x0B) around a line/key/value: `image=foo\x0B` ->
//!   name "foo\x0B" -> name "foo"; a line holding only VT -> search ended
//!   (no '=') -> line skipped, later `image=` found.
//! * GAME.CFG longer than 256 B whose byte 256 splits a multi-byte char ->
//!   all defaults -> the lines before the split character are used.

use gamecfg::{complete_utf8_prefix, trim_bytes, Meta};

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
fn gdemu_cfg_vt_difference() {
    // before: VT kept in the name; after: trimmed.
    assert_eq!(image_name(b"image=foo\x0B\n", old_trim_ascii).unwrap(), b"foo\x0B");
    assert_eq!(image_name(b"image=foo\x0B\n", trim_bytes).unwrap(), b"foo");
    assert_eq!(image_name(b"\x0Bimage=foo\n", old_trim_ascii).unwrap_or_default(), b"");
    assert_eq!(image_name(b"\x0Bimage=foo\n", trim_bytes).unwrap(), b"foo");
    // before: a VT-only line has no '=' and ends the search; after: skipped.
    assert_eq!(image_name(b"\x0B\nimage=foo\n", old_trim_ascii), None);
    assert_eq!(image_name(b"\x0B\nimage=foo\n", trim_bytes).unwrap(), b"foo");
}

#[test]
fn utf8_prefix_cuts() {
    let two = "\u{e9}".as_bytes(); // 2
    let three = "\u{3042}".as_bytes(); // 3
    let four = "\u{1F600}".as_bytes(); // 4
    for ch in [two, three, four] {
        let n = ch.len();
        for k in 1..n {
            let mut v = b"title=\"a\"\n".to_vec();
            v.extend_from_slice(&ch[..k]);
            assert_eq!(complete_utf8_prefix(&v), b"title=\"a\"\n", "cut {k}/{n}");
        }
        let mut v = b"ab".to_vec();
        v.extend_from_slice(ch);
        assert_eq!(complete_utf8_prefix(&v), &v[..], "complete char");
    }
    assert_eq!(complete_utf8_prefix(b""), b"");
    assert_eq!(complete_utf8_prefix(b"abc"), b"abc");
    // Cut at a boundary: nothing dropped.
    assert_eq!(complete_utf8_prefix("ab\u{3042}".as_bytes()), "ab\u{3042}".as_bytes());
    // Invalid bytes are left for the validator.
    assert_eq!(complete_utf8_prefix(b"a\xFF"), b"a\xFF");
    assert_eq!(complete_utf8_prefix(b"a\x80"), b"a\x80");
    assert_eq!(complete_utf8_prefix(b"\x80\x80\x80\x80\x80"), b"\x80\x80\x80\x80\x80");
}

/// What `read_game_cfg` does with a file: reads up to 257 bytes.
fn read_cfg(file: &[u8]) -> Meta {
    let got = file.len().min(257);
    let buf = &file[..got];
    let got = if got > 256 { complete_utf8_prefix(&buf[..256]).len() } else { got };
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

fn padded(prefix: &[u8], len: usize) -> Vec<u8> {
    // `prefix` then comment filler up to exactly `len` bytes.
    let mut v = prefix.to_vec();
    while v.len() < len {
        v.push(b'#');
    }
    v.truncate(len);
    v
}

#[test]
fn game_cfg_up_to_256_bytes_unchanged() {
    let head = b"title = \"Skies\"\r\nnext = \"Disc 2\"\r\ntype = \"disc\"\r\n";
    for len in [0usize, 1, 10, head.len(), 100, 255, 256] {
        let f = padded(head, len);
        assert!(same(&read_cfg(&f), &old_read_cfg(&f)), "len {len}");
    }
    // Files whose last bytes are a split character, <= 256 B: still defaults.
    let mut f = padded(head, 253);
    f.extend_from_slice(&"\u{3042}".as_bytes()[..2]); // exactly 255 B, invalid tail
    assert!(read_cfg(&f).title.is_none() && old_read_cfg(&f).title.is_none());
    let mut f = padded(head, 254);
    f.extend_from_slice(&"\u{3042}".as_bytes()[..2]); // exactly 256 B, invalid tail
    assert!(read_cfg(&f).title.is_none() && old_read_cfg(&f).title.is_none());
    // Invalid UTF-8 early in a long file: defaults, as before.
    let mut f = padded(head, 300);
    f[5] = 0xFF;
    assert!(read_cfg(&f).title.is_none() && old_read_cfg(&f).title.is_none());
}

#[test]
fn game_cfg_over_256_bytes_cut_inside_a_char() {
    for ch in ["\u{e9}", "\u{3042}", "\u{1F600}"] {
        let cb = ch.as_bytes();
        for k in 1..cb.len() {
            // Character starts so that exactly k of its bytes fit below 256.
            let mut f = padded(b"title = \"Skies\"\nnext = \"B\"\n", 256 - k);
            f.extend_from_slice(cb);
            f.extend_from_slice(b"\nboot = \"x\"\n");
            assert!(f.len() > 256);
            let old = old_read_cfg(&f);
            assert!(old.title.is_none(), "before: defaults");
            let new = read_cfg(&f);
            assert_eq!(new.title.as_deref(), Some("Skies"));
            assert_eq!(new.next.as_deref(), Some("B"));
            assert!(new.boot.is_none());
        }
        // Cut exactly at a boundary, and a char wholly inside: same as before.
        let mut f = padded(b"title = \"Skies\"\n", 256 - cb.len());
        f.extend_from_slice(cb);
        f.extend_from_slice(b"tail\n");
        assert!(same(&read_cfg(&f), &old_read_cfg(&f)));
        assert_eq!(read_cfg(&f).title.as_deref(), Some("Skies"));
    }
    // ASCII-only long file: identical.
    let f = padded(b"title = \"Skies\"\n", 400);
    assert!(same(&read_cfg(&f), &old_read_cfg(&f)));
}
