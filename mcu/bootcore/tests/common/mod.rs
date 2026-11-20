//! Shared test support: a flash model with lock bits, "no read while busy"
//! and power cuts; a FAT32 image builder; app / OGFW image builders.
#![allow(dead_code)]

use bootcore::app::{APP_BASE, DESC_OFF};
use bootcore::crc;
use bootcore::fat::BlockRead;
use bootcore::fcr::{FcrCmd, Plane, EEFC0_BASE, EEFC1_BASE};
use bootcore::flash::{Flash, FlashError, FLASH_BASE, PAGE, PLANE1_BASE};
use bootcore::part::{Part, FWS_RUN, FWS_WRITE};
use std::cell::Cell;
use std::rc::Rc;

pub mod sim;

// ---------------------------------------------------------------- watchdog

/// What the watchdog sees of a run: the resident rewinds it in every
/// `Card::read_block` and every `Model::ewp`; between two rewinds the model
/// adds up the "work" of the pure code in between, as bytes read from flash
/// (the one thing that loops without I/O: CRC passes over the image).
#[derive(Clone, Default)]
pub struct Wdt(Rc<Cell<WdtState>>);

#[derive(Clone, Copy, Default, Debug)]
pub struct WdtState {
    /// Flash bytes read since the last rewind.
    pub since: u64,
    /// Largest `since` ever seen at a rewind or at [`Wdt::finish`].
    pub max: u64,
    pub feeds: u64,
}

impl Wdt {
    pub fn feed(&self) {
        let mut s = self.0.get();
        s.max = s.max.max(s.since);
        s.since = 0;
        s.feeds += 1;
        self.0.set(s);
    }

    pub fn work(&self, bytes: u64) {
        let mut s = self.0.get();
        s.since += bytes;
        self.0.set(s);
    }

    /// Close the last interval (at hand-off).
    pub fn finish(&self) {
        let mut s = self.0.get();
        s.max = s.max.max(s.since);
        self.0.set(s);
    }

    pub fn state(&self) -> WdtState {
        self.0.get()
    }
}

// ---------------------------------------------------------------- rng

pub fn lcg(seed: u32, n: usize) -> Vec<u8> {
    let mut s = seed.wrapping_mul(2654435761).wrapping_add(12345);
    (0..n)
        .map(|_| {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            (s >> 24) as u8
        })
        .collect()
}

// ---------------------------------------------------------------- images

