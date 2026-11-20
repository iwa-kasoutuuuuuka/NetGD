//! Write session: the pure state behind `IMAGE_CTL` sub-commands 6/7/8
//! (`docs/design/bba-sd-write.md` 4.2, F6-F8).
//!
//! A write session is "the GD-ROM is ejected on purpose so a host can write
//! the card". This holds what the firmware must remember about it and the
//! decisions that depend only on arguments: the begin/end transitions, the
//! hard-reset rule, the protected range (F7), the checksum of each write
//! command (F8) and the 48-byte STATUS reply. The firmware owns the one
//! instance, the hardware and the remount; it calls in.

/// `IMAGE_CTL` packet[1] values.
pub const SUB_WSESSION_BEGIN: u8 = 6;
pub const SUB_WSESSION_END: u8 = 7;
pub const SUB_WSESSION_STATUS: u8 = 8;

/// BEGIN's key, `packet[2..6]`: a stray 0xE0 must not eject the drive.
pub const BEGIN_KEY: &[u8; 4] = b"NGWS";
/// STATUS reply: length and magic.
pub const STATUS_LEN: usize = 48;
pub const STATUS_MAGIC: &[u8; 4] = b"NGST";
/// STATUS flag bits (byte 4).
pub const FLAG_MEDIA: u8 = 0x01;
pub const FLAG_SESSION: u8 = 0x02;
pub const FLAG_LAST_OK: u8 = 0x04;

pub const FNV_INIT: u32 = 2166136261;

/// FNV-1a, 32 bit, continuing from `h`.
pub fn fnv1a(mut h: u32, data: &[u8]) -> u32 {
    for &b in data {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    h
}

/// What `WSESSION_END` should mount (`packet[2]`).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Remount {
    /// 0: leave the drive empty.
    None,
    /// 1: the previous selection or the configured default (`load_media`).
    Default,
    /// 2: the name staged with `MOUNT_NAME` (`mount_named`).
    Named,
}

impl Remount {
    pub fn from_byte(b: u8) -> Option<Remount> {
        match b {
            0 => Some(Remount::None),
            1 => Some(Remount::Default),
            2 => Some(Remount::Named),
            _ => None,
        }
    }
}

/// Why BEGIN or END was refused (the firmware answers ILLEGAL REQUEST).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    BadKey,
    NotActive,
    BadSelector,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct WriteSession {
    active: bool,
    /// First LBA the session may write (the first partition's start);
    /// 0 = unknown, nothing protected.
    protect_below: u32,
    sectors: u32,
    failed: u32,
    last_lba: u32,
    last_count: u32,
    last_fnv: u32,
    last_ok: bool,
}

impl WriteSession {
    pub const fn new() -> Self {
        Self {
            active: false,
            protect_below: 0,
            sectors: 0,
            failed: 0,
            last_lba: 0,
            last_count: 0,
            last_fnv: FNV_INIT,
            last_ok: false,
        }
    }

    pub fn active(&self) -> bool {
        self.active
    }

    /// `WSESSION_BEGIN`. `partition_start` is the first partition's LBA as
    /// found on the card (0 = none found). A second BEGIN while active is
    /// accepted and starts over (statistics cleared, boundary re-taken):
    /// an app that died without END and runs again must not be locked out,
    /// since only END and a hard reset end a session.
    pub fn begin(&mut self, key: &[u8], partition_start: u32) -> Result<(), Refused> {
        if key != BEGIN_KEY {
            return Err(Refused::BadKey);
        }
        *self = Self::new();
        self.active = true;
        self.protect_below = partition_start;
        Ok(())
    }

    /// `WSESSION_END`. The selector is validated first: a bad one leaves the
    /// session as it was. Statistics are kept until the next BEGIN.
    pub fn end(&mut self, selector: u8) -> Result<Remount, Refused> {
        if !self.active {
            return Err(Refused::NotActive);
        }
        let r = Remount::from_byte(selector).ok_or(Refused::BadSelector)?;
        self.active = false;
        Ok(r)
    }

    /// A hard reset (RSTn falling edge, never SRST) ends the session.
    /// Returns whether one was active.
    pub fn hard_reset(&mut self) -> bool {
        let was = self.active;
        self.active = false;
        was
    }

    /// Whether a write of `count` sectors at `lba` touches the protected
    /// range. Only meaningful during a session.
    pub fn is_protected(&self, lba: u32, count: u32) -> bool {
        self.active && count != 0 && lba < self.protect_below
    }

    /// Start of one write command: records it as the "last" one (checksum
    /// restarted) and returns whether it may proceed. A refused write is
    /// recorded as a failed command.
    pub fn write_begin(&mut self, lba: u32, count: u32) -> bool {
        self.last_lba = lba;
        self.last_count = count;
        self.last_fnv = FNV_INIT;
        self.last_ok = false;
        if self.is_protected(lba, count) {
            self.failed = self.failed.wrapping_add(1);
            return false;
        }
        true
    }

    /// Bytes popped from the FIFO for the current command, in order (any
    /// split, e.g. one 512-byte sector at a time or across F4's runs).
    pub fn write_data(&mut self, data: &[u8]) {
        self.last_fnv = fnv1a(self.last_fnv, data);
    }

    /// End of the command; `sectors_done` is how many reached the card.
    pub fn write_end(&mut self, ok: bool, sectors_done: u32) {
        self.last_ok = ok;
        self.sectors = self.sectors.wrapping_add(sectors_done);
        if !ok {
            self.failed = self.failed.wrapping_add(1);
        }
    }

    /// The 48-byte STATUS reply (little endian).
    pub fn status(&self, media_present: bool) -> [u8; STATUS_LEN] {
        let mut o = [0u8; STATUS_LEN];
        o[..4].copy_from_slice(STATUS_MAGIC);
        o[4] = (media_present as u8 * FLAG_MEDIA)
            | (self.active as u8 * FLAG_SESSION)
            | (self.last_ok as u8 * FLAG_LAST_OK);
        o[8..12].copy_from_slice(&self.sectors.to_le_bytes());
        o[12..16].copy_from_slice(&self.failed.to_le_bytes());
        o[16..20].copy_from_slice(&self.last_lba.to_le_bytes());
        o[20..24].copy_from_slice(&self.last_count.to_le_bytes());
        o[24..28].copy_from_slice(&self.last_fnv.to_le_bytes());
        o[28..32].copy_from_slice(&self.protect_below.to_le_bytes());
        o
    }
}

impl Default for WriteSession {
    fn default() -> Self {
        Self::new()
    }
}

/// First partition's start LBA from sector 0 of the card, read the way
/// `fat::probe` does. `lba0_is_bpb` is the firmware's `looks_like_bpb(sec0)`.
/// 0 when the card has no MBR signature, when LBA 0 already is a volume
/// (superfloppy) or entry 0 is empty: nothing is then protected.
pub fn partition_start(sec0: &[u8; 512], lba0_is_bpb: bool) -> u32 {
    if sec0[510] != 0x55 || sec0[511] != 0xAA || lba0_is_bpb {
        return 0;
    }
    let o = 0x1BE + 8;
    u32::from_le_bytes([sec0[o], sec0[o + 1], sec0[o + 2], sec0[o + 3]])
}
