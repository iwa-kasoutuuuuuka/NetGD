//! The firmware's original function bodies, copied verbatim (only the
//! statics replaced by arguments and `ShortFileName` by `String<32>`), kept
//! as an oracle: the extracted crate must agree with these on every input.
//! Once the firmware has been switched over these are the only remaining
//! copy of the old code.
#![allow(dead_code)]

use heapless::{String, Vec};

#[derive(Clone, Debug)]
pub struct Track {
    pub number: u8,
    pub start_lba: u32,
    pub ctrl: u8,
    pub sector_size: u16,
    pub sector_count: u32,
    pub filename: String<32>,
}

pub struct Gdi {
    pub tracks: Vec<Track, 99>,
    pub boot_area_lba: u32,
    pub is_cdi: bool,
    pub sessions: u8,
}

// ---- gdi.rs ---------------------------------------------------------------

#[derive(Debug, PartialEq)]
pub enum GdiError {
    Parse,
    TooManyTracks,
}

pub fn parse(data: &[u8]) -> Result<Vec<Track, 99>, GdiError> {
    let mut tracks: Vec<Track, 99> = Vec::new();
    let mut lines = SplitLines::new(data);

    let header = lines.next().ok_or(GdiError::Parse)?;
    let _expected: u32 = parse_u32(header)?;

    while let Some(line) = lines.next() {
        let trimmed = trim(line);
        if trimmed.is_empty() {
            continue;
        }
        let mut fields = SplitWS::new(trimmed);
        let number = parse_u32(fields.next().ok_or(GdiError::Parse)?)? as u8;
        let start_lba = parse_u32(fields.next().ok_or(GdiError::Parse)?)?;
        let ctrl = parse_u32(fields.next().ok_or(GdiError::Parse)?)? as u8;
        let sector_size = parse_u32(fields.next().ok_or(GdiError::Parse)?)? as u16;
        let filename_raw = fields.next().ok_or(GdiError::Parse)?;
        let _ = fields.next();

        let filename = sfn_from_bytes(filename_raw)?;
        tracks
            .push(Track { number, start_lba, ctrl, sector_size, sector_count: 0, filename })
            .map_err(|_| GdiError::TooManyTracks)?;
    }
    Ok(tracks)
}

fn parse_u32(s: &[u8]) -> Result<u32, GdiError> {
    let mut n: u32 = 0;
    let mut any = false;
    for &b in s {
        if !b.is_ascii_digit() {
            if any { break; }
            return Err(GdiError::Parse);
        }
        n = n.checked_mul(10).and_then(|x| x.checked_add((b - b'0') as u32))
            .ok_or(GdiError::Parse)?;
        any = true;
    }
    if !any { return Err(GdiError::Parse); }
    Ok(n)
}

fn sfn_from_bytes(raw: &[u8]) -> Result<String<32>, GdiError> {
    let mut buf: String<32> = String::new();
    for &b in raw {
        if b == b'"' { continue; }
        buf.push(b as char).map_err(|_| GdiError::Parse)?;
    }
    Ok(buf)
}

fn trim(s: &[u8]) -> &[u8] {
    let mut start = 0;
    while start < s.len() && (s[start] == b' ' || s[start] == b'\t' || s[start] == b'\r') {
        start += 1;
    }
    let mut end = s.len();
    while end > start && (s[end - 1] == b' ' || s[end - 1] == b'\t' || s[end - 1] == b'\r') {
        end -= 1;
    }
    &s[start..end]
}

struct SplitLines<'a> { data: &'a [u8], i: usize }
impl<'a> SplitLines<'a> {
    fn new(data: &'a [u8]) -> Self { Self { data, i: 0 } }
    fn next(&mut self) -> Option<&'a [u8]> {
        if self.i >= self.data.len() { return None; }
        let start = self.i;
        while self.i < self.data.len() && self.data[self.i] != b'\n' {
            self.i += 1;
        }
        let line = &self.data[start..self.i];
        if self.i < self.data.len() { self.i += 1; }
        Some(line)
    }
}

struct SplitWS<'a> { data: &'a [u8], i: usize }
impl<'a> SplitWS<'a> {
    fn new(data: &'a [u8]) -> Self { Self { data, i: 0 } }
    fn next(&mut self) -> Option<&'a [u8]> {
        while self.i < self.data.len() && (self.data[self.i] == b' ' || self.data[self.i] == b'\t') {
            self.i += 1;
        }
        if self.i >= self.data.len() { return None; }
        let start = self.i;
        while self.i < self.data.len() && self.data[self.i] != b' ' && self.data[self.i] != b'\t' {
            self.i += 1;
        }
        Some(&self.data[start..self.i])
    }
}

