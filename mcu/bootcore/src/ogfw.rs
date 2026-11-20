//! OGFW format 3 (design 2-6): the file `NETGD.OGF` is one or two images,
//! each a 512-byte header block followed by `len` payload bytes.
//!
//! Header words (little-endian), as format 2 plus word 9:
//!
//! ```text
//!   w0 magic "OGFW" 0x5746474F     w5 len      (multiple of 512, non-zero)
//!   w1 format 3                    w6 payload_crc  CRC-32 of the payload
//!   w2 kind 0 app, 1 bitstream     w7 image_version
//!   w3 target_cidr                 w8 header_crc   CRC-32 of bytes 0..32
//!   w4 dest                        w9 layout 0x2000 (size of the resident)
//! ```
//!
//! `header_crc` (w8) is the CRC-32 of bytes 0..32 **followed by** bytes
//! 36..40 (w9), i.e. of every field but the CRC word: a flipped layout bit is
//! `BadHeader`. A layout other than `0x2000` is `BadDest`. Checks run in the
//! order of `tools/fwimage/FORMAT.md`: header (magic, format, CRC, kind),
//! target, layout, dest, length, payload CRC, vectors, descriptor, trailer
//! (shared vectors `header3.txt`, `image3.txt`).
//!
//! kind 0 payload: the application image (vector table, descriptor at +0x100,
//! trailing CRC-32 of everything before it). `dest` must be exactly
//! `APP_BASE`. Because the trailing CRC makes the CRC of the whole payload a
//! constant (`crc::RESIDUE`), `payload_crc` of a kind-0 image carries no
//! information of its own; it is checked anyway.
//!
//! kind 1 payload: the bitstream at plane 1 (`0x0010_0000`, SAM3U4E only).

use crate::app::{check_descriptor, check_vectors, vectors_of, AppError, APP_BASE, APP_MAX_LEN, DESC_OFF, DESC_LEN, LAYOUT, PLANE_LEN};
use crate::crc;
use crate::flash::PLANE1_BASE;
use crate::part::Part;
use crate::rd32;

pub const MAGIC: u32 = 0x5746_474F;
pub const FORMAT: u32 = 3;
pub const KIND_APP: u32 = 0;
pub const KIND_BITSTREAM: u32 = 1;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reject {
    BadHeader,
    BadTarget,
    BadDest,
    BadLength,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Header {
    pub kind: u32,
    pub target_cidr: u32,
    pub dest: u32,
    pub len: u32,
    pub crc: u32,
    pub image_version: u32,
    /// Size of the resident region the image was built for (w9).
    pub layout: u32,
}

/// Parse the header block. Intrinsic checks only (magic, format, header CRC,
/// kind, layout); whether it suits this board is [`check_header`].
pub fn parse_header(b: &[u8; 512]) -> Result<Header, Reject> {
    if rd32(b, 0) != MAGIC || rd32(b, 4) != FORMAT {
        return Err(Reject::BadHeader);
    }
    let c = crc::update(crc::update(crc::INIT, &b[..32]), &b[36..40]);
    if crc::finish(c) != rd32(b, 32) {
        return Err(Reject::BadHeader);
    }
    let kind = rd32(b, 8);
    if kind != KIND_APP && kind != KIND_BITSTREAM {
        return Err(Reject::BadHeader);
    }
    Ok(Header {
        kind,
        target_cidr: rd32(b, 12),
        dest: rd32(b, 16),
        len: rd32(b, 20),
        crc: rd32(b, 24),
        image_version: rd32(b, 28),
        layout: rd32(b, 36),
    })
}

/// Is this image for this part and does it go where such an image may go?
/// kind 0 only to `APP_BASE` exactly (anything before it, whatever the
/// length, is `BadDest`); kind 1 only to plane 1 of a part that has one.
pub fn check_header(h: &Header, part: Part) -> Result<(), Reject> {
    if Part::from_cidr(h.target_cidr) != Some(part) {
        return Err(Reject::BadTarget);
    }
    if h.layout != LAYOUT {
        return Err(Reject::BadDest);
    }
    let (ok_dest, max) = if h.kind == KIND_APP {
        (h.dest == APP_BASE, APP_MAX_LEN)
    } else {
        (h.dest == PLANE1_BASE && part.has_plane1(), PLANE_LEN)
    };
    if !ok_dest {
        return Err(Reject::BadDest);
    }
    if h.len == 0 || h.len % 512 != 0 || h.len > max {
        return Err(Reject::BadLength);
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ImageError {
    /// File ended inside the payload, or more/less chunks than `len`.
    Truncated,
    /// Descriptor missing/invalid, or its length differs from the header's.
    BadDescriptor,
    /// Descriptor length 0 (not sealed).
    Unsealed,
    BadVector,
    /// CRC of the payload differs from the header's.
    CrcMismatch,
    /// The image's trailing CRC does not match (kind 0 only).
    BadTrailer,
}

/// Streaming check of one image's payload, 512 bytes at a time:
/// descriptor and vectors on the first chunk, CRCs over everything.
pub struct ImageStream {
    h: Header,
    part: Part,
    crc: u32,
    fed: u32,
    /// First descriptor or vector fault, reported after the payload CRC.
    early: Option<ImageError>,
}

impl ImageStream {
    pub fn new(h: &Header, part: Part) -> Self {
        Self { h: *h, part, crc: crc::INIT, fed: 0, early: None }
    }

    pub fn feed(&mut self, chunk: &[u8; 512]) -> Result<(), ImageError> {
        if self.fed >= self.h.len {
            return Err(ImageError::Truncated);
        }
        if self.fed == 0 && self.h.kind == KIND_APP {
            let mut d = [0u8; DESC_LEN];
            d.copy_from_slice(&chunk[DESC_OFF as usize..DESC_OFF as usize + DESC_LEN]);
            // The order of FORMAT.md is vectors, then descriptor.
            if check_vectors(&vectors_of(chunk), self.h.dest, self.h.len, self.part).is_err() {
                self.early = Some(ImageError::BadVector);
            } else if let Err(e) = check_descriptor(&d, self.h.len) {
                self.early = Some(if e == AppError::Unsealed { ImageError::Unsealed } else { ImageError::BadDescriptor });
            }
        }
        self.crc = crc::update(self.crc, chunk);
        self.fed += 512;
        Ok(())
    }

    /// Verdict in the order of FORMAT.md: payload CRC, vectors, descriptor,
    /// trailer.
    pub fn finish(self) -> Result<(), ImageError> {
        if self.fed != self.h.len {
            return Err(ImageError::Truncated);
        }
        let c = crc::finish(self.crc);
        if c != self.h.crc {
            return Err(ImageError::CrcMismatch);
        }
        if let Some(e) = self.early {
            return Err(e);
        }
        if self.h.kind == KIND_APP && c != crc::RESIDUE {
            return Err(ImageError::BadTrailer);
        }
        Ok(())
    }
}
