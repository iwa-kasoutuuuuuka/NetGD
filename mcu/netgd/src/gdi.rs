//! GDI parser + LBA-to-file map.
//!
//! GDI ("GD-ROM Image") is the de-facto Dreamcast disc-image format. The
//! `.gdi` text file at the root of the image directory describes the
//! tracks; each track points at a binary file holding raw sectors.
//!
//! Layout of a typical 4-track GD-ROM image:
//!
//! ```text
//! 4
//! 1 0 4 2352 track01.bin 0
//! 2 600 0 2352 track02.raw 0
//! 3 45000 4 2352 track03.bin 0
//! 4 549150 0 2352 track04.raw 0
//! ```
//!
//! Columns: track-number, start-LBA, ctrl-bits, sector-size, filename,
//! pregap (we ignore pregap for now). Tracks 1-2 live in the SDA (Single
//! Density Area, LBAs 0..11400), track 3+ live in the HDA (High Density
//! Area, starts at LBA 45000 by Sega convention).

use defmt::{info, warn, Format};
use embedded_sdmmc::{Directory, ShortFileName, VolumeManager};
use heapless::{String, Vec};

use crate::blockdev::{BlockError, HsmciBlockDevice};

type RootDir<'a> = Directory<'a, HsmciBlockDevice, crate::blockdev::DummyClock, 4, 4, 1>;

/// Maximum tracks we ever expect. Red Book technically caps at 99 but
/// Red Book's limit, and the real one.
///
/// This was 32, justified by "real Dreamcast GDIs are <=22 tracks". That is
/// not true: seventeen of the 298 US retail discs exceed 32, and Tom Clancy's
/// Rainbow Six - Rogue Spear has 98. They are the CDDA-heavy ones, so the
/// more music a game shipped with the less likely it was to mount at all --
/// `TooManyTracks`, surfacing to the console as an ILLEGAL REQUEST on MOUNT.
///
/// The 32 did buy something real, though: a `Vec<Track, 99>` is about 4 KB
/// and this part has roughly 2 KB of stack, so the tracks cannot live in a
/// `Gdi` that is built as a local and returned by value. Hence [`TRACKS`].
pub const MAX_TRACKS: usize = 99;
/// Max GDI file size we'll read. A 4-track GDI is ~80 bytes; a 99-track one
/// with Redump-length names (up to 128 bytes, `discimg::MAX_NAME`) is about
/// 11 KB. It is the first part of `ide::loader_scratch()` (14 KB, no new RAM);
/// the rest is the FAT reader's block buffer. A file that fills it is refused
/// (`parse_image`), not read short.
const MAX_GDI_BYTES: usize = 12 * 1024;

#[derive(Clone, Format)]
pub struct Track {
    pub number: u8,
    pub start_lba: u32,
    pub ctrl: u8,
    pub sector_size: u16,
    /// Length in sectors. Derived from the track file's size on disk
    /// (`file_size / sector_size`). Used to compute the lead-out FAD
    /// reported in the TOC.
    pub sector_count: u32,
    /// 8.3 short file name as embedded-sdmmc reports it. Used to open
    /// the track binary at read time.
    pub filename: ShortFileName,
    /// Byte offset within `filename` at which this track's `start_lba`
    /// sector begins. GDI keeps one file per track so this is always 0;
    /// CDI concatenates every track into a single file, so each track
    /// sits at its own base offset (past the preceding tracks, and past
    /// its own pregap).
    pub file_offset: u32,
    /// Slice of the shared extent pool (`extents()`) describing where
    /// `filename` sits on the card. With `ext_len == 0`, `ext_start` is
    /// either 0 (the file could not be mapped; reads go through the
    /// filesystem) or the 1-based slot of a *lazy* file (too fragmented for
    /// the pool, read through `fat::lazy_locate`). Use `is_mapped`.
    pub ext_start: u16,
    pub ext_len: u16,
    /// CDI: 1-based session number (`discimg::TrackDesc::session`), for
    /// REQ_SES(n); 0 for a GDI. Fits in the struct's tail padding.
    pub session: u8,
}

/// Block-run map of every mapped track file, filled by `load_image`.
static mut EXTENTS: Vec<crate::fat::Extent, { crate::fat::MAX_EXTENTS }> = Vec::new();

/// The mounted image's tracks.
///
/// A static pool rather than a field of [`Gdi`], because 99 of them is about
/// 4 KB and this part has about 2 KB of stack -- as a field they would be
/// built as a local in [`parse`] and moved through two more frames on the way
/// to `ide::GDI`, which is the deepest, most stack-fragile path there is.
///
/// One pool, because one image is mounted at a time. A load that fails part
/// way leaves it holding rubbish, which is safe only because a failed mount
/// already clears the mounted image (`ide.rs`, `SUB_MOUNT`): the console is
/// told the media is gone rather than being served half of the last disc.
static mut TRACKS: Vec<Track, MAX_TRACKS> = Vec::new();

/// Everything currently in the pool.
fn tracks() -> &'static [Track] {
    unsafe { &*core::ptr::addr_of!(TRACKS) }
}

pub(crate) fn tracks_mut() -> &'static mut Vec<Track, MAX_TRACKS> {
    unsafe { &mut *core::ptr::addr_of_mut!(TRACKS) }
}

impl Track {
    /// The track file's block runs, if it was mapped at load time.
    pub fn extents(&self) -> Option<&'static [crate::fat::Extent]> {
        if self.ext_len == 0 {
            return None;
        }
        let pool = unsafe { &*core::ptr::addr_of!(EXTENTS) };
        pool.get(self.ext_start as usize..(self.ext_start + self.ext_len) as usize)
    }

    /// Whether `locate_byte` can answer for this track: its runs are in the
    /// pool, or it is a lazy file. False means reads fall back to the
    /// filesystem, which is an order of magnitude slower.
    pub fn is_mapped(&self) -> bool {
        self.ext_len != 0 || self.ext_start != 0
    }

    /// Byte offset within `filename` of disc LBA `lba`.
    pub fn byte_offset_of(&self, lba: u32) -> Option<u32> {
        lba.checked_sub(self.start_lba)?
            .checked_mul(self.sector_size as u32)?
            .checked_add(self.file_offset)
    }

    /// SD block holding file byte `byte_off` of this track's file:
    /// `(block, offset within the block, contiguous blocks from there)`.
    /// `None` if the track is unmapped or the offset lies past its runs.
    pub fn locate_byte(&self, byte_off: u32) -> Option<(u32, u32, u32)> {
        if self.ext_len != 0 {
            crate::fat::locate(self.extents()?, byte_off)
        } else if self.ext_start != 0 {
            // May read FAT sectors; the data path is synchronous here, as it
            // is for the CMD18 reads it feeds.
            crate::fat::lazy_locate(self.ext_start as u8, byte_off)
        } else {
            None
        }
    }

    /// Byte offset of the 2048-byte user data within one stored sector.
    /// 2352 is a raw sector (12 sync + 4 header) and 2336 is the CDI
    /// data form, which drops sync and header but keeps the 8-byte
    /// Mode 2 subheader. 2048 is already cooked.
    pub fn user_data_offset(&self) -> usize {
        match self.sector_size {
            2352 => 16,
            2336 => 8,
            _ => 0,
        }
    }

    #[allow(dead_code)]
    pub fn is_data(&self) -> bool {
        // Ctrl bit 2 set → data track (CD-ROM Mode 1/2). Audio = bit 2 = 0.
        (self.ctrl & 0x04) != 0
    }
}

