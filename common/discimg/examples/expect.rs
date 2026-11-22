//! Predict what NetGD's firmware would answer to the gdprobe battery, in gdprobe's record format.
//!
//!     cargo run --example expect -- --image disc.gdi|disc.cdi [--img N] [--label netgd-expected]
//!                                   [--switch] [--empty-folder] [--missing-folder] [--no-ua] [--header] [--end]
//!                                   [--ip-fad N] [--cdread-flags 0x24] [--no-flags-test] [--raw-sectors]
//!                                   [--no-throughput] [--tp-batches 64] [--tp-sectors 16]
//!                                   [--no-seek] [--seek-n 16] [--seek-span 50000] [--no-reads] [--sw-only]
//!
//! What it does: loads the image the way `mcu/netgd` does (the same `discimg` parsers, the same
//! boot-area / session rules, GDI track sizes from the files) and answers each probe of tools/gdprobe
//! (`probes.c`) with the same `discimg::atapi` builders the firmware's `ide.rs` calls, wrapped the way
//! `handle_packet` wraps them: status / error / interrupt-reason registers (a CHECK CONDITION is status 0x11,
//! error = sense key << 4, as stock), REQ_MODE contents, REQ_ERROR (`F0`, the pending sense key and its
//! ASC/ASCQ), the swap window after `52 82` (no medium, becoming ready, then UNIT ATTENTION), the answers with
//! no medium (`atapi::not_ready_answer`), CD_READ checking the expected sector type in byte 1 of the CDB as
//! stock does (5/64/00 on a mismatch, LOG R134) and returning 2048 bytes per sector (no 2352-byte raw reads).
//!
//! Time is not simulated. The window's phases (`atapi::swap_phase`; GDEMU.INI open_time / detect_time, else
//! 500 + 250 ms) are written as the sequence
//! of SW_POLL state changes a poller that samples every phase sees (gdprobe polls every ~20 ms); when each
//! change happens (`t_ms`) is left `-`. A poller slower than a phase, or commands other than TUR inside the
//! window, are not modelled.
//!
//! Fields NetGD cannot predict are written as `-`: every `us=`, `t_ms=`, `polls=`, `changes=`, `poll=`,
//! `min_us=`, `max_us=`, `kib_s=` (compare.py skips a `-`). The record has the same line order and keys as
//! a real gdprobe run, so `compare.py stock.txt predicted.txt` lists exactly the places where NetGD
//! differs from the stock GDEMU. `tools/netgd-expect/expect_sd.py` runs this over a whole mksdcard card.

use discimg::atapi::{self, AudioState};
use discimg::{cdi, gdemu, gdi, TrackDesc, VecSink};
use std::fmt::Write as _;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const FAD_OFFSET: u32 = 150;
/// `data=` shows at most this many bytes (gdprobe's REC_DATA_MAX).
const REC_DATA_MAX: usize = 512;
/// What the firmware's `GDEMU_COMPAT_VERSION` is (ide.rs): the `52 00` reply.
const GDEMU_COMPAT_VERSION: (u8, u8, u8) = (5, 20, 0);

// ------------------------------------------------------------------ options

struct Opts {
    image: Option<PathBuf>,
    img: i32,
    label: String,
    switch: bool,
    empty_folder: bool,
    missing_folder: bool,
    ua: Option<bool>,
    header: bool,
    end: bool,
    ip_fad: u32,
    cdread_flags: u8,
    flags_test: bool,
    /// Predict the firmware built with feature `raw-sectors` (D10).
    raw_sectors: bool,
    throughput: bool,
    tp_batches: u32,
    tp_sectors: u32,
    seek: bool,
    seek_n: u32,
    seek_span: u32,
    reads: bool,
    sw_only: bool,
}

fn usage() -> ! {
    eprintln!(
        "usage: expect --image <disc.gdi|disc.cdi> [--img N] [--label L] [--switch] [--empty-folder]\n\
         \x20             [--missing-folder] [--no-ua]\n\
         \x20             [--header] [--end] [--ip-fad N] [--cdread-flags 0x24] [--no-flags-test] [--raw-sectors]\n\
         \x20             [--no-throughput] [--tp-batches N] [--tp-sectors N] [--no-seek] [--seek-n N]\n\
         \x20             [--seek-span N] [--no-reads] [--sw-only]"
    );
    std::process::exit(2)
}

