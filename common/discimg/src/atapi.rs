//! Sega GD-ROM ATAPI reply builders: TOC, REQ_SES, REQ_STAT, GET_SCD.
//!
//! Pure functions of a track table plus the pickup state. Moved verbatim from
//! the firmware's `ide.rs` / `gdi.rs`; additions that could overflow use
//! wrapping arithmetic because the firmware is built without overflow checks
//! and the replies must stay byte-identical.

use crate::TrackInfo;

/// CDDA player state as the replies see it (mirrors the firmware's
/// `cdda::Status`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioState {
    // Same variant order as the firmware's enum, so mapping one to the other
    // compiles to nothing.
    Standby,
    Playing,
    Paused,
    Finished,
    Error,
}

/// Lead-out LBA of a GDI: a pressed GD-ROM's high-density area always runs
/// out to this.
pub const GDROM_LEADOUT_LBA: u32 = 549150;

/// Track containing `lba`: the last track starting at or before it.
pub fn track_for_lba<T: TrackInfo>(tracks: &[T], lba: u32) -> Option<&T> {
    tracks.iter().rfind(|t| t.start_lba() <= lba)
}

/// `boot_area_lba` of a GDI: the first track numbered 3 or above (the High
/// Density Area), else the conventional 45000 (a two-track GDI has no HDA).
pub fn gdi_boot_area_lba<T: TrackInfo>(tracks: &[T]) -> u32 {
    tracks.iter().find(|t| t.number() >= 3).map(|t| t.start_lba()).unwrap_or(45000)
}

/// `boot_area_lba` of a CDI: the start of `boot_track`, the first track of
/// the LAST session (what the BIOS boots; Flycast's `GetSessionInfo` reports
/// the same per-session first track). Falls back to the first track's start.
pub fn cdi_boot_area_lba<T: TrackInfo>(tracks: &[T], boot_track: u8) -> u32 {
    tracks
        .iter()
        .find(|t| t.number() == boot_track)
        .or_else(|| tracks.first())
        .map(|t| t.start_lba())
        .unwrap_or(0)
}

/// Tracks to look for IP.BIN in, most likely first, given the image's
/// `boot_area_lba` (from [`gdi_boot_area_lba`] / [`cdi_boot_area_lba`], the
/// same value the TOC, REQ_SES and the boot prefetch use).
///
/// The first candidate is what the BIOS boots: the first data track at or
/// above the boot area -- track 3 of a GDI, the first track of a CDI's last
/// session. The rest are only for a caller that wants *a* header to show
/// (a title for a menu) from an image whose boot track is unreadable or
/// unsigned: the remaining data tracks above the boot area, then the data
/// tracks below it, each in table order. Audio tracks are never candidates.
///
/// This replaces a fixed "starts at LBA 11700 or later" test, which guessed
/// the session of a track from its address: wrong for a CDI whose second
/// session starts lower, and for one with three sessions, where it preferred
/// the middle session to the last.
pub fn ip_bin_candidates<T: TrackInfo>(
    tracks: &[T],
    boot_area_lba: u32,
) -> impl Iterator<Item = &T> {
    let data = |t: &&T| t.ctrl() & 0x04 != 0;
    tracks
        .iter()
        .filter(data)
        .filter(move |t| t.start_lba() >= boot_area_lba)
        .chain(tracks.iter().filter(data).filter(move |t| t.start_lba() < boot_area_lba))
}

/// Total disc length in LBAs, used as the lead-out address. A CDI knows its
/// real length (last track's start + sector count); a GDI reports the
/// pressed-disc capacity.
pub fn leadout_lba<T: TrackInfo>(tracks: &[T], is_cdi: bool) -> u32 {
    if is_cdi {
        if let Some(last) = tracks.last() {
            return last.start_lba().wrapping_add(last.sector_count());
        }
    }
    GDROM_LEADOUT_LBA
}

