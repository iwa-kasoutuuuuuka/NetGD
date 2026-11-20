//! SD recovery: read `NETGD.OGF` from the card and rewrite the application
//! (design 2-3). The invariant that makes it power-cut safe:
//!
//! 1. The whole file is validated first (every header, every payload CRC,
//!    descriptor, vectors); nothing is erased unless it all passes and none
//!    of the target lock regions is locked.
//! 0. If the application in flash is already valid and is the one of the
//!    file (same length, same trailer CRC; and the bitstream matches),
//!    nothing is written: a button that stays pressed, or a repeated
//!    request, costs no flash endurance.
//! 2. The first two pages of the application (vector table, descriptor) are
//!    erased first, descriptor page first. From then on the application is
//!    invalid: no descriptor, so the next boot goes straight to recovery
//!    (checked: the descriptor must no longer parse, else the recovery stops).
//! 3. The bitstream (plane 1, SAM3U4E) and the rest of the application are
//!    written, every page read back and compared right after its EWP.
//! 4. Only then the first two pages are written, vector page first and the
//!    descriptor page last. The application becomes valid with the last EWP,
//!    and only if the image as a whole passes `verify_flash_app`.
//!
//! The first pass reads the file once, the second once, and a third short
//! pass re-reads up to the application's first chunk, because with one
//! 512-byte buffer the first chunk cannot be kept while the others are
//! written. (Deviation from the design's "two reads"; for a file that starts
//! with the application image the extra read is 3 sectors plus the FAT.)

use crate::app::{parse_descriptor, verify_flash_app, APP_BASE, DESC_LEN, DESC_OFF, LAYOUT};
use crate::crc;
use crate::fat::{BlockRead, DirEnt, FatError, FileReader, Volume};
use crate::fcr::{FcrCmd, Plane};
use crate::flash::{locate, Flash, FlashError, FLASH_BASE, PAGE, PLANE1_BASE, PLANE_LEN};
use crate::ogfw::{check_header, parse_header, Header, ImageError, ImageStream, Reject, KIND_APP};
use crate::part::Part;
use crate::rd32;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RecoverError {
    /// The card could not be read or is not a FAT32 volume we understand.
    Fat(FatError),
    /// No such file in the root directory.
    NoFile,
    /// A header was refused (wrong format, target, destination, length, layout).
    Header(Reject),
    /// A payload was refused (descriptor, vectors, CRC, truncated).
    Image(ImageError),
    /// The file differs between two reads.
    Changed,
    /// The file has no application image.
    NoApp,
    /// A target lock region is locked (FLOCKE would follow).
    Locked,
    Flash(FlashError),
    /// A page read back differently, or the final check failed.
    Verify,
}

#[derive(Clone, Copy)]
struct Plan {
    app: Header,
    bs: Option<Header>,
    /// Number of images in the file (1 or 2) and the position of the
    /// application among them: the later passes must see the same.
    count: u8,
    app_idx: u8,
    /// The trailing CRC of the application, as in the file.
    trailer: u32,
}

fn fe(e: FlashError) -> RecoverError {
    if e == FlashError::Locked {
        RecoverError::Locked
    } else {
        RecoverError::Flash(e)
    }
}

/// Read the header of the next image, `None` at the end of the file.
fn next_header<D: BlockRead>(
    r: &mut FileReader,
    vol: &Volume,
    dev: &mut D,
    buf: &mut [u8; 512],
    part: Part,
) -> Result<Option<Header>, RecoverError> {
    if r.remaining() == 0 {
        return Ok(None);
    }
    chunk(r, vol, dev, buf)?;
    let h = parse_header(buf).map_err(RecoverError::Header)?;
    check_header(&h, part).map_err(RecoverError::Header)?;
    Ok(Some(h))
}

/// One full 512-byte chunk; a file that ends inside one is truncated.
fn chunk<D: BlockRead>(r: &mut FileReader, vol: &Volume, dev: &mut D, buf: &mut [u8; 512]) -> Result<(), RecoverError> {
    match r.read_next(vol, dev, buf) {
        Ok(512) => Ok(()),
        Ok(_) => Err(RecoverError::Image(ImageError::Truncated)),
        Err(e) => Err(RecoverError::Fat(e)),
    }
}