fn num(s: &str) -> u32 {
    let r = if let Some(h) = s.strip_prefix("0x") { u32::from_str_radix(h, 16) } else { s.parse() };
    r.unwrap_or_else(|_| {
        eprintln!("expect: bad number '{}'", s);
        std::process::exit(2)
    })
}

fn parse_args() -> Opts {
    let mut o = Opts {
        image: None,
        img: 0,
        label: "netgd-expected".into(),
        switch: false,
        empty_folder: false,
        missing_folder: false,
        ua: None,
        header: false,
        end: false,
        ip_fad: 0,
        cdread_flags: 0x24,
        flags_test: true,
        raw_sectors: false,
        throughput: true,
        tp_batches: 64,
        tp_sectors: 16,
        seek: true,
        seek_n: 16,
        seek_span: 50000,
        reads: true,
        sw_only: false,
    };
    let mut a = std::env::args().skip(1);
    while let Some(k) = a.next() {
        let mut val = || a.next().unwrap_or_else(|| usage());
        match k.as_str() {
            "--image" => o.image = Some(PathBuf::from(val())),
            "--img" => o.img = num(&val()) as i32,
            "--label" => o.label = val(),
            "--switch" => o.switch = true,
            "--empty-folder" => o.empty_folder = true,
            "--missing-folder" => o.missing_folder = true,
            "--no-ua" => o.ua = Some(false),
            "--ua" => o.ua = Some(true),
            "--header" => o.header = true,
            "--end" => o.end = true,
            "--ip-fad" => o.ip_fad = num(&val()),
            "--cdread-flags" => o.cdread_flags = num(&val()) as u8,
            "--no-flags-test" => o.flags_test = false,
            "--raw-sectors" => o.raw_sectors = true,
            "--no-throughput" => o.throughput = false,
            "--tp-batches" => o.tp_batches = num(&val()),
            "--tp-sectors" => o.tp_sectors = num(&val()).clamp(1, 16),
            "--no-seek" => o.seek = false,
            "--seek-n" => o.seek_n = num(&val()),
            "--seek-span" => o.seek_span = num(&val()),
            "--no-reads" => o.reads = false,
            "--sw-only" => o.sw_only = true,
            _ => usage(),
        }
    }
    o
}

// ------------------------------------------------------------------ crc32 / hex

fn crc32(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for i in 0..256u32 {
            let mut c = i;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            t[i as usize] = c;
        }
        t
    });
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c = table[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    !c
}

fn hex(d: &[u8]) -> String {
    let mut s = String::with_capacity(d.len() * 2);
    for b in d {
        let _ = write!(s, "{:02x}", b);
    }
    s
}

// ------------------------------------------------------------------ the disc as the firmware sees it

struct Disc {
    tracks: Vec<TrackDesc>,
    is_cdi: bool,
    boot_area_lba: u32,
    sessions: u8,
    /// GDI: the directory of the track files. CDI: the image file itself.
    dir: PathBuf,
    cdi_file: Option<PathBuf>,
}

fn load_disc(path: &Path) -> Result<Disc, String> {
    let lower = path.to_string_lossy().to_ascii_lowercase();
    let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    if lower.ends_with(".gdi") {
        let data = std::fs::read(path).map_err(|e| format!("read {}: {}", path.display(), e))?;
        let parsed = gdi::parse_vec(&data).map_err(|e| format!("gdi parse error: {:?}", e))?;
        let mut tracks: Vec<TrackDesc> = parsed.iter().cloned().collect();
        // ide.rs / gdi.rs: sector_count = size of the track file / sector size.
        for t in tracks.iter_mut() {
            let size = std::fs::metadata(dir.join(t.filename.as_str())).map(|m| m.len()).unwrap_or(0);
            t.sector_count = (size / t.sector_size as u64) as u32;
        }
        // gdi.rs `parse`: HDA = first track numbered 3 or above, else 45000; one session.
        let boot = atapi::gdi_boot_area_lba(&tracks[..]);
        Ok(Disc { tracks, is_cdi: false, boot_area_lba: boot, sessions: 1, dir, cdi_file: None })
    } else if lower.ends_with(".cdi") {
        let mut f = File::open(path).map_err(|e| format!("open {}: {}", path.display(), e))?;
        let len = f.metadata().map_err(|e| e.to_string())?.len() as u32;
        let mut sink = VecSink::new();
        let info = cdi::parse_info(
            len,
            |off, buf| {
                f.seek(SeekFrom::Start(off as u64)).map_err(|_| ())?;
                f.read_exact(buf).map_err(|_| ())
            },
            &mut sink,
        )
        .map_err(|e| format!("cdi parse error: {:?}", e))?;
        let tracks: Vec<TrackDesc> = sink.0.iter().cloned().collect();
        let boot = atapi::cdi_boot_area_lba(&tracks[..], info.boot_track);
        Ok(Disc { tracks, is_cdi: true, boot_area_lba: boot, sessions: info.sessions, dir, cdi_file: Some(path.to_path_buf()) })
    } else {
        Err("unknown extension (want .gdi or .cdi)".into())
    }
}

