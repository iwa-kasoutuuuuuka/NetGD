//! Sega GD-ROM ATAPI reply builders: TOC, REQ_SES, REQ_STAT, GET_SCD.
//!
//! Pure functions of a track table plus the pickup state. Moved verbatim from
//! the firmware's `ide.rs` / `gdi.rs`; additions that could overflow use
//! wrapping arithmetic because the firmware is built without overflow checks
//! and the replies must stay byte-identical.

use crate::{TrackInfo, TRACK_AUDIO, TRACK_MODE1, TRACK_MODE2};

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

/// A pressed GD-ROM's capacity: its high-density area runs out to this LBA.
/// Only the lead-out of an (impossible) empty track table now; a GDI reports
/// the end of its last track, as stock GDEMU 6.0 does (D1, see
/// [`leadout_lba`]).
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

/// Total disc length in LBAs, used as the lead-out address: the last track's
/// start + sector count, for a GDI as for a CDI. Stock GDEMU 6.0 reports a
/// GDI that way (D1: GDI 03, last track 45000..64999, gives lead-out FAD
/// 65150); this used to be the pressed-disc capacity, 549150. A GDI's sector
/// counts are its track files' sizes / sector size (the firmware's
/// `gdi.rs` fills them when it mounts). `is_cdi` no longer matters.
pub fn leadout_lba<T: TrackInfo>(tracks: &[T], _is_cdi: bool) -> u32 {
    match tracks.last() {
        Some(last) => last.start_lba().wrapping_add(last.sector_count()),
        None => GDROM_LEADOUT_LBA,
    }
}

/// Build a 408-byte Sega GD-ROM TOC response. `area` 0 is the single
/// density area, 1 the high density area. A CDI has no areas: every area
/// number gets the same TOC of all its tracks (stock GDEMU 6.0 answers
/// GET_TOC area 1 of a CDI that way, R117; it used to be all 0xFF).
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
            true
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
        // Lead-out entry (offset 101*4 = 404): the end of the area's last
        // track, CTRL of that track (stock GDI 03 area 1: 41 00 fe 7e). The
        // HD area of a GDI used to be padded out to 549300 (D1).
        let leadout_fad = lt.start_lba().wrapping_add(lt.sector_count()).wrapping_add(150);
        out[404] = ctrl_adr(lt.ctrl());
        out[405] = (leadout_fad >> 16) as u8;
        out[406] = (leadout_fad >> 8) as u8;
        out[407] = leadout_fad as u8;
    }
}

/// Where the pickup "is" for REQ_STAT / GET_SCD: `(fad, track)` -- the
/// sector CDDA is playing or paused at (`cdda_fad`), or `idle_fad` when no
/// CD-DA has been started ([`idle_fad`]: FAD 150 after power-on, the park
/// position after an image switch, D4).
///
/// `last_tno` is the track number the previous REQ_STAT / GET_SCD reported
/// from on a track ([`head_on`]; 0 = none since the mount). Off every track
/// and pregap stock keeps reporting that number (seq 23 CDI 04: after FAD
/// 150 in track 1, the gap FADs 11696 / 11701 before track 2's pregap say
/// track 1; after 11858 in track 2, 11694 says track 2; seq 22 GDI 03: 1506
/// right after a read of 1505 in track 2 says track 3, the number the query
/// before it gave -- a read does not set it, a query does), with CTRL 4
/// ([`head_on`] false; seq 23 `ctl=0x41` there although track 1 is audio).
/// With none, [`head_track`]'s next / last track. The caller stores the
/// number back when [`head_on`] holds.
#[inline(never)] // shared by REQ_STAT and GET_SCD; inlined twice it costs ~100 bytes
pub fn head_position<T: TrackInfo>(
    tracks: &[T],
    state: AudioState,
    cdda_fad: u32,
    idle_fad: u32,
    last_tno: u8,
) -> (u32, Option<&T>) {
    let fad = match state {
        AudioState::Standby => idle_fad,
        _ => cdda_fad,
    };
    let track = head_track(tracks, fad);
    let track = match track {
        Some(t) if !head_on(t, fad) && last_tno != 0 => {
            tracks.iter().find(|u| u.number() == last_tno).or(track)
        }
        _ => track,
    };
    (fad, track)
}

/// Whether a head at `fad` is on `track`'s sectors or its pregap
/// ([`PREGAP_SECTORS`] before it): REQ_STAT / GET_SCD then report the
/// track's own CTRL, index and relative position ([`head_index_rel`]); off
/// it CTRL 4, index 1 and [`SCD_REL_OUTSIDE`] (stock seq 21-23).
pub fn head_on<T: TrackInfo>(track: &T, fad: u32) -> bool {
    let start = track.start_lba().wrapping_add(150);
    (fad >= start && fad - start < track.sector_count()) || (fad < start && start - fad <= PREGAP_SECTORS)
}

/// Sectors before each track's index 1 that stock GDEMU 6.0 counts as its
/// pregap (index 0): seq 22 reads FAD 0 / 100 (track 1), 905 (GDI audio
/// track 2 at 906), 11851 (CDI track 2 at 11852) and 45149 (GDI track 3 at
/// 45150) without a CHECK.
pub const PREGAP_SECTORS: u32 = 150;

/// GET_SCD format 1's relative field while the head is on no track's
/// sectors nor its pregap (a gap, the lead-out): stock GDEMU 6.0 sends this
/// constant, 513660 (seq 21 CDI 04 / 10 FAD 3075 / 6001; seq 22 the lead-out
/// + 0 / 1 / 100 of CDI 04 / 10 and GDI 03, the GDI's gap between the areas
/// 1506 / 23328) -- whatever the FAD and the track.
pub const SCD_REL_OUTSIDE: u32 = 0x07_D67C;

