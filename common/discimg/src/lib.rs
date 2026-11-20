//! Disc-image parsing and ATAPI reply building, as pure functions.
//!
//! Everything here takes explicit inputs and returns bytes or track
//! descriptions: no statics, no filesystem, no hardware. The firmware
//! (`mcu/netgd`) keeps the file I/O and the global state and calls in;
//! the host tests pin the outputs byte for byte.
//!
//! Tracks are not copied into this crate's own type on the firmware side
//! (a 99-entry table is about 4 KB and the part has roughly 2 KB of stack):
//! the parsers push into a [`TrackSink`] and the reply builders are generic
//! over [`TrackInfo`], so the firmware's own track pool is used in place.
#![cfg_attr(not(test), no_std)]

pub mod ata;
pub mod atapi;
pub mod cdi;
pub mod gdemu;
pub mod gdi;
pub mod slot;
pub mod wsession;

pub use heapless::{String, Vec};

/// Maximum tracks a disc may carry (Red Book's real limit).
pub const MAX_TRACKS: usize = 99;

/// One track as parsed from a descriptor.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct TrackDesc {
    pub number: u8,
    pub start_lba: u32,
    pub ctrl: u8,
    pub sector_size: u16,
    /// Track length in sectors. CDI carries it; a GDI does not (0 here, the
    /// firmware fills it in from the file size).
    pub sector_count: u32,
    /// GDI: the file name with any `"` removed. CDI: empty (the whole image
    /// is one file, which the caller already knows).
    pub filename: String<32>,
    /// Byte offset of the track's first sector within its file.
    pub file_offset: u32,
}

/// What the reply builders need to know about a track.
pub trait TrackInfo {
    fn number(&self) -> u8;
    fn start_lba(&self) -> u32;
    fn ctrl(&self) -> u8;
    fn sector_count(&self) -> u32;
}

impl TrackInfo for TrackDesc {
    fn number(&self) -> u8 {
        self.number
    }
    fn start_lba(&self) -> u32 {
        self.start_lba
    }
    fn ctrl(&self) -> u8 {
        self.ctrl
    }
    fn sector_count(&self) -> u32 {
        self.sector_count
    }
}

/// Where parsed tracks go. The firmware implements this over its static
/// pool; [`VecSink`] is the host-side collector.
pub trait TrackSink {
    /// The sink's own failure (pool full, unusable file name, ...). Passed
    /// back to the caller untouched as [`Error::Sink`].
    type Error;
    /// Called once the parser has committed to replacing the table.
    fn clear(&mut self);
    fn push(&mut self, t: &TrackDesc) -> Result<(), Self::Error>;
    /// CDI only: the descriptor length and session count, read before any
    /// track. Lets the caller log them.
    fn cdi_header(&mut self, _desc_len: u32, _sessions: u8) {}
}

/// The track table is full.
#[derive(Debug, PartialEq, Eq)]
pub struct Full;

/// Collects tracks into a `heapless::Vec`.
pub struct VecSink(pub Vec<TrackDesc, MAX_TRACKS>);

impl VecSink {
    pub fn new() -> Self {
        VecSink(Vec::new())
    }
}
impl Default for VecSink {
    fn default() -> Self {
        Self::new()
    }
}
impl TrackSink for VecSink {
    type Error = Full;
    fn clear(&mut self) {
        self.0.clear()
    }
    fn push(&mut self, t: &TrackDesc) -> Result<(), Full> {
        self.0.push(t.clone()).map_err(|_| Full)
    }
}

/// Parse failure. `E` is the sink's own error type.
#[derive(Debug, PartialEq, Eq)]
pub enum Error<E> {
    /// Malformed GDI text.
    Parse,
    /// Malformed or unsupported CDI descriptor.
    BadCdi,
    /// The caller's reader failed or hit EOF.
    Read,
    /// The sink refused a track.
    Sink(E),
}