impl Disc {
    fn track_for_lba(&self, lba: u32) -> Option<&TrackDesc> {
        atapi::track_for_lba(&self.tracks, lba)
    }

    /// `handle_cd_read` + `fill_cd_read_buf_*`: Some(count * 2048 bytes) or None = ILLEGAL REQUEST.
    fn cd_read(&self, fad: u32, count: u32) -> Option<Vec<u8>> {
        let lba = fad.saturating_sub(FAD_OFFSET);
        let t = self.track_for_lba(lba);
        let track_sectors = t.map(|t| t.sector_count).unwrap_or(0);
        let available = track_sectors.saturating_sub(lba.saturating_sub(t.map(|t| t.start_lba).unwrap_or(0)));
        let total = (count as usize).saturating_mul(2048);
        if total == 0 || count > available {
            return None;
        }
        let t = t?;
        // gdi.rs `Track::user_data_offset`: 2352 -> 16, 2336 -> 8, else 0.
        let uoff = match t.sector_size {
            2352 => 16u64,
            2336 => 8,
            _ => 0,
        };
        let path = match &self.cdi_file {
            Some(p) => p.clone(),
            None => self.dir.join(t.filename.as_str()),
        };
        let mut f = File::open(path).ok()?;
        let mut out = vec![0u8; total];
        for s in 0..count as u64 {
            let off = t.file_offset as u64 + (lba as u64 + s - t.start_lba as u64) * t.sector_size as u64 + uoff;
            f.seek(SeekFrom::Start(off)).ok()?;
            f.read_exact(&mut out[s as usize * 2048..(s as usize + 1) * 2048]).ok()?;
        }
        Some(out)
    }

    /// Feature `raw-sectors`: one whole 2,352-byte sector at `fad`, if the track stores 2,352 or 2,336 bytes.
    fn cd_read_raw(&self, fad: u32) -> Option<Vec<u8>> {
        let lba = fad.saturating_sub(FAD_OFFSET);
        let t = self.track_for_lba(lba)?;
        if !atapi::raw_from_stored(t.sector_size as u32) || lba - t.start_lba >= t.sector_count {
            return None;
        }
        let path = match &self.cdi_file {
            Some(p) => p.clone(),
            None => self.dir.join(t.filename.as_str()),
        };
        let mut f = File::open(path).ok()?;
        let mut stored = vec![0u8; t.sector_size as usize];
        f.seek(SeekFrom::Start(t.file_offset as u64 + (lba - t.start_lba) as u64 * t.sector_size as u64)).ok()?;
        f.read_exact(&mut stored).ok()?;
        let mut out = vec![0u8; atapi::RAW_SECTOR_BYTES];
        atapi::raw_sector(&stored, lba, &mut out);
        Some(out)
    }
}

// ------------------------------------------------------------------ record lines

/// One probe's result as the gdprobe runner records it.
struct Res {
    ok: bool,
    status: u8,
    err: u8,
    irr: u8,
    total: usize,
    data: Vec<u8>,
}

impl Res {
    /// A data-in (or no-data) command that completed: `complete(0, STATUS_IDLE)`, IRR 3.
    fn ok(data: &[u8]) -> Res {
        Res { ok: true, status: 0x50, err: 0, irr: 0x03, total: data.len(), data: data.to_vec() }
    }
    fn ok_big(total: usize, first: &[u8]) -> Res {
        Res { ok: true, status: 0x50, err: 0, irr: 0x03, total, data: first.to_vec() }
    }
    /// CHECK CONDITION with sense key `key` (`finish_check`: status 0x11, error = key << 4; D5).
    fn check(key: u8) -> Res {
        Res { ok: false, status: atapi::CHECK_STATUS, err: atapi::check_error(key), irr: 0x03, total: 0, data: Vec::new() }
    }
    /// The UNIT ATTENTION answer (key 6).
    fn ua() -> Res {
        Res::check(atapi::SENSE_UNIT_ATTENTION)
    }
    /// NOT READY (key 2: no medium, the swap window).
    fn not_ready() -> Res {
        Res::check(atapi::SENSE_NOT_READY)
    }
    /// ILLEGAL REQUEST (key 5: REQ_SES out of range, REQ_MODE with an unsupported window, CD_READ out of the track,
    /// a `52 82` to a folder number that does not exist).
    fn illegal() -> Res {
        Res::check(atapi::SENSE_ILLEGAL_REQUEST)
    }
}