impl Gdi {
    pub fn track_for_lba(&self, lba: u32) -> Option<&Track> {
        self.tracks.iter().rfind(|t| t.start_lba <= lba)
    }

    pub fn leadout_lba(&self) -> u32 {
        if self.is_cdi {
            if let Some(last) = self.tracks.last() {
                return last.start_lba + last.sector_count;
            }
        }
        549150
    }

    pub fn build_toc(&self, area: u8, out: &mut [u8; 408]) {
        out.fill(0xFF);
        let in_area = |t: &Track| -> bool {
            if self.is_cdi {
                area == 0
            } else if area == 0 {
                t.start_lba < self.boot_area_lba
            } else {
                t.start_lba >= self.boot_area_lba
            }
        };
        let ctrl_adr = |ctl: u8| -> u8 { (ctl << 4) | 0x01 };

        let mut first: Option<u8> = None;
        let mut last: Option<u8> = None;
        let mut last_track_ref: Option<&Track> = None;

        for t in self.tracks.iter() {
            if !in_area(t) {
                continue;
            }
            if t.number == 0 || t.number > 99 {
                continue;
            }
            let idx = (t.number - 1) as usize * 4;
            let fad = t.start_lba + 150;
            out[idx] = ctrl_adr(t.ctrl);
            out[idx + 1] = (fad >> 16) as u8;
            out[idx + 2] = (fad >> 8) as u8;
            out[idx + 3] = fad as u8;

            first.get_or_insert(t.number);
            last = Some(t.number);
            last_track_ref = Some(t);
        }

        if let (Some(f), Some(l), Some(lt)) = (first, last, last_track_ref) {
            let first_ctrl = self.tracks.iter()
                .find(|t| t.number == f)
                .map(|t| ctrl_adr(t.ctrl))
                .unwrap_or(0x01);
            out[396] = first_ctrl;
            out[397] = f;
            out[398] = 0;
            out[399] = 0;
            out[400] = ctrl_adr(lt.ctrl);
            out[401] = l;
            out[402] = 0;
            out[403] = 0;
            let mut leadout_fad = (lt.start_lba + lt.sector_count) + 150;
            if area == 1 && !self.is_cdi {
                leadout_fad = leadout_fad.max(self.leadout_lba() + 150);
            }
            out[404] = ctrl_adr(lt.ctrl);
            out[405] = (leadout_fad >> 16) as u8;
            out[406] = (leadout_fad >> 8) as u8;
            out[407] = leadout_fad as u8;
        }
    }
}

// ---- ide.rs ---------------------------------------------------------------

#[derive(Clone, Copy)]
pub enum Status { Playing, Paused, Finished, Error, Standby }

#[derive(Clone, Copy)]
pub struct Snapshot { pub status: Status, pub repeat: u8, pub fad: u32 }

pub fn head_position<'a>(s: &Snapshot, gdi: Option<&'a Gdi>) -> (u32, Option<&'a Track>) {
    let fad = match s.status {
        Status::Standby => 150,
        _ => s.fad,
    };
    let track = gdi.and_then(|g| g.track_for_lba(fad.saturating_sub(150)));
    (fad, track)
}

