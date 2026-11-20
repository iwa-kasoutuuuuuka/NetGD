//! The updater's pure decisions, kept free of registers so they can run (and
//! be tested) on the host: `cargo test --lib --target x86_64-pc-windows-msvc`.
//!
//! `tools/fwimage/fwimage.py` implements the same checks; both are tested
//! against the shared cases in `tools/fwimage/vectors/`, so a change here that
//! the host tool does not follow fails one of the two test suites.
//!
//! # Image header (format version 2)
//!
//! One 512-byte block, little-endian 32-bit words, the rest zero:
//!
//! ```text
//!   w0  magic          "OGFW" = 0x5746474F
//!   w1  format         2
//!   w2  kind           0 = code (plane 0, starts with the vector table)
//!                      1 = bitstream (plane 1, SAM3U4E only)
//!   w3  target_cidr    CHIPID_CIDR of the part the image is for
//!                      (VERSION bits [4:0] are ignored on compare)
//!   w4  dest           flash address of the first byte
//!   w5  len            payload bytes, a multiple of 512
//!   w6  payload_crc32  CRC-32 (zlib) of the `len` payload bytes
//!   w7  image_version  free-form build number, informational
//!   w8  header_crc32   CRC-32 of w0..w7 (the first 32 bytes)
//! ```

#![cfg_attr(not(test), no_std)]

pub const MAGIC: u32 = 0x5746_474F;
pub const FORMAT_VERSION: u32 = 2;

pub const KIND_CODE: u32 = 0;
pub const KIND_BITSTREAM: u32 = 1;

/// Words of the header block that carry fields, the CRC word included.
pub const HEADER_WORDS: usize = 9;

pub const PLANE0_BASE: u32 = 0x0008_0000;
pub const PLANE1_BASE: u32 = 0x0010_0000;
pub const PLANE_LEN: u32 = 0x0002_0000;
/// Flash page size, and the granularity of EWP.
pub const PAGE: u32 = 256;
/// Size of one lock region (SAM3U datasheet table 8-3).
pub const LOCK_REGION: u32 = 8192;

pub const SRAM0_BASE: u32 = 0x2000_0000;

pub const CIDR_SAM3U2E: u32 = 0x281A_0761;
pub const CIDR_SAM3U4E: u32 = 0x2810_0961;
/// CHIPID_CIDR[4:0] is VERSION (silicon revision); everything above it
/// identifies the part.
pub const CIDR_VERSION_MASK: u32 = 0x1F;

/// Why a header or vector table was refused. `main.rs` maps these onto the
/// ERROR-register codes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reject {
    BadHeader,
    BadTarget,
    BadDest,
    BadLength,
    BadVector,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    Sam3u2e,
    Sam3u4e,
}

impl Part {
    /// Identify the running part from CHIPID_CIDR, ignoring the silicon
    /// revision. `None` for anything that is not one of the two supported
    /// parts: such a board gets no updates at all.
    pub fn from_cidr(cidr: u32) -> Option<Part> {
        match cidr & !CIDR_VERSION_MASK {
            c if c == CIDR_SAM3U2E & !CIDR_VERSION_MASK => Some(Part::Sam3u2e),
            c if c == CIDR_SAM3U4E & !CIDR_VERSION_MASK => Some(Part::Sam3u4e),
            _ => None,
        }
    }

    /// One past the last byte of SRAM0. (SRAM1 at 0x2008_0000 is not a valid
    /// initial stack for the firmware: it holds the staging buffers.)
    pub fn sram0_end(self) -> u32 {
        match self {
            Part::Sam3u2e => SRAM0_BASE + 0x4000,
            Part::Sam3u4e => SRAM0_BASE + 0x8000,
        }
    }