/// The track REQ_STAT / GET_SCD report for a head at `fad`: the track whose
/// sectors hold it, else the next track after it (a FAD in a pregap, in a
/// CDI's space between the sessions, in the GDI's gap between the areas,
/// below 150), else (the lead-out) the last track. Stock GDEMU 6.0: seq 21
/// CDI 04 / 10 FAD 3075 / 6001 (between track 1's 302 sectors and track 2)
/// -> track 2; seq 22 GDI 03 FAD 1506 (after audio track 2, before track 3)
/// and 23328 -> track 3, FAD 1 / 101 -> track 1 (index 0), the lead-out ->
/// the last track (2 / 3). Tracks are in disc order. Where tracks overlap,
/// the open-ended [`track_for_lba`]'s pick wins when it holds the sector.
pub fn head_track<T: TrackInfo>(tracks: &[T], fad: u32) -> Option<&T> {
    let next = || {
        tracks
            .iter()
            .find(|t| t.start_lba().wrapping_add(150) > fad)
            .or_else(|| tracks.last())
    };
    if fad < 150 {
        return next();
    }
    let lba = fad - 150;
    match track_for_lba(tracks, lba) {
        Some(t) if lba - t.start_lba() < t.sector_count() => Some(t),
        _ => next(),
    }
}

/// Index and relative position REQ_STAT (byte 4) and GET_SCD report for a
/// head at `fad` on `track` ([`head_track`]): on the track's sectors index 1
/// and the distance from its start; in its pregap ([`PREGAP_SECTORS`] before
/// it) index 0 and the distance to its start, counting down (seq 22: FAD 1
/// -> `idx=0`, relative 149; 101 -> 49); anywhere else (a gap, the lead-out)
/// index 1 and [`SCD_REL_OUTSIDE`].
pub fn head_index_rel<T: TrackInfo>(track: &T, fad: u32) -> (u8, u32) {
    let start = track.start_lba().wrapping_add(150);
    if fad >= start && fad - start < track.sector_count() {
        (1, fad - start)
    } else if fad < start && start - fad <= PREGAP_SECTORS {
        (0, start - fad)
    } else {
        (1, SCD_REL_OUTSIDE)
    }
}

/// The idle head position (no CD-DA started) for REQ_STAT / GET_SCD (D4).
/// Stock GDEMU 6.0 after an image switch (`switched`; records 03-08, every
/// `img=n` after `52 82 n`): a GDI at the start of its first high-density
/// track (track 3: REQ_STAT `01 8f 41 03 01 00 b0 5e`, FAD 45150), a CDI at
/// track 1, FAD 150 (`01 2f 01 01 01 00 00 96`, not its data session). After
/// power-on, before any switch, FAD 150 (`02 2f 01 01 01 00 00 96`, the
/// dcload CDI, records 02/05/06). Stock has no power-on record of a GDI;
/// NetGD keeps FAD 150 there too (the power-on rule, not the switch rule). A
/// GDI without a track 3 parks at 150.
pub fn idle_fad<T: TrackInfo>(tracks: &[T], is_cdi: bool, switched: bool) -> u32 {
    if switched && !is_cdi {
        if let Some(t) = tracks.iter().find(|t| t.number() >= 3) {
            return t.start_lba().wrapping_add(150);
        }
    }
    150
}

/// Where a CD_READ from `start_fad` leaves the head (REQ_STAT / GET_SCD):
/// just past the last of the `transferred` sectors. Stock GDEMU 6.0 (seq
/// 17/18, `RS_REQ_STAT` after the IP_READ and a same-folder re-switch that
/// changes nothing): 7 sectors from FAD 45150 of GDI 03 -> `01 8f 41 03 01
/// 00 b0 65` (45157); the 7-sector read of CDI 04's Mode 2 track at 0x2E4C
/// with type Mode 1, refused 5/64/00 before any sector, -> `01 2f 41 02 01
/// 00 2e 4c` (the start: the head went there, nothing was read). The status
/// stays pause; track, index and CTRL are those of the track the FAD is in
/// ([`head_position`]).
///
/// `transferred` is what actually reached the host (whole sectors): a read
/// abandoned by a reset or a new command, or cut short, moves the head only
/// that far (stock unmeasured; seq 21).
pub fn read_end_fad(start_fad: u32, transferred: u32) -> u32 {
    start_fad.wrapping_add(transferred)
}

/// Whether a CD_READ of `count` sectors from `start_fad` moves the head to
/// its start before anything is transferred, and so where to: every request
/// with at least one sector, wherever it starts and whether or not it is
/// refused. Stock GDEMU 6.0: refused for its sector type 5/64/00 (seq 17,
/// CDI 04 at 0x2E4C; seq 21 `HD_CHK24` / `HD_CHK28`), refused 5/21/00 from a
/// gap no track holds (seq 21 `HD_AUD1` / `HD_AUD7`, FAD 3075 / 6001; seq 22
/// the GDI's gap between the areas, `GP_HDG_MID` 23328) or from the lead-out
/// and past it (seq 22 `GP_LO_0` / `_1` / `_100`: pos = lead-out + 0 / 1 /
/// 100), and a read below 150 that stock serves from track 1's pregap (seq
/// 22 `GP_LOW_0` / `_100`: pos 1 / 101 after one sector) all leave REQ_STAT
/// at the start (+ what was sent). A count of 0 is not measured (gdprobe's
/// whitelist) and leaves the head. A command refused before CD_READ runs --
/// NOT READY, UNIT ATTENTION -- never gets here.
pub fn read_head_start(start_fad: u32, count: u32) -> Option<u32> {
    (count != 0).then_some(start_fad)
}