#[derive(Format)]
pub struct Gdi {
    /// LBA where the bootable area starts (typically 45000).
    ///
    /// Named for what it is on both kinds of image rather than for the
    /// GD-ROM's name for it: on a GDI this is the start of the High Density
    /// Area, and on a CDI it is the start of session 2 -- a MIL-CD has no
    /// high-density area at all. Both are the same thing to everything that
    /// reads this: where IP.BIN lives, which tracks sit above the split, and
    /// which track REQ_SES calls the second session's first.
    pub boot_area_lba: u32,
    /// Path from the card root to the folder holding the .gdi file and the
    /// per-track binaries. Stored so the IDE-side data path can re-open it
    /// on every CD_READ without re-walking the volume.
    pub image_dir: ImagePath,
    /// SD card block count, captured at parse time so the IDE task can
    /// rebuild a VolumeManager on demand without re-querying CSD.
    pub num_blocks: u32,
    /// True when this image came from a `.cdi` rather than a `.gdi`.
    ///
    /// A CDI here means a MIL-CD, and it is presented as what it is: a CD,
    /// not a GD-ROM. That changes three things, all of which have to agree or
    /// the BIOS abandons the boot -- the disc format reported in SECNR
    /// (CD-ROM XA, see `ide::disc_format`), the TOC (every track in the
    /// single-density area, since a CD has no high-density one), and the
    /// lead-out (the truth; a GDI now reports the end of its last track too,
    /// as stock GDEMU 6.0 does, D1). See [`Gdi::leadout_lba`] and
    /// [`Gdi::build_toc`].
    pub is_cdi: bool,
    /// How many sessions the disc has.
    ///
    /// Reported to the host as REQ_SES(0)'s track byte, which is where the
    /// BIOS reads the session *count* -- not, as was assumed here, the first
    /// track number. The two are indistinguishable on a GD-ROM, which stock
    /// GDEMU reports as a single session whose first track is also 1; they
    /// differ on a MIL-CD, which has two sessions and keeps everything
    /// bootable in the second. Saying 1 there tells the BIOS session 2 does
    /// not exist, and it stops looking. Verified against flycast
    /// `core/imgread/common.h` (`GetSessionInfo`: `to[2] = sessions.size()`).
    pub sessions: u8,
}

impl Gdi {
    /// This image's tracks.
    ///
    /// They live in [`TRACKS`], not in here: see that static for why. The
    /// consequence to know is that a `Gdi` kept across a mount describes the
    /// *new* image's tracks, so there is no point holding one -- ask
    /// `ide::gdi_ref()` each time, which everything already does.
    pub fn tracks(&self) -> &'static [Track] {
        tracks()
    }

    /// Track containing `lba`, if any. Linear scan since N ≤ 99.
    pub fn track_for_lba(&self, lba: u32) -> Option<&Track> {
        discimg::atapi::track_for_lba(self.tracks(), lba)
    }

    /// `(filename, byte_offset)` for reading `lba`. Caller opens the
    /// file by name and seeks to the offset.
    #[allow(dead_code)]
    pub fn locate(&self, lba: u32) -> Option<(ShortFileName, u32)> {
        let t = self.track_for_lba(lba)?;
        let sector = lba - t.start_lba;
        let offset = sector
            .checked_mul(t.sector_size as u32)?
            .checked_add(t.file_offset)?;
        Some((t.filename.clone(), offset))
    }

    /// First and last track numbers (used by READ_TOC).
    #[allow(dead_code)]
    pub fn track_bounds(&self) -> Option<(u8, u8)> {
        let first = self.tracks().first()?.number;
        let last = self.tracks().last()?.number;
        Some((first, last))
    }

    /// Total disc length in LBAs, used as the lead-out address: the last
    /// track's start + sector count.
    ///
    /// A GDI used to report a pressed GD-ROM's capacity, LBA 549150. Stock
    /// GDEMU 6.0 reports the end of the last track instead (D1: GDI 03, last
    /// track 45000..64999, lead-out FAD 65150), so a GDI does too now; its
    /// sector counts are the track files' sizes / sector size (filled by
    /// [`parse_image`] when the image mounts).
    ///
    /// For a CDI, reporting 549150 is a lie the BIOS catches. The
    /// DiscJuggler descriptor gives an exact sector count per
    /// track, so the real end of the disc is known; a MIL-CD's data session
    /// ends where its last track ends, nowhere near a pressed disc's
    /// capacity. Claiming otherwise describes a GD-ROM whose high-density
    /// area runs ~192000 sectors past its own last track, and the BIOS
    /// abandons the boot and starts over rather than load anything -- it
    /// never reads past the ISO9660 root directory.
    #[allow(dead_code)]
    pub fn leadout_lba(&self) -> u32 {
        discimg::atapi::leadout_lba(self.tracks(), self.is_cdi)
    }

    /// Build a 408-byte Sega GD-ROM TOC response. `area` selects which
    /// session of the disc to describe:
    ///   0 = Single Density Area (tracks with LBA < 45000)
    ///   1 = High  Density Area (tracks with LBA >= 45000)
    ///
    /// Layout: 99 4-byte entries (tracks 1..99), then first-track,
    /// last-track, and lead-out entries — also 4 bytes each. Each
    /// entry is `[CTRL_ADR, FAD_hi, FAD_mid, FAD_lo]` where CTRL_ADR
    /// = `(ctl << 4) | 1` (CTL high nibble, ADR=1 low nibble — the
    /// Sega/iceGDROM convention, opposite to what the standard ATAPI
    /// READ_TOC uses). Unused track slots are 0xFF-filled.
    pub fn build_toc(&self, area: u8, out: &mut [u8; 408]) {
        discimg::atapi::build_toc(self.tracks(), self.is_cdi, self.boot_area_lba, area, out)
    }
}

/// How deep an image may sit below the card root: the game's own folder,
/// plus room for a couple of levels above it.
pub const MAX_PATH_PARTS: usize = 3;

/// Where an image's folder is, as the components to walk from the root.
///
/// Not a single name. The host names an image by path -- `us/Crazy Taxi …`
/// -- because a card holding several hundred discs wants them grouped, and
/// nothing here needs to know what the grouping means: it walks whatever it
/// is handed. A path of one component is the old behaviour exactly.
pub type ImagePath = Vec<ShortFileName, MAX_PATH_PARTS>;

/// Walk `path` from `dir`, which becomes the image's directory.
///
/// `change_dir` rather than `open_dir` because the latter returns a
/// directory borrowing the one it came from, which cannot be walked in a
/// loop -- and because descending in place holds one directory slot rather
/// than one per level.
fn descend<D, T, const A: usize, const B: usize, const C: usize>(
    dir: &mut embedded_sdmmc::Directory<'_, D, T, A, B, C>,
    path: &ImagePath,
) -> Result<(), GdiError>
where
    D: embedded_sdmmc::BlockDevice,
    T: embedded_sdmmc::TimeSource,
{
    for part in path.iter() {
        dir.change_dir(part.clone())
            .map_err(|_| GdiError::OpenImageDir)?;
    }
    Ok(())
}

/// One selectable disc image: the directory holding it and the descriptor
/// file inside, plus which format that file is.
#[derive(Clone, Format)]
pub struct ImageRef {
    pub dir: ImagePath,
    pub file: ShortFileName,
    pub is_cdi: bool,
    /// The directory's long name when it has one, else its 8.3 name. This is
    /// what the config matches on and what the host is shown.
    ///
    /// Matching on the 8.3 alias would be fragile: `SEGARA~1` is whatever the
    /// tool that created the directory happened to pick, and a different
    /// collision order on another machine yields `SEGARA~2` for the same disc,
    /// silently breaking the config. Files are still *opened* by short name --
    /// embedded-sdmmc reads long names but does not look up by them.
    pub name: String<MAX_NAME_CHARS>,
}