struct Out {
    s: String,
    img: i32,
}

impl Out {
    fn probe(&mut self, name: &str, extra: Option<&str>, cdb: Option<&[u8; 12]>, r: &Res, show_data: bool) {
        let _ = write!(self.s, "img={} probe={}", self.img, name);
        if let Some(e) = extra {
            let _ = write!(self.s, " {}", e);
        }
        if let Some(c) = cdb {
            let _ = write!(self.s, " cdb={}", hex(c));
        }
        let _ = write!(
            self.s,
            " rc={} status=0x{:02x} err=0x{:02x} irr=0x{:02x} bytes={}",
            if r.ok { "ok" } else { "check" },
            r.status,
            r.err,
            r.irr,
            r.total
        );
        if show_data && !r.data.is_empty() {
            let n = r.data.len().min(REC_DATA_MAX);
            let _ = write!(self.s, " data={}", hex(&r.data[..n]));
        }
        self.s.push_str(" us=-\n");
    }

    fn event(&mut self, name: &str, fields: &str) {
        let _ = write!(self.s, "img={} probe={}", self.img, name);
        if !fields.is_empty() {
            let _ = write!(self.s, " {}", fields);
        }
        self.s.push('\n');
    }

    fn hash(&mut self, fad: u32, crc: u32) {
        let _ = writeln!(self.s, "img={} probe=HASH fad={} ssz=2048 crc={:08x}", self.img, fad, crc);
    }
}

// ------------------------------------------------------------------ CDB builders (probes.c)

fn cdb_simple(op: u8, b1: u8, b2: u8, alloc: u8) -> [u8; 12] {
    let mut c = [0u8; 12];
    c[0] = op;
    c[1] = b1;
    c[2] = b2;
    c[4] = alloc;
    c
}

fn cdb_be16(op: u8, b1: u8, alloc: u32) -> [u8; 12] {
    let mut c = [0u8; 12];
    c[0] = op;
    c[1] = b1;
    c[3] = (alloc >> 8) as u8;
    c[4] = alloc as u8;
    c
}

fn cdb_read(flags: u8, fad: u32, count: u32) -> [u8; 12] {
    let mut c = [0u8; 12];
    c[0] = 0x30;
    c[1] = flags;
    c[2] = (fad >> 16) as u8;
    c[3] = (fad >> 8) as u8;
    c[4] = fad as u8;
    c[8] = (count >> 16) as u8;
    c[9] = (count >> 8) as u8;
    c[10] = count as u8;
    c
}

fn cdb_mount(n: u32) -> [u8; 12] {
    let mut c = [0u8; 12];
    c[0] = 0x52;
    c[1] = 0x82;
    c[2] = n as u8;
    c[3] = (n >> 8) as u8;
    c
}

fn fad3(p: &[u8]) -> u32 {
    ((p[0] as u32) << 16) | ((p[1] as u32) << 8) | p[2] as u32
}

// ------------------------------------------------------------------ the firmware's answers

/// REQ_MODE (ide.rs): a window of the stock 32-byte block (`atapi::req_mode_reply`), else ILLEGAL REQUEST.
fn req_mode(c: &[u8; 12]) -> Res {
    match atapi::req_mode_reply(c[2], c[4]) {
        Some(r) => Res::ok(r),
        None => Res::illegal(),
    }
}

fn cut(data: &[u8], alloc: usize) -> Res {
    Res::ok(&data[..alloc.min(data.len())])
}