/// A CD_READ that starts in a track's pregap ([`PREGAP_SECTORS`] before its
/// index 1, on no track's sectors) and ends in that pregap or on the
/// track's sectors: `(track, zeros)`, the first `zeros` sectors being the
/// pregap's. Stock GDEMU 6.0 serves it as that track's: the pregap sectors
/// are zeros (seq 23 `scrc=` / `crc=` on CDI 04: FAD 0 / 1 / 100 / 149 ->
/// CRC be97ce3f = 2,352 zero bytes; 11702..11851 -> d0912a98 = 2,324 zero
/// bytes, the pregap of the Mode 2 track as Form 2 -- the same as the CDI
/// file's stored pregaps), then the track's own sectors (`GP_XPRE1`: FAD
/// 140, 16 sectors = 10 zero + track 1's first 6, crc 053616bb;
/// `GP_XPRE2`: 11842 = 10 x 2,324 zero + 6 x 2,048 of track 2, 35,528 B).
/// A GDI does not store its pregaps; stock's bytes there are not recorded
/// (the seq 23 run stopped before GDI 03): NetGD sends zeros too. `None` for
/// a count of 0, a start no pregap holds (seq 23 `GP_XGAP2`, 11694 x 16
/// from the gap into the pregap: 5/21/00) or a request past the track's end.
pub fn pregap_read<T: TrackInfo>(tracks: &[T], start_fad: u32, count: u32) -> Option<(&T, u32)> {
    let t = tracks.iter().find(|t| t.start_lba().wrapping_add(150) > start_fad)?;
    let start = t.start_lba().wrapping_add(150);
    let end = start_fad.checked_add(count)?;
    if count == 0 || start - start_fad > PREGAP_SECTORS || end > start.wrapping_add(t.sector_count()) {
        return None;
    }
    // No other track's sectors in the pregap part.
    let zeros_end = end.min(start);
    let on_a_track = tracks.iter().any(|u| {
        let s = u.start_lba().wrapping_add(150);
        s < zeros_end && start_fad < s.wrapping_add(u.sector_count())
    });
    (!on_a_track).then_some((t, zeros_end - start_fad))
}

/// Whether stock GDEMU 6.0 would refuse a CD_READ from `start_fad` as LBA
/// out of range, 5/21/00 ([`ASC_LBA_OUT_OF_RANGE`]): no track holds the
/// sector, nor its pregap ([`PREGAP_SECTORS`] before the track). Seq 21: CDI
/// 04 / 10 FAD 3075 / 6001 (between track 1's 302 sectors and track 2's
/// pregap at 11702); seq 22: the lead-out + 0 / 1 / 100 (CDI and GDI), the
/// GDI's gap 1506 and 23328 -- while 1505 (last sector of audio track 2),
/// 905 / 11851 / 45149 (pregaps) and 0 / 100 (track 1's pregap) read. NetGD
/// does not serve pregap sectors yet: it refuses them 5/00/00 (stock's bytes
/// there are not recorded). A start inside a track whose count runs past the
/// track's end is refused too, ASC not measured (NetGD: 5/00/00).
pub fn cd_read_start_outside<T: TrackInfo>(tracks: &[T], start_fad: u32) -> bool {
    !tracks.iter().any(|t| {
        let start = t.start_lba().wrapping_add(150);
        start_fad >= start.saturating_sub(PREGAP_SECTORS) && start_fad < start.wrapping_add(t.sector_count())
    })
}

/// The head with no CD-DA started: the end of the last CD_READ since the
/// mount (`last_read` from [`read_end_fad`] / [`read_head_start`], or the
/// position where CD-DA was stopped; `None` = nothing since the mount) once
/// the unit has switched, else [`idle_fad`]. Before any switch stock keeps standby at
/// FAD 150 though the BIOS has read the boot disc (records 02/05/06/09/16/17
/// img=0, a cold power-on); after a Dreamcast reset -- which remounts
/// (reset_goto) like a switch -- and the BIOS's reads it shows pause, track
/// 2, FAD 0x05017A, lead-out - 2 of the dcload CDI (records 03/07/08/10-15/
/// 18/20 img=0, R130's open point). A mount or a DEVICE RESET (seq 17 `DR_`)
/// starts again from the park position.
pub fn rest_fad<T: TrackInfo>(tracks: &[T], is_cdi: bool, switched: bool, last_read: Option<u32>) -> u32 {
    match last_read {
        Some(fad) if switched => fad,
        _ => idle_fad(tracks, is_cdi, switched),
    }
}

/// Drive status codes (low nibble of SECNR, byte 0 of REQ_STAT / REQ_SES).
pub const DRIVE_PAUSE: u8 = 0x01;
pub const DRIVE_STANDBY: u8 = 0x02;
pub const DRIVE_PLAY: u8 = 0x03;
/// Busy, tray open, no disc (flycast `gdromv3.h`): what SECNR shows through
/// the swap window and after it with nothing mounted ([`swap_drive_status`]).
pub const DRIVE_BUSY: u8 = 0x00;
pub const DRIVE_OPEN: u8 = 0x06;
pub const DRIVE_NODISC: u8 = 0x07;

/// Disc format, already in the high nibble (SECNR, REQ_STAT byte 1;
/// Flycast `gdrom_if.h`: CD-DA 0x00, CD-ROM 0x10, CD-ROM XA 0x20, GD-ROM 0x80).
/// See [`disc_format`]. Stock GDEMU 6.0 never reports CD-DA for an image
/// (R155: an audio-only CDI is CD-ROM); the constant is Flycast's.
pub const DISC_FORMAT_CDDA: u8 = 0x00;
pub const DISC_FORMAT_CDROM: u8 = 0x10;
pub const DISC_FORMAT_CDROM_XA: u8 = 0x20;
pub const DISC_FORMAT_GDROM: u8 = 0x80;

/// The disc format a mounted image reports (SECNR's and REQ_STAT byte 1's
/// high nibble), as stock GDEMU 6.0 does: a GDI is a GD-ROM; a CDI with any
/// Mode 2 track is CD-ROM XA (R144: the mkdcdisc CDI 04 reports `2_`);
/// every other CDI is CD-ROM -- Mode 1 data (R144: CDI 10, REQ_STAT
/// `01 1f ...`, SECNR `0x11`) and audio only alike (R155: the two-audio-track
/// CDI 11, REQ_STAT `01 1f ...`, SECNR `0x11`; R148 had guessed CD-DA 0x00
/// from Flycast's `GuessDiscType`).
pub fn disc_format<T: TrackInfo>(tracks: &[T], is_cdi: bool) -> u8 {
    if !is_cdi {
        return DISC_FORMAT_GDROM;
    }
    if tracks.iter().any(|t| t.mode() == TRACK_MODE2) {
        DISC_FORMAT_CDROM_XA
    } else {
        DISC_FORMAT_CDROM
    }
}

