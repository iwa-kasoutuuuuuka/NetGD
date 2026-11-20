//! The only way to build an `EEFC_FCR` value.
//!
//! Everything that writes `EEFC_FCR` (resident, updater) must obtain the word
//! from [`fcr_word`] / [`fcr_word_for`]; there is no other constructor and
//! no way to ask for a command that is not on the allowlist. The allowlist is
//! **EWP, GLB and (resident only) SLB**. SGPB (which, with argument 0, is the
//! security-bit word `0x5A00000B` that locks SWD out), CGPB, CLB, EA, WP, WPL,
//! EWPL, GGPB, GETD, STUI and SPUI cannot be produced, for any argument.
//!
//! Datasheet facts used (SAM3U datasheet Atmel-6430G, text extract
//! `sam3u.txt`; line numbers refer to that extract):
//!
//! * `EEFC_FCR` is at 0x400E0804 (EEFC0) / 0x400E0A04 (EEFC1), write-only,
//!   FKEY bits 31:24, FARG bits 23:8, FCMD bits 7:0 (lines 13405-13436).
//! * FKEY must be 0x5A, otherwise nothing happens (lines 13438-13441).
//! * Command values (Table 20-1, lines 13015-13028): GETD 0x00, WP 0x01,
//!   WPL 0x02, **EWP 0x03**, EWPL 0x04, EA 0x05, **SLB 0x08**, CLB 0x09,
//!   **GLB 0x0A**, SGPB 0x0B, CGPB 0x0C, GGPB 0x0D, STUI 0x0E, SPUI 0x0F.
//! * FARG of a programming command is the page number (within that EEFC's
//!   plane, as `mcu/fwupdater/src/main.rs` already does), of a lock command
//!   a page number of the region to lock (lines 13432-13436); for GLB it is
//!   "meaningless" (line 13251), so we only ever produce 0 there.
//! * Page size 256 B, lock region 8 KB = 32 pages, 16 lock bits per EEFC
//!   (Table 8-3, lines 1699-1710). A 128 KB plane therefore has 512 pages.
//! * A write or erase in a locked region is aborted and FLOCKE (EEFC_FSR
//!   bit 2) is set (lines 13045-13047, 13209-13210, 13480-13483).
//! * Flash may be read while SLB/CLB/GLB run (line 13262), not
//!   while EWP runs (design section 1-1).
//! * GPNVM0 is the security bit; only SGPB sets it; only the ERASE pin
//!   clears it (lines 1729-1731).
//!
//! The rules enforced here, from design section 3-4:
//! * EWP on plane 0 only for pages >= 32, i.e. never inside the resident's
//!   own 8 KB lock region 0 ([`RESIDENT_PAGES`]). EWP on plane 1 (EEFC1,
//!   which holds only the bitstream and no resident) for any page 0..512.
//! * GLB only with argument 0.
//! * SLB only if the caller's build allows it ([`ALLOW_SLB`]: the resident
//!   does, an updater passes `false` to [`fcr_word_with`]) and only on
//!   plane 0 with a page inside the resident (0..32).
//!
//! Note the Hamming distance to the security-bit word: an EWP word for a
//! page >= 32 differs from `0x5A00000B` (and from SGPB with arguments 1, 2
//! that would set GPNVM1/2) in at least two bits (tested).

/// Flash writing protection key (FKEY).
pub const KEY: u32 = 0x5A;
pub const CMD_EWP: u8 = 0x03;
pub const CMD_SLB: u8 = 0x08;
pub const CMD_GLB: u8 = 0x0A;

/// Pages in one 128 KB plane.
pub const PAGES_PER_PLANE: u32 = 512;
/// Pages of the resident (lock region 0 of plane 0): 8 KB / 256 B.
pub const RESIDENT_PAGES: u32 = 32;

/// Whether the default entry points may produce SLB. True for the resident.
pub const ALLOW_SLB: bool = true;

/// Which EEFC. EEFC0 controls plane 0 (0x0008_0000, with the resident),
/// EEFC1 plane 1 (0x0010_0000, SAM3U4E only).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Plane {
    P0,
    P1,
}

/// The commands that exist as far as this crate is concerned.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cmd {
    /// Erase page and write page; arg = page within the plane.
    Ewp,
    /// Get lock bits; arg is 0.
    Glb,
    /// Set lock bit; arg = a page inside the resident (resident only).
    Slb,
}

/// Plane-0 command word (the EEFC that carries the resident).
#[inline]
pub fn fcr_word(cmd: Cmd, arg: u32) -> Option<u32> {
    fcr_word_raw(Plane::P0, cmd_code(cmd), arg, ALLOW_SLB)
}

#[inline]
pub fn fcr_word_for(plane: Plane, cmd: Cmd, arg: u32) -> Option<u32> {
    fcr_word_raw(plane, cmd_code(cmd), arg, ALLOW_SLB)
}

/// Same with the self-lock permission given explicitly (an updater passes
/// `false`).
#[inline]
pub fn fcr_word_with(plane: Plane, cmd: Cmd, arg: u32, allow_slb: bool) -> Option<u32> {
    fcr_word_raw(plane, cmd_code(cmd), arg, allow_slb)
}

#[inline]
fn cmd_code(c: Cmd) -> u8 {
    match c {
        Cmd::Ewp => CMD_EWP,
        Cmd::Glb => CMD_GLB,
        Cmd::Slb => CMD_SLB,
    }
}

/// The single place where an `EEFC_FCR` word is assembled, from a raw
/// command code so that the exhaustive test can try all 256 of them. `None`
/// for everything outside the allowlist. Not part of the normal API: use
/// [`fcr_word`].
#[doc(hidden)]
pub fn fcr_word_raw(plane: Plane, cmd: u8, arg: u32, allow_slb: bool) -> Option<u32> {
    let ok = match cmd {
        CMD_EWP => match plane {
            Plane::P0 => arg >= RESIDENT_PAGES && arg < PAGES_PER_PLANE,
            Plane::P1 => arg < PAGES_PER_PLANE,
        },
        CMD_GLB => arg == 0,
        CMD_SLB => allow_slb && plane == Plane::P0 && arg < RESIDENT_PAGES,
        _ => false,
    };
    if ok {
        Some((KEY << 24) | (arg << 8) | cmd as u32)
    } else {
        None
    }
}