    /// Only the 4E has the second flash plane (and EEFC1).
    pub fn has_plane1(self) -> bool {
        self == Part::Sam3u4e
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Header {
    pub kind: u32,
    pub target_cidr: u32,
    pub dest: u32,
    pub len: u32,
    pub crc: u32,
    pub image_version: u32,
}

// --- CRC-32 (reflected, 0xEDB88320: zlib, PNG) ------------------------------
//
// Bitwise rather than table-driven: a table is 1 KB in a blob that has to be
// copied into RAM. The state is the raw register (starts at `CRC_INIT`);
// `crc_finish` is the final inversion.

pub const CRC_INIT: u32 = 0xFFFF_FFFF;

#[inline]
pub fn crc_byte(mut crc: u32, b: u8) -> u32 {
    crc ^= b as u32;
    for _ in 0..8 {
        let mask = 0u32.wrapping_sub(crc & 1);
        crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
    }
    crc
}

/// Feed one little-endian word, i.e. the four bytes as they sit in memory.
#[inline]
pub fn crc_word(mut crc: u32, w: u32) -> u32 {
    for b in w.to_le_bytes() {
        crc = crc_byte(crc, b);
    }
    crc
}

#[inline]
pub fn crc_finish(crc: u32) -> u32 {
    !crc
}

pub fn crc32(data: &[u8]) -> u32 {
    let mut c = CRC_INIT;
    for &b in data {
        c = crc_byte(c, b);
    }
    crc_finish(c)
}

// --- header -----------------------------------------------------------------

/// Parse the field words of a header block. Checks only what is intrinsic to
/// the block (magic, format, header CRC, known kind); whether the image suits
/// *this* board is [`check_header`].
pub fn parse_header(w: &[u32; HEADER_WORDS]) -> Result<Header, Reject> {
    if w[0] != MAGIC || w[1] != FORMAT_VERSION {
        return Err(Reject::BadHeader);
    }
    let mut c = CRC_INIT;
    for &x in &w[..8] {
        c = crc_word(c, x);
    }
    if crc_finish(c) != w[8] {
        return Err(Reject::BadHeader);
    }
    if w[2] != KIND_CODE && w[2] != KIND_BITSTREAM {
        return Err(Reject::BadHeader);
    }
    Ok(Header {
        kind: w[2],
        target_cidr: w[3],
        dest: w[4],
        len: w[5],
        crc: w[6],
        image_version: w[7],
    })
}

/// Is this image for this part, and does it go where such an image may go?
///
/// A code image always replaces the whole plane-0 image from its first byte
/// (the vector table); a bitstream image goes to the start of plane 1 and
/// only a part that has one accepts it. There is deliberately no way to write
/// anywhere else.
pub fn check_header(h: &Header, part: Part) -> Result<(), Reject> {
    if Part::from_cidr(h.target_cidr) != Some(part) {
        return Err(Reject::BadTarget);
    }
    let ok_dest = match h.kind {
        KIND_CODE => h.dest == PLANE0_BASE,
        _ => h.dest == PLANE1_BASE && part.has_plane1(),
    };
    if !ok_dest {
        return Err(Reject::BadDest);
    }
    if h.len == 0 || h.len % 512 != 0 || h.len > PLANE_LEN {
        return Err(Reject::BadLength);
    }
    Ok(())
}

/// Number of lock regions an image of `len` bytes spans, as a GLB bit mask.
pub fn lock_mask(len: u32) -> u32 {
    let n = (len + LOCK_REGION - 1) / LOCK_REGION; // at most 16
    ((1u64 << n) - 1) as u32
}

// --- vector table -----------------------------------------------------------

/// Check the first four words of a code image: initial SP, Reset, NMI,
/// HardFault. A reset into garbage is the one failure that cannot be undone
/// from the host, so this runs before a single page is erased.
///
/// * SP: word aligned, inside SRAM0 *of this part* (`0x2000_0000 < sp <=
///   sram0_end`: the top of the bank is a valid initial value).
/// * handlers: Thumb (odd), and the target inside the image being written.
pub fn check_vectors(v: &[u32; 4], dest: u32, len: u32, part: Part) -> Result<(), Reject> {
    let sp = v[0];
    if sp % 4 != 0 || sp <= SRAM0_BASE || sp > part.sram0_end() {
        return Err(Reject::BadVector);
    }
    let end = dest.saturating_add(len);
    for &h in &v[1..] {
        let a = h & !1;
        if h & 1 == 0 || a < dest || a >= end {
            return Err(Reject::BadVector);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(s: &str) -> Part {
        match s {
            "2e" => Part::Sam3u2e,
            "4e" => Part::Sam3u4e,
            _ => panic!("part {s}"),
        }
    }

    fn num(s: &str) -> u32 {
        u32::from_str_radix(s.trim_start_matches("0x"), 16).unwrap()
    }

    fn hex(s: &str) -> Vec<u8> {
        if s == "-" {
            return vec![];
        }
        (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect()
    }

    fn cases(text: &'static str) -> impl Iterator<Item = Vec<&'static str>> {
        text.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| l.split_whitespace().collect())
    }

    #[test]
    fn crc_vectors() {
        let mut n = 0;
        for c in cases(include_str!("../../../tools/fwimage/vectors/crc32.txt")) {
            assert_eq!(crc32(&hex(c[0])), num(c[1]), "{}", c[0]);
            n += 1;
        }
        assert!(n >= 4);
    }

    #[test]
    fn crc_word_matches_bytes() {
        let w = 0x1234_5678u32;
        assert_eq!(crc_word(CRC_INIT, w), w.to_le_bytes().iter().fold(CRC_INIT, |c, &b| crc_byte(c, b)));
    }

    #[test]
    fn vtable_vectors() {
        let mut n = 0;
        for c in cases(include_str!("../../../tools/fwimage/vectors/vtable.txt")) {
            // part dest len sp reset nmi hardfault expect
            let v = [num(c[3]), num(c[4]), num(c[5]), num(c[6])];
            let r = check_vectors(&v, num(c[1]), num(c[2]), part(c[0]));
            let want = if c[7] == "OK" { Ok(()) } else { Err(Reject::BadVector) };
            assert_eq!(r, want, "{c:?}");
            n += 1;
        }
        assert!(n >= 10);
    }

    fn words(hex_str: &str) -> [u32; HEADER_WORDS] {
        let b = hex(hex_str);
        assert_eq!(b.len(), HEADER_WORDS * 4);
        let mut w = [0u32; HEADER_WORDS];
        for i in 0..HEADER_WORDS {
            w[i] = u32::from_le_bytes(b[4 * i..4 * i + 4].try_into().unwrap());
        }
        w
    }

    #[test]
    fn header_vectors() {
        let mut n = 0;
        for c in cases(include_str!("../../../tools/fwimage/vectors/header.txt")) {
            // part header-hex expect
            let r = parse_header(&words(c[1])).and_then(|h| check_header(&h, part(c[0])));
            let want = match c[2] {
                "OK" => Ok(()),
                "BadHeader" => Err(Reject::BadHeader),
                "BadTarget" => Err(Reject::BadTarget),
                "BadDest" => Err(Reject::BadDest),
                "BadLength" => Err(Reject::BadLength),
                x => panic!("{x}"),
            };
            assert_eq!(r, want, "{c:?}");
            n += 1;
        }
        assert!(n >= 10);
    }

    #[test]
    fn cidr_and_masks() {
        assert_eq!(Part::from_cidr(0x281A_0761), Some(Part::Sam3u2e));
        assert_eq!(Part::from_cidr(0x281A_0762), Some(Part::Sam3u2e)); // later silicon rev
        assert_eq!(Part::from_cidr(0x2810_0961), Some(Part::Sam3u4e));
        assert_eq!(Part::from_cidr(0x2819_0561), None); // 3U1E
        assert_eq!(Part::from_cidr(0), None);
        assert_eq!(lock_mask(512), 1);
        assert_eq!(lock_mask(8192), 1);
        assert_eq!(lock_mask(8193), 3);
        assert_eq!(lock_mask(PLANE_LEN), 0xFFFF);
    }
}
