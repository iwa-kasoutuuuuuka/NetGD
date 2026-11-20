//! VFAT long-file-name matching.
//!
//! A long name lives in the directory as a run of 32-byte entries *in front
//! of* the 8.3 entry it belongs to, last piece first:
//!
//! ```text
//!   0x42 'o' 'n' 'g' ...   <- ordinal 2, flagged 0x40 = last piece
//!   0x01 'v' 'e' 'r' ...   <- ordinal 1
//!   LONGNA~1 TXT ...       <- the 8.3 entry (the "alias")
//! ```
//!
//! Each piece carries 13 UTF-16 code units, the alias' checksum, and the
//! attribute byte 0x0F that old software reads as "hidden system volume
//! label" and skips. A run is trusted only if its ordinals count down to 1
//! without a gap, every piece carries the same checksum, and that checksum
//! is the one of the 8.3 entry that follows. Anything else -- a piece whose
//! alias was deleted, a half-overwritten run, a name edited by a tool that
//! knows nothing of long names -- is an orphan and is ignored, exactly as
//! Windows does.
//!
//! Nothing here assembles the name. The caller wants one answer, "is this
//! the file called X", so [`LfnMatcher`] holds X and compares each piece as
//! it goes past. That keeps the state to a handful of bytes plus X itself
//! (the firmware has about 2 KB of stack and no business holding a 510-byte
//! UTF-16 name), and works the same however the directory is split across
//! sectors and clusters because the caller feeds entries in order.
//!
//! # Case and non-ASCII
//!
//! Comparison folds ASCII `A-Z` to `a-z` and nothing else. Windows folds the
//! whole BMP through its upcase table; carrying that table would cost more
//! flash than the rest of this crate, and a GDI that spells a non-ASCII name
//! in a different case from the file is rare (the GDI is normally written by
//! the same tool that named the files). Non-ASCII code units must therefore
//! match exactly. The 8.3 alias is unaffected: it is matched
//! case-insensitively as before by the caller.

/// Code-unit offsets of the 13 characters inside an LFN entry.
const CHAR_OFFSETS: [usize; 13] = [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];

/// Characters per LFN entry.
pub const CHARS_PER_ENTRY: usize = 13;

/// A long name is at most 255 UTF-16 code units, hence at most 20 pieces.
pub const MAX_LFN_CHARS: usize = 255;
const MAX_ORDINAL: u8 = 20;

/// The checksum a long-name piece stores for its 8.3 entry: rotate right by
/// one and add each of the 11 name bytes.
pub fn short_checksum(name: &[u8]) -> u8 {
    let mut sum = 0u8;
    for &b in name.iter().take(11) {
        sum = sum.rotate_right(1).wrapping_add(b);
    }
    sum
}

/// ASCII-only case fold of one UTF-16 code unit.
#[inline]
fn fold(c: u16) -> u16 {
    if (b'A' as u16..=b'Z' as u16).contains(&c) {
        c + 32
    } else {
        c
    }
}

/// Encode `s` as UTF-16 into `out`. `None` if it does not fit.
pub fn utf16_from_str(s: &str, out: &mut [u16]) -> Option<usize> {
    let mut n = 0;
    for u in s.encode_utf16() {
        *out.get_mut(n)? = u;
        n += 1;
    }
    Some(n)
}

/// Encode a name taken from a GDI line into UTF-16.
///
/// The GDI parser widens every byte of the file name to a `char`
/// (`discimg::gdi`: "each byte is widened to a char"), so a name written in
/// UTF-8 arrives as mojibake: `é` is the two chars `Ã©`. Undo that first --
/// if the widened bytes are valid UTF-8, that is what the author wrote, and
/// it is what a PC stored in the long name. If they are not, the file was
/// Latin-1 (or the bytes are noise) and the chars are used as they are. A
/// char above U+00FF cannot have come from a byte, so such a string is taken
/// as-is.
pub fn utf16_from_gdi_name(widened: &str, out: &mut [u16]) -> Option<usize> {
    let mut bytes = [0u8; 128];
    let mut n = 0;
    for ch in widened.chars() {
        match (ch as u32 <= 0xFF, bytes.get_mut(n)) {
            (true, Some(slot)) => *slot = ch as u8,
            _ => return utf16_from_str(widened, out),
        }
        n += 1;
    }
    // (`from_utf8` is already in the firmware; a hand-written decoder
    // measured 230 bytes larger.)
    match core::str::from_utf8(&bytes[..n]) {
        Ok(s) => utf16_from_str(s, out),
        Err(_) => utf16_from_str(widened, out),
    }
}

/// What one directory entry turned out to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feed {
    /// First byte 0x00: nothing follows, stop reading the directory.
    End,
    /// A deleted entry, a long-name piece, or a volume label: no file here.
    /// Pieces are consumed into the matcher's state.
    Other,
    /// A real entry (file or directory). `long` is true when a complete,
    /// checksum-valid run of long-name pieces right before it spells the
    /// name the matcher was built with.
    Entry { long: bool },
}

/// Recognises one particular long name in a stream of directory entries.
///
/// `N` is the capacity of the name being sought in UTF-16 code units. The
/// firmware uses 128 (the GDI parser's own limit); the tests use 255.
pub struct LfnMatcher<const N: usize> {
    target: [u16; N],
    tlen: usize,
    /// Ordinal of the piece accepted last; 0 when no run is in progress.
    expect: u8,
    csum: u8,
    /// Every piece so far agreed with the target.
    ok: bool,
    /// Ordinal 1 has been seen: the run is whole.
    complete: bool,
}

