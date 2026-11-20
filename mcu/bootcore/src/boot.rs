//! The boot decision (design 2-3 and D5). Pure functions: the caller
//! samples the button and GPBR0 and checks the app lazily if it wants to
//! save the ~20-55 ms of CRC when recovery is requested anyway.

/// What the application writes into GPBR0 before resetting to ask for an SD
/// update. The design leaves the value open ("合言葉"); `"NGRC"` read as a
/// little-endian word is proposed, and it is not 0 or all-ones, which are
/// what a cold GPBR holds.
pub const GPBR_RECOVER_MAGIC: u32 = u32::from_le_bytes(*b"NGRC");

/// What the **resident** writes into GPBR0 in place of the request, before it
/// touches the card: "a requested recovery is in progress" (contract 3-1,
/// finding B6 of R89). It is the complement of the request word (all 32 bits
/// differ; not 0, not all-ones). The resident writes 0 over it at hand-off.
/// A boot that finds it knows that a requested recovery was cut short by a
/// reset (a hang ended by the watchdog, an EWP timeout, the reset button) and
/// does **not** start another one on that ground; it reports `REQ`, so the
/// application does not ask again either. Without it the request would be
/// forgotten by the reset, and "request, hang, 16 s, application, request"
/// would go round for ever.
pub const GPBR_RECOVER_BUSY: u32 = !GPBR_RECOVER_MAGIC;

/// What GPBR0 says at reset.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Request {
    /// Anything but the two words below: nothing asked. GPBR0 is left alone.
    None,
    /// [`GPBR_RECOVER_MAGIC`]: the application asks for a recovery.
    New,
    /// [`GPBR_RECOVER_BUSY`]: the previous boot was recovering on request and
    /// was reset before it handed off.
    Interrupted,
}

impl Request {
    #[inline]
    pub fn of(gpbr0: u32) -> Request {
        if gpbr0 == GPBR_RECOVER_MAGIC {
            Request::New
        } else if gpbr0 == GPBR_RECOVER_BUSY {
            Request::Interrupted
        } else {
            Request::None
        }
    }

    /// The word to store in GPBR0 **before recovery starts** (before the card
    /// is touched), `None` = do not write. A new request becomes "in
    /// progress"; an interrupted one stays as it is while the resident
    /// recovers (the application is invalid or the button is held), so that
    /// one more reset still remembers it.
    #[inline]
    pub fn gpbr0_before_recovery(self) -> Option<u32> {
        match self {
            Request::New => Some(GPBR_RECOVER_BUSY),
            _ => None,
        }
    }

    /// The word to store in GPBR0 **at hand-off**, `None` = do not write
    /// (GPBR0 is not the resident's unless it holds one of its two words).
    #[inline]
    pub fn gpbr0_at_handoff(self) -> Option<u32> {
        match self {
            Request::None => None,
            _ => Some(0),
        }
    }

    /// Does the report carry `REQ`? Yes for a new request and for an
    /// interrupted one: in both this run goes back to the application's
    /// request, and it must not ask again (rule X6).
    #[inline]
    pub fn reported(self) -> bool {
        self != Request::None
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// Set VTOR, load MSP, branch to the application's reset vector.
    ///
    /// `request` is `None` or `Interrupted`, never `New`. With `Interrupted`
    /// the report carries `REQ` and [`RESULT_INTERRUPTED`], and the card is
    /// not touched.
    RunApp { request: Request },
    /// Initialise the SD card and look for `NETGD.OGF`.
    ///
    /// Before the card is touched the caller stores
    /// [`Request::gpbr0_before_recovery`] in GPBR0. GPBR survives a reset;
    /// a request left standing would be served again by every following
    /// reset, and one simply cleared would be forgotten by a reset in the
    /// middle (see [`GPBR_RECOVER_BUSY`]).
    Recover { request: Request },
}

/// D5: an SD update happens only when the application is invalid, or the
/// front button is held at power-on, or the application asked for it through
/// GPBR0. A file on the card alone never triggers anything. A request that
/// was already being served when a reset came ([`Request::Interrupted`]) does
/// not trigger another recovery.
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
/// On `Recover`, store [`Request::gpbr0_before_recovery`] in GPBR0, run
/// `sd_recover`; if it returns `Ok`, jump to the application at once (do not
/// reset and `decide` again: the button may still be held); if it fails,
/// follow [`after_failed_recovery`]. A button that stays pressed costs one
/// card read per boot but no flash write: `sd_recover` leaves the flash alone
/// when the application in it is valid and is the file's (same length and
/// trailer CRC, and the bitstream matches).
///
/// **Hand-off** (the one `handoff()` of the resident, contract 2 and Y3), in
/// this order:
///
/// 1. **rewind the watchdog first** (`WDT_CR = 0xA5000001`). Later SAM
///    datasheets forbid a `WDT_MR` write within three slow-clock periods
///    (about 92 us) of a `WDT_CR` restart; the SAM3U datasheet does not say
///    it, but the application's first act may be its one `WDT_MR` write, so
///    the rewind is put as far before the jump as it can be (W2, hardware
///    check H9);
/// 2. put pins, HSMCI, SysTick back, FWS = 4 on every plane;
/// 3. GPBR0 = [`Request::gpbr0_at_handoff`], GPBR1 = [`report`];
/// 4. make sure at least [`WDT_RESTART_TO_JUMP_US`] have passed since step 1
///    (wait if steps 2 and 3 were faster), then VTOR, MSP, jump.
pub fn decide(app_valid: bool, button_held: bool, gpbr0: u32) -> Action {
    let request = Request::of(gpbr0);
    if button_held || request == Request::New || !app_valid {
        Action::Recover { request }
    } else {
        Action::RunApp { request }
    }
}

/// Least time between the watchdog rewind that opens the hand-off and the
/// jump to the application, in microseconds (three slow-clock periods are
/// 92 us; see [`decide`]).
pub const WDT_RESTART_TO_JUMP_US: u32 = 100;

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
    /// No valid application, and this boot has already made
    /// [`MAX_WRITING_ATTEMPTS`] attempts that wrote flash and failed: stop
    /// trying. Keep showing the last failure and keep rewinding the watchdog;
    /// a reset or a power cycle starts a new count.
    GiveUp,
}

