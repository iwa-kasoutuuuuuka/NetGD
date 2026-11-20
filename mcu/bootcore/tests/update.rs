//! The SD recovery state machine: normal runs and refusals.

mod common;

use bootcore::app::{verify_flash_app, APP_BASE};
use bootcore::fat::{FatError, NAME_APP, NAME_BITSTREAM};
use bootcore::flash::{FlashError, PLANE1_BASE};
use bootcore::ogfw::{ImageError, Reject};
use bootcore::part::Part;
use bootcore::update::*;
use common::*;

pub fn sd_file(file: &[u8], layout: Layout) -> Vec<u8> {
    let mut f = Fat::new(8192 * 2, 1);
    f.add_file(b"JUNK    TXT", &lcg(1, 700), Layout::Stride3, true);
    f.add_file(&NAME_APP, file, layout, true);
    with_mbr(&f.img, 0x0C)
}

fn recover(m: &mut Model, part: Part, card: &mut Card) -> Result<u32, RecoverError> {
    let mut buf = [0u8; 512];
    sd_recover(card, m, part, &mut buf, &NAME_APP)
}

fn valid(m: &Model, part: Part) -> Option<u32> {
    let mut m = m.clone();
    m.power_up();
    verify_flash_app(&mut m, part, false).ok()
}

fn junk_app_area(m: &mut Model) {
    m.poke(APP_BASE, &lcg(5, 0x1E000));
}

#[test]
fn recovers_on_blank_flash_2e() {
    for len in [512usize, 1024, 16384, 0x1E000] {
        let part = Part::Sam3u2e;
        let app = app_image(part, len, len as u32);
        let mut m = Model::new(part);
        let resident = m.peek(FLASH_START, 0x2000).to_vec();
        let mut card = Card::new(sd_file(&image(0, part, &app), Layout::Contig));
        assert!(valid(&m, part).is_none());
        let r = recover(&mut m, part, &mut card);
        assert_eq!(r, Ok(len as u32), "len {len}");
        assert_eq!(m.peek(APP_BASE, len), &app[..]);
        assert_eq!(valid(&m, part), Some(len as u32));
        assert_eq!(m.violations, 0, "flash read while a command was busy");
        assert_eq!(m.flocke, 0);
        assert_eq!(m.peek(FLASH_START, 0x2000), &resident[..], "resident touched");
        // every EWP in the app area (plane 0) or plane 1, none in the resident
        assert!(m.log.iter().all(|&a| a >= APP_BASE));
        assert_eq!(m.ops, len / 256 + 2, "two extra for the early erase");
    }
}

const FLASH_START: u32 = 0x0008_0000;

#[test]
fn write_order_first_two_pages_erased_first_written_last() {
    let part = Part::Sam3u2e;
    let app = app_image(part, 8192, 3);
    let mut m = Model::new(part);
    junk_app_area(&mut m);
    let mut card = Card::new(sd_file(&image(0, part, &app), Layout::Stride3));
    recover(&mut m, part, &mut card).unwrap();
    let n = m.log.len();
    assert_eq!(&m.log[..2], &[APP_BASE + 256, APP_BASE], "descriptor page erased first");
    assert_eq!(&m.log[n - 2..], &[APP_BASE, APP_BASE + 256], "vector page then descriptor page last");
    assert!(m.log[2..n - 2].iter().all(|&a| a >= APP_BASE + 512));
    // every page exactly once besides the early erases
    let mut body: Vec<u32> = m.log[2..n - 2].to_vec();
    body.sort();
    body.dedup();
    assert_eq!(body.len(), n - 4);
}

#[test]
fn replaces_a_valid_app_and_leaves_pages_beyond_a_shorter_one() {
    let part = Part::Sam3u2e;
    let old = app_image(part, 12288, 1);
    let new = app_image(part, 4096, 2);
    let mut m = Model::new(part);
    junk_app_area(&mut m);
    m.poke(APP_BASE, &old);
    assert_eq!(valid(&m, part), Some(12288));
    let mut card = Card::new(sd_file(&image(0, part, &new), Layout::Contig));
    assert_eq!(recover(&mut m, part, &mut card), Ok(4096));
    assert_eq!(valid(&m, part), Some(4096));
    assert_eq!(m.peek(APP_BASE, 4096), &new[..]);
    // the old tail is still there, harmless
    assert_eq!(m.peek(APP_BASE + 4096, 4096), &old[4096..8192]);
}

