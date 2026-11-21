//! `GDEMU.INI`: the original GDEMU's settings file, at the root of the card.
//!
//! Keys, ranges and defaults are the documented ones from
//! <https://gdemu.wordpress.com/details/gdemu-details/> (see
//! `docs/research/web-findings-2026-11.md` section 2-1):
//!
//! | key           | range                            | default |
//! |---------------|----------------------------------|---------|
//! | `open_time`   | 150..=5000 ms                    | 500     |
//! | `detect_time` | 150..=5000 ms                    | 250     |
//! | `image_tests` | 0 / 1                            | 1       |
//! | `high_speed`  | 0 / 1                            | 0       |
//! | `reset_goto`  | 0..=9999 (0 = disabled)          | 0       |
//! | `read_limit`  | 600..=1250 KiB/s, 0 off, -1 auto | 0       |
//!
//! The official page does not spell out the comment syntax or sections, so
//! the parser is lenient: `key = value` lines, keys case-insensitive, `#` or
//! `;` starts a comment (whole line or after a value), `[sections]` are
//! skipped, CRLF is fine. Anything else is counted and ignored.
//!
//! Policy on bad values (NetGD's own choice, the official page is silent): a
//! number outside the range of `open_time`, `detect_time` or `read_limit`
//! is clamped to the nearest end; a `reset_goto` above 9999 means disabled;
//! a value that is not a number at all leaves that key at its default.

/// Smallest and largest `open_time` / `detect_time`, milliseconds.
pub const TIME_MIN: u16 = 150;
pub const TIME_MAX: u16 = 5000;
pub const OPEN_TIME_DEFAULT: u16 = 500;
pub const DETECT_TIME_DEFAULT: u16 = 250;
pub const RESET_GOTO_MAX: u16 = 9999;
pub const READ_LIMIT_MIN: i16 = 600;
pub const READ_LIMIT_MAX: i16 = 1250;

/// A parsed `GDEMU.INI`. [`Ini::default`] is the official defaults, which is
/// also what a file that says nothing means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Ini {
    /// Milliseconds the tray is shown open after an image swap (NOT READY,
    /// no medium).
    pub open_time: u16,
    /// Milliseconds the drive is shown busy detecting the disc after that
    /// (NOT READY, becoming ready).
    pub detect_time: u16,
    pub image_tests: bool,
    pub high_speed: bool,
    /// Image number to switch to on a host hard reset; 0 = disabled.
    pub reset_goto: u16,
    /// KiB/s read limit; 0 = off, -1 = automatic.
    pub read_limit: i16,
    /// Which of `open_time` / `detect_time` the file actually set: only those
    /// override the firmware's swap window ([`Ini::swap_window_ms`]).
    pub open_time_set: bool,
    pub detect_time_set: bool,
    /// Lines with a key this parser does not know.
    pub unknown: u8,
}

impl Default for Ini {
    fn default() -> Self {
        Self::new()
    }
}

impl Ini {
    /// The official defaults.
    pub const fn new() -> Self {
        Self {
            open_time: OPEN_TIME_DEFAULT,
            detect_time: DETECT_TIME_DEFAULT,
            image_tests: true,
            high_speed: false,
            reset_goto: 0,
            read_limit: 0,
            open_time_set: false,
            detect_time_set: false,
            unknown: 0,
        }
    }

    /// Parse the contents of a `GDEMU.INI`. Never fails.
    pub fn parse(text: &str) -> Self {
        Self::parse_bytes(text.as_bytes())
    }

    /// Parse a whole file held in memory. See [`Ini::parse_reader`] for the
    /// byte-level rules (line ends, BOM, non-ASCII bytes).
    pub fn parse_bytes(data: &[u8]) -> Self {
        let mut out = Ini::new();
        let mut first = true;
        for line in data.split(|&b| b == b'\r' || b == b'\n') {
            out.line(line, first);
            first = false;
        }
        out
    }

