//! Stack high-water mark for the NetGD firmware (docs/PLAN.md A-1).
//!
//! At boot the firmware paints the free part of the stack -- from `__sheap`
//! (the end of `.bss`/`.uninit`) up to just below its own stack pointer --
//! with [`PATTERN`]. The stack grows down, so whatever the deepest call ever
//! reached has overwritten the pattern from the top; the words at the bottom
//! that still hold it were never used. [`untouched_words`] counts them, and
//! [`Report`] turns that into the numbers the `0xE0` sub-command 9 (STACK)
//! returns. Everything here is pure, so it is tested on the host; the
//! painting and the volatile reads are in `mcu/netgd/src/stack.rs`.
//!
//! What the number is and is not:
//! * It is a high-water mark since the last reset, of the one stack the
//!   firmware has (thread mode and every interrupt run on MSP).
//! * It can only over-report by a call that happened to write the pattern
//!   value itself (one word in 2^32), and it can miss a frame that reserved
//!   stack without writing to it (a buffer that was never filled) -- the
//!   usual limits of stack painting.
//! * The top `unpainted` bytes (the frame of `main` at the moment of
//!   painting, plus a guard) are never painted, so `used` is never below
//!   that. They are counted as used.
//! * `used == reserved` means the pattern is gone all the way down to
//!   `__sheap`: the stack has reached (and probably overrun) the statics.

#![cfg_attr(not(test), no_std)]

/// The paint. Not a value the stack is likely to hold: not a small integer,
/// not a RAM (0x2000_0000..) or flash (0x0008_0000..) address, not 0 or
/// all-ones, and odd, so not an aligned pointer either.
pub const PATTERN: u32 = 0xC5C5_C5C5;

/// `0xE0` sub-command number (packet[1]). 6..8 are the write session
/// (`discimg::wsession`), 2, 3 and 5 the image control in `ide.rs`.
pub const SUB_STACK: u8 = 9;

/// VERSION reply byte 5, bit 3: this firmware answers [`SUB_STACK`].
pub const FEATURE_BIT: u8 = 0x08;

/// First four bytes of the reply.
pub const MAGIC: [u8; 4] = *b"NGSK";

/// Reply layout version (byte 4). Bump when a field moves or changes meaning;
/// new fields go after the last one, so a host that reads fewer bytes still
/// understands the ones it reads.
pub const FORMAT: u8 = 1;

/// Length of the reply in bytes.
pub const REPLY_LEN: usize = 20;

/// Number of words at the bottom of the stack region that still hold
/// [`PATTERN`], scanning upward from index 0 (the lowest address, `__sheap`)
/// and stopping at the first word that does not. `read(i)` returns word `i`;
/// on the target it is a volatile read, so no slice over live stack memory is
/// ever formed.
pub fn untouched_words(len: usize, mut read: impl FnMut(usize) -> u32) -> usize {
    let mut i = 0;
    while i < len && read(i) == PATTERN {
        i += 1;
    }
    i
}

/// The STACK reply, all byte counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Report {
    /// `_stack_start - __sheap`: the whole region the stack may use.
    pub reserved: u32,
    /// High-water mark: `reserved` minus the bytes that still hold the
    /// pattern. Never below `unpainted`.
    pub used: u32,
    /// Bytes at the top that were not painted (`main`'s frame and the guard).
    /// 0 means the stack was not painted at all (then `used == reserved`).
    pub unpainted: u32,
}

impl Report {
    /// From the region size and the untouched-word count.
    pub fn new(reserved: u32, untouched_words: usize, unpainted: u32) -> Self {
        let free = (untouched_words as u32).saturating_mul(4).min(reserved);
        Report { reserved, used: reserved - free, unpainted }
    }

    /// Bytes still free below the high-water mark.
    pub fn free(&self) -> u32 {
        self.reserved - self.used
    }

