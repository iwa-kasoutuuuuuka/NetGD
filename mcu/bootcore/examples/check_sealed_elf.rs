//! Does a resident-layout application ELF, sealed by `tools/fwimage seal`,
//! pass the resident's own app checks? (stage S2 proof)
//!
//! ```text
//! python tools/fwimage/fwimage.py seal <elf> -o sealed.elf
//! cargo run --example check_sealed_elf -- sealed.elf [2e|4e]     (in mcu/bootcore)
//! ```
//!
//! Loads the ELF's flash segments at `APP_BASE` and runs the same functions
//! `verify_flash_app` runs: `parse_descriptor` (sealed only), `check_vectors`,
//! `check_trailer`. Exits non-zero with the failing check.

use bootcore::app::*;
use bootcore::part::Part;

fn rd(b: &[u8], o: usize, n: usize) -> u32 {
    (0..n).rev().fold(0u32, |v, i| v << 8 | b[o + i] as u32)
}

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().expect("usage: check_sealed_elf <sealed.elf> [2e|4e]");
    let part = if a.next().as_deref() == Some("2e") { Part::Sam3u2e } else { Part::Sam3u4e };
    let elf = std::fs::read(&path).unwrap();
    assert_eq!(&elf[..4], b"\x7fELF");
    let (phoff, phentsize, phnum) = (rd(&elf, 28, 4) as usize, rd(&elf, 42, 2) as usize, rd(&elf, 44, 2) as usize);

    // plane-0 bytes from APP_BASE, gaps 0xFF
    let mut img = vec![0xFFu8; APP_MAX_LEN as usize];
    let mut top = 0usize;
    for i in 0..phnum {
        let p = phoff + i * phentsize;
        let (ty, off, paddr, sz) = (rd(&elf, p, 4), rd(&elf, p + 4, 4), rd(&elf, p + 12, 4), rd(&elf, p + 16, 4));
        if ty != 1 || sz == 0 || !(0x0008_0000..0x000A_0000).contains(&paddr) {
            continue;
        }
        assert!(paddr >= APP_BASE, "segment at {paddr:#x} below APP_BASE");
        let o = (paddr - APP_BASE) as usize;
        assert!(o + sz as usize <= img.len(), "segment past the end of the app region");
        img[o..o + sz as usize].copy_from_slice(&elf[off as usize..(off + sz) as usize]);
        top = top.max(o + sz as usize);
    }

    let mut d = [0u8; DESC_LEN];
    d.copy_from_slice(&img[DESC_OFF as usize..DESC_OFF as usize + DESC_LEN]);
    let desc = parse_descriptor(&d, false).unwrap_or_else(|e| panic!("descriptor: {e:?}"));
    assert_eq!(desc.image_len as usize, top, "descriptor length != end of loaded image");
    let len = desc.image_len as usize;
    check_vectors(&vectors_of(&img[..16]), APP_BASE, len as u32, part).unwrap_or_else(|e| panic!("vectors: {e:?}"));
    assert!(check_trailer(&img[..len]), "trailer CRC");
    // the check can fail: one flipped bit
    let mut bad = img[..len].to_vec();
    bad[len / 2] ^= 1;
    assert!(!check_trailer(&bad));
    println!(
        "OK: image {len} bytes ({len:#x}) at {APP_BASE:#x}..{:#x}; vectors SP={:#x} Reset={:#x}; bitstream addr={:#x} len={} raw={} kind={} version={:#x}",
        APP_BASE as usize + len, rd(&img, 0, 4), rd(&img, 4, 4), desc.bs_addr, desc.bs_len, desc.bs_raw_len, desc.bs_kind, desc.app_version
    );
}
