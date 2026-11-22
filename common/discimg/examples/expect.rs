//! Predict what NetGD's firmware would answer to the gdprobe battery, in gdprobe's record format.
//!
//!     cargo run --example expect -- --image disc.gdi|disc.cdi [--img N] [--label netgd-expected]
//!                                   [--switch] [--empty-folder] [--missing-folder] [--no-ua] [--header] [--end]
//!                                   [--ip-fad N] [--cdread-flags 0x24] [--no-flags-test] [--raw-sectors]
//!                                   [--no-throughput] [--tp-batches 64] [--tp-sectors 16]
//!                                   [--no-seek] [--seek-n 16] [--seek-span 50000] [--no-reads] [--sw-only]
//!                                   [--regpoll] [--devreset MS] [--reswitch] [--devreset-only] [--devreset-cmd MS]
//!                                   [--old-bitstream]
//!                                   [--window OPEN,DETECT]
//!                                   [--head-probe] [--head-flags 0x20] [--head-counts 1,2,7,16]
//!                                   [--head-late-ms 300] [--head-idle-ms 2000] [--no-head-scd]
//!                                   [--gap-probe] [--gap-flags 0x20] [--gap-late-ms 300] [--no-gap-scd]
//!                                   [--gap-track-end N] [--gap-leadout 0,1,100] [--gap-low 0,100] [--no-gap-count0]
//!                                   [--gap-crc] [--pregap-probe] [--pregap-low L] [--pregap-offs L] [--pregap-cross N]
//!                                   [--risky-cross inline|off] [--ctl-probe]
//!
//! What it does: loads the image the way `mcu/netgd` does (the same `discimg` parsers, the same
//! boot-area / session rules, GDI track sizes from the files) and answers each probe of tools/gdprobe
//! (`probes.c`) with the same `discimg::atapi` builders the firmware's `ide.rs` calls, wrapped the way
//! `handle_packet` wraps them: status / error / interrupt-reason registers (a CHECK CONDITION is status 0x11,
//! error = sense key << 4, as stock), REQ_MODE contents, REQ_ERROR (`F0`, the pending sense key and its
//! ASC/ASCQ), the swap window after `52 82` (no medium, becoming ready, then UNIT ATTENTION), the answers with
//! no medium (`atapi::not_ready_answer`), CD_READ checking the expected sector type in byte 1 of the CDB against
//! the track's mode as stock does (5/64/00 on a mismatch, LOG R134, R144) and returning 2048 bytes per data sector
//! (2352 for 0x10 only with --raw-sectors) and the whole 2352-byte PCM sector of an audio track (R144).
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
//!
//! `--head-probe` adds gdprobe seq 21's head-position cases after the battery (`head_probe` in probes.c: `HD_START`,
//! then `HD_<case>` / `HD_<case>_POS` lines): REQ_STAT, GET_SCD format 1 and the late REQ_STAT at the head where
//! NetGD's CD_READ left it (`atapi::read_head_start` / `read_end_fad`, `atapi::head_position`). Time is not
//! simulated: the late REQ_STAT and the one after `IDLE` equal the first. `--gap-probe` adds seq 22's reads at the
//! TOC's edges (`gap_probe`: `GP_START`, the `GP_SCAN_R` binary search for the end of track 1, the lead-out, the GDI's
//! gap between the areas, FADs below 150, `GP_CNT0_SKIP`) the same way, with the extra `ctl=` / `scd_ctl=` /
//! `scd_rel=` fields. `--gap-crc` / `--pregap-probe` add seq 23's CRCs on those read lines and its pregap cases
//! (`GP_PG_START`, `PGL_*`, `GP_PGB` + `PG<t>_*`, `XPRE<t>` / `XGAP<t>`, `GP_DSCAN_R` / `GP_DSCAN` + `XEND`, `XLD`, `XLO`);
//! `--risky-cross off` / `--ctl-probe` seq 24's skipped risky reads and off-track CTRL cases (`GP_CTL_REGION`,
//! `CTLF_*` / `CTLR_*`, `GP_CTL`).

use discimg::atapi::{self, AudioState};
use discimg::{cdi, gdemu, gdi, TrackDesc, VecSink};
use std::fmt::Write as _;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const FAD_OFFSET: u32 = 150;
/// `data=` shows at most this many bytes (gdprobe's REC_DATA_MAX).
const REC_DATA_MAX: usize = 512;
/// gdprobe's read buffer (probes.c `BUFSZ`): seq 23's `crc=` covers at most this much.
const GDPROBE_BUFSZ: usize = 40_000;
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
    /// Predict the firmware built with feature `form2-sectors` (D14).
    form2_sectors: bool,
    /// gdprobe's `cdbst=` / `secnr=` state carried over from the previous section (`expect_sd.py`):
    /// was the last command before this section a CHECK, and the SECNR byte at rest before it.
    prev_check: bool,
    prev_secnr: Option<u8>,
    /// The ERROR register carried over (`expect_sd.py`): stock and NetGD keep the last CHECK's value through the
    /// commands that succeed (sticky, fix/head-pos-sticky-err); it shows in the register polls.
    prev_err: u8,
    /// Where the last CD_READ since the mount left the head (`atapi::read_end_fad`), None = nothing since the mount.
    prev_head: Option<u32>,
    /// The track number REQ_STAT reported last from on a track (`# expect-state tno=`), 0 = none.
    prev_tno: u8,
    throughput: bool,
    tp_batches: u32,
    tp_sectors: u32,
    seek: bool,
    seek_n: u32,
    seek_span: u32,
    reads: bool,
    sw_only: bool,
    /// gdprobe `swap_regpoll=1`: the command-free register poll after the switch (R154).
    regpoll: bool,
    /// gdprobe `swap_devreset`: `52 82 n` again for the folder just switched to (a same-folder re-switch), and a
    /// DEVICE RESET this many ms into the register poll (probe names get the prefix `R<ms>_`, and REQ_STAT /
    /// REQ_ERROR follow the polls). Use with --switch.
    devreset_ms: Option<u32>,
    /// gdprobe seq 17 `swap_reswitch`: `52 82 n` again with no reset (`RS_`). Replaces --switch.
    reswitch: bool,
    /// gdprobe seq 17 `devreset_only`: a DEVICE RESET with no switch command (`DR_`). Replaces --switch.
    devreset_only: bool,
    /// gdprobe seq 20 `devreset_cmd`: a DEVICE RESET, commands this many ms later, then TUR polls until READY
    /// (`DC<ms>_`). Replaces --switch.
    devreset_cmd_ms: Option<u32>,
    /// The bitstream keeps MCU-written ERROR bits 0-1 (feat/fpga-regs) and no longer clears ERROR on the PACKET
    /// write (fix/head-pos-sticky-err), so DEVICE RESET's ERROR 0x01 shows and ERROR stays sticky.
    /// `--old-bitstream` predicts a bitstream before both: 0x00 after DEVICE RESET, 0x00 after every packet command
    /// that succeeds. (feat/fpga-regs alone -- 0x01, cleared on PACKET -- is not modelled.)
    err_lo: bool,
    /// The swap window's open / detect lengths (GDEMU.INI open_time / detect_time; the probe card's 150 / 150).
    /// The re-detect after DEVICE RESET lasts detect_time. The register poll assumes the window (and the
    /// re-detect) is over before gdprobe's polls start (1,000 ms, or reset + 300 ms).
    window: (u32, u32),
    /// gdprobe seq 21 `head_probe=1`: the head-position cases after the battery.
    head_probe: bool,
    head_flags: u8,
    head_counts: Vec<u32>,
    head_late_ms: u32,
    head_idle_ms: u32,
    head_scd: bool,
    /// gdprobe seq 22 `gap_probe=1`: reads at the TOC's edges after the battery (and head_probe).
    gap_probe: bool,
    gap_flags: u8,
    gap_late_ms: u32,
    gap_scd: bool,
    gap_track_end: u32,
    gap_leadout: Vec<u32>,
    gap_low: Vec<u32>,
    gap_count0: bool,
    /// gdprobe seq 23 `gap_crc=1`: `crc=` / `scrc=` / `stored=` on every GP_ read line.
    gap_crc: bool,
    /// gdprobe seq 23 `pregap_probe=1`: pregap contents and reads across boundaries.
    pregap_probe: bool,
    pregap_low: Vec<u32>,
    pregap_offs: Vec<i64>,
    pregap_cross: u32,
    /// gdprobe seq 24 `risky_cross`: true = inline (seq 23), false = off (the reads from data past its end are
    /// skipped, `XGAP<t>` only when its first sector does not read). `end` is not modelled.
    risky_inline: bool,
    /// gdprobe seq 24 `ctl_probe=1`: the off-track CTRL cases.
    ctl_probe: bool,
}

