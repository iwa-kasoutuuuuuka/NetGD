//! Helpers for proving a faster SD link with real data
//! (firmware feature `sd-highspeed`).
//!
//! A bus that is marginal at 48 MHz corrupts bits at random, and a block of
//! zeros hides that: a data line stuck low reads back exactly the zeros that
//! were stored. The check therefore needs blocks whose bits are close to
//! random ([`block_is_busy`]), and compares
//! each block read at the new speed against a hash of what the Default Speed
//! link returned ([`block_hash`]) so the reference does not have to stay in
//! RAM.

/// Distinct byte values in a block. A block of compressed or encrypted data
/// has about 220 of 256 (512 random bytes); code and text have well under
/// 128; an erased or zero-filled block has 1 or 2.
pub fn distinct_bytes(block: &[u8]) -> u32 {
    let mut seen = [0u32; 8];
    for &b in block {
        seen[(b >> 5) as usize] |= 1 << (b & 31);
    }
    seen.iter().map(|w| w.count_ones()).sum()
}

/// Enough distinct byte values to count as high-entropy data.
pub const BUSY_MIN_DISTINCT: u32 = 160;

/// Is this 512-byte block close enough to random data to exercise every data
/// line of the bus?
pub fn block_is_busy(block: &[u8]) -> bool {
    distinct_bytes(block) >= BUSY_MIN_DISTINCT
}

/// 32-bit hash of a block, word by word (little endian). Not cryptographic;
/// it only has to notice random bit errors, which it misses with probability
/// 2^-32, and it is cheap enough (a few cycles per word) to run on thousands
/// of blocks at boot.
pub fn block_hash(block: &[u8]) -> u32 {
    let mut h = 0x811C_9DC5u32;
    let mut it = block.chunks_exact(4);
    for c in &mut it {
        let w = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
        h = (h.rotate_left(5) ^ w).wrapping_mul(0x9E37_79B1);
    }
    for &b in it.remainder() {
        h = (h.rotate_left(5) ^ b as u32).wrapping_mul(0x9E37_79B1);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(seed: u32, n: usize) -> Vec<u8> {
        let mut x = seed;
        (0..n)
            .map(|_| {
                x = x.wrapping_mul(1664525).wrapping_add(1013904223);
                (x >> 24) as u8
            })
            .collect()
    }

    #[test]
    fn zeros_and_text_are_not_busy_noise_is() {
        assert_eq!(distinct_bytes(&[0u8; 512]), 1);
        assert!(!block_is_busy(&[0u8; 512]));
        assert!(!block_is_busy(&[0xFFu8; 512]));
        let text: Vec<u8> = b"the quick brown fox jumps over the lazy dog "
            .iter()
            .cycle()
            .take(512)
            .copied()
            .collect();
        assert!(!block_is_busy(&text));
        assert!(block_is_busy(&noise(1, 512)));
    }

    #[test]
    fn distinct_counts_every_value_once() {
        let all: Vec<u8> = (0..=255u8).chain(0..=255u8).collect();
        assert_eq!(distinct_bytes(&all), 256);
        assert_eq!(distinct_bytes(&[7, 7, 7, 200]), 2);
    }

    #[test]
    fn hash_sees_every_single_bit_flip() {
        let base = noise(7, 512);
        let h0 = block_hash(&base);
        for byte in (0..512).step_by(37) {
            for bit in 0..8 {
                let mut b = base.clone();
                b[byte] ^= 1 << bit;
                assert_ne!(block_hash(&b), h0, "byte {byte} bit {bit}");
            }
        }
    }

    #[test]
    fn hash_sees_a_stuck_data_line() {
        // One of the four data lines stuck low: every nibble loses a bit.
        let base = noise(9, 512);
        let stuck: Vec<u8> = base.iter().map(|b| b & 0xEE).collect();
        assert_ne!(block_hash(&stuck), block_hash(&base));
    }

    #[test]
    fn hash_sees_word_order() {
        let mut a = noise(3, 512);
        let h = block_hash(&a);
        a.swap(0, 4);
        a.swap(1, 5);
        a.swap(2, 6);
        a.swap(3, 7);
        assert_ne!(block_hash(&a), h);
    }
}
