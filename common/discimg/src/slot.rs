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
}

impl Nearest {
    pub fn new(current: Option<u16>, dir: Step) -> Self {
        Nearest { current, dir, beyond: None, wrap: None }
    }

    pub fn offer(&mut self, n: u16) {
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
