//! GDI text parser. Behaviour is that of the original firmware parser,
//! quirks included (see the tests).

use crate::{Error, Full, String, TrackDesc, TrackSink, Vec, VecSink, MAX_TRACKS};

/// Parse `data` (the whole `.gdi` text) into `sink`.
///
/// The sink is cleared first, so a failed parse cannot leave the previous
/// image's tail behind. The first line (the track count) must be a number but
/// is otherwise ignored; blank lines are skipped; the pregap column and
/// anything after it is ignored. A leading UTF-8 BOM is skipped. The file
/// name is either a bare token or a double-quoted string, which may contain
/// spaces (`"Track 01.bin"`); the quotes are not part of the name.
pub fn parse<S: TrackSink>(data: &[u8], sink: &mut S) -> Result<(), Error<S::Error>> {
    sink.clear();
    let data = data.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(data);
    let mut lines = SplitLines::new(data);

    let header = lines.next().ok_or(Error::Parse)?;
    let _expected: u32 = parse_u32(header).map_err(|_| Error::Parse)?;

    while let Some(line) = lines.next() {
        let trimmed = trim(line);
        if trimmed.is_empty() {
            continue;
        }
        let mut fields = SplitWS::new(trimmed);
        let number = num(&mut fields)? as u8;
        let start_lba = num(&mut fields)?;
        let ctrl = num(&mut fields)? as u8;
        let sector_size = num(&mut fields)? as u16;
        let filename_raw = fields.next_name().ok_or(Error::Parse)?;
        let _ = fields.next(); // pregap, ignored

        let filename = strip_quotes(filename_raw).ok_or(Error::Parse)?;
        let t = TrackDesc {
            number,
            start_lba,
            ctrl,
            sector_size,
            sector_count: 0,
            filename,
            file_offset: 0,
            session: 0,
        };
        sink.push(&t).map_err(Error::Sink)?;
    }
    Ok(())
}

fn num<E>(f: &mut SplitWS) -> Result<u32, Error<E>> {
    parse_u32(f.next().ok_or(Error::Parse)?).map_err(|_| Error::Parse)
}

/// Convenience for host use: parse into a fresh table.
pub fn parse_vec(data: &[u8]) -> Result<Vec<TrackDesc, MAX_TRACKS>, Error<Full>> {
    let mut s = VecSink::new();
    parse(data, &mut s)?;
    Ok(s.0)
}

/// Copy of the name with every `"` dropped. Each byte is widened to a char,
/// so a byte >= 0x80 costs two bytes of the 32 -- `None` if it overflows.
fn strip_quotes(raw: &[u8]) -> Option<String<32>> {
    let mut buf: String<32> = String::new();
    for &b in raw {
        if b == b'"' {
            continue;
        }
        buf.push(b as char).ok()?;
    }
    Some(buf)
}

fn parse_u32(s: &[u8]) -> Result<u32, ()> {
    let mut n: u32 = 0;
    let mut any = false;
    for &b in s {
        if !b.is_ascii_digit() {
            if any {
                break;
            }
            return Err(());
        }
        n = n
            .checked_mul(10)
            .and_then(|x| x.checked_add((b - b'0') as u32))
            .ok_or(())?;
        any = true;
    }
    if !any {
        return Err(());
    }
    Ok(n)
}

#[inline(always)]
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

struct SplitLines<'a> {
    data: &'a [u8],
    i: usize,
}
impl<'a> SplitLines<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, i: 0 }
    }
    fn next(&mut self) -> Option<&'a [u8]> {
        if self.i >= self.data.len() {
            return None;
        }
        let start = self.i;
        while self.i < self.data.len() && self.data[self.i] != b'\n' {
            self.i += 1;
        }
        let line = &self.data[start..self.i];
        if self.i < self.data.len() {
            self.i += 1;
        }
        Some(line)
    }
}

struct SplitWS<'a> {
    data: &'a [u8],
    i: usize,
}
impl<'a> SplitWS<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, i: 0 }
    }
    fn next(&mut self) -> Option<&'a [u8]> {
        while self.i < self.data.len() && (self.data[self.i] == b' ' || self.data[self.i] == b'\t') {
            self.i += 1;
        }
        if self.i >= self.data.len() {
            return None;
        }
        let start = self.i;
        while self.i < self.data.len() && self.data[self.i] != b' ' && self.data[self.i] != b'\t' {
            self.i += 1;
        }
        Some(&self.data[start..self.i])
    }

    /// The file-name field. A name that starts with `"` runs to the closing
    /// quote (or the end of the line if there is none) and may hold blanks;
    /// the returned slice excludes both quotes. Otherwise it is an ordinary
    /// whitespace-delimited token (with any inner quotes still in it).
    fn next_name(&mut self) -> Option<&'a [u8]> {
        while self.i < self.data.len() && (self.data[self.i] == b' ' || self.data[self.i] == b'\t') {
            self.i += 1;
        }
        if self.data.get(self.i) != Some(&b'"') {
            return self.next();
        }
        let start = self.i + 1;
        let mut end = start;
        while end < self.data.len() && self.data[end] != b'"' {
            end += 1;
        }
        self.i = if end < self.data.len() { end + 1 } else { end };
        Some(&self.data[start..end])
    }
}
