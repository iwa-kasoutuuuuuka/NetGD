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
/// Protocol of the caller: on `Recover`, clear GPBR0 if asked, run
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
/// read pressed.
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