/// Longest image name we keep.
///
/// A name is a path now, and the folder at the end of it is a full retail
/// title -- "Alone in the Dark - The New Nightmare v1.000 (2001)(Infogrames)
/// (US)(Disc 2 of 2)[!]" is 83 characters before any shelf in front of it.
/// At 64 those names truncated, and two discs of the same game truncated to
/// the *same* string, so a mount picked whichever came first.
pub const MAX_NAME_CHARS: usize = gamecfg::MAX_NEXT;

/// Scratch for reading long file names. Shared by the two places that walk
/// a directory -- they never run at the same time, and this part has about
/// two kilobytes of stack to its name, so a second copy is not free.
static mut LFN_STORE: [u8; 4 * MAX_NAME_CHARS] = [0; 4 * MAX_NAME_CHARS];


/// Config file at the root of the card. 8.3 only -- the DC-side tools build
/// FatFs with FF_USE_LFN=0, and "OPENGDEMU.CFG" has a nine-character base
/// name, so it cannot even be opened there.
const CONFIG_NAME: &str = "GDEMU.CFG";
const MAX_CONFIG_BYTES: usize = 512;

/// The original GDEMU's settings file, same place, same 8.3 rule.
const INI_NAME: &str = "GDEMU.INI";

/// Scratch for reading the two root config files. Never both at once: each
/// reader finishes with it before returning.
static mut CFG: [u8; MAX_CONFIG_BYTES] = [0; MAX_CONFIG_BYTES];

/// The settings from the card's `GDEMU.INI`, as last read.
static mut INI: gamecfg::ini::Ini = gamecfg::ini::Ini::new();

/// The current `GDEMU.INI` settings (official defaults until a card with one
/// is seen).
pub fn ini() -> gamecfg::ini::Ini {
    unsafe { *core::ptr::addr_of!(INI) }
}

/// Re-read `GDEMU.INI` from the card root. Called whenever the card is
/// (re)scanned, as the original firmware re-reads it on card detection, and
/// by the IDE task before a hard-reset `reset_goto` and a write-session
/// remount (the file may have changed under it). A card without the file
/// leaves the previous settings in place.
///
/// The whole file is parsed, whatever its size, through the shared `CFG`
/// buffer: `Ini::parse_reader` keeps the unfinished line at the front of it
/// and reads on, so no extra stack is needed.
pub fn reload_ini(num_blocks: u32) {
    let dev = HsmciBlockDevice::new();
    dev.set_num_blocks(num_blocks);
    let mgr = VolumeManager::new(dev, crate::blockdev::DummyClock);
    let Ok(volume) = mgr.open_volume(embedded_sdmmc::VolumeIdx(0)) else { return };
    let Ok(root) = volume.open_root_dir() else { return };
    let Ok(name) = ShortFileName::create_from_str(INI_NAME) else { return };
    let Ok(f) = root.open_file_in_dir(&name, embedded_sdmmc::Mode::ReadOnly) else {
        info!("gdi: no GDEMU.INI, keeping the previous settings");
        return;
    };
    let buf = unsafe { &mut *core::ptr::addr_of_mut!(CFG) };
    // A read error ends the file where it stands.
    let ini = gamecfg::ini::Ini::parse_reader(buf, |b| f.read(b).unwrap_or(0));
    info!(
        "gdi: GDEMU.INI open_time={} (set {}) detect_time={} (set {}) reset_goto={}",
        ini.open_time, ini.open_time_set, ini.detect_time, ini.detect_time_set, ini.reset_goto
    );
    if ini.unknown != 0 || ini.read_limit != 0 || ini.high_speed || !ini.image_tests {
        info!(
            "gdi: GDEMU.INI: image_tests/high_speed/read_limit are not implemented ({} unknown keys)",
            ini.unknown
        );
    }
    unsafe { INI = ini };
}

/// How a directory entry is named for matching and display.
///
/// The long filename when there is one, else the 8.3 name. A directory whose
/// name already fits 8.3 and needs no case preservation -- LAUNCH, MENU --
/// has no long-filename entry at all, so `long` is `None` for it. Deriving
/// this in one place is the point: `enumerate_images` had the fallback and
/// `load_by_name` did not, which made those folders listable but impossible
/// to mount by name (`NoGdiFile`, since no directory ever matched).
fn entry_name(entry: &embedded_sdmmc::DirEntry, long: Option<&str>) -> String<MAX_NAME_CHARS> {
    let mut name: String<MAX_NAME_CHARS> = String::new();
    match long {
        Some(l) if !l.is_empty() => {
            for c in l.chars() {
                if name.push(c).is_err() {
                    break;
                }
            }
        }
        _ => {
            for b in entry.name.base_name() {
                let _ = name.push(*b as char);
            }
            if !entry.name.extension().is_empty() {
                let _ = name.push('.');
                for b in entry.name.extension() {
                    let _ = name.push(*b as char);
                }
            }
        }
    }
    name
}


/// Read a folder's `GAME.CFG`, or the defaults if it has none.
///
/// A folder without one is an ordinary single disc, which is what
/// `Meta::default` means -- so a card that predates this file keeps working
/// exactly as it did.
fn read_game_cfg<D, T, const A: usize, const B: usize, const C: usize>(
    dir: &embedded_sdmmc::Directory<'_, D, T, A, B, C>,
) -> gamecfg::Meta
where
    D: embedded_sdmmc::BlockDevice,
    T: embedded_sdmmc::TimeSource,
{
    let Ok(f) = dir.open_file_in_dir(gamecfg::FILENAME, embedded_sdmmc::Mode::ReadOnly) else {
        return gamecfg::Meta::default();
    };
    // Static rather than stack: this runs in the same frame that later holds
    // the GDI text buffer, and the two together used to overflow the task.
    //
    // Small on purpose. These files hold a handful of keys, and this part has
    // about two kilobytes of RAM to spare in total -- a 1 KB buffer here was
    // a quarter of it.
    static mut CFG_BUF: [u8; 256] = [0; 256];
    let buf = unsafe { &mut *core::ptr::addr_of_mut!(CFG_BUF) };
    let got = f.read(buf).unwrap_or(0);
    gamecfg::Meta::parse(core::str::from_utf8(&buf[..got]).unwrap_or(""))
}

/// The image named by `GDEMU.CFG` at the root of the card, if it names one.
///
/// Returns the *name*, not an index into anything. Mounting is by name all
/// the way down -- `load_by_name` finds the one directory it needs -- so
/// there is nothing here for an index to be an index into, and resolving one
/// would mean walking the whole card first.
pub fn configured_name(num_blocks: u32) -> Option<String<MAX_NAME_CHARS>> {
    let dev = HsmciBlockDevice::new();
    dev.set_num_blocks(num_blocks);
    let mgr = VolumeManager::new(dev, crate::blockdev::DummyClock);
    let volume = mgr.open_volume(embedded_sdmmc::VolumeIdx(0)).ok()?;
    let root = volume.open_root_dir().ok()?;
    let name = ShortFileName::create_from_str(CONFIG_NAME).ok()?;

    let buf = unsafe { &mut *core::ptr::addr_of_mut!(CFG) };
    let len = read_file_to_buf(&root, &name, buf).ok()?;

    for line in buf[..len].split(|c| *c == b'\n') {
        let line = trim_ascii(line);
        if line.is_empty() || line[0] == b'#' {
            continue;
        }
        let mut parts = line.splitn(2, |c| *c == b'=');
        let key = trim_ascii(parts.next()?);
        let val = trim_ascii(parts.next()?);
        if !key.eq_ignore_ascii_case(b"image") {
            continue;
        }
        let mut out: String<MAX_NAME_CHARS> = String::new();
        for b in val {
            if out.push(*b as char).is_err() {
                break;
            }
        }
        info!("gdi: config selects '{}'", out.as_str());
        return Some(out);
    }
    None
}

