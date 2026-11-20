//! A reference run of the resident, from reset to hand-off, made of bootcore's
//! pieces and the host models: lock procedure, button, application check,
//! `decide`, `sd_recover_ex`, the report in GPBR1, the hand-off. It is what
//! `mcu/resident` (S3) has to do in the same order; it is a test harness, not
//! the resident. Plus a stand-in for the application's side of GPBR (X6).
#![allow(dead_code)]

use super::*;
use bootcore::app::{verify_flash_app, APP_BASE};
use bootcore::boot::*;
use bootcore::fat::NAME_APP;
use bootcore::update::sd_recover_attempt;
use std::sync::Arc;

pub struct Board {
    pub part: Part,
    pub dev: bool,
    pub flash: Model,
    /// The card image; `None` = no card.
    pub card: Option<Arc<Vec<u8>>>,
    /// `Some(n)`: the card fails every read from the n-th on.
    pub card_fail_at: Option<usize>,
    pub gpbr0: u32,
    pub gpbr1: u32,
    pub button: bool,
    pub wdt: Wdt,
    /// Boots run so far.
    pub boots: u32,
    /// Resets requested by the application so far.
    pub app_resets: u32,
    /// Recovery attempts of this boot that wrote flash (the resident keeps
    /// it in RAM: a reset starts from 0).
    pub write_attempts: u8,
    /// Attempts made (card polled) over the life of the board.
    pub attempts: u32,
    /// Set while the resident waits for a card: what the next poll needs.
    waiting: Option<(Trigger, LockOutcome, Request)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Boot {
    /// The resident jumped to the application; the report it left in GPBR1.
    Jump(Report),
    /// No valid application and recovery failed: it keeps polling the card
    /// ([`Board::poll`]).
    Waiting(u8),
    /// No valid application, and this boot has used up its writing attempts
    /// (`MAX_WRITING_ATTEMPTS`): it shows the failure and does nothing more
    /// until a reset.
    Halted(u8),
    /// The flash model went dead (power cut, or a reset, inside a command).
    PowerLost,
}

impl Board {
    pub fn new(part: Part, dev: bool) -> Board {
        let wdt = Wdt::default();
        let mut flash = Model::new(part);
        flash.wdt = Some(wdt.clone());
        Board {
            part,
            dev,
            flash,
            card: None,
            card_fail_at: None,
            // cold GPBR: undefined; pick a pattern that is neither magic
            gpbr0: 0x5A5A_A5A5,
            gpbr1: 0xFFFF_FFFF,
            button: false,
            wdt,
            boots: 0,
            app_resets: 0,
            write_attempts: 0,
            attempts: 0,
            waiting: None,
        }
    }

    pub fn install_app(&mut self, image: &[u8]) {
        self.flash.poke(APP_BASE, image);
    }

    pub fn set_card(&mut self, name_file: &[u8]) {
        self.card = Some(Arc::new(card_with(&NAME_APP, name_file)));
    }

    /// Power returns (or reset): the controller is idle, FWS at its reset
    /// value. GPBR0 and GPBR1 keep their values (backup domain).
    pub fn reset(&mut self) {
        self.flash.power_up();
        self.waiting = None;
    }

    fn app_state(&mut self) -> (bool, bool) {
        let v = verify_flash_app(&mut self.flash, self.part, self.dev);
        (v.is_ok(), v == Ok(0))
    }

    /// The hand-off, in the order of `bootcore::boot::decide`'s docs.
    fn jump(&mut self, request: Request, word: u32) -> Boot {
        // 1. the watchdog rewind comes first (B7 of R89)
        self.wdt.feed();
        self.wdt.finish();
        // 2. FWS is 4 already: the vector table is fetched at the FWS the
        // resident leaves; a flash read at FWS 6 counts as a violation.
        let mut v = [0u8; 4];
        self.flash.read(APP_BASE, &mut v);
        // 3. GPBR0 (only if it held one of the resident's two words), GPBR1
        if let Some(w) = request.gpbr0_at_handoff() {
            self.gpbr0 = w;
        }
        self.gpbr1 = word;
        self.waiting = None;
        Boot::Jump(Report::parse(word).expect("the resident wrote a report"))
    }

