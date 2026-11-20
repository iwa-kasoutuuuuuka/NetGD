//! The only way to build an `EEFC_FCR` value.
//!
//! Everything that writes `EEFC_FCR` (resident, updater) must obtain an
//! [`FcrCmd`] (the EEFC register block and the word, private fields, built
//! only by the constructors below, which take the [`Part`]); there is no
//! other constructor and no way to ask for a command that is not on the
//! allowlist, or for plane 1 on a part that has none. The allowlist is
//! **EWP, GLB and (resident only) SLB**. SGPB (which, with argument 0, is the
//! security-bit word `0x5A00000B` that locks SWD out), CGPB, CLB, EA, WP, WPL,
//! EWPL, GGPB, GETD, STUI and SPUI cannot be produced, for any argument.
//!
//! Datasheet facts used (SAM3U datasheet Atmel-6430G, text extract
//! `sam3u.txt`; line numbers refer to that extract):
//!
//! * `EEFC_FCR` is at 0x400E0804 (EEFC0) / 0x400E0A04 (EEFC1), write-only,
//!   FKEY bits 31:24, FARG bits 23:8, FCMD bits 7:0 (lines 13405-13436).
//!   The register blocks are at 0x400E0800 (EEFC0) and 0x400E0A00 (EEFC1)
//!   (lines 13345-13361); FSR is at +8 (lines 13449).
//! * FKEY must be 0x5A, otherwise nothing happens (lines 13438-13441).
//! * Command values (Table 20-1, lines 13015-13028): GETD 0x00, WP 0x01,
//!   WPL 0x02, **EWP 0x03**, EWPL 0x04, EA 0x05, **SLB 0x08**, CLB 0x09,
//!   **GLB 0x0A**, SGPB 0x0B, CGPB 0x0C, GGPB 0x0D, STUI 0x0E, SPUI 0x0F.
//! * FARG of a programming command is the page number (within that EEFC's
//!   plane, as `mcu/fwupdater/src/main.rs` already does), of a lock command
//!   a page number of the region to lock (lines 13432-13436), but the text
//!   leaves open whether SLB means a page or a lock-bit number, so SLB is
//!   only produced with 0 (page 0 and bit 0 are both the resident). For GLB
//!   FARG is "meaningless" (line 13251), so the fixed [`GLB_FARG`] = 0x20 is
//!   used: a one-bit flip of the word then cannot become SGPB of a real
//!   GPNVM bit (0..2).
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
//! * GLB only with argument [`GLB_FARG`].
//! * SLB only if the caller opts in explicitly (the resident does, an
//!   updater passes `false`), only on plane 0 and only with argument 0.
//! * Plane 1 only on a part that has it (SAM3U4E).
//!
//! Hamming distance (tested over every producible word, `tests/basics.rs`):
//! every word differs from SGPB/CGPB of GPNVM 0..2, EA and CLB in at least
//! two bits, with named exceptions: EWP vs WP of the same page and SLB 0 vs
//! CGPB 0 cannot do harm (WP rewrites the page that is being written; CGPB 0
//! cannot clear the security bit); **SLB 0 vs CLB 0 is distance 1 and CLB 0
//! would unlock the resident**, which cannot be avoided (0 is the only SLB
//! argument that means region 0 under both readings of FARG) and needs a
//! one-bit error in the one FCR write of the lock step (the caller reads GLB
//! afterwards).

use crate::part::Part;

/// Flash writing protection key (FKEY).
pub const KEY: u32 = 0x5A;
pub const CMD_EWP: u8 = 0x03;
pub const CMD_SLB: u8 = 0x08;
pub const CMD_GLB: u8 = 0x0A;

/// Pages in one 128 KB plane.
pub const PAGES_PER_PLANE: u32 = 512;
/// Pages of the resident (lock region 0 of plane 0): 8 KB / 256 B.
pub const RESIDENT_PAGES: u32 = 32;
/// The one FARG GLB is issued with (meaningless to the controller).
pub const GLB_FARG: u32 = 0x20;