#[test]
fn recovers_on_4e_with_bitstream_in_either_order() {
    let part = Part::Sam3u4e;
    let app = app_image(part, 8192, 4);
    let bs = bitstream(0x1_0000, 4);
    for bs_first in [false, true] {
        let mut file = vec![];
        let (a, b) = (image(0, part, &app), image(1, part, &bs));
        if bs_first {
            file.extend(&b);
            file.extend(&a);
        } else {
            file.extend(&a);
            file.extend(&b);
        }
        let mut m = Model::new(part);
        let mut card = Card::new(sd_file(&file, Layout::Reverse));
        assert_eq!(recover(&mut m, part, &mut card), Ok(8192));
        assert_eq!(m.peek(APP_BASE, 8192), &app[..]);
        assert_eq!(m.peek(PLANE1_BASE, bs.len()), &bs[..]);
        assert_eq!(valid(&m, part), Some(8192));
        assert_eq!(m.violations, 0);
        // the app was invalidated before the bitstream was touched
        let first_bs = m.log.iter().position(|&a| a >= PLANE1_BASE).unwrap();
        assert!(first_bs >= 2);
        assert_eq!(&m.log[..2], &[APP_BASE + 256, APP_BASE]);
        // and the app's own first pages are the very last two writes
        let n = m.log.len();
        assert_eq!(&m.log[n - 2..], &[APP_BASE, APP_BASE + 256]);
    }
}

#[test]
fn name_decides_which_file_is_read() {
    let part = Part::Sam3u2e;
    let app = app_image(part, 1024, 4);
    let mut f = Fat::new(8192 * 2, 1);
    f.add_file(&NAME_BITSTREAM, &image(0, part, &app), Layout::Contig, false);
    let mut card = Card::new(with_mbr(&f.img, 0x0C));
    let mut m = Model::new(part);
    assert_eq!(recover(&mut m, part, &mut card), Err(RecoverError::NoFile));
    assert_eq!(m.ops, 0);
    let mut buf = [0u8; 512];
    assert_eq!(sd_recover(&mut card, &mut m, part, &mut buf, &NAME_BITSTREAM), Ok(1024));
}

fn refused(part: Part, file: Vec<u8>, want: RecoverError) {
    let mut m = Model::new(part);
    junk_app_area(&mut m);
    let before = m.mem.clone();
    let mut card = Card::new(sd_file(&file, Layout::Contig));
    assert_eq!(recover(&mut m, part, &mut card), Err(want));
    assert_eq!(m.ops, 0, "{want:?}: nothing may be erased");
    assert_eq!(m.mem, before);
}