    /// Parse a file of any size through `buf`, which is also the line buffer
    /// (a line longer than `buf` is dropped whole). `read` fills the slice it
    /// is given and returns how many bytes it wrote; 0 means end of file (or
    /// an error, which keeps what was read so far).
    ///
    /// Byte-level rules, so a card edited on any platform parses the same:
    /// CR, LF and CRLF all end a line; a UTF-8 BOM at the start of the file
    /// is skipped; a line whose settings part (before `#` / `;`) holds a
    /// non-ASCII byte is ignored on its own -- a Shift-JIS comment never
    /// disables the keys after it.
    pub fn parse_reader(buf: &mut [u8], mut read: impl FnMut(&mut [u8]) -> usize) -> Self {
        let mut out = Ini::new();
        let mut keep = 0usize; // bytes of an unfinished line at buf[..keep]
        let mut first = true;
        let mut discarding = false; // inside a line too long for `buf`
        loop {
            if keep >= buf.len() {
                return out;
            }
            let n = read(&mut buf[keep..]);
            let end = keep + n;
            let mut start = 0usize;
            let mut i = 0usize;
            while i < end {
                if buf[i] == b'\r' || buf[i] == b'\n' {
                    if discarding {
                        discarding = false;
                    } else {
                        out.line(&buf[start..i], first);
                    }
                    first = false;
                    start = i + 1;
                }
                i += 1;
            }
            if n == 0 {
                if !discarding && start < end {
                    out.line(&buf[start..end], first);
                }
                return out;
            }
            if start == 0 && end == buf.len() {
                // No line end in a full buffer: drop this line.
                discarding = true;
                first = false;
                keep = 0;
            } else {
                buf.copy_within(start..end, 0);
                keep = end - start;
            }
        }
    }