fn usage() -> ! {
    eprintln!(
        "usage: expect --image <disc.gdi|disc.cdi> [--img N] [--label L] [--switch] [--empty-folder]\n\
         \x20             [--missing-folder] [--no-ua]\n\
         \x20             [--header] [--end] [--ip-fad N] [--cdread-flags 0x24] [--no-flags-test] [--raw-sectors]\n\
         \x20             [--no-throughput] [--tp-batches N] [--tp-sectors N] [--no-seek] [--seek-n N]\n\
         \x20             [--seek-span N] [--no-reads] [--sw-only]
                       [--regpoll] [--devreset MS] [--reswitch] [--devreset-only] [--devreset-cmd MS]
                       [--old-bitstream] [--window OPEN,DETECT]
                       [--head-probe] [--head-flags 0x20] [--head-counts 1,2,7,16]
                       [--head-late-ms 300] [--head-idle-ms 2000] [--no-head-scd]
                       [--gap-probe] [--gap-flags 0x20] [--gap-late-ms 300] [--no-gap-scd] [--gap-track-end N]
                       [--gap-leadout 0,1,100] [--gap-low 0,100] [--no-gap-count0]
                       [--gap-crc] [--pregap-probe] [--pregap-low 0,1,100,149]
                       [--pregap-offs -156,-151,-150,-149,-100,-2,-1,0] [--pregap-cross 16]
                       [--risky-cross inline|off] [--ctl-probe]"
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
        form2_sectors: false,
        prev_check: false,
        prev_secnr: None,
        prev_err: 0,
        prev_head: None,
        prev_tno: 0,
        throughput: true,
        tp_batches: 64,
        tp_sectors: 16,
        seek: true,
        seek_n: 16,
        seek_span: 50000,
        reads: true,
        sw_only: false,
        regpoll: false,
        devreset_ms: None,
        reswitch: false,
        devreset_only: false,
        devreset_cmd_ms: None,
        err_lo: true,
        window: (150, 150),
        head_probe: false,
        head_flags: 0x20,
        head_counts: vec![1, 2, 7, 16],
        head_late_ms: 300,
        head_idle_ms: 2000,
        head_scd: true,
        gap_probe: false,
        gap_flags: 0x20,
        gap_late_ms: 300,
        gap_scd: true,
        gap_track_end: 0,
        gap_leadout: vec![0, 1, 100],
        gap_low: vec![0, 100],
        gap_count0: true,
        gap_crc: false,
        pregap_probe: false,
        pregap_low: vec![0, 1, 100, 149],
        pregap_offs: vec![-156, -151, -150, -149, -100, -2, -1, 0],
        pregap_cross: 16,
        risky_inline: true,
        ctl_probe: false,
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
            "--form2-sectors" => o.form2_sectors = true,
            "--prev-check" => o.prev_check = num(&val()) != 0,
            "--prev-secnr" => o.prev_secnr = Some(num(&val()) as u8),
            "--prev-err" => o.prev_err = num(&val()) as u8,
            "--prev-head" => o.prev_head = Some(num(&val())),
            "--prev-tno" => o.prev_tno = num(&val()) as u8,
            "--no-throughput" => o.throughput = false,
            "--tp-batches" => o.tp_batches = num(&val()),
            "--tp-sectors" => o.tp_sectors = num(&val()).clamp(1, 16),
            "--no-seek" => o.seek = false,
            "--seek-n" => o.seek_n = num(&val()),
            "--seek-span" => o.seek_span = num(&val()),
            "--no-reads" => o.reads = false,
            "--sw-only" => o.sw_only = true,
            "--regpoll" => o.regpoll = true,
            "--devreset" => o.devreset_ms = Some(num(&val())),
            "--reswitch" => o.reswitch = true,
            "--devreset-only" => o.devreset_only = true,
            "--devreset-cmd" => o.devreset_cmd_ms = Some(num(&val())),
            "--old-bitstream" => o.err_lo = false,
            "--head-probe" => o.head_probe = true,
            // gdprobe cfg.c: bit 0 forced to 0.
            "--head-flags" => o.head_flags = (num(&val()) & 0xFE) as u8,
            "--head-counts" => o.head_counts = val().split(',').map(|x| num(x.trim()).clamp(1, 16)).collect(),
            "--head-late-ms" => o.head_late_ms = num(&val()),
            "--head-idle-ms" => o.head_idle_ms = num(&val()),
            "--no-head-scd" => o.head_scd = false,
            "--gap-probe" => o.gap_probe = true,
            "--gap-flags" => o.gap_flags = (num(&val()) & 0xFE) as u8,
            "--gap-late-ms" => o.gap_late_ms = num(&val()),
            "--no-gap-scd" => o.gap_scd = false,
            "--gap-track-end" => o.gap_track_end = num(&val()),
            "--gap-leadout" => o.gap_leadout = val().split(',').map(|x| num(x.trim())).collect(),
            "--gap-low" => o.gap_low = val().split(',').map(|x| num(x.trim())).collect(),
            "--no-gap-count0" => o.gap_count0 = false,
            "--gap-crc" => o.gap_crc = true,
            "--pregap-probe" => o.pregap_probe = true,
            "--pregap-low" => o.pregap_low = val().split(',').map(|x| num(x.trim()).min(149)).collect(),
            "--pregap-offs" => {
                o.pregap_offs = val()
                    .split(',')
                    .map(|x| {
                        let x = x.trim();
                        match x.strip_prefix('-') {
                            Some(v) => -(num(v) as i64),
                            None => num(x) as i64,
                        }
                    })
                    .collect()
            }
            "--pregap-cross" => o.pregap_cross = num(&val()).clamp(2, 16),
            "--risky-cross" => match val().as_str() {
                "inline" => o.risky_inline = true,
                "off" => o.risky_inline = false,
                _ => usage(),
            },
            "--ctl-probe" => o.ctl_probe = true,
            "--window" => {
                let v = val();
                let (a, b) = v.split_once(',').unwrap_or_else(|| usage());
                o.window = (num(a), num(b));
            }
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
    /// Feature `form2-sectors` (set from the options after loading).
    form2_sectors: bool,
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
        Ok(Disc { tracks, is_cdi: false, boot_area_lba: boot, sessions: 1, dir, cdi_file: None, form2_sectors: false })
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
        Ok(Disc { tracks, is_cdi: true, boot_area_lba: boot, sessions: info.sessions, dir, cdi_file: Some(path.to_path_buf()), form2_sectors: false })
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
        self.cd_read_raw_n(fad, 1)
    }

    /// `count` whole 2,352-byte sectors from `fad` (an audio track's PCM, or a raw data read), or None
    /// = ILLEGAL REQUEST (the track does not store them, or the request runs past it).
    /// Feature `form2-sectors` (`form2_request` + staging): `count` sectors at `fad` as 2,324-byte
    /// Form 2 data, if they are all Form 2 and fit one staging round (5 sectors of a mapped 2,336-byte
    /// track, `stage_sectors_for`); None = the usual 2,048-byte read.
    fn cd_read_form2(&self, fad: u32, count: u32) -> Option<Vec<u8>> {
        let lba = fad.saturating_sub(FAD_OFFSET);
        let t = self.track_for_lba(lba)?;
        let available = t.sector_count.saturating_sub(lba - t.start_lba);
        if count == 0 || count > 5 || count > available {
            return None;
        }
        let path = match &self.cdi_file {
            Some(p) => p.clone(),
            None => self.dir.join(t.filename.as_str()),
        };
        let mut f = File::open(path).ok()?;
        let mut stored = vec![0u8; t.sector_size as usize];
        let mut out = Vec::with_capacity(atapi::FORM2_SECTOR_BYTES * count as usize);
        for s in 0..count {
            let at = t.file_offset as u64 + (lba + s - t.start_lba) as u64 * t.sector_size as u64;
            f.seek(SeekFrom::Start(at)).ok()?;
            f.read_exact(&mut stored).ok()?;
            if !atapi::stored_is_form2(&stored) {
                return None;
            }
            out.extend_from_slice(&stored[atapi::FORM2_DATA_OFFSET..atapi::FORM2_DATA_OFFSET + atapi::FORM2_SECTOR_BYTES]);
        }
        Some(out)
    }

    fn cd_read_raw_n(&self, fad: u32, count: u32) -> Option<Vec<u8>> {
        let lba = fad.saturating_sub(FAD_OFFSET);
        let t = self.track_for_lba(lba)?;
        let available = t.sector_count.saturating_sub(lba - t.start_lba);
        if !atapi::raw_from_stored(t.sector_size as u32) || count == 0 || count > available {
            return None;
        }
        let path = match &self.cdi_file {
            Some(p) => p.clone(),
            None => self.dir.join(t.filename.as_str()),
        };
        let mut f = File::open(path).ok()?;
        let mut stored = vec![0u8; t.sector_size as usize];
        let mut out = vec![0u8; atapi::RAW_SECTOR_BYTES * count as usize];
        for s in 0..count {
            let at = t.file_offset as u64 + (lba + s - t.start_lba) as u64 * t.sector_size as u64;
            f.seek(SeekFrom::Start(at)).ok()?;
            f.read_exact(&mut stored).ok()?;
            let o = s as usize * atapi::RAW_SECTOR_BYTES;
            atapi::raw_sector(&stored, lba + s, &mut out[o..o + atapi::RAW_SECTOR_BYTES]);
        }
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
    /// The last command's final status was CHECK (0x11). gdprobe's `cdbst=` is the ALTSTATUS in a command's
    /// own CDB phase, which keeps the previous command's DRDY: 0x18 after a CHECK, else 0x58 (R147; NetGD's
    /// FPGA keeps the last status the MCU wrote, so the same).
    last_check: bool,
    /// The SECNR byte (gdprobe `secnr=`, read at completion), None = not known (the boot disc's).
    secnr: Option<u8>,
    /// Probe-name prefix (gdprobe's `R<ms>_` for a DEVICE RESET switch), on probe and event lines.
    pfx: String,
    /// The ERROR register as a register poll reads it: the last CHECK's value (sticky); with `old_bs` 0 after
    /// any packet command that succeeds (that bitstream clears it on the PACKET write).
    err_reg: u8,
    old_bs: bool,
    /// Where the last CD_READ since the mount left the head, None = nothing since the mount (ide.rs `LAST_READ_FAD`).
    head: Option<u32>,
    /// The track number the last REQ_STAT / GET_SCD reported from on a track, 0 = none (ide.rs `HEAD_TNO`).
    tno: u8,
}

impl Out {
    fn probe(&mut self, name: &str, extra: Option<&str>, cdb: Option<&[u8; 12]>, r: &Res, show_data: bool) {
        let _ = write!(self.s, "img={} probe={}{}", self.img, self.pfx, name);
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
        let _ = write!(self.s, " cdbst=0x{:02x}", if self.last_check { 0x18 } else { 0x58 });
        if let Some(v) = self.secnr {
            let _ = write!(self.s, " secnr=0x{:02x}", v);
        }
        self.last_check = !r.ok;
        self.packet_done(r.ok, r.err);
        if show_data && !r.data.is_empty() {
            let n = r.data.len().min(REC_DATA_MAX);
            let _ = write!(self.s, " data={}", hex(&r.data[..n]));
        }
        self.s.push_str(" us=-\n");
    }

    /// A packet command ended: a CHECK writes its ERROR; a success writes nothing (ide.rs `set_error`), but the
    /// old bitstream has already cleared ERROR on the PACKET write.
    fn packet_done(&mut self, ok: bool, err: u8) {
        if !ok {
            self.err_reg = err;
        } else if self.old_bs {
            self.err_reg = 0;
        }
    }

    /// A CD_READ of `count` from `fad` (ide.rs `handle_cd_read`): any read with a sector moves the head to its start
    /// (`atapi::read_head_start`, refused or not), past the last sector when the read went through
    /// (`atapi::read_end_fad`; gdprobe reads it all).
    fn note_read(&mut self, _tracks: &[TrackDesc], ok: bool, fad: u32, count: u32) {
        if let Some(start) = atapi::read_head_start(fad, count) {
            self.head = Some(if ok { atapi::read_end_fad(fad, count) } else { start });
        }
    }

    /// REQ_STAT / GET_SCD's head with nothing playing (ide.rs `head_now`): the track it reports, remembering the
    /// number while the head is on a track (`tno`; stock keeps it off every track, seq 23).
    fn head_at<'t>(&mut self, tracks: &'t [TrackDesc], fad: u32) -> (u32, Option<&'t TrackDesc>) {
        let (fad, track) = atapi::head_position(tracks, AudioState::Standby, 0, fad, self.tno);
        if let Some(t) = track {
            if atapi::head_on(t, fad) {
                self.tno = t.number;
            }
        }
        (fad, track)
    }

    fn event(&mut self, name: &str, fields: &str) {
        let _ = write!(self.s, "img={} probe={}{}", self.img, self.pfx, name);
        if !fields.is_empty() {
            let _ = write!(self.s, " {}", fields);
        }
        self.s.push('\n');
    }

    fn hash(&mut self, fad: u32, ssz: usize, crc: u32) {
        let _ = writeln!(self.s, "img={} probe=HASH fad={} ssz={} crc={:08x}", self.img, fad, ssz, crc);
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
    // From a track's pregap (`atapi::pregap_read`): zero sectors, then the track's own if the request runs on.
    // The zero sectors take `atapi::pregap_zero_bytes` (2,352 B on an audio track, 2,324 on a CDI Mode 2 track
    // with --form2-sectors, else 2,048), the track's own sectors their usual size (2,324 only if all Form 2).
    // Else never a start below 150.
    if let Some((t, zeros)) = atapi::pregap_read(&d.tracks, fad, count) {
        if atapi::cd_read_type_mismatch(flags, t.mode) || atapi::cd_read_unservable(t.mode, t.sector_size as u32) {
            return (Res::illegal(), None);
        }
        let raw = atapi::cd_read_raw_out(flags, t.mode, t.sector_size as u32, false);
        let ssz = atapi::pregap_zero_bytes(flags, t.mode, t.sector_size as u32, raw, d.form2_sectors);
        let mut data = vec![0u8; ssz * zeros as usize];
        let rest = count - zeros;
        if rest > 0 {
            let tf = t.start_lba + FAD_OFFSET;
            let form2 = if !raw && d.form2_sectors && atapi::cd_read_may_be_form2(flags, t.mode, t.sector_size as u32) {
                d.cd_read_form2(tf, rest)
            } else {
                None
            };
            match form2.or_else(|| if raw { d.cd_read_raw_n(tf, rest) } else { d.cd_read(tf, rest) }) {
                Some(m) => data.extend_from_slice(&m),
                None => return (Res::illegal(), None),
            }
        }
        return (Res::ok_big(data.len(), &data[..data.len().min(REC_DATA_MAX)]), Some(data));
    }
    if fad < FAD_OFFSET {
        return (Res::illegal(), None);
    }
    // handle_cd_read: the expected sector type is checked first, by the start's track (`atapi::track_of_start`:
    // the next track for a start in its pregap; D8, D9, R144). A pregap start `pregap_read` does not serve (it
    // runs past the track's end) is refused.
    if let Some(t) = atapi::track_of_start(&d.tracks, fad) {
        if atapi::cd_read_type_mismatch(flags, t.mode) || t.start_lba + FAD_OFFSET > fad {
            return (Res::illegal(), None);
        }
        // An audio track not stored 2,352 bytes a sector: ILLEGAL REQUEST.
        if atapi::cd_read_unservable(t.mode, t.sector_size as u32) {
            return (Res::illegal(), None);
        }
        // An audio track sends its 2,352-byte PCM sectors as stored (R144),
        // in every build (`cd_read_raw_out` with raw_data = false).
        if atapi::cd_read_raw_out(flags, t.mode, t.sector_size as u32, false) {
            return match d.cd_read_raw_n(fad, count) {
                Some(data) => (Res::ok_big(data.len(), &data[..data.len().min(REC_DATA_MAX)]), Some(data)),
                None => (Res::illegal(), None),
            };
        }
        if d.form2_sectors && atapi::cd_read_may_be_form2(flags, t.mode, t.sector_size as u32) {
            if let Some(data) = d.cd_read_form2(fad, count) {
                return (Res::ok_big(data.len(), &data[..data.len().min(REC_DATA_MAX)]), Some(data));
            }
        }
    }
    match d.cd_read(fad, count) {
        Some(data) => (Res::ok_big(data.len(), &data[..data.len().min(REC_DATA_MAX)]), Some(data)),
        None => (Res::illegal(), None),
    }
}

fn hash_sectors(o: &mut Out, fad: u32, count: u32, data: &Option<Vec<u8>>) {
    if let Some(data) = data {
        // 2,048-byte payloads, or whole 2,352-byte sectors (audio).
        let ssz = data.len() / count.max(1) as usize;
        for i in 0..count {
            let s = &data[i as usize * ssz..(i as usize + 1) * ssz];
            o.hash(fad + i, ssz, crc32(s));
        }
    }
}

// ------------------------------------------------------------------ the battery

/// What gdprobe's battery remembers from the TOC for seq 21's cases (probes.c `toc_ip` / `toc_ip_end` /
/// `toc_audio` / `toc_audio_end`): the first data track of the last area that has one and the first audio track of
/// the first area that has one, each with its end (the next track's start in that area, else the area's lead-out;
/// 0 = unknown).
#[derive(Default, Clone, Copy)]
struct HeadToc {
    ip: u32,
    ip_end: u32,
    audio: u32,
    audio_end: u32,
}

/// probes.c `toc_track_end`: the start of the next valid entry after `t` that lies past it, else the area's
/// lead-out if that lies past it, else 0.
fn toc_track_end(toc: &[u8; 408], t: usize, area_lo: u32) -> u32 {
    let f = fad3(&toc[4 * t + 1..4 * t + 4]);
    for u in t + 1..99 {
        let e = &toc[4 * u..4 * u + 4];
        let g = fad3(&e[1..4]);
        if e[0] == 0xFF || g < 150 || g >= 0x00F0_0000 {
            continue;
        }
        if g > f {
            return g;
        }
    }
    if area_lo > f {
        area_lo
    } else {
        0
    }
}

fn battery(opt: &Opts, d: &Disc, o: &mut Out, ua: bool, switched: bool) -> HeadToc {
    // warmup(): TUR until READY. A pending UNIT ATTENTION is answered by the first command and cleared.
    let tur = [0u8; 12];
    let req_err = cdb_simple(0x13, 0, 0, 10);
    // SECNR at rest (ide.rs `rest_secnr` / `secnr(idle_status())`): the disc format over the idle status --
    // pause after a switch, standby on a unit fresh from power-on. Nothing in the battery changes it.
    o.secnr = Some(atapi::disc_format(&d.tracks, d.is_cdi) | if switched { atapi::DRIVE_PAUSE } else { atapi::DRIVE_STANDBY });
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
    // `IDLE_STATUS` / `SWITCHED`). The battery's reads come after this REQ_STAT; they move the head for the
    // later sections (`o.head`, `state_lines`).
    let idle = if switched { atapi::DRIVE_PAUSE } else { atapi::DRIVE_STANDBY };
    let idle_fad = atapi::idle_fad(&d.tracks, d.is_cdi, switched);
    let (fad, track) = o.head_at(&d.tracks, idle_fad);
    // disc_format() in ide.rs: GDI -> GD-ROM, CDI by its tracks' modes (Mode 2 -> CD-ROM XA,
    // Mode 1 -> CD-ROM, R144); the repeat nibble is F whenever nothing plays (req_stat_reply).
    let fmt = atapi::disc_format(&d.tracks, d.is_cdi);
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
    // probes.c `toc_any` / `toc_audio`: any valid track entry, the first audio track's FAD.
    let (mut toc_any, mut toc_audio) = (false, 0u32);
    let mut ht = HeadToc::default();
    for s in 0..2u8 {
        let c = cdb_be16(0x14, s, 408);
        let mut toc = [0u8; 408];
        atapi::build_toc(&d.tracks, d.is_cdi, d.boot_area_lba, s, &mut toc);
        o.probe(&format!("GET_TOC_{}", s), None, Some(&c), &Res::ok(&toc), true);
        let lo = fad3(&toc[405..408]);
        let mut area_lo = 0;
        if toc[404] != 0xFF && lo != 0 && lo != 0xFF_FFFF {
            leadout = lo;
            area_lo = lo;
        }
        // The first data track of each area; the last area that has one wins.
        let mut found = false;
        for t in 0..99 {
            let e = &toc[4 * t..4 * t + 4];
            let f = fad3(&e[1..4]);
            if e[0] == 0xFF || f < 150 || f >= 0x00F0_0000 {
                continue;
            }
            toc_any = true;
            if (e[0] & 0x40) == 0 {
                if toc_audio == 0 {
                    toc_audio = f;
                    ht.audio = f;
                    ht.audio_end = toc_track_end(&toc, t, area_lo);
                }
            } else if !found {
                toc_ip = f;
                ht.ip = f;
                ht.ip_end = toc_track_end(&toc, t, area_lo);
                found = true;
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
        return ht;
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

    // An image whose TOC has tracks but no data track (the audio-only CDI 11, R154/R155): gdprobe skips IP_READ,
    // throughput and seek and runs only the flag battery on the first audio track.
    let no_data = opt.ip_fad == 0 && toc_any && toc_ip == 0;
    let ip = if no_data {
        o.event("NO_DATA_TRACK", &format!("audio_fad={} reads=skipped", toc_audio));
        toc_audio
    } else {
        let c = cdb_read(opt.cdread_flags, ip, 7);
        let (r, data) = read_res(d, opt.cdread_flags, ip, 7);
        o.probe("IP_READ", Some(&format!("fad={} count=7", ip)), Some(&c), &r, true);
        o.note_read(&d.tracks, r.ok, ip, 7);
        hash_sectors(o, ip, 7, &data);
        ip
    };

    if opt.flags_test && ip != 0 {
        // The sector type in byte 1 is checked by the track's mode (D8, D9, R144); a data track reads 2048 bytes
        // (2352 for 0x10 with --raw-sectors, D10), an audio track its 2352-byte PCM sectors (R144).
        for flags in [0x20u8, 0x24, 0x28, 0x10] {
            let c = cdb_read(flags, ip, 1);
            let (mut r, mut data) = read_res(d, flags, ip, 1);
            if opt.raw_sectors && data.is_some() && atapi::cd_read_wants_raw(flags) {
                if let Some(raw) = d.cd_read_raw(ip) {
                    r = Res::ok_big(raw.len(), &raw[..raw.len().min(REC_DATA_MAX)]);
                    data = Some(raw);
                }
            }
            let mismatch = atapi::track_of_start(&d.tracks, ip)
                .map_or(false, |t| atapi::cd_read_type_mismatch(flags, t.mode));
            let (ssz, crc, sense) = match &data {
                Some(dd) => (dd.len().to_string(), format!("{:08x}", crc32(dd)), "none"),
                None if mismatch => ("-".to_string(), "-".to_string(), "5/64/00"), // illegal mode for this track
                None => ("-".to_string(), "-".to_string(), refusal_sense(d, flags, ip)),
            };
            let ex = format!("flags=0x{:02x} fad={} count=1 ssz={} crc={} sense={}", flags, ip, ssz, crc, sense);
            o.probe("CDREAD_FLAGS", Some(&ex), Some(&c), &r, false);
            o.note_read(&d.tracks, r.ok, ip, 1);
            if !r.ok {
                // probes.c reads the sense with a REQ_ERROR that is not logged: the next CDB phase follows it.
                o.last_check = false;
                o.packet_done(true, 0);
            }
        }
    }

    if no_data {
        return ht;
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
            o.note_read(&d.tracks, r.ok, fad, n);
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
            o.note_read(&d.tracks, r.ok, fad, 1);
            hash_sectors(o, fad, 1, &data);
        }
    }
    ht
}

/// The sense gdprobe reads after a refused CD_READ, as `handle_cd_read` sets it: 5/64/00 for a sector type that
/// does not fit the start's track (`atapi::track_of_start`: the next track for a start in its pregap, checked
/// first), 5/21/00 for a start no track holds (`atapi::cd_read_start_outside`), else 5/00/00 (past the track's end,
/// an audio track not stored as whole sectors).
fn refusal_sense(d: &Disc, flags: u8, fad: u32) -> &'static str {
    let t = atapi::track_of_start(&d.tracks, fad);
    if t.map_or(false, |t| atapi::cd_read_type_mismatch(flags, t.mode)) {
        "5/64/00"
    } else if atapi::cd_read_start_outside(&d.tracks, fad) {
        "5/21/00"
    } else {
        "5/00/00"
    }
}

/// gdprobe seq 21 `head_probe` / seq 22 `gap_probe` (probes.c `hp_state`), after the battery, as NetGD answers it.
struct HeadProbe<'a> {
    d: &'a Disc,
    /// Line prefix (`HD` / `GP`).
    pfx: &'static str,
    /// Seq 22's extra fields (`ctl=`, `scd_ctl=`, `scd_rel=`).
    ext: bool,
    late_ms: u32,
    scd: bool,
    /// Seq 23 `gap_crc`: `crc=` / `scrc=` / `stored=` after `sense=`.
    crc: bool,
    /// `pos` of the previous `*_POS` line (`moved=`).
    prev: Option<u32>,
    /// Seq 24: the last two REQ_STAT answers and the last GET_SCD(1) of `pos` (for `GP_CTL`).
    q: Option<[u8; 10]>,
    pq: Option<[u8; 10]>,
    qscd: Option<[u8; 14]>,
}

impl HeadProbe<'_> {
    /// `hp_pos`: REQ_STAT, GET_SCD(1) (`scd`), the late REQ_STAT (`late_ms`), one line. The head is where the last
    /// CD_READ since the mount left it, else the park position (`atapi::rest_fad`); nothing plays.
    fn pos(&mut self, o: &mut Out, label: &str, read: Option<(u32, u32)>, pre: &str) {
        let d = self.d;
        let fad = atapi::rest_fad(&d.tracks, d.is_cdi, true, o.head);
        let (fad, track) = o.head_at(&d.tracks, fad);
        let fmt = atapi::disc_format(&d.tracks, d.is_cdi);
        let stat = atapi::req_stat_reply(AudioState::Standby, atapi::DRIVE_PAUSE, fmt, 0, fad, track);
        let st = |b0: u8| match b0 & 0x0F {
            0 => "busy",
            1 => "pause",
            2 => "standby",
            3 => "play",
            _ => "?",
        };
        let pos = fad3(&stat[5..8]);
        let mut f = String::new();
        if !pre.is_empty() {
            let _ = write!(f, "{} ", pre);
        }
        let _ = write!(f, "stat_rc=ok st={} trk={} idx={} pos={}", st(stat[0]), stat[3], stat[4], pos);
        if self.ext {
            let _ = write!(f, " ctl=0x{:02x}", stat[2]);
        }
        if let Some((rf, rc)) = read {
            let _ = write!(f, " d_start={:+} d_end={:+}", pos as i64 - rf as i64, pos as i64 - (rf + rc) as i64);
        }
        match self.prev {
            Some(p) => {
                let _ = write!(f, " moved={}", (p != pos) as u8);
            }
            None => f.push_str(" moved=-"),
        }
        let mut scd = [0u8; 100];
        if self.scd {
            atapi::get_scd_reply(1, AudioState::Standby, fad, track, &mut scd);
            let _ = write!(f, " scd_rc=ok scd_trk={} scd_idx={} scd_pos={}", scd[5], scd[6], fad3(&scd[11..14]));
            if self.ext {
                let _ = write!(f, " scd_ctl=0x{:02x} scd_rel={}", scd[4], fad3(&scd[7..10]));
            }
        }
        if self.late_ms != 0 {
            let _ = write!(f, " late_rc=ok late_st={} late_pos={}", st(stat[0]), pos);
        }
        let _ = write!(f, " stat={}", hex(&stat));
        if self.scd {
            let _ = write!(f, " scd={}", hex(&scd[..14]));
        }
        self.prev = Some(pos);
        self.pq = self.q;
        self.q = Some(stat);
        if self.scd {
            let mut q = [0u8; 14];
            q.copy_from_slice(&scd[..14]);
            self.qscd = Some(q);
        } else {
            self.qscd = None;
        }
        o.event(&format!("{}_{}_POS", self.pfx, label), &f);
        // The queries all succeed: the next logged command follows a good status.
        o.last_check = false;
    }

    /// `hp_read_line`: the CD_READ (a CHECK is followed by an unlogged REQ_ERROR) and its line `name`, `pre` first
    /// among the fields. Returns whether it read.
    fn read_line(&mut self, o: &mut Out, name: &str, pre: &str, flags: u8, fad: u32, count: u32) -> bool {
        let d = self.d;
        let c = cdb_read(flags, fad, count);
        let (r, data) = read_res(d, flags, fad, count);
        let (ssz, sense) = match &data {
            // gdprobe prints the sector size only when the bytes divide by the count (`GP_XPRE2` on stock:
            // 10 x 2,324 + 6 x 2,048 -> `ssz=-`).
            Some(dd) if r.ok && !dd.is_empty() && dd.len() % count as usize == 0 => {
                ((dd.len() / count as usize).to_string(), "none")
            }
            _ if r.ok => ("-".to_string(), "none"),
            _ => ("-".to_string(), refusal_sense(d, flags, fad)),
        };
        let mut ex = String::new();
        if !pre.is_empty() {
            let _ = write!(ex, "{} ", pre);
        }
        let _ = write!(ex, "flags=0x{:02x} fad={} count={} ssz={} sense={}", flags, fad, count, ssz, sense);
        if self.crc {
            // probes.c: over what fits gdprobe's 40,000-byte buffer; per sector when it all fits and divides.
            match &data {
                Some(dd) if r.ok && !dd.is_empty() => {
                    let got = dd.len().min(GDPROBE_BUFSZ);
                    let _ = write!(ex, " crc={:08x}", crc32(&dd[..got]));
                    if got < dd.len() {
                        let _ = write!(ex, " stored={}", got);
                    }
                    if count > 1 && got == dd.len() && got % count as usize == 0 {
                        let z = got / count as usize;
                        let v: Vec<String> = dd.chunks(z).map(|c| format!("{:08x}", crc32(c))).collect();
                        let _ = write!(ex, " scrc={}", v.join(","));
                    }
                }
                _ => ex.push_str(" crc=-"),
            }
        }
        o.probe(name, Some(&ex), Some(&c), &r, false);
        o.note_read(&d.tracks, r.ok, fad, count);
        if !r.ok {
            o.last_check = false;
            o.packet_done(true, 0);
        }
        r.ok
    }

    /// `hp_read`: the read's line, then `pos`. Returns whether it read.
    fn read(&mut self, o: &mut Out, label: &str, flags: u8, fad: u32, count: u32) -> bool {
        let name = format!("{}_{}", self.pfx, label);
        let ok = self.read_line(o, &name, "", flags, fad, count);
        self.pos(o, label, Some((fad, count)), "");
        ok
    }

    /// `hp_cmd`: TUR / GET_TOC / REQ_SES, its line, then `pos`.
    fn cmd(&mut self, o: &mut Out, label: &str, c: &[u8; 12], r: &Res) {
        o.probe(&format!("{}_{}", self.pfx, label), None, Some(c), r, false);
        self.pos(o, label, None, "");
    }
}

