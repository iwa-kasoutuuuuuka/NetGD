//! 8.3 names: the firmware's replacement for `embedded_sdmmc::ShortFileName`.

use crate::lfn::utf8_next;

/// An 8.3 name as stored on disk: 8 + 3 bytes, space padded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sfn(pub [u8; 11]);

impl Sfn {
    /// Spell `name` (UTF-8) as an 8.3 name exactly as
    /// `embedded_sdmmc::ShortFileName::create_from_str` does, `None` where it
    /// gives an error:
    ///
    /// * `""` and `"."` are `.`, `".."` is `..`;
    /// * ASCII is upper-cased; U+0080..=U+00FF is stored as that byte (not
    ///   upper-cased); anything above, or invalid UTF-8, is refused;
    /// * control characters, space and `"*+,/:;<=>?[\]|` are refused;
    /// * a dot ends the base (1..=8 bytes before it); a dot with no base, or
    ///   one after the extension started, is refused -- except directly
    ///   after another dot, which is swallowed (`A..B` is `A.B`);
    /// * more than 8 + 3 characters, or none, is refused.
    pub fn from_bytes(name: &[u8]) -> Option<Sfn> {
        let mut out = [b' '; 11];
        match name {
            b".." => return Some(Sfn(*b"..         ")),
            b"" | b"." => return Some(Sfn(*b".          ")),
            _ => {}
        }
        let mut s = name;
        let mut idx = 0usize;
        let mut dot = false;
        while !s.is_empty() {
            let (cp, len) = utf8_next(s)?;
            s = &s[len..];
            if !(0x20..0x100).contains(&cp)
                || matches!(cp as u8, b' ' | b'"' | b'*' | b'+' | b',' | b'/' | b':' | b';' | b'<' | b'=' | b'>' | b'?' | b'[' | b'\\' | b']' | b'|')
            {
                return None;
            }
            if cp == b'.' as u32 {
                if !(1..9).contains(&idx) {
                    return None;
                }
                idx = 8;
                dot = true;
                continue;
            }
            if idx >= if dot { 11 } else { 8 } {
                return None;
            }
            out[idx] = (cp as u8).to_ascii_uppercase();
            idx += 1;
        }
        if idx == 0 {
            return None;
        }
        Some(Sfn(out))
    }

    /// The base name without its padding.
    pub fn base(&self) -> &[u8] {
        trim(&self.0[..8])
    }

    /// The extension without its padding.
    pub fn ext(&self) -> &[u8] {
        trim(&self.0[8..])
    }
}

fn trim(s: &[u8]) -> &[u8] {
    let mut n = s.len();
    while n > 0 && s[n - 1] == b' ' {
        n -= 1;
    }
    &s[..n]
}