/// How many recovery attempts **that wrote flash** one boot may make
/// (finding B8 of R89). A card that passes validation and then fails while
/// the body is written costs up to about 480 page writes per attempt; without
/// a bound the resident would rewrite them every time it polls the card, for
/// as long as the board is powered. Attempts that wrote nothing (no card, no
/// file, a file that is refused) are free and are not counted.
pub const MAX_WRITING_ATTEMPTS: u8 = 3;

/// What to do after recovery failed (no card, no file, bad file...).
/// `writing_attempts`: how many attempts of this boot wrote flash, the one
/// that just failed included (the caller keeps the count in RAM and adds
/// [`crate::update::Attempt::wrote`]).
pub fn after_failed_recovery(app_valid: bool, writing_attempts: u8) -> Fallback {
    if app_valid {
        Fallback::StartApp
    } else if writing_attempts >= MAX_WRITING_ATTEMPTS {
        Fallback::GiveUp
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

// Flags, bits 15..8 of the report word (design 3-2). The reserved bit (15)
// is written as 0 and ignored when read.
/// Recovery was entered because the button was held.
pub const REPORT_BTN: u32 = 1 << 8;
/// Recovery was entered because GPBR0 held [`GPBR_RECOVER_MAGIC`]. The
/// application must not ask for recovery again in this run (rule X6,
/// [`request_allowed`]). Also set when GPBR0 held [`GPBR_RECOVER_BUSY`]: the
/// recovery the application asked for was cut short by a reset.
pub const REPORT_REQ: u32 = 1 << 9;
/// This boot wrote flash.
pub const REPORT_WROTE: u32 = 1 << 10;
/// The last GLB read lock bit 0 (the resident's own region) as 1. Never set
/// together with [`REPORT_LOCK_UNKNOWN`].
pub const REPORT_LOCKED: u32 = 1 << 11;
/// This boot issued SLB.
pub const REPORT_SLB: u32 = 1 << 12;
/// An unsealed application (length 0 in its descriptor) was accepted; only
/// a development resident does that.
pub const REPORT_UNSEALED: u32 = 1 << 13;
/// The last GLB **failed** (FCMDE, or no FRDY in time): whether region 0 is
/// locked is not known. `LOCKED` is 0 then, but it does not mean "unlocked"
/// (finding B12 of R89: a failed GLB must not read as locked).
pub const REPORT_LOCK_UNKNOWN: u32 = 1 << 14;
/// All flag bits that are defined.
pub const REPORT_FLAGS_MASK: u32 =
    REPORT_BTN | REPORT_REQ | REPORT_WROTE | REPORT_LOCKED | REPORT_SLB | REPORT_UNSEALED | REPORT_LOCK_UNKNOWN;

// `result` values (design 3-2 and 6-2). 2..=9 are also the LED blink counts;
// 10 is never blinked (the resident starts the application at once).
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
/// GPBR0 held [`GPBR_RECOVER_BUSY`] and the application is valid: a requested
/// recovery was cut short by a reset, and this boot did not try again.
/// Always comes with `REQ`.
pub const RESULT_INTERRUPTED: u8 = 10;

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
            Action::RunApp { request } => Trigger { button: false, request: request.reported() },
            Action::Recover { request } => Trigger { button, request: request.reported() },
        }
    }
}

