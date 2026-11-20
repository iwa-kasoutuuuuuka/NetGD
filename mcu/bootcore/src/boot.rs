//! The boot decision (design 2-3 and D5). Pure functions: the caller
//! samples the button and GPBR0 and checks the app lazily if it wants to
//! save the ~20-55 ms of CRC when recovery is requested anyway.

/// What the application writes into GPBR0 before resetting to ask for an SD
/// update. The design leaves the value open ("合言葉"); `"NGRC"` read as a
/// little-endian word is proposed, and it is not 0 or all-ones, which are
/// what a cold GPBR holds.
pub const GPBR_RECOVER_MAGIC: u32 = u32::from_le_bytes(*b"NGRC");

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// Set VTOR, load MSP, branch to the application's reset vector.
    RunApp,
    /// Initialise the SD card and look for `NETGD.OGF`.
    ///
    /// `clear_request`: the request came from GPBR0 (the magic is there), so
    /// the caller must write 0 to GPBR0 **before** it starts recovering.
    /// GPBR survives a reset; without this a recovery that succeeds (or
    /// fails) would be requested again by every following reset.
    Recover { clear_request: bool },
}

/// D5: an SD update happens only when the application is invalid, or the
/// front button is held at power-on, or the application asked for it through
/// GPBR0. A file on the card alone never triggers anything.
///
/// Protocol of the caller, from reset to hand-off (design 4-1, Y5):
///
/// 1. clocks and FWS = 4, then [`lock_resident`] (GLB, SLB if needed, GLB);
/// 2. read the button **once**; only if it reads pressed, read it twice more
///    20 ms apart and pass `[true, b2, b3]` to [`button_held`]; if the first
///    sample is not pressed, pass `[false, false, false]` without reading
///    again. (Reading all three always would delay every boot by 40 ms, H13.)
/// 3. [`decide`].
///
/// On `Recover`, clear GPBR0 if asked, run
/// `sd_recover`; if it returns `Ok`, jump to the application at once (do not
/// reset and `decide` again: the button may still be held); if it fails,
/// follow [`after_failed_recovery`]. A button that stays pressed costs one
/// card read per boot but no flash write: `sd_recover` leaves the flash alone
/// when the application in it is valid and is the file's (same length and
/// trailer CRC, and the bitstream matches).
pub fn decide(app_valid: bool, button_held: bool, gpbr0: u32) -> Action {
    let requested = gpbr0 == GPBR_RECOVER_MAGIC;
    if button_held || requested || !app_valid {
        Action::Recover { clear_request: requested }
    } else {
        Action::RunApp
    }
}

