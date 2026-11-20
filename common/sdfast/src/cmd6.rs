//! SD CMD6 SWITCH_FUNC, as used to enter High-Speed (50 MHz) mode.
//!
//! CMD6 returns a 512-bit (64-byte) status block, MSB first: byte 0 holds
//! bits 511..504. Only function group 1 (access mode) is of interest here;
//! SD Physical Layer spec, "Switch Function Status".

/// Check mode (bit 31 = 0), group 1 = High-Speed, everything else
/// "no change" (0xF). The card reports what it supports; nothing switches.
pub const ARG_CHECK_HS: u32 = 0x00FF_FFF1;
/// Switch mode (bit 31 = 1), group 1 = High-Speed.
pub const ARG_SWITCH_HS: u32 = 0x80FF_FFF1;
/// Switch mode, group 1 = Default Speed. Used to back out.
pub const ARG_SWITCH_DEFAULT: u32 = 0x80FF_FFF0;

/// Size of the CMD6 status block.
pub const STATUS_LEN: usize = 64;

/// Function group 1 support bitmask, bits 415..400: byte 12 holds 415..408
/// and byte 13 holds 407..400. Function `n` is bit `400 + n`.
fn group1_support(st: &[u8; STATUS_LEN]) -> u16 {
    ((st[12] as u16) << 8) | st[13] as u16
}

/// Does the card advertise High-Speed (group 1, function 1)?
pub fn hs_supported(st: &[u8; STATUS_LEN]) -> bool {
    group1_support(st) & (1 << 1) != 0
}

/// Function group 1 selected by the last switch, bits 379..376 (low nibble
/// of byte 16). 0xF means the card refused the function.
pub fn group1_selected(st: &[u8; STATUS_LEN]) -> u8 {
    st[16] & 0x0F
}

/// Group 1 busy status, bits 287..272: byte 28 holds 287..280 and byte 29
/// holds 279..272. Function `n` is bit `272 + n`. A set bit means the card
/// is still busy completing that function.
pub fn hs_busy(st: &[u8; STATUS_LEN]) -> bool {
    let busy = ((st[28] as u16) << 8) | st[29] as u16;
    busy & (1 << 1) != 0
}

/// True when a switch-mode CMD6 to High-Speed took effect.
pub fn switched_to_hs(st: &[u8; STATUS_LEN]) -> bool {
    group1_selected(st) == 1 && !hs_busy(st)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank() -> [u8; STATUS_LEN] {
        [0; STATUS_LEN]
    }

    #[test]
    fn arguments_match_the_spec_encoding() {
        // mode bit 31, groups 6..2 = 0xF (no change), group 1 = function.
        assert_eq!(ARG_CHECK_HS >> 31, 0);
        assert_eq!(ARG_SWITCH_HS >> 31, 1);
        assert_eq!(ARG_SWITCH_HS & 0x00FF_FFFF, 0x00FF_FFF1);
        assert_eq!(ARG_SWITCH_DEFAULT & 0xF, 0);
        assert_eq!(ARG_CHECK_HS & 0x00FF_FFF0, 0x00FF_FFF0);
    }

    #[test]
    fn support_bit_is_bit_401() {
        let mut st = blank();
        assert!(!hs_supported(&st));
        st[13] = 0x02; // bit 401
        assert!(hs_supported(&st));
        st[13] = 0x01; // default speed only
        assert!(!hs_supported(&st));
        st[13] = 0x00;
        st[12] = 0x02; // bit 409: not group 1 function 1
        assert!(!hs_supported(&st));
    }

    #[test]
    fn selected_function_is_the_low_nibble_of_byte_16() {
        let mut st = blank();
        st[16] = 0xF1; // group 2 = F, group 1 = 1
        assert_eq!(group1_selected(&st), 1);
        st[16] = 0x1F;
        assert_eq!(group1_selected(&st), 0xF);
    }

    #[test]
    fn busy_bit_is_bit_273() {
        let mut st = blank();
        assert!(!hs_busy(&st));
        st[29] = 0x02;
        assert!(hs_busy(&st));
        st[29] = 0;
        st[28] = 0x02; // bit 281: not group 1
        assert!(!hs_busy(&st));
    }

    #[test]
    fn switch_success_needs_selected_and_not_busy() {
        let mut st = blank();
        st[16] = 0x01;
        assert!(switched_to_hs(&st));
        st[29] = 0x02;
        assert!(!switched_to_hs(&st));
        st[29] = 0;
        st[16] = 0x0F; // refused
        assert!(!switched_to_hs(&st));
        st[16] = 0x00; // stayed in default speed
        assert!(!switched_to_hs(&st));
    }
}
