//! CRC, EEFC command words, app descriptor/vectors, OGFW format 3, boot
//! decision. Vectors are the files shared with `mcu/fwupdater` and
//! `tools/fwimage`.

mod common;

use bootcore::app::*;
use bootcore::boot::*;
use bootcore::crc;
use bootcore::fcr::*;
use bootcore::ogfw::*;
use bootcore::part::Part;
use common::*;

fn num(s: &str) -> u32 {
    u32::from_str_radix(s.trim_start_matches("0x"), 16).unwrap()
}

fn hex(s: &str) -> Vec<u8> {
    if s == "-" {
        return vec![];
    }
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect()
}

fn cases(text: &'static str) -> impl Iterator<Item = Vec<&'static str>> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.split_whitespace().collect())
}

fn part(s: &str) -> Part {
    match s {
        "2e" => Part::Sam3u2e,
        "4e" => Part::Sam3u4e,
        _ => panic!("part {s}"),
    }
}

// ------------------------------------------------------------------ CRC

#[test]
fn crc_vectors_shared_with_fwimage() {
    let mut n = 0;
    for c in cases(include_str!("../../../tools/fwimage/vectors/crc32.txt")) {
        assert_eq!(crc::crc32(&hex(c[0])), num(c[1]), "{}", c[0]);
        n += 1;
    }
    assert!(n >= 4);
    assert_eq!(crc::crc32(b"123456789"), 0xCBF4_3926);
}