    /// The reply bytes:
    ///
    /// | bytes  | field                                   |
    /// |--------|-----------------------------------------|
    /// | 0..4   | "NGSK"                                  |
    /// | 4      | format = 1                              |
    /// | 5..8   | reserved, zero                          |
    /// | 8..12  | `reserved`, u32 little-endian           |
    /// | 12..16 | `used` (high-water), u32 little-endian  |
    /// | 16..20 | `unpainted`, u32 little-endian          |
    pub fn encode(&self) -> [u8; REPLY_LEN] {
        let mut b = [0u8; REPLY_LEN];
        b[..4].copy_from_slice(&MAGIC);
        b[4] = FORMAT;
        b[8..12].copy_from_slice(&self.reserved.to_le_bytes());
        b[12..16].copy_from_slice(&self.used.to_le_bytes());
        b[16..20].copy_from_slice(&self.unpainted.to_le_bytes());
        b
    }

    /// Inverse of [`encode`](Self::encode), for tests and host tools. `None`
    /// for a wrong magic, an unknown (lower) format or too few bytes.
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() < 16 || b[..4] != MAGIC || b[4] < 1 {
            return None;
        }
        let w = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        let unpainted = if b.len() >= 20 { w(16) } else { 0 };
        Some(Report { reserved: w(8), used: w(12), unpainted })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(words: &[u32]) -> usize {
        untouched_words(words.len(), |i| words[i])
    }

    #[test]
    fn untouched_counts_from_the_bottom() {
        let p = PATTERN;
        assert_eq!(scan(&[]), 0);
        assert_eq!(scan(&[p, p, p]), 3);
        assert_eq!(scan(&[1, p, p]), 0);
        assert_eq!(scan(&[p, p, 0, p, 7]), 2);
        // A stale pattern word above the first used word does not count: the
        // stack below it was used, whatever it left behind.
        assert_eq!(scan(&[p, 0, p, p, p]), 1);
    }

    #[test]
    fn scan_stops_at_the_first_used_word() {
        let mut reads = 0;
        let words = [PATTERN, PATTERN, 5, 6, 7];
        let n = untouched_words(words.len(), |i| {
            reads += 1;
            words[i]
        });
        assert_eq!((n, reads), (2, 3));
    }

    #[test]
    fn report_numbers() {
        // 2E release-z resident: 7,128 bytes reserved, 2,000 bytes untouched.
        let r = Report::new(7128, 500, 96);
        assert_eq!(r, Report { reserved: 7128, used: 5128, unpainted: 96 });
        assert_eq!(r.free(), 2000);
        // Overrun: nothing untouched.
        assert_eq!(Report::new(7128, 0, 96).used, 7128);
        // More untouched words than the region holds cannot underflow.
        assert_eq!(Report::new(8, 100, 0).used, 0);
    }

    #[test]
    fn encode_layout() {
        let r = Report { reserved: 0x1BD8, used: 0x1408, unpainted: 0x60 };
        let b = r.encode();
        assert_eq!(&b[..4], b"NGSK");
        assert_eq!(b[4], 1);
        assert_eq!(&b[5..8], &[0, 0, 0]);
        assert_eq!(&b[8..12], &[0xD8, 0x1B, 0, 0]);
        assert_eq!(&b[12..16], &[0x08, 0x14, 0, 0]);
        assert_eq!(&b[16..20], &[0x60, 0, 0, 0]);
        assert_eq!(Report::decode(&b), Some(r));
    }

    #[test]
    fn decode_rejects_and_tolerates() {
        let b = Report { reserved: 10, used: 4, unpainted: 2 }.encode();
        let mut bad = b;
        bad[0] = b'X';
        assert_eq!(Report::decode(&bad), None);
        let mut v0 = b;
        v0[4] = 0;
        assert_eq!(Report::decode(&v0), None);
        assert_eq!(Report::decode(&b[..15]), None);
        // A host that asked for 16 bytes (allocation length) gets the first
        // two fields.
        assert_eq!(Report::decode(&b[..16]), Some(Report { reserved: 10, used: 4, unpainted: 0 }));
        // A later format with more fields still decodes.
        let mut v2 = [0u8; 24];
        v2[..20].copy_from_slice(&b);
        v2[4] = 2;
        assert_eq!(Report::decode(&v2), Some(Report { reserved: 10, used: 4, unpainted: 2 }));
    }
}
