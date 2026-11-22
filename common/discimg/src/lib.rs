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

/// Longest GDI track file name accepted, in bytes (a byte >= 0x80 costs two,
/// see `gdi::strip_quotes`). VFAT allows 255 UTF-16 units; the firmware
/// holds the name on its stack several times over while it looks the file up
/// by its long name (the descriptor, the widened copy, the UTF-16 target of
/// the matcher: about 5 bytes per name byte), and 128 covers every Redump
/// name (`... (Track 01).bin` is under 70) at about 650 bytes. A longer name
/// is [`Error::NameTooLong`], not a silent truncation.
pub const MAX_NAME: usize = 128;

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
    pub filename: String<MAX_NAME>,
    /// Byte offset of the track's first sector within its file.
    pub file_offset: u32,
    /// CDI: 1-based number of the (non-empty) session the track belongs to,
    /// in descriptor order. GDI: 0 (not tracked; REQ_SES uses the fixed
    /// GD-ROM layout there).
    pub session: u8,
    /// Sector mode of the track: [`TRACK_AUDIO`], [`TRACK_MODE1`] or
    /// [`TRACK_MODE2`] (see [`track_mode`]).
    pub mode: u8,
}

/// Track sector modes ([`TrackDesc::mode`], [`TrackInfo::mode`]). Stock GDEMU
/// 6.0 decides CD_READ's sector-type check and the reported disc format by
/// the track's mode, not by the image format (R144: a CDI whose data track is
/// Mode 1 takes `0x24`, refuses `0x28` and reports CD-ROM).
pub const TRACK_AUDIO: u8 = 0;
pub const TRACK_MODE1: u8 = 1;
pub const TRACK_MODE2: u8 = 2;

/// Mode of a track from its control nibble (`ctrl`, bit 2 = data), the mode
/// its descriptor states (`desc_mode`: a CDI's DiscJuggler track mode, 0
/// audio / 1 Mode 1 / 2 Mode 2; `None` for a GDI, which has no such field) and
/// its stored sector size.
///
/// A track the descriptor calls Mode 1 or Mode 2 is that, whatever its control
/// nibble says (Flycast's CDI reader decides audio by the descriptor mode
/// alone; a data track mislabelled audio in `ctrl` used to read, and still
/// does). Otherwise the control nibble decides: audio, or data whose mode
/// defaults -- a 2,336-byte track is Mode 2 (that size only exists for Mode
/// 2), a GDI's data is Mode 1 (every GD-ROM's is), a CDI's Mode 2 (what
/// mkdcdisc writes and what NetGD assumed before R144). The control nibble
/// itself is left as it is for the TOC and Q channel.
pub fn track_mode(ctrl: u8, desc_mode: Option<u32>, sector_size: u16) -> u8 {
    match desc_mode {
        Some(1) => TRACK_MODE1,
        Some(2) => TRACK_MODE2,
        _ if ctrl & 0x04 == 0 => TRACK_AUDIO,
        _ if sector_size == 2336 => TRACK_MODE2,
        None => TRACK_MODE1,
        Some(_) => TRACK_MODE2,
    }
}

/// Pack a track's control nibble (low 4 bits of `ctrl`) and its mode (a
/// `TRACK_*` value) into one byte: ctrl in bits 0..3, mode in bits 4..5. The
/// firmware keeps its 99-entry track table at 32 bytes a track this way.
pub const fn pack_ctrl_mode(ctrl: u8, mode: u8) -> u8 {
    (ctrl & 0x0F) | ((mode & 0x03) << 4)
}

/// Control nibble of a [`pack_ctrl_mode`] byte.
pub const fn packed_ctrl(packed: u8) -> u8 {
    packed & 0x0F
}

/// Mode (`TRACK_*`) of a [`pack_ctrl_mode`] byte.
pub const fn packed_mode(packed: u8) -> u8 {
    (packed >> 4) & 0x03
}

/// What the reply builders need to know about a track.
pub trait TrackInfo {
    fn number(&self) -> u8;
    fn start_lba(&self) -> u32;
    fn ctrl(&self) -> u8;
    fn sector_count(&self) -> u32;
    /// CDI session number of the track (1-based), or 0 if the implementor
    /// does not track it (the default). With 0 everywhere,
    /// [`atapi::req_ses_reply`] falls back to "session 2 = first track of the
    /// last session" (`boot_area_lba`).
    fn session(&self) -> u8 {
        0
    }
    /// Sector mode ([`TRACK_AUDIO`], [`TRACK_MODE1`], [`TRACK_MODE2`]).
    fn mode(&self) -> u8;
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
    fn session(&self) -> u8 {
        self.session
    }
    fn mode(&self) -> u8 {
        self.mode
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
    /// A GDI track file name is longer than [`MAX_NAME`] bytes.
    NameTooLong,
    /// The sink refused a track.
    Sink(E),
}