/// Build a 408-byte Sega GD-ROM TOC response. `area` 0 is the single
/// density area, 1 the high density area; on a CDI every track is in area 0.
///
/// Layout: 99 4-byte entries (tracks 1..99), then first-track, last-track and
/// lead-out entries, 4 bytes each. Each entry is `[CTRL_ADR, FAD_hi, FAD_mid,
/// FAD_lo]` with `CTRL_ADR = (ctl << 4) | 1`. Unused slots are 0xFF.
pub fn build_toc<T: TrackInfo>(
    tracks: &[T],
    is_cdi: bool,
    boot_area_lba: u32,
    area: u8,
    out: &mut [u8; 408],
) {
    out.fill(0xFF);

    let in_area = |t: &T| -> bool {
        if is_cdi {
            area == 0
        } else if area == 0 {
            t.start_lba() < boot_area_lba
        } else {
            t.start_lba() >= boot_area_lba
        }
    };
    let ctrl_adr = |ctl: u8| -> u8 { (ctl << 4) | 0x01 };

    let mut first: Option<u8> = None;
    let mut last: Option<u8> = None;
    let mut last_track_ref: Option<&T> = None;

    for t in tracks.iter() {
        if !in_area(t) {
            continue;
        }
        if t.number() == 0 || t.number() > 99 {
            continue;
        }
        let idx = (t.number() - 1) as usize * 4;
        let fad = t.start_lba().wrapping_add(150);
        out[idx] = ctrl_adr(t.ctrl());
        out[idx + 1] = (fad >> 16) as u8;
        out[idx + 2] = (fad >> 8) as u8;
        out[idx + 3] = fad as u8;

        first.get_or_insert(t.number());
        last = Some(t.number());
        last_track_ref = Some(t);
    }

    if let (Some(f), Some(l), Some(lt)) = (first, last, last_track_ref) {
        // First track entry (offset 99*4 = 396).
        let first_ctrl = tracks
            .iter()
            .find(|t| t.number() == f)
            .map(|t| ctrl_adr(t.ctrl()))
            .unwrap_or(0x01);
        out[396] = first_ctrl;
        out[397] = f;
        out[398] = 0;
        out[399] = 0;
        // Last track entry (offset 100*4 = 400).
        out[400] = ctrl_adr(lt.ctrl());
        out[401] = l;
        out[402] = 0;
        out[403] = 0;
        // Lead-out entry (offset 101*4 = 404). The HD area of a GDI is
        // padded out to the pressed-disc capacity; a CDI is exact.
        let mut leadout_fad = lt.start_lba().wrapping_add(lt.sector_count()).wrapping_add(150);
        if area == 1 && !is_cdi {
            leadout_fad = leadout_fad.max(leadout_lba(tracks, is_cdi).wrapping_add(150));
        }
        out[404] = ctrl_adr(lt.ctrl());
        out[405] = (leadout_fad >> 16) as u8;
        out[406] = (leadout_fad >> 8) as u8;
        out[407] = leadout_fad as u8;
    }
}

/// Where the pickup "is" for REQ_STAT / GET_SCD: `(fad, track)` -- the
/// sector CDDA is playing or paused at (`cdda_fad`), or FAD 150 when idle.
#[inline(never)] // shared by REQ_STAT and GET_SCD; inlined twice it costs ~100 bytes
pub fn head_position<T: TrackInfo>(
    tracks: &[T],
    state: AudioState,
    cdda_fad: u32,
) -> (u32, Option<&T>) {
    let fad = match state {
        AudioState::Standby => 150,
        _ => cdda_fad,
    };
    let track = track_for_lba(tracks, fad.saturating_sub(150));
    (fad, track)
}

/// 10-byte Sega REQ_STAT reply: drive status, repeat count, CTRL/ADR, TNO,
/// index, FAD MSB..LSB, max retry, 0. `fad`/`track` come from
/// [`head_position`].
pub fn req_stat_reply<T: TrackInfo>(
    state: AudioState,
    repeat: u8,
    fad: u32,
    track: Option<&T>,
) -> [u8; 10] {
    let status: u8 = match state {
        AudioState::Playing => 3,
        AudioState::Paused | AudioState::Finished => 1,
        AudioState::Error => 9,
        AudioState::Standby => 2,
    };
    let (ctrl_adr, tno) = track
        .map(|t| ((t.ctrl() << 4) | 1, t.number()))
        .unwrap_or((0x41, 1));
    [
        status,
        repeat,
        ctrl_adr,
        tno,
        0x01,
        (fad >> 16) as u8,
        (fad >> 8) as u8,
        fad as u8,
        0x00,
        0x00,
    ]
}

pub fn bcd(n: u32) -> u8 {
    (((n / 10) << 4) | (n % 10)) as u8
}

/// Minute/second/frame of a block address, BCD, into `out[0..3]`.
pub fn put_msf(out: &mut [u8], blk: u32) {
    out[0] = bcd(blk / 4500);
    out[1] = bcd((blk % 4500) / 75);
    out[2] = bcd(blk % 75);
}