fn trim_ascii(mut s: &[u8]) -> &[u8] {
    while let Some((f, rest)) = s.split_first() {
        if f.is_ascii_whitespace() { s = rest; } else { break; }
    }
    while let Some((l, rest)) = s.split_last() {
        if l.is_ascii_whitespace() { s = rest; } else { break; }
    }
    s
}

/// The enumerated images, and which one is mounted.
///
/// The selection lives in RAM only, on purpose. `/OPENGDEMU.CFG` supplies the
/// boot default; swapping discs at runtime changes this and nothing else, so a
/// disc change never costs the card a write. Power-cycling returns to the
/// configured default.
///
/// It is held as a *name* rather than an index because the list is rebuilt on
/// every eject/load cycle -- that is the whole point of the eject window, the
/// host may have added or removed images while it was open -- and an index
/// would silently come to mean a different disc.
static mut SELECTED_DIR: Option<ImagePath> = None;
/// The mounted image under the name a host would use for it, so a remount can
/// go back through `load_by_name` without a catalog to look an index up in.
static mut SELECTED_NAME: Option<String<MAX_NAME_CHARS>> = None;

fn remember(dir: ImagePath, name: &str) {
    let mut n: String<MAX_NAME_CHARS> = String::new();
    for c in name.chars() {
        if n.push(c).is_err() {
            break;
        }
    }
    unsafe {
        SELECTED_DIR = Some(dir);
        SELECTED_NAME = Some(n);
    }
}





/// Where [`load_default`] looks for the image to mount, in the order it
/// tries them.
#[derive(Clone, Copy)]
enum DefaultSource {
    /// The runtime choice ([`SELECTED_NAME`]).
    Selected,
    /// `GDEMU.CFG` `image=` ([`configured_name`]).
    Configured,
    /// Root folder 01 ([`folder_for_number`]).
    Folder01,
}

/// The name `source` gives, if it gives one.
///
/// Out of line on purpose: `configured_name` and `folder_for_number` each open
/// the card (a volume manager with its block cache, about a kilobyte), and
/// inlined into `load_default` that frame stayed on the stack underneath every
/// `load_by_name` it then made -- the deepest path in the firmware. Here it is
/// gone again before the mount starts, and `load_default` keeps only the name.
#[inline(never)]
fn default_name(num_blocks: u32, source: DefaultSource) -> Option<String<MAX_NAME_CHARS>> {
    match source {
        DefaultSource::Selected => unsafe { &*core::ptr::addr_of!(SELECTED_NAME) }.clone(),
        DefaultSource::Configured => configured_name(num_blocks),
        DefaultSource::Folder01 => folder_for_number(num_blocks, 1),
    }
}

/// Load whichever image should be mounted now: the runtime choice if there is
/// one, else whatever `GDEMU.CFG` names, else folder 01. A card with none of
/// the three comes up empty.
///
/// Both are resolved by name, so mounting opens one directory. Nothing here
/// walks the card or builds a list of images: which disc to serve is the
/// host's business, and the host picks it by name over 0xE0.
///
/// Each source is asked only once the one before it has failed to mount
/// (`default_name`), and only the winning name is held across the mount.
pub fn load_default(num_blocks: u32) -> Result<Gdi, GdiError> {
    // A card scan: pick up GDEMU.INI first, so its settings are current for
    // whatever mounts below. `GDEMU.CFG` `image=` still decides which image.
    reload_ini(num_blocks);
    for source in [DefaultSource::Selected, DefaultSource::Configured, DefaultSource::Folder01] {
        let Some(name) = default_name(num_blocks, source) else { continue };
        let e = match load_by_name(num_blocks, name.as_str()) {
            Ok(gdi) => return Ok(gdi),
            Err(e) => e,
        };
        match source {
            // The card may have changed under an open tray; fall through
            // rather than refusing to mount anything at all.
            DefaultSource::Selected => {
                warn!("gdi: selected image '{}' is gone ({:?})", name.as_str(), e)
            }
            DefaultSource::Configured => warn!(
                "gdi: config names '{}', which is not on the card ({:?})",
                name.as_str(),
                e
            ),
            // Nothing named, so do what the original GDEMU does on a card
            // insert: mount folder 01 (where GDmenu / openMenu usually live).
            // This is a lookup of one numbered folder, not a guess at "some
            // image" -- a card without a folder 01 still comes up empty and
            // the host sees no disc.
            DefaultSource::Folder01 => {
                warn!("gdi: default folder '{}' is unusable ({:?})", name.as_str(), e)
            }
        }
    }
    warn!("gdi: no image selected, no usable GDEMU.CFG and no folder 01 -- coming up empty");
    Err(GdiError::NoGdiFile)
}

/// Call `f(number, entry)` for every numbered image folder in the root.
///
/// Numbered means [`discimg::slot::number_of`] accepts the folder's 8.3 base
/// name and it has no extension. One pass, no collection of the entries.
fn for_each_slot<F: FnMut(u16, &embedded_sdmmc::DirEntry)>(num_blocks: u32, mut f: F) {
    let dev = HsmciBlockDevice::new();
    dev.set_num_blocks(num_blocks);
    let mgr = VolumeManager::new(dev, crate::blockdev::DummyClock);
    let Ok(volume) = mgr.open_volume(embedded_sdmmc::VolumeIdx(0)) else { return };
    let Ok(root) = volume.open_root_dir() else { return };
    let _ = root.iterate_dir(|entry| {
        if !entry.attributes.is_directory() || !entry.name.extension().is_empty() {
            return;
        }
        if let Some(n) = discimg::slot::number_of(entry.name.base_name()) {
            f(n, entry);
        }
    });
}

/// The root folder numbered `n`, under the name `load_by_name` takes.
/// With several folders of the same number (`01` and `001`) the first on the
/// card wins.
pub fn folder_for_number(num_blocks: u32, n: u16) -> Option<String<MAX_NAME_CHARS>> {
    let mut found: Option<String<MAX_NAME_CHARS>> = None;
    for_each_slot(num_blocks, |num, entry| {
        if num == n && found.is_none() {
            found = Some(entry_name(entry, None));
        }
    });
    found
}

/// The number of the mounted folder, if it is a numbered one.
pub fn mounted_number() -> Option<u16> {
    let name = unsafe { &*core::ptr::addr_of!(SELECTED_NAME) }.as_ref()?;
    let name = gamecfg::trim(name.as_str()).trim_end_matches('/');
    if name.contains('/') {
        return None;
    }
    discimg::slot::number_of(name.as_bytes())
}

/// The next numbered folder in `dir` from the mounted one, wrapping, with
/// numbering gaps skipped. `None` when the card has no numbered folder.
///
/// One pass over the root picks the number ([`discimg::slot::Nearest`]),
/// a second fetches its name. Probing number by number instead would walk
/// the card once per gap -- a folder called `2024` next to `01` would cost
/// two thousand directory scans inside one ATAPI command.
///
/// Folders without a mountable image are skipped like gaps
/// ([`folder_has_image`]): stepping onto one would empty the drive for
/// nothing. A rejected pick is stepped on from, via
/// [`discimg::slot::step_accepted`], at most once per numbered folder. Cost:
/// each round is one root scan plus one name lookup (another scan) plus one
/// folder check, so `k` image-less folders in the way cost `k + 1` rounds --
/// the common card, where every numbered folder has an image, pays the single
/// round it always did plus the folder check.
pub fn step_folder(
    num_blocks: u32,
    dir: discimg::slot::Step,
) -> Option<String<MAX_NAME_CHARS>> {
    let mut found: Option<String<MAX_NAME_CHARS>> = None;
    discimg::slot::step_accepted(
        mounted_number(),
        dir,
        |pick| for_each_slot(num_blocks, |num, _| pick.offer(num)),
        |n| match folder_for_number(num_blocks, n) {
            Some(name) if folder_has_image(num_blocks, name.as_str()) => {
                found = Some(name);
                true
            }
            _ => false,
        },
    )?;
    found
}

