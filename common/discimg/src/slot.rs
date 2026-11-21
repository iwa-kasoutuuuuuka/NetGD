//! Numbered image folders, the way the original GDEMU and its menus use them.
//!
//! The SD card's root holds folders called `01`, `02`, ... (two digits up to
//! 99, three up to 999, four from 1000). A menu selects one by number; the
//! front-panel button steps to the next. Pure functions only: the firmware
//! walks the card and calls in with what it found.

/// The slot number a folder name stands for, if it is one.
///
/// `name` is the folder's base name without extension: 2 to 4 ASCII digits
/// with a value of at least 1. Leading zeros are allowed (`001` is 1).
pub fn number_of(name: &[u8]) -> Option<u16> {
    if !(2..=4).contains(&name.len()) {
        return None;
    }
    let mut v: u16 = 0;
    for &b in name {
        if !b.is_ascii_digit() {
            return None;
        }
        v = v * 10 + (b - b'0') as u16;
    }
    if v >= 1 {
        Some(v)
    } else {
        None
    }
}

/// Whether the firmware may switch images on its own (the front button, or a
/// switch it was asked for between commands).
///
/// `media_present`: an image is mounted. `empty_slot`: nothing is mounted
/// because the selected folder had nothing to mount (stock accepts such a
/// switch and reports no medium, D12) -- the card is still the firmware's,
/// so the button must keep working or nothing short of the host brings a
/// disc back. With neither, the drive was ejected for the host (START/STOP,
/// a write session, or a disk write while empty), which may hold a CMD25 open
/// across its waits: hands off.
pub const fn swap_allowed(media_present: bool, empty_slot: bool) -> bool {
    media_present || empty_slot
}

/// Direction of a step between slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Next,
    Prev,
}

/// Picks the slot one step from `current` out of the numbers present on the
/// card, in a single pass over them.
///
/// Feed every slot number found with [`Nearest::offer`], in any order, then
/// read [`Nearest::result`]:
/// - [`Step::Next`]: the smallest number above `current`, or else (wrapping)
///   the smallest number of all.
/// - [`Step::Prev`]: the largest number below `current`, or else (wrapping)
///   the largest number of all.
///
/// Gaps in the numbering cost nothing: only numbers that exist are offered.
/// With no current slot, Next gives the smallest and Prev the largest. When
/// the current slot is the only one, the result is the current slot itself.
/// `None` only when nothing was offered.
#[derive(Clone, Copy, Debug)]
pub struct Nearest {
    current: Option<u16>,
    dir: Step,
    beyond: Option<u16>,
    wrap: Option<u16>,
    offered: u16,
}

impl Nearest {
    pub fn new(current: Option<u16>, dir: Step) -> Self {
        Nearest { current, dir, beyond: None, wrap: None, offered: 0 }
    }

    /// How many numbers have been offered (duplicates count).
    pub fn offered(&self) -> u16 {
        self.offered
    }

    pub fn offer(&mut self, n: u16) {
        self.offered = self.offered.saturating_add(1);
        match self.dir {
            Step::Next => {
                if self.wrap.map_or(true, |w| n < w) {
                    self.wrap = Some(n);
                }
                let above = self.current.map_or(true, |c| n > c);
                if above && self.beyond.map_or(true, |b| n < b) {
                    self.beyond = Some(n);
                }
            }
            Step::Prev => {
                if self.wrap.map_or(true, |w| n > w) {
                    self.wrap = Some(n);
                }
                let below = self.current.map_or(true, |c| n < c);
                if below && self.beyond.map_or(true, |b| n > b) {
                    self.beyond = Some(n);
                }
            }
        }
    }

    pub fn result(&self) -> Option<u16> {
        self.beyond.or(self.wrap)
    }
}

/// [`Nearest`] with a veto: the slot one step from `current` that `accept`
/// agrees to, skipping the ones it rejects like gaps in the numbering.
///
/// `scan` is called once per round with a fresh [`Nearest`] and must offer
/// every slot number present. A rejected pick becomes the new `current`, so
/// the next round steps on from it (wrapping, like a plain step). The loop is
/// bounded by the number of slots the first round saw, so a card where
/// nothing is acceptable ends in `None` rather than going round forever. The
/// cost is one `scan` per round plus one `accept` per pick: with `k` rejected
/// slots, `k + 1` scans of the card.
///
/// When `current` is the only acceptable slot the result is `current` itself
/// (the wrap lands back on it), exactly as a plain step does.
pub fn step_accepted<S, A>(
    current: Option<u16>,
    dir: Step,
    mut scan: S,
    mut accept: A,
) -> Option<u16>
where
    S: FnMut(&mut Nearest),
    A: FnMut(u16) -> bool,
{
    let mut from = current;
    let mut limit = 0u16;
    let mut tries = 0u16;
    loop {
        let mut n = Nearest::new(from, dir);
        scan(&mut n);
        if tries == 0 {
            limit = n.offered();
        }
        let pick = n.result()?;
        if accept(pick) {
            return Some(pick);
        }
        tries += 1;
        if tries >= limit {
            return None;
        }
        from = Some(pick);
    }
}