fn head_probe(opt: &Opts, d: &Disc, o: &mut Out, ht: HeadToc) {
    let (dd, e, a, ae) = (ht.ip, ht.ip_end, ht.audio, ht.audio_end);
    let fl = opt.head_flags;
    let data_ok = dd != 0 && e > dd && e - dd >= 256;
    let aud_ok = a != 0 && ae > a && ae - a >= 32;
    o.event(
        "HD_START",
        &format!(
            "data_fad={} data_end={} audio_fad={} audio_end={} data_cases={} audio_cases={}",
            dd, e, a, ae, data_ok as u8, aud_ok as u8
        ),
    );
    let mut hp = HeadProbe { d, pfx: "HD", ext: false, late_ms: opt.head_late_ms, scd: opt.head_scd, crc: false, prev: None, q: None, pq: None, qscd: None };
    hp.pos(o, "BASE", None, "");
    if data_ok {
        let len = e - dd;
        let (mut last_fad, mut last_n) = (dd, 1u32);
        for (i, &n) in opt.head_counts.iter().enumerate() {
            last_fad = dd + ((len as u64 * (i as u64 + 1)) / 16) as u32;
            last_n = n;
            hp.read(o, &format!("R{}", n), fl, last_fad, n);
        }
        hp.read(o, "HIT", fl, last_fad + last_n / 2, 1);
        hp.read(o, "MID7", fl, dd + len / 2, 7);
        hp.read(o, "END7", fl, e - 40, 7);
        hp.read(o, "END1", fl, e - 2, 1);
        hp.read(o, "START7", fl, dd, 7);
        hp.read(o, "CHK24", 0x24, dd + ((len as u64 * 10) / 16) as u32, 7);
        hp.read(o, "CHK28", 0x28, dd + ((len as u64 * 12) / 16) as u32, 7);
    }
    hp.cmd(o, "TUR", &[0u8; 12], &Res::ok(&[]));
    let mut toc = [0u8; 408];
    atapi::build_toc(&d.tracks, d.is_cdi, d.boot_area_lba, 0, &mut toc);
    hp.cmd(o, "TOC", &cdb_be16(0x14, 0, 408), &Res::ok(&toc));
    let r = match atapi::req_ses_reply(&d.tracks, d.is_cdi, d.boot_area_lba, d.sessions, 1, atapi::DRIVE_PAUSE) {
        Some(rep) => Res::ok(&rep),
        None => Res::illegal(),
    };
    hp.cmd(o, "SES", &cdb_simple(0x15, 0, 1, 6), &r);
    if aud_ok {
        let alen = ae - a;
        hp.read(o, "AUD1", fl, a + alen / 4, 1);
        hp.read(o, "AUD7", fl, a + alen / 2, 7);
    }
    if opt.head_idle_ms != 0 {
        hp.pos(o, "IDLE", None, &format!("idle_ms={}", opt.head_idle_ms));
    }
}

