//! Minimal FAT32 reader for building block-extent maps of the track files.
//!
//! The data path wants to read raw sectors straight off the SD card with
//! multi-block CMD18 reads into whichever buffer it likes -- the CD_READ
//! staging buffer, or the CDDA ring that must keep up with real time. The
//! `embedded_sdmmc` path we use for everything else re-opens the volume,
//! re-walks the directory and reads one 512-byte block at a time through a
//! bounce buffer, which cost ~55 ms per CD_READ chunk. So at image load we
//! walk the FAT once and remember each track file as a short list of
//! contiguous block runs (`Extent`); after that a disc LBA maps to an SD
//! block with integer arithmetic and no filesystem code at all.
//!
//! Only FAT32 on a 512-byte-sector volume is handled (with or without an
//! MBR in front). Anything else -- FAT16, exFAT -- leaves the track unmapped
//! and the slow path stays in charge, so this is strictly an accelerator.
//!
//! A file with more runs than the shared pool can hold is not unmapped any
//! more: it is registered as a *lazy* file (`lazy_add`), and its runs are
//! found on demand through a small window over its cluster chain
//! (`lazy_locate`, `fatmap::extent::Window`). Streaming such a file walks its
//! chain once; only a backward seek walks it again. The `embedded_sdmmc`
//! fallback that this replaces re-opened the file and walked the chain from
//! its start for every 2 KB chunk, one CMD17 per block (docs/research/
//! g3-read-path.md, "Unmapped tracks").
//!
//! The directory walk is shared by the 8.3 lookup and the long-name lookup
//! (`lookup`), so a track named `Track 01.bin` in the GDI is found the same
//! way whichever the card stores it under (`fatmap::lfn`).

use defmt::{info, warn, Format};
use embassy_atsam3::hsmci;
use embedded_sdmmc::ShortFileName;
use fatmap::extent::{ClusterGeom, Cursor, Walk};
#[cfg(feature = "lazy-extents")]
use fatmap::extent::Window;
use fatmap::lfn::{Feed, LfnMatcher};
use heapless::Vec;

/// One contiguous run of a file on the card (8 bytes: a run's file offset is
/// the sum of the runs before it).
pub use fatmap::extent::Extent;

/// Shared pool: every mapped track file owns a slice of it (see
/// `Track::ext_start` / `ext_len`). A freshly formatted card writes each
/// file in one or two runs; 96 leaves room for a moderately fragmented
/// 20-track image. A file that does not fit is read through the lazy window
/// (`lazy_add`) rather than the filesystem.
pub const MAX_EXTENTS: usize = 96;

/// Volume geometry, everything in 512-byte blocks.
#[derive(Clone, Copy, Format)]
pub struct Geometry {
    fat_lba: u32,
    sectors_per_fat: u32,
    data_lba: u32,
    blocks_per_cluster: u32,
    pub root_cluster: u32,
}

impl Geometry {
    pub fn cluster_lba(&self, cluster: u32) -> u32 {
        self.data_lba + (cluster - 2) * self.blocks_per_cluster
    }

    fn cluster_geom(&self) -> ClusterGeom {
        ClusterGeom { data_lba: self.data_lba, blocks_per_cluster: self.blocks_per_cluster }
    }
}

/// A directory entry we care about.
#[derive(Clone, Copy, Format)]
pub struct Entry {
    pub first_cluster: u32,
    pub size: u32,
    pub is_dir: bool,
    /// The on-disk 8.3 name (the alias, when the file has a long name).
    pub short: [u8; 11],
}

fn rd16(b: &[u8], off: usize) -> u32 {
    u16::from_le_bytes([b[off], b[off + 1]]) as u32
}

fn rd32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn read_block(lba: u32, buf: &mut [u8; 512]) -> Result<(), ()> {
    unsafe { hsmci::read_block(lba, buf) }.map_err(|_| ())
}

fn looks_like_bpb(s: &[u8; 512]) -> bool {
    (s[0] == 0xEB || s[0] == 0xE9) && rd16(s, 0x0B) == 512 && s[0x0D] != 0
}

/// First partition's start LBA (F7's protected boundary), read as `probe`
/// does: entry 0 of the MBR, 0 for a card whose LBA 0 already is a volume or
/// that has no MBR. `buf` is the caller's scratch, not a stack frame.
pub fn partition_start(buf: &mut [u8; 512]) -> Result<u32, ()> {
    read_block(0, buf)?;
    Ok(discimg::wsession::partition_start(buf, looks_like_bpb(buf)))
}