/// Whether the folder called `name` (as `load_by_name` takes it) holds a
/// mountable image: a GDI or a CDI descriptor file.
///
/// Presence only -- nothing is parsed and no static is touched, so it is safe
/// to ask about a folder that is not about to be mounted. Mounting a folder
/// that fails this check empties the drive (see `mount_named`), which is
/// what callers use it to avoid.
pub fn folder_has_image(num_blocks: u32, name: &str) -> bool {
    let dev = HsmciBlockDevice::new();
    dev.set_num_blocks(num_blocks);
    let mgr = VolumeManager::new(dev, crate::blockdev::DummyClock);
    let Ok(volume) = mgr.open_volume(embedded_sdmmc::VolumeIdx(0)) else { return false };
    let Ok(mut d) = volume.open_root_dir() else { return false };
    let mut path = ImagePath::new();
    walk_to(&mut d, name, &mut path).is_ok() && locate_image(&d).is_some()
}

/// Whether `part` -- bytes the host or `GDEMU.CFG` sent, each widened to a
/// char -- spells `name`, a long name held as UTF-8. The ordinary comparison
/// sees two chars where the card has one non-ASCII char, so it never matches a
/// non-ASCII folder. ASCII case-insensitive, no buffer.
fn widened_eq(name: &str, part: &str) -> bool {
    let mut bytes = name.bytes();
    for c in part.chars() {
        let cp = c as u32;
        match bytes.next() {
            Some(b) if cp <= 0xFF && b.eq_ignore_ascii_case(&(cp as u8)) => {}
            _ => return false,
        }
    }
    bytes.next().is_none()
}

/// Resolve `want` (components split by `/`) from `d`, one level at a time
/// against the long names, leaving `d` in the folder and the 8.3 path in
/// `path`. Shared by `load_by_name` and `folder_has_image`.
#[inline(never)]
fn walk_to(d: &mut RootDir<'_>, want: &str, path: &mut ImagePath) -> Result<(), GdiError> {
    for part in want.split('/').filter(|p| !p.is_empty()) {
        let mut lfn =
            embedded_sdmmc::LfnBuffer::new(unsafe { &mut *core::ptr::addr_of_mut!(LFN_STORE) });
        let mut hit: Option<ShortFileName> = None;
        let _ = d.iterate_dir_lfn(&mut lfn, |entry, long| {
            if hit.is_some()
                || !entry.attributes.is_directory()
                || entry.name.base_name() == b"."
                || entry.name.base_name() == b".."
            {
                return;
            }
            let name = entry_name(entry, long);
            if gamecfg::resolve(core::iter::once(name.as_str()), part).is_some()
                || widened_eq(name.as_str(), part)
            {
                hit = Some(entry.name.clone());
            }
        });
        let Some(short) = hit else {
            return Err(GdiError::NoGdiFile);
        };
        if path.push(short.clone()).is_err() {
            warn!("gdi: '{}' is deeper than {} levels", want, MAX_PATH_PARTS);
            return Err(GdiError::NoGdiFile);
        }
        d.change_dir(short).map_err(|_| GdiError::OpenImageDir)?;
    }
    if path.is_empty() {
        return Err(GdiError::NoGdiFile);
    }
    Ok(())
}

/// The descriptor file in `d`, and whether it is a CDI (a GDI wins).
fn locate_image(d: &RootDir<'_>) -> Option<(ShortFileName, bool)> {
    match find_gdi(d) {
        Ok(n) => Some((n, false)),
        Err(_) => crate::cdi::find_cdi(d).ok().map(|n| (n, true)),
    }
}


/// The folder the mounted image's `GAME.CFG` names as `next`, read now
/// rather than remembered.
///
/// Opening the card for one file is cheap, and this runs from the button --
/// the shallowest caller there is. Keeping it instead would mean recording
/// it during a mount, which is the deep path this whole arrangement exists
/// to keep light.
pub fn next_of_mounted(num_blocks: u32) -> Option<String<{ gamecfg::MAX_NEXT }>> {
    let dir = unsafe { &*core::ptr::addr_of!(SELECTED_DIR) }.clone()?;
    let dev = HsmciBlockDevice::new();
    dev.set_num_blocks(num_blocks);
    let mgr = VolumeManager::new(dev, crate::blockdev::DummyClock);
    let volume = mgr.open_volume(embedded_sdmmc::VolumeIdx(0)).ok()?;
    let mut d = volume.open_root_dir().ok()?;
    descend(&mut d, &dir).ok()?;
    read_game_cfg(&d).next
}

/// Load the image in the folder called `want`, **without enumerating**.
///
/// The card is opened, the one directory is found by name, and its
/// descriptor is parsed. That is all a mount needs: which disc to serve is
/// the menu's business, and the menu can read the card itself.
///
/// The distinction matters more than it looks. `load_selected` calls
/// `enumerate_images`, which returns its sixteen-entry vector *by value* --
/// roughly 1.5 KB of stack on a part with about 2 KB of it in total. That
/// fits from `main_task` at boot and just fits from the host's MOUNT, but
/// from the button, deeper again and under a running game, it wedged the
/// firmware hard enough to take the debug port with it.
pub fn load_by_name(num_blocks: u32, want: &str) -> Result<Gdi, GdiError> {
    let which = resolve_name(num_blocks, want)?;
    load_image(num_blocks, &which)
}

/// The image in the folder called `want`: walk to it, remember it as the
/// selection, find its descriptor. The first half of [`load_by_name`].
///
/// Out of line, and done with the card before `load_image` opens it again:
/// inlined, this volume manager (block cache included, about a kilobyte)
/// stayed open on the stack under the whole parse, the deepest path in the
/// firmware. Only the small `ImageRef` crosses over.
#[inline(never)]
fn resolve_name(num_blocks: u32, want: &str) -> Result<ImageRef, GdiError> {
    let dev = HsmciBlockDevice::new();
    dev.set_num_blocks(num_blocks);
    let mgr = VolumeManager::new(dev, crate::blockdev::DummyClock);
    let volume = mgr
        .open_volume(embedded_sdmmc::VolumeIdx(0))
        .map_err(|_| GdiError::OpenVolume)?;
    let mut d = volume.open_root_dir().map_err(|_| GdiError::OpenRoot)?;

    // Resolve the path one component at a time (see `walk_to`). A name with no
    // separator walks exactly one level, which is what a card without shelves
    // has.
    let mut path = ImagePath::new();
    walk_to(&mut d, want, &mut path)?;
    // Remember it, so a later remount keeps this disc -- and before looking
    // inside: a folder with nothing to mount is still the selection (stock
    // accepts `52 82` to one, D12), so `52 81` and the button step on from
    // its number instead of from the disc before it (R18 item 1).
    remember(path.clone(), want);

    let Some((file, is_cdi)) = locate_image(&d) else {
        return Err(GdiError::NoGdiFile);
    };
    let mut name: String<MAX_NAME_CHARS> = String::new();
    for c in want.chars() {
        if name.push(c).is_err() {
            break;
        }
    }
    Ok(ImageRef {
        dir: path,
        file,
        is_cdi,
        name,
    })
}

