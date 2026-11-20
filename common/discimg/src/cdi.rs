//! CDI (DiscJuggler) descriptor parser. See the layout notes in the
//! firmware's `cdi.rs`; the arithmetic here is a verbatim move.
//!
//! The parser pulls small pieces through a reader callback rather than
//! needing the whole descriptor in memory (it runs to several KB on a
//! many-track image and the part has very little RAM).

use crate::{Error, String, TrackDesc, TrackSink, Vec, VecSink, MAX_TRACKS, Full};

/// Bytes of a track header before the variable-length filename.
const HDR_BEFORE_NAME: u32 = 25;
/// Bytes of a track header after the filename.
const HDR_AFTER_NAME: u32 = 31;
/// A track descriptor, when it is the last of its session. Non-final tracks
/// omit the trailing 8 bytes.
const TRACK_DATA_LEN: u32 = 188;
const SESSION_HDR_LEN: u32 = 7;

/// Trailer version words (last 8 bytes: version, descriptor offset).
const CDI_V3: u32 = 0x8000_0005;
const CDI_V35: u32 = 0x8000_0006;

/// FAD of LBA 0 (the 2-second pregap of the first track).
const FAD_OFFSET: u32 = 150;

const OFF_PREGAP: usize = 2;
const OFF_SECTOR_COUNT: usize = 6;
const OFF_START_LBA: usize = 32;
const OFF_TOTAL_LENGTH: usize = 36;
const OFF_READ_MODE: usize = 56;
const OFF_CONTROL: usize = 60;

fn rd_u16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn rd_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Stored bytes per sector for a CDI `read_mode`.
pub fn sector_size(read_mode: u32) -> Option<u16> {
    match read_mode {
        0 => Some(2048),
        1 => Some(2336),
        2 => Some(2352),
        _ => None,
    }
}

/// What the descriptor says about sessions, beyond the track table itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CdiInfo {
    /// Session count.
    pub sessions: u8,
    /// Number of the first track of the last (non-empty) session: where the
    /// Dreamcast looks for IP.BIN.
    pub boot_track: u8,
}

/// Parse the descriptor of a CDI file of `file_len` bytes into `sink`,
/// returning the session count.
///
/// `read_at(offset, buf)` must fill `buf` from file offset `offset`, or
/// fail (including on EOF before `buf` is full).
pub fn parse<S, R>(file_len: u32, read_at: R, sink: &mut S) -> Result<u8, Error<S::Error>>
where
    S: TrackSink,
    R: FnMut(u32, &mut [u8]) -> Result<(), ()>,
{
    parse_info(file_len, read_at, sink).map(|i| i.sessions)
}