impl<const N: usize> LfnMatcher<N> {
    /// Seek the name `units`. `None` if it is empty or longer than `N` or 255.
    pub fn new(units: &[u16]) -> Option<Self> {
        if units.is_empty() || units.len() > N || units.len() > MAX_LFN_CHARS {
            return None;
        }
        let mut target = [0u16; N];
        target[..units.len()].copy_from_slice(units);
        Some(Self { target, tlen: units.len(), expect: 0, csum: 0, ok: false, complete: false })
    }

    /// Seek `name`, a Rust string.
    pub fn from_str(name: &str) -> Option<Self> {
        let mut u = [0u16; N];
        let n = utf16_from_str(name, &mut u)?;
        Self::new(&u[..n])
    }

    /// Seek a name as the GDI parser hands it over (see
    /// [`utf16_from_gdi_name`]).
    pub fn from_gdi_name(widened: &str) -> Option<Self> {
        let mut m = Self { target: [0; N], tlen: 0, expect: 0, csum: 0, ok: false, complete: false };
        let n = utf16_from_gdi_name(widened, &mut m.target)?;
        if n == 0 || n > MAX_LFN_CHARS {
            return None;
        }
        m.tlen = n;
        Some(m)
    }

    /// Forget any run in progress. The matcher does this itself at every real
    /// entry; call it when reusing one for a second directory.
    pub fn reset(&mut self) {
        self.expect = 0;
        self.ok = false;
        self.complete = false;
    }

    /// Pieces the sought name needs.
    fn pieces(&self) -> u8 {
        ((self.tlen + CHARS_PER_ENTRY - 1) / CHARS_PER_ENTRY) as u8
    }

    /// Feed the next 32-byte directory entry, in directory order. A slice
    /// shorter than 32 bytes counts as the end of the directory.
    pub fn feed(&mut self, e: &[u8]) -> Feed {
        if e.len() < 32 || e[0] == 0x00 {
            self.reset();
            return Feed::End;
        }
        if e[0] == 0xE5 {
            // A deleted entry ends any run: its alias may be gone, and what
            // follows is not part of it.
            self.reset();
            return Feed::Other;
        }
        let attr = e[11];
        if attr & 0x3F == 0x0F {
            self.piece(e);
            return Feed::Other;
        }
        let long = self.complete
            && self.ok
            && self.expect == 1
            && self.csum == short_checksum(&e[..11]);
        self.reset();
        if attr & 0x08 != 0 {
            return Feed::Other; // volume label
        }
        Feed::Entry { long }
    }

    fn piece(&mut self, e: &[u8]) {
        let ord = e[0];
        let seq = ord & 0x3F;
        // Type must be 0 and the (unused) first-cluster field zero; anything
        // else is not a long-name piece, whatever the attribute says.
        let sane = e[12] == 0 && e[26] == 0 && e[27] == 0 && seq != 0 && seq <= MAX_ORDINAL;
        if !sane {
            self.reset();
            return;
        }
        if ord & 0x40 != 0 {
            // First piece of a run (the name's tail): a new run starts here,
            // whatever was in progress.
            self.expect = seq;
            self.csum = e[13];
            self.complete = false;
            // The tail's ordinal fixes the name's length to within a piece.
            self.ok = seq == self.pieces();
        } else {
            if self.expect == 0 || seq != self.expect - 1 || e[13] != self.csum {
                self.reset();
                return;
            }
            self.expect = seq;
        }
        if self.ok {
            let base = (seq as usize - 1) * CHARS_PER_ENTRY;
            for (k, &off) in CHAR_OFFSETS.iter().enumerate() {
                let c = u16::from_le_bytes([e[off], e[off + 1]]);
                let idx = base + k;
                let good = if idx < self.tlen {
                    fold(c) == fold(self.target[idx])
                } else if idx == self.tlen {
                    c == 0x0000
                } else {
                    // Padding after the terminator is 0xFFFF; some writers
                    // use 0x0000, which is harmless.
                    c == 0xFFFF || c == 0x0000
                };
                if !good {
                    self.ok = false;
                    break;
                }
            }
        }
        if seq == 1 {
            self.complete = true;
        }
    }
}

/// Render a raw 11-byte directory name as `BASE.EXT` (no dot when the
/// extension is empty), trailing spaces dropped. Returns the length written
/// into `out`. Bytes are copied as they are, not interpreted: the OEM code
/// page is the caller's business.
pub fn display_83(raw: &[u8], out: &mut [u8; 12]) -> usize {
    let trim = |s: &[u8]| {
        let mut n = s.len();
        while n > 0 && s[n - 1] == b' ' {
            n -= 1;
        }
        n
    };
    let nb = trim(&raw[..8]);
    out[..nb].copy_from_slice(&raw[..nb]);
    let mut n = nb;
    let ne = trim(&raw[8..11]);
    if ne > 0 {
        out[n] = b'.';
        n += 1;
        out[n..n + ne].copy_from_slice(&raw[8..8 + ne]);
        n += ne;
    }
    // 0x05 stands for a first byte of 0xE5 on disk.
    if n > 0 && out[0] == 0x05 {
        out[0] = 0xE5;
    }
    n
}