fn validate<D: BlockRead>(vol: &Volume, dev: &mut D, ent: &DirEnt, part: Part, buf: &mut [u8; 512]) -> Result<Plan, RecoverError> {
    let mut r = FileReader::new(ent);
    let mut app: Option<Header> = None;
    let mut bs: Option<Header> = None;
    let mut count = 0u8;
    let mut app_idx = 0u8;
    let mut trailer = 0u32;
    while let Some(h) = next_header(&mut r, vol, dev, buf, part)? {
        let slot = if h.kind == KIND_APP { &mut app } else { &mut bs };
        if slot.is_some() {
            // two images of one kind
            return Err(RecoverError::Header(Reject::BadHeader));
        }
        let mut st = ImageStream::new(&h, part);
        let mut off = 0;
        while off < h.len {
            chunk(&mut r, vol, dev, buf)?;
            st.feed(buf).map_err(RecoverError::Image)?;
            off += 512;
            if h.kind == KIND_APP && off == h.len {
                trailer = rd32(buf, 508);
            }
        }
        st.finish().map_err(RecoverError::Image)?;
        if h.kind == KIND_APP {
            app_idx = count;
        }
        count = count.wrapping_add(1);
        *slot = Some(h);
    }
    match app {
        Some(app) => Ok(Plan { app, bs, count, app_idx, trailer }),
        None => Err(RecoverError::NoApp),
    }
}

/// GLB bit mask (bit n = lock region n of the plane `dest` is in) of the
/// regions under `[dest, dest + len)`. Same values as `lock_mask_at` in
/// `mcu/fwupdater/src/logic.rs` for every range inside a plane; the app at
/// `APP_BASE` covers bits 1.., never bit 0 (the resident region). A range
/// that is not inside one plane gives all ones ("everything locked"), so a
/// caller that checks `lock_bits & mask` refuses it; no arithmetic here can
/// underflow.
pub fn lock_mask_at(dest: u32, len: u32) -> u32 {
    if len == 0 {
        return 0;
    }
    let base = if dest >= PLANE1_BASE { PLANE1_BASE } else { FLASH_BASE };
    let off = match dest.checked_sub(base) {
        Some(o) => o,
        None => return u32::MAX,
    };
    let end = match off.checked_add(len) {
        Some(e) if e <= PLANE_LEN => e,
        _ => return u32::MAX,
    };
    let first = off / LAYOUT;
    let last = (end - 1) / LAYOUT;
    let hi = if last >= 31 { u32::MAX } else { (1u32 << (last + 1)) - 1 };
    hi & !((1u32 << first) - 1)
}

fn check_locks<F: Flash>(f: &mut F, plan: &Plan) -> Result<(), RecoverError> {
    if f.lock_bits(Plane::P0) & lock_mask_at(APP_BASE, plan.app.len) != 0 {
        return Err(RecoverError::Locked);
    }
    if let Some(bs) = &plan.bs {
        if f.lock_bits(Plane::P1) & lock_mask_at(PLANE1_BASE, bs.len) != 0 {
            return Err(RecoverError::Locked);
        }
    }
    Ok(())
}

/// EWP one page (no read-back).
fn ewp_page<F: Flash>(f: &mut F, part: Part, addr: u32, data: &[u8; PAGE]) -> Result<(), RecoverError> {
    let (plane, page) = locate(addr).ok_or(RecoverError::Flash(FlashError::Refused))?;
    let cmd = FcrCmd::ewp(part, plane, page).ok_or(RecoverError::Flash(FlashError::Refused))?;
    f.ewp(cmd, data).map_err(fe)
}

/// EWP one page, then read it back and compare.
fn write_page<F: Flash>(f: &mut F, part: Part, addr: u32, data: &[u8; PAGE]) -> Result<(), RecoverError> {
    ewp_page(f, part, addr, data)?;
    let mut t = [0u8; 32];
    let mut a = addr;
    for piece in data.chunks_exact(32) {
        f.read(a, &mut t);
        if t[..] != *piece {
            return Err(RecoverError::Verify);
        }
        a += 32;
    }
    Ok(())
}

/// The two pages of one 512-byte chunk, lower page first.
fn write_chunk<F: Flash>(f: &mut F, part: Part, addr: u32, buf: &[u8; 512]) -> Result<(), RecoverError> {
    let mut a = addr;
    for page in buf.as_chunks::<PAGE>().0 {
        write_page(f, part, a, page)?;
        a += PAGE as u32;
    }
    Ok(())
}

/// Invalidate the application: erase the descriptor page, then the vector
/// page, and check that the descriptor no longer parses (even with the
/// unsealed dev rule). A weak cell that does not read back as 0xFF does not
/// matter as long as the application is invalid; the pages are written and
/// read back later anyway.
fn erase_head<F: Flash>(f: &mut F, part: Part) -> Result<(), RecoverError> {
    let ff = [0xFFu8; PAGE];
    ewp_page(f, part, APP_BASE + PAGE as u32, &ff)?;
    ewp_page(f, part, APP_BASE, &ff)?;
    let mut d = [0u8; DESC_LEN];
    f.read(APP_BASE + DESC_OFF, &mut d);
    if parse_descriptor(&d, true).is_ok() {
        return Err(RecoverError::Verify);
    }
    Ok(())
}

