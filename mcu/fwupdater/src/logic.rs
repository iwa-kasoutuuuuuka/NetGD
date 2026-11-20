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
//!
//! # Format 3 (boards with a resident boot region)
//!
//! `docs/design/fw-bootloader.md` section 2-6, `tools/fwimage/FORMAT.md`. Same
//! words w0..w7, `format = 3`, plus `w9 = layout` (size of the resident
//! region, 0x2000); `header_crc32` (w8) is the CRC-32 of w0..w7 *followed by*
//! w9. A code image goes to `APP_BASE` (0x0008_2000) exactly, at most
//! 0x20000 - layout bytes, with the "NGAP" descriptor at +0x100 and the CRC-32
//! of everything before it in its last four bytes. Which format a build
//! accepts is the cargo feature `resident` (format 3 only; without it, format
//! 2 only). The format-specific functions exist in both builds so the host
//! tests cover both.

#![cfg_attr(not(test), no_std)]

pub const MAGIC: u32 = 0x5746_474F;
/// Format 2: no resident region, code at the start of plane 0.
pub const FORMAT_2: u32 = 2;
/// Format 3: resident region first, app at `APP_BASE`.
pub const FORMAT_3: u32 = 3;
/// The format this build accepts (see the module docs).
pub const FORMAT_VERSION: u32 = if RESIDENT { FORMAT_3 } else { FORMAT_2 };
/// Built for a board with a resident boot region?
pub const RESIDENT: bool = cfg!(feature = "resident");

pub const KIND_CODE: u32 = 0;
pub const KIND_BITSTREAM: u32 = 1;

/// Words of the header block that carry fields, the CRC word included, in
/// the format this build accepts.
pub const HEADER_WORDS: usize = if RESIDENT { HEADER_WORDS_V3 } else { HEADER_WORDS_V2 };
pub const HEADER_WORDS_V2: usize = 9;
pub const HEADER_WORDS_V3: usize = 10;

/// Size of the resident region, and so the offset of the app in plane 0.
pub const LAYOUT: u32 = 0x2000;
/// Where a format-3 code image goes and where the app is linked.
pub const APP_BASE: u32 = PLANE0_BASE + LAYOUT;
/// The app descriptor sits at this offset of the app image, 32 bytes.
pub const DESC_OFFSET: usize = 0x100;
pub const DESC_LEN: usize = 32;
pub const DESC_MAGIC: [u8; 4] = *b"NGAP";
pub const DESC_VERSION: u32 = 1;
/// The image CRC-32 in the last four bytes of a format-3 code image.
pub const TRAILER_LEN: usize = 4;

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
    /// 2 or 3.
    pub format: u32,
    /// Size of the resident region the image was built for; 0 in format 2.
    pub layout: u32,
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

/// Parse the field words of a format-2 header block. Checks only what is
/// intrinsic to the block (magic, format, header CRC, known kind); whether the
/// image suits *this* board is [`check_header`].
pub fn parse_header_v2(w: &[u32; HEADER_WORDS_V2]) -> Result<Header, Reject> {
    if w[0] != MAGIC || w[1] != FORMAT_2 {
        return Err(Reject::BadHeader);
    }
    let mut c = CRC_INIT;
    for &x in &w[..8] {
        c = crc_word(c, x);
    }
    if crc_finish(c) != w[8] {
        return Err(Reject::BadHeader);
    }
    fields(w, 0)
}

/// The same for a format-3 header: ten words, `w9` = layout, and the header
/// CRC covers w0..w7 and then w9 (everything except the CRC word).
pub fn parse_header_v3(w: &[u32; HEADER_WORDS_V3]) -> Result<Header, Reject> {
    if w[0] != MAGIC || w[1] != FORMAT_3 {
        return Err(Reject::BadHeader);
    }
    let mut c = CRC_INIT;
    for &x in &w[..8] {
        c = crc_word(c, x);
    }
    c = crc_word(c, w[9]);
    if crc_finish(c) != w[8] {
        return Err(Reject::BadHeader);
    }
    fields(w, w[9])
}

fn fields(w: &[u32], layout: u32) -> Result<Header, Reject> {
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
        format: w[1],
        layout,
    })
}

