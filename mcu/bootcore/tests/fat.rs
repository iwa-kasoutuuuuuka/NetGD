//! The FAT32 reader on built and crafted images.

mod common;

use bootcore::fat::*;
use common::*;

/// Another name on the card (the resident no longer knows `NETGD.BIT`).
const NAME_OTHER: [u8; 11] = *b"NETGD   BIT";

const NAME: [u8; 11] = *b"NETGD   OGF";

fn read_all(card: &mut Card, vol: &Volume, ent: &DirEnt) -> Result<Vec<u8>, FatError> {
    let mut r = FileReader::new(ent);
    let mut buf = [0u8; 512];
    let mut out = vec![];
    loop {
        let n = r.read_next(vol, card, &mut buf)?;
        if n == 0 {
            return Ok(out);
        }
        out.extend_from_slice(&buf[..n]);
    }
}

fn open(img: Vec<u8>, name: &[u8; 11]) -> (Card, Result<(Volume, DirEnt), FatError>) {
    let mut card = Card::new(img);
    let mut buf = [0u8; 512];
    let r = Volume::mount(&mut card, &mut buf).and_then(|v| v.find(&mut card, &mut buf, name).map(|e| (v, e)));
    (card, r)
}

fn expect_file(img: Vec<u8>, data: &[u8]) {
    let (mut card, r) = open(img, &NAME);
    let (vol, ent) = r.expect("open");
    assert_eq!(ent.size as usize, data.len());
    assert_eq!(read_all(&mut card, &vol, &ent).unwrap(), data);
}

fn file(n: usize) -> Vec<u8> {
    lcg(n as u32, n)
}

#[test]
fn contiguous_file_all_cluster_sizes_and_sizes() {
    for spc in [1u32, 2, 4, 8, 64, 128] {
        for size in [1usize, 511, 512, 513, 1024, 4096, 70_000, 200_000] {
            let mut f = Fat::new(8192 * 6, spc);
            let d = file(size);
            f.add_file(&NAME, &d, Layout::Contig, false);
            expect_file(f.img.clone(), &d);
        }
    }
}

#[test]
fn behind_an_mbr_and_as_superfloppy() {
    let d = file(5000);
    let mut f = Fat::new(8192 * 2, 2);
    f.add_file(&NAME, &d, Layout::Contig, false);
    expect_file(f.img.clone(), &d); // LBA 0 is the BPB
    for ty in [0x0B, 0x0C] {
        expect_file(with_mbr(&f.img, ty), &d);
    }
}

#[test]
fn fragmented_files() {
    let d = file(100_000);
    for layout in [Layout::Reverse, Layout::Stride3] {
        for spc in [1u32, 4] {
            let mut f = Fat::new(8192 * 4, spc);
            // another file first so that the clusters really interleave
            f.add_file(b"OTHER   BIN", &file(30_000), Layout::Stride3, false);
            let first = f.add_file(&NAME, &d, layout, false);
            let ch = f.chain(first);
            assert!(ch.windows(2).any(|w| w[1] != w[0] + 1), "chain is not contiguous");
            expect_file(f.img.clone(), &d);
        }
    }
}

#[test]
fn long_name_entries_around_the_short_entry() {
    let d = file(3000);
    let mut f = Fat::new(8192 * 2, 1);
    // an LFN run for an unrelated name, a deleted entry with the same short
    // name, a directory with the same short name, a volume label, an LFN
    // entry whose first 11 bytes spell the name, then the real thing
    f.push_entry(&Fat::dirent(b"MY VOLUME  ", 0x08, 0, 0));
    f.push_entry(&Fat::lfn_entry(0x41, None));
    f.push_entry(&Fat::dirent(b"OTHER   TXT", 0x20, 0, 0));
    let mut del = Fat::dirent(&NAME, 0x20, 5, 99);
    del[0] = 0xE5;
    f.push_entry(&del);
    f.push_entry(&Fat::dirent(&NAME, 0x10, 7, 0));
    f.push_entry(&Fat::lfn_entry(0x41, Some(&NAME)));
    f.add_file(&NAME, &d, Layout::Contig, true);
    expect_file(f.img.clone(), &d);
}

