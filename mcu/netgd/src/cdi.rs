//! CDI (DiscJuggler) parser.
//!
//! CDI stores a disc back to front: raw track data starts at file offset 0
//! with no header at all, a descriptor is appended at the end, and the final
//! four bytes give the descriptor's length. Everything is little-endian and
//! packed.
//!
//! Descriptor layout, confirmed against a real image (dcload-ser 1.0.6) and
//! against the writer in mkdcdisc (`src/disc_image/formats/cdi.c`):
//!
//! ```text
//!   u8   session_count
//!   per session:
//!     u8   padding
//!     u16  track_count
//!     u32  unknown
//!     per track:
//!       track header   56 + filename_length bytes
//!       track data     180 bytes, or 188 for the last track of a session
//!   terminator: one empty session header, one track header, disc info
//!   u32  descriptor length, counted back from EOF
//! ```
//!
//! The filename is *variable length* — mkdcdisc always writes 32 bytes, but
//! DiscJuggler writes the full source path (56 bytes in the image above), so
//! a fixed 32 cannot be assumed; getting this wrong silently misaligns every
//! field after it.
//!
//! Track data is laid out in the file back to back in descriptor order, each
//! track occupying `total_length` sectors (pregap included) of
//! `sector_size(read_mode)` bytes. A track's *data* therefore begins at the
//! running base plus `pregap * sector_size`. For the reference image that
//! arithmetic accounts for the file exactly: 17025*2352 + 16725*2352 +
//! 312653*2336 = 809,737,408, and the file is 809,738,626 — the 1218-byte
//! descriptor.

use embedded_sdmmc::{Directory, ShortFileName};

use crate::blockdev::HsmciBlockDevice;
use crate::gdi::{GdiError, PoolSink};

type Dir<'a> = Directory<'a, HsmciBlockDevice, crate::blockdev::DummyClock, 4, 4, 1>;

/// Find the first `*.CDI` in `dir`.
pub fn find_cdi(dir: &Dir<'_>) -> Result<ShortFileName, GdiError> {
    let mut found: Option<ShortFileName> = None;
    let _ = dir.iterate_dir(|entry| {
        if found.is_none()
            && !entry.attributes.is_directory()
            && entry.name.extension().eq_ignore_ascii_case(b"CDI")
        {
            found = Some(entry.name.clone());
        }
    });
    found.ok_or(GdiError::NoGdiFile)
}

/// Parse the descriptor of `name` in `dir` into a track map.
///
/// Walks the descriptor with small seeks rather than buffering it whole: a
/// many-track image runs to several KB and this part has 52 KB of SRAM to
/// live in, most of it already spoken for by the CD_READ staging buffer.
/// Fills the shared track pool rather than returning the tracks: 99 of them
/// is about 4 KB, and this part has roughly 2 KB of stack. See
/// [`crate::gdi::MAX_TRACKS`].
/// Returns the number of sessions the descriptor declares. A MIL-CD has two
/// (audio, then the bootable data track), and the host is told that count via
/// REQ_SES(0) -- it is how the BIOS learns session 2 exists at all.
pub fn parse(dir: &Dir<'_>, name: &ShortFileName) -> Result<discimg::cdi::CdiInfo, GdiError> {
    let f = dir
        .open_file_in_dir(name, embedded_sdmmc::Mode::ReadOnly)
        .map_err(|_| GdiError::OpenGdiFile)?;
    let file_len = f.length();
    if file_len < 8 {
        return Err(GdiError::BadCdi);
    }

    // Any seek or read failure, or EOF short of the requested length, is a
    // read error.
    let read_at = |off: u32, buf: &mut [u8]| -> Result<(), ()> {
        f.seek_from_start(off).map_err(|_| ())?;
        let mut done = 0usize;
        while done < buf.len() {
            let n = f.read(&mut buf[done..]).map_err(|_| ())?;
            if n == 0 {
                return Err(());
            }
            done += n;
        }
        Ok(())
    };

    // Cleared by the parser once it has the descriptor header, so a parse
    // that fails part way cannot leave the previous image's tail behind.
    discimg::cdi::parse_info(file_len, read_at, &mut PoolSink::cdi(name.clone())).map_err(|e| match e {
        discimg::Error::Sink(e) => e,
        discimg::Error::Read => GdiError::ReadGdiFile,
        _ => GdiError::BadCdi,
    })
}