/// toc.c `gp_toc_parse`: one GET_TOC area's valid entries (number, CTRL/ADR byte, FAD) and lead-out (0 = none).
struct TocArea {
    trk: Vec<(u32, u32, u32)>,
    leadout: u32,
    /// The lead-out entry's CTRL/ADR byte (seq 24).
    leadout_ctl: u32,
}

impl TocArea {
    fn parse(toc: &[u8; 408]) -> TocArea {
        let mut trk = Vec::new();
        for t in 0..99 {
            let e = &toc[4 * t..4 * t + 4];
            let f = fad3(&e[1..4]);
            if e[0] == 0xFF || f < 150 || f >= 0x00F0_0000 {
                continue;
            }
            trk.push((t as u32 + 1, e[0] as u32, f));
        }
        let lo = fad3(&toc[405..408]);
        let ok = toc[404] != 0xFF && lo != 0 && lo < 0x00F0_0000;
        TocArea { trk, leadout: if ok { lo } else { 0 }, leadout_ctl: if ok { toc[404] as u32 } else { 0 } }
    }
    fn valid(&self) -> bool {
        !self.trk.is_empty()
    }
}

/// toc.c `gp_gap_plan_make`.
#[derive(Default)]
struct GapPlan {
    trk_ok: bool,
    trk_why: &'static str,
    a: (u32, u32, u32),
    b: (u32, u32, u32),
    leadout: u32,
    last: (u32, u32),
    lo_ok: bool,
    lo_why: &'static str,
    hd_ok: bool,
    hd_why: &'static str,
    ld_leadout: u32,
    hd: (u32, u32),
    ref_fad: u32,
    /// Both areas' tracks in FAD order (toc.c `merge`): (number, CTRL/ADR, FAD).
    tracks: Vec<(u32, u32, u32)>,
}