/// 6-byte REQ_SES reply for one session (see the firmware's `ide.rs` for
/// the history of why each subcommand reports what it does).
///   byte 0 = 0x01 (ready), byte 1 = 0, byte 2 = track, bytes 3..6 = FAD BE
///   session 0: session count + lead-out FAD
///   session 1: GDI: first track below `boot_area_lba`; CDI: the first track
///   session n >= 2 (CDI): the first track of session n when the tracks know
///     their session ([`TrackInfo::session`] != 0, so any number of sessions);
///     otherwise (firmware track type without it) only session 2 and only
///     when `sessions >= 2`: first track at/above `boot_area_lba` (= the
///     first track of the last session, see [`cdi_boot_area_lba`]), which is
///     right for boot but not for the middle session of a 3+ session CDI
///   anything else, or no such track: `None` (caller aborts)
pub fn req_ses_reply<T: TrackInfo>(
    tracks: &[T],
    is_cdi: bool,
    boot_area_lba: u32,
    sessions: u8,
    session: u8,
) -> Option<[u8; 6]> {
    let mut r = [0u8; 6];
    r[0] = 0x01;
    let (track, fad) = match session {
        0 => (sessions, leadout_lba(tracks, is_cdi).wrapping_add(150)),
        1 => {
            let t = if is_cdi {
                // The boot area of a CDI may be track 1 itself (one session).
                tracks.first()?
            } else {
                tracks.iter().find(|t| t.start_lba() < boot_area_lba)?
            };
            (t.number(), t.start_lba().wrapping_add(150))
        }
        // CDI whose tracks carry their session number: session n's own first
        // track (also 3+ sessions). Session 1 above is already the first
        // track of the disc, which is session 1's.
        n if is_cdi && n >= 2 && tracks.iter().any(|t| t.session() != 0) => {
            let t = tracks.iter().find(|t| t.session() == n)?;
            (t.number(), t.start_lba().wrapping_add(150))
        }
        2 if is_cdi && sessions >= 2 => {
            let t = tracks.iter().find(|t| t.start_lba() >= boot_area_lba)?;
            (t.number(), t.start_lba().wrapping_add(150))
        }
        _ => return None,
    };
    r[2] = track;
    r[3] = (fad >> 16) as u8;
    r[4] = (fad >> 8) as u8;
    r[5] = fad as u8;
    Some(r)
}

/// Build a CD_SCD reply. Format 1 is the plain 14-byte form (header
/// `[0, audio_status, 0, 14]` then Q[0..10]); anything else is the 100-byte
/// form with Q bit-expanded (`0x7F` / `0x3F`) and a CRC-CCITT substituted
/// into Q[10..12]. Returns the reply length. `fad`/`track` come from
/// [`head_position`].
pub fn get_scd_reply<T: TrackInfo>(
    format: u8,
    state: AudioState,
    fad: u32,
    track: Option<&T>,
    r: &mut [u8; 100],
) -> usize {
    let audio_status: u8 = match state {
        AudioState::Playing => 0x11,
        AudioState::Paused => 0x12,
        AudioState::Finished => 0x13,
        AudioState::Error => 0x14,
        AudioState::Standby => 0x15,
    };
    let mut q: [u8; 12] = [0; 12];
    match track {
        Some(t) => {
            q[0] = (t.ctrl() << 4) | 1; // CTRL, ADR=1 (Q-mode 1)
            q[1] = bcd(t.number() as u32);
            put_msf(&mut q[3..6], fad.saturating_sub(t.start_lba().wrapping_add(150)));
        }
        None => q[0] = 0x41,
    }
    q[2] = 0x01; // index 1
    put_msf(&mut q[7..10], fad);
    // q[10..12] will be overwritten with CRC mid-expansion.

    r.fill(0);
    r[1] = audio_status;
    if format == 1 {
        r[3] = 14;
        r[4..14].copy_from_slice(&q[..10]);
        return 14;
    }
    r[3] = 100; // data length

    // Expand bits MSB-first; compute CRC-CCITT across the bit stream. After
    // the first 80 bits (Q[0..10]) the inverted CRC is substituted into
    // q[10..12] before continuing.
    let mut crc: u16 = 0;
    for i in 0..12 {
        if i == 10 {
            let crc_done = !crc;
            q[10] = (crc_done >> 8) as u8;
            q[11] = (crc_done & 0xff) as u8;
        }
        let byte = q[i];
        for bit in 0..8 {
            let b = (byte >> (7 - bit)) & 1;
            let top = (crc >> 15) & 1;
            crc = (crc << 1) ^ if (top ^ b as u16) != 0 { 0x1021 } else { 0 };
            r[4 + i * 8 + bit] = if b != 0 { 0x7F } else { 0x3F };
        }
    }
    100
}
