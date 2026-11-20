//! SD recovery: read `NETGD.OGF` from the card and rewrite the application
//! (design 2-3). The invariant that makes it power-cut safe:
//!
//! 1. The whole file is validated first (every header, every payload CRC,
//!    descriptor, vectors); nothing is erased unless it all passes and none
//!    of the target lock regions is locked. If the lock bits cannot be read
//!    (GLB failed), a valid application is not touched (this attempt ends
//!    with `LockUnknown`; the next one asks again), and an invalid one is
//!    rewritten anyway: there is nothing to lose, and a page that is locked
//!    answers FLOCKE before it is changed.
//! 0. If the application in flash is already valid and is the one of the
//!    file (same length, same trailer CRC; and the bitstream matches: CRC
//!    and last word), nothing is written: a button that stays pressed, or a
//!    repeated request, costs no flash endurance.
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
use crate::boot::{
    RESULT_CRC, RESULT_HEADER, RESULT_LOCKED, RESULT_NO_FILE, RESULT_READ, RESULT_VERIFY, RESULT_WRITE,
};
use crate::part::{Part, FWS_RUN, FWS_WRITE};
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
    /// The lock bits could not be read (GLB failed) and the application in
    /// flash is valid, so it was left alone. Not final: every attempt reads
    /// the lock bits again.
    LockUnknown,
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
    /// The last word of the bitstream image, as in the file (0 without one).
    /// For a stamped bitstream it is the CRC-32 of the rest (contract 4-3),
    /// and then the header's `payload_crc` is the same for every bitstream
    /// (the CRC residue 0x2144DF1C): only this word tells two of them apart.
    bs_trailer: u32,
}

impl RecoverError {
    /// The `result` of the boot report and the LED count for this failure
    /// (design 3-2, 6-2): 3 no file (also a card without a FAT32 volume we
    /// can read), 4 read error, damaged chain, or a file that changed between
    /// passes, 5 header, target, layout or descriptor refused, 6 application
    /// region locked, 7 flash command failed, 8 CRC mismatch or truncated
    /// payload, 9 verification after writing failed. "No card" (2) is decided
    /// before `sd_recover` runs, by the caller. Exhaustive on purpose: a new
    /// error must get a code.
    pub fn result_code(&self) -> u8 {
        match self {
            RecoverError::Fat(e) => match e {
                FatError::Read | FatError::BadChain | FatError::Loop | FatError::Truncated => RESULT_READ,
                FatError::NoSignature
                | FatError::Gpt
                | FatError::NotFat32
                | FatError::BadBpb
                | FatError::NotFound
                | FatError::Empty => RESULT_NO_FILE,
            },
            RecoverError::NoFile => RESULT_NO_FILE,
            RecoverError::Header(_) | RecoverError::NoApp => RESULT_HEADER,
            RecoverError::Image(e) => match e {
                ImageError::BadDescriptor | ImageError::Unsealed | ImageError::BadVector => RESULT_HEADER,
                ImageError::Truncated | ImageError::CrcMismatch | ImageError::BadTrailer => RESULT_CRC,
            },
            RecoverError::Changed => RESULT_READ,
            RecoverError::Locked | RecoverError::LockUnknown => RESULT_LOCKED,
            RecoverError::Flash(_) => RESULT_WRITE,
            RecoverError::Verify => RESULT_VERIFY,
        }
    }
}

/// A recovery that ran to the end.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Recovered {
    /// Length of the application now in flash.
    pub len: u32,
    /// Flash was written (false: the application in it was already the file's).
    pub wrote: bool,
}