fn gap_plan(area: &[TocArea; 2]) -> GapPlan {
    let mut p = GapPlan { trk_why: "-", lo_why: "-", hd_why: "-", ..Default::default() };
    // Both areas' tracks in FAD order, without the duplicates a CD's copied area 1 adds.
    let mut t: Vec<(u32, u32, u32)> = Vec::new();
    for a in area.iter().filter(|a| a.valid()) {
        for &e in &a.trk {
            if t.iter().any(|x| x.2 == e.2) {
                continue;
            }
            let at = t.iter().position(|x| x.2 > e.2).unwrap_or(t.len());
            t.insert(at, e);
        }
    }
    let n = t.len();
    if n < 1 {
        p.trk_why = "no_toc";
    } else if n < 2 {
        p.trk_why = "one_track";
    } else if t[1].2 < t[0].2 + 2 {
        p.trk_why = "no_room";
    } else {
        p.trk_ok = true;
        p.a = t[0];
        p.b = t[1];
    }
    for a in area.iter().filter(|a| a.valid()) {
        p.leadout = p.leadout.max(a.leadout);
    }
    for e in &t {
        if e.2 < p.leadout && e.2 >= p.last.1 {
            p.last = (e.0, e.2);
        }
    }
    if p.leadout == 0 {
        p.lo_why = "no_leadout";
    } else if p.last.1 == 0 || p.leadout < p.last.1 + 2 {
        p.lo_why = "no_track_before_leadout";
    } else {
        p.lo_ok = true;
    }
    if !area[0].valid() || !area[1].valid() {
        p.hd_why = "no_hd_area";
    } else if area[0].leadout == 0 {
        p.hd_why = "no_ld_leadout";
    } else {
        let mut h = (0u32, 0u32);
        for e in &area[1].trk {
            if e.2 > area[0].leadout && (h.1 == 0 || e.2 < h.1) {
                h = (e.0, e.2);
            }
        }
        if h.1 == 0 {
            p.hd_why = "no_hd_area";
        } else if h.1 < area[0].leadout + 2 {
            p.hd_why = "no_gap";
        } else {
            p.hd_ok = true;
            p.ld_leadout = area[0].leadout;
            p.hd = h;
        }
    }
    // Reference: the first data track + 16 (inside it), else the first track's.
    let i = t.iter().position(|e| e.1 & 0x40 != 0).unwrap_or(0);
    if n > 0 {
        let end = if i + 1 < n { t[i + 1].2 } else { p.leadout };
        p.ref_fad = if end > t[i].2 + 17 { t[i].2 + 16 } else { t[i].2 };
    }
    p.tracks = t;
    p
}

/// probes.c `gp_find_gap`: 1-sector reads (`name` lines, `n=` first), binary search for the last readable sector in
/// lo..hi. Returns the first gap FAD, or 0 and why.
fn find_gap(o: &mut Out, hp: &mut HeadProbe, name: &str, fl: u8, lo: u32, hi: u32, reads: &mut i32) -> (u32, &'static str) {
    let (mut lo, mut hi) = (lo, hi);
    let scan = |o: &mut Out, hp: &mut HeadProbe, reads: &mut i32, fad: u32| {
        *reads += 1;
        hp.read_line(o, name, &format!("n={}", reads), fl, fad, 1)
    };
    if !scan(o, hp, reads, lo) {
        return (0, "track_start_unreadable");
    }
    if scan(o, hp, reads, hi) {
        return (0, "no_gap");
    }
    loop {
        let t = bsearch_next(lo, hi);
        if t == 0 || *reads >= 48 {
            break;
        }
        if scan(o, hp, reads, t) {
            lo = t;
        } else {
            hi = t;
        }
    }
    if bsearch_next(lo, hi) != 0 {
        (0, "scan_too_long")
    } else {
        (lo + 1, "-")
    }
}

/// toc.c `gp_bsearch_next`.
fn bsearch_next(lo: u32, hi: u32) -> u32 {
    if hi <= lo + 1 {
        0
    } else {
        lo + (hi - lo) / 2
    }
}

/// gdprobe seq 22 `gap_probe` (probes.c), after the battery and head_probe, as NetGD answers it.
fn gap_probe(opt: &Opts, d: &Disc, o: &mut Out) {
    let mut areas = [[0u8; 408]; 2];
    for (s, toc) in areas.iter_mut().enumerate() {
        atapi::build_toc(&d.tracks, d.is_cdi, d.boot_area_lba, s as u8, toc);
    }
    let area = [TocArea::parse(&areas[0]), TocArea::parse(&areas[1])];
    let p = gap_plan(&area);
    let why = |ok: bool, w: &'static str| if ok { "ok" } else { w };
    o.event(
        "GP_START",
        &format!(
            "toc0={}/{} toc1={}/{} trk_a={}/{}/0x{:02x} trk_b={}/{}/0x{:02x} leadout={} last={}/{} ld_leadout={} hd={}/{} ref={} trk={} lo={} hdg={}",
            area[0].trk.len(), area[0].leadout, area[1].trk.len(), area[1].leadout,
            p.a.0, p.a.2, p.a.1, p.b.0, p.b.2, p.b.1, p.leadout, p.last.0, p.last.1,
            p.ld_leadout, p.hd.0, p.hd.1, p.ref_fad,
            why(p.trk_ok, p.trk_why), why(p.lo_ok, p.lo_why), why(p.hd_ok, p.hd_why)
        ),
    );
    let mut hp = HeadProbe { d, pfx: "GP", ext: true, late_ms: opt.gap_late_ms, scd: opt.gap_scd, crc: opt.gap_crc, prev: None, q: None, pq: None, qscd: None };
    hp.pos(o, "BASE", None, "");
    if opt.gap_probe {
        gap_cases22(opt, o, &mut hp, &p);
    }
    if opt.pregap_probe {
        pregap_cases(opt, o, &mut hp, &p);
    }
    if opt.ctl_probe {
        ctl_cases(opt, o, &mut hp, &area, &p);
    }
}

/// probes.c `gap_cases22` (seq 22).
fn gap_cases22(opt: &Opts, o: &mut Out, hp: &mut HeadProbe, p: &GapPlan) {
    let fl = opt.gap_flags;

    // (1) the gap after track 1
    if !p.trk_ok {
        o.event("GP_TRK_SKIP", &format!("reason={}", p.trk_why));
    } else {
        let (a_fad, b_fad) = (p.a.2, p.b.2);
        let mut reads = 0;
        let mut why = "-";
        let src;
        let g = if opt.gap_track_end != 0 {
            src = "cfg";
            let g = opt.gap_track_end;
            if g <= a_fad || g >= b_fad {
                why = "gap_track_end_outside";
                0
            } else {
                g
            }
        } else {
            src = "scan";
            let (g, w) = find_gap(o, hp, "GP_SCAN_R", fl, a_fad, b_fad - 1, &mut reads);
            why = w;
            g
        };
        if g == 0 {
            o.event("GP_TRK_SKIP", &format!("a_fad={} b_fad={} src={} reads={} reason={}", a_fad, b_fad, src, reads, why));
        } else {
            let gh = b_fad - 1;
            let mid = g + (gh - g) / 2;
            let t1 = if a_fad + 50 < g { a_fad + 50 } else { a_fad };
            o.event(
                "GP_SCAN",
                &format!(
                    "a_fad={} b_fad={} last_ok={} gap_lo={} gap_hi={} gap_len={} gap_mid={} src={} reads={}",
                    a_fad, b_fad, g - 1, g, gh, gh - g + 1, mid, src, reads
                ),
            );
            hp.read(o, "TRK_T1", fl, t1, 1);
            hp.read(o, "TRK_GAPMID", fl, mid, 1);
            hp.read(o, "TRK_T1END", fl, g - 1, 1);
            hp.read(o, "TRK_GAPLO", fl, g, 1);
            hp.read(o, "TRK_T2", fl, b_fad + 16, 1);
            hp.read(o, "TRK_GAPHI", fl, gh, 1);
            hp.read(o, "TRK_T2B", fl, b_fad + 32, 1);
            hp.read(o, "TRK_GAPMID2", fl, mid, 1);
        }
    }

    // (2) the lead-out
    if !p.lo_ok {
        o.event("GP_LO_SKIP", &format!("leadout={} reason={}", p.leadout, p.lo_why));
    } else {
        hp.read(o, "LO_REF", fl, p.leadout - 2, 1);
        for &off in &opt.gap_leadout {
            hp.read(o, &format!("LO_{}", off), fl, p.leadout + off, 1);
        }
    }

    // (3) a GD-ROM's gap between the areas
    if !p.hd_ok {
        o.event("GP_HDG_SKIP", &format!("reason={}", p.hd_why));
    } else {
        let mid = p.ld_leadout + (p.hd.1 - p.ld_leadout) / 2;
        o.event(
            "GP_HDG",
            &format!("ld_leadout={} hd_fad={} gap_lo={} gap_hi={} gap_mid={}", p.ld_leadout, p.hd.1, p.ld_leadout, p.hd.1 - 1, mid),
        );
        hp.read(o, "HDG_REF", fl, p.ld_leadout - 1, 1);
        hp.read(o, "HDG_L0", fl, p.ld_leadout, 1);
        hp.read(o, "HDG_MID", fl, mid, 1);
        hp.read(o, "HDG_HI", fl, p.hd.1 - 1, 1);
        hp.read(o, "HDG_HD", fl, p.hd.1 + 16, 1);
        hp.read(o, "HDG_MID2", fl, mid, 1);
    }

    // (4) below 150
    if p.ref_fad == 0 {
        o.event("GP_LOW_SKIP", "reason=no_toc");
    } else if !opt.gap_low.is_empty() {
        hp.read(o, "LOW_REF", fl, p.ref_fad, 1);
        for &f in &opt.gap_low {
            hp.read(o, &format!("LOW_{}", f), fl, f, 1);
        }
    }

    // (5) count 0: gdprobe's whitelist allows 1..64, nothing is sent.
    if opt.gap_count0 {
        if p.ref_fad == 0 {
            o.event("GP_CNT0_SKIP", "reason=no_toc");
        } else {
            let c = cdb_read(fl, p.ref_fad, 0);
            o.event("GP_CNT0_SKIP", &format!("fad={} cdb={} reason=whitelist_count_1_64", p.ref_fad, hex(&c)));
        }
    }
}