pub fn build_req_stat_reply(s: &Snapshot, gdi: Option<&Gdi>) -> [u8; 10] {
    let status: u8 = match s.status {
        Status::Playing => 3,
        Status::Paused | Status::Finished => 1,
        Status::Error => 9,
        Status::Standby => 2,
    };
    let (fad, track) = head_position(s, gdi);
    let (ctrl_adr, tno) = track.map(|t| ((t.ctrl << 4) | 1, t.number)).unwrap_or((0x41, 1));
    [
        status,
        s.repeat,
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

pub fn put_msf(out: &mut [u8], blk: u32) {
    out[0] = bcd(blk / 4500);
    out[1] = bcd((blk % 4500) / 75);
    out[2] = bcd(blk % 75);
}

pub fn build_req_ses_reply(gdi: Option<&Gdi>, session: u8) -> Option<[u8; 6]> {
    let mut r = [0u8; 6];
    r[0] = 0x01;
    let (track, fad) = if let Some(g) = gdi {
        match session {
            0 => (g.sessions, g.leadout_lba() + 150),
            1 => {
                let t = g.tracks.iter().find(|t| t.start_lba < g.boot_area_lba)?;
                (t.number, t.start_lba + 150)
            }
            2 if g.is_cdi => {
                let t = g.tracks.iter().find(|t| t.start_lba >= g.boot_area_lba)?;
                (t.number, t.start_lba + 150)
            }
            _ => return None,
        }
    } else {
        return None;
    };
    r[2] = track;
    r[3] = (fad >> 16) as u8;
    r[4] = (fad >> 8) as u8;
    r[5] = fad as u8;
    Some(r)
}

pub fn build_get_scd_reply(format: u8, s: &Snapshot, gdi: Option<&Gdi>, r: &mut [u8; 100]) -> usize {
    let audio_status: u8 = match s.status {
        Status::Playing => 0x11,
        Status::Paused => 0x12,
        Status::Finished => 0x13,
        Status::Error => 0x14,
        Status::Standby => 0x15,
    };
    let (fad, track) = head_position(s, gdi);
    let mut q: [u8; 12] = [0; 12];
    match track {
        Some(t) => {
            q[0] = (t.ctrl << 4) | 1;
            q[1] = bcd(t.number as u32);
            put_msf(&mut q[3..6], fad.saturating_sub(t.start_lba + 150));
        }
        None => q[0] = 0x41,
    }
    q[2] = 0x01;
    put_msf(&mut q[7..10], fad);

    r.fill(0);
    r[1] = audio_status;
    if format == 1 {
        r[3] = 14;
        r[4..14].copy_from_slice(&q[..10]);
        return 14;
    }
    r[3] = 100;

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

// ---- cdi.rs ---------------------------------------------------------------

/// The old CDI walk over an in-memory file (reads replaced by slice access,
/// everything else verbatim). Returns (sessions, tracks) or the error name.
pub fn cdi_parse(file: &[u8]) -> Result<(u8, Vec<Track, 99>, Vec<u32, 99>), &'static str> {
    fn rd_u16(b: &[u8], at: usize) -> u16 { u16::from_le_bytes([b[at], b[at + 1]]) }
    fn rd_u32(b: &[u8], at: usize) -> u32 { u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]) }
    fn sector_size(read_mode: u32) -> Option<u16> {
        match read_mode { 0 => Some(2048), 1 => Some(2336), 2 => Some(2352), _ => None }
    }
    let file_len = file.len() as u32;
    if file_len < 8 { return Err("BadCdi"); }
    let read_at = |off: u32, buf: &mut [u8]| -> Result<(), &'static str> {
        let off = off as usize;
        match file.get(off..off + buf.len()) {
            Some(s) => { buf.copy_from_slice(s); Ok(()) }
            None => Err("Read"),
        }
    };
    let mut w4 = [0u8; 4];
    read_at(file_len - 4, &mut w4)?;
    let desc_len = rd_u32(&w4, 0);
    if desc_len < 16 || desc_len > file_len { return Err("BadCdi"); }
    let desc_start = file_len - desc_len;
    let mut one = [0u8; 1];
    read_at(desc_start, &mut one)?;
    let session_count = one[0];

    let mut tracks: Vec<Track, 99> = Vec::new();
    let mut offsets: Vec<u32, 99> = Vec::new();
    let mut base: u32 = 0;
    let mut cursor = desc_start + 1;
    let mut number: u8 = 1;
    for _ in 0..session_count {
        let mut sh = [0u8; 7];
        read_at(cursor, &mut sh)?;
        let track_count = rd_u16(&sh, 1);
        cursor += 7;
        for ti in 0..track_count {
            let mut head = [0u8; 25];
            read_at(cursor, &mut head)?;
            let name_len = head[24] as u32;
            let td_at = cursor + 25 + name_len + 31;
            let mut td = [0u8; 188];
            read_at(td_at, &mut td)?;
            let pregap = rd_u32(&td, 2);
            let sector_count = rd_u32(&td, 6);
            let start_lba = rd_u32(&td, 32);
            let total_length = rd_u32(&td, 36);
            let read_mode = rd_u32(&td, 56);
            let control = rd_u32(&td, 60);
            let ssize = sector_size(read_mode).ok_or("BadCdi")?;
            let file_offset = base
                .checked_add(pregap.checked_mul(ssize as u32).ok_or("BadCdi")?)
                .ok_or("BadCdi")?;
            if tracks
                .push(Track { number, start_lba, ctrl: (control & 0x0F) as u8, sector_size: ssize,
                              sector_count, filename: String::new() })
                .is_err()
            {
                return Err("BadCdi");
            }
            let _ = offsets.push(file_offset);
            base = base
                .checked_add(total_length.checked_mul(ssize as u32).ok_or("BadCdi")?)
                .ok_or("BadCdi")?;
            number = number.wrapping_add(1);
            let is_last = ti + 1 == track_count;
            cursor = td_at + if is_last { 188 } else { 188 - 8 };
        }
    }
    if tracks.is_empty() { return Err("BadCdi"); }
    Ok((session_count, tracks, offsets))
}