/// Register block of EEFC0 (plane 0, with the resident).
pub const EEFC0_BASE: u32 = 0x400E_0800;
/// Register block of EEFC1 (plane 1, SAM3U4E only).
pub const EEFC1_BASE: u32 = 0x400E_0A00;

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
    /// Get lock bits; arg is [`GLB_FARG`].
    Glb,
    /// Set lock bit; arg is 0 (resident only).
    Slb,
}

/// An allowed command for one EEFC: which register block it goes to and the
/// `EEFC_FCR` word. The fields are private; the only way to get one is a
/// constructor that checks part, plane, command and argument together.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FcrCmd {
    base: u32,
    word: u32,
}

impl FcrCmd {
    /// EWP of `page` of `plane` (page >= 32 on plane 0; plane 1 only on a 4E).
    #[inline]
    pub fn ewp(part: Part, plane: Plane, page: u32) -> Option<FcrCmd> {
        FcrCmd::new(part, plane, Cmd::Ewp, page, false)
    }

    /// GLB of `plane`, with the fixed [`GLB_FARG`].
    #[inline]
    pub fn glb(part: Part, plane: Plane) -> Option<FcrCmd> {
        FcrCmd::new(part, plane, Cmd::Glb, GLB_FARG, false)
    }

    /// SLB (lock the resident's region). `allow_self_lock` is explicit:
    /// true only in the resident, false in an updater.
    #[inline]
    pub fn slb(part: Part, allow_self_lock: bool) -> Option<FcrCmd> {
        FcrCmd::new(part, Plane::P0, Cmd::Slb, 0, allow_self_lock)
    }

    /// The general constructor; `None` for anything outside the allowlist.
    #[inline]
    pub fn new(part: Part, plane: Plane, cmd: Cmd, arg: u32, allow_slb: bool) -> Option<FcrCmd> {
        let word = fcr_word_raw(part, plane, cmd_code(cmd), arg, allow_slb)?;
        Some(FcrCmd { base: plane_base(plane), word })
    }

    /// `EEFC_FCR` value.
    #[inline]
    pub fn word(self) -> u32 {
        self.word
    }

    /// Address of the EEFC register block (FMR at +0).
    #[inline]
    pub fn eefc_base(self) -> u32 {
        self.base
    }

    /// Address of `EEFC_FCR` (write-only).
    #[inline]
    pub fn fcr_addr(self) -> u32 {
        self.base + 4
    }

    /// Address of `EEFC_FSR`.
    #[inline]
    pub fn fsr_addr(self) -> u32 {
        self.base + 8
    }

    #[inline]
    pub fn plane(self) -> Plane {
        if self.base == EEFC1_BASE {
            Plane::P1
        } else {
            Plane::P0
        }
    }

    /// The command code (FCMD).
    #[inline]
    pub fn code(self) -> u8 {
        self.word as u8
    }

    /// FARG (the page of an EWP).
    #[inline]
    pub fn arg(self) -> u32 {
        (self.word >> 8) & 0xFFFF
    }

    /// Flash address of page `arg()` of this command's plane: where an EWP
    /// latches its data.
    #[inline]
    pub fn page_addr(self) -> u32 {
        let flash = if self.base == EEFC1_BASE { 0x0010_0000 } else { 0x0008_0000 };
        flash + self.arg() * 256
    }
}

#[inline]
fn plane_base(p: Plane) -> u32 {
    match p {
        Plane::P0 => EEFC0_BASE,
        Plane::P1 => EEFC1_BASE,
    }
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
/// [`FcrCmd`].
#[doc(hidden)]
pub fn fcr_word_raw(part: Part, plane: Plane, cmd: u8, arg: u32, allow_slb: bool) -> Option<u32> {
    if plane == Plane::P1 && !part.has_plane1() {
        return None;
    }
    let ok = match cmd {
        CMD_EWP => match plane {
            Plane::P0 => arg >= RESIDENT_PAGES && arg < PAGES_PER_PLANE,
            Plane::P1 => arg < PAGES_PER_PLANE,
        },
        CMD_GLB => arg == GLB_FARG,
        CMD_SLB => allow_slb && plane == Plane::P0 && arg == 0,
        _ => false,
    };
    if ok {
        Some((KEY << 24) | (arg << 8) | cmd as u32)
    } else {
        None
    }
}