/// The button is "held" only if three samples (20 ms apart on the board) all
/// read pressed. The caller reads the second and third sample only when the
/// first one read pressed; when it did not it passes `false` for the rest
/// (see [`decide`]), so an unpressed button costs one read and no delay.
pub fn button_held(samples: [bool; 3]) -> bool {
    samples[0] && samples[1] && samples[2]
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fallback {
    /// Recovery was only requested (button, GPBR) and the application is
    /// fine: report the failure on the LED and start the application after a
    /// few seconds.
    StartApp,
    /// No valid application: keep polling the card (a card inserted later
    /// still works).
    RetryRecovery,
}

/// What to do after recovery failed (no card, no file, bad file...).
pub fn after_failed_recovery(app_valid: bool) -> Fallback {
    if app_valid {
        Fallback::StartApp
    } else {
        Fallback::RetryRecovery
    }
}

// ------------------------------------------------------------------ GPBR1: the boot report

/// Upper half of the GPBR1 report word, `0x4E 0xB1`. A cold GPBR is
/// undefined (datasheet table 19-1), so an application that reads a word
/// without this mark treats it as "no report" (a direct start from a probe,
/// or an older resident). Layout: `mark << 16 | flags << 8 | result`.
pub const GPBR1_MARK: u32 = 0x4EB1;
/// Bit position of [`GPBR1_MARK`] in the word.
pub const REPORT_MARK_SHIFT: u32 = 16;
/// The word with the mark and nothing else.
pub const REPORT_MARK_WORD: u32 = GPBR1_MARK << REPORT_MARK_SHIFT;
/// `result` is bits 7..0.
pub const REPORT_RESULT_MASK: u32 = 0xFF;

// Flags, bits 15..8 of the report word (design 3-2). Reserved bits (14, 15)
// are written as 0 and ignored when read.
/// Recovery was entered because the button was held.
pub const REPORT_BTN: u32 = 1 << 8;
/// Recovery was entered because GPBR0 held [`GPBR_RECOVER_MAGIC`]. The
/// application must not ask for recovery again in this run (rule X6,
/// [`request_allowed`]).
pub const REPORT_REQ: u32 = 1 << 9;
/// This boot wrote flash.
pub const REPORT_WROTE: u32 = 1 << 10;
/// The last GLB read lock bit 0 (the resident's own region) as 1.
pub const REPORT_LOCKED: u32 = 1 << 11;
/// This boot issued SLB.
pub const REPORT_SLB: u32 = 1 << 12;
/// An unsealed application (length 0 in its descriptor) was accepted; only
/// a development resident does that.
pub const REPORT_UNSEALED: u32 = 1 << 13;
/// All flag bits that are defined.
pub const REPORT_FLAGS_MASK: u32 =
    REPORT_BTN | REPORT_REQ | REPORT_WROTE | REPORT_LOCKED | REPORT_SLB | REPORT_UNSEALED;

// `result` values (design 3-2 and 6-2). 2..=9 are also the LED blink counts.
/// Normal start: recovery was not attempted.
pub const RESULT_NONE: u8 = 0;
/// Recovery ran to the end: written, or already the same image.
pub const RESULT_RECOVERED: u8 = 1;
/// No card (decided by the caller: card initialisation failed).
pub const RESULT_NO_CARD: u8 = 2;
/// No `NETGD.OGF` (or the card has no FAT32 volume we can read).
pub const RESULT_NO_FILE: u8 = 3;
/// A read error, or the file changed or its cluster chain is damaged.
pub const RESULT_READ: u8 = 4;
/// Header, target or application layout refused.
pub const RESULT_HEADER: u8 = 5;
/// The application region is locked.
pub const RESULT_LOCKED: u8 = 6;
/// A flash command failed.
pub const RESULT_WRITE: u8 = 7;
/// CRC mismatch or truncated payload.
pub const RESULT_CRC: u8 = 8;
/// Verification after writing failed.
pub const RESULT_VERIFY: u8 = 9;

/// What made the resident enter recovery.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Trigger {
    pub button: bool,
    pub request: bool,
}

impl Trigger {
    /// Of a [`decide`] verdict and the debounced button state. An invalid
    /// application alone is not a reported trigger.
    pub fn of(a: Action, button: bool) -> Trigger {
        match a {
            Action::RunApp => Trigger { button: false, request: false },
            Action::Recover { clear_request } => Trigger { button, request: clear_request },
        }
    }
}

/// The report word the resident stores in GPBR1 just before it jumps to the
/// application. Written on every boot, so no value of the previous boot
/// survives. `result` is one of the `RESULT_*` values.
#[inline]
pub fn report(t: Trigger, wrote: bool, locked: bool, slb: bool, unsealed: bool, result: u8) -> u32 {
    let mut w = REPORT_MARK_WORD | result as u32;
    if t.button {
        w |= REPORT_BTN;
    }
    if t.request {
        w |= REPORT_REQ;
    }
    if wrote {
        w |= REPORT_WROTE;
    }
    if locked {
        w |= REPORT_LOCKED;
    }
    if slb {
        w |= REPORT_SLB;
    }
    if unsealed {
        w |= REPORT_UNSEALED;
    }
    w
}

/// A decoded GPBR1 word (the application's side).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Report {
    pub trigger: Trigger,
    pub wrote: bool,
    pub locked: bool,
    pub slb: bool,
    pub unsealed: bool,
    pub result: u8,
}