/// Locate the first FAT32 volume on the card.
pub fn probe() -> Option<Geometry> {
    let mut sec = [0u8; 512];
    read_block(0, &mut sec).ok()?;
    if sec[510] != 0x55 || sec[511] != 0xAA {
        return None;
    }
    // Same choice embedded-sdmmc makes for VolumeIdx(0): partition entry 0
    // of an MBR, or the whole card if LBA 0 already is a BPB.
    let base = if looks_like_bpb(&sec) {
        0
    } else {
        let start = rd32(&sec, 0x1BE + 8);
        if start == 0 {
            return None;
        }
        read_block(start, &mut sec).ok()?;
        if !looks_like_bpb(&sec) || sec[510] != 0x55 || sec[511] != 0xAA {
            return None;
        }
        start
    };
    let blocks_per_cluster = sec[0x0D] as u32;
    let reserved = rd16(&sec, 0x0E);
    let num_fats = sec[0x10] as u32;
    let root_entries = rd16(&sec, 0x11);
    let spf16 = rd16(&sec, 0x16);
    let spf32 = rd32(&sec, 0x24);
    let root_cluster = rd32(&sec, 0x2C);
    if root_entries != 0 || spf16 != 0 || spf32 == 0 || num_fats == 0 || root_cluster < 2 {
        warn!("fat: volume at {} is not FAT32", base);
        return None;
    }
    let g = Geometry {
        fat_lba: base + reserved,
        sectors_per_fat: spf32,
        data_lba: base + reserved + num_fats * spf32,
        blocks_per_cluster,
        root_cluster,
    };
    info!("fat: FAT32 at {}, {} blocks/cluster, fat@{} data@{}",
          base, blocks_per_cluster, g.fat_lba, g.data_lba);
    Some(g)
}

/// Next cluster in the chain, or `None` at end-of-chain / bad entry.
///
/// `cached` is the FAT sector last read into `buf`, plus one (0: nothing), so
/// a chain walk costs a read only when it crosses into another sector and a
/// static of these is all zeroes. A caller that reuses `buf` for something
/// else sets it back to 0.
fn fat_next(g: &Geometry, cluster: u32, cached: &mut u32, buf: &mut [u8; 512]) -> Result<Option<u32>, ()> {
    let sector = cluster / 128;
    if sector >= g.sectors_per_fat {
        return Err(());
    }
    if sector + 1 != *cached {
        *cached = 0;
        read_block(g.fat_lba + sector, buf)?;
        *cached = sector + 1;
    }
    let v = rd32(buf, (cluster % 128) as usize * 4) & 0x0FFF_FFFF;
    if v >= 0x0FFF_FFF8 {
        Ok(None)
    } else if v < 2 || v == 0x0FFF_FFF7 {
        Err(())
    } else {
        Ok(Some(v))
    }
}

/// Compare an on-disk 11-byte name with a `ShortFileName`.
fn name_matches(raw: &[u8], name: &ShortFileName) -> bool {
    let trim = |s: &[u8]| -> usize {
        let mut n = s.len();
        while n > 0 && s[n - 1] == b' ' {
            n -= 1;
        }
        n
    };
    let base = &raw[..trim(&raw[..8])];
    let ext = &raw[8..8 + trim(&raw[8..11])];
    base.eq_ignore_ascii_case(name.base_name()) && ext.eq_ignore_ascii_case(name.extension())
}

/// Longest long name the lookup accepts, in UTF-16 code units. The GDI
/// parser keeps file names to 32 bytes (`discimg::TrackDesc::filename`), so
/// nothing longer is ever asked for.
pub const MAX_LOOKUP_CHARS: usize = 32;