/// What the runner prints for the CD_READ at `fad` (`handle_cd_read`).
fn read_res(d: &Disc, flags: u8, fad: u32, count: u32) -> (Res, Option<Vec<u8>>) {
    // handle_cd_read: the expected sector type is checked first (D8, D9).
    let lba = fad.saturating_sub(FAD_OFFSET);
    if let Some(t) = atapi::track_for_lba(&d.tracks, lba) {
        if atapi::cd_read_type_mismatch(flags, d.is_cdi, t.ctrl) {
            return (Res::illegal(), None);
        }
    }
    match d.cd_read(fad, count) {
        Some(data) => (Res::ok_big(data.len(), &data[..data.len().min(REC_DATA_MAX)]), Some(data)),
        None => (Res::illegal(), None),
    }
}

fn hash_sectors(o: &mut Out, fad: u32, count: u32, data: &Option<Vec<u8>>) {
    if let Some(data) = data {
        for i in 0..count {
            let s = &data[i as usize * 2048..(i as usize + 1) * 2048];
            o.hash(fad + i, crc32(s));
        }
    }
}

// ------------------------------------------------------------------ the battery

fn battery(opt: &Opts, d: &Disc, o: &mut Out, ua: bool, switched: bool) {
    // warmup(): TUR until READY. A pending UNIT ATTENTION is answered by the first command and cleared.
    let tur = [0u8; 12];
    let req_err = cdb_simple(0x13, 0, 0, 10);
    if ua {
        o.probe("TUR_W0", None, Some(&tur), &Res::ua(), false);
        o.probe("REQ_ERROR_W0", None, Some(&req_err), &Res::ok(&atapi::req_error_reply(6, atapi::ASC_MEDIUM_CHANGED.0, atapi::ASC_MEDIUM_CHANGED.1)), true);
        o.probe("TUR_W1", None, Some(&tur), &Res::ok(&[]), false);
    } else {
        o.probe("TUR_W0", None, Some(&tur), &Res::ok(&[]), false);
    }
    o.event("WARMUP", "ready=1");

    // battery_info
    o.probe("TUR", None, Some(&tur), &Res::ok(&[]), false);

    // Nothing plays: the idle status and head (D2/D4) -- pause and the switch's park position after a
    // `52 82` (`switched`), standby at FAD 150 on a unit that has not switched since power-on (ide.rs
    // `IDLE_STATUS` / `SWITCHED`). CD_READ moves neither, in the firmware or here.
    let idle = if switched { atapi::DRIVE_PAUSE } else { atapi::DRIVE_STANDBY };
    let idle_fad = atapi::idle_fad(&d.tracks, d.is_cdi, switched);
    let (fad, track) = atapi::head_position(&d.tracks, AudioState::Standby, 0, idle_fad);
    // disc_format() in ide.rs: CDI -> CD-ROM XA, anything else -> GD-ROM; the repeat
    // nibble is F whenever nothing plays (req_stat_reply).
    let fmt = if d.is_cdi { atapi::DISC_FORMAT_CDROM_XA } else { atapi::DISC_FORMAT_GDROM };
    let stat = atapi::req_stat_reply(AudioState::Standby, idle, fmt, 0, fad, track);
    let c = cdb_simple(0x10, 0, 0, 10);
    o.probe("REQ_STAT", None, Some(&c), &cut(&stat, 10), true);

    for (name, c) in [
        ("REQ_MODE_0_10", cdb_simple(0x11, 0, 0, 10)),
        ("REQ_MODE_18_8", cdb_simple(0x11, 0, 18, 8)),
        ("REQ_MODE_0_32", cdb_simple(0x11, 0, 0, 32)),
    ] {
        o.probe(name, None, Some(&c), &req_mode(&c), true);
    }
    o.probe("REQ_ERROR", None, Some(&req_err), &Res::ok(&atapi::req_error_reply(0, 0, 0)), true);

    // TOC of both areas; the runner remembers the last valid lead-out and the first data track.
    let mut leadout = 0u32;
    let mut toc_ip = 0u32;
    for s in 0..2u8 {
        let c = cdb_be16(0x14, s, 408);
        let mut toc = [0u8; 408];
        atapi::build_toc(&d.tracks, d.is_cdi, d.boot_area_lba, s, &mut toc);
        o.probe(&format!("GET_TOC_{}", s), None, Some(&c), &Res::ok(&toc), true);
        let lo = fad3(&toc[405..408]);
        if toc[404] != 0xFF && lo != 0 && lo != 0xFF_FFFF {
            leadout = lo;
        }
        for t in 0..99 {
            let e = &toc[4 * t..4 * t + 4];
            let f = fad3(&e[1..4]);
            if e[0] != 0xFF && (e[0] & 0x40) != 0 && f >= 150 && f < 0x00F0_0000 {
                toc_ip = f;
                break;
            }
        }
    }

    // Sessions 0..3. The firmware answers an out-of-range one with ILLEGAL REQUEST (stock's 0x11 / 0x50).
    let mut ses_ip = 0u32;
    for s in 0..4u8 {
        let c = cdb_simple(0x15, 0, s, 6);
        let r = match atapi::req_ses_reply(&d.tracks, d.is_cdi, d.boot_area_lba, d.sessions, s, idle) {
            Some(rep) => {
                if s == 2 {
                    let f = fad3(&rep[3..6]);
                    if f >= 150 && f < 0x00F0_0000 {
                        ses_ip = f;
                    }
                }
                Res::ok(&rep)
            }
            None => Res::illegal(),
        };
        o.probe(&format!("REQ_SES_{}", s), None, Some(&c), &r, true);
    }

    // GET_SCD with no CD-DA (audio status 0x15) at the idle head, format = CDB byte 1 & 0x0F, cut to the
    // allocation length.
    for (name, fmt, alloc) in [("GET_SCD_0", 0u8, 100u32), ("GET_SCD_1", 1u8, 14u32)] {
        let c = cdb_be16(0x40, fmt, alloc);
        let mut rep = [0u8; 100];
        let len = atapi::get_scd_reply(fmt & 0x0F, AudioState::Standby, fad, track, &mut rep);
        o.probe(name, None, Some(&c), &cut(&rep[..len], alloc as usize), true);
    }

    gdemu_version(o);

    if !opt.reads {
        return;
    }

    // battery_reads
    let ip = if opt.ip_fad != 0 {
        opt.ip_fad
    } else if ses_ip != 0 {
        ses_ip
    } else if toc_ip != 0 {
        toc_ip
    } else {
        45150
    };

    let c = cdb_read(opt.cdread_flags, ip, 7);
    let (r, data) = read_res(d, opt.cdread_flags, ip, 7);
    o.probe("IP_READ", Some(&format!("fad={} count=7", ip)), Some(&c), &r, true);
    hash_sectors(o, ip, 7, &data);

    if opt.flags_test {
        // The sector type in byte 1 is checked (D8, D9); what reads is always 2048 bytes (no raw 2352, D10).
        for flags in [0x20u8, 0x24, 0x28, 0x10] {
            let c = cdb_read(flags, ip, 1);
            let (mut r, mut data) = read_res(d, flags, ip, 1);
            if opt.raw_sectors && data.is_some() && atapi::cd_read_wants_raw(flags) {
                if let Some(raw) = d.cd_read_raw(ip) {
                    r = Res::ok_big(raw.len(), &raw[..raw.len().min(REC_DATA_MAX)]);
                    data = Some(raw);
                }
            }
            let lba = ip.saturating_sub(FAD_OFFSET);
            let mismatch = atapi::track_for_lba(&d.tracks, lba)
                .map_or(false, |t| atapi::cd_read_type_mismatch(flags, d.is_cdi, t.ctrl));
            let (ssz, crc, sense) = match &data {
                Some(dd) => (dd.len().to_string(), format!("{:08x}", crc32(dd)), "none"),
                None if mismatch => ("-".to_string(), "-".to_string(), "5/64/00"), // illegal mode for this track
                None => ("-".to_string(), "-".to_string(), "5/00/00"), // ILLEGAL REQUEST, ASC 0 (stock's is not known)
            };
            let ex = format!("flags=0x{:02x} fad={} count=1 ssz={} crc={} sense={}", flags, ip, ssz, crc, sense);
            o.probe("CDREAD_FLAGS", Some(&ex), Some(&c), &r, false);
        }
    }

    if opt.throughput {
        let n = opt.tp_sectors;
        let (mut ok, mut bytes) = (0u32, 0usize);
        for b in 0..opt.tp_batches {
            let fad = ip + b * n;
            if leadout != 0 && fad + n > leadout {
                break;
            }
            let c = cdb_read(opt.cdread_flags, fad, n);
            let (r, data) = read_res(d, opt.cdread_flags, fad, n);
            o.probe(&format!("TP_{:03}", b), Some(&format!("fad={} count={}", fad, n)), Some(&c), &r, false);
            if r.ok {
                ok += 1;
                bytes += r.total;
                hash_sectors(o, fad, n, &data);
            }
        }
        o.event("TP_SUMMARY", &format!("ok={} bytes={} us=- min_us=- max_us=- kib_s=-", ok, bytes));
    }

    if opt.seek && opt.seek_n >= 2 {
        let n = opt.seek_n;
        let mut span = opt.seek_span;
        if leadout > ip + 2 && leadout - ip - 1 < span {
            span = leadout - ip - 1;
        }
        for i in 0..n {
            let idx = if i % 2 == 0 { i / 2 } else { n - 1 - i / 2 };
            let fad = ip + ((span as u64 * idx as u64) / (n as u64 - 1)) as u32;
            let c = cdb_read(opt.cdread_flags, fad, 1);
            let (r, data) = read_res(d, opt.cdread_flags, fad, 1);
            o.probe(&format!("SEEK_{:02}", i), Some(&format!("fad={}", fad)), Some(&c), &r, false);
            hash_sectors(o, fad, 1, &data);
        }
    }
}