/// Drive status for the pickup state: what REQ_STAT and REQ_SES put in
/// byte 0 (and SECNR's low nibble). `idle` is the status while no CD-DA has
/// been started ([`AudioState::Standby`]): [`DRIVE_STANDBY`] after power-on
/// until the first image switch, [`DRIVE_PAUSE`] after a switch (D2: stock
/// GDEMU 6.0 answers `02` before any switch, records 02/05/06 img=0, and `01`
/// after every `52 82 n`, records 03-08, before and after its CD_READs).
pub fn drive_status(state: AudioState, idle: u8) -> u8 {
    match state {
        AudioState::Playing => DRIVE_PLAY,
        AudioState::Paused | AudioState::Finished => DRIVE_PAUSE,
        AudioState::Error => 9,
        AudioState::Standby => idle,
    }
}

/// 10-byte Sega REQ_STAT reply: drive status ([`drive_status`] of `state`
/// and `idle`), disc format | repeat count,
/// CTRL/ADR, TNO, index, FAD MSB..LSB, max retry, 0. `disc_format` is a
/// `DISC_FORMAT_*` value (high nibble; D-3, stock GDEMU 6.0 sends 0x2_ for a
/// CDI, R117); `repeat` is the 4-bit CDDA repeat count, sent only while
/// playing: whenever no CD-DA plays the low nibble is 0xF (D3; stock sends
/// `2f` for a CDI and `8f` for a GDI in standby and pause -- there is no
/// stock record of playback, so that keeps the count). `fad`/`track` come
/// from [`head_position`].
pub fn req_stat_reply<T: TrackInfo>(
    state: AudioState,
    idle: u8,
    disc_format: u8,
    repeat: u8,
    fad: u32,
    track: Option<&T>,
) -> [u8; 10] {
    let status = drive_status(state, idle);
    let repeat = if state == AudioState::Playing { repeat } else { 0x0F };
    let (ctrl_adr, tno, index) = track
        .map(|t| (if head_on(t, fad) { (t.ctrl() << 4) | 1 } else { 0x41 }, t.number(), head_index_rel(t, fad).0))
        .unwrap_or((0x41, 1, 1));
    [
        status,
        (disc_format & 0xF0) | (repeat & 0x0F),
        ctrl_adr,
        tno,
        index,
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

/// 24-bit big-endian FAD into `out[0..3]`.
fn put_fad(out: &mut [u8], fad: u32) {
    out[0] = (fad >> 16) as u8;
    out[1] = (fad >> 8) as u8;
    out[2] = fad as u8;
}

/// The 32 bytes REQ_MODE reads a window of, as stock GDEMU 6.0 returns them
/// for (offset 0, length 32) (R117): ten parameter bytes, hardware id
/// `"SE      "`, firmware `"Rev 6.42"`, date `"990316"`.
pub const REQ_MODE_BLOCK: [u8; 32] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0xb4, 0x19, 0x00, 0x00, 0x08, // parameters
    b'S', b'E', b' ', b' ', b' ', b' ', b' ', b' ', // hardware id
    b'R', b'e', b'v', b' ', b'6', b'.', b'4', b'2', // firmware version
    b'9', b'9', b'0', b'3', b'1', b'6', // date
];

/// REQ_MODE reply for CDB offset `offset` (byte 2) and length `len` (byte 4):
/// that window of [`REQ_MODE_BLOCK`], or `None` (the caller aborts) for an
/// empty window or one past the 32 bytes -- what stock does there is not known.
pub fn req_mode_reply(offset: u8, len: u8) -> Option<&'static [u8]> {
    let (o, l) = (offset as usize, len as usize);
    if l == 0 || o + l > REQ_MODE_BLOCK.len() {
        return None;
    }
    Some(&REQ_MODE_BLOCK[o..o + l])
}

/// ASC/ASCQ pairs stock GDEMU 6.0 reports with the sense key (records
/// 03-07): NOT READY with no medium (2/3A/00, e.g. an empty image slot),
/// NOT READY while becoming ready (2/04/01, the swap window), UNIT
/// ATTENTION after a medium change (6/28/00), and ILLEGAL REQUEST / illegal
/// mode for this track (5/64/00) for a CD_READ data-type mismatch
/// ([`cd_read_type_mismatch`]).
pub const ASC_NO_MEDIUM: (u8, u8) = (0x3A, 0x00);
pub const ASC_ILLEGAL_MODE: (u8, u8) = (0x64, 0x00);
pub const ASC_BECOMING_READY: (u8, u8) = (0x04, 0x01);
pub const ASC_MEDIUM_CHANGED: (u8, u8) = (0x28, 0x00);
/// ILLEGAL REQUEST / logical block address out of range (5/21/00): a CD_READ
/// whose start no track holds ([`cd_read_start_outside`]; stock seq 21).
pub const ASC_LBA_OUT_OF_RANGE: (u8, u8) = (0x21, 0x00);

/// CD_READ / CD_READ2 byte 1, bits 3..1: the sector type the host expects.
pub const CD_READ_TYPE_ANY: u8 = 0;
pub const CD_READ_TYPE_MODE1: u8 = 2;
pub const CD_READ_TYPE_MODE2_FORM1: u8 = 4;

/// Does a CD_READ asking for sector type `(flags >> 1) & 7` (CDB byte 1) not
/// fit the track it reads, whose mode is `track_mode` ([`crate::TrackInfo::mode`])
/// (D8, D9)? Then stock GDEMU 6.0 answers CHECK 5/64/00 and sends nothing.
///
/// Stock decides by the track's mode, not by the image format (R144): a Mode
/// 1 track (GDI data, the Mode 1 CDI 10) takes Mode 1 (`0x24`) and refuses
/// Mode 2 Form 1 (`0x28`); a Mode 2 track (mkdcdisc CDI data) the reverse; an
/// audio track (GDI 03 at FAD 906) refuses both. "Any type" (`0x20`, `0x10`)
/// always reads, one sector or seven. Other types (CD-DA, Mode 2 formless,
/// Form 2) were never probed, so they are let through as before.
pub fn cd_read_type_mismatch(flags: u8, track_mode: u8) -> bool {
    match (flags >> 1) & 0x07 {
        CD_READ_TYPE_MODE1 => track_mode != TRACK_MODE1,
        CD_READ_TYPE_MODE2_FORM1 => track_mode != TRACK_MODE2,
        _ => false,
    }
}

