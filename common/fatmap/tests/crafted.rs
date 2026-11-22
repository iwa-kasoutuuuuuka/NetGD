//! Crafted cards against the reader (`fatmap::{vol, dir}`): the failure
//! classes a review found in `embedded_sdmmc` 0.9.0 -- nonsense BPB
//! geometry, cluster numbers past the 28 bits FAT32 has, and cyclic chains
//! -- must end in an error, never a panic, a wrong block or a hang.
//!
//! The firmware has no watchdog while it reads the card, so a "hang" here
//! is anything whose work grows with the volume's size instead of with what
//! was asked for. The cards count their reads and refuse to read past a
//! budget, so a regression fails the test instead of hanging the suite.

mod common;

use common::*;
use fatmap::dir::{find, walk, File};
use fatmap::vol::{Block, BlockRead, Error, Volume};

/// A card that counts reads and fails every read past `budget`.
struct Counted {
    card: Card,
    reads: u32,
    budget: u32,
}

impl Counted {
    fn new(card: Card) -> Counted {
        Counted { card, reads: 0, budget: 200_000 }
    }
}

impl BlockRead for Counted {
    fn read_block(&mut self, lba: u32, buf: &mut [u8; 512]) -> Result<(), ()> {
        self.reads += 1;
        if self.reads > self.budget || lba >= self.card.total {
            return Err(());
        }
        *buf = self.card.get(lba);
        Ok(())
    }
}

fn mount(card: &Card) -> Result<Volume, Error> {
    Volume::mount(&mut Counted::new(card.clone()), &mut Block::new())
}

fn put16(s: &mut [u8; 512], off: usize, v: u16) {
    s[off..off + 2].copy_from_slice(&v.to_le_bytes());
}