/// `52 00`: bootloader 0.0.0 then the firmware version, little-endian words; no allocation length = all 8.
fn gdemu_version(o: &mut Out) {
    let mut c = [0u8; 12];
    c[0] = 0x52;
    let (ma, mi, pa) = GDEMU_COMPAT_VERSION;
    let mut ver = [0u8; 8];
    ver[4..].copy_from_slice(&gdemu::version_word(ma, mi, pa).to_le_bytes());
    o.probe("GDEMU_VERSION", None, Some(&c), &Res::ok(&ver), true);
}

/// The battery with no medium (an empty folder after its swap window): warmup gives up after eight TUR /
/// REQ_ERROR rounds, then the information commands as `atapi::not_ready_answer` sorts them, no reads.
fn battery_no_medium(o: &mut Out) {
    let tur = [0u8; 12];
    let req_err = cdb_simple(0x13, 0, 0, 10);
    let (asc, ascq) = atapi::ASC_NO_MEDIUM;
    let no_medium = atapi::req_error_reply(atapi::SENSE_NOT_READY, asc, ascq);
    for i in 0..8 {
        o.probe(&format!("TUR_W{}", i), None, Some(&tur), &Res::not_ready(), false);
        o.probe(&format!("REQ_ERROR_W{}", i), None, Some(&req_err), &Res::ok(&no_medium), true);
    }
    o.event("WARMUP", "ready=0");
    let answer = |c: &[u8; 12], served: Res| match atapi::not_ready_answer(c) {
        atapi::NotReadyAnswer::Serve => served,
        atapi::NotReadyAnswer::NotReady => Res::not_ready(),
        atapi::NotReadyAnswer::IllegalRequest => Res::illegal(),
    };
    o.probe("TUR", None, Some(&tur), &answer(&tur, Res::ok(&[])), false);
    let c = cdb_simple(0x10, 0, 0, 10);
    o.probe("REQ_STAT", None, Some(&c), &answer(&c, Res::ok(&[])), true);
    for (name, c) in [
        ("REQ_MODE_0_10", cdb_simple(0x11, 0, 0, 10)),
        ("REQ_MODE_18_8", cdb_simple(0x11, 0, 18, 8)),
        ("REQ_MODE_0_32", cdb_simple(0x11, 0, 0, 32)),
    ] {
        o.probe(name, None, Some(&c), &answer(&c, req_mode(&c)), true);
    }
    // The REQ_MODEs were served, which clears the sense REQ_STAT left.
    o.probe("REQ_ERROR", None, Some(&req_err), &Res::ok(&atapi::req_error_reply(0, 0, 0)), true);
    for s in 0..2u8 {
        let c = cdb_be16(0x14, s, 408);
        o.probe(&format!("GET_TOC_{}", s), None, Some(&c), &answer(&c, Res::ok(&[])), true);
    }
    for s in 0..4u8 {
        let c = cdb_simple(0x15, 0, s, 6);
        o.probe(&format!("REQ_SES_{}", s), None, Some(&c), &answer(&c, Res::ok(&[])), true);
    }
    for (name, fmt, alloc) in [("GET_SCD_0", 0u8, 100u32), ("GET_SCD_1", 1u8, 14u32)] {
        let c = cdb_be16(0x40, fmt, alloc);
        o.probe(name, None, Some(&c), &answer(&c, Res::ok(&[])), true);
    }
    gdemu_version(o);
}