/// Does a CD_READ that passed [`cd_read_type_mismatch`] send whole
/// 2,352-byte sectors rather than 2,048-byte payloads? An audio track always
/// does (stock GDEMU 6.0 returns its PCM sectors as stored for `0x20` and
/// `0x10`, one sector or seven, R144), provided it is stored 2,352 bytes per
/// sector. A data track only for data select `0x1_` ([`cd_read_wants_raw`],
/// D10) and only when `raw_data` (the firmware's `raw-sectors` feature), from
/// a sector stored raw or as 2,336 bytes ([`raw_from_stored`]).
///
/// An audio track stored any other way cannot be sent as stock sends it:
/// [`cd_read_unservable`] refuses that read first.
pub fn cd_read_raw_out(flags: u8, track_mode: u8, sector_size: u32, raw_data: bool) -> bool {
    if track_mode == TRACK_AUDIO {
        sector_size == RAW_SECTOR_BYTES as u32
    } else {
        raw_data && cd_read_wants_raw(flags) && raw_from_stored(sector_size)
    }
}

/// Can a CD_READ of a track with this mode and stored sector size not be
/// answered at all? An audio track not stored 2,352 bytes per sector (a GDI
/// line or CDI read mode that claims 2,048 or 2,336 for audio) has no whole
/// PCM sector to send, and a 2,048-byte cut from it would be neither the
/// audio nor anything stock sends: the firmware answers ILLEGAL REQUEST.
pub fn cd_read_unservable(track_mode: u8, sector_size: u32) -> bool {
    track_mode == TRACK_AUDIO && sector_size != RAW_SECTOR_BYTES as u32
}

/// Does CD_READ byte 1 ask for the whole 2,352-byte sector (data select
/// `0x1_`, D10)? Stock GDEMU 6.0 answers `0x10` with 2,352 bytes per sector
/// on GDI and CDI data tracks (records R117-R132); other data selects were
/// not probed.
pub fn cd_read_wants_raw(flags: u8) -> bool {
    flags & 0xF0 == 0x10
}

/// Bytes per sector of a raw read.
pub const RAW_SECTOR_BYTES: usize = 2352;

/// Can the raw form be made from a sector stored with `sector_size` bytes?
/// 2,352 is stored raw already; 2,336 (a CDI's Mode 2 data) only lacks the
/// 16-byte sync + header ([`mode2_raw_header`]). A 2,048-byte track would
/// need EDC/ECC computed, and stock's answer there is not known.
pub fn raw_from_stored(sector_size: u32) -> bool {
    sector_size == 2352 || sector_size == 2336
}

/// The 16 bytes in front of a 2,336-byte stored Mode 2 sector in its raw
/// form: sync (00, ten FF, 00), the absolute address `lba + 150` as BCD
/// minute/second/frame, mode 2. What stock GDEMU 6.0 puts there (its 0x10
/// reads of the CDIs match `tools/mksdcard`'s `raw_xa_form1`, D10).
pub fn mode2_raw_header(lba: u32) -> [u8; 16] {
    let mut h = [0xFFu8; 16];
    h[0] = 0;
    h[11] = 0;
    put_msf(&mut h[12..15], lba.wrapping_add(150));
    h[15] = 2;
    h
}

/// Write the raw 2,352-byte form of a stored sector of `stored.len()` bytes
/// (2,352 or 2,336, see [`raw_from_stored`]) at `lba` into `out[..2352]`.
pub fn raw_sector(stored: &[u8], lba: u32, out: &mut [u8]) {
    if stored.len() == 2336 {
        out[..16].copy_from_slice(&mode2_raw_header(lba));
        out[16..2352].copy_from_slice(stored);
    } else {
        out[..2352].copy_from_slice(&stored[..2352]);
    }
}

/// In place: turn `count` stored sectors of `sec_size` bytes
/// (2,352 or 2,336) starting `head` bytes into `buf` into raw 2,352-byte
/// sectors from offset 0. 2,352-byte sectors only move down (front to back).
/// A 2,336-byte sector grows by its 16-byte header: after dropping the
/// lead-in every destination lies at or above its source, so the walk goes
/// from the last sector down. Returns the length.
pub fn expand_raw_in_place(buf: &mut [u8], head: usize, sec_size: usize, lba: u32, count: usize) -> usize {
    const RAW: usize = RAW_SECTOR_BYTES;
    let pad = RAW - sec_size; // 0 or 16
    if pad == 0 {
        // Downwards only: front to back is safe.
        for s in 0..count {
            buf.copy_within(head + s * sec_size..head + (s + 1) * sec_size, s * RAW);
        }
    } else {
        // First drop the block-alignment lead-in, so that sector s's source
        // (s * sec_size) never lies above its destination (s * RAW + 16).
        buf.copy_within(head..head + count * sec_size, 0);
        for s in (0..count).rev() {
            let src = s * sec_size;
            buf.copy_within(src..src + sec_size, s * RAW + pad);
            buf[s * RAW..s * RAW + pad]
                .copy_from_slice(&mode2_raw_header(lba + s as u32));
        }
    }
    count * RAW
}

/// Bytes of a Mode 2 Form 2 sector's user data: what stock GDEMU 6.0 sends
/// for each Form 2 sector a `0x20` read covers (D14).
pub const FORM2_SECTOR_BYTES: usize = 2324;
/// Where those bytes start in a 2,336-byte stored sector: after the 8-byte
/// subheader (the 4-byte Form 2 EDC at the end is left out).
pub const FORM2_DATA_OFFSET: usize = 8;

/// Is a 2,336-byte stored Mode 2 sector Form 2? The subheader's submode
/// byte (offset 2) carries the Form bit, 0x20 (mkdcdisc's postgap: `00 00 20
/// 00 00 00 20 00`).
pub fn stored_is_form2(stored: &[u8]) -> bool {
    stored.len() > 2 && stored[2] & 0x20 != 0
}