#[test]
fn bad_files_are_refused_before_anything_is_erased() {
    let p2 = Part::Sam3u2e;
    let p4 = Part::Sam3u4e;
    let app = app_image(p2, 4096, 9);
    let good = image(0, p2, &app);

    // wrong target
    refused(p4, good.clone(), RecoverError::Header(Reject::BadTarget));
    // wrong destination (an old-style image at 0x80000)
    let mut h = header(0, p2, 0x0008_0000, &app);
    h.extend(&app);
    refused(p2, h, RecoverError::Header(Reject::BadDest));
    // layout
    let mut h = good.clone();
    put32(&mut h, 36, 0x4000);
    fix_header_crc(&mut h);
    refused(p2, h, RecoverError::Header(Reject::BadDest));
    // layout flipped without redoing the header CRC
    let mut h = good.clone();
    h[36] ^= 1;
    refused(p2, h, RecoverError::Header(Reject::BadHeader));
    // header CRC
    let mut h = good.clone();
    h[20] ^= 1;
    refused(p2, h, RecoverError::Header(Reject::BadHeader));
    // payload corrupted
    let mut h = good.clone();
    h[512 + 3000] ^= 1;
    refused(p2, h, RecoverError::Image(ImageError::CrcMismatch));
    // vectors into the resident, trailing CRC and header redone
    let mut bad = app.clone();
    put32(&mut bad, 4, 0x0008_0101);
    let c = bootcore::crc::crc32(&bad[..4092]);
    put32(&mut bad, 4092, c);
    refused(p2, image(0, p2, &bad), RecoverError::Image(ImageError::BadVector));
    // descriptor length differs
    let mut bad = app.clone();
    put32(&mut bad, 0x108, 2048);
    refused(p2, image(0, p2, &bad), RecoverError::Image(ImageError::BadDescriptor));
    // truncated file: loses the last sector / a few bytes
    let mut t = good.clone();
    t.truncate(t.len() - 512);
    refused(p2, t, RecoverError::Image(ImageError::Truncated));
    let mut t = good.clone();
    t.truncate(t.len() - 5);
    refused(p2, t, RecoverError::Image(ImageError::Truncated));
    // trailing bytes that are not an image
    let mut t = good.clone();
    t.extend(vec![0u8; 100]);
    refused(p2, t, RecoverError::Image(ImageError::Truncated));
    let mut t = good.clone();
    t.extend(vec![0u8; 512]);
    refused(p2, t, RecoverError::Header(Reject::BadHeader));
    // two apps
    let mut t = good.clone();
    t.extend(&good);
    refused(p2, t, RecoverError::Header(Reject::BadHeader));
    // a bitstream alone on a 4E: no app to boot afterwards
    refused(p4, image(1, p4, &bitstream(4096, 1)), RecoverError::NoApp);
    // a bitstream on a 2E
    let mut b = header(1, p2, PLANE1_BASE, &bitstream(4096, 1));
    b.extend(bitstream(4096, 1));
    refused(p2, b, RecoverError::Header(Reject::BadDest));
    // a good app followed by a corrupt bitstream: the app is not written either
    let mut t = image(0, p4, &app_image(p4, 4096, 1));
    let mut bsi = image(1, p4, &bitstream(4096, 2));
    bsi[512 + 77] ^= 8;
    t.extend(bsi);
    refused(p4, t, RecoverError::Image(ImageError::CrcMismatch));
}

#[test]
fn card_problems() {
    let part = Part::Sam3u2e;
    let mut m = Model::new(part);
    // blank card
    let mut card = Card::new(vec![0; 512 * 64]);
    assert_eq!(recover(&mut m, part, &mut card), Err(RecoverError::Fat(FatError::NoSignature)));
    // no file
    let mut f = Fat::new(8192, 1);
    f.add_file(b"OTHER   TXT", &lcg(1, 100), Layout::Contig, false);
    let mut card = Card::new(with_mbr(&f.img, 0x0C));
    assert_eq!(recover(&mut m, part, &mut card), Err(RecoverError::NoFile));
    // empty file
    let mut f = Fat::new(8192, 1);
    f.push_entry(&Fat::dirent(&NAME_APP, 0x20, 0, 0));
    let mut card = Card::new(with_mbr(&f.img, 0x0C));
    assert_eq!(recover(&mut m, part, &mut card), Err(RecoverError::Fat(FatError::Empty)));
    assert_eq!(m.ops, 0);
}

#[test]
fn locked_target_regions_are_refused_without_erasing() {
    let part = Part::Sam3u2e;
    let app = app_image(part, 0x1E000, 11);
    for region in 1..16 {
        let mut m = Model::new(part);
        junk_app_area(&mut m);
        m.lock[0] |= 1 << region;
        let before = m.mem.clone();
        let mut card = Card::new(sd_file(&image(0, part, &app), Layout::Contig));
        assert_eq!(recover(&mut m, part, &mut card), Err(RecoverError::Locked), "region {region}");
        assert_eq!(m.ops, 0);
        assert_eq!(m.mem, before);
    }
    // a lock outside the image's regions does not matter
    let small = app_image(part, 8192, 11); // regions 1 only (0x82000..0x84000)
    let mut m = Model::new(part);
    m.lock[0] |= 1 << 9;
    let mut card = Card::new(sd_file(&image(0, part, &small), Layout::Contig));
    assert_eq!(recover(&mut m, part, &mut card), Ok(8192));
    // plane 1 lock on a 4E with a bitstream
    let p4 = Part::Sam3u4e;
    let mut file = image(0, p4, &app_image(p4, 4096, 1));
    file.extend(image(1, p4, &bitstream(4096, 1)));
    let mut m = Model::new(p4);
    m.lock[1] = 1;
    let mut card = Card::new(sd_file(&file, Layout::Contig));
    assert_eq!(recover(&mut m, p4, &mut card), Err(RecoverError::Locked));
    assert_eq!(m.ops, 0);
    // the resident region stays locked and a lock in it does not stop an update
    let mut m = Model::new(part);
    assert_eq!(m.lock[0] & 1, 1);
    let mut card = Card::new(sd_file(&image(0, part, &small), Layout::Contig));
    assert!(recover(&mut m, part, &mut card).is_ok());
    assert_eq!(m.lock[0] & 1, 1);
}