/// The report word the resident stores in GPBR1 just before it jumps to the
/// application. Written on every boot, so no value of the previous boot
/// survives. `result` is one of the `RESULT_*` values.
#[inline]
pub fn report(t: Trigger, wrote: bool, lock: LockOutcome, unsealed: bool, result: u8) -> u32 {
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
    if lock.unknown {
        w |= REPORT_LOCK_UNKNOWN;
    } else if lock.locked {
        w |= REPORT_LOCKED;
    }
    if lock.slb {
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
    /// Region 0 read locked. False when `lock_unknown`.
    pub locked: bool,
    pub slb: bool,
    pub unsealed: bool,
    /// The GLB failed: `locked` says nothing.
    pub lock_unknown: bool,
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
            locked: w & (REPORT_LOCKED | REPORT_LOCK_UNKNOWN) == REPORT_LOCKED,
            slb: w & REPORT_SLB != 0,
            unsealed: w & REPORT_UNSEALED != 0,
            lock_unknown: w & REPORT_LOCK_UNKNOWN != 0,
            result: w as u8,
        })
    }

    /// The word again (reserved bits 0).
    pub fn word(&self) -> u32 {
        let lock = LockOutcome { locked: self.locked, slb: self.slb, unknown: self.lock_unknown };
        report(self.trigger, self.wrote, lock, self.unsealed, self.result)
    }
}

/// Rule X6, for the application: may it write GPBR0 and reset to ask for a
/// recovery? Not when this very run began as the recovery it asked for, or
/// follows one that a reset cut short (`REQ`): otherwise an application whose problem a recovery cannot fix
/// (an FPGA that does not start) would reset forever. With no report (no
/// resident, a probe start, an older resident) the answer is no as well:
/// nothing would answer the request, and the reset would start the same
/// application again, for ever.
#[inline]
pub fn request_allowed(r: Option<Report>) -> bool {
    match r {
        Some(r) => !r.trigger.request,
        None => false,
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
/// board, and the GLB that follows catches it. `glb_before` is `None` when
/// the GLB failed: then nothing is known, and SLB is not issued (it would be
/// issued to a region that may well be locked, on a controller that has just
/// refused a harmless command).
#[inline]
pub fn lock_step(glb_before: Option<u32>, dev: bool) -> LockStep {
    match glb_before {
        Some(bits) if !dev && bits & LOCK_RESIDENT_BIT == 0 => LockStep::Slb,
        _ => LockStep::Skip,
    }
}

/// The `LOCKED` flag: region 0 reads locked in the last GLB. A GLB that
/// failed (`None`) is **not** locked; [`lock_unknown`] says so.
#[inline]
pub fn lock_report(glb_after: Option<u32>) -> bool {
    matches!(glb_after, Some(bits) if bits & LOCK_RESIDENT_BIT != 0)
}

/// The `LOCK_UNKNOWN` flag: the last GLB failed.
#[inline]
pub fn lock_unknown(glb_after: Option<u32>) -> bool {
    glb_after.is_none()
}

/// Outcome of [`lock_resident`], for the report.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct LockOutcome {
    /// Region 0 read locked in the last GLB (false when `unknown`).
    pub locked: bool,
    /// This boot issued SLB.
    pub slb: bool,
    /// The last GLB failed.
    pub unknown: bool,
}

impl LockOutcome {
    fn of(glb: Option<u32>, slb: bool) -> LockOutcome {
        LockOutcome { locked: lock_report(glb), slb, unknown: lock_unknown(glb) }
    }
}

/// Design 4-1, steps 1 to 3: GLB; if [`lock_step`] says so, FWS = 6, SLB,
/// FWS = 4, GLB again. **Never fails**: a region that cannot be locked, or
/// whose lock bit cannot be read, still boots (a stuck lock bit must not make
/// a brick); the outcome tells, and the next boot tries again.
///
/// Call it with FWS = [`crate::part::FWS_RUN`].
pub fn lock_resident<F: crate::flash::Flash>(f: &mut F, part: crate::part::Part, dev: bool) -> LockOutcome {
    use crate::fcr::{FcrCmd, Plane};
    use crate::part::{FWS_RUN, FWS_WRITE};
    let before = f.lock_bits(Plane::P0);
    if lock_step(before, dev) == LockStep::Skip {
        return LockOutcome::of(before, false);
    }
    if let Some(cmd) = FcrCmd::slb(part, true) {
        f.set_fws(FWS_WRITE);
        // Whatever happens, the next GLB tells; there is nothing to do with
        // an error here.
        let _ = f.slb(cmd);
        f.set_fws(FWS_RUN);
    }
    LockOutcome::of(f.lock_bits(Plane::P0), true)
}