/// Could a CD_READ with byte 1 `flags` get Form 2 payloads (feature
/// `form2-sectors`, D14)? Data select `0x2_` with no expected type, on a
/// Mode 2 track stored 2,336 bytes a sector (a CDI's data) -- the case the
/// stock records show (record 05, `0x20` on the postgap). The firmware then
/// looks at each sector's submode ([`all_form2`]).
pub fn cd_read_may_be_form2(flags: u8, track_mode: u8, sector_size: u32) -> bool {
    flags & 0xF0 == 0x20
        && (flags >> 1) & 0x07 == CD_READ_TYPE_ANY
        && track_mode == TRACK_MODE2
        && sector_size == 2336
}

/// Are all `count` 2,336-byte stored sectors in `buf`, the first `head`
/// bytes in, Form 2 ([`stored_is_form2`])? A read mixing Form 1 and Form 2
/// is not in the stock records, so the firmware keeps 2,048 B for it.
pub fn all_form2(buf: &[u8], head: usize, count: usize) -> bool {
    (0..count).all(|s| stored_is_form2(&buf[head + s * 2336..]))
}

/// In place: take `len` bytes from offset `off` of each of `count` stored
/// sectors of `sec_size` bytes (the first `head` bytes into `buf`) and pack
/// them from the front, `len` apart. Returns `count * len`. Needs
/// `len <= sec_size`, so every copy moves towards lower addresses.
pub fn compact_in_place(buf: &mut [u8], head: usize, sec_size: usize, count: usize, off: usize, len: usize) -> usize {
    debug_assert!(off + len <= sec_size);
    for s in 0..count {
        let src = head + s * sec_size + off;
        buf.copy_within(src..src + len, s * len);
    }
    count * len
}

/// Sense keys NetGD reports.
pub const SENSE_NOT_READY: u8 = 0x02;
pub const SENSE_MEDIUM_ERROR: u8 = 0x03;
pub const SENSE_ILLEGAL_REQUEST: u8 = 0x05;
pub const SENSE_UNIT_ATTENTION: u8 = 0x06;

/// Status register of every CHECK CONDITION on stock GDEMU 6.0 (D5, records
/// 03-07): DSC | ERR, with DRDY clear.
pub const CHECK_STATUS: u8 = 0x11;

/// Error register of a CHECK CONDITION on stock GDEMU 6.0 (D5): the sense key
/// in the high nibble and nothing else (no ABRT bit): 0x20 not ready, 0x50
/// illegal request, 0x60 unit attention.
pub const fn check_error(sense_key: u8) -> u8 {
    (sense_key & 0x0F) << 4
}

/// The NOT READY window after an image switch, as stock GDEMU 6.0 shows it
/// (D13, `stock_20261121_04_img3-7.txt` and every other switch record):
/// [`SwapPhase::Open`] for the first `open_ms` (2/3A/00, "no medium"), then
/// [`SwapPhase::Detect`] for `detect_ms` (2/04/01, "becoming ready"), then
/// [`SwapPhase::Done`]: one UNIT ATTENTION 6/28/00 and READY with an image,
/// 2/3A/00 for good without one. Stock takes the two lengths from GDEMU.INI
/// `open_time` / `detect_time` (150/150 -> READY at 331-351 ms, R119; 600/300
/// -> 932-956 ms, R129). These constants are its values when the INI does not
/// set the keys: 500 + 250 ms (READY at 785-795 ms, R135), the documented
/// GDEMU defaults.
pub const SWAP_OPEN_MS: u32 = 500;
pub const SWAP_DETECT_MS: u32 = 250;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwapPhase {
    /// Tray shown open: NOT READY, no medium.
    Open,
    /// Disc being detected: NOT READY, becoming ready.
    Detect,
    /// The window is over.
    Done,
}

/// Whether a command runs the swap window's clock (`drive_not_ready`: closes an
/// expired window and raises its UNIT ATTENTION). REQUEST SENSE (0x03) and
/// REQ_ERROR (0x13) do not: they report the sense of the last CHECK, and a
/// window closing inside them would replace that sense with 6/28/00 and
/// swallow the UNIT ATTENTION before any CHECK showed it (R139). The window
/// ends on the next normal command.
pub fn advances_swap_window(opcode: u8) -> bool {
    opcode != 0x03 && opcode != 0x13
}

/// Whether a sense read that reported `key` also clears the UNIT ATTENTION: only
/// when it reported it (key 6). `52 00` clears the pending key without
/// serving the UA, so a REQ_ERROR after it reports nothing and must leave the
/// UA for the next command's CHECK (R139).
pub fn sense_read_clears_ua(key: u8) -> bool {
    key == SENSE_UNIT_ATTENTION
}

/// The phase `elapsed_ms` after a switch.
pub fn swap_phase(elapsed_ms: u32, open_ms: u32, detect_ms: u32) -> SwapPhase {
    if elapsed_ms < open_ms {
        SwapPhase::Open
    } else if elapsed_ms - open_ms < detect_ms {
        SwapPhase::Detect
    } else {
        SwapPhase::Done
    }
}

/// ASC/ASCQ of a NOT READY answer in `phase` (`Done` = no medium after it).
pub fn not_ready_asc(phase: SwapPhase) -> (u8, u8) {
    match phase {
        SwapPhase::Detect => ASC_BECOMING_READY,
        _ => ASC_NO_MEDIUM,
    }
}

/// SECNR's drive status nibble for `phase`; `medium` = an image mounted.
/// Open, busy, then paused (or no disc): what stock GDEMU 6.0 shows (R144,
/// gdprobe `secnr=`; R126 had guessed the same from the GD-ROM's own codes
/// for a tray cycle). The whole byte is [`swap_secnr`].
pub fn swap_drive_status(phase: SwapPhase, medium: bool) -> u8 {
    match phase {
        SwapPhase::Open => DRIVE_OPEN,
        SwapPhase::Detect => DRIVE_BUSY,
        SwapPhase::Done if medium => DRIVE_PAUSE,
        SwapPhase::Done => DRIVE_NODISC,
    }
}