/// The bit-serial reference of mcu/fwupdater/src/logic.rs.
fn ref_crc(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[test]
fn nibble_crc_equals_bitwise_crc() {
    for n in 0..300 {
        let d = lcg(n as u32, n);
        assert_eq!(crc::crc32(&d), ref_crc(&d), "len {n}");
    }
    let d = lcg(7, 70_000);
    assert_eq!(crc::crc32(&d), ref_crc(&d));
    // streaming in odd pieces
    let mut c = crc::INIT;
    for p in d.chunks(333) {
        c = crc::update(c, p);
    }
    assert_eq!(crc::finish(c), ref_crc(&d));
}

#[test]
fn trailing_crc_residue() {
    for n in [1usize, 4, 100, 508, 4096] {
        let mut d = lcg(n as u32, n);
        let c = crc::crc32(&d);
        d.extend_from_slice(&c.to_le_bytes());
        assert_eq!(crc::crc32(&d), crc::RESIDUE);
    }
}

// ------------------------------------------------------------------ fcr_word

const SGPB_WORDS: [u32; 3] = [0x5A00_000B, 0x5A00_010B, 0x5A00_020B];

#[test]
fn fcr_exhaustive_over_every_command_and_argument() {
    // [part][allow_slb][plane]
    let mut counts = [[[0usize; 2]; 2]; 2];
    for (ti, part) in [Part::Sam3u2e, Part::Sam3u4e].into_iter().enumerate() {
        for (ai, allow) in [false, true].into_iter().enumerate() {
            for (pi, plane) in [Plane::P0, Plane::P1].into_iter().enumerate() {
                for cmd in 0..=255u8 {
                    let mut args: Vec<u32> = (0..=0x3FFFu32).collect();
                    args.extend([0xFFFF, 0x1_0000, 0x1_0100, 0x00FF_FFFF, 0x0100_0000, 0x7FFF_FFFF, 0x8000_0000, u32::MAX]);
                    for arg in args {
                        let Some(w) = fcr_word_raw(part, plane, cmd, arg, allow) else { continue };
                        counts[ti][ai][pi] += 1;
                        // shape
                        assert_eq!(w >> 24, 0x5A, "key");
                        assert_eq!(w & 0xFF, cmd as u32);
                        assert_eq!((w >> 8) & 0xFFFF, arg);
                        assert!(arg < 0x10000);
                        // only the allowlist
                        assert!(matches!(cmd, CMD_EWP | CMD_GLB | CMD_SLB), "cmd {cmd:#x} produced");
                        // never SGPB 0x0B, CGPB 0x0C, CLB 0x09, EA 0x05, WP* ...
                        for bad in [0x00u8, 0x01, 0x02, 0x04, 0x05, 0x09, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F] {
                            assert_ne!(w & 0xFF, bad as u32);
                        }
                        assert!(!SGPB_WORDS.contains(&w));
                        // no plane 1 on a part without it
                        assert!(plane == Plane::P0 || part.has_plane1());
                        match cmd {
                            CMD_EWP if plane == Plane::P0 => assert!((32..512).contains(&arg)),
                            CMD_EWP => assert!(arg < 512),
                            CMD_GLB => assert_eq!(arg, GLB_FARG),
                            CMD_SLB => {
                                assert!(allow && plane == Plane::P0 && arg == 0);
                            }
                            _ => unreachable!(),
                        }
                    }
                }
            }
        }
    }
    // exactly the intended set: EWP 480 (P0) / 512 (P1), GLB 1, SLB 1 on P0
    assert_eq!(counts[0][0], [480 + 1, 0]);
    assert_eq!(counts[0][1], [480 + 1 + 1, 0]);
    assert_eq!(counts[1][0], [480 + 1, 512 + 1]);
    assert_eq!(counts[1][1], [480 + 1 + 1, 512 + 1]);
}

#[test]
fn fcr_public_entry_points() {
    let (e, f) = (Part::Sam3u2e, Part::Sam3u4e);
    let w = |c: Option<FcrCmd>| c.map(|c| c.word());
    assert_eq!(w(FcrCmd::ewp(e, Plane::P0, 32)), Some(0x5A00_2003));
    assert_eq!(w(FcrCmd::ewp(e, Plane::P0, 511)), Some(0x5A01_FF03));
    assert_eq!(w(FcrCmd::glb(e, Plane::P0)), Some(0x5A00_200A));
    assert_eq!(w(FcrCmd::slb(e, true)), Some(0x5A00_0008));
    for page in 0..32 {
        assert!(FcrCmd::ewp(e, Plane::P0, page).is_none(), "EWP into the resident, page {page}");
        assert!(FcrCmd::ewp(f, Plane::P0, page).is_none());
    }
    assert!(FcrCmd::ewp(e, Plane::P0, 512).is_none());
    assert!(FcrCmd::new(e, Plane::P0, Cmd::Glb, 0, false).is_none());
    assert!(FcrCmd::new(e, Plane::P0, Cmd::Glb, 3, false).is_none());
    assert!(FcrCmd::new(e, Plane::P0, Cmd::Slb, 1, true).is_none());
    assert!(FcrCmd::new(e, Plane::P0, Cmd::Slb, 31, true).is_none());
    assert!(FcrCmd::new(e, Plane::P0, Cmd::Slb, 32, true).is_none());
    assert!(FcrCmd::slb(e, false).is_none(), "an updater build may not lock");
    assert!(FcrCmd::slb(f, false).is_none());
    // no plane 1 command at all on a 2E
    assert!(FcrCmd::ewp(e, Plane::P1, 0).is_none());
    assert!(FcrCmd::glb(e, Plane::P1).is_none());
    assert!(FcrCmd::new(e, Plane::P1, Cmd::Slb, 0, true).is_none());
    assert!(FcrCmd::new(f, Plane::P1, Cmd::Slb, 0, true).is_none());
    assert_eq!(w(FcrCmd::ewp(f, Plane::P1, 0)), Some(0x5A00_0003));
    assert_eq!(w(FcrCmd::glb(f, Plane::P1)), Some(0x5A00_200A));
    // the register block travels with the word
    let c0 = FcrCmd::ewp(f, Plane::P0, 40).unwrap();
    let c1 = FcrCmd::ewp(f, Plane::P1, 40).unwrap();
    assert_eq!((c0.eefc_base(), c0.fcr_addr(), c0.fsr_addr()), (0x400E_0800, 0x400E_0804, 0x400E_0808));
    assert_eq!((c1.eefc_base(), c1.fcr_addr(), c1.fsr_addr()), (0x400E_0A00, 0x400E_0A04, 0x400E_0A08));
    assert_eq!(c0.word(), c1.word());
    assert_ne!(c0, c1);
    assert_eq!((c0.page_addr(), c1.page_addr()), (0x0008_0000 + 40 * 256, 0x0010_0000 + 40 * 256));
    assert_eq!((c0.plane(), c1.plane(), c0.code(), c0.arg()), (Plane::P0, Plane::P1, 3, 40));
    assert_eq!(FcrCmd::glb(e, Plane::P0).unwrap().eefc_base(), 0x400E_0800);
    assert_eq!(FcrCmd::slb(e, true).unwrap().eefc_base(), 0x400E_0800);
}

fn dist(a: u32, b: u32) -> u32 {
    (a ^ b).count_ones()
}

/// The design remark (3-4), extended: every word the crate can produce
/// (EWP of every page of both planes, GLB, SLB) against every word that does
/// harm (SGPB and CGPB of GPNVM 0..2, EA, WP, CLB with every argument
/// 0..512). Whatever one flipped bit does to a produced word, it must not be
/// a dangerous command. The exceptions are named (see the match below).
#[test]
fn produced_words_are_two_bits_from_dangerous_words() {
    // dangerous words: (name, word)
    let mut danger: Vec<(&str, u32)> = vec![];
    for g in 0..3u32 {
        danger.push(("SGPB", 0x5A00_000B | (g << 8)));
        danger.push(("CGPB", 0x5A00_000C | (g << 8)));
    }
    for a in 0..512u32 {
        danger.push(("EA", 0x5A00_0005 | (a << 8)));
        danger.push(("WP", 0x5A00_0001 | (a << 8)));
        danger.push(("CLB", 0x5A00_0009 | (a << 8)));
    }
    let f = Part::Sam3u4e;
    // (plane, name, word) of everything producible
    let mut mine: Vec<(Plane, &str, u32)> = vec![];
    for page in 32..512 {
        mine.push((Plane::P0, "EWP", FcrCmd::ewp(f, Plane::P0, page).unwrap().word()));
    }
    for page in 0..512 {
        mine.push((Plane::P1, "EWP", FcrCmd::ewp(f, Plane::P1, page).unwrap().word()));
    }
    mine.push((Plane::P0, "GLB", FcrCmd::glb(f, Plane::P0).unwrap().word()));
    mine.push((Plane::P1, "GLB", FcrCmd::glb(f, Plane::P1).unwrap().word()));
    mine.push((Plane::P0, "SLB", FcrCmd::slb(f, true).unwrap().word()));
    assert_eq!(mine.len(), 480 + 512 + 3);

    let mut min_p0 = 32;
    let mut min_p1 = 32;
    let mut ones: Vec<String> = vec![];
    for &(plane, name, w) in &mine {
        for &(dn, d) in &danger {
            let k = dist(w, d);
            if plane == Plane::P0 {
                min_p0 = min_p0.min(k);
            } else {
                min_p1 = min_p1.min(k);
            }
            if k >= 2 {
                continue;
            }
            assert_eq!(k, 1, "a produced word equals a dangerous one: {name} {w:#x} = {dn}");
            // The exceptions that do no harm:
            let ok = match (plane, name, dn) {
                // EWP vs WP of the same page: WP writes without erasing the
                // page the caller means to write anyway (same page, >= 32 on
                // plane 0, so never the resident); the read-back catches it.
                (_, "EWP", "WP") => true,
                // SLB 0 vs CGPB 0: CGPB of GPNVM0 cannot clear the security
                // bit (only the ERASE pin does, datasheet lines 1729-1731).
                (Plane::P0, "SLB", "CGPB") => w == 0x5A00_0008 && d == 0x5A00_000C,
                // SLB 0 vs CLB 0: NOT harmless in principle (CLB 0 unlocks
                // the resident region) and not removable: argument 0 is the
                // only one that means region 0 under both readings of FARG.
                // It takes a one-bit error in the single FCR write of the
                // lock step; the resident reads GLB afterwards.
                (Plane::P0, "SLB", "CLB") => w == 0x5A00_0008 && d == 0x5A00_0009,
                // EEFC1 has no GPNVM bits (datasheet 8.2.3.10): an SGPB or
                // CGPB that reaches it changes nothing.
                (Plane::P1, _, "SGPB" | "CGPB") => true,
                _ => false,
            };
            assert!(ok, "distance 1: {name} {w:#x} ({plane:?}) vs {dn} {d:#x}");
            ones.push(format!("{name} {w:#010x} vs {dn} {d:#010x}"));
        }
    }
    ones.sort();
    ones.dedup();
    println!("minimum Hamming distance, plane 0 words vs dangerous words: {min_p0}");
    println!("minimum Hamming distance, plane 1 words vs dangerous words: {min_p1}");
    println!("distinct distance-1 pairs ({}), all of the named harmless kinds", ones.len());
    // Plane 0 includes the exceptions above (EWP/WP, SLB/CGPB 0); everything
    // else on plane 0 is at least 2 apart. Count them separately:
    let mut min_p0_strict = 32;
    for &(plane, name, w) in &mine {
        if plane != Plane::P0 {
            continue;
        }
        for &(dn, d) in &danger {
            if name == "EWP" && dn == "WP" || name == "SLB" && (d == 0x5A00_000C || d == 0x5A00_0009) {
                continue;
            }
            min_p0_strict = min_p0_strict.min(dist(w, d));
        }
    }
    println!("minimum distance on plane 0 without the two named exceptions: {min_p0_strict}");
    assert!(min_p0_strict >= 2);
    // the one-bit flips of the GLB word, which the resident issues at every boot
    let glb = FcrCmd::glb(f, Plane::P0).unwrap().word();
    assert_eq!(glb, 0x5A00_200A);
    for bit in 0..32 {
        let flipped = glb ^ (1 << bit);
        assert!(!SGPB_WORDS.contains(&flipped), "GLB with bit {bit} flipped is SGPB of a real GPNVM bit");
    }
    assert_eq!(dist(glb, SGPB_WORDS[0]), 2);
}

// ------------------------------------------------------------------ descriptor

fn desc_bytes(len: u32) -> [u8; 32] {
    let mut d = [0u8; 32];
    put32(&mut d, 0, DESC_MAGIC);
    put32(&mut d, 4, 1);
    put32(&mut d, 8, len);
    put32(&mut d, 12, 0x0010_0000);
    put32(&mut d, 16, 1234);
    put32(&mut d, 20, 5678);
    put32(&mut d, 24, 0x0100_0000 | 0x00AB_CDEF);
    let c = crc::crc32(&d[..28]);
    put32(&mut d, 28, c);
    d
}

#[test]
fn descriptor_parse_and_reject() {
    let d = parse_descriptor(&desc_bytes(0x1000), false).unwrap();
    assert_eq!(d.image_len, 0x1000);
    assert_eq!(d.bs_addr, 0x0010_0000);
    assert_eq!(d.bs_len, 1234);
    assert_eq!(d.bs_raw_len, 5678);
    assert_eq!(d.bs_kind, 1);
    assert_eq!(d.app_version, 0x00AB_CDEF);

    // every bit flip in the 28 covered bytes is caught by magic/version/CRC
    for bit in 0..28 * 8 {
        let mut b = desc_bytes(0x1000);
        b[bit / 8] ^= 1 << (bit % 8);
        assert!(parse_descriptor(&b, false).is_err(), "bit {bit}");
    }
    // erased flash
    assert_eq!(parse_descriptor(&[0xFF; 32], false), Err(AppError::Descriptor));
    assert_eq!(parse_descriptor(&[0; 32], false), Err(AppError::Descriptor));
    // lengths
    for bad in [4u32, 256, 513, 0x1_E200, 0x2_0000, 0xFFFF_FE00] {
        let r = parse_descriptor(&desc_bytes(bad), false);
        assert_eq!(r, Err(AppError::Length), "{bad:#x}");
    }
    assert!(parse_descriptor(&desc_bytes(512), false).is_ok());
    assert!(parse_descriptor(&desc_bytes(APP_MAX_LEN), false).is_ok());
    // wrong version
    let mut b = desc_bytes(0x1000);
    put32(&mut b, 4, 2);
    assert_eq!(parse_descriptor(&b, false), Err(AppError::Descriptor));
}

#[test]
fn unsealed_descriptor_only_when_allowed() {
    // What an ELF holds: magic, version, zeros.
    let mut b = [0u8; 32];
    put32(&mut b, 0, DESC_MAGIC);
    put32(&mut b, 4, 1);
    assert_eq!(parse_descriptor(&b, false), Err(AppError::Unsealed));
    assert_eq!(parse_descriptor(&b, true).unwrap().image_len, 0);
    // but a non-zero length still needs its CRC
    put32(&mut b, 8, 0x1000);
    assert_eq!(parse_descriptor(&b, true), Err(AppError::DescriptorCrc));
}

#[test]
fn vector_vectors_shared_with_fwimage() {
    let mut n = 0;
    for c in cases(include_str!("../../../tools/fwimage/vectors/vtable.txt")) {
        // part dest len sp reset nmi hardfault expect
        let v = [num(c[3]), num(c[4]), num(c[5]), num(c[6])];
        let r = check_vectors(&v, num(c[1]), num(c[2]), part(c[0]));
        let want = if c[7] == "OK" { Ok(()) } else { Err(AppError::Vector) };
        assert_eq!(r, want, "{c:?}");
        n += 1;
    }
    assert!(n >= 10);
}

#[test]
fn vectors_with_app_base() {
    let ok = [0x2000_4000, APP_BASE + 1, APP_BASE + 3, APP_BASE + 5];
    assert_eq!(check_vectors(&ok, APP_BASE, 0x1000, Part::Sam3u2e), Ok(()));
    // a handler into the resident (an image still linked at 0x80000)
    let old = [0x2000_4000, 0x0008_0101, 0x0008_0103, 0x0008_0105];
    assert!(check_vectors(&old, APP_BASE, 0x1000, Part::Sam3u2e).is_err());
    // one byte past the end / last byte inside
    let edge = [0x2000_4000, APP_BASE + 0x1000 - 1, APP_BASE + 1, APP_BASE + 1];
    assert_eq!(check_vectors(&edge, APP_BASE, 0x1000, Part::Sam3u2e), Ok(()));
    let past = [0x2000_4000, APP_BASE + 0x1001, APP_BASE + 1, APP_BASE + 1];
    assert!(check_vectors(&past, APP_BASE, 0x1000, Part::Sam3u2e).is_err());
    // SRAM size depends on the part
    let sp8 = [0x2000_8000, APP_BASE + 1, APP_BASE + 1, APP_BASE + 1];
    assert!(check_vectors(&sp8, APP_BASE, 0x1000, Part::Sam3u2e).is_err());
    assert!(check_vectors(&sp8, APP_BASE, 0x1000, Part::Sam3u4e).is_ok());
}

// ------------------------------------------------------------------ OGFW 3

fn verdict(r: Result<(), Reject>) -> String {
    match r {
        Ok(()) => "OK".into(),
        Err(e) => format!("{e:?}"),
    }
}

/// header3.txt: the format-3 reader, row by row, same expectations as the
/// Python tool, the updater's logic.rs and the C sender.
#[test]
fn header3_vectors_shared_with_fwimage_and_updater() {
    let mut n = 0;
    for c in cases(include_str!("../../../tools/fwimage/vectors/header3.txt")) {
        let raw = hex(c[1]);
        assert_eq!(raw.len(), 40);
        let mut blk = vec![0u8; 512];
        blk[..40].copy_from_slice(&raw);
        let r = parse_header(blk.as_slice().try_into().unwrap()).and_then(|h| check_header(&h, part(c[0])));
        assert_eq!(verdict(r), c[2], "{c:?}");
        n += 1;
    }
    assert!(n >= 20, "{n}");
}

/// header.txt rows are format-2 headers (36 bytes): this reader takes format
/// 3 only, so every one of them, whatever its format-2 verdict, is a
/// BadHeader here (a `format = 3` row there is a header without a valid
/// layout/CRC here).
#[test]
fn format_2_header_rows_are_bad_headers_here() {
    let mut n = 0;
    for c in cases(include_str!("../../../tools/fwimage/vectors/header.txt")) {
        let raw = hex(c[1]);
        let mut blk = vec![0u8; 512];
        blk[..raw.len()].copy_from_slice(&raw);
        let r = parse_header(blk.as_slice().try_into().unwrap()).and_then(|h| check_header(&h, part(c[0])));
        assert_eq!(verdict(r), "BadHeader", "{c:?}");
        n += 1;
    }
    assert!(n >= 10);
}

/// desc.txt: descriptor bytes, image length, expect.
#[test]
fn descriptor_vectors_shared() {
    let mut n = 0;
    for c in cases(include_str!("../../../tools/fwimage/vectors/desc.txt")) {
        let d: [u8; 32] = hex(c[0]).try_into().unwrap();
        let got = match check_descriptor(&d, num(c[1])) {
            Ok(_) => "OK",
            Err(AppError::Unsealed) => "Unsealed",
            Err(_) => "BadDescriptor",
        };
        assert_eq!(got, c[2], "{c:?}");
        n += 1;
    }
    assert!(n >= 10);
}

/// trailer.txt: image bytes (trailer included), expect.
#[test]
fn trailer_vectors_shared() {
    let mut n = 0;
    for c in cases(include_str!("../../../tools/fwimage/vectors/trailer.txt")) {
        let got = if check_trailer(&hex(c[0])) { "OK" } else { "BadTrailer" };
        assert_eq!(got, c[1], "{c:?}");
        n += 1;
    }
    assert!(n >= 8);
}

/// image3.txt: a whole one-image file, checked end to end in the documented
/// order (header, target, layout, dest, length, payload CRC, vectors,
/// descriptor, trailer).
#[test]
fn image3_vectors_shared() {
    let mut n = 0;
    for c in cases(include_str!("../../../tools/fwimage/vectors/image3.txt")) {
        let file = hex(c[1]);
        let p = part(c[0]);
        let blk: &[u8; 512] = file[..512].try_into().unwrap();
        let got = match parse_header(blk).and_then(|h| check_header(&h, p).map(|_| h)) {
            Err(e) => format!("{e:?}"),
            Ok(h) => {
                assert_eq!(file.len() - 512, h.len as usize);
                let mut st = ImageStream::new(&h, p);
                for ch in file[512..].chunks(512) {
                    st.feed(ch.try_into().unwrap()).unwrap();
                }
                match st.finish() {
                    Ok(()) => "OK".to_string(),
                    Err(e) => format!("{e:?}"),
                }
            }
        };
        assert_eq!(got, c[2], "{} {}", c[0], &c[1][..40]);
        n += 1;
    }
    assert!(n >= 10);
}

#[test]
fn lock_masks_equal_the_updaters() {
    use bootcore::flash::{FLASH_BASE, PLANE1_BASE};
    use bootcore::update::lock_mask_at;
    assert_eq!(lock_mask_at(FLASH_BASE, 512), 0x0001);
    assert_eq!(lock_mask_at(FLASH_BASE, 0x2_0000), 0xFFFF);
    assert_eq!(lock_mask_at(FLASH_BASE, 8193), 0x0003);
    assert_eq!(lock_mask_at(APP_BASE, 512), 0x0002);
    assert_eq!(lock_mask_at(APP_BASE, 8192), 0x0002);
    assert_eq!(lock_mask_at(APP_BASE, 8193), 0x0006);
    assert_eq!(lock_mask_at(APP_BASE, 0x1E000), 0xFFFE);
    assert_eq!(lock_mask_at(FLASH_BASE, 0x2000), 0x0001);
    assert_eq!(lock_mask_at(FLASH_BASE + 0x1FFF, 2), 0x0003);
    assert_eq!(lock_mask_at(PLANE1_BASE, 0x2_0000), 0xFFFF);
    assert_eq!(lock_mask_at(PLANE1_BASE, 9000), 0x0003);
    assert_eq!(lock_mask_at(PLANE1_BASE + 8192, 512), 0x0002);
    assert_eq!(lock_mask_at(FLASH_BASE, 0), 0);
    // out of range: no underflow, "everything locked"
    assert_eq!(lock_mask_at(0, 512), u32::MAX);
    assert_eq!(lock_mask_at(FLASH_BASE - 1, 512), u32::MAX);
    assert_eq!(lock_mask_at(0x7_FFFF, 2), u32::MAX);
    assert_eq!(lock_mask_at(FLASH_BASE + 0x2_0000 - 1, 2), u32::MAX);
    assert_eq!(lock_mask_at(PLANE1_BASE + 0x2_0000, 512), u32::MAX);
    assert_eq!(lock_mask_at(FLASH_BASE, u32::MAX), u32::MAX);
    assert_eq!(lock_mask_at(u32::MAX, u32::MAX), u32::MAX);
    assert_eq!(lock_mask_at(0, 0), 0);
}

#[test]
fn header_format_3_rules() {
    let part = Part::Sam3u2e;
    let pl = app_image(part, 512, 1);
    let ok = header(0, part, APP_BASE, &pl);
    let h = parse_header(ok.as_slice().try_into().unwrap()).unwrap();
    assert_eq!(check_header(&h, part), Ok(()));
    assert_eq!((h.kind, h.dest, h.len), (0, APP_BASE, 512));

    let parse = |b: &[u8]| parse_header(b.try_into().unwrap());
    // layout word
    let mut b = ok.clone();
    put32(&mut b, 36, 0x4000);
    assert_eq!(parse(&b), Err(Reject::BadHeader), "w9 is under the header CRC");
    fix_header_crc(&mut b);
    assert_eq!(check_header(&parse(&b).unwrap(), part), Err(Reject::BadDest));
    put32(&mut b, 36, 0);
    fix_header_crc(&mut b);
    assert_eq!(check_header(&parse(&b).unwrap(), part), Err(Reject::BadDest));
    // format 2 and 4
    for f in [1u32, 2, 4] {
        let mut b = ok.clone();
        put32(&mut b, 4, f);
        fix_header_crc(&mut b);
        assert_eq!(parse(&b), Err(Reject::BadHeader));
    }
    // dest: anything but APP_BASE for an app is BadDest, whatever the length
    for dest in [0u32, 0x0008_0000, 0x0008_1F00, 0x0008_1FFF, 0x0008_2100, 0x0008_4000, 0x0010_0000] {
        let b = header(0, part, dest, &pl);
        let h = parse(&b).unwrap();
        assert_eq!(check_header(&h, part), Err(Reject::BadDest), "{dest:#x}");
    }
    // length limits: 0x20000 - layout for the app, 0x20000 for a bitstream
    let big = |kind, len: usize, p: Part| {
        let dest = if kind == 0 { APP_BASE } else { 0x0010_0000 };
        let mut b = header(kind, p, dest, &vec![0u8; 512]);
        put32(&mut b, 20, len as u32);
        fix_header_crc(&mut b);
        let h = parse(&b).unwrap();
        check_header(&h, p)
    };
    assert_eq!(big(0, 0x1E000, part), Ok(()));
    assert_eq!(big(0, 0x1E200, part), Err(Reject::BadLength));
    assert_eq!(big(0, 0x2_0000, part), Err(Reject::BadLength));
    assert_eq!(big(0, 0, part), Err(Reject::BadLength));
    assert_eq!(big(0, 100, part), Err(Reject::BadLength));
    assert_eq!(big(1, 0x2_0000, Part::Sam3u4e), Ok(()));
    assert_eq!(big(1, 0x2_0200, Part::Sam3u4e), Err(Reject::BadLength));
    // bitstream only on the 4E
    assert_eq!(big(1, 512, Part::Sam3u2e), Err(Reject::BadDest));
    // wrong target
    let b = header(0, Part::Sam3u4e, APP_BASE, &pl);
    assert_eq!(check_header(&parse(&b).unwrap(), part), Err(Reject::BadTarget));
    // later silicon revision still matches
    let mut b = header(0, part, APP_BASE, &pl);
    put32(&mut b, 12, cidr(part) | 5);
    fix_header_crc(&mut b);
    assert_eq!(check_header(&parse(&b).unwrap(), part), Ok(()));
}

#[test]
fn image_stream_checks() {
    let part = Part::Sam3u2e;
    let run = |payload: &[u8], mutate: &dyn Fn(&mut Vec<u8>, &mut Vec<u8>)| -> Result<(), ImageError> {
        let mut hdr = header(0, part, APP_BASE, payload);
        let mut pl = payload.to_vec();
        mutate(&mut hdr, &mut pl);
        let h = parse_header(hdr.as_slice().try_into().unwrap()).unwrap();
        let mut st = ImageStream::new(&h, part);
        for c in pl.chunks(512) {
            st.feed(c.try_into().unwrap())?;
        }
        st.finish()
    };
    let good = app_image(part, 2048, 3);
    assert_eq!(run(&good, &|_, _| {}), Ok(()));
    // payload byte flipped after the header was made
    assert_eq!(run(&good, &|_, p| p[1000] ^= 1), Err(ImageError::CrcMismatch));
    // descriptor length differs from the header's
    let other = app_image(part, 1024, 3);
    let mut padded = other.clone();
    padded.extend_from_slice(&other);
    assert_eq!(run(&padded, &|_, _| {}), Err(ImageError::BadDescriptor));
    // vectors into the resident
    let mut bad = good.clone();
    put32(&mut bad, 4, 0x0008_0101);
    let c = crc::crc32(&bad[..2044]);
    put32(&mut bad, 2044, c);
    assert_eq!(run(&bad, &|_, _| {}), Err(ImageError::BadVector));
    // trailing CRC wrong but payload CRC (made after the damage) right
    let mut bad = good.clone();
    bad[2000] ^= 0x40;
    assert_eq!(run(&bad, &|_, _| {}), Err(ImageError::BadTrailer));
    // truncated
    let h = parse_header(header(0, part, APP_BASE, &good).as_slice().try_into().unwrap()).unwrap();
    let mut st = ImageStream::new(&h, part);
    st.feed(good[..512].try_into().unwrap()).unwrap();
    assert_eq!(st.finish(), Err(ImageError::Truncated));
    // too many chunks
    let mut st = ImageStream::new(&h, part);
    for c in good.chunks(512) {
        st.feed(c.try_into().unwrap()).unwrap();
    }
    assert_eq!(st.feed(&[0; 512]), Err(ImageError::Truncated));
    // an unsealed descriptor is not acceptable in a recovery file
    let mut un = good.clone();
    for b in &mut un[0x108..0x120] {
        *b = 0;
    }
    let c = crc::crc32(&un[..2044]);
    put32(&mut un, 2044, c);
    assert_eq!(run(&un, &|_, _| {}), Err(ImageError::Unsealed));
}

// ------------------------------------------------------------------ boot decision

#[test]
fn boot_decision_explicit_rows() {
    const M: u32 = GPBR_RECOVER_MAGIC;
    const B: u32 = GPBR_RECOVER_BUSY;
    use Request::{Interrupted, New};
    let run = |r: Request| Action::RunApp { request: r };
    let rec = |r: Request| Action::Recover { request: r };
    let none = Request::None;
    // (app_valid, button, gpbr0, expected)
    let rows: [(bool, bool, u32, Action); 24] = [
        // D5: no request, valid app -> run, nothing about files on the card
        (true, false, 0, run(none)),
        (true, false, 0xFFFF_FFFF, run(none)),
        (true, false, M ^ 1, run(none)),
        (true, false, 0x1234_5678, run(none)),
        // the button
        (true, true, 0, rec(none)),
        (true, true, 0xFFFF_FFFF, rec(none)),
        (true, true, M ^ 1, rec(none)),
        // the request of the application: recover, GPBR0 becomes "in progress"
        (true, false, M, rec(New)),
        (true, true, M, rec(New)),
        // no valid application
        (false, false, 0, rec(none)),
        (false, false, 0xFFFF_FFFF, rec(none)),
        (false, false, M ^ 1, rec(none)),
        (false, false, 0x1234_5678, rec(none)),
        (false, true, 0, rec(none)),
        (false, true, M ^ 1, rec(none)),
        // an invalid app and a request
        (false, false, M, rec(New)),
        (false, true, M, rec(New)),
        (false, true, 0xFFFF_FFFF, rec(none)),
        // B6: a requested recovery was cut short by a reset. A valid app runs
        // (no second recovery on that ground); the button or an invalid app
        // still recover, and the request is remembered for the report.
        (true, false, B, run(Interrupted)),
        (true, true, B, rec(Interrupted)),
        (false, false, B, rec(Interrupted)),
        (false, true, B, rec(Interrupted)),
        // one bit off is nothing
        (true, false, B ^ 1, run(none)),
        (false, false, B ^ 0x8000_0000, rec(none)),
    ];
    for (valid, button, g, want) in rows {
        assert_eq!(decide(valid, button, g), want, "valid={valid} button={button} gpbr={g:#x}");
    }
    // the two words, and what the resident writes over them
    assert_eq!(B, 0xBCAD_B8B1);
    assert_eq!((M ^ B).count_ones(), 32);
    assert!(B != 0 && B != u32::MAX && B >> 16 != GPBR1_MARK);
    assert_eq!(Request::of(M), New);
    assert_eq!(Request::of(B), Interrupted);
    for g in [0u32, u32::MAX, M ^ 1, B ^ 1, 0x4EB1_0001, M.swap_bytes(), B.swap_bytes()] {
        assert_eq!(Request::of(g), none, "{g:#x}");
    }
    assert_eq!(none.gpbr0_before_recovery(), None);
    assert_eq!(New.gpbr0_before_recovery(), Some(B));
    assert_eq!(Interrupted.gpbr0_before_recovery(), None, "it stays BUSY while the resident recovers");
    assert_eq!(none.gpbr0_at_handoff(), None, "GPBR0 is not the resident's to change");
    assert_eq!(New.gpbr0_at_handoff(), Some(0));
    assert_eq!(Interrupted.gpbr0_at_handoff(), Some(0));
    assert!(!none.reported() && New.reported() && Interrupted.reported());
    assert!(WDT_RESTART_TO_JUMP_US >= 100);
    assert_ne!(GPBR_RECOVER_MAGIC, 0);
    assert_ne!(GPBR_RECOVER_MAGIC, u32::MAX);
    assert!(button_held([true, true, true]));
    for i in 0..3 {
        let mut s = [true; 3];
        s[i] = false;
        assert!(!button_held(s));
    }
    // B8: a valid application always starts; without one the resident keeps
    // polling, until this boot has written MAX_WRITING_ATTEMPTS times
    assert_eq!(MAX_WRITING_ATTEMPTS, 3);
    for n in 0..=255u8 {
        assert_eq!(after_failed_recovery(true, n), Fallback::StartApp);
        let want = if n >= MAX_WRITING_ATTEMPTS { Fallback::GiveUp } else { Fallback::RetryRecovery };
        assert_eq!(after_failed_recovery(false, n), want, "{n}");
    }
}