/// The header parser of this build: format 3 with the `resident` feature,
/// format 2 without it.
pub fn parse_header(w: &[u32; HEADER_WORDS]) -> Result<Header, Reject> {
    #[cfg(feature = "resident")]
    {
        parse_header_v3(w)
    }
    #[cfg(not(feature = "resident"))]
    {
        parse_header_v2(w)
    }
}

/// Is this image for this part, and does it go where such an image may go?
///
/// Format 2: a code image replaces the whole plane-0 image from its first byte
/// (the vector table). Format 3: it goes to exactly `APP_BASE` (anything
/// before it is the resident region and is `BadDest` whatever the length), is
/// at most `PLANE_LEN - LAYOUT` long, and was built for this `LAYOUT`. A
/// bitstream image goes to the start of plane 1 and only a part that has one
/// accepts it. There is deliberately no way to write anywhere else.
pub fn check_header(h: &Header, part: Part) -> Result<(), Reject> {
    if Part::from_cidr(h.target_cidr) != Some(part) {
        return Err(Reject::BadTarget);
    }
    let v3 = h.format == FORMAT_3;
    if v3 && h.layout != LAYOUT {
        return Err(Reject::BadDest);
    }
    let ok_dest = match h.kind {
        KIND_CODE => h.dest == if v3 { APP_BASE } else { PLANE0_BASE },
        _ => h.dest == PLANE1_BASE && part.has_plane1(),
    };
    if !ok_dest {
        return Err(Reject::BadDest);
    }
    let limit = if v3 && h.kind == KIND_CODE { PLANE_LEN - LAYOUT } else { PLANE_LEN };
    if h.len == 0 || h.len % 512 != 0 || h.len > limit {
        return Err(Reject::BadLength);
    }
    Ok(())
}

/// Lock regions an image of `len` bytes spans from the start of its plane, as
/// a GLB bit mask (format 2: every image starts at a plane base).
pub fn lock_mask(len: u32) -> u32 {
    lock_mask_at(PLANE0_BASE, len)
}

/// The GLB bit mask (EEFC_FRR bit n = lock region n *of the plane the
/// address is in*) of the regions under `[dest, dest + len)`. With a resident
/// region the app starts in region 1, so the mask is shifted up: the app
/// image at `APP_BASE` covers bits 1.., never bit 0 (the resident region).
pub fn lock_mask_at(dest: u32, len: u32) -> u32 {
    if len == 0 {
        return 0;
    }
    let base = if dest >= PLANE1_BASE { PLANE1_BASE } else { PLANE0_BASE };
    let first = (dest - base) / LOCK_REGION;
    let last = (dest - base + len - 1) / LOCK_REGION;
    let hi = if last >= 31 { u32::MAX } else { (1u32 << (last + 1)) - 1 };
    let lo = (1u32 << first) - 1;
    hi & !lo
}

/// The lock regions this build must find unlocked before it writes
/// `[dest, dest + len)`: [`lock_mask_at`]. `main.rs` uses this one (not
/// [`lock_mask`], which assumes the image starts at a plane base and would
/// test bit 0, the resident's own lock, for a format-3 image at `APP_BASE`:
/// on a board whose resident is locked every update would then be refused as
/// `Locked`). For a header [`check_header`] accepted in a `resident` build
/// the mask never has bit 0 of plane 0.
pub fn lock_mask_of(dest: u32, len: u32) -> u32 {
    lock_mask_at(dest, len)
}

// --- EEFC_FCR ----------------------------------------------------------------

/// Flash writing protection key (FKEY, `EEFC_FCR` bits 31:24).
pub const FKEY: u32 = 0x5A;
/// Erase page and write page.
pub const FCMD_EWP: u32 = 0x03;
/// Pages of the resident region (lock region 0 of plane 0).
pub const RESIDENT_PAGES: u32 = LAYOUT / PAGE;
/// `RSTC_CR` (0x400E_1200) word: key 0xA5, PERRST, PROCRST. What a `resident`
/// build resets with (contract X7; `AIRCR.SYSRESETREQ` leaves it open
/// whether the peripherals are reset).
pub const RSTC_CR_PROC_PERI_RESET: u32 = 0xA500_0005;