/// What a `52 82 n` leaves behind.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    /// The folder has an image: it mounts.
    Image,
    /// The folder exists with nothing to mount: accepted, no medium (D12).
    Empty,
    /// No folder with that number and past 999: refused, the old image stays. (A missing number up to 999
    /// mounts folder 01 like stock, stock_20261122_09: expect_sd.py runs that as a switch to 01's image, or
    /// `--empty-folder` when 01 has none, `discimg::slot::mount_target`.)
    Missing,
}

fn sw_poll(o: &mut Out, r: &Res, sense: &str) {
    o.event(
        "SW_POLL",
        &format!("t_ms=- poll=- rc={} status=0x{:02x} err=0x{:02x} sense={}", if r.ok { "ok" } else { "check" }, r.status, r.err, sense),
    );
}

/// `switch_to`: `52 82 n`, then TUR / REQ_ERROR polls, one line per change of state. With an image or an
/// empty folder the swap window plays out (`atapi::swap_phase`): no medium, becoming ready, then UNIT
/// ATTENTION and READY -- or no medium again, until gdprobe's switch timeout. A refused mount leaves the old
/// image (and its already-consumed UNIT ATTENTION) in place.
fn switch_lines(o: &mut Out, n: u32, target: Target) {
    let c = cdb_mount(n);
    let fmt = |(asc, ascq): (u8, u8), key: u8| format!("{}/{:02x}/{:02x}", key, asc, ascq);
    let nr = atapi::SENSE_NOT_READY;
    let ready = match target {
        Target::Missing => {
            o.probe("SW_CMD", Some(&format!("n={}", n)), Some(&c), &Res::illegal(), false);
            sw_poll(o, &Res::ok(&[]), "none");
            true
        }
        Target::Image | Target::Empty => {
            o.probe("SW_CMD", Some(&format!("n={}", n)), Some(&c), &Res::ok(&[]), false);
            for phase in [atapi::SwapPhase::Open, atapi::SwapPhase::Detect] {
                sw_poll(o, &Res::not_ready(), &fmt(atapi::not_ready_asc(phase), nr));
            }
            if target == Target::Image {
                sw_poll(o, &Res::ua(), &fmt(atapi::ASC_MEDIUM_CHANGED, atapi::SENSE_UNIT_ATTENTION));
                sw_poll(o, &Res::ok(&[]), "none");
                true
            } else {
                sw_poll(o, &Res::not_ready(), &fmt(atapi::not_ready_asc(atapi::SwapPhase::Done), nr));
                false
            }
        }
    };
    o.event("SW_RESULT", &format!("n={} rc={} t_ms=- polls=- changes=-", n, if ready { "ok" } else { "timeout" }));
}