/// The whole SECNR byte for `phase` ([`swap_drive_status`] low): the disc
/// format (`disc_format`, high nibble) only once the window is over with a
/// medium. Stock GDEMU 6.0 (R144, gdprobe `secnr=`): `0x06` in the open
/// phase and `0x00` in the detect phase, format nibble 0 in both, then
/// `0xX1` with the format by the UNIT ATTENTION (CDI 04 `0x21`, Mode 1 CDI 10
/// `0x11`, GDI 03 `0x81`); an empty folder `0x06`, `0x00`, then `0x07`.
pub fn swap_secnr(phase: SwapPhase, medium: bool, disc_format: u8) -> u8 {
    let status = swap_drive_status(phase, medium);
    if phase == SwapPhase::Done && medium {
        (disc_format & 0xF0) | status
    } else {
        status
    }
}

/// What a running NOT READY window is: the tray cycle of an image switch, or
/// the re-detect a DEVICE RESET starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowKind {
    /// After `52 82 n` to another folder (or any mount): open, detect, done.
    Switch,
    /// After ATA DEVICE RESET (0x08): detect, done -- never the open `0x06`
    /// ([`device_reset_window`]).
    Redetect,
}

/// How long the re-detect after DEVICE RESET lasts: GDEMU.INI detect_time,
/// the swap window's detect phase (`detect_ms`; [`SWAP_DETECT_MS`] without
/// the key). Stock GDEMU 6.0: SECNR `0x00` at the reset and the final byte
/// 147-154 ms later with detect_time = 150 (R155, R159, seq 17), about
/// 401 ms later with detect_time = 400 (seq 18, open_time 150), so it is
/// detect_time, not open_time and not a fixed time.
pub const fn device_reset_redetect_ms(detect_ms: u32) -> u32 {
    detect_ms
}

/// The window an ATA DEVICE RESET (0x08) leaves running, as `(open_ms,
/// detect_ms)` for [`swap_phase`] from the reset: no open phase, then the
/// re-detect. Stock GDEMU 6.0 (R155, R159, seq 17) does this on every DEVICE
/// RESET -- inside a switch's window, after a same-folder re-switch, and with
/// no switch at all: SECNR `0x00` at once (never `0x06`), the final byte
/// detect_time later, then one UNIT ATTENTION 6/28/00 with a medium (seq 17
/// `DR_`: `ua_28=1`; an empty folder stays 2/3A/00). A reset inside a
/// switch's window replaces it, and ends the same way.
pub const fn device_reset_window(detect_ms: u32) -> (u32, u32) {
    (0, device_reset_redetect_ms(detect_ms))
}

/// Status (ALTSTATUS) a DEVICE RESET completes with: `0x00`, DRDY clear,
/// until the next command -- the re-detect's end does not change it (R155,
/// R159: through 1,000 and 2,300 ms of polling).
pub const DEVICE_RESET_STATUS: u8 = 0x00;
/// Error register after DEVICE RESET: diagnostic code 0x01, "no error"
/// (R155, R159). Bit 0 needs a bitstream with the MCU-writable ERROR bits
/// 0-1 (feat/fpga-regs); an older one shows 0x00.
pub const DEVICE_RESET_ERROR: u8 = 0x01;
/// Interrupt reason after DEVICE RESET (the signature's sector count, 0x01).
pub const DEVICE_RESET_IRR: u8 = 0x01;

/// SECNR for a window of `kind` in `phase`. A switch's window is
/// [`swap_secnr`] (it ends paused: stock's `0xX1` after `52 82`). The
/// re-detect after DEVICE RESET is busy `0x00` and then the byte at rest:
/// the disc format with `idle_status` (the status REQ_STAT reports idle, D2)
/// -- stock's records only show it after a switch, i.e. paused, where the two
/// agree -- or NO DISC `0x07` with no medium.
pub fn window_secnr(kind: WindowKind, phase: SwapPhase, medium: bool, disc_format: u8, idle_status: u8) -> u8 {
    match (kind, phase) {
        (WindowKind::Redetect, SwapPhase::Done) if medium => (disc_format & 0xF0) | (idle_status & 0x0F),
        _ => swap_secnr(phase, medium, disc_format),
    }
}

/// How the last mount attempt by folder name ended (for [`same_folder_reswitch`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MountOutcome {
    /// An image mounted.
    Mounted,
    /// The folder exists and has nothing to mount: stock accepts it and
    /// reports no medium (D12).
    NoImage,
    /// Anything else: no such folder, a parse error, a card read error. The
    /// next `52 82 n` must try again.
    Failed,
}

/// Whether `52 82` / `52 81` to the folder `staged` is a same-folder
/// re-switch, which stock GDEMU 6.0 treats as a no-op (R159; seq 17 `RS_`:
/// no window, SECNR and ALTSTATUS flat, no UNIT ATTENTION -- `ua_28=0`, the
/// next TUR is READY --, REQ_STAT's state and head position unchanged).
///
/// `last` is the last mount attempt's folder name and outcome (`None` once
/// any other mount -- the default, a hard-reset remount -- has run). Only
/// the same folder, compared by name (so `04` and `0004` stay different
/// folders), and only while the drive still holds what that attempt left:
/// the image (`medium`) after [`MountOutcome::Mounted`], the empty folder's
/// no medium (`empty_slot`) after [`MountOutcome::NoImage`]. A failed attempt,
/// an eject or a write session never makes one: the host can retry.
pub fn same_folder_reswitch(staged: &str, last: Option<(&str, MountOutcome)>, medium: bool, empty_slot: bool) -> bool {
    match last {
        Some((name, MountOutcome::Mounted)) => medium && !staged.is_empty() && name == staged,
        Some((name, MountOutcome::NoImage)) => !medium && empty_slot && !staged.is_empty() && name == staged,
        _ => false,
    }
}