/// probes.c `pregap_cases` (seq 23).
fn pregap_cases(opt: &Opts, o: &mut Out, hp: &mut HeadProbe, p: &GapPlan) {
    let fl = opt.gap_flags;
    let t = &p.tracks;
    let (cross, half) = (opt.pregap_cross, opt.pregap_cross / 2);
    let list: Vec<String> = t.iter().map(|e| format!("{}/{}/0x{:02x}", e.0, e.2, e.1)).collect();
    o.event(
        "GP_PG_START",
        &format!(
            "cross={} tracks={} leadout={} ld_leadout={}",
            cross,
            if list.is_empty() { "-".to_string() } else { list.join(",") },
            p.leadout,
            p.ld_leadout
        ),
    );
    if t.is_empty() {
        o.event("GP_PG_SKIP", "reason=no_toc");
        return;
    }
    for &f in &opt.pregap_low {
        hp.read(o, &format!("PGL_{}", f), fl, f, 1);
    }
    for i in 1..t.len() {
        let fads: Vec<String> = opt
            .pregap_offs
            .iter()
            .map(|&off| t[i].2 as i64 + off)
            .filter(|&f| f >= 0 && f < 0x00F0_0000)
            .map(|f| f.to_string())
            .collect();
        o.event(
            "GP_PGB",
            &format!("trk={} start={} prev={}/{} fads={}", t[i].0, t[i].2, t[i - 1].0, t[i - 1].2, fads.join(",")),
        );
        for &off in &opt.pregap_offs {
            let fa = t[i].2 as i64 + off;
            if fa < 0 {
                continue;
            }
            let lb = format!("PG{}_{}{}", t[i].0, if off < 0 { 'M' } else { 'P' }, off.abs());
            hp.read(o, &lb, fl, fa as u32, 1);
        }
    }
    for (i, e) in t.iter().enumerate() {
        if e.2 >= 10 {
            hp.read(o, &format!("XPRE{}", e.0), fl, e.2 - 10, cross);
        }
        if i > 0 && e.2 >= 158 {
            let lb = format!("XGAP{}", e.0);
            // Seq 24: if its first sector reads (GP_XGAP<t>_CHK, no position queries), it starts on data.
            if !opt.risky_inline && hp.read_line(o, &format!("GP_{}_CHK", lb), "", fl, e.2 - 158, 1) {
                o.event(&format!("GP_{}_SKIP", lb), &format!("fad={} reason=starts_on_data_risky_cross_off", e.2 - 158));
            } else {
                hp.read(o, &lb, fl, e.2 - 158, cross);
            }
        }
    }
    if !opt.risky_inline {
        o.event("GP_RISKY_SKIP", "cases=XEND,XLD,XLO reason=risky_cross_off");
        return;
    }
    if !p.trk_ok {
        o.event("GP_XEND_SKIP", &format!("reason={}", p.trk_why));
    } else {
        let (a_fad, b_fad) = (p.a.2, p.b.2);
        let hi = if b_fad > 151 { b_fad - 151 } else { 0 };
        let mut reads = 0;
        let (src, g, why) = if opt.gap_track_end != 0 {
            let g = opt.gap_track_end;
            if g <= a_fad || g >= b_fad {
                ("cfg", 0, "gap_track_end_outside")
            } else {
                ("cfg", g, "-")
            }
        } else if hi <= a_fad {
            ("scan", 0, "short_track")
        } else {
            let (g, w) = find_gap(o, hp, "GP_DSCAN_R", fl, a_fad, hi, &mut reads);
            ("scan", g, w)
        };
        if g == 0 {
            o.event(
                "GP_XEND_SKIP",
                &format!("a_fad={} b_fad={} hi={} src={} reads={} reason={}", a_fad, b_fad, hi, src, reads, why),
            );
        } else {
            o.event(
                "GP_DSCAN",
                &format!("a_fad={} b_fad={} hi={} last_ok={} gap_lo={} src={} reads={}", a_fad, b_fad, hi, g - 1, g, src, reads),
            );
            hp.read(o, "XEND", fl, if g > half { g - half } else { 0 }, cross);
        }
    }
    if p.hd_ok {
        hp.read(o, "XLD", fl, p.ld_leadout - half, cross);
    }
    if p.lo_ok {
        hp.read(o, "XLO", fl, p.leadout - half, cross);
    } else {
        o.event("GP_XLO_SKIP", &format!("reason={}", p.lo_why));
    }
}

/// toc.c `gp_ctl_region`.
struct CtlRegion {
    name: &'static str,
    prev: (u32, u32),
    in_fad: u32,
    /// The next track (number, CTRL), or None for the lead-out (then `next_ctl` is the lead-out entry's).
    next: Option<u32>,
    next_ctl: u32,
    next_in: u32,
    rev: (u32, u32, u32),
    off: Vec<u32>,
}

/// toc.c `gp_ctl_regions`.
fn ctl_regions(area: &[TocArea; 2], p: &GapPlan) -> Vec<CtlRegion> {
    let t = &p.tracks;
    let mut out = Vec::new();
    if t.is_empty() {
        return out;
    }
    if p.trk_ok && p.b.2 > p.a.2 + 151 + 17 {
        out.push(CtlRegion {
            name: "GAP",
            prev: (p.a.0, p.a.1),
            in_fad: p.a.2 + 16,
            next: Some(p.b.0),
            next_ctl: p.b.1,
            next_in: p.b.2 + 16,
            rev: (0, 0, 0),
            off: vec![p.b.2 - 151],
        });
    }
    if p.hd_ok && p.hd.1 > p.ld_leadout + 151 {
        let (mut pf, mut pn, mut pc) = (0u32, 0u32, 0u32);
        for e in &area[0].trk {
            if e.2 < p.ld_leadout && e.2 >= pf {
                pf = e.2;
                pn = e.0;
                pc = e.1;
            }
        }
        let next_ctl = area[1].trk.iter().filter(|e| e.2 == p.hd.1).map(|e| e.1).last().unwrap_or(0x41);
        out.push(CtlRegion {
            name: "HDG",
            prev: (pn, pc),
            in_fad: if pf + 16 < p.ld_leadout { pf + 16 } else { pf },
            next: Some(p.hd.0),
            next_ctl,
            next_in: p.hd.1 + 16,
            rev: (0, 0, 0),
            off: vec![p.ld_leadout, p.ld_leadout + (p.hd.1 - 151 - p.ld_leadout) / 2, p.hd.1 - 151],
        });
    }
    if p.lo_ok {
        let mut lc = 0;
        for a in area.iter().filter(|a| a.valid()) {
            if a.leadout == p.leadout {
                lc = a.leadout_ctl;
            }
        }
        let prev_ctl = t.iter().filter(|e| e.2 == p.last.1).map(|e| e.1).last().unwrap_or(0);
        out.push(CtlRegion {
            name: "LO",
            prev: (p.last.0, prev_ctl),
            in_fad: if p.last.1 + 16 < p.leadout { p.last.1 + 16 } else { p.last.1 },
            next: None,
            next_ctl: lc,
            next_in: t[0].2 + 16,
            rev: (0, 0, 0),
            off: vec![p.leadout + 1],
        });
    }
    for g in out.iter_mut() {
        g.rev = match g.next {
            Some(n) => (n, g.next_ctl, g.next_in),
            None => (t[0].0, t[0].1, t[0].2 + 16),
        };
    }
    out
}

/// probes.c `ctl_verdict`: what the off-track read's queries reported and which hypotheses it matches.
fn ctl_verdict(o: &mut Out, hp: &HeadProbe, g: &CtlRegion, dir: &str, off: u32, ok: bool, last: (u32, u32)) {
    let mut f = format!("region={} dir={} off_fad={} off_rc={}", g.name, dir, off, if ok { "ok" } else { "check" });
    if ok {
        f.push_str(" off_track=0");
    }
    let ctl = hp.q.map(|q| q[2] as u32);
    match hp.q {
        Some(q) => {
            let _ = write!(f, " trk={} idx={} ctl=0x{:02x}", q[3], q[4], q[2]);
        }
        None => f.push_str(" trk=- idx=- ctl=-"),
    }
    if let Some(c) = hp.qscd {
        let _ = write!(f, " scd_trk={} scd_ctl=0x{:02x} scd_rel={}", c[5], c[4], fad3(&c[7..10]));
    }
    let _ = write!(f, " last_read={}/0x{:02x}", last.0, last.1);
    match hp.pq {
        Some(q) => {
            let _ = write!(f, " prev_query={}/0x{:02x}", q[3], q[2]);
        }
        None => f.push_str(" prev_query=-"),
    }
    match g.next {
        Some(n) => {
            let _ = write!(f, " next={}/0x{:02x}", n, g.next_ctl);
        }
        None => {
            let _ = write!(f, " next=- leadout_entry=0x{:02x}", g.next_ctl);
        }
    }
    let mut m: Vec<&str> = Vec::new();
    if let Some(c) = ctl {
        if c == 0x41 {
            m.push("const41");
        }
        if c == last.1 {
            m.push("last_read");
        }
        if hp.pq.map_or(false, |q| q[2] as u32 == c) {
            m.push("prev_query");
        }
        if c == g.next_ctl {
            m.push(if g.next.is_some() { "next" } else { "leadout_entry" });
        }
    }
    let _ = write!(f, " match={}", if m.is_empty() { "none".to_string() } else { m.join(",") });
    o.event("GP_CTL", &f);
}

