//! Pure logic behind the optional SD read-path speed-ups, kept free of
//! hardware so it can be tested on the host (docs/research/g3-read-path.md).
//!
//! * [`readahead`] -- decides when, and how much, to read ahead of a
//!   streaming `CD_READ` (firmware feature `sd-multiblock`).
//! * [`cmd6`] -- the SD CMD6 (SWITCH_FUNC) arguments and the 64-byte status
//!   parser used to enter High-Speed mode (firmware feature `sd-highspeed`).

#![cfg_attr(not(test), no_std)]

pub mod cmd6;
pub mod readahead;