    /// One boot, from reset to hand-off (or to waiting / power loss).
    pub fn boot(&mut self) -> Boot {
        self.boots += 1;
        self.write_attempts = 0;
        self.waiting = None;
        self.wdt.feed();
        // 1. clocks (not modelled), FWS 4, lock
        self.flash.set_fws(FWS_RUN);
        let lock = lock_resident(&mut self.flash, self.part, self.dev);
        if self.flash.dead {
            return Boot::PowerLost;
        }
        // 2. the button: one read; two more only when it read pressed
        let held = if self.button {
            button_held([true, true, true])
        } else {
            button_held([false, false, false])
        };
        // 3. the application, the decision
        let (valid, unsealed) = self.app_state();
        let action = decide(valid, held, self.gpbr0);
        let trig = Trigger::of(action, held);
        match action {
            Action::RunApp { request } => {
                let result = if request == Request::Interrupted { RESULT_INTERRUPTED } else { RESULT_NONE };
                self.jump(request, report(trig, false, lock, unsealed, result))
            }
            Action::Recover { request } => {
                // before the card is touched
                if let Some(w) = request.gpbr0_before_recovery() {
                    self.gpbr0 = w;
                }
                self.attempt(trig, lock, request)
            }
        }
    }

    /// The resident is waiting for a card (the last result was
    /// [`Boot::Waiting`]): it polls once more, in the same boot.
    pub fn poll(&mut self) -> Boot {
        let (trig, lock, request) = self.waiting.expect("the resident is not waiting");
        self.wdt.feed();
        self.attempt(trig, lock, request)
    }

    fn attempt(&mut self, trig: Trigger, lock: LockOutcome, request: Request) -> Boot {
        self.attempts += 1;
        let (r, wrote) = match self.card.clone() {
            None => (Err(RESULT_NO_CARD), false),
            Some(img) => {
                let mut card = Card::shared(img);
                card.fail_at = self.card_fail_at;
                card.wdt = Some(self.wdt.clone());
                let mut buf = [0u8; 512];
                let a = sd_recover_attempt(&mut card, &mut self.flash, self.part, &mut buf, &NAME_APP);
                (a.result.map_err(|e| e.result_code()), a.wrote)
            }
        };
        if self.flash.dead {
            self.waiting = None;
            return Boot::PowerLost;
        }
        if wrote {
            self.write_attempts += 1;
        }
        match r {
            Ok(rec) => self.jump(request, report(trig, rec.wrote, lock, false, RESULT_RECOVERED)),
            Err(code) => {
                let (valid, unsealed) = self.app_state();
                match after_failed_recovery(valid, self.write_attempts) {
                    Fallback::StartApp => self.jump(request, report(trig, false, lock, unsealed, code)),
                    Fallback::RetryRecovery => {
                        self.waiting = Some((trig, lock, request));
                        Boot::Waiting(code)
                    }
                    Fallback::GiveUp => {
                        self.waiting = None;
                        Boot::Halted(code)
                    }
                }
            }
        }
    }

    /// The application's side of X6: it wants a recovery (say its FPGA does
    /// not start). Returns true if it wrote GPBR0 and reset.
    pub fn app_asks_for_recovery(&mut self) -> bool {
        if request_allowed(Report::parse(self.gpbr1)) {
            self.gpbr0 = GPBR_RECOVER_MAGIC;
            self.app_resets += 1;
            true
        } else {
            false
        }
    }

    /// Boot, and let an application that always wants a recovery ask for it,
    /// until it stays. Returns the last boot's result and the number of boots.
    /// `limit` bounds a loop that must not happen.
    pub fn run_app_that_wants_recovery(&mut self, limit: u32) -> (Boot, u32) {
        let start = self.boots;
        loop {
            let b = self.boot();
            match b {
                Boot::Jump(_) => {
                    if !self.app_asks_for_recovery() {
                        return (b, self.boots - start);
                    }
                }
                _ => return (b, self.boots - start),
            }
            assert!(self.boots - start < limit, "reset loop");
        }
    }
}