/// probes.c `ctl_cases` (seq 24).
fn ctl_cases(opt: &Opts, o: &mut Out, hp: &mut HeadProbe, area: &[TocArea; 2], p: &GapPlan) {
    let fl = opt.gap_flags;
    let regions = ctl_regions(area, p);
    if regions.is_empty() {
        o.event("GP_CTL_SKIP", "reason=no_region");
        return;
    }
    for g in &regions {
        let mut f = format!("region={} prev={}/0x{:02x} in={} next=", g.name, g.prev.0, g.prev.1, g.in_fad);
        match g.next {
            Some(n) => {
                let _ = write!(f, "{}/0x{:02x}", n, g.next_ctl);
            }
            None => {
                let _ = write!(f, "- leadout_entry=0x{:02x}", g.next_ctl);
            }
        }
        let offs: Vec<String> = g.off.iter().map(|x| x.to_string()).collect();
        let _ = write!(f, " rev={}/0x{:02x}/{} off={}", g.rev.0, g.rev.1, g.rev.2, offs.join(","));
        o.event("GP_CTL_REGION", &f);
        hp.read(o, &format!("CTLF_{}_IN", g.name), fl, g.in_fad, 1);
        for (k, &off) in g.off.iter().enumerate() {
            let ok = hp.read(o, &format!("CTLF_{}_OFF{}", g.name, k), fl, off, 1);
            ctl_verdict(o, hp, g, "fwd", off, ok, g.prev);
        }
        let mid = g.off[g.off.len() / 2];
        hp.read(o, &format!("CTLR_{}_PRE", g.name), fl, g.rev.2, 1);
        let ok = hp.read(o, &format!("CTLR_{}_OFF", g.name), fl, mid, 1);
        ctl_verdict(o, hp, g, "rev", mid, ok, (g.rev.0, g.rev.1));
        hp.read(o, &format!("CTLR_{}_IN", g.name), fl, g.in_fad, 1);
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
    // SECNR with no medium: NO DISC, format 0 (ide.rs `rest_secnr`, R144 b).
    o.secnr = Some(atapi::DRIVE_NODISC);
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

fn sw_poll(o: &mut Out, r: &Res, sense: &str, secnr: Option<u8>) {
    let mut f = format!("t_ms=- poll=- rc={} status=0x{:02x} err=0x{:02x} sense={}", if r.ok { "ok" } else { "check" }, r.status, r.err, sense);
    if let Some(v) = secnr {
        f.push_str(&format!(" secnr=0x{:02x}", v));
    }
    o.event("SW_POLL", &f);
    // A poll is a TUR, after a CHECK followed by a REQ_ERROR (both unlogged): it ends on a good status.
    o.last_check = false;
    o.packet_done(r.ok, r.err);
    if !r.ok {
        o.packet_done(true, 0); // the REQ_ERROR
    }
    o.secnr = secnr;
}

/// gdprobe's command-free register poll after a switch (`swap_regpoll`, R154), as NetGD answers.
struct RegPoll {
    open_ms: u32,
    detect_ms: u32,
}

/// One `SW_REGPOLL` line: ALTSTATUS / ERROR / interrupt reason / SECNR.
fn regpoll_line(o: &mut Out, altst: u8, err: u8, irr: u8, secnr: Option<u8>) {
    let mut f = format!("t_ms=- sample=- altst=0x{:02x} err=0x{:02x} irr=0x{:02x}", altst, err, irr);
    if let Some(v) = secnr {
        f.push_str(&format!(" secnr=0x{:02x}", v));
    }
    o.event("SW_REGPOLL", &f);
}

/// The register poll after an accepted `52 82 n` to another folder: one line per change. The registers are those
/// `52 82` completed with (ALTSTATUS 0x50, IRR 3, ERROR the last CHECK's -- sticky --) and SECNR walks the window (ide.rs
/// `show_window_open`, then `ide_task`'s timer): the old byte, `0x06`, `0x00`, the final one. Ends with the window over.
///
/// Order: for an image `gdemu_mount_staged` completes the command (`finish_ok`: status 0x50, INTRQ, SECNR still the
/// old byte) and only then writes the window's `0x06` (stock's order, R155 / R159-R161). The old byte is therefore the
/// state at completion; how long it stays visible is not modelled (NetGD writes `0x06` microseconds after the
/// completion, stock a few ms), so a poll that starts 1 ms later may already see `0x06` first. For the empty folder /
/// no medium it writes `0x06` before completing, so the poll starts at `0x06`.
///
/// NOTE: this matches stock in records 15-18 (images 3 / 4 / 11 show the old byte at sample 1 and `0x06` a few ms
/// later, the empty folder 07 shows `0x06` already at sample 1). Likely stock completes first in both cases and the
/// empty folder simply has nothing to load (hypothesis from the records, not verified).
fn regpoll_lines(o: &mut Out, rp: &RegPoll, medium: bool, disc_format: u8) {
    use atapi::SwapPhase::*;
    let _ = (rp.open_ms, rp.detect_ms);
    let mut last = o.secnr;
    let err = o.err_reg;
    if medium {
        regpoll_line(o, 0x50, err, 0x03, last);
    } else {
        last = None;
    }
    for phase in [Open, Detect, Done] {
        let v = Some(atapi::swap_secnr(phase, medium, disc_format));
        if v != last {
            regpoll_line(o, 0x50, err, 0x03, v);
            last = v;
        }
    }
    o.event("SW_REGPOLL_END", "t_ms=- samples=- changes=- stop=time");
}

/// The ATA DEVICE RESET itself, as NetGD answers it (ide.rs CMD_DEVICE_RESET, R159): gdprobe's `SW_DEVRESET` line
/// with the signature (IRR 1, 14/EB), status 0x00, ERROR 0x01 -- 0x00 on a bitstream without the MCU-writable
/// ERROR bits 0-1 (`err_lo` false) --, SECNR busy `0x00` at once (the re-detect, `atapi::device_reset_window`).
/// The reset's ERROR stays until the next CHECK; the head goes back to the park position. Returns (status, ERROR,
/// IRR, the busy SECNR, the SECNR once the re-detect is over).
fn devreset_signature(o: &mut Out, delay_ms: u32, detect_ms: u32, medium: bool, disc_format: u8, err_lo: bool) -> (u8, u8, u8, u8, u8) {
    use atapi::{SwapPhase, WindowKind::Redetect};
    let err = if err_lo { atapi::DEVICE_RESET_ERROR } else { 0 };
    let (op, det) = atapi::device_reset_window(detect_ms);
    // After a switch the drive idles paused (D2).
    let busy = atapi::window_secnr(Redetect, atapi::swap_phase(0, op, det), medium, disc_format, atapi::DRIVE_PAUSE);
    let done = atapi::window_secnr(Redetect, SwapPhase::Done, medium, disc_format, atapi::DRIVE_PAUSE);
    let (st, irr) = (atapi::DEVICE_RESET_STATUS, atapi::DEVICE_RESET_IRR);
    o.event(
        "SW_DEVRESET",
        &format!(
            "delay_ms={} t_ms=- rc=ok reset_us=- bsy_seen=1 altst=0x{:02x} err=0x{:02x} irr=0x{:02x} secnr=0x{:02x} bcl=0x14 bch=0xeb",
            delay_ms, st, err, irr, busy
        ),
    );
    o.err_reg = err;
    o.head = None;
    o.tno = 0;
    (st, err, irr, busy, done)
}

/// An ATA DEVICE RESET `delay_ms` into gdprobe's register poll (`devreset_signature`), then the byte at rest when
/// the re-detect (detect_time) ends; status and ERROR stay until the next command. Assumes the re-detect is over
/// before gdprobe's polls start. Returns the final SECNR.
fn devreset_lines(o: &mut Out, delay_ms: u32, detect_ms: u32, medium: bool, disc_format: u8, err_lo: bool) -> u8 {
    let (st, err, irr, busy, done) = devreset_signature(o, delay_ms, detect_ms, medium, disc_format, err_lo);
    regpoll_line(o, st, err, irr, Some(busy));
    if done != busy {
        regpoll_line(o, st, err, irr, Some(done));
    }
    o.event("SW_REGPOLL_END", "t_ms=- samples=- changes=- stop=time");
    done
}

/// REQ_STAT and REQ_ERROR once a reset / re-switch section is READY (or not): read-only, like the battery.
/// The head is where the last CD_READ since the mount ended, else the switch's park position (`atapi::rest_fad`).
/// `pre` goes before the probe names (`CMD_` in a seq 20 section).
fn state_lines(o: &mut Out, disc: Option<&Disc>, pre: &str) {
    let rs = cdb_simple(0x10, 0, 0, 10);
    let re = cdb_simple(0x13, 0, 0, 10);
    let (n_rs, n_re) = (format!("{}REQ_STAT", pre), format!("{}REQ_ERROR", pre));
    match disc {
        Some(d) => {
            let fad = atapi::rest_fad(&d.tracks, d.is_cdi, true, o.head);
            let (fad, track) = o.head_at(&d.tracks, fad);
            let stat = atapi::req_stat_reply(AudioState::Standby, atapi::DRIVE_PAUSE, atapi::disc_format(&d.tracks, d.is_cdi), 0, fad, track);
            o.probe(&n_rs, None, Some(&rs), &cut(&stat, 10), true);
            o.probe(&n_re, None, Some(&re), &Res::ok(&atapi::req_error_reply(0, 0, 0)), true);
        }
        None => {
            let (asc, ascq) = atapi::ASC_NO_MEDIUM;
            o.probe(&n_rs, None, Some(&rs), &Res::not_ready(), true);
            o.probe(&n_re, None, Some(&re), &Res::ok(&atapi::req_error_reply(atapi::SENSE_NOT_READY, asc, ascq)), true);
        }
    }
}

/// One TEST UNIT READY of a seq 20 section (gdprobe `dc_tur`: after a CHECK a REQ_ERROR fetches the sense, both
/// unlogged; `altst=` is read after them, 0x50 as the last command ended good). `lead` = `delay_ms=N` (CMD_TUR)
/// or `poll=-` (CMD_POLL).
fn dc_tur(o: &mut Out, name: &str, lead: &str, r: &Res, sense: &str, secnr: u8) {
    o.event(
        name,
        &format!(
            "{} t_ms=- rc={} status=0x{:02x} err=0x{:02x} sense={} secnr=0x{:02x} altst=0x50",
            lead,
            if r.ok { "ok" } else { "check" },
            r.status,
            r.err,
            sense,
            secnr
        ),
    );
    o.last_check = false;
    o.packet_done(r.ok, r.err);
    if !r.ok {
        o.packet_done(true, 0); // the REQ_ERROR
    }
    o.secnr = Some(secnr);
}

/// gdprobe seq 20 `devreset_cmd` (`devreset_cmd_run`, prefix `DC<delay>_`), after the battery of the folder just
/// switched to, as NetGD answers it: 500 ms idle, ATA DEVICE RESET (`devreset_signature`, no register poll),
/// `delay_ms` later a TUR (`CMD_TUR`), REQ_STAT, REQ_ERROR, then a TUR every 10 ms until READY (`CMD_POLL` per
/// change, at most 2 s) and `CMD_RESULT`. Inside the re-detect (detect_time from the reset,
/// `atapi::device_reset_window`) the disc commands get NOT READY 2/04/01 with SECNR busy (ide.rs `drive_not_ready`,
/// `atapi::not_ready_answer`; REQ_ERROR is served and reports it); the first command after it gets the UNIT
/// ATTENTION with a medium (SECNR at rest), then READY; with no medium 2/3A/00 until gdprobe gives up. Time is not
/// simulated: TUR, REQ_STAT and REQ_ERROR are taken to fall in the phase of `delay_ms` (a REQ_STAT that lands just
/// past the re-detect's end is not modelled).
fn devreset_cmd_lines(o: &mut Out, delay_ms: u32, detect_ms: u32, disc_format: u8, err_lo: bool, disc: Option<&Disc>) {
    let medium = disc.is_some();
    let fmt = |(asc, ascq): (u8, u8), key: u8| format!("{}/{:02x}/{:02x}", key, asc, ascq);
    o.event("IDLE", "idle_ms=500");
    let (_, _, _, busy, done) = devreset_signature(o, 0, detect_ms, medium, disc_format, err_lo);
    let (op, det) = atapi::device_reset_window(detect_ms);
    let in_redetect = atapi::swap_phase(delay_ms, op, det) != atapi::SwapPhase::Done;
    let lead = format!("delay_ms={}", delay_ms);
    let nr = atapi::SENSE_NOT_READY;
    let ua = fmt(atapi::ASC_MEDIUM_CHANGED, atapi::SENSE_UNIT_ATTENTION);
    let no_medium = fmt(atapi::ASC_NO_MEDIUM, nr);
    if in_redetect {
        let asc = atapi::ASC_BECOMING_READY;
        dc_tur(o, "CMD_TUR", &lead, &Res::not_ready(), &fmt(asc, nr), busy);
        o.probe("CMD_REQ_STAT", None, Some(&cdb_simple(0x10, 0, 0, 10)), &Res::not_ready(), true);
        o.probe("CMD_REQ_ERROR", None, Some(&cdb_simple(0x13, 0, 0, 10)), &Res::ok(&atapi::req_error_reply(nr, asc.0, asc.1)), true);
        // The re-detect ends: the next TUR gets the UNIT ATTENTION (or no medium for good).
        if medium {
            dc_tur(o, "CMD_POLL", "poll=-", &Res::ua(), &ua, done);
            dc_tur(o, "CMD_POLL", "poll=-", &Res::ok(&[]), "none", done);
        } else {
            dc_tur(o, "CMD_POLL", "poll=-", &Res::not_ready(), &no_medium, done);
        }
    } else {
        if medium {
            dc_tur(o, "CMD_TUR", &lead, &Res::ua(), &ua, done);
        } else {
            dc_tur(o, "CMD_TUR", &lead, &Res::not_ready(), &no_medium, done);
        }
        state_lines(o, disc, "CMD_");
        if medium {
            dc_tur(o, "CMD_POLL", "poll=-", &Res::ok(&[]), "none", done);
        }
    }
    o.event(
        "CMD_RESULT",
        &format!(
            "delay_ms={} rc={} ready={} ready_ms=- t_ms=- polls=- changes=-",
            delay_ms,
            if medium { "ok" } else { "timeout" },
            medium as u8
        ),
    );
}

/// `switch_to`: `52 82 n` to another folder, then TUR / REQ_ERROR polls, one line per change of state. With an image
/// or an empty folder the swap window plays out (`atapi::swap_phase`): no medium, becoming ready, then UNIT
/// ATTENTION and READY -- or no medium again, until gdprobe's switch timeout. A refused mount leaves the old
/// image (and its already-consumed UNIT ATTENTION) in place. With the register poll (`rp`) the polls only
/// start once the window is over.
fn switch_lines(o: &mut Out, n: u32, target: Target, disc_format: u8, rp: Option<&RegPoll>) {
    let c = cdb_mount(n);
    let fmt = |(asc, ascq): (u8, u8), key: u8| format!("{}/{:02x}/{:02x}", key, asc, ascq);
    let nr = atapi::SENSE_NOT_READY;
    let ready = match target {
        Target::Missing => {
            o.probe("SW_CMD", Some(&format!("n={}", n)), Some(&c), &Res::illegal(), false);
            let keep = o.secnr;
            sw_poll(o, &Res::ok(&[]), "none", keep);
            true
        }
        Target::Image | Target::Empty => {
            o.probe("SW_CMD", Some(&format!("n={}", n)), Some(&c), &Res::ok(&[]), false);
            // SECNR through the window (ide.rs `window_secnr`, `atapi::swap_secnr`, R144 b).
            let medium = target == Target::Image;
            match rp {
                Some(rp) => regpoll_lines(o, rp, medium, disc_format),
                None => {
                    for phase in [atapi::SwapPhase::Open, atapi::SwapPhase::Detect] {
                        let v = atapi::swap_secnr(phase, medium, disc_format);
                        sw_poll(o, &Res::not_ready(), &fmt(atapi::not_ready_asc(phase), nr), Some(v));
                    }
                }
            }
            let done = Some(atapi::swap_secnr(atapi::SwapPhase::Done, medium, disc_format));
            ready_polls(o, medium, done)
        }
    };
    o.event("SW_RESULT", &format!("n={} rc={} t_ms=- polls=- changes=-", n, if ready { "ok" } else { "timeout" }));
}

/// The polls once a window (switch or re-detect) is over: with a medium its UNIT ATTENTION, then READY; without
/// one NOT READY 2/3A/00 until gdprobe's timeout. Returns whether it became READY.
fn ready_polls(o: &mut Out, medium: bool, secnr: Option<u8>) -> bool {
    let fmt = |(asc, ascq): (u8, u8), key: u8| format!("{}/{:02x}/{:02x}", key, asc, ascq);
    if medium {
        sw_poll(o, &Res::ua(), &fmt(atapi::ASC_MEDIUM_CHANGED, atapi::SENSE_UNIT_ATTENTION), secnr);
        sw_poll(o, &Res::ok(&[]), "none", secnr);
        true
    } else {
        sw_poll(o, &Res::not_ready(), &fmt(atapi::not_ready_asc(atapi::SwapPhase::Done), atapi::SENSE_NOT_READY), secnr);
        false
    }
}

/// What a section for the folder already mounted does (gdprobe, after a switch and its battery).
#[derive(Clone, Copy, PartialEq)]
enum Same {
    /// `R<ms>_` (records 15/16): `52 82 n` again, a DEVICE RESET `ms` into the register poll.
    ReswitchReset(u32),
    /// `RS_` (seq 17): `52 82 n` again, no reset.
    Reswitch,
    /// `DR_` (seq 17): 500 ms idle, no switch command, a DEVICE RESET.
    ResetOnly,
}

/// A same-folder section as NetGD answers it now (R159, seq 17). The re-switch is a no-op (ide.rs
/// `gdemu_mount_staged`, `atapi::same_folder_reswitch`): `52 82` completes with 0x50 / 0x00, no window, SECNR flat,
/// no UNIT ATTENTION. A DEVICE RESET re-detects (`devreset_lines`) and raises the UNIT ATTENTION with a medium.
/// Ends with REQ_STAT / REQ_ERROR. `disc` None = the empty folder.
fn same_folder_lines(o: &mut Out, n: u32, kind: Same, detect_ms: u32, disc_format: u8, err_lo: bool, disc: Option<&Disc>) {
    let medium = disc.is_some();
    let old = o.secnr;
    match kind {
        Same::ResetOnly => o.event("IDLE", "idle_ms=500"),
        _ => {
            o.probe("SW_CMD", Some(&format!("n={}", n)), Some(&cdb_mount(n)), &Res::ok(&[]), false);
            let err = o.err_reg;
            regpoll_line(o, 0x50, err, 0x03, old);
        }
    }
    let reset = match kind {
        Same::ReswitchReset(ms) => Some(ms),
        Same::ResetOnly => Some(0),
        Same::Reswitch => None,
    };
    let ready = match reset {
        Some(ms) => {
            let done = devreset_lines(o, ms, detect_ms, medium, disc_format, err_lo);
            ready_polls(o, medium, Some(done))
        }
        None => {
            o.event("SW_REGPOLL_END", "t_ms=- samples=- changes=- stop=time");
            if medium {
                sw_poll(o, &Res::ok(&[]), "none", old);
            } else {
                let (asc, ascq) = atapi::ASC_NO_MEDIUM;
                sw_poll(o, &Res::not_ready(), &format!("{}/{:02x}/{:02x}", atapi::SENSE_NOT_READY, asc, ascq), old);
            }
            medium
        }
    };
    let mut f = format!("n={} rc={} t_ms=- polls=- changes=-", n, if ready { "ok" } else { "timeout" });
    if !matches!(kind, Same::ReswitchReset(_)) {
        // gdprobe's seq 17 sections say whether a 6/28/00 came.
        f.push_str(&format!(" ua_28={}", (reset.is_some() && medium) as u8));
    }
    o.event("SW_RESULT", &f);
    state_lines(o, disc, "");
}

fn main() {
    let opt = parse_args();
    let mut o = Out {
        s: String::new(),
        img: opt.img,
        last_check: opt.prev_check,
        secnr: opt.prev_secnr,
        pfx: String::new(),
        err_reg: opt.prev_err,
        old_bs: !opt.err_lo,
        // A switch to another folder mounts afresh (ide.rs `mounted`): the head is parked again (a refused one
        // keeps it).
        head: if opt.switch && opt.devreset_ms.is_none() && !opt.missing_folder { None } else { opt.prev_head },
        tno: if opt.switch && opt.devreset_ms.is_none() && !opt.missing_folder { 0 } else { opt.prev_tno },
    };

    let disc = match &opt.image {
        Some(p) => match load_disc(p) {
            Ok(mut d) => {
                d.form2_sectors = opt.form2_sectors;
                Some(d)
            }
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
        let fmt = disc.as_ref().map_or(0, |d| atapi::disc_format(&d.tracks, d.is_cdi));
        match opt.devreset_ms {
            // gdprobe resets only after an accepted switch, to the folder already mounted.
            Some(r) if target != Target::Missing => {
                o.pfx = format!("R{}_", r);
                same_folder_lines(&mut o, opt.img as u32, Same::ReswitchReset(r), opt.window.1, fmt, opt.err_lo, disc.as_ref());
            }
            _ => {
                let rp = (opt.regpoll || opt.devreset_ms.is_some())
                    .then(|| RegPoll { open_ms: opt.window.0, detect_ms: opt.window.1 });
                switch_lines(&mut o, opt.img as u32, target, fmt, rp.as_ref());
            }
        }
        o.pfx.clear();
    } else if opt.reswitch || opt.devreset_only {
        let fmt = disc.as_ref().map_or(0, |d| atapi::disc_format(&d.tracks, d.is_cdi));
        let (pfx, kind) = if opt.reswitch { ("RS_", Same::Reswitch) } else { ("DR_", Same::ResetOnly) };
        o.pfx = pfx.into();
        same_folder_lines(&mut o, opt.img as u32, kind, opt.window.1, fmt, opt.err_lo, disc.as_ref());
        o.pfx.clear();
    } else if let Some(d) = opt.devreset_cmd_ms {
        let fmt = disc.as_ref().map_or(0, |d| atapi::disc_format(&d.tracks, d.is_cdi));
        o.pfx = format!("DC{}_", d);
        devreset_cmd_lines(&mut o, d, opt.window.1, fmt, opt.err_lo, disc.as_ref());
        o.pfx.clear();
    }
    if !opt.sw_only && !opt.reswitch && !opt.devreset_only && opt.devreset_cmd_ms.is_none() {
        match &disc {
            Some(d) => {
                // A switch consumes the UNIT ATTENTION in its polls; a bare run on a freshly booted unit sees it.
                let ua = opt.ua.unwrap_or(!opt.switch);
                // --switch: the battery runs after a switch (pause, park position); without it, on a unit
                // fresh from power-on (standby, FAD 150). A refused switch (--missing-folder) keeps the old
                // image, which expect_sd.py only does after an earlier switch.
                let ht = battery(&opt, d, &mut o, ua, opt.switch);
                if opt.head_probe {
                    head_probe(&opt, d, &mut o, ht);
                }
                if opt.gap_probe || opt.pregap_probe || opt.ctl_probe {
                    gap_probe(&opt, d, &mut o);
                }
            }
            None => battery_no_medium(&mut o),
        }
    }
    if opt.end {
        o.event("END", "abort=0");
    }
    // Carried into the next section by expect_sd.py (and dropped from the record there).
    o.s.push_str(&format!("# expect-state last_check={} err=0x{:02x}", o.last_check as u8, o.err_reg));
    if let Some(h) = o.head {
        o.s.push_str(&format!(" head={}", h));
    }
    if o.tno != 0 {
        o.s.push_str(&format!(" tno={}", o.tno));
    }
    if let Some(v) = o.secnr {
        o.s.push_str(&format!(" secnr=0x{:02x}", v));
    }
    o.s.push('\n');
    print!("{}", o.s);
}