#[test]
fn name_must_match_exactly() {
    let d = file(600);
    let mut f = Fat::new(8192 * 2, 1);
    f.add_file(b"NETGD   OGG", &file(600), Layout::Contig, false);
    f.add_file(b"NETGD1  OGF", &file(600), Layout::Contig, false);
    f.add_file(b"netgd   ogf", &file(600), Layout::Contig, false);
    f.add_file(b"NETGD   BIT", &d, Layout::Contig, false);
    let (_, r) = open(f.img.clone(), &NAME);
    assert_eq!(r.err(), Some(FatError::NotFound));
    let (mut card, r) = open(f.img.clone(), &NAME_OTHER);
    let (vol, ent) = r.unwrap();
    assert_eq!(read_all(&mut card, &vol, &ent).unwrap(), d);
}

#[test]
fn file_found_in_a_later_root_cluster() {
    let d = file(1500);
    let mut f = Fat::new(8192 * 2, 1); // 16 entries per root cluster
    for i in 0..50u32 {
        let mut n = *b"JUNK0000TXT";
        n[4..8].copy_from_slice(format!("{:04}", i).as_bytes());
        f.add_file(&n, &file(10), Layout::Contig, false);
    }
    f.add_file(&NAME, &d, Layout::Reverse, true);
    assert!(f.root_chain.len() >= 4);
    expect_file(f.img.clone(), &d);
}

#[test]
fn directory_end_marker_stops_the_search() {
    let mut f = Fat::new(8192 * 2, 1);
    f.add_file(b"A       TXT", &file(10), Layout::Contig, false);
    // an entry for NAME placed after a 0x00 entry is invisible
    f.push_entry(&[0u8; 32]);
    f.add_file(&NAME, &file(100), Layout::Contig, false);
    let (_, r) = open(f.img.clone(), &NAME);
    assert_eq!(r.err(), Some(FatError::NotFound));
}

// ------------------------------------------------------------------ rejected volumes

fn mount_err(img: Vec<u8>) -> FatError {
    let mut card = Card::new(img);
    let mut buf = [0u8; 512];
    Volume::mount(&mut card, &mut buf).expect_err("must not mount")
}

fn good_fs() -> Fat {
    let mut f = Fat::new(8192 * 2, 1);
    f.add_file(&NAME, &file(2000), Layout::Contig, false);
    f
}

#[test]
fn fat16_is_refused() {
    let mut f = good_fs();
    // a FAT16 BPB: root entries, 16-bit FAT size, no 32-bit FAT size
    f.img[0x11..0x13].copy_from_slice(&512u16.to_le_bytes());
    f.img[0x16..0x18].copy_from_slice(&32u16.to_le_bytes());
    put32(&mut f.img, 0x24, 0);
    f.img[0x52..0x5A].copy_from_slice(b"FAT16   ");
    assert_eq!(mount_err(f.img.clone()), FatError::NotFat32);
    assert_eq!(mount_err(with_mbr(&f.img, 0x06)), FatError::NotFat32);
    assert_eq!(mount_err(with_mbr(&f.img, 0x0E)), FatError::NotFat32);
}

