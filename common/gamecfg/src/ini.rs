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
    /// Milliseconds the tray is shown open after an image swap.
    pub open_time: u16,
    /// Milliseconds the drive is shown busy detecting the disc after that.
    pub detect_time: u16,
    pub image_tests: bool,
    pub high_speed: bool,
    /// Image number to switch to on a host hard reset; 0 = disabled.
    pub reset_goto: u16,
    /// KiB/s read limit; 0 = off, -1 = automatic.
    pub read_limit: i16,
    /// Which of `open_time` / `detect_time` the file actually set. A swap
    /// delay is only applied for keys the card asked for.
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
        let mut out = Ini::new();
        for line in text.lines() {
            let line = match line.find(['#', ';']) {
                Some(i) => &line[..i],
                None => line,
            }
            .trim();
            if line.is_empty() || line.starts_with('[') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
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
        out
    }

    /// Extra time, in milliseconds, a swapped-in disc is held NOT READY:
    /// `open_time` plus `detect_time`, counting only keys the file set.
    pub fn swap_delay_ms(&self) -> u32 {
        let o = if self.open_time_set { self.open_time as u32 } else { 0 };
        let d = if self.detect_time_set { self.detect_time as u32 } else { 0 };
        o + d
    }
}

fn clamp(v: i32, lo: i32, hi: i32) -> i32 {
    v.max(lo).min(hi)
}

/// A decimal integer, optionally signed, optionally quoted. Anything else is
/// not a number (a leading `+` is accepted, hex is not).
fn num(v: &str) -> Option<i32> {
    let v = v.trim().trim_matches('"').trim();
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
    use super::*;

    #[test]
    fn empty_is_the_official_defaults() {
        let i = Ini::parse("");
        assert_eq!(i, Ini::default());
        assert_eq!((i.open_time, i.detect_time), (500, 250));
        assert!(i.image_tests && !i.high_speed);
        assert_eq!((i.reset_goto, i.read_limit), (0, 0));
        assert_eq!(i.swap_delay_ms(), 0, "unset keys apply no delay");
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
        assert_eq!(i.swap_delay_ms(), 1000);
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
    fn set_flags_drive_the_swap_delay() {
        assert_eq!(Ini::parse("open_time=500").swap_delay_ms(), 500);
        assert_eq!(Ini::parse("detect_time=250").swap_delay_ms(), 250);
    }
}
