#![no_std]

//! Per-folder game metadata: `GAME.CFG`.
//!
//! Shared between the console and the firmware, which both need to read
//! it and must agree on what it means. The console shows titles and
//! launches programs; the firmware needs `disc` and `set` to know what
//! the front-panel button should swap to, and it needs that while a game
//! is running and the menu is long gone.
//!
//! One of these sits beside the disc images or the program in each folder and
//! says what the entry is. Everything in it is optional -- a folder with a
//! `.gdi` and no `GAME.CFG` keeps working exactly as before, which is the
//! point: the file adds information, it is not a new requirement.
//!
//! ```text
//! title = "Skies of Arcadia (Disc 1)"
//! next  = "Skies of Arcadia Disc 2"
//!
//! # or, for a program rather than a disc:
//! type = "elf"
//! boot = "prog.elf"
//! ```
//!
//! `next` names the folder the front-panel button swaps to. A link rather
//! than a numbering: any folder can point at any other, the two discs of a
//! game point at each other, and nothing has to agree about an ordering or
//! about which set a folder belongs to.
//!
//! **The syntax is a subset of TOML**, so a file written here is valid TOML
//! and an editor will highlight it, but the parser is a few hundred lines
//! rather than a dependency: `key = value`, `#` comments, quoted strings,
//! bare integers, and `[sections]` skipped. YAML was the alternative and is
//! much harder to hand-parse correctly -- its indentation and implicit typing
//! rules are exactly the kind of thing that half-works.
//!
//! **The name is `GAME.CFG` because of 8.3.** `GAME.TOML` has a four
//! character extension, so it only exists on the card under a generated alias
//! like `GAME~1.TOM`, and which alias depends on the tool that wrote the
//! card. The extension has to be three characters for the name to be the same
//! everywhere.

pub mod ini;

use heapless::String;

/// The file this module looks for, in every image folder.
pub const FILENAME: &str = "GAME.CFG";

/// Longest title kept. Matches the firmware's own limit, so a title that
/// fits one fits the other.
pub const MAX_TITLE: usize = 64;
/// Longest `boot` filename and `next` folder name.
pub const MAX_BOOT: usize = 32;
/// Matches `gdrom::catalog::NAME_LEN`: `next` names another folder, and a
/// value too short to hold one cannot be matched against the catalog.
pub const MAX_NEXT: usize = 96;

/// What kind of entry a folder is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Kind {
    /// A disc image: the firmware mounts it and the BIOS boots it. What a
    /// folder is when it says nothing.
    #[default]
    Disc,
    /// A program the console loads and runs itself.
    Elf,
}

/// A parsed `GAME.CFG`. Every field is optional; [`Meta::default`] is what a
/// folder without the file means.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Meta {
    /// Display name. Falls back to the folder's own name when absent.
    pub title: Option<String<MAX_TITLE>>,
    /// Folder the front-panel button swaps to, by name.
    ///
    /// A link, not a position: disc one points at disc two and disc two
    /// points back, so the button walks the pair without anything needing to
    /// know how many discs there are or what order they go in. Anything may
    /// point at anything.
    pub next: Option<String<MAX_NEXT>>,
    pub kind: Kind,
    /// For `type = "elf"`, the file to load. When absent the loader takes
    /// the only `.elf` in the folder.
    pub boot: Option<String<MAX_BOOT>>,
}

impl Meta {
    /// Parse the contents of a `GAME.CFG`.
    ///
    /// Never fails. A line that does not parse is skipped: the file is
    /// hand-edited on a PC, and refusing to show a game because someone
    /// fat-fingered a quote would be the wrong trade. Anything genuinely
    /// needed has a fallback.
    pub fn parse(text: &str) -> Self {
        Self::parse_bytes(text.as_bytes())
    }