impl Report {
    /// `None` when the mark is not there (cold GPBR, no resident, older
    /// resident). Reserved flag bits are ignored.
    pub fn parse(w: u32) -> Option<Report> {
        if w >> REPORT_MARK_SHIFT != GPBR1_MARK {
            return None;
        }
        Some(Report {
            trigger: Trigger { button: w & REPORT_BTN != 0, request: w & REPORT_REQ != 0 },
            wrote: w & REPORT_WROTE != 0,
            locked: w & REPORT_LOCKED != 0,
            slb: w & REPORT_SLB != 0,
            unsealed: w & REPORT_UNSEALED != 0,
            result: w as u8,
        })
    }

    /// The word again (reserved bits 0).
    pub fn word(&self) -> u32 {
        report(self.trigger, self.wrote, self.locked, self.slb, self.unsealed, self.result)
    }
}

/// Rule X6, for the application: may it write GPBR0 and reset to ask for a
/// recovery? Not when this very run began as the recovery it asked for
/// (`REQ`): otherwise an application whose problem a recovery cannot fix
/// (an FPGA that does not start) would reset forever. With no report (no
/// resident, a probe start) the answer is yes.
#[inline]
pub fn request_allowed(r: Option<Report>) -> bool {
    match r {
        Some(r) => !r.trigger.request,
        None => true,
    }
}

// ------------------------------------------------------------------ locking the resident region

/// What the lock procedure does after reading the lock bits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LockStep {
    /// Nothing to do: the region is locked already, or this is a development
    /// resident (rule D4: it never locks).
    Skip,
    /// Issue SLB for region 0.
    Slb,
}

/// Plane-0 lock bit 0 is the resident's own 8 KB.
pub const LOCK_RESIDENT_BIT: u32 = 1;

/// Step 2 of design 4-1. SLB is issued **only** when region 0 reads
/// unlocked: the word of SLB 0 is one bit from CLB 0, which would unlock it,
/// so with this rule the flip can matter at most once in the life of a
/// board, and the GLB that follows catches it.
#[inline]
pub fn lock_step(glb_before: u32, dev: bool) -> LockStep {
    if dev || glb_before & LOCK_RESIDENT_BIT != 0 {
        LockStep::Skip
    } else {
        LockStep::Slb
    }
}

/// The `LOCKED` flag: region 0 reads locked in the last GLB. A GLB that
/// failed reads as all ones ([`crate::flash::Flash::lock_bits`]), so it
/// reads locked.
#[inline]
pub fn lock_report(glb_after: u32) -> bool {
    glb_after & LOCK_RESIDENT_BIT != 0
}

/// Outcome of [`lock_resident`], for the report.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LockOutcome {
    pub locked: bool,
    pub slb: bool,
}

/// Design 4-1, steps 1 to 3: GLB; if [`lock_step`] says so, FWS = 6, SLB,
/// FWS = 4, GLB again. **Never fails**: a region that cannot be locked still
/// boots (a stuck lock bit must not make a brick); the outcome tells.
///
/// Call it with FWS = [`crate::part::FWS_RUN`].
pub fn lock_resident<F: crate::flash::Flash>(f: &mut F, part: crate::part::Part, dev: bool) -> LockOutcome {
    use crate::fcr::{FcrCmd, Plane};
    use crate::part::{FWS_RUN, FWS_WRITE};
    let before = f.lock_bits(Plane::P0);
    if lock_step(before, dev) == LockStep::Skip {
        return LockOutcome { locked: lock_report(before), slb: false };
    }
    if let Some(cmd) = FcrCmd::slb(part, true) {
        f.set_fws(FWS_WRITE);
        // Whatever happens, the next GLB tells; there is nothing to do with
        // an error here.
        let _ = f.slb(cmd);
        f.set_fws(FWS_RUN);
    }
    LockOutcome { locked: lock_report(f.lock_bits(Plane::P0)), slb: true }
}