    /// One line, without its terminator.
    fn line(&mut self, mut line: &[u8], first: bool) {
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
        let line = crate::trim(line);
        if line.is_empty() || line.starts_with('[') {
            return;
        }
        let Some((key, value)) = line.split_once('=') else {
            return;
        };
        let (key, value) = (crate::trim(key), crate::trim(value));
        let out = self;
        if key.eq_ignore_ascii_case("open_time") {
            if let Some(v) = num(value) {
                out.open_time = clamp(v, TIME_MIN as i32, TIME_MAX as i32) as u16;
                out.open_time_set = true;
            }
        } else if key.eq_ignore_ascii_case("detect_time") {
            if let Some(v) = num(value) {
                out.detect_time = clamp(v, TIME_MIN as i32, TIME_MAX as i32) as u16;
                out.detect_time_set = true;
            }
        } else if key.eq_ignore_ascii_case("image_tests") {
            if let Some(v) = num(value) {
                out.image_tests = v != 0;
            }
        } else if key.eq_ignore_ascii_case("high_speed") {
            if let Some(v) = num(value) {
                out.high_speed = v != 0;
            }
        } else if key.eq_ignore_ascii_case("reset_goto") {
            if let Some(v) = num(value) {
                out.reset_goto = if (0..=RESET_GOTO_MAX as i32).contains(&v) {
                    v as u16
                } else {
                    0
                };
            }
        } else if key.eq_ignore_ascii_case("read_limit") {
            if let Some(v) = num(value) {
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

    /// The two phases of the NOT READY window after an image switch, in
    /// milliseconds: (`open_time`, `detect_time`), each the file's value if it
    /// set the key, else the caller's default. The firmware's defaults are the
    /// stock GDEMU 6.0 measurement (`discimg::atapi::SWAP_OPEN_MS` /
    /// `SWAP_DETECT_MS`), not the documented 500 / 250 nobody has measured.
    pub fn swap_window_ms(&self, open_default: u32, detect_default: u32) -> (u32, u32) {
        let o = if self.open_time_set { self.open_time as u32 } else { open_default };
        let d = if self.detect_time_set { self.detect_time as u32 } else { detect_default };
        (o, d)
    }
}

fn clamp(v: i32, lo: i32, hi: i32) -> i32 {
    v.max(lo).min(hi)
}

/// A decimal integer, optionally signed, optionally quoted. Anything else is
/// not a number (a leading `+` is accepted, hex is not).
fn num(v: &str) -> Option<i32> {
    let v = crate::trim(crate::trim(v).trim_matches('"'));
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

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::{format, vec, vec::Vec};

    #[test]
    fn empty_is_the_official_defaults() {
        let i = Ini::parse("");
        assert_eq!(i, Ini::default());
        assert_eq!((i.open_time, i.detect_time), (500, 250));
        assert!(i.image_tests && !i.high_speed);
        assert_eq!((i.reset_goto, i.read_limit), (0, 0));
        // Unset keys: the caller's defaults (the firmware passes the stock
        // GDEMU 6.0 measurement, 150 + 150 ms), not the documented 500/250.
        assert_eq!(i.swap_window_ms(150, 150), (150, 150), "unset keys take the defaults");
    }

    #[test]
    fn every_key() {
        let i = Ini::parse(
            "open_time=700\ndetect_time=300\nimage_tests=0\nhigh_speed=1\nreset_goto=12\nread_limit=900\n",
        );
        assert_eq!(i.open_time, 700);
        assert_eq!(i.detect_time, 300);
        assert!(!i.image_tests);
        assert!(i.high_speed);
        assert_eq!(i.reset_goto, 12);
        assert_eq!(i.read_limit, 900);
        assert_eq!(i.swap_window_ms(150, 150), (700, 300));
        assert_eq!(i.unknown, 0);
    }

    #[test]
    fn time_ranges_clamp() {
        let i = Ini::parse("open_time=10\ndetect_time=99999\n");
        assert_eq!((i.open_time, i.detect_time), (150, 5000));
        let i = Ini::parse("open_time=150\ndetect_time=5000\n");
        assert_eq!((i.open_time, i.detect_time), (150, 5000));
        let i = Ini::parse("open_time=-5\n");
        assert_eq!(i.open_time, 150);
    }

    #[test]
    fn reset_goto_range() {
        assert_eq!(Ini::parse("reset_goto=9999").reset_goto, 9999);
        assert_eq!(Ini::parse("reset_goto=10000").reset_goto, 0);
        assert_eq!(Ini::parse("reset_goto=-1").reset_goto, 0);
        assert_eq!(Ini::parse("reset_goto=0").reset_goto, 0);
    }

    #[test]
    fn read_limit_values() {
        assert_eq!(Ini::parse("read_limit=-1").read_limit, -1);
        assert_eq!(Ini::parse("read_limit=-7").read_limit, -1);
        assert_eq!(Ini::parse("read_limit=0").read_limit, 0);
        assert_eq!(Ini::parse("read_limit=100").read_limit, 600);
        assert_eq!(Ini::parse("read_limit=5000").read_limit, 1250);
        assert_eq!(Ini::parse("read_limit=600").read_limit, 600);
    }

    #[test]
    fn flags_are_zero_or_nonzero() {
        assert!(Ini::parse("image_tests=1").image_tests);
        assert!(!Ini::parse("image_tests=0").image_tests);
        assert!(Ini::parse("high_speed=2").high_speed);
    }

    #[test]
    fn comments_case_and_spacing() {
        let i = Ini::parse(
            "# a comment\n; another\n[GDEMU]\n  Open_Time = 600  # trailing\nRESET_GOTO\t=\t3 ; trailing\n",
        );
        assert_eq!(i.open_time, 600);
        assert_eq!(i.reset_goto, 3);
        assert_eq!(i.unknown, 0);
    }

    #[test]
    fn crlf_and_no_final_newline() {
        let i = Ini::parse("open_time=800\r\ndetect_time=400\r\nreset_goto=2");
        assert_eq!((i.open_time, i.detect_time, i.reset_goto), (800, 400, 2));
    }

    #[test]
    fn unknown_keys_are_counted_and_ignored() {
        let i = Ini::parse("image_tag=x\nfoo=1\nreset_goto=4\n");
        assert_eq!(i.unknown, 2);
        assert_eq!(i.reset_goto, 4);
    }

    #[test]
    fn malformed_lines_keep_defaults() {
        let i = Ini::parse("open_time\n=5\nopen_time=abc\nreset_goto=\ndetect_time=1x\nhigh_speed=0x1\n");
        assert_eq!(i, Ini { unknown: 1, ..Ini::default() });
        assert!(!i.open_time_set && !i.detect_time_set);
    }

    #[test]
    fn last_assignment_wins_and_quotes_tolerated() {
        let i = Ini::parse("reset_goto=2\nreset_goto=\"5\"\n");
        assert_eq!(i.reset_goto, 5);
    }

    #[test]
    fn set_flags_drive_the_swap_window() {
        // Each phase separately: a key the file set wins, else the default.
        assert_eq!(Ini::parse("open_time=500").swap_window_ms(150, 150), (500, 150));
        assert_eq!(Ini::parse("detect_time=250").swap_window_ms(150, 150), (150, 250));
        // The stock probe card's GDEMU.INI.
        let probe = Ini::parse("open_time = 150\ndetect_time = 150\nreset_goto = 1");
        assert_eq!(probe.swap_window_ms(1, 2), (150, 150));
        // Clamped like the keys themselves (150..=5000).
        assert_eq!(Ini::parse("open_time=1\ndetect_time=9000").swap_window_ms(0, 0), (150, 5000));
    }

    /// Feed `data` through `parse_reader` in chunks of at most `chunk`.
    fn chunked(data: &[u8], bufsize: usize, chunk: usize) -> Ini {
        let mut buf = vec![0u8; bufsize];
        let mut pos = 0;
        Ini::parse_reader(&mut buf, |b| {
            let n = b.len().min(chunk).min(data.len() - pos);
            b[..n].copy_from_slice(&data[pos..pos + n]);
            pos += n;
            n
        })
    }

    #[test]
    fn utf8_bom_is_skipped() {
        let i = Ini::parse_bytes(b"\xEF\xBB\xBFopen_time=700\nreset_goto=3\n");
        assert_eq!((i.open_time, i.reset_goto, i.unknown), (700, 3, 0));
        assert!(i.open_time_set);
        let i = chunked(b"\xEF\xBB\xBFreset_goto=3\n", 512, 2);
        assert_eq!((i.reset_goto, i.unknown), (3, 0));
    }

    #[test]
    fn shift_jis_comment_does_not_hide_later_keys() {
        let data = b"# \x93\xfa\x96\x7b\x8c\xea\nreset_goto=12 # \x82\xa0\nopen_time=600\n";
        let i = Ini::parse_bytes(data);
        assert_eq!((i.reset_goto, i.open_time, i.unknown), (12, 600, 0));
        assert_eq!(chunked(data, 512, 512), i);
        // A non-ASCII byte in the settings part drops only that line.
        let i = Ini::parse_bytes(b"open_time=\x82\xa0\nreset_goto=5\n");
        assert_eq!((i.reset_goto, i.open_time_set), (5, false));
    }

    #[test]
    fn cr_only_line_endings() {
        let i = Ini::parse_bytes(b"open_time=800\rdetect_time=400\rreset_goto=2\r");
        assert_eq!((i.open_time, i.detect_time, i.reset_goto), (800, 400, 2));
        let i = Ini::parse_bytes(b"open_time=800\r\n\r\ndetect_time=400\n\rreset_goto=2");
        assert_eq!((i.open_time, i.detect_time, i.reset_goto), (800, 400, 2));
        assert_eq!(chunked(b"reset_goto=2\rdetect_time=400", 512, 512).detect_time, 400);
    }

    #[test]
    fn key_split_at_the_buffer_boundary() {
        let mut data = vec![b'#'; 497];
        data.push(b'\n');
        data.extend_from_slice(b"reset_goto=12\nopen_time=700\n");
        assert!(data.len() > 512);
        for chunk in [1usize, 7, 64, 512] {
            let i = chunked(&data, 512, chunk);
            assert_eq!((i.reset_goto, i.open_time, i.unknown), (12, 700, 0), "chunk {chunk}");
        }
        // The value's digits straddle byte 512 for some of these paddings.
        for pad in 480..=512usize {
            let mut d = vec![b'#'; pad];
            d.push(b'\n');
            d.extend_from_slice(b"reset_goto=1234\n");
            assert_eq!(chunked(&d, 512, 512).reset_goto, 1234, "pad {pad}");
        }
    }

    #[test]
    fn files_over_512_bytes_parse_completely() {
        let mut data = Vec::new();
        for n in 0..60 {
            data.extend_from_slice(format!("# filler line number {n}\r\n").as_bytes());
        }
        data.extend_from_slice(b"detect_time=333\r\nread_limit=900");
        assert!(data.len() > 1024);
        let i = chunked(&data, 512, 512);
        assert_eq!((i.detect_time, i.read_limit), (333, 900));
    }

    #[test]
    fn overlong_line_is_dropped_not_the_file() {
        let mut data = b"open_time=700\n".to_vec();
        data.extend(std::iter::repeat(b'x').take(2000));
        data.extend_from_slice(b"\nreset_goto=9\n");
        let i = chunked(&data, 512, 512);
        assert_eq!((i.open_time, i.reset_goto), (700, 9));
        assert_eq!(i.unknown, 0, "the overlong line is not counted");
        let mut data = vec![b'y'; 512];
        data.extend_from_slice(b"\nreset_goto=4");
        assert_eq!(chunked(&data, 512, 100).reset_goto, 4);
    }

    #[test]
    fn reader_matches_whole_file_parse() {
        let data = b"[x]\r\nopen_time = 650 ;c\nimage_tests=0\r\nfoo=1\nreset_goto=7";
        for chunk in [1usize, 3, 512] {
            assert_eq!(chunked(data, 32, chunk), Ini::parse_bytes(data));
        }
    }
}