/// The one place where this crate makes the `EEFC_FCR` word of a page write:
/// `(plane 1?, word)` for the page at flash address `addr`, `None` when this
/// build may not write there. Same words as `bootcore::fcr::FcrCmd::ewp`
/// (tested against it). `addr` must be page aligned and inside a plane; in a
/// `resident` build (`resident` = true) pages 0..31 of plane 0, the resident
/// region, cannot be named at all (contract X3), whatever the header said.
/// Whether the part has a plane 1 is `check_header`'s business.
pub fn ewp_fcr_for(addr: u32, resident: bool) -> Option<(bool, u32)> {
    if addr % PAGE != 0 {
        return None;
    }
    let (plane1, base) = if addr >= PLANE1_BASE { (true, PLANE1_BASE) } else { (false, PLANE0_BASE) };
    if addr < base || addr - base >= PLANE_LEN {
        return None;
    }
    let page = (addr - base) / PAGE;
    if resident && !plane1 && page < RESIDENT_PAGES {
        return None;
    }
    Some((plane1, (FKEY << 24) | (page << 8) | FCMD_EWP))
}

/// [`ewp_fcr_for`] of this build.
pub fn ewp_fcr(addr: u32) -> Option<(bool, u32)> {
    ewp_fcr_for(addr, RESIDENT)
}

// --- app descriptor and trailer (format 3) ----------------------------------

/// Why a format-3 code image was refused after the header passed. (Separate
/// from [`Reject`] because `main.rs` maps `Reject` onto status codes
/// exhaustively; stage S5 decides what these become on the wire.)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ImageFault {
    /// No "NGAP" descriptor, wrong version, bad descriptor CRC, or a length
    /// that is not the image's.
    BadDescriptor,
    /// A well-formed descriptor whose length is 0: linked but not sealed.
    Unsealed,
    /// The image CRC in the last four bytes does not match.
    BadTrailer,
}

/// The fields of the app descriptor (design 2-5). Only `length` is the
/// resident region's business; the rest is for the app and for `fwimage`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Descriptor {
    pub length: u32,
    pub bitstream_addr: u32,
    pub bitstream_len: u32,
    pub bitstream_raw_len: u32,
    pub kind_version: u32,
}

fn le32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// Check the 32 bytes at `APP_BASE + 0x100` of an image `image_len` bytes
/// long (trailer included). Order: magic, version, length 0 -> `Unsealed`, CRC
/// over the first 28 bytes, length equal to `image_len` and a multiple of 512.
pub fn check_descriptor(d: &[u8; DESC_LEN], image_len: u32) -> Result<Descriptor, ImageFault> {
    if d[..4] != DESC_MAGIC || le32(d, 4) != DESC_VERSION {
        return Err(ImageFault::BadDescriptor);
    }
    let length = le32(d, 8);
    if length == 0 {
        return Err(ImageFault::Unsealed);
    }
    if crc32(&d[..28]) != le32(d, 28) {
        return Err(ImageFault::BadDescriptor);
    }
    if length != image_len || length % 512 != 0 {
        return Err(ImageFault::BadDescriptor);
    }
    Ok(Descriptor {
        length,
        bitstream_addr: le32(d, 12),
        bitstream_len: le32(d, 16),
        bitstream_raw_len: le32(d, 20),
        kind_version: le32(d, 24),
    })
}

