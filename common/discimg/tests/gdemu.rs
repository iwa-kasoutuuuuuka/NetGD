use discimg::gdemu::{bcd, version_word};

#[test]
fn bcd_packs_decimal_digits() {
    assert_eq!(bcd(0), 0x00);
    assert_eq!(bcd(9), 0x09);
    assert_eq!(bcd(14), 0x14);
    assert_eq!(bcd(20), 0x20);
    assert_eq!(bcd(99), 0x99);
    assert_eq!(bcd(200), 0x99, "clamped");
}

#[test]
fn version_word_matches_openmenu_examples() {
    // gdemu_sdk.c: `00 00 09 01 00 00 14 05` = bootloader 1.9.0, firmware 5.14.0.
    assert_eq!(version_word(1, 9, 0).to_le_bytes(), [0, 0, 9, 1]);
    assert_eq!(version_word(5, 14, 0).to_le_bytes(), [0, 0, 0x14, 5]);
    assert_eq!(version_word(5, 20, 0).to_le_bytes(), [0, 0, 0x20, 5]);
    assert_eq!(version_word(0, 0, 0), 0);
    assert_eq!(version_word(5, 20, 3).to_le_bytes(), [0, 3, 0x20, 5]);
}