    /// [`Meta::parse`] on the raw file. Bytes that are not valid UTF-8 as a
    /// whole give [`Meta::default`], which is what the firmware's
    /// `parse(from_utf8(buf).unwrap_or(""))` did; this checks with
    /// `fatmap::lfn::utf8_valid` instead, so the firmware carries no
    /// `from_utf8`. Everything after that works on bytes: every delimiter is
    /// ASCII, and a UTF-8 continuation byte never equals an ASCII one, so
    /// splitting the bytes cuts exactly where splitting the `str` did.
    pub fn parse_bytes(data: &[u8]) -> Self {
        let mut out = Meta::default();
        if !fatmap::lfn::utf8_valid(data) {
            return out;
        }
        // `str::lines` splits at `\n` and drops one `\r` before it; `trim`
        // drops that `\r` (and any other trailing ASCII space) anyway.
        for line in data.split(|&b| b == b'\n') {
            let line = trim_bytes(strip_comment(line));
            if line.is_empty() || line[0] == b'[' {
                continue;
            }
            let Some(eq) = line.iter().position(|&b| b == b'=') else {
                continue;
            };
            let key = trim_bytes(&line[..eq]);
            let value = trim_bytes(&line[eq + 1..]);

            match key {
                b"title" => out.title = unquote(value).map(fit),
                b"boot" => out.boot = unquote(value).map(fit),
                // `next_disk` accepted too: it is the obvious other name for
                // this and costs one line to not be wrong about.
                b"next" | b"next_disk" | b"next_disc" => out.next = unquote(value).map(fit),
                b"type" => {
                    if let Some(v) = unquote(value) {
                        out.kind = match v {
                            b"elf" | b"bin" | b"prog" => Kind::Elf,
                            _ => Kind::Disc,
                        };
                    }
                }
                _ => {}
            }
        }
        out
    }
}

/// `str::trim` for the ASCII whitespace only: space, tab, LF, VT, FF, CR.
///
/// One shared, deliberately not inlined copy ([`trim_bytes`]). `str::trim` decodes UTF-8 and
/// consults the Unicode `White_Space` table from both ends, and every call
/// site got its own copy of that; `str::trim_ascii` is small but inlines, so
/// the call sites of it together came out larger still. This is one loop,
/// called. The firmware uses it as well (`gdi.rs`), which is why it is public.
///
/// What it gives up: a line padded with U+00A0 or U+3000 keeps the padding.
/// These files are keys, numbers and quoted names; nothing in them is
/// delimited by such a character.
pub fn trim(s: &str) -> &str {
    let t = trim_bytes(s.as_bytes());
    // Only ASCII bytes were stepped over, so both ends are char boundaries.
    // SAFETY: a sub-slice of a `str` cut at char boundaries is UTF-8.
    unsafe { core::str::from_utf8_unchecked(t) }
}

/// `data` without a trailing UTF-8 character that was cut off.
///
/// For a buffer that holds only the start of a longer file: a multi-byte
/// character split at the end would fail validation for the whole buffer, so
/// the complete characters before it are kept instead. Only a trailing lead
/// byte followed by too few continuation bytes is dropped; anything else
/// (including invalid bytes earlier in the data) is returned unchanged, so
/// [`Meta::parse_bytes`] still gives the defaults for it.
pub fn complete_utf8_prefix(data: &[u8]) -> &[u8] {
    let n = data.len();
    // The lead byte of the last character is among the last four bytes.
    let mut back = 1;
    while back <= 4 && back <= n {
        let b = data[n - back];
        if b & 0xC0 != 0x80 {
            let need = match b {
                0xC2..=0xDF => 2,
                0xE0..=0xEF => 3,
                0xF0..=0xF4 => 4,
                _ => return data,
            };
            return if back < need { &data[..n - back] } else { data };
        }
        back += 1;
    }
    data
}

/// [`trim`] on bytes: the same six ASCII whitespace bytes from both ends.
/// The one copy the parsers here and the firmware share.
#[inline(never)]
pub fn trim_bytes(mut b: &[u8]) -> &[u8] {
    let ws = |c: u8| c == b' ' || (b'\t'..=b'\r').contains(&c);
    while let [c, rest @ ..] = b {
        if !ws(*c) {
            break;
        }
        b = rest;
    }
    while let [rest @ .., c] = b {
        if !ws(*c) {
            break;
        }
        b = rest;
    }
    b
}

