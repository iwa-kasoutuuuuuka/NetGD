//! Pure pieces of the original GDEMU's command protocol.

/// A value 0..=99 as packed BCD (`20` -> `0x20`); anything above 99 clamps
/// to `0x99`.
pub fn bcd(v: u8) -> u8 {
    let v = v.min(99);
    ((v / 10) << 4) | (v % 10)
}

/// A version as one word, `major << 24 | minor << 16 | patch << 8`, each
/// part BCD. openMenu's `gdemu_sdk.c` documents `00 00 09 01` (little-endian)
/// as bootloader 1.9.0 and `00 00 14 05` as firmware 5.14.0, so the digits
/// are the decimal digits: 5.20.0 is `00 00 20 05`.
pub fn version_word(major: u8, minor: u8, patch: u8) -> u32 {
    (bcd(major) as u32) << 24 | (bcd(minor) as u32) << 16 | (bcd(patch) as u32) << 8
}