/// The last four bytes of `image` (little-endian) are the CRC-32 of the bytes
/// before them.
pub fn check_trailer(image: &[u8]) -> Result<(), ImageFault> {
    if image.len() < TRAILER_LEN {
        return Err(ImageFault::BadTrailer);
    }
    let (body, t) = image.split_at(image.len() - TRAILER_LEN);
    if crc32(body) != le32(t, 0) {
        return Err(ImageFault::BadTrailer);
    }
    Ok(())
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

    fn words<const N: usize>(hex_str: &str) -> [u32; N] {
        let b = hex(hex_str);
        assert_eq!(b.len(), N * 4);
        let mut w = [0u32; N];
        for i in 0..N {
            w[i] = u32::from_le_bytes(b[4 * i..4 * i + 4].try_into().unwrap());
        }
        w
    }

    fn reject_name(r: Result<(), Reject>) -> String {
        match r {
            Ok(()) => "OK".into(),
            Err(e) => format!("{e:?}"),
        }
    }

    #[test]
    fn header_vectors() {
        // format 2 (header.txt): what a board without a resident region accepts
        let mut n = 0;
        for c in cases(include_str!("../../../tools/fwimage/vectors/header.txt")) {
            // part header-hex(36 bytes) expect
            let r = parse_header_v2(&words::<9>(c[1])).and_then(|h| check_header(&h, part(c[0])));
            assert_eq!(reject_name(r), c[2], "{c:?}");
            n += 1;
        }
        assert!(n >= 10);
    }

    #[test]
    fn header3_vectors() {
        // format 3 (header3.txt): what a board with a resident region accepts
        let mut n = 0;
        for c in cases(include_str!("../../../tools/fwimage/vectors/header3.txt")) {
            // part header-hex(40 bytes) expect
            let r = parse_header_v3(&words::<10>(c[1])).and_then(|h| check_header(&h, part(c[0])));
            assert_eq!(reject_name(r), c[2], "{c:?}");
            n += 1;
        }
        assert!(n >= 20);
    }

    #[test]
    fn this_build_parses_its_own_format() {
        // `parse_header` is v3 under the feature, v2 without it
        let v2 = cases(include_str!("../../../tools/fwimage/vectors/header.txt")).next().unwrap()[1];
        let v3 = cases(include_str!("../../../tools/fwimage/vectors/header3.txt")).next().unwrap()[1];
        let h3 = parse_header_v3(&words::<10>(v3));
        assert!(h3.is_ok(), "{h3:?}");
        assert_eq!(parse_header_v2(&words::<9>(v2)).map(|h| (h.format, h.layout)), Ok((2, 0)));
        if RESIDENT {
            assert_eq!(HEADER_WORDS, 10);
            assert_eq!(parse_header(&words::<HEADER_WORDS>(v3)), h3);
            assert_eq!(FORMAT_VERSION, 3);
        } else {
            assert_eq!(HEADER_WORDS, 9);
            assert_eq!(parse_header(&words::<HEADER_WORDS>(v2)).map(|h| h.format), Ok(2));
            assert_eq!(FORMAT_VERSION, 2);
        }
    }

    #[test]
    fn descriptor_vectors() {
        let mut n = 0;
        for c in cases(include_str!("../../../tools/fwimage/vectors/desc.txt")) {
            // descriptor-hex image-len expect
            let d: [u8; DESC_LEN] = hex(c[0]).try_into().unwrap();
            let r = check_descriptor(&d, num(c[1])).map(|_| ());
            let got = match r {
                Ok(()) => "OK".to_string(),
                Err(e) => format!("{e:?}"),
            };
            assert_eq!(got, c[2], "{c:?}");
            n += 1;
        }
        assert!(n >= 10);
    }

    #[test]
    fn descriptor_fields() {
        let d: [u8; DESC_LEN] = hex(
            "4e47415001000000000200000000100000900000000002000700000100000000",
        )
        .try_into()
        .unwrap();
        // (the CRC word above is wrong on purpose: only the fields are read here)
        let mut ok = d;
        ok[28..].copy_from_slice(&crc32(&d[..28]).to_le_bytes());
        let got = check_descriptor(&ok, 0x200).unwrap();
        assert_eq!(
            got,
            Descriptor {
                length: 0x200,
                bitstream_addr: 0x0010_0000,
                bitstream_len: 0x9000,
                bitstream_raw_len: 0x2_0000,
                kind_version: 0x0100_0007
            }
        );
        assert_eq!(check_descriptor(&d, 0x200), Err(ImageFault::BadDescriptor));
    }

    #[test]
    fn trailer_vectors() {
        let mut n = 0;
        for c in cases(include_str!("../../../tools/fwimage/vectors/trailer.txt")) {
            // image-hex expect
            let got = match check_trailer(&hex(c[0])) {
                Ok(()) => "OK".to_string(),
                Err(e) => format!("{e:?}"),
            };
            assert_eq!(got, c[1], "{c:?}");
            n += 1;
        }
        assert!(n >= 8);
    }

    /// One format-3 file as the resident-region reader checks it: header,
    /// payload CRC, vectors, descriptor, trailer, in that order.
    fn check_file3(file: &[u8], board: Part) -> String {
        assert!(file.len() > 512);
        let w: [u32; 10] = words::<10>(&hex_of(&file[..40]));
        let h = match parse_header_v3(&w).and_then(|h| check_header(&h, board).map(|_| h)) {
            Ok(h) => h,
            Err(e) => return format!("{e:?}"),
        };
        let payload = &file[512..];
        assert_eq!(payload.len() as u32, h.len, "test file is not a single whole image");
        if crc32(payload) != h.crc {
            return "CrcMismatch".into();
        }
        if h.kind == KIND_CODE {
            let v = [le32(payload, 0), le32(payload, 4), le32(payload, 8), le32(payload, 12)];
            if let Err(e) = check_vectors(&v, h.dest, h.len, board) {
                return format!("{e:?}");
            }
            let d: [u8; DESC_LEN] = payload[DESC_OFFSET..DESC_OFFSET + DESC_LEN].try_into().unwrap();
            if let Err(e) = check_descriptor(&d, h.len) {
                return format!("{e:?}");
            }
            if let Err(e) = check_trailer(payload) {
                return format!("{e:?}");
            }
        }
        "OK".into()
    }

    fn hex_of(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn image3_vectors() {
        let mut n = 0;
        for c in cases(include_str!("../../../tools/fwimage/vectors/image3.txt")) {
            // board-part file-hex expect
            let got = check_file3(&hex(c[1]), part(c[0]));
            assert_eq!(got, c[2], "{} {}", c[0], &c[1][..40]);
            n += 1;
        }
        assert!(n >= 10);
    }

    /// B10 of R89: the EWP words, against the resident's own producer.
    #[test]
    fn ewp_words_match_bootcore_and_spare_the_resident() {
        use bootcore::fcr::{FcrCmd, Plane};
        let b4 = bootcore::part::Part::Sam3u4e;
        assert_eq!(RESIDENT_PAGES, 32);
        assert_eq!(RESIDENT_PAGES, bootcore::fcr::RESIDENT_PAGES);
        assert_eq!(FKEY, bootcore::fcr::KEY);
        assert_eq!(FCMD_EWP, bootcore::fcr::CMD_EWP as u32);
        for (base, plane) in [(PLANE0_BASE, Plane::P0), (PLANE1_BASE, Plane::P1)] {
            for page in 0..PLANE_LEN / PAGE {
                let addr = base + page * PAGE;
                let theirs = FcrCmd::ewp(b4, plane, page).map(|c| (c.plane() == Plane::P1, c.word()));
                // a resident build names exactly the pages the resident's own rule allows
                assert_eq!(ewp_fcr_for(addr, true), theirs, "{addr:#x}");
                // a format-2 build may also write pages 0..31 of plane 0 (there is no resident)
                let ours = ewp_fcr_for(addr, false).unwrap();
                assert_eq!(ours, (plane == Plane::P1, 0x5A00_0003 | page << 8), "{addr:#x}");
                if let Some(t) = theirs {
                    assert_eq!(ours, t);
                } else {
                    assert!(plane == Plane::P0 && page < 32);
                }
                // not page aligned
                for d in [1, 4, 255] {
                    assert_eq!(ewp_fcr_for(addr + d, false), None);
                    assert_eq!(ewp_fcr_for(addr + d, true), None);
                }
            }
        }
        // the resident region, explicitly
        assert_eq!(ewp_fcr_for(PLANE0_BASE, true), None);
        assert_eq!(ewp_fcr_for(APP_BASE - PAGE, true), None);
        assert_eq!(ewp_fcr_for(APP_BASE, true), Some((false, 0x5A00_2003)));
        assert_eq!(ewp_fcr_for(PLANE0_BASE, false), Some((false, 0x5A00_0003)));
        // outside both planes
        for resident in [false, true] {
            for a in [0, PLANE0_BASE - PAGE, PLANE0_BASE + PLANE_LEN, PLANE1_BASE - PAGE, PLANE1_BASE + PLANE_LEN, 0x2000_0000, 0xFFFF_FF00] {
                assert_eq!(ewp_fcr_for(a, resident), None, "{a:#x}");
            }
        }
        // this build's own
        assert_eq!(ewp_fcr(PLANE0_BASE).is_some(), !RESIDENT);
        assert_eq!(ewp_fcr(APP_BASE), Some((false, 0x5A00_2003)));
        // no word is one of the dangerous commands (SGPB 0x0B, CGPB 0x0C, CLB 0x09, EA 0x05, SLB 0x08)
        assert_eq!(ewp_fcr_for(PLANE1_BASE + PLANE_LEN - PAGE, true).unwrap().1 & 0xFF, 3);
        assert_eq!(RSTC_CR_PROC_PERI_RESET, 0xA500_0005);
    }

    /// B10 of R89: the lock check of a format-3 image must not look at bit 0
    /// (the resident's lock, set on every board in the field).
    #[test]
    fn lock_mask_of_an_accepted_format3_image_never_has_the_resident_bit() {
        for len in (512..=PLANE_LEN - LAYOUT).step_by(512) {
            let m = lock_mask_of(APP_BASE, len);
            assert_eq!(m & 1, 0, "{len}");
            assert_ne!(m & 2, 0);
            assert_eq!(m, lock_mask_at(APP_BASE, len));
            assert_eq!(m, bootcore::update::lock_mask_at(APP_BASE, len), "{len}");
            // what `lock_mask` (plane-base images) would have tested instead
            assert_eq!(lock_mask(len) & 1, 1);
        }
        // a resident whose region is locked (GLB bit 0 = 1) does not make the app region "locked"
        assert_eq!(1 & lock_mask_of(APP_BASE, PLANE_LEN - LAYOUT), 0);
        // format 2 and bitstreams: unchanged
        for len in [512, 8192, 8193, PLANE_LEN] {
            assert_eq!(lock_mask_of(PLANE0_BASE, len), lock_mask(len));
            assert_eq!(lock_mask_of(PLANE1_BASE, len), lock_mask(len));
        }
    }

    #[test]
    fn layout_constants() {
        assert_eq!(APP_BASE, 0x0008_2000);
        assert_eq!(LAYOUT % LOCK_REGION, 0); // the resident region is whole lock regions
        assert_eq!(PLANE_LEN - LAYOUT, 0x1E000);
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

    #[test]
    fn lock_mask_shifted_by_destination_region() {
        // format 2 images start at a plane base: same as `lock_mask`
        assert_eq!(lock_mask_at(PLANE0_BASE, 512), 0x0001);
        assert_eq!(lock_mask_at(PLANE0_BASE, PLANE_LEN), 0xFFFF);
        assert_eq!(lock_mask_at(PLANE0_BASE, 8193), 0x0003);
        for len in [1, 512, 8192, 8193, 0x10000, PLANE_LEN] {
            assert_eq!(lock_mask_at(PLANE0_BASE, len), lock_mask(len));
        }
        // format 3: the app starts in lock region 1, the resident region (bit 0) is never in the mask
        assert_eq!(lock_mask_at(APP_BASE, 512), 0x0002);
        assert_eq!(lock_mask_at(APP_BASE, 8192), 0x0002);
        assert_eq!(lock_mask_at(APP_BASE, 8193), 0x0006);
        assert_eq!(lock_mask_at(APP_BASE, PLANE_LEN - LAYOUT), 0xFFFE);
        assert_eq!(lock_mask_at(APP_BASE, PLANE_LEN - LAYOUT) & 1, 0);
        // the resident region on its own (what a write to it would touch)
        assert_eq!(lock_mask_at(PLANE0_BASE, LAYOUT), 0x0001);
        assert_eq!(lock_mask_at(PLANE0_BASE + LAYOUT - 1, 2), 0x0003);
        // plane 1 is its own EEFC: region numbers restart at 0
        assert_eq!(lock_mask_at(PLANE1_BASE, PLANE_LEN), 0xFFFF);
        assert_eq!(lock_mask_at(PLANE1_BASE, 9000), 0x0003);
        assert_eq!(lock_mask_at(PLANE1_BASE + 8192, 512), 0x0002);
        assert_eq!(lock_mask_at(PLANE0_BASE, 0), 0);
    }
}