#[cfg(test)]
mod trim_tests {
    use super::trim;

    #[test]
    fn trims_ascii_whitespace_from_both_ends() {
        assert_eq!(trim("  a b \t\r\n"), "a b");
        assert_eq!(trim("\x0b\x0c x \x0c\x0b"), "x");
        assert_eq!(trim("x"), "x");
        assert_eq!(trim(""), "");
        assert_eq!(trim(" \t\r\n"), "");
    }

    #[test]
    fn agrees_with_str_trim_on_ascii() {
        // Every first and last byte, around each kind of middle.
        for a in 0..128u8 {
            for b in [b' ', b'\t', b'x', 0x1f, 0x0b] {
                for c in 0..128u8 {
                    let buf = [a, b, c];
                    let s = core::str::from_utf8(&buf).unwrap();
                    assert_eq!(trim(s), s.trim(), "{:?}", s);
                }
            }
        }
    }

    #[test]
    fn leaves_non_ascii_alone() {
        // Multi-byte text next to the trimmed whitespace stays whole.
        assert_eq!(trim(" \u{30b7}\u{30a7}\u{30f3}\u{30e0}\u{30fc} "), "\u{30b7}\u{30a7}\u{30f3}\u{30e0}\u{30fc}");
        // Unicode whitespace is not trimmed (the documented difference).
        assert_eq!(trim("\u{3000}a\u{a0}"), "\u{3000}a\u{a0}");
        assert_eq!(trim(" \u{3000}a\u{a0}\n"), "\u{3000}a\u{a0}");
    }
}

/// Drop a trailing `# comment`, but not a `#` inside quotes.
fn strip_comment(line: &[u8]) -> &[u8] {
    let mut in_quotes = false;
    for (i, &c) in line.iter().enumerate() {
        match c {
            b'"' => in_quotes = !in_quotes,
            b'#' if !in_quotes => return &line[..i],
            _ => {}
        }
    }
    line
}

/// Take the inside of a quoted string, or a bare word as-is.
fn unquote(v: &[u8]) -> Option<&[u8]> {
    let v = trim_bytes(v);
    match v {
        [b'"', inner @ .., b'"'] => Some(inner),
        // An unterminated quote is a typo, not a value.
        [] | [b'"', ..] => None,
        _ => Some(v),
    }
}

/// Copy `s` into a `String<N>`, cut at the last char boundary that fits.
///
/// Truncate rather than reject: a title too long for the buffer is still a
/// better label than the folder's 8.3 name. The same result as pushing
/// `s.chars()` until one does not fit, which the firmware also does with its
/// names (`gdi.rs`), hence public.
///
/// `s` is meant to be UTF-8 (a sub-slice of a `str` cut at char
/// boundaries). Bytes that are not give an empty string, never a broken
/// `String`.
pub fn fit<const N: usize>(s: &[u8]) -> String<N> {
    let head = fit_head(s, N);
    let mut out: String<N> = String::new();
    // SAFETY: `fit_head` returns UTF-8 of at most `N` bytes.
    let _ = unsafe { out.as_mut_vec() }.extend_from_slice(head);
    out
}