/// [`parse`], also returning where the last session starts.
pub fn parse_info<S, R>(file_len: u32, mut read_at: R, sink: &mut S) -> Result<CdiInfo, Error<S::Error>>
where
    S: TrackSink,
    R: FnMut(u32, &mut [u8]) -> Result<(), ()>,
{
    if file_len < 8 {
        return Err(Error::BadCdi);
    }
    let mut rd = |off: u32, buf: &mut [u8]| read_at(off, buf).map_err(|_| Error::Read);

    // Trailer: version word, then the descriptor's offset (DiscJuggler; the
    // same fields Flycast's `CDI_init` reads). v3.5 counts the offset back
    // from the end of the file, v3 forward from its start. The track layout of
    // the two is the same. v2 has a different per-track layout, which this
    // parser does not know: rejected.
    let mut tail = [0u8; 8];
    rd(file_len - 8, &mut tail)?;
    let version = rd_u32(&tail, 0);
    let off = rd_u32(&tail, 4);
    let desc_start = match version {
        CDI_V35 => {
            if off < 16 || off > file_len {
                return Err(Error::BadCdi);
            }
            file_len - off
        }
        CDI_V3 => {
            // The descriptor holds at least 16 bytes and ends at the trailer.
            if off == 0 || off > file_len || file_len - off < 16 {
                return Err(Error::BadCdi);
            }
            off
        }
        _ => return Err(Error::BadCdi),
    };
    let desc_len = file_len - desc_start;

    let mut one = [0u8; 1];
    rd(desc_start, &mut one)?;
    let session_count = one[0];
    sink.cdi_header(desc_len, session_count);

    sink.clear();
    // Running base of the next track's stored data within the file.
    let mut base: u32 = 0;
    let mut cursor = desc_start.wrapping_add(1);
    let mut number: u8 = 1;
    let mut count = 0usize;
    let mut boot_track: u8 = 1;
    let mut session_no: u8 = 0;

    for _ in 0..session_count {
        let mut sh = [0u8; SESSION_HDR_LEN as usize];
        rd(cursor, &mut sh)?;
        let track_count = rd_u16(&sh, 1);
        cursor = cursor.wrapping_add(SESSION_HDR_LEN);
        if track_count > 0 {
            session_no = session_no.wrapping_add(1);
        }

        for ti in 0..track_count {
            if ti == 0 {
                boot_track = number;
            }
            // Header: read up to the length byte, then skip the name.
            let mut head = [0u8; HDR_BEFORE_NAME as usize];
            rd(cursor, &mut head)?;
            let name_len = head[HDR_BEFORE_NAME as usize - 1] as u32;
            let td_at = cursor
                .wrapping_add(HDR_BEFORE_NAME)
                .wrapping_add(name_len)
                .wrapping_add(HDR_AFTER_NAME);

            let mut td = [0u8; TRACK_DATA_LEN as usize];
            rd(td_at, &mut td)?;

            let pregap = rd_u32(&td, OFF_PREGAP);
            let sector_count = rd_u32(&td, OFF_SECTOR_COUNT);
            let start_lba = rd_u32(&td, OFF_START_LBA);
            let total_length = rd_u32(&td, OFF_TOTAL_LENGTH);
            let read_mode = rd_u32(&td, OFF_READ_MODE);
            let control = rd_u32(&td, OFF_CONTROL);

            let ssize = sector_size(read_mode).ok_or(Error::BadCdi)?;

            // Data begins past this track's own pregap.
            let mut file_offset = base
                .checked_add(pregap.checked_mul(ssize as u32).ok_or(Error::BadCdi)?)
                .ok_or(Error::BadCdi)?;

            // The descriptor's `start_lba` is where the track's *pregap*
            // begins, counted from the first pregap sector of the disc
            // (mkdcdisc: running sum of earlier tracks' pregap+data+postgap,
            // plus whole earlier sessions; Flycast: track FAD = start_lba +
            // pregap_length). FAD = LBA + 150, so the LBA of the first data
            // sector is start_lba + pregap - 150. With the usual 150-sector
            // pregap that is `start_lba` itself; a track with a shorter
            // pregap (DiscJuggler's own images) starts that much earlier.
            //
            // A track whose data would begin before FAD 150 (LBA < 0) cannot
            // be addressed from LBA 0. Pinning its LBA to 0 and keeping the
            // file offset put every sector `shortfall` sectors off, so the
            // sectors below LBA 0 are skipped instead: the file offset
            // advances and the count shrinks by the same amount. A track
            // that lies wholly below LBA 0 has nothing to map: BadCdi.
            let first_fad = start_lba.saturating_add(pregap);
            let mut sector_count = sector_count;
            let data_lba = if first_fad >= FAD_OFFSET {
                first_fad - FAD_OFFSET
            } else {
                let short = FAD_OFFSET - first_fad;
                if short >= sector_count {
                    return Err(Error::BadCdi);
                }
                file_offset = file_offset
                    .checked_add(short.checked_mul(ssize as u32).ok_or(Error::BadCdi)?)
                    .ok_or(Error::BadCdi)?;
                sector_count -= short;
                0
            };

            let t = TrackDesc {
                number,
                start_lba: data_lba,
                ctrl: (control & 0x0F) as u8,
                sector_size: ssize,
                sector_count,
                filename: String::new(),
                file_offset,
                session: session_no,
            };
            sink.push(&t).map_err(Error::Sink)?;
            count += 1;

            base = base
                .checked_add(total_length.checked_mul(ssize as u32).ok_or(Error::BadCdi)?)
                .ok_or(Error::BadCdi)?;
            number = number.wrapping_add(1);

            // The last track of a session carries 8 extra bytes.
            let is_last = ti + 1 == track_count;
            cursor = td_at.wrapping_add(if is_last { TRACK_DATA_LEN } else { TRACK_DATA_LEN - 8 });
        }
    }

    if count == 0 {
        return Err(Error::BadCdi);
    }
    Ok(CdiInfo { sessions: session_count, boot_track })
}

/// Convenience for host use: parse a CDI held entirely in memory.
pub fn parse_slice(file: &[u8]) -> Result<(u8, Vec<TrackDesc, MAX_TRACKS>), Error<Full>> {
    let mut s = VecSink::new();
    let n = parse(
        file.len() as u32,
        |off, buf| {
            let off = off as usize;
            let src = file.get(off..off.checked_add(buf.len()).ok_or(())?).ok_or(())?;
            buf.copy_from_slice(src);
            Ok(())
        },
        &mut s,
    )?;
    Ok((n, s.0))
}
