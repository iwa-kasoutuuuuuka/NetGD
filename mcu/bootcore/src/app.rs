//! The application image as it sits in flash behind the resident
//! (design 2-5): vector table at `APP_BASE`, the 32-byte "NGAP" descriptor
//! at `APP_BASE + 0x100`, and the image's own CRC-32 in its last four bytes.
//!
//! ```text
//!   desc +0   magic "NGAP"
//!        +4   descriptor version (1)
//!        +8   image length, trailing CRC included, multiple of 512
//!        +12  bitstream address (not checked here)
//!        +16  bitstream length  (not checked here)
//!        +20  bitstream unpacked length (not checked here)
//!        +24  top 8 bits bitstream kind, low 24 bits app version
//!        +28  CRC-32 of the 28 bytes before
//! ```
//!
//! The resident only needs the length and the descriptor CRC; the bitstream
//! fields are for the application and for `fwimage`.

use crate::crc;
use crate::flash::Flash;
use crate::part::{Part, SRAM0_BASE};
use crate::rd32;

/// First byte of the application: the resident owns the 8 KB before it.
pub const APP_BASE: u32 = 0x0008_2000;
/// Size of the resident's lock region (word 9 of an OGFW format 3 header).
pub const LAYOUT: u32 = 0x2000;
pub const PLANE_LEN: u32 = 0x0002_0000;
/// Longest application image: the rest of plane 0.
pub const APP_MAX_LEN: u32 = PLANE_LEN - LAYOUT;
pub const DESC_OFF: u32 = 0x100;
pub const DESC_LEN: usize = 32;
/// `"NGAP"` read as a little-endian word.
pub const DESC_MAGIC: u32 = u32::from_le_bytes(*b"NGAP");
pub const DESC_VERSION: u32 = 1;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AppError {
    /// No descriptor: wrong magic or version (also what erased flash gives).
    Descriptor,
    /// A well-formed descriptor whose length is 0: linked but not sealed
    /// (only accepted when the caller allows it).
    Unsealed,
    /// Magic is right but the descriptor's own CRC is not.
    DescriptorCrc,
    /// Length not a multiple of 512, zero, or beyond the app region.
    Length,
    /// Initial SP or the Reset/NMI/HardFault handlers are not plausible.
    Vector,
    /// The CRC over the whole image does not match its trailing CRC.
    Crc,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Descriptor {
    /// 0 = unsealed (a dev build straight from the ELF, only accepted when
    /// the caller allows it).
    pub image_len: u32,
    pub bs_addr: u32,
    pub bs_len: u32,
    pub bs_raw_len: u32,
    pub bs_kind: u8,
    pub app_version: u32,
}

/// Parse and check a descriptor, in the order of `tools/fwimage/FORMAT.md`
/// (shared vectors `desc.txt`): magic and version, length 0 (`Unsealed`),
/// descriptor CRC, length a multiple of 512 within the app region.
///
/// `allow_unsealed` (the resident's `dev` feature): a descriptor with the
/// right magic and version and length 0 is accepted without looking at its
/// CRC (the ELF has zeros there until `fwimage seal` fills them in).
pub fn parse_descriptor(b: &[u8; DESC_LEN], allow_unsealed: bool) -> Result<Descriptor, AppError> {
    if rd32(b, 0) != DESC_MAGIC || rd32(b, 4) != DESC_VERSION {
        return Err(AppError::Descriptor);
    }
    let len = rd32(b, 8);
    let d = Descriptor {
        image_len: len,
        bs_addr: rd32(b, 12),
        bs_len: rd32(b, 16),
        bs_raw_len: rd32(b, 20),
        bs_kind: (rd32(b, 24) >> 24) as u8,
        app_version: rd32(b, 24) & 0x00FF_FFFF,
    };
    if len == 0 {
        return if allow_unsealed { Ok(d) } else { Err(AppError::Unsealed) };
    }
    if crc::crc32(&b[..28]) != rd32(b, 28) {
        return Err(AppError::DescriptorCrc);
    }
    if len < 512 || len % 512 != 0 || len > APP_MAX_LEN {
        return Err(AppError::Length);
    }
    Ok(d)
}

/// Descriptor of an image of exactly `image_len` bytes: [`parse_descriptor`]
/// (sealed only) and the descriptor's length equals the image's. Same verdicts
/// as `check_descriptor` in `mcu/fwupdater/src/logic.rs` (`Descriptor`,
/// `DescriptorCrc` and `Length` are its `BadDescriptor`).
pub fn check_descriptor(d: &[u8; DESC_LEN], image_len: u32) -> Result<Descriptor, AppError> {
    let x = parse_descriptor(d, false)?;
    if x.image_len != image_len {
        return Err(AppError::Descriptor);
    }
    Ok(x)
}

/// The last four bytes of `image` are the little-endian CRC-32 of the bytes
/// before them (`trailer.txt`).
pub fn check_trailer(image: &[u8]) -> bool {
    image.len() >= 4 && crc::crc32(image) == crc::RESIDUE
}

/// Check the first four words of an image: initial SP, Reset, NMI,
/// HardFault. Same rule as `check_vectors` in `mcu/fwupdater/src/logic.rs`
/// (and tested against the same `tools/fwimage/vectors/vtable.txt`):
///
/// * SP word aligned, `SRAM0_BASE < sp <= sram0_end` of this part;
/// * handlers odd (Thumb) and `h & !1` inside `[dest, dest + len)`.
pub fn check_vectors(v: &[u32; 4], dest: u32, len: u32, part: Part) -> Result<(), AppError> {
    let sp = v[0];
    if sp % 4 != 0 || sp <= SRAM0_BASE || sp > part.sram0_end() {
        return Err(AppError::Vector);
    }
    let end = dest.saturating_add(len);
    for &h in &v[1..] {
        let a = h & !1;
        if h & 1 == 0 || a < dest || a >= end {
            return Err(AppError::Vector);
        }
    }
    Ok(())
}

#[inline]
pub fn vectors_of(b: &[u8]) -> [u32; 4] {
    [rd32(b, 0), rd32(b, 4), rd32(b, 8), rd32(b, 12)]
}

/// Is the application in flash fit to jump to? Descriptor, vector table and
/// the CRC of the whole image (unless unsealed and allowed). Returns the
/// image length (0 for an unsealed dev image).
///
/// Cost: one pass over the image, 256 bytes at a time; about 18 ms at 96 MHz
/// for 118 KB with the nibble table (design 5-3, estimate).
pub fn verify_flash_app<F: Flash>(f: &mut F, part: Part, allow_unsealed: bool) -> Result<u32, AppError> {
    let mut db = [0u8; DESC_LEN];
    f.read(APP_BASE + DESC_OFF, &mut db);
    let d = parse_descriptor(&db, allow_unsealed)?;
    let sealed = d.image_len != 0;
    let len = if sealed { d.image_len } else { APP_MAX_LEN };

    let mut vb = [0u8; 16];
    f.read(APP_BASE, &mut vb);
    check_vectors(&vectors_of(&vb), APP_BASE, len, part)?;

    if sealed {
        let mut c = crc::INIT;
        let mut buf = [0u8; 256];
        let mut off = 0;
        while off < len {
            f.read(APP_BASE + off, &mut buf);
            c = crc::update(c, &buf);
            off += 256;
        }
        if crc::finish(c) != crc::RESIDUE {
            return Err(AppError::Crc);
        }
    }
    Ok(d.image_len)
}