#[test]
fn exfat_is_refused() {
    // The boot sector of an exFAT volume: jump, "EXFAT   ", 53 zero bytes
    // where the BPB would be, then exFAT fields.
    let mut s = vec![0u8; 512];
    s[0] = 0xEB;
    s[1] = 0x76;
    s[2] = 0x90;
    s[3..11].copy_from_slice(b"EXFAT   ");
    s[0x48] = 0x20;
    s[0x6C] = 9; // bytes per sector shift
    s[510] = 0x55;
    s[511] = 0xAA;
    let mut img = s.clone();
    img.resize(512 * 100, 0);
    let e = mount_err(img);
    assert!(e == FatError::NotFat32 || e == FatError::BadBpb, "{e:?}");
    // behind an MBR of type 0x07 and, to be sure, mislabelled as 0x0C
    let mut vol = s.clone();
    vol.resize(512 * 100, 0);
    assert_eq!(mount_err(with_mbr(&vol, 0x07)), FatError::NotFat32);
    assert_eq!(mount_err(with_mbr(&vol, 0x0C)), FatError::NotFat32);
}

#[test]
fn gpt_protective_mbr_is_refused() {
    let f = good_fs();
    assert_eq!(mount_err(with_mbr(&f.img, 0xEE)), FatError::Gpt);
}

#[test]
fn bad_mbr_and_signatures() {
    let f = good_fs();
    // blank card
    assert_eq!(mount_err(vec![0; 512 * 8]), FatError::NoSignature);
    // MBR with no signature
    let mut m = with_mbr(&f.img, 0x0C);
    m[510] = 0;
    assert_eq!(mount_err(m), FatError::NoSignature);
    // partition start 0
    let mut m = with_mbr(&f.img, 0x0C);
    put32(&mut m, 0x1BE + 8, 0);
    assert_eq!(mount_err(m), FatError::BadBpb);
    // partition start points at garbage / beyond the card
    let mut m = with_mbr(&f.img, 0x0C);
    put32(&mut m, 0x1BE + 8, 5);
    assert!(matches!(mount_err(m), FatError::NotFat32 | FatError::NoSignature));
    let mut m = with_mbr(&f.img, 0x0C);
    put32(&mut m, 0x1BE + 8, 0x00FF_0000);
    assert_eq!(mount_err(m), FatError::Read);
    // the volume's own boot sector has no signature
    let mut m = with_mbr(&f.img, 0x0C);
    m[2048 * 512 + 510] = 0;
    assert_eq!(mount_err(m), FatError::NoSignature);
    // device failure on the first read
    let mut card = Card::new(f.img.clone());
    card.fail_at = Some(0);
    let mut buf = [0u8; 512];
    assert_eq!(Volume::mount(&mut card, &mut buf), Err(FatError::Read));
}

#[test]
fn broken_bpbs_are_refused() {
    let base = good_fs();
    let cases: Vec<(&str, Box<dyn Fn(&mut Vec<u8>)>)> = vec![
        ("bytes per sector 1024", Box::new(|i| i[0x0B..0x0D].copy_from_slice(&1024u16.to_le_bytes()))),
        ("spc 0", Box::new(|i| i[0x0D] = 0)),
        ("spc 3", Box::new(|i| i[0x0D] = 3)),
        ("spc 129+", Box::new(|i| i[0x0D] = 255)),
        ("reserved 0", Box::new(|i| i[0x0E..0x10].copy_from_slice(&0u16.to_le_bytes()))),
        ("0 FATs", Box::new(|i| i[0x10] = 0)),
        ("3 FATs", Box::new(|i| i[0x10] = 3)),
        ("sectors/FAT 0", Box::new(|i| put32(i, 0x24, 0))),
        ("sectors/FAT huge", Box::new(|i| put32(i, 0x24, 0xFFFF_FFF0))),
        ("sectors/FAT tiny", Box::new(|i| put32(i, 0x24, 1))),
        ("total sectors below metadata", Box::new(|i| put32(i, 0x20, 40))),
        ("total sectors 0", Box::new(|i| put32(i, 0x20, 0))),
        ("root cluster 0", Box::new(|i| put32(i, 0x2C, 0))),
        ("root cluster 1", Box::new(|i| put32(i, 0x2C, 1))),
        ("root cluster beyond volume", Box::new(|i| put32(i, 0x2C, 0x0FFF_FFF0))),
        ("no jump", Box::new(|i| i[0] = 0x00)),
        // A BPB claiming more than 2^28 clusters (FAT and volume both huge;
        // only LBA 0 is read): the count is capped at 0x0FFFFFF5, so a root
        // of 0x10000002 is out of range instead of an alias of cluster 2.
        (
            "root cluster with the top bits set",
            Box::new(|i| {
                i[0x10] = 1;
                put32(i, 0x20, u32::MAX);
                put32(i, 0x24, 0x01FC_1000);
                put32(i, 0x2C, 0x1000_0002);
            }),
        ),
    ];
    for (name, f) in cases {
        let mut img = base.img.clone();
        f(&mut img);
        assert!(Volume::mount(&mut Card::new(img), &mut [0u8; 512]).is_err(), "{name}");
    }
    // sanity: the unmodified image mounts
    Volume::mount(&mut Card::new(base.img.clone()), &mut [0u8; 512]).unwrap();
}