/// Is the application in flash valid and the one of the file (same length,
/// same trailer CRC), with the bitstream of the file (if any) in plane 1?
fn already_installed<F: Flash>(f: &mut F, part: Part, plan: &Plan) -> bool {
    match verify_flash_app(f, part, false) {
        Ok(len) if len == plan.app.len => {}
        _ => return false,
    }
    let mut t = [0u8; 4];
    f.read(APP_BASE + plan.app.len - 4, &mut t);
    if u32::from_le_bytes(t) != plan.trailer {
        return false;
    }
    match &plan.bs {
        Some(bs) => crc_flash(f, PLANE1_BASE, bs.len) == bs.crc,
        None => true,
    }
}

fn crc_flash<F: Flash>(f: &mut F, addr: u32, len: u32) -> u32 {
    let mut c = crc::INIT;
    let mut b = [0u8; PAGE];
    let mut off = 0;
    while off < len {
        f.read(addr + off, &mut b);
        c = crc::update(c, &b);
        off += PAGE as u32;
    }
    crc::finish(c)
}

/// Write everything except the application's first chunk (pass 2).
fn write_body<D: BlockRead, F: Flash>(
    vol: &Volume,
    dev: &mut D,
    flash: &mut F,
    ent: &DirEnt,
    plan: &Plan,
    part: Part,
    buf: &mut [u8; 512],
) -> Result<(), RecoverError> {
    let mut r = FileReader::new(ent);
    let mut idx = 0u8;
    while let Some(h) = next_header(&mut r, vol, dev, buf, part)? {
        let app = h.kind == KIND_APP;
        let expect = if app { Some(plan.app) } else { plan.bs };
        // the same image, at the same position, and no more of them than in pass 1
        if expect != Some(h) || idx >= plan.count || app != (idx == plan.app_idx) {
            return Err(RecoverError::Changed);
        }
        idx += 1;
        let base = if app { APP_BASE } else { PLANE1_BASE };
        let mut off = 0;
        while off < h.len {
            chunk(&mut r, vol, dev, buf)?;
            if !(app && off == 0) {
                write_chunk(flash, part, base + off, buf)?;
            }
            off += 512;
        }
    }
    if idx != plan.count {
        return Err(RecoverError::Changed);
    }
    Ok(())
}

/// Write the application's first chunk (pass 3), vector page then descriptor page.
fn write_head<D: BlockRead, F: Flash>(
    vol: &Volume,
    dev: &mut D,
    flash: &mut F,
    ent: &DirEnt,
    plan: &Plan,
    part: Part,
    buf: &mut [u8; 512],
) -> Result<(), RecoverError> {
    let mut r = FileReader::new(ent);
    let mut idx = 0u8;
    while let Some(h) = next_header(&mut r, vol, dev, buf, part)? {
        if h.kind == KIND_APP {
            if h != plan.app || idx != plan.app_idx {
                return Err(RecoverError::Changed);
            }
            chunk(&mut r, vol, dev, buf)?;
            return write_chunk(flash, part, APP_BASE, buf);
        }
        idx = idx.wrapping_add(1);
        let mut off = 0;
        while off < h.len {
            chunk(&mut r, vol, dev, buf)?;
            off += 512;
        }
    }
    Err(RecoverError::Changed)
}

/// Recover the application from `name` on the card. Returns the length of
/// the application now in flash.
///
/// `buf` is the single 512-byte working buffer. `allow_unsealed` is not a
/// parameter: a recovery file must carry a sealed descriptor.
pub fn sd_recover<D: BlockRead, F: Flash>(
    dev: &mut D,
    flash: &mut F,
    part: Part,
    buf: &mut [u8; 512],
    name: &[u8; 11],
) -> Result<u32, RecoverError> {
    let vol = Volume::mount(dev, buf).map_err(RecoverError::Fat)?;
    let ent = match vol.find(dev, buf, name) {
        Ok(e) => e,
        Err(FatError::NotFound) => return Err(RecoverError::NoFile),
        Err(e) => return Err(RecoverError::Fat(e)),
    };

    // 1. nothing is touched unless the whole file is good and unlocked
    let plan = validate(&vol, dev, &ent, part, buf)?;
    // already there: write nothing
    if already_installed(flash, part, &plan) {
        return Ok(plan.app.len);
    }
    check_locks(flash, &plan)?;

    // 2. the application becomes invalid
    erase_head(flash, part)?;

    // 3. everything but its first chunk
    write_body(&vol, dev, flash, &ent, &plan, part, buf)?;
    if let Some(bs) = &plan.bs {
        if crc_flash(flash, PLANE1_BASE, bs.len) != bs.crc {
            return Err(RecoverError::Verify);
        }
    }

    // 4. the first chunk, last
    write_head(&vol, dev, flash, &ent, &plan, part, buf)?;
    match verify_flash_app(flash, part, false) {
        Ok(len) if len == plan.app.len => Ok(len),
        _ => {
            // Do not leave something that looks half valid.
            let _ = erase_head(flash, part);
            Err(RecoverError::Verify)
        }
    }
}