/// One recovery attempt, however it ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Attempt {
    pub result: Result<Recovered, RecoverError>,
    /// The attempt got as far as issuing flash commands (the application's
    /// head was, or was about to be, erased). True for every success that
    /// wrote and for every failure after validation and the lock check. The
    /// resident counts these against
    /// [`crate::boot::MAX_WRITING_ATTEMPTS`].
    pub wrote: bool,
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
    let mut bs_trailer = 0u32;
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
            if off == h.len {
                if h.kind == KIND_APP {
                    trailer = rd32(buf, 508);
                } else {
                    bs_trailer = rd32(buf, 508);
                }
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
        Some(app) => Ok(Plan { app, bs, count, app_idx, trailer, bs_trailer }),
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

/// May the regions under `mask` be written, given the GLB result `bits`?
/// `None` = the GLB failed (finding B12 of R89). Then nothing is known: a
/// valid application is not put at risk (were a region in the middle of the
/// image locked, the head would be erased and the body could never be
/// written), an invalid one is rewritten regardless.
pub fn lock_verdict(bits: Option<u32>, mask: u32, app_valid: bool) -> Result<(), RecoverError> {
    match bits {
        Some(b) if b & mask != 0 => Err(RecoverError::Locked),
        Some(_) => Ok(()),
        None if app_valid => Err(RecoverError::LockUnknown),
        None => Ok(()),
    }
}

fn check_locks<F: Flash>(f: &mut F, plan: &Plan, app_valid: bool) -> Result<(), RecoverError> {
    lock_verdict(f.lock_bits(Plane::P0), lock_mask_at(APP_BASE, plan.app.len), app_valid)?;
    if let Some(bs) = &plan.bs {
        lock_verdict(f.lock_bits(Plane::P1), lock_mask_at(PLANE1_BASE, bs.len), app_valid)?;
    }
    Ok(())
}

/// EWP one page (no read-back).
fn ewp_page<F: Flash>(f: &mut F, part: Part, addr: u32, data: &[u8; PAGE]) -> Result<(), RecoverError> {
    let (plane, page) = locate(addr).ok_or(RecoverError::Flash(FlashError::Refused))?;
    let cmd = FcrCmd::ewp(part, plane, page).ok_or(RecoverError::Flash(FlashError::Refused))?;
    // FWS = 6 for the command only; nothing is read from flash in between.
    f.set_fws(FWS_WRITE);
    let r = f.ewp(cmd, data);
    f.set_fws(FWS_RUN);
    r.map_err(fe)
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

fn flash_word<F: Flash>(f: &mut F, addr: u32) -> u32 {
    let mut t = [0u8; 4];
    f.read(addr, &mut t);
    u32::from_le_bytes(t)
}

/// Is the bitstream of the file (if it has one) what plane 1 holds: the CRC
/// of the header, and the last word (see `Plan::bs_trailer`)?
fn bitstream_is<F: Flash>(f: &mut F, plan: &Plan) -> bool {
    match &plan.bs {
        Some(bs) => {
            crc_flash(f, PLANE1_BASE, bs.len) == bs.crc
                && flash_word(f, PLANE1_BASE + bs.len - 4) == plan.bs_trailer
        }
        None => true,
    }
}

/// Is the application in flash (valid, of length `installed`) the one of the
/// file (same length, same trailer CRC), with the bitstream of the file (if
/// any) in plane 1?
fn already_installed<F: Flash>(f: &mut F, installed: u32, plan: &Plan) -> bool {
    installed == plan.app.len
        && flash_word(f, APP_BASE + plan.app.len - 4) == plan.trailer
        && bitstream_is(f, plan)
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
    sd_recover_ex(dev, flash, part, buf, name).map(|r| r.len)
}

/// [`sd_recover`] that also says whether it wrote (the report's `WROTE`).
pub fn sd_recover_ex<D: BlockRead, F: Flash>(
    dev: &mut D,
    flash: &mut F,
    part: Part,
    buf: &mut [u8; 512],
    name: &[u8; 11],
) -> Result<Recovered, RecoverError> {
    sd_recover_attempt(dev, flash, part, buf, name).result
}

/// [`sd_recover_ex`] that says whether flash commands were issued also when
/// it failed: what the resident counts to bound its retries
/// ([`crate::boot::after_failed_recovery`]).
pub fn sd_recover_attempt<D: BlockRead, F: Flash>(
    dev: &mut D,
    flash: &mut F,
    part: Part,
    buf: &mut [u8; 512],
    name: &[u8; 11],
) -> Attempt {
    let mut wrote = false;
    let result = recover(dev, flash, part, buf, name, &mut wrote);
    Attempt { result, wrote }
}

fn recover<D: BlockRead, F: Flash>(
    dev: &mut D,
    flash: &mut F,
    part: Part,
    buf: &mut [u8; 512],
    name: &[u8; 11],
    wrote: &mut bool,
) -> Result<Recovered, RecoverError> {
    // Before anything is read from flash: FWS on every plane. On a 4E the
    // application sets EEFC1's FWS, not the resident's start-up, and after a
    // reset it is 0; plane 1 read at 96 MHz with FWS = 0 gives garbage, the
    // bitstream "differs", and the whole image would be rewritten at every
    // recovery (contract K7, finding B2 of R89).
    flash.set_fws(FWS_RUN);

    let vol = Volume::mount(dev, buf).map_err(RecoverError::Fat)?;
    let ent = match vol.find(dev, buf, name) {
        Ok(e) => e,
        Err(FatError::NotFound) => return Err(RecoverError::NoFile),
        Err(e) => return Err(RecoverError::Fat(e)),
    };

    // 1. nothing is touched unless the whole file is good and unlocked
    let plan = validate(&vol, dev, &ent, part, buf)?;
    // already there: write nothing
    let installed = verify_flash_app(flash, part, false);
    if let Ok(len) = installed {
        if already_installed(flash, len, &plan) {
            return Ok(Recovered { len: plan.app.len, wrote: false });
        }
    }
    check_locks(flash, &plan, installed.is_ok())?;

    // 2. the application becomes invalid
    *wrote = true;
    erase_head(flash, part)?;

    // 3. everything but its first chunk
    write_body(&vol, dev, flash, &ent, &plan, part, buf)?;
    if !bitstream_is(flash, &plan) {
        return Err(RecoverError::Verify);
    }

    // 4. the first chunk, last
    write_head(&vol, dev, flash, &ent, &plan, part, buf)?;
    match verify_flash_app(flash, part, false) {
        Ok(len) if len == plan.app.len => Ok(Recovered { len, wrote: true }),
        _ => {
            // Do not leave something that looks half valid.
            let _ = erase_head(flash, part);
            Err(RecoverError::Verify)
        }
    }
}
