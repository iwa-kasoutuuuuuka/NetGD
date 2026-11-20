//! ATA disk-side helpers: the pure parts of the secondary (SD card) disk.
//!
//! `atapi` builds the GD-ROM's replies; this holds what the plain ATA disk
//! needs that can be decided from arguments alone -- the IDENTIFY DEVICE
//! words that advertise DMA, and how one host command's sector count is cut
//! into the runs a single SD CMD25 can carry.

/// IDENTIFY word 49, bit 9: LBA supported.
pub const WORD49_LBA: u16 = 0x0200;
/// IDENTIFY word 49, bit 8: DMA supported.
pub const WORD49_DMA: u16 = 0x0100;
/// IDENTIFY word 63: multiword DMA modes 0-2 supported (bits 2:0), mode 2
/// selected (bit 10). KOS's `g1ata` only uses DMA when bit 2 is set.
pub const WORD63_MWDMA2: u16 = 0x0407;
/// IDENTIFY words 65 and 66: minimum / manufacturer-recommended multiword DMA
/// cycle time in nanoseconds. 120 ns is MWDMA mode 2's cycle time.
pub const MWDMA_CYCLE_NS: u16 = 0x0078;

/// Advertise DMA in an IDENTIFY DEVICE buffer: word 49 gains the DMA bit
/// (other bits are kept), word 63 becomes [`WORD63_MWDMA2`] and words 65/66
/// (cycle times) become [`MWDMA_CYCLE_NS`]; a host that reads them as 0 may
/// take them for "no timing given" and refuse DMA.
pub fn advertise_mwdma(words: &mut [u16; 256]) {
    words[49] |= WORD49_DMA;
    words[63] = WORD63_MWDMA2;
    words[65] = MWDMA_CYCLE_NS;
    words[66] = MWDMA_CYCLE_NS;
}

/// The consecutive `(first_lba, sector_count)` runs that cover `count`
/// sectors from `lba`, each at most `max_run` long (a `max_run` of 0 is
/// treated as 1). `count` is the real sector count: LBA48's "0 means 65536"
/// is the caller's mapping. No allocation.
pub fn runs(lba: u32, count: u32, max_run: u32) -> Runs {
    Runs { lba, left: count, max_run: max_run.max(1) }
}

/// Iterator returned by [`runs`].
#[derive(Clone, Debug)]
pub struct Runs {
    lba: u32,
    left: u32,
    max_run: u32,
}

impl Iterator for Runs {
    type Item = (u32, u32);

    fn next(&mut self) -> Option<(u32, u32)> {
        if self.left == 0 {
            return None;
        }
        let n = self.left.min(self.max_run);
        let first = self.lba;
        self.lba = self.lba.wrapping_add(n);
        self.left -= n;
        Some((first, n))
    }
}