fn put32(s: &mut [u8; 512], off: usize, v: u32) {
    s[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

/// A sparse FAT32 "superfloppy" (BPB at LBA 0): `reserved` = 32, one FAT
/// of `spf` sectors, `total` blocks, the root at cluster 2 ending there.
/// Returns the card and the first data block.
fn superfloppy(spc: u8, spf: u32, total: u32) -> (Card, u32) {
    let mut c = Card::new(total);
    let s = c.sector_mut(0);
    s[0] = 0xEB;
    s[1] = 0x58;
    s[2] = 0x90;
    put16(s, 0x0B, 512);
    s[0x0D] = spc;
    put16(s, 0x0E, 32);
    s[0x10] = 1;
    put32(s, 0x20, total);
    put32(s, 0x24, spf);
    put32(s, 0x2C, 2);
    s[510] = 0x55;
    s[511] = 0xAA;
    set_fat(&mut c, 0, 0x0FFF_FFF8);
    set_fat(&mut c, 1, 0x0FFF_FFFF);
    set_fat(&mut c, 2, 0x0FFF_FFFF);
    (c, 32 + spf)
}

fn set_fat(c: &mut Card, cluster: u32, v: u32) {
    c.write(32 * 512 + cluster as u64 * 4, &v.to_le_bytes());
}

/// (A) Geometry that no formatter writes is not a volume. Sectors per
/// cluster 0 (a division by zero in `embedded_sdmmc`), other sector sizes
/// and no FATs were already refused; zero reserved sectors (FAT 0 would be
/// the boot sector itself) was mounted.
#[test]
fn mount_rejects_bad_geometry() {
    let good = build(FAT32, vec![file(sfn("A", "TXT"), vec![1])]);
    assert!(mount(&good).is_ok());
    let cases: [(&str, fn(&mut [u8; 512])); 5] = [
        ("sectors/cluster 0", |s| s[0x0D] = 0),
        ("1024 bytes/sector", |s| put16(s, 0x0B, 1024)),
        ("256 bytes/sector", |s| put16(s, 0x0B, 256)),
        ("no FATs", |s| s[0x10] = 0),
        ("no reserved sectors", |s| put16(s, 0x0E, 0)),
    ];
    for (what, f) in cases {
        let mut c = good.clone();
        f(c.sector_mut(2048));
        assert_eq!(mount(&c).err(), Some(Error::NoVolume), "{what}");
    }
}

#[cfg(feature = "fat16")]
#[test]
fn mount_rejects_bad_geometry_fat16() {
    let good = build(FAT16, vec![file(sfn("A", "TXT"), vec![1])]);
    assert!(mount(&good).is_ok());
    for (off, v) in [(0x0Du16, 0u16), (0x0E, 0), (0x10, 0)] {
        let mut c = good.clone();
        let s = c.sector_mut(63);
        if off == 0x0E {
            put16(s, 0x0E, v);
        } else {
            s[off as usize] = v as u8;
        }
        assert_eq!(mount(&c).err(), Some(Error::NoVolume), "byte {off:#x}");
    }
}

/// (A) A partition whose metadata or data area would end past block
/// 2^32 - 1 is refused instead of overflowing the block arithmetic (a
/// panic in a debug build, a wrapped block number in the firmware).
#[test]
fn mount_rejects_volume_past_lba_space() {
    let good = build(FAT32, vec![file(sfn("A", "TXT"), vec![1])]);
    let bpb = good.get(2048);
    for (start, total) in [(0xFFFF_FF00u32, None), (0xF000_0000, Some(0x2000_0000u32))] {
        let mut c = Card::new(u32::MAX);
        let mbr = c.sector_mut(0);
        *mbr = good.get(0);
        put32(mbr, 0x1BE + 8, start);
        let s = c.sector_mut(start);
        *s = bpb;
        if let Some(t) = total {
            // A FAT big enough that the data area, not the FAT, sets the
            // cluster count.
            put32(s, 0x20, t);
            put32(s, 0x24, 0x0040_0000);
        }
        let r = std::panic::catch_unwind(|| mount(&c));
        assert!(matches!(r, Ok(Err(Error::NoVolume))), "start {start:#x}: {:?}", r.map_err(|_| "panic"));
    }
}

/// (B) FAT32 cluster numbers have 28 bits. A BPB may claim more clusters
/// than that (a huge FAT on a huge volume); the count is capped at the
/// FAT32 maximum, so a root or a directory entry with any of the top four
/// bits set is outside the volume rather than a cluster that aliases a
/// real one, and the bad-cluster mark 0x0FFFFFF7 is never a link.
#[test]
fn cluster_numbers_have_28_bits() {
    let (mut c, data) = superfloppy(1, 0x0040_0000, u32::MAX);
    let v = mount(&c).unwrap();
    assert!(v.clusters() <= 0x0FFF_FFF5, "{:#x} clusters", v.clusters());
    // Root cluster 0x1000_0002 (cluster 2 with a high bit).
    let mut r = c.clone();
    put32(r.sector_mut(0), 0x2C, 0x1000_0002);
    assert_eq!(mount(&r).err(), Some(Error::NoVolume));
    // FAT[2] = bad cluster.
    let mut b = c.clone();
    set_fat(&mut b, 2, 0x0FFF_FFF7);
    let mut dev = Counted::new(b);
    assert_eq!(v.next_cluster(&mut dev, &mut Block::new(), 2), Err(Error::BadChain));
    // A file whose first cluster is 0x1000_0003.
    let ent = short_entry(&sfn("BIG", "BIN"), 0x20, 0, 0x1000_0003, 100);
    c.write(data as u64 * 512, &ent);
    let mut dev = Counted::new(c);
    let mut blk = Block::new();
    let e = find(&v, &mut dev, &mut blk, 0, &sfn("BIG", "BIN")).unwrap();
    let mut f = File::open(&e);
    assert_eq!(f.read(&v, &mut dev, &mut blk, &mut [0u8; 100]), Err(Error::BadChain));
    // Nor is it a directory.
    assert_eq!(walk(&v, &mut dev, &mut blk, e.first_cluster, &mut [], &mut |_, _| false), Err(Error::BadChain));
}

/// (C) A directory whose chain loops ends after at most 65536 entries
/// (2 MiB, the FAT limit for a directory), not after as many clusters as
/// the volume has: on a 64 GB card with 32 KB clusters that bound was 2M
/// hops of 64 block reads each, hours without a watchdog.
#[test]
fn directory_loop_ends_within_dir_limit() {
    for spc in [1u8, 8, 64] {
        let spc32 = spc as u32;
        let clusters = 2_000_000u32;
        let spf = clusters.div_ceil(128) + 1;
        let (mut c, data) = superfloppy(spc, spf, 32 + spf + clusters * spc32);
        set_fat(&mut c, 2, 3);
        set_fat(&mut c, 3, 2);
        // Both clusters full of deleted entries: the walk never sees an end.
        let mut del = [0u8; 512];
        for k in 0..16 {
            del[k * 32] = 0xE5;
        }
        for b in 0..2 * spc32 {
            *c.sector_mut(data + b) = del;
        }
        let v = mount(&c).unwrap();
        let mut dev = Counted::new(c);
        let r = walk(&v, &mut dev, &mut Block::new(), 0, &mut [0u8; 765], &mut |_, _| false);
        assert_eq!(r, Err(Error::BadChain), "spc {spc}");
        // 4096 directory blocks, one FAT read per cluster, the boot sector.
        let limit = 4096 + 4096 / spc32 + spc32 + 2;
        assert!(dev.reads <= limit, "spc {spc}: {} reads", dev.reads);
    }
}

/// A directory of exactly 65536 entries is still listed in full.
#[test]
fn directory_of_65536_entries_is_listed() {
    let spc = 8u8;
    let clusters = 70_000u32;
    let spf = clusters.div_ceil(128) + 1;
    let (mut c, data) = superfloppy(spc, spf, 32 + spf + clusters * spc as u32);
    let dir_clusters = 4096 / spc as u32;
    for i in 0..dir_clusters {
        set_fat(&mut c, 2 + i, if i + 1 == dir_clusters { 0x0FFF_FFFF } else { 3 + i });
    }
    let mut blk = [0u8; 512];
    for k in 0..16 {
        blk[k * 32..k * 32 + 11].copy_from_slice(b"F          ");
        blk[k * 32 + 11] = 0x20;
    }
    for b in 0..4096 {
        *c.sector_mut(data + b) = blk;
    }
    let v = mount(&c).unwrap();
    let mut dev = Counted::new(c);
    let mut n = 0u32;
    let r = walk(&v, &mut dev, &mut Block::new(), 0, &mut [], &mut |_, _| {
        n += 1;
        false
    });
    assert_eq!((r, n), (Ok(None), 65536));
}