/// Forget the runtime choice: the next load goes back to the configured
/// default (or the first image), exactly as a power cycle would.
pub fn select_default() {
    unsafe {
        SELECTED_DIR = None;
        SELECTED_NAME = None;
    }
    info!("gdi: selection reset to the configured default");
}

/// Load the image `which` names, ready to serve: descriptor parsed and every
/// track file mapped to its SD block runs.
///
/// Out of line for the same reason as `resolve_name`: [`load_by_name`] calls
/// the two one after the other, and inlined, this frame (the parse's volume
/// manager) was reserved for the whole of `load_by_name`, under
/// `resolve_name` too.
#[inline(never)]
pub fn load_image(num_blocks: u32, which: &ImageRef) -> Result<Gdi, GdiError> {
    let mut gdi = parse_image(num_blocks, which)?;
    map_track_extents(&mut gdi);
    Ok(gdi)
}

/// Parse `which` into a track table without touching the extent pool, so it
/// can be done for an image other than the mounted one (metadata lookups).
/// Borrows the CD_READ staging buffer for the descriptor text.
pub fn parse_image(num_blocks: u32, which: &ImageRef) -> Result<Gdi, GdiError> {
    let dev = HsmciBlockDevice::new();
    dev.set_num_blocks(num_blocks);

    let mgr = VolumeManager::new(dev, crate::blockdev::DummyClock);
    let volume = mgr
        .open_volume(embedded_sdmmc::VolumeIdx(0))
        .map_err(|_| GdiError::OpenVolume)?;
    let root = volume.open_root_dir().map_err(|_| GdiError::OpenRoot)?;

    let image_path = which.dir.clone();
    let image_file = which.file.clone();
    let is_cdi = which.is_cdi;
    info!("gdi: image '{}' file '{}'", which.name.as_str(), show_sfn(&image_file));
    let mut image_dir = root;
    descend(&mut image_dir, &image_path)?;

    // Prefer a .gdi; fall back to a .cdi. CDI keeps every track in one file
    // with the track map appended at the end, so it needs no companion track
    // binaries and its per-track sector counts come out of the descriptor
    // rather than from file sizes.
    let from_cdi = is_cdi;
    let mut gdi = if !is_cdi {
        {
            // Borrow the CD_READ staging buffer: nothing is streaming while
            // an image is being (re)loaded, and it saves keeping a 4 KB
            // buffer around for a file read once per boot. The first 4 KB
            // hold the text; the rest is the FAT reader's block buffer for
            // looking up long track names (`PoolSink`), so that costs no
            // stack under the volume manager.
            let scratch = crate::ide::loader_scratch();
            let (text, work) = scratch.split_at_mut(MAX_GDI_BYTES);
            let len = read_file_to_buf(&image_dir, &image_file, text)?;
            if len >= MAX_GDI_BYTES {
                // Parsing a cut-off list would drop the tracks after the cut.
                warn!("gdi: descriptor is {} bytes or more; refusing it", MAX_GDI_BYTES);
                return Err(GdiError::Parse);
            }
            parse(&text[..len], &image_path, work)?
        }
    } else {
        {
            let cdi_name = image_file.clone();
            info!("cdi: parsing '{}'", show_sfn(&cdi_name));
            let info = crate::cdi::parse(&image_dir, &cdi_name)?;
            let sessions = info.sessions;
            // The BIOS boots the first track of the LAST session, wherever
            // that is (11700 for audio+data, 45000 for a GD-shaped CDI, 0 for
            // a single data track). Not "track >= 3": see LOG R65.
            let hda = discimg::atapi::cdi_boot_area_lba(tracks(), info.boot_track);
            Gdi {
                boot_area_lba: hda,
                image_dir: image_path.clone(),
                num_blocks,
                is_cdi: true,
                sessions,
            }
        }
    };
    gdi.image_dir = image_path;
    gdi.num_blocks = num_blocks;

    // Walk the image dir and fill in each track's sector_count from the
    // on-disk file size. The closure runs inside iterate_dir's scope and
    // needs the tracks mutably while comparing their filenames, so it goes
    // through a raw pointer to the pool rather than a borrow of it. The
    // names compared are the 8.3 ones `parse` left in the tracks, aliases
    // included.
    if !from_cdi {
        let tracks_ptr = tracks_mut().as_mut_ptr();
        let tracks_len = tracks().len();
        let _ = image_dir.iterate_dir(|entry| {
            if entry.attributes.is_directory() {
                return;
            }
            for i in 0..tracks_len {
                let t = unsafe { &mut *tracks_ptr.add(i) };
                if entry.name.base_name() == t.filename.base_name()
                    && entry.name.extension() == t.filename.extension()
                {
                    // `push` accepts only 2048/2336/2352; never divide by 0.
                    t.sector_count = entry.size.checked_div(t.sector_size as u32).unwrap_or(0);
                    break;
                }
            }
        });
    }
    // `image_dir` is the root directory, descended in place, so dropping it
    // releases the one slot both used to hold.
    drop(image_dir);
    drop(volume);
    Ok(gdi)
}

/// Bytes of the IP.BIN header the host is given: hardware ID through the
/// 128-byte title (offsets 0x00..0x100 of the boot track's first sector).
#[allow(dead_code)]
pub const IP_HEADER_BYTES: usize = 256;

/// Read the IP.BIN header of `which` without mounting it. Returns the track
/// number the header came from and whether it carries the `SEGA SEGAKATANA`
/// signature.
///
/// The BIOS boots from the first track of the last session: track 3 on a
/// GD-ROM layout, the last session's first track on a CDI. That is the
/// track at `boot_area_lba`, the one decision the TOC, REQ_SES and the boot
/// prefetch already share, so it is tried first; the other data tracks
/// follow (`discimg::atapi::ip_bin_candidates`) and the first signed one
/// wins. A disc with no signed track yields the first readable candidate's
/// sector unsigned, and `NoIpBin` only when nothing at all could be read.
/// The returned track number tells the caller whether the header is the
/// boot track's.
#[allow(dead_code)]
pub fn read_ip_header(
    num_blocks: u32,
    which: &ImageRef,
    out: &mut [u8; IP_HEADER_BYTES],
) -> Result<(u8, bool), GdiError> {
    let gdi = parse_image(num_blocks, which)?;
    // Static for the same stack reason as `enumerate_images`: the Gdi above
    // is already several KB of this frame.
    static mut RAW: [u8; 16 + IP_HEADER_BYTES] = [0; 16 + IP_HEADER_BYTES];
    let raw = unsafe { &mut *core::ptr::addr_of_mut!(RAW) };
    let mut fallback: Option<u8> = None;
    for t in discimg::atapi::ip_bin_candidates(tracks(), gdi.boot_area_lba) {
        let off = t.user_data_offset();
        let want = off + IP_HEADER_BYTES;
        match read_at_lba(&gdi, t.start_lba, &mut raw[..want]) {
            Ok(n) if n == want => {}
            _ => continue,
        }
        if &raw[off..off + 16] == b"SEGA SEGAKATANA " {
            out.copy_from_slice(&raw[off..want]);
            return Ok((t.number, true));
        }
        if fallback.is_none() {
            out.copy_from_slice(&raw[off..want]);
            fallback = Some(t.number);
        }
    }
    fallback.map(|n| (n, false)).ok_or(GdiError::NoIpBin)
}

