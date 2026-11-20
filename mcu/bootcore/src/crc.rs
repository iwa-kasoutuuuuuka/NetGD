//! CRC-32 as in zlib/PNG (reflected, polynomial 0xEDB88320, init and final
//! xor 0xFFFFFFFF), computed with a 16-entry (64 byte) nibble table instead
//! of the 1 KB byte table. Bit-identical to `fwupdater_logic::crc32` and to
//! `tools/fwimage/vectors/crc32.txt` (tested).

const fn make_table() -> [u32; 16] {
    let mut t = [0u32; 16];
    let mut i = 0;
    while i < 16 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 4 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

static TAB: [u32; 16] = make_table();

/// Initial register value.
pub const INIT: u32 = 0xFFFF_FFFF;

/// The CRC of any `data || crc32(data) as little-endian u32`. An image whose
/// last four bytes are the CRC of the rest therefore checks with one pass
/// over the whole image: `finish(update(INIT, image)) == RESIDUE`.
pub const RESIDUE: u32 = 0x2144_DF1C;

/// Feed bytes into the raw register.
#[inline]
pub fn update(mut crc: u32, data: &[u8]) -> u32 {
    for &b in data {
        crc ^= b as u32;
        crc = (crc >> 4) ^ TAB[(crc & 15) as usize];
        crc = (crc >> 4) ^ TAB[(crc & 15) as usize];
    }
    crc
}

#[inline]
pub fn finish(crc: u32) -> u32 {
    !crc
}

pub fn crc32(data: &[u8]) -> u32 {
    finish(update(INIT, data))
}