pub fn put32(b: &mut [u8], off: usize, v: u32) {
    b[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

/// A valid application image of `len` bytes (multiple of 512): random body,
/// good vector table, sealed descriptor, trailing CRC.
pub fn app_image(part: Part, len: usize, seed: u32) -> Vec<u8> {
    assert!(len % 512 == 0 && len >= 512);
    let mut b = lcg(seed, len);
    put32(&mut b, 0, part.sram0_end());
    put32(&mut b, 4, APP_BASE + 0x121);
    put32(&mut b, 8, APP_BASE + 0x123);
    put32(&mut b, 12, APP_BASE + 0x125);
    let d = DESC_OFF as usize;
    put32(&mut b, d, bootcore::app::DESC_MAGIC);
    put32(&mut b, d + 4, 1);
    put32(&mut b, d + 8, len as u32);
    put32(&mut b, d + 12, 0);
    put32(&mut b, d + 16, 0);
    put32(&mut b, d + 20, 0);
    put32(&mut b, d + 24, 0x0100_0000 | (seed & 0xFFFFFF));
    let c = crc::crc32(&b[d..d + 28]);
    put32(&mut b, d + 28, c);
    let c = crc::crc32(&b[..len - 4]);
    put32(&mut b, len - 4, c);
    b
}

pub fn bitstream(len: usize, seed: u32) -> Vec<u8> {
    lcg(seed ^ 0x5555, len)
}

pub fn cidr(part: Part) -> u32 {
    match part {
        Part::Sam3u2e => 0x281A_0761,
        Part::Sam3u4e => 0x2810_0961,
    }
}

pub fn fix_header_crc(h: &mut [u8]) {
    let c = crc::finish(crc::update(crc::update(crc::INIT, &h[..32]), &h[36..40]));
    put32(h, 32, c);
}

/// Header block for an image (format 3, layout 0x2000).
pub fn header(kind: u32, part: Part, dest: u32, payload: &[u8]) -> Vec<u8> {
    let mut h = vec![0u8; 512];
    put32(&mut h, 0, 0x5746_474F);
    put32(&mut h, 4, 3);
    put32(&mut h, 8, kind);
    put32(&mut h, 12, cidr(part));
    put32(&mut h, 16, dest);
    put32(&mut h, 20, payload.len() as u32);
    put32(&mut h, 24, crc::crc32(payload));
    put32(&mut h, 28, 7);
    put32(&mut h, 36, 0x2000);
    fix_header_crc(&mut h);
    h
}

pub fn image(kind: u32, part: Part, payload: &[u8]) -> Vec<u8> {
    let dest = if kind == 0 { APP_BASE } else { PLANE1_BASE };
    let mut v = header(kind, part, dest, payload);
    v.extend_from_slice(payload);
    v
}

// ---------------------------------------------------------------- SD card

pub struct Card {
    pub img: std::sync::Arc<Vec<u8>>,
    pub reads: usize,
    /// Fail the n-th read (0-based) and all after it.
    pub fail_at: Option<usize>,
    /// Flip a byte of the n-th read (transient corruption), (read index, byte offset).
    pub glitch: Option<(usize, usize)>,
    /// From the n-th read on, serve this image instead (the card changed
    /// between two passes).
    pub swap: Option<(usize, std::sync::Arc<Vec<u8>>)>,
    /// Rewound at every `read_block`, as the real implementation must.
    pub wdt: Option<Wdt>,
}

impl Card {
    pub fn new(img: Vec<u8>) -> Card {
        Card::shared(std::sync::Arc::new(img))
    }

    /// Many cards over one image (the sweeps make tens of thousands).
    pub fn shared(img: std::sync::Arc<Vec<u8>>) -> Card {
        Card { img, reads: 0, fail_at: None, glitch: None, swap: None, wdt: None }
    }
}

impl BlockRead for Card {
    fn read_block(&mut self, lba: u32, buf: &mut [u8; 512]) -> Result<(), ()> {
        let n = self.reads;
        self.reads += 1;
        if let Some(w) = &self.wdt {
            w.feed();
        }
        if let Some(f) = self.fail_at {
            if n >= f {
                return Err(());
            }
        }
        let o = lba as usize * 512;
        let img = match &self.swap {
            Some((k, alt)) if n >= *k => alt,
            _ => &self.img,
        };
        match img.get(o..o + 512) {
            Some(s) => buf.copy_from_slice(s),
            None => return Err(()),
        }
        if let Some((i, off)) = self.glitch {
            if i == n {
                buf[off] ^= 0x10;
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- flash model

#[derive(Clone, Copy, Debug)]
pub enum Outcome {
    /// Power went before the command started to change anything.
    Untouched,
    /// Erased, nothing written yet.
    Erased,
    /// Erased and the first k bytes programmed.
    Prefix(usize),
    /// Erase got k bytes far, the rest still old.
    ErasePrefix(usize),
    /// Arbitrary content.
    Garbage,
}

#[derive(Clone, Copy, Debug)]
pub struct Cut {
    pub op: usize,
    pub outcome: Outcome,
}

const P1_OFF: usize = 0x2_0000;

#[derive(Clone)]
pub struct Model {
    /// Plane 0 then plane 1, 128 KB each; erased = 0xFF.
    pub mem: Vec<u8>,
    pub has_p1: bool,
    /// Lock bits per plane (GLB result).
    pub lock: [u32; 2],
    /// True while `ewp` runs. Bootcore is single threaded and `ewp` blocks,
    /// so a `read` seen with this set is a re-entrant call: counted as a
    /// violation (must stay 0).
    in_ewp: bool,
    /// Reads issued while an EWP was running (must stay 0).
    pub violations: u32,
    /// EWPs accepted for execution (not counting refused/locked ones).
    pub ops: usize,
    /// Address of every accepted EWP, in order.
    pub log: Vec<u32>,
    /// FLOCKE count.
    pub flocke: u32,
    pub cut: Option<Cut>,
    /// Like `cut`, but FRDY never comes. The real `ewp` does not return then
    /// (`bootcore::flash` module docs): its RAM routine resets the chip. The
    /// model leaves the outcome in the page, counts `timeout_resets` and goes
    /// `dead`, exactly as for a power cut; the harness must `power_up`.
    pub timeout: Option<Cut>,
    /// EWPs that timed out and reset the chip.
    pub timeout_resets: u32,
    /// The chip is not running any more (power cut or reset inside a
    /// command). Whatever the code under test does after this did not happen
    /// on a board; every flash command fails.
    pub dead: bool,
    /// A weak cell: an EWP of an all-0xFF page (an erase) leaves bit 0 of
    /// this byte cleared.
    pub weak_erase: Option<u32>,
    /// An EWP of the page holding this address does nothing (and succeeds).
    pub ignore: Option<u32>,
    /// A stuck bit: after an EWP that covers this address, bit 0 of that
    /// byte reads back flipped (a failing cell).
    pub stuck: Option<u32>,
    /// `EEFC_FMR.FWS` as the portable code last set it. `None` = not set
    /// since reset (the reset value 0, with which flash reads garbage at
    /// 96 MHz): the resident's start-up sets plane 0, nobody sets plane 1 of
    /// a 4E before `sd_recover` does.
    pub fws: Option<u8>,
    /// Flash commands (EWP, SLB) issued at an FWS other than 6, flash reads
    /// at FWS 6 (which includes the hand-off), and flash reads before the
    /// first `set_fws`: must stay 0. Not reset by `power_up`.
    pub fws_violations: u32,
    /// GLBs issued, per plane.
    pub glbs: [u32; 2],
    /// Per plane: this many of the next GLBs fail (`u32::MAX`: all of them).
    pub glb_fail: [u32; 2],
    /// SLB commands issued.
    pub slbs: u32,
    /// SLB commands issued while region 0 already read locked (must stay 0:
    /// SLB 0 is one bit from CLB 0).
    pub slb_while_locked: u32,
    /// Fault for the next SLB only.
    pub slb_fault: Option<SlbFault>,
    pub wdt: Option<Wdt>,
}

/// What happens to the next SLB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlbFault {
    /// The command word arrives as CLB 0: lock bit 0 is cleared, not set.
    BecomesClb,
    /// Nothing happens (and the command reports success).
    Ignored,
    /// Power goes in the middle; the lock bit is left set (`true`) or clear.
    PowerCut { bit_set: bool },
}

impl Model {
    pub fn new(part: Part) -> Model {
        let mut m = Model {
            mem: vec![0xFF; 2 * P1_OFF],
            has_p1: part.has_plane1(),
            lock: [1, 0], // region 0 of plane 0 = the resident, locked
            in_ewp: false,
            violations: 0,
            ops: 0,
            log: vec![],
            flocke: 0,
            cut: None,
            timeout: None,
            timeout_resets: 0,
            dead: false,
            weak_erase: None,
            ignore: None,
            stuck: None,
            fws: None,
            fws_violations: 0,
            glbs: [0; 2],
            glb_fail: [0; 2],
            slbs: 0,
            slb_while_locked: 0,
            slb_fault: None,
            wdt: None,
        };
        // A resident that nobody may touch.
        let r = lcg(99, 0x2000);
        m.mem[..0x2000].copy_from_slice(&r);
        m
    }

    fn idx(addr: u32) -> Option<usize> {
        if addr >= FLASH_BASE && addr < FLASH_BASE + 0x2_0000 {
            Some((addr - FLASH_BASE) as usize)
        } else if addr >= PLANE1_BASE && addr < PLANE1_BASE + 0x2_0000 {
            Some(P1_OFF + (addr - PLANE1_BASE) as usize)
        } else {
            None
        }
    }

    /// Put bytes in flash directly (test setup, not an EWP).
    pub fn poke(&mut self, addr: u32, data: &[u8]) {
        let i = Model::idx(addr).unwrap();
        self.mem[i..i + data.len()].copy_from_slice(data);
    }

    pub fn peek(&self, addr: u32, len: usize) -> &[u8] {
        let i = Model::idx(addr).unwrap();
        &self.mem[i..i + len]
    }

    /// Back from a power cut or a reset: controller idle, FWS at its reset
    /// value (unset), cut plan cleared.
    pub fn power_up(&mut self) {
        self.dead = false;
        self.in_ewp = false;
        self.cut = None;
        self.timeout = None;
        self.fws = None;
        self.violations = 0;
        self.ops = 0;
        self.log.clear();
    }

    fn apply_cut(&mut self, i: usize, new: &[u8; PAGE], o: Outcome) {
        let old: Vec<u8> = self.mem[i..i + PAGE].to_vec();
        let page = &mut self.mem[i..i + PAGE];
        match o {
            Outcome::Untouched => {}
            Outcome::Erased => page.fill(0xFF),
            Outcome::Prefix(k) => {
                page.fill(0xFF);
                page[..k].copy_from_slice(&new[..k]);
            }
            Outcome::ErasePrefix(k) => {
                page.copy_from_slice(&old);
                page[..k].fill(0xFF);
            }
            Outcome::Garbage => page.copy_from_slice(&lcg(i as u32 ^ 0xBEEF, PAGE)),
        }
    }
}

impl Model {
    fn ewp_inner(&mut self, cmd: FcrCmd, data: &[u8; PAGE]) -> Result<(), FlashError> {
        if self.dead {
            return Err(FlashError::Failed);
        }
        // The hardware sees exactly this register and word, nothing else.
        let w = cmd.word();
        let plane = cmd.plane();
        let page = cmd.arg();
        assert_eq!(w >> 24, 0x5A);
        assert_eq!(w & 0xFF, 3, "not an EWP");
        assert_eq!(cmd.eefc_base(), if plane == Plane::P0 { EEFC0_BASE } else { EEFC1_BASE });
        assert_eq!(cmd.fcr_addr(), cmd.eefc_base() + 4);
        assert_eq!(cmd.page_addr(), if plane == Plane::P0 { FLASH_BASE } else { PLANE1_BASE } + page * PAGE as u32);
        if plane == Plane::P1 && !self.has_p1 {
            return Err(FlashError::Refused);
        }
        let pi = if plane == Plane::P0 { 0 } else { 1 };
        let i = pi * P1_OFF + page as usize * PAGE;
        let region = page / 32;
        if (self.lock[pi] >> region) & 1 != 0 {
            // aborted, FLOCKE set, plane untouched
            self.flocke += 1;
            return Err(FlashError::Locked);
        }
        let op = self.ops;
        self.ops += 1;
        self.log.push(cmd.page_addr());
        if let Some(c) = self.cut {
            if c.op == op {
                self.apply_cut(i, data, c.outcome);
                self.dead = true;
                return Err(FlashError::Failed);
            }
        }
        if let Some(c) = self.timeout {
            if c.op == op {
                // the RAM routine resets the chip: nothing after this runs
                self.apply_cut(i, data, c.outcome);
                self.timeout_resets += 1;
                self.dead = true;
                return Err(FlashError::Failed);
            }
        }
        if let Some(a) = self.ignore {
            if let Some(si) = Model::idx(a) {
                if si >= i && si < i + PAGE {
                    return Ok(());
                }
            }
        }
        self.mem[i..i + PAGE].copy_from_slice(data);
        if let Some(a) = self.weak_erase {
            if let Some(si) = Model::idx(a) {
                if si >= i && si < i + PAGE && data.iter().all(|&b| b == 0xFF) {
                    self.mem[si] &= !1;
                }
            }
        }
        if let Some(a) = self.stuck {
            if let Some(si) = Model::idx(a) {
                if si >= i && si < i + PAGE {
                    self.mem[si] ^= 1;
                }
            }
        }
        Ok(())
    }
}

impl Flash for Model {
    fn read(&mut self, addr: u32, out: &mut [u8]) {
        if self.in_ewp {
            self.violations += 1;
        }
        if self.fws != Some(FWS_RUN) {
            // FWS 6 (a command's setting left on) or never set since reset
            self.fws_violations += 1;
        }
        if let Some(w) = &self.wdt {
            w.work(out.len() as u64);
        }
        match Model::idx(addr) {
            Some(i) if i + out.len() <= self.mem.len() => out.copy_from_slice(&self.mem[i..i + out.len()]),
            _ => out.fill(0xFF),
        }
    }

    fn set_fws(&mut self, fws: u8) {
        assert!(fws == FWS_RUN || fws == FWS_WRITE, "FWS {fws}");
        self.fws = Some(fws);
    }

    fn slb(&mut self, cmd: FcrCmd) -> Result<(), FlashError> {
        assert_eq!(cmd.word(), 0x5A00_0008, "not SLB 0");
        assert_eq!(cmd.eefc_base(), EEFC0_BASE);
        if self.fws != Some(FWS_WRITE) {
            self.fws_violations += 1;
        }
        if self.dead {
            return Err(FlashError::Failed);
        }
        self.slbs += 1;
        if self.lock[0] & 1 != 0 {
            self.slb_while_locked += 1;
        }
        match self.slb_fault.take() {
            None => self.lock[0] |= 1,
            Some(SlbFault::BecomesClb) => self.lock[0] &= !1,
            Some(SlbFault::Ignored) => {}
            Some(SlbFault::PowerCut { bit_set }) => {
                if bit_set {
                    self.lock[0] |= 1;
                } else {
                    self.lock[0] &= !1;
                }
                self.dead = true;
                return Err(FlashError::Failed);
            }
        }
        Ok(())
    }

    fn ewp(&mut self, cmd: FcrCmd, data: &[u8; PAGE]) -> Result<(), FlashError> {
        if let Some(w) = &self.wdt {
            w.feed();
        }
        if self.fws != Some(FWS_WRITE) {
            self.fws_violations += 1;
        }
        assert!(!self.in_ewp, "EWP entered re-entrantly");
        self.in_ewp = true;
        let r = self.ewp_inner(cmd, data);
        self.in_ewp = false;
        r
    }

    fn lock_bits(&mut self, plane: Plane) -> Option<u32> {
        let pi = if plane == Plane::P0 { 0 } else { 1 };
        self.glbs[pi] += 1;
        if self.dead || (pi == 1 && !self.has_p1) {
            return None;
        }
        if self.glb_fail[pi] > 0 {
            if self.glb_fail[pi] != u32::MAX {
                self.glb_fail[pi] -= 1;
            }
            return None;
        }
        Some(self.lock[pi])
    }
}

// ---------------------------------------------------------------- FAT32 builder

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// Lowest free clusters, ascending.
    Contig,
    /// Lowest free clusters, chained in descending order.
    Reverse,
    /// Every third free cluster.
    Stride3,
}

pub struct Fat {
    pub img: Vec<u8>,
    pub spc: u32,
    pub reserved: u32,
    pub nfats: u32,
    pub spf: u32,
    pub total: u32,
    pub used: Vec<bool>,
    pub root_chain: Vec<u32>,
    pub nent: usize,
}

pub const EOC: u32 = 0x0FFF_FFFF;

impl Fat {
    pub fn new(total: u32, spc: u32) -> Fat {
        let reserved = 32;
        let nfats = 2;
        let mut spf = 1;
        loop {
            let clusters = (total - reserved - nfats * spf) / spc;
            if (clusters + 2) * 4 <= spf * 512 {
                break;
            }
            spf += 1;
        }
        let mut f = Fat {
            img: vec![0; total as usize * 512],
            spc,
            reserved,
            nfats,
            spf,
            total,
            used: vec![],
            root_chain: vec![],
            nent: 0,
        };
        let clusters = (total - reserved - nfats * spf) / spc;
        f.used = vec![false; clusters as usize + 2];
        f.used[0] = true;
        f.used[1] = true;
        let b = &mut f.img[..512];
        b[0] = 0xEB;
        b[1] = 0x58;
        b[2] = 0x90;
        b[3..11].copy_from_slice(b"MSWIN4.1");
        b[0x0B..0x0D].copy_from_slice(&512u16.to_le_bytes());
        b[0x0D] = spc as u8;
        b[0x0E..0x10].copy_from_slice(&(reserved as u16).to_le_bytes());
        b[0x10] = nfats as u8;
        b[0x15] = 0xF8;
        put32(b, 0x20, total);
        put32(b, 0x24, spf);
        put32(b, 0x2C, 2);
        b[0x30] = 1;
        b[0x32] = 6;
        b[0x42] = 0x29;
        b[0x52..0x5A].copy_from_slice(b"FAT32   ");
        b[510] = 0x55;
        b[511] = 0xAA;
        f.set_fat(0, 0x0FFF_FFF8);
        f.set_fat(1, EOC);
        let root = f.alloc_one();
        assert_eq!(root, 2);
        f.set_fat(root, EOC);
        f.root_chain.push(root);
        f
    }

    pub fn clusters(&self) -> u32 {
        self.used.len() as u32 - 2
    }

    pub fn set_fat(&mut self, c: u32, v: u32) {
        for n in 0..self.nfats {
            let off = ((self.reserved + n * self.spf) * 512 + c * 4) as usize;
            put32(&mut self.img, off, v);
        }
    }

    pub fn get_fat(&self, c: u32) -> u32 {
        let off = (self.reserved * 512 + c * 4) as usize;
        u32::from_le_bytes(self.img[off..off + 4].try_into().unwrap())
    }

    fn alloc_one(&mut self) -> u32 {
        let c = self.used.iter().position(|u| !u).expect("volume full") as u32;
        self.used[c as usize] = true;
        c
    }

    pub fn cluster_off(&self, c: u32) -> usize {
        ((self.reserved + self.nfats * self.spf + (c - 2) * self.spc) * 512) as usize
    }

    pub fn alloc_list(&mut self, n: usize, layout: Layout) -> Vec<u32> {
        let mut v = vec![];
        match layout {
            Layout::Contig | Layout::Reverse => {
                for _ in 0..n {
                    v.push(self.alloc_one());
                }
                if layout == Layout::Reverse {
                    v.reverse();
                }
            }
            Layout::Stride3 => {
                let mut c = 2;
                while v.len() < n {
                    if !self.used[c] {
                        self.used[c] = true;
                        v.push(c as u32);
                        c += 3;
                    } else {
                        c += 1;
                    }
                }
            }
        }
        v
    }

    pub fn push_entry(&mut self, e: &[u8; 32]) {
        let per = (self.spc * 16) as usize;
        let ci = self.nent / per;
        while ci >= self.root_chain.len() {
            let c = self.alloc_one();
            let prev = *self.root_chain.last().unwrap();
            self.set_fat(prev, c);
            self.set_fat(c, EOC);
            self.root_chain.push(c);
        }
        let off = self.cluster_off(self.root_chain[ci]) + (self.nent % per) * 32;
        self.img[off..off + 32].copy_from_slice(e);
        self.nent += 1;
    }

    pub fn dirent(name: &[u8; 11], attr: u8, first: u32, size: u32) -> [u8; 32] {
        let mut e = [0u8; 32];
        e[..11].copy_from_slice(name);
        e[11] = attr;
        e[20..22].copy_from_slice(&((first >> 16) as u16).to_le_bytes());
        e[26..28].copy_from_slice(&(first as u16).to_le_bytes());
        e[28..32].copy_from_slice(&size.to_le_bytes());
        e
    }

    /// A long-name entry; `first11` lets a test make its first 11 bytes
    /// anything (e.g. the 8.3 name it must not be mistaken for).
    pub fn lfn_entry(seq: u8, first11: Option<&[u8; 11]>) -> [u8; 32] {
        let mut e = [0u8; 32];
        e[0] = seq;
        for (i, c) in b"long-file-name".iter().take(5).enumerate() {
            e[1 + 2 * i] = *c;
        }
        e[11] = 0x0F;
        if let Some(n) = first11 {
            e[..11].copy_from_slice(n);
        }
        e
    }

    /// Store a file and return its first cluster. `lfn`: write two long-name
    /// entries in front of the short entry.
    pub fn add_file(&mut self, name: &[u8; 11], data: &[u8], layout: Layout, lfn: bool) -> u32 {
        let first = self.put_data(data, layout);
        if lfn {
            self.push_entry(&Fat::lfn_entry(0x42, None));
            self.push_entry(&Fat::lfn_entry(0x01, None));
        }
        self.push_entry(&Fat::dirent(name, 0x20, first, data.len() as u32));
        first
    }

    pub fn put_data(&mut self, data: &[u8], layout: Layout) -> u32 {
        let csz = (self.spc * 512) as usize;
        let n = (data.len() + csz - 1) / csz;
        let list = self.alloc_list(n, layout);
        for (i, &c) in list.iter().enumerate() {
            let off = self.cluster_off(c);
            let part = &data[i * csz..data.len().min((i + 1) * csz)];
            self.img[off..off + part.len()].copy_from_slice(part);
            let next = if i + 1 < list.len() { list[i + 1] } else { EOC };
            self.set_fat(c, next);
        }
        list[0]
    }

    pub fn chain(&self, first: u32) -> Vec<u32> {
        let mut v = vec![first];
        loop {
            let n = self.get_fat(*v.last().unwrap()) & 0x0FFF_FFFF;
            if n >= 0x0FFF_FFF8 || v.len() > 100000 {
                return v;
            }
            v.push(n);
        }
    }
}

/// Put a FAT32 volume behind an MBR (partition 0 at LBA 2048).
pub fn with_mbr(fs: &[u8], ty: u8) -> Vec<u8> {
    let start = 2048usize;
    let mut out = vec![0u8; start * 512];
    let e = 0x1BE;
    out[e] = 0x80;
    out[e + 4] = ty;
    put32(&mut out, e + 8, start as u32);
    put32(&mut out, e + 12, (fs.len() / 512) as u32);
    out[510] = 0x55;
    out[511] = 0xAA;
    out.extend_from_slice(fs);
    out
}

/// A convenient 32 MB card with one file, behind an MBR.
pub fn card_with(name: &[u8; 11], data: &[u8]) -> Vec<u8> {
    let mut f = Fat::new(8192 * 4, 1);
    f.add_file(name, data, Layout::Contig, false);
    with_mbr(&f.img, 0x0C)
}