// ------------------------------------------------------------------ bad chains

#[test]
fn chain_loops_are_detected() {
    // self loop in a file
    let d = file(10 * 512);
    let mut f = Fat::new(8192 * 2, 1);
    let first = f.add_file(&NAME, &d, Layout::Contig, false);
    let ch = f.chain(first);
    f.set_fat(ch[3], ch[3]);
    let (mut card, r) = open(f.img.clone(), &NAME);
    let (vol, ent) = r.unwrap();
    assert_eq!(read_all(&mut card, &vol, &ent), Err(FatError::Loop));

    // two-cluster loop, long file
    let mut f = Fat::new(8192 * 2, 1);
    let d = file(40 * 512);
    let first = f.add_file(&NAME, &d, Layout::Contig, false);
    let ch = f.chain(first);
    f.set_fat(ch[5], ch[4]);
    let (mut card, r) = open(f.img.clone(), &NAME);
    let (vol, ent) = r.unwrap();
    assert_eq!(read_all(&mut card, &vol, &ent), Err(FatError::Loop));

    // the last cluster pointing back to the start is never looked at: the
    // size in the directory entry ends the read first
    let mut f = Fat::new(8192 * 2, 1);
    let first = f.add_file(&NAME, &d, Layout::Contig, false);
    let ch = f.chain(first);
    f.set_fat(*ch.last().unwrap(), ch[0]);
    expect_file(f.img.clone(), &d);
    // ... but one cluster too few in the size makes it a loop through the start
    let mut f = Fat::new(8192 * 2, 1);
    let first = f.add_file(&NAME, &d, Layout::Contig, false);
    let ch = f.chain(first);
    f.set_fat(*ch.last().unwrap(), ch[0]);
    let off = f.cluster_off(2); // root directory: patch the size to 150 clusters
    f.img[off + 28..off + 32].copy_from_slice(&((150 * 512) as u32).to_le_bytes());
    let (mut card, r) = open(f.img.clone(), &NAME);
    let (vol, ent) = r.unwrap();
    assert_eq!(read_all(&mut card, &vol, &ent), Err(FatError::Loop));
}

#[test]
fn directory_chain_loop_does_not_hang() {
    let mut f = Fat::new(8192 * 2, 1);
    for i in 0..48u32 {
        let mut n = *b"JUNK0000TXT";
        n[4..8].copy_from_slice(format!("{:04}", i).as_bytes());
        f.add_file(&n, &file(10), Layout::Contig, false);
    }
    let rc = f.root_chain.clone();
    assert!(rc.len() >= 3);
    f.set_fat(*rc.last().unwrap(), rc[1]);
    let (_, r) = open(f.img.clone(), &NAME);
    assert_eq!(r.err(), Some(FatError::Loop));
    // root chain pointing to itself
    let mut f = Fat::new(8192 * 2, 1);
    for i in 0..16u32 {
        let mut n = *b"JUNK0000TXT";
        n[4..8].copy_from_slice(format!("{:04}", i).as_bytes());
        f.push_entry(&Fat::dirent(&n, 0x20, 0, 0));
    }
    f.set_fat(2, 2);
    let (_, r) = open(f.img.clone(), &NAME);
    assert_eq!(r.err(), Some(FatError::Loop));
}