/// Build the block-run map for every track file so the data path can read
/// the card directly. Best effort: a track that cannot be mapped (not FAT32,
/// file missing, broken chain) keeps `ext_start == ext_len == 0` and is served
/// through the filesystem as before; one that only does not fit the pool
/// becomes a lazy file.
fn map_track_extents(gdi: &mut Gdi) {
    let pool = unsafe { &mut *core::ptr::addr_of_mut!(EXTENTS) };
    pool.clear();
    for t in tracks_mut().iter_mut() {
        t.ext_start = 0;
        t.ext_len = 0;
    }
    let Some(geom) = crate::fat::probe() else {
        crate::fat::lazy_reset(None);
        warn!("gdi: no FAT32 geometry; track reads stay on the slow path");
        return;
    };
    crate::fat::lazy_reset(Some(geom));
    let mut buf = [0u8; 512];
    let Some(dir_cluster) = dir_cluster_of(&geom, &gdi.image_dir, &mut buf) else {
        warn!("gdi: image dir not found in FAT; slow path");
        return;
    };

    // Read through `tracks()` and write through a *fresh, short-lived*
    // `tracks_mut()` each time. Two `tracks_mut()` calls live at once are two
    // `&mut` to the same static, which is UB -- and which I wrote here first,
    // by rewriting the field accesses mechanically. It compiled and the
    // mapping came out wrong, which costs every read the slow path.
    let n = tracks().len();
    for i in 0..n {
        // CDI: every track shares one file, so reuse its map.
        let shared = (0..i).find(|&j| {
            let t = tracks();
            t[j].is_mapped() && t[j].filename == t[i].filename
        });
        if let Some(j) = shared {
            let (start, len) = (tracks()[j].ext_start, tracks()[j].ext_len);
            let t = &mut tracks_mut()[i];
            t.ext_start = start;
            t.ext_len = len;
            continue;
        }

        let filename = tracks()[i].filename.clone();
        let number = tracks()[i].number;
        let Some(entry) = crate::fat::find_in_dir(&geom, dir_cluster, &filename, &mut buf) else {
            warn!("gdi: track {} file missing from FAT", number);
            continue;
        };
        let start = pool.len();
        match crate::fat::map_extents(&geom, &entry, pool) {
            Ok(len) => {
                let first = pool[start].lba;
                let t = &mut tracks_mut()[i];
                t.ext_start = start as u16;
                t.ext_len = len as u16;
                info!("gdi: track {} -> {} extent(s) from block {}", number, len, first);
            }
            // Fine but fragmented: the pool has no room for its runs, so they
            // are found on demand instead of going through the filesystem.
            Err(crate::fat::MapError::PoolFull) => match crate::fat::lazy_add(&entry) {
                Some(slot) => {
                    tracks_mut()[i].ext_start = slot as u16;
                    info!("gdi: track {} fragmented beyond the pool; lazy slot {}", number, slot);
                }
                None => warn!("gdi: track {} unmapped; slow path", number),
            },
            Err(_) => warn!("gdi: track {} unmapped; slow path", number),
        }
    }
}

/// First cluster of the directory `path` names, walked from the root.
fn dir_cluster_of(geom: &crate::fat::Geometry, path: &ImagePath, buf: &mut [u8; 512]) -> Option<u32> {
    let mut dir_cluster = geom.root_cluster;
    for part in path.iter() {
        let e = crate::fat::find_in_dir(geom, dir_cluster, part, buf).filter(|e| e.is_dir)?;
        // A `..` entry pointing at the root stores cluster 0; the root's own
        // cluster is what that means.
        dir_cluster = if e.first_cluster < 2 { geom.root_cluster } else { e.first_cluster };
    }
    Some(dir_cluster)
}

/// Read `buf.len()` bytes starting at `byte_offset` within the track
/// file for the given LBA. Re-opens the FAT volume on each call —
/// fine for short single-sector probes but too slow for game data.
#[allow(dead_code)]
pub fn read_at_lba(gdi: &Gdi, lba: u32, buf: &mut [u8]) -> Result<usize, GdiError> {
    let (track_filename, byte_offset) = gdi.locate(lba).ok_or(GdiError::LbaOutOfRange)?;

    let dev = HsmciBlockDevice::new();
    dev.set_num_blocks(gdi.num_blocks);
    let mgr = VolumeManager::new(dev, crate::blockdev::DummyClock);
    let volume = mgr
        .open_volume(embedded_sdmmc::VolumeIdx(0))
        .map_err(|_| GdiError::OpenVolume)?;
    let root = volume.open_root_dir().map_err(|_| GdiError::OpenRoot)?;
    let mut dir = root;
    descend(&mut dir, &gdi.image_dir)?;
    let f = dir
        .open_file_in_dir(&track_filename, embedded_sdmmc::Mode::ReadOnly)
        .map_err(|_| GdiError::OpenGdiFile)?;
    f.seek_from_start(byte_offset).map_err(|_| GdiError::ReadGdiFile)?;
    let mut total = 0usize;
    while total < buf.len() {
        let n = f.read(&mut buf[total..]).map_err(|_| GdiError::ReadGdiFile)?;
        if n == 0 {
            break;
        }
        total += n;
    }
    Ok(total)
}



fn find_gdi(dir: &Directory<'_, HsmciBlockDevice, crate::blockdev::DummyClock, 4, 4, 1>)
    -> Result<ShortFileName, GdiError>
{
    let mut found: Option<ShortFileName> = None;
    let _ = dir.iterate_dir(|entry| {
        if found.is_none() && !entry.attributes.is_directory()
            && extension_eq(&entry.name, b"GDI")
        {
            found = Some(entry.name.clone());
        }
    });
    found.ok_or(GdiError::NoGdiFile)
}

fn extension_eq(name: &ShortFileName, ext: &[u8]) -> bool {
    let e = name.extension();
    e.len() == ext.len() && e.iter().zip(ext.iter()).all(|(a, b)| a.eq_ignore_ascii_case(b))
}

fn read_file_to_buf(
    dir: &Directory<'_, HsmciBlockDevice, crate::blockdev::DummyClock, 4, 4, 1>,
    name: &ShortFileName,
    buf: &mut [u8],
) -> Result<usize, GdiError> {
    let f = dir.open_file_in_dir(name, embedded_sdmmc::Mode::ReadOnly)
        .map_err(|_| GdiError::OpenGdiFile)?;
    let mut total = 0usize;
    while total < buf.len() {
        let n = f.read(&mut buf[total..]).map_err(|_| GdiError::ReadGdiFile)?;
        if n == 0 { break; }
        total += n;
    }
    Ok(total)
}

/// The GDI text into the track pool.
///
/// A track whose name is not 8.3 (`Track 01.bin`, `Crazy Taxi (Track 1).bin`:
/// what a PC makes of a copied file, and what the GDI keeps saying) is found
/// on the card by its long name as it is parsed, and the 8.3 alias the card
/// stores it under goes into the track. Everything downstream -- the size
/// lookup, the extent map, the open by name -- goes by that alias and is
/// unchanged. See `PoolSink::track_file`. `work` is a block of scratch for
/// that lookup.
fn parse(data: &[u8], path: &ImagePath, work: &mut [u8]) -> Result<Gdi, GdiError> {
    // Straight into the pool: 99 of these will not fit on the stack, which
    // is the whole reason the pool exists. Cleared first (by the parser), so
    // a failed parse cannot leave the previous image's tail behind this one's.
    discimg::gdi::parse(data, &mut PoolSink::gdi(path, work)).map_err(|e| match e {
        discimg::Error::Sink(e) => e,
        discimg::Error::NameTooLong => GdiError::BadFilename,
        _ => GdiError::Parse,
    })?;
    let tracks = tracks_mut();

    let boot_area_lba = discimg::atapi::gdi_boot_area_lba(&tracks[..]);

    let gdi = Gdi {
        boot_area_lba,
        // Filled in by the caller -- `parse` does not know where the image
        // came from. Empty means "the card root", which is also what a path
        // with no components walks to.
        image_dir: ImagePath::new(),
        num_blocks: 0,
        // This is the `.gdi` text parser; the CDI path builds its own `Gdi`.
        is_cdi: false,
        // One session, which is what stock GDEMU reports even for a
        // dual-area GD-ROM -- and the value this used to send unconditionally.
        sessions: 1,
    };
    info!("gdi: parsed {} tracks, HDA starts at LBA {}", tracks.len(), gdi.boot_area_lba);
    for t in tracks.iter() {
        info!("  track {} lba={} ctrl=0x{:02x} sec={} file={}",
            t.number, t.start_lba, t.ctrl, t.sector_size, show_sfn(&t.filename));
    }
    Ok(gdi)
}

/// Feeds the static track pool from `discimg`'s parsers.
///
/// `cdi` is the image file's name when parsing a CDI (every track lives in
/// that one file); `dir` is the image folder when parsing a GDI (each track
/// names its own file, found in it).
pub(crate) struct PoolSink<'a> {
    cdi: Option<ShortFileName>,
    dir: Option<&'a ImagePath>,
    /// Block buffer for the long-name lookup (at least 512 bytes).
    work: Option<&'a mut [u8]>,
    /// The card's FAT32 geometry and the image folder's first cluster, worked
    /// out the first time a track name needs them. `Some(None)` is "tried,
    /// and the card is not FAT32 or the folder is not there".
    fat: Option<Option<(crate::fat::Geometry, u32)>>,
}