/// SECNR to put back after a host bus reset (or when a swap window is cut
/// short) outside a swap window, from the byte the FPGA left latched.
///
/// With no medium it is NO DISC with format 0 (`0x07`), as stock shows for no
/// medium (R144) and as start-up and DEVICE RESET already write: the FPGA's
/// reset reloads its fixed `0x81`, and keeping that status nibble -- or a
/// stale format after an EJECT -- would claim a disc. With a medium only the
/// format nibble is replaced and the latched status nibble is kept (the
/// FPGA's `1` after a bus reset), as before.
pub fn reset_secnr(medium: bool, disc_format: u8, latched: u8) -> u8 {
    if !medium {
        return DRIVE_NODISC;
    }
    (disc_format & 0xF0) | (latched & 0x0F)
}

/// How a command is answered while the drive is not ready (swap window, or
/// no medium).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotReadyAnswer {
    /// As usual.
    Serve,
    /// CHECK CONDITION, NOT READY (key 2, ASC from [`not_ready_asc`]).
    NotReady,
    /// CHECK CONDITION, ILLEGAL REQUEST (key 5).
    IllegalRequest,
}

/// Stock GDEMU 6.0 with no medium (records 04 img=6/7, `06_gdi-variants`):
/// TEST UNIT READY, REQ_STAT, GET_TOC (both areas), REQ_SES 0 and GET_SCD
/// are NOT READY; REQ_SES 1 and up are ILLEGAL REQUEST; REQ_MODE, REQ_ERROR
/// and `52 00` answer normally. Not in the records: CD_READ and CD_PLAY are
/// NOT READY (NetGD's answer while ejected); everything else is served
/// (REQUEST SENSE, `0x52` -- the switch is what leaves the state --, NetGD's
/// `0xE0`, START/STOP, SET_MODE, CD_SEEK, the security commands).
pub fn not_ready_answer(cdb: &[u8; 12]) -> NotReadyAnswer {
    match cdb[0] {
        0x00 | 0x10 | 0x14 | 0x20 | 0x30 | 0x31 | 0x40 => NotReadyAnswer::NotReady,
        0x15 if cdb[2] == 0 => NotReadyAnswer::NotReady,
        0x15 => NotReadyAnswer::IllegalRequest,
        _ => NotReadyAnswer::Serve,
    }
}

/// 10-byte REQ_ERROR reply (SPI): byte 0 = 0xF0, byte 2 low nibble = sense
/// key, bytes 8/9 = ASC/ASCQ. Stock GDEMU 6.0 sends `F0 00 ...` with nothing
/// pending (R117; this used to be all zeros) and fills ASC/ASCQ (D7, e.g.
/// `f0 00 02 .. 3a 00` for no medium; they used to be 0).
pub fn req_error_reply(sense_key: u8, asc: u8, ascq: u8) -> [u8; 10] {
    let mut r = [0u8; 10];
    r[0] = 0xF0;
    r[2] = sense_key & 0x0F;
    r[8] = asc;
    r[9] = ascq;
    r
}

/// IDENTIFY PACKET DEVICE (ATA 0xA1) data, as stock GDEMU 6.0 returns it:
/// 80 bytes (the GD-ROM's 00h-4Fh block, SPI table 3.5), not ATA's 512 (D16,
/// `stock_20261122_08_identify.txt`). Byte order is the host's: each 16-bit
/// data-register word carries bytes `2i` (low) and `2i + 1` (high), which is
/// how gdprobe stores them. Bytes 0-2 0x20, then zeros to 15; 16-31 the
/// manufacturer "SE", 32-47 the model "CD-ROM DRIVE", 48-63 the firmware
/// revision "6.42", all space padded and not byte-swapped; 64-79 zeros.
pub const IDENTIFY_PACKET_REPLY: [u8; 80] = *b"\
\x20\x20\x20\0\0\0\0\0\0\0\0\0\0\0\0\0\
SE              \
CD-ROM DRIVE    \
6.42            \
\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";

/// 6-byte REQ_SES reply for one session (see the firmware's `ide.rs` for
/// the history of why each subcommand reports what it does).
///   byte 0 = `status`, the drive status ([`drive_status`]; stock GDEMU 6.0
///   sends 0x02 in standby, R117, and 0x01 after a switch, D2 -- this was a
///   constant 0x01), byte 1 = 0,
///   byte 2 = track, bytes 3..6 = FAD BE
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
    status: u8,
) -> Option<[u8; 6]> {
    let mut r = [0u8; 6];
    r[0] = status;
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

/// Build a CD_SCD reply. Format 1 is the plain 14-byte form: header
/// `[0, audio_status, 0, 14]`, then CTRL/ADR, TNO, index, the relative and
/// (after a 0) the absolute position as 24-bit binary FADs (D-2: stock
/// GDEMU 6.0 sends `00 00 96` at standby, R117; this used to be BCD MSF).
/// Anything else is the 100-byte form: the raw Q subcode (MSF in BCD, as on
/// a disc) with a CRC-CCITT substituted into Q[10..12], one byte per bit,
/// Q in bit 6 (`0x40`) and P, R..W zero (stock, R117; was `0x7F` / `0x3F`).
/// Returns the reply length. `fad`/`track` come from [`head_position`].
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
    let mut rel = 0;
    q[2] = 0x01; // index 1
    match track {
        Some(t) => {
            // CTRL, ADR=1 (Q-mode 1); CTRL 4 off the track (stock seq 23).
            q[0] = if head_on(t, fad) { (t.ctrl() << 4) | 1 } else { 0x41 };
            q[1] = bcd(t.number() as u32);
            // Index 0 and a countdown in the pregap, stock's constant off
            // the track (`head_index_rel`, seq 22).
            (q[2], rel) = head_index_rel(t, fad);
        }
        None => q[0] = 0x41,
    }

    r.fill(0);
    r[1] = audio_status;
    if format == 1 {
        r[3] = 14;
        r[4..7].copy_from_slice(&q[..3]);
        put_fad(&mut r[7..10], rel);
        put_fad(&mut r[11..14], fad);
        return 14;
    }
    put_msf(&mut q[3..6], rel);
    put_msf(&mut q[7..10], fad);
    // q[10..12] will be overwritten with CRC mid-expansion.
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
            r[4 + i * 8 + bit] = (b as u8) << 6;
        }
    }
    100
}