/// What to look for in a directory.
pub enum Key<'a> {
    /// The 8.3 name. Matches the entry's own name, case-insensitively.
    Short(&'a ShortFileName),
    /// A long name as the GDI parser hands it over. Matches the file whose
    /// VFAT long name equals it (ASCII case-insensitively; see
    /// `fatmap::lfn`). A name that is already 8.3 is looked up as `Short`.
    Long(&'a str),
}

/// Look `key` up in the directory starting at `dir_cluster`.
///
/// One walk serves both kinds of key. It reads the directory a block at a
/// time and feeds every 32-byte entry to the long-name matcher, which keeps
/// its state across blocks and clusters; the only memory is one block and the
/// matcher (about 80 bytes). `buf` is the caller's block buffer, so that this
/// can run under the volume manager without a frame of its own.
pub fn lookup(g: &Geometry, dir_cluster: u32, key: &Key<'_>, buf: &mut [u8; 512]) -> Option<Entry> {
    let mut matcher = match key {
        Key::Long(name) => Some(LfnMatcher::<MAX_LOOKUP_CHARS>::from_gdi_name(name)?),
        Key::Short(_) => None,
    };
    let short_key = match key {
        Key::Short(n) => Some(*n),
        Key::Long(_) => None,
    };
    let mut cluster = dir_cluster;
    loop {
        let base = g.cluster_lba(cluster);
        for b in 0..g.blocks_per_cluster {
            read_block(base + b, buf).ok()?;
            for e in buf.chunks_exact(32) {
                let long = match matcher.as_mut() {
                    Some(m) => match m.feed(e) {
                        Feed::End => return None,
                        Feed::Other => continue,
                        Feed::Entry { long } => long,
                    },
                    None => match e[0] {
                        0x00 => return None,
                        0xE5 => continue,
                        _ => {
                            let attr = e[11];
                            if attr & 0x0F == 0x0F || attr & 0x08 != 0 {
                                continue; // LFN piece or volume label
                            }
                            false
                        }
                    },
                };
                let hit = long
                    || short_key.map_or(false, |n| name_matches(&e[..11], n));
                if hit {
                    let attr = e[11];
                    let mut short = [0u8; 11];
                    short.copy_from_slice(&e[..11]);
                    return Some(Entry {
                        first_cluster: (rd16(e, 0x14) << 16) | rd16(e, 0x1A),
                        size: rd32(e, 0x1C),
                        is_dir: attr & 0x10 != 0,
                        short,
                    });
                }
            }
        }
        // The directory block is spent, so the FAT sector may take its place.
        cluster = fat_next(g, cluster, &mut 0, buf).ok()??;
    }
}

/// Look an 8.3 name up in the directory starting at `dir_cluster`.
pub fn find_in_dir(g: &Geometry, dir_cluster: u32, name: &ShortFileName, buf: &mut [u8; 512]) -> Option<Entry> {
    lookup(g, dir_cluster, &Key::Short(name), buf)
}

/// The 8.3 alias of the file called `name` (as the GDI spells it) in the
/// directory at `dir_cluster`, for use with everything that opens or maps a
/// file by short name.
pub fn resolve_long(g: &Geometry, dir_cluster: u32, name: &str, buf: &mut [u8; 512]) -> Option<ShortFileName> {
    let e = lookup(g, dir_cluster, &Key::Long(name), buf)?;
    if e.is_dir {
        return None;
    }
    let mut text = [0u8; 12];
    let n = fatmap::lfn::display_83(&e.short, &mut text);
    // Bytes above 0x7F are OEM-code-page characters; ShortFileName takes
    // ISO-8859-1 and stores the byte back, which is what the directory holds.
    let mut s = heapless::String::<12>::new();
    for &b in &text[..n] {
        s.push(b as char).ok()?;
    }
    ShortFileName::create_from_str(&s).ok()
}

/// Why `map_extents` gave up on a file.
#[derive(Clone, Copy, Format, PartialEq, Eq)]
pub enum MapError {
    /// The shared pool is full: the file is fine, just fragmented. It can be
    /// served lazily.
    PoolFull,
    /// Empty file, or its chain is broken.
    Unmappable,
}

/// Walk `entry`'s cluster chain and append its runs to `out`. Returns the
/// number of extents added. On failure `out` is left as it was.
pub fn map_extents<const N: usize>(
    g: &Geometry,
    entry: &Entry,
    out: &mut Vec<Extent, N>,
) -> Result<usize, MapError> {
    let Some(cur) = Cursor::start(entry.first_cluster, entry.size) else {
        return Err(MapError::Unmappable);
    };
    let start_len = out.len();
    let mut buf = [0u8; 512];
    let mut cached = 0;
    let r = fatmap::extent::walk(
        &g.cluster_geom(),
        cur,
        |c| fat_next(g, c, &mut cached, &mut buf),
        |_, e| out.push(e).is_ok(),
    );
    match r {
        Walk::Done => Ok(out.len() - start_len),
        Walk::More(_) => {
            warn!("fat: extent pool full");
            out.truncate(start_len);
            Err(MapError::PoolFull)
        }
        Walk::Broken => {
            warn!("fat: chain ends early or is damaged");
            out.truncate(start_len);
            Err(MapError::Unmappable)
        }
    }
}

/// Resolve a byte offset within a mapped file to `(sd_block, offset within
/// that block, contiguous blocks available from there)`.
pub fn locate(extents: &[Extent], byte_off: u32) -> Option<(u32, u32, u32)> {
    fatmap::extent::locate(extents, 0, byte_off)
}

// ---------------------------------------------------------------------------
// Lazy files: too fragmented for the pool, read through a window.
// ---------------------------------------------------------------------------

/// Files that can be lazy at once. One per unmapped track file; a CDI is one
/// file whatever its track count, and a GDI whose tracks are *all* too
/// fragmented for the pool is not a case worth RAM.
#[cfg(feature = "lazy-extents")]
const MAX_LAZY_FILES: usize = 6;

/// Runs held in the window. An extent is 8 bytes.
#[cfg(feature = "lazy-extents")]
const WINDOW_EXTENTS: usize = 12;

#[cfg(feature = "lazy-extents")]
#[derive(Clone, Copy)]
struct LazyFile {
    first_cluster: u32,
    size: u32,
}

#[cfg(feature = "lazy-extents")]
struct Lazy {
    geom: Option<Geometry>,
    files: Vec<LazyFile, MAX_LAZY_FILES>,
    window: Window<WINDOW_EXTENTS>,
    /// One FAT sector, and which it is (see `fat_next`).
    fat_buf: [u8; 512],
    fat_cached: u32,
}

#[cfg(feature = "lazy-extents")]
static mut LAZY: Lazy = Lazy {
    geom: None,
    files: Vec::new(),
    window: Window::new(),
    fat_buf: [0; 512],
    fat_cached: 0,
};

#[cfg(feature = "lazy-extents")]
fn lazy() -> &'static mut Lazy {
    unsafe { &mut *core::ptr::addr_of_mut!(LAZY) }
}