/// The part of `s` [`fit`] keeps, for any capacity: one copy, not one per
/// `N`.
#[inline(never)]
fn fit_head(s: &[u8], cap: usize) -> &[u8] {
    let mut end = s.len().min(cap);
    // Back off to a char boundary: never in front of a continuation byte.
    while end > 0 && end < s.len() && s[end] & 0xC0 == 0x80 {
        end -= 1;
    }
    let head = &s[..end];
    if fatmap::lfn::utf8_valid(head) {
        head
    } else {
        &[]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_documented_example() {
        let m = Meta::parse(
            "# a game
title = \"Skies of Arcadia (Disc 1)\"
next = \"Skies of Arcadia Disc 2\"
",
        );
        assert_eq!(m.title.as_deref(), Some("Skies of Arcadia (Disc 1)"));
        assert_eq!(m.next.as_deref(), Some("Skies of Arcadia Disc 2"));
        assert_eq!(m.kind, Kind::Disc);
    }

    #[test]
    fn reads_a_program_entry() {
        let m = Meta::parse("type = \"elf\"\nboot = \"prog.elf\"\n");
        assert_eq!(m.kind, Kind::Elf);
        assert_eq!(m.boot.as_deref(), Some("prog.elf"));
    }

    #[test]
    fn a_missing_file_is_a_plain_disc() {
        let m = Meta::default();
        assert_eq!(m.kind, Kind::Disc);
        assert!(m.title.is_none());
        assert!(m.next.is_none());
    }

    #[test]
    fn survives_junk() {
        let m = Meta::parse("[meta]\nnonsense\ntitle = \"ok\"\nnext = \n= 3\n");
        assert_eq!(m.title.as_deref(), Some("ok"));
        assert_eq!(m.next, None);
    }

    #[test]
    fn hash_inside_quotes_is_not_a_comment() {
        let m = Meta::parse("title = \"Hash # Game\" # real comment\n");
        assert_eq!(m.title.as_deref(), Some("Hash # Game"));
    }
}

/// Resolve a `next` name to an index in the catalog.
///
/// `names` yields each entry's folder name in catalog order. Matching is
/// case-insensitive and ignores a trailing slash, because the name is typed
/// by hand on a PC and "Skies of Arcadia Disc 2/" is the same folder.
///
/// Returns `None` when nothing matches, which is the right answer for a
/// typo: the button does nothing rather than swapping to a surprise.
pub fn resolve<'a>(names: impl Iterator<Item = &'a str>, want: &str) -> Option<usize> {
    names
        .enumerate()
        .find_map(|(i, n)| name_matches(n.as_bytes(), want.as_bytes()).then_some(i))
}

/// The test [`resolve`] applies to one name: `want`, trimmed and without
/// trailing slashes, equals `name` up to ASCII case.
///
/// On bytes, which is the same answer the old char-by-char comparison gave
/// for UTF-8 of equal length: ASCII case folding touches only ASCII bytes,
/// and two non-ASCII chars are equal exactly when their bytes are.
#[inline(never)]
pub fn name_matches(name: &[u8], want: &[u8]) -> bool {
    name.eq_ignore_ascii_case(trim_slashes(trim_bytes(want)))
}

/// `s` without its trailing `/`s (`str::trim_end_matches('/')`).
pub fn trim_slashes(mut s: &[u8]) -> &[u8] {
    while let [rest @ .., b'/'] = s {
        s = rest;
    }
    s
}

#[cfg(test)]
mod link_tests {
    use super::*;

    const NAMES: &[&str] = &["LAUNCH", "Skies of Arcadia", "Skies of Arcadia Disc 2"];

    #[test]
    fn finds_a_folder_by_name() {
        assert_eq!(resolve(NAMES.iter().copied(), "Skies of Arcadia Disc 2"), Some(2));
    }

    #[test]
    fn ignores_case_and_a_trailing_slash() {
        assert_eq!(resolve(NAMES.iter().copied(), "skies of arcadia"), Some(1));
        assert_eq!(resolve(NAMES.iter().copied(), "LAUNCH/"), Some(0));
    }

    #[test]
    fn a_typo_resolves_to_nothing() {
        assert_eq!(resolve(NAMES.iter().copied(), "Skies of Arcadia Disc 3"), None);
        // A prefix is not a match: it would link to the wrong disc.
        assert_eq!(resolve(NAMES.iter().copied(), "Skies of Arcadia D"), None);
    }

    #[test]
    fn parses_the_link_and_its_aliases() {
        for key in ["next", "next_disk", "next_disc"] {
            let m = Meta::parse(&alloc_line(key));
            assert_eq!(m.next.as_deref(), Some("Disc 2"), "{key}");
        }
    }

    fn alloc_line(key: &str) -> heapless::String<64> {
        let mut s: heapless::String<64> = heapless::String::new();
        s.push_str(key).unwrap();
        s.push_str(" = \"Disc 2\"").unwrap();
        s
    }
}