#[test]
fn truncated_and_invalid_chains() {
    let d = file(10 * 512);
    // chain ends early
    let mut f = Fat::new(8192 * 2, 1);
    let first = f.add_file(&NAME, &d, Layout::Contig, false);
    let ch = f.chain(first);
    f.set_fat(ch[4], EOC);
    let (mut card, r) = open(f.img.clone(), &NAME);
    let (vol, ent) = r.unwrap();
    assert_eq!(read_all(&mut card, &vol, &ent), Err(FatError::Truncated));

    for (what, v) in [("free", 0u32), ("reserved 1", 1), ("bad cluster", 0x0FFF_FFF7), ("beyond volume", 0x0FFF_0000)] {
        let mut f = Fat::new(8192 * 2, 1);
        let first = f.add_file(&NAME, &d, Layout::Contig, false);
        let ch = f.chain(first);
        f.set_fat(ch[4], v);
        let (mut card, r) = open(f.img.clone(), &NAME);
        let (vol, ent) = r.unwrap();
        assert_eq!(read_all(&mut card, &vol, &ent), Err(FatError::BadChain), "{what}");
    }

    // bad directory entries
    let mut f = Fat::new(8192 * 2, 1);
    f.push_entry(&Fat::dirent(&NAME, 0x20, 0, 500)); // first cluster 0
    assert_eq!(open(f.img.clone(), &NAME).1.err(), Some(FatError::BadChain));
    let mut f = Fat::new(8192 * 2, 1);
    f.push_entry(&Fat::dirent(&NAME, 0x20, 0x0FFF_0000, 500)); // beyond the volume
    assert_eq!(open(f.img.clone(), &NAME).1.err(), Some(FatError::BadChain));
    let mut f = Fat::new(8192 * 2, 1);
    f.push_entry(&Fat::dirent(&NAME, 0x20, 3, 0)); // empty
    assert_eq!(open(f.img.clone(), &NAME).1.err(), Some(FatError::Empty));
    let mut f = Fat::new(8192 * 2, 1);
    f.push_entry(&Fat::dirent(&NAME, 0x20, 3, 0xFFFF_FFFF)); // larger than the volume
    assert_eq!(open(f.img.clone(), &NAME).1.err(), Some(FatError::BadChain));
}

#[test]
fn read_errors_propagate_at_every_read() {
    let d = file(20_000);
    let mut f = Fat::new(8192 * 2, 2);
    f.add_file(&NAME, &d, Layout::Stride3, false);
    let img = with_mbr(&f.img, 0x0C);
    // how many reads does a full open + read take?
    let (mut card, r) = open(img.clone(), &NAME);
    let (vol, ent) = r.unwrap();
    assert_eq!(read_all(&mut card, &vol, &ent).unwrap(), d);
    let total = card.reads;
    for fail in 0..total {
        let mut card = Card::new(img.clone());
        card.fail_at = Some(fail);
        let mut buf = [0u8; 512];
        let res = Volume::mount(&mut card, &mut buf)
            .and_then(|v| v.find(&mut card, &mut buf, &NAME).map(|e| (v, e)))
            .and_then(|(v, e)| read_all(&mut card, &v, &e));
        assert_eq!(res, Err(FatError::Read), "fail at read {fail}");
    }
}

#[test]
fn only_one_buffer_is_used() {
    // The whole read goes through a single 512-byte buffer: after every
    // call the buffer holds only the latest chunk, the FAT and directory
    // reads do not need a second one. (Covered by read_all above; here the
    // cursor state is checked to be small.)
    assert!(std::mem::size_of::<FileReader>() <= 24);
    assert!(std::mem::size_of::<Volume>() <= 28);
}