impl<'a> PoolSink<'a> {
    fn gdi(dir: &'a ImagePath, work: &'a mut [u8]) -> Self {
        PoolSink { cdi: None, dir: Some(dir), work: Some(work), fat: None }
    }

    pub(crate) fn cdi(name: ShortFileName) -> PoolSink<'static> {
        PoolSink { cdi: Some(name), dir: None, work: None, fat: None }
    }

    /// The 8.3 name of a GDI track file. A name that already is 8.3 is
    /// itself; any other is looked up by its long name in the image folder,
    /// and the answer is the alias the card keeps it under.
    ///
    /// `BadFilename` if there is no such file, the card is not FAT32 (the
    /// long-name walk is this firmware's own FAT reader), or the folder
    /// cannot be found.
    fn track_file(&mut self, name: &str) -> Result<ShortFileName, GdiError> {
        if name.is_empty() {
            return Err(GdiError::BadFilename);
        }
        // Only a pure ASCII name can be taken as its own 8.3 form. Anything
        // with a byte >= 0x80 may be the long name of a file whose alias is in
        // the card's code page (cp932, cp437): look that up first.
        let ascii = name.is_ascii();
        if ascii {
            if let Ok(n) = ShortFileName::create_from_str(name) {
                return Ok(n);
            }
        }
        match self.track_file_long(name) {
            Ok(n) => Ok(n),
            // No long-name entry: a name that is the file's own raw 8.3 bytes
            // (no VFAT entry is kept for those) still opens as before.
            Err(e) => match (ascii, ShortFileName::create_from_str(name)) {
                (false, Ok(n)) => Ok(n),
                _ => Err(e),
            },
        }
    }

    fn track_file_long(&mut self, name: &str) -> Result<ShortFileName, GdiError> {
        let (Some(dir), Some(work)) = (self.dir, self.work.as_deref_mut()) else {
            return Err(GdiError::BadFilename);
        };
        let Some(buf) = work.first_chunk_mut::<512>() else {
            return Err(GdiError::BadFilename);
        };
        let ctx = self.fat.get_or_insert_with(|| {
            let geom = crate::fat::probe()?;
            let cluster = dir_cluster_of(&geom, dir, buf)?;
            Some((geom, cluster))
        });
        let Some((geom, cluster)) = ctx else {
            warn!("gdi: track file '{}' is not 8.3 and the card/folder cannot be walked", name);
            return Err(GdiError::BadFilename);
        };
        let Some(short) = crate::fat::resolve_long(geom, *cluster, name, buf) else {
            warn!("gdi: track file '{}' not found by its long name", name);
            return Err(GdiError::BadFilename);
        };
        info!("gdi: '{}' is {}", name, show_sfn(&short));
        Ok(short)
    }
}

impl discimg::TrackSink for PoolSink<'_> {
    type Error = GdiError;

    fn cdi_header(&mut self, desc_len: u32, sessions: u8) {
        info!("cdi: descriptor {} bytes, {} session(s)", desc_len, sessions);
    }

    fn clear(&mut self) {
        tracks_mut().clear();
    }

    fn push(&mut self, t: &discimg::TrackDesc) -> Result<(), GdiError> {
        let is_cdi = self.cdi.is_some();
        // Sector sizes the data path knows. 0 would divide by zero in the size
        // lookup and anything above 2352 slices past the sector buffer.
        if !matches!(t.sector_size, 2048 | 2336 | 2352) {
            return Err(if is_cdi { GdiError::BadCdi } else { GdiError::Parse });
        }
        let filename = match &self.cdi {
            Some(name) => name.clone(),
            None => self.track_file(t.filename.as_str())?,
        };
        tracks_mut()
            .push(Track {
                number: t.number,
                start_lba: t.start_lba,
                ctrl: t.ctrl,
                sector_size: t.sector_size,
                sector_count: t.sector_count,
                filename,
                file_offset: t.file_offset,
                ext_start: 0,
                ext_len: 0,
                session: t.session,
            })
            .map_err(|_| if is_cdi { GdiError::BadCdi } else { GdiError::TooManyTracks })?;
        if is_cdi {
            info!(
                "cdi: track {} lba={} sectors={} ssize={} ctrl={:#04x} off={}",
                t.number, t.start_lba, t.sector_count, t.sector_size, t.ctrl, t.file_offset
            );
        }
        Ok(())
    }
}

impl discimg::TrackInfo for Track {
    fn number(&self) -> u8 { self.number }
    fn start_lba(&self) -> u32 { self.start_lba }
    fn ctrl(&self) -> u8 { self.ctrl }
    fn sector_count(&self) -> u32 { self.sector_count }
    fn session(&self) -> u8 { self.session }
}

/// Format a short filename as `BASE.EXT` into a fixed-size buffer
/// suitable for `defmt::info!("{=str}", ..)`.
fn show_sfn(name: &ShortFileName) -> SfnStr {
    let mut out = [b' '; 12];
    let mut i = 0;
    for &b in name.base_name() {
        if i < 8 {
            out[i] = b;
            i += 1;
        }
    }
    let ext = name.extension();
    if !ext.is_empty() {
        out[8] = b'.';
        let n = ext.len().min(3);
        out[9..9 + n].copy_from_slice(&ext[..n]);
    }
    SfnStr(out)
}

struct SfnStr([u8; 12]);

impl Format for SfnStr {
    fn format(&self, f: defmt::Formatter) {
        // Render as a regular ASCII string. ShortFileName bytes are
        // already in the printable range, but be defensive.
        let s = core::str::from_utf8(&self.0).unwrap_or("?");
        defmt::write!(f, "{}", s)
    }
}

#[derive(Debug, Format)]
pub enum GdiError {
    OpenVolume,
    OpenRoot,
    OpenImageDir,
    OpenGdiFile,
    ReadGdiFile,
    #[allow(dead_code)]
    LbaOutOfRange,
    #[allow(dead_code)]
    NoImageDir,
    NoGdiFile,
    BadCdi,
    Parse,
    BadFilename,
    TooManyTracks,
    #[allow(dead_code)]
    NoIpBin,
    Block(BlockError),
}

impl From<BlockError> for GdiError {
    fn from(e: BlockError) -> Self { GdiError::Block(e) }
}