/// Forget every lazy file and the window: a new image is being mapped.
/// `geom` is the volume the files live on.
#[cfg(feature = "lazy-extents")]
pub fn lazy_reset(geom: Option<Geometry>) {
    let l = lazy();
    l.geom = geom;
    l.files.clear();
    l.window.invalidate();
}

/// Register `entry` as a lazily mapped file. Returns its slot (1-based, so 0
/// can mean "none" in a track), or `None` if it cannot be served this way.
#[cfg(feature = "lazy-extents")]
pub fn lazy_add(entry: &Entry) -> Option<u8> {
    let l = lazy();
    l.geom?;
    Cursor::start(entry.first_cluster, entry.size)?;
    l.files
        .push(LazyFile { first_cluster: entry.first_cluster, size: entry.size })
        .ok()?;
    Some(l.files.len() as u8)
}

/// `locate` for a lazy file: `(sd_block, offset within the block, contiguous
/// blocks from there)`. May read FAT sectors from the card when the offset is
/// outside the window; reads nothing when it is inside.
#[cfg(feature = "lazy-extents")]
pub fn lazy_locate(slot: u8, byte_off: u32) -> Option<(u32, u32, u32)> {
    let l = lazy();
    let g = l.geom?;
    let f = *l.files.get((slot as usize).checked_sub(1)?)?;
    // The FAT sector cached by the last walk is not trusted across lookups:
    // the host can write the card.
    l.fat_cached = 0;
    let (buf, cached) = (&mut l.fat_buf, &mut l.fat_cached);
    l.window.lookup(slot, &g.cluster_geom(), f.first_cluster, f.size, byte_off, |c| {
        fat_next(&g, c, cached, buf)
    })
}

// Without the feature the same three entry points say "no": every file the
// pool cannot hold stays on the filesystem path.
#[cfg(not(feature = "lazy-extents"))]
pub fn lazy_reset(_geom: Option<Geometry>) {}

#[cfg(not(feature = "lazy-extents"))]
pub fn lazy_add(_entry: &Entry) -> Option<u8> {
    None
}

#[cfg(not(feature = "lazy-extents"))]
pub fn lazy_locate(_slot: u8, _byte_off: u32) -> Option<(u32, u32, u32)> {
    None
}