fn main() {
    let opt = parse_args();
    let mut o = Out { s: String::new(), img: opt.img };

    let disc = match &opt.image {
        Some(p) => match load_disc(p) {
            Ok(d) => Some(d),
            Err(e) => {
                eprintln!("expect: {}", e);
                std::process::exit(1);
            }
        },
        None => {
            if !opt.empty_folder && !opt.missing_folder {
                usage();
            }
            None
        }
    };

    if opt.header {
        o.s.push_str(&format!(
            "# gdprobe version=1 kos=- label={} stamp=predicted seq=0 dcload=0\n# predicted by common/discimg/examples/expect.rs (NetGD firmware logic; timings are '-')\n",
            opt.label
        ));
    }
    // --empty-folder: the drive has no medium afterwards, whatever --image says. --missing-folder (a number
    // past 999 with no folder): the previously mounted --image stays (none = no medium).
    let disc = if opt.empty_folder { None } else { disc };
    if opt.switch {
        let target = if opt.empty_folder {
            Target::Empty
        } else if opt.missing_folder {
            Target::Missing
        } else {
            Target::Image
        };
        switch_lines(&mut o, opt.img as u32, target);
    }
    if !opt.sw_only {
        match &disc {
            Some(d) => {
                // A switch consumes the UNIT ATTENTION in its polls; a bare run on a freshly booted unit sees it.
                let ua = opt.ua.unwrap_or(!opt.switch);
                // --switch: the battery runs after a switch (pause, park position); without it, on a unit
                // fresh from power-on (standby, FAD 150). A refused switch (--missing-folder) keeps the old
                // image, which expect_sd.py only does after an earlier switch.
                battery(&opt, d, &mut o, ua, opt.switch);
            }
            None => battery_no_medium(&mut o),
        }
    }
    if opt.end {
        o.event("END", "abort=0");
    }
    print!("{}", o.s);
}