/// A lock that appears after the check (not possible on the board, but the
/// FLOCKE path must report it): the model locks region 2 after the first op.
#[test]
fn flocke_midway_is_reported_and_leaves_the_app_invalid() {
    let part = Part::Sam3u2e;
    let app = app_image(part, 0x4000, 11);
    let mut m = Model::new(part);
    m.poke(APP_BASE, &app_image(part, 0x2000, 3));
    // The model has no hook for "lock appears later", so emulate it: wrap it.
    struct LateLock(Model, usize);
    impl bootcore::flash::Flash for LateLock {
        fn read(&mut self, a: u32, o: &mut [u8]) {
            self.0.read(a, o)
        }
        fn ewp_start(&mut self, p: bootcore::fcr::Plane, pg: u32, d: &[u8; 256]) -> Result<(), FlashError> {
            if self.0.ops == self.1 {
                self.0.lock[0] |= 1 << 2;
            }
            self.0.ewp_start(p, pg, d)
        }
        fn ewp_poll(&mut self) -> bootcore::flash::Poll {
            self.0.ewp_poll()
        }
        fn lock_bits(&mut self, p: bootcore::fcr::Plane) -> u32 {
            self.0.lock_bits(p)
        }
    }
    let mut lm = LateLock(m, 6);
    let mut card = Card::new(sd_file(&image(0, part, &app), Layout::Contig));
    let mut buf = [0u8; 512];
    let r = sd_recover(&mut card, &mut lm, part, &mut buf, &NAME_APP);
    assert_eq!(r, Err(RecoverError::Locked));
    assert_eq!(lm.0.flocke, 1);
    assert!(valid(&lm.0, part).is_none());
}

#[test]
fn flash_cell_failure_is_caught_by_the_read_back() {
    let part = Part::Sam3u2e;
    let app = app_image(part, 8192, 11);
    // one stuck bit in the body, in the vector page, in the descriptor page
    for off in [0x1000u32 + 17, 0, 4, 0x100, 0x11F, 0x3F0] {
        let mut m = Model::new(part);
        m.stuck = Some(APP_BASE + off);
        let mut card = Card::new(sd_file(&image(0, part, &app), Layout::Contig));
        let r = recover(&mut m, part, &mut card);
        assert_eq!(r, Err(RecoverError::Verify), "offset {off:#x}");
        assert!(valid(&m, part).is_none(), "offset {off:#x}: a bad write must not leave a valid app");
    }
}

#[test]
fn sd_read_failures_and_glitches_never_leave_a_wrong_app() {
    let part = Part::Sam3u2e;
    let old = app_image(part, 4096, 1);
    let app = app_image(part, 8192, 12);
    let file = image(0, part, &app);
    let img = sd_file(&file, Layout::Stride3);
    // reads of a clean run
    let mut m = Model::new(part);
    let mut card = Card::new(img.clone());
    recover(&mut m, part, &mut card).unwrap();
    let total = card.reads;
    assert!(total > 30);
    for n in 0..total {
        for mode in 0..3 {
            let mut m = Model::new(part);
            m.poke(APP_BASE, &old);
            let mut card = Card::new(img.clone());
            match mode {
                0 => card.fail_at = Some(n),
                1 => card.glitch = Some((n, 7)),
                _ => card.glitch = Some((n, 300)),
            }
            let r = recover(&mut m, part, &mut card);
            let v = valid(&m, part);
            match r {
                Ok(len) => {
                    assert_eq!(v, Some(len));
                    assert_eq!(m.peek(APP_BASE, 8192), &app[..], "read {n} mode {mode}: Ok but wrong flash");
                }
                Err(_) => {
                    // either untouched (old app still valid) or invalid; never a different app
                    match v {
                        None => {}
                        Some(l) => assert_eq!((l, m.peek(APP_BASE, l as usize)), (4096, &old[..]), "read {n} mode {mode}"),
                    }
                }
            }
            assert_eq!(m.violations, 0);
        }
    }
}
