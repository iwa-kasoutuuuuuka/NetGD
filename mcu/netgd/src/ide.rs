//! ATA/ATAPI command task on the MCU side, serving both emulated devices.
//!
//! One task, `ide_task`, services the FPGA's two IDE devices. The FPGA ORs
//! each device's `COMMAND_PEND` onto MCU_IRQn, which lands on PA22 here
//! (falling edge, `pioa_wait_irq`). After every wake-up the task sweeps both
//! devices; the disc-swap button wakes it the same way. A command is
//! snapshotted from the register window, dispatched on its opcode, answered
//! through STATUS / ERROR / INTRQ, and acknowledged by clearing COMMAND_PEND
//! -- by number, so that only the command that was snapshotted is
//! acknowledged (`take_command`, `ack_command`).
//! The register layout is in `docs/AddressMapMCU.md`.
//!
//! The two devices are told apart by `Role`:
//!
//! - Primary, `Role::Packet`: the GD-ROM, an ATAPI packet device. IDENTIFY
//!   DEVICE is ABRTed so the host falls back to IDENTIFY PACKET DEVICE.
//!   PACKET (`0xA0`) runs the ATAPI packet protocol in `handle_packet`. That
//!   covers TEST UNIT READY, REQUEST SENSE and START STOP UNIT (eject and
//!   load), plus Sega's GD-ROM set: TOC and session queries, CD_READ and
//!   CD_READ2 from the mounted image (`handle_cd_read`, PIO or DMA, streamed
//!   from SD), CD_PLAY and CD_SEEK for CD audio (`cdda.rs`), and vendor `0xE0`
//!   (IMAGE_CTL) to mount an image by folder name. A unit attention left by a
//!   reset is reported as CHECK CONDITION once, on the first command after the
//!   reset other than REQUEST SENSE / REQ_ERROR; the sense key stays pending
//!   for the REQUEST SENSE that should follow.
//! - Secondary, `Role::Disk`: a plain ATA disk that exposes the whole SD card,
//!   MBR included. IDENTIFY DEVICE is answered. READ and WRITE SECTORS, their
//!   DMA forms and the LBA48 variants go to `handle_disk_read` and
//!   `handle_disk_write`; CHS is refused. Writes are refused while GD-ROM
//!   media is loaded, so the host has to eject first. Vendor `0xF9` is the
//!   firmware updater entry point (`fwupdate.rs`).
//!
//! Generic ATA commands (NOP, SET FEATURES) are acknowledged on both devices;
//! DEVICE RESET and PACKET only on the packet device. Anything else is ABRTed
//! and logged. Every command, known or
//! not, is logged via defmt so bus traffic can be followed during bring-up.

use core::ptr::{read_volatile, write_volatile};

use defmt::{info, warn};
use embassy_atsam3::smc;
use embassy_futures::yield_now;
use embassy_time::{Duration, Instant, Timer};

// ----- Per-device 7-bit register window (matches FPGA hw/ide_device.py). ---

const REG_HW_INFO: usize = 0;
const REG_STATUS: usize = 1;
#[allow(dead_code)]
const REG_DEVICE: usize = 2;
const REG_ERROR: usize = 3;
const REG_CMD_FEAT: usize = 4;
const REG_SECTOR_COUNT: usize = 5;
const REG_LBA_LOW_MID: usize = 6;
const REG_LBA_HIGH_LOWP: usize = 7;
const REG_LBA_MIDP_HIGHP: usize = 8;
const REG_DATA_FIFO: usize = 9;
const REG_READ_FIFO_LEVEL: usize = 10;
const REG_WRITE_FIFO_LEVEL: usize = 11;
/// IDE INTRQ-pending flop. Write 1 to assert; cleared in hardware when
/// the host reads the IDE STATUS register.
const REG_INTRQ_PENDING: usize = 13;
/// DMA mode arm/disarm. Write 1 before staging the data-in payload of a
/// DMA-mode PACKET so DMARQ goes high once the read FIFO has data; write
/// 0 once the host has finished draining. Without this the FPGA's DMARQ
/// stays low and the host's DMA controller waits forever.
const REG_DMA_MODE: usize = 19;
/// Hard-reset (RSTn) edge counters, read-only: low byte rises, high byte
/// falls. Driven by the G1 RSTn line alone -- SRST (Device Control bit 2)
/// never moves them, which is how a hard reset is told from SRST
/// (`fpga/amaranth/hw/ide_device.py`, `rstn_fall_evt`).
const REG_RSTN_EDGES: usize = 16;
/// In-fabric signal capture (opt-in in the FPGA). Slot 20 arms and reports,
/// slot 21 is write-index / read-value.
const REG_CAP_CTRL: usize = 20;
const REG_CAP_DATA: usize = 21;
const CAP_DEPTH: usize = 512;
/// Host access counters since the last Command write: Status/Alt Status
/// reads (slot 24, lo/hi bytes), Data reads (slot 25), last Device Control
/// byte / write count (slot 26). Read-only; the FPGA clears 24/25 itself.
const REG_HOST_STATUS_RDS: usize = 24;
const REG_HOST_DATA_RDS: usize = 25;
const REG_HOST_DEVCTL: usize = 26;
/// Slots 24/25 frozen at the most recent Command write: what the host did
/// during the *previous* command, readable after it has moved on.
const REG_HOST_PREV_STATUS_RDS: usize = 27;
const REG_HOST_PREV_DATA_RDS: usize = 28;

// HW_INFO bit layout.
const HW_CMD_PEND: u16 = 0x0002;
/// Sticky: the host reset the device (ATA hard reset or SRST) since the
/// MCU last cleared it. Write 1 to clear.
const HW_RESET_SEEN: u16 = 0x0004;
const HW_NIEN: u16 = 0x0008;
const HW_IN_RESET: u16 = 0x0010;
/// The command's number (bits 8-14, counts Command writes) and the FPGA's
/// "I acknowledge by number" flag (bit 15). Written back to slot 0 with
/// COMMAND_PEND clear, this byte acknowledges that command and no later one.
/// A bitstream from before the number reads it as 0, and writing 0 back is
/// the old unconditional acknowledge -- so nothing here asks which it has.
const HW_ACK_TOKEN: u16 = 0xFF00;

/// Slot 19 write mask (bits 8-12 enable the write of bits 0-4; see
/// `ide_device.py`, case 19): DMA mode and both PIO flow controls, the
/// transfer state this side owns. Bit 4, the packet phase, is the FPGA's --
/// it raises it on a PACKET Command write -- and stays out of every write
/// made here. An older bitstream ignores the mask and writes all five.
const XFER_MASK: u16 = 0x0F00;

/// `HW_ACK_TOKEN` of the command `sweep` last took, for `ack_command` and
/// `post_status`. One
/// cell serves both devices: `ide_task` takes, serves and acknowledges one
/// command at a time, and nothing acknowledges outside that.
static ACK_TOKEN: core::sync::atomic::AtomicU16 = core::sync::atomic::AtomicU16::new(0);

// Status register bits (matches ATA: BSY DRDY DF DSC DRQ - - ERR).
const STATUS_BSY: u8 = 0x80;
const STATUS_DRDY: u8 = 0x40;
#[allow(dead_code)]
const STATUS_DF: u8 = 0x20;
const STATUS_DSC: u8 = 0x10;
const STATUS_DRQ: u8 = 0x08;
const STATUS_ERR: u8 = 0x01;

// Error register bits (we only model ABRT for now).
const ERR_ABRT: u8 = 0x04;
/// ATAPI Error register for a CD_READ the card failed under: sense key 3
/// (MEDIUM ERROR) in the high nibble, plus ABRT.
const ERR_MEDIUM: u8 = 0x30 | ERR_ABRT;

// ATA / ATAPI command opcodes we handle (or explicitly ABRT).
const CMD_NOP: u8 = 0x00;
const CMD_DEVICE_RESET: u8 = 0x08;
const CMD_PACKET: u8 = 0xA0;
const CMD_IDENTIFY_PACKET: u8 = 0xA1;
const CMD_IDENTIFY: u8 = 0xEC;
const CMD_SET_FEATURES: u8 = 0xEF;
const CMD_FLUSH_CACHE: u8 = 0xE7;
const CMD_FLUSH_CACHE_EXT: u8 = 0xEA;

const STATUS_IDLE: u8 = STATUS_DRDY | STATUS_DSC;
const STATUS_IDLE_ERR: u8 = STATUS_DRDY | STATUS_DSC | STATUS_ERR;

/// Wrapper around one of the two FPGA-emulated IDE devices.
#[derive(Clone, Copy)]
pub struct IdeDevice {
    pub(crate) base: *mut u16,
    /// Words this device's read FIFO can hold. The two devices are built with
    /// different depths (`READ_FIFO_DEPTH` = 4096, `SECONDARY_READ_FIFO_DEPTH`
    /// = 1024 in hw/ide_device.py) and this used to be one global constant set
    /// to the primary's 4096. The secondary was 256 words then (one sector;
    /// now four), so that told `push_buf_to_fifo` there were thousands of
    /// words of room, it pushed a whole 512-byte block into less space than
    /// that and the overflow was dropped on the floor. The host then ran the FIFO
    /// dry before the end of the sector, read a word that was never serviced,
    /// and every word after it in the transfer was shifted by one.
    read_fifo_depth: u16,
}

unsafe impl Send for IdeDevice {}

impl IdeDevice {
    pub fn primary() -> Self {
        Self { base: smc::fpga_window(), read_fifo_depth: PRIMARY_FIFO_DEPTH_WORDS }
    }

    pub fn secondary() -> Self {
        // Secondary device window is at MCU_ADDR[7]=1 → +0x80 words.
        Self {
            base: unsafe { smc::fpga_window().add(0x80) },
            read_fifo_depth: SECONDARY_FIFO_DEPTH_WORDS,
        }
    }

    /// Words this device's read FIFO holds. Must match the depth the FPGA was
    /// built with, or a push overruns it silently.
    #[inline]
    fn fifo_depth_words(&self) -> u16 {
        self.read_fifo_depth
    }

    #[inline]
    fn read(&self, reg: usize) -> u16 {
        unsafe { read_volatile(self.base.add(reg)) }
    }

    #[inline]
    fn write(&self, reg: usize, val: u16) {
        unsafe { write_volatile(self.base.add(reg), val) }
    }

    /// RSTn falling edges seen since the FPGA came up, modulo 256. Slot 16
    /// is the same on either device; only a hard reset moves it.
    fn rstn_falls(&self) -> u8 {
        (self.read(REG_RSTN_EDGES) >> 8) as u8
    }

    fn command_pending(&self) -> bool {
        (self.read(REG_HW_INFO) & HW_CMD_PEND) != 0
    }

    /// The host reset the device since `clear_reset_seen`.
    fn reset_seen(&self) -> bool {
        (self.read(REG_HW_INFO) & HW_RESET_SEEN) != 0
    }

    /// The command being served is dead: the host reset the bus, or wrote a
    /// new command to this device (R89 A1). Only meaningful after the current
    /// command's `ack_command`. Both flags are sticky until the command loop
    /// gets back to them, so a caller may ask after the fact.
    ///
    /// Whoever sees this must stop writing Status, the byte count and the
    /// interrupt reason -- they belong to the new command now, or to the
    /// FPGA's post-reset state -- and post no completion. The FPGA clears its
    /// FIFOs on a Command write but leaves DMA mode and PIO flow control
    /// armed, so `disarm` is the one write still owed. (It is masked to
    /// those bits: the new command may be a PACKET whose CDB phase the FPGA
    /// has already opened in the same slot.)
    ///
    /// A latch with the Command register reading 0x00 is not a command (see
    /// `complete`) and does not count.
    fn aborted(&self) -> bool {
        let hw = self.read(REG_HW_INFO);
        hw & HW_RESET_SEEN != 0
            || (hw & HW_CMD_PEND != 0 && (self.read(REG_CMD_FEAT) & 0xFF) != 0)
    }

    /// DMA mode off and both PIO flow controls released (slot 19 holds all
    /// three, and the packet phase, which is left alone).
    fn disarm(&self) {
        self.write(REG_DMA_MODE, XFER_MASK);
    }

    /// Clear the sticky reset flag without disturbing COMMAND_PEND (slot 0
    /// writes carry both).
    ///
    /// Writing back COMMAND_PEND as read keeps a pending command pending. If
    /// it read 0, the number read with it goes back too, so the write can
    /// only "acknowledge" a latch that is already clear: a command that
    /// arrives between the read and the write has the next number. (On a
    /// bitstream without the number, that command is lost, as it always was.)
    fn clear_reset_seen(&self) {
        let hw = self.read(REG_HW_INFO) & (HW_ACK_TOKEN | HW_CMD_PEND);
        self.write(REG_HW_INFO, hw | HW_RESET_SEEN);
    }

    fn is_primary(&self) -> bool {
        self.base == smc::fpga_window()
    }

    /// The other device on the bus.
    fn peer(&self) -> IdeDevice {
        if self.is_primary() { Self::secondary() } else { Self::primary() }
    }

    /// Take the pending command: its registers, and the number
    /// `ack_command` will acknowledge it by.
    ///
    /// The number is read on both sides of the snapshot, and the snapshot
    /// retaken until they agree. That makes the pair consistent -- the
    /// registers are the ones written with that Command write, not half the
    /// next one's -- and it is also what makes a single read of the number
    /// safe to trust: the FPGA changes it on its own clock, so one read that
    /// straddles a change can return bits of both values, but not the same
    /// wrong value twice.
    ///
    /// A command written after the second read has a later number; the
    /// acknowledge then leaves its COMMAND_PEND alone and `sweep` comes
    /// round to it (R89 A-c).
    fn take_command(&self) -> CommandSnapshot {
        let mut token = self.read(REG_HW_INFO) & HW_ACK_TOKEN;
        loop {
            let snap = self.snapshot_command();
            let again = self.read(REG_HW_INFO) & HW_ACK_TOKEN;
            if again == token {
                ACK_TOKEN.store(token, core::sync::atomic::Ordering::Relaxed);
                return snap;
            }
            token = again;
        }
    }

    /// Read the latched command opcode + features + LBA + sector count.
    fn snapshot_command(&self) -> CommandSnapshot {
        let cmd_feat = self.read(REG_CMD_FEAT);
        let low_mid = self.read(REG_LBA_LOW_MID);
        let high_lowp = self.read(REG_LBA_HIGH_LOWP);
        let midp_highp = self.read(REG_LBA_MIDP_HIGHP);
        let count = self.read(REG_SECTOR_COUNT);
        let device = self.read(REG_DEVICE) as u8;
        CommandSnapshot {
            device,
            command: (cmd_feat & 0xFF) as u8,
            features: (cmd_feat >> 8) as u8,
            lba_low: (low_mid & 0xFF) as u8,
            lba_mid: (low_mid >> 8) as u8,
            lba_high: (high_lowp & 0xFF) as u8,
            // The LBA48 shadow half of the sliding window: the host's first
            // write to each register, pushed aside by its second.
            lba_low_prev: (high_lowp >> 8) as u8,
            lba_mid_prev: (midp_highp & 0xFF) as u8,
            lba_high_prev: (midp_highp >> 8) as u8,
            sector_count: count,
        }
    }

    fn set_error(&self, err: u8) {
        self.write(REG_ERROR, err as u16);
    }

    fn set_status(&self, status: u8) {
        self.post_status(status as u16);
    }

    /// Every Status write of a command being served: slot 1, marked with the
    /// number of the command `take_command` took (bit 15, and the number's
    /// low six bits in 9-14 -- `ACK_TOKEN` moved up one, past the INTRQ bit).
    /// The FPGA drops the write, INTRQ included, if the host has written
    /// another command since. Otherwise a handler that had already decided
    /// its status would post it over the new command: BSY down with nothing
    /// served yet, or the DRQ of a PACKET's CDB phase closed. `complete`
    /// looks before it writes, but a look cannot cover the time up to the
    /// write, nor the write strobe itself; the comparison in the FPGA does.
    ///
    /// The dropped write needs no follow-up: the new command is latched and
    /// `sweep` serves it. On a bitstream without the number the token is 0
    /// and this is the plain write; one with the number but without this
    /// check ignores the upper bits.
    #[inline(never)]
    fn post_status(&self, val: u16) {
        let t = ACK_TOKEN.load(core::sync::atomic::Ordering::Relaxed);
        self.write(REG_STATUS, val | (t & 0x8000) | ((t << 1) & 0x7E00));
    }

    /// Post a new status and assert INTRQ in the same FPGA clock (slot 1
    /// bit 8). Every host-visible transition that interrupts -- DRQ up for
    /// a data-in block, command completion -- goes through here, never
    /// through `set_status` followed by `assert_intrq`: the sub-microsecond
    /// gap between those two writes was enough for a host polling Alt
    /// Status to see BSY drop, read Status and write its next command, so
    /// the interrupt then landed on that command as a spurious one. Quake
    /// III's WinCE driver, hit that way, slept through the real DRQ
    /// interrupt of its next small PIO read and dropped to the BIOS menu.
    fn set_status_intrq(&self, status: u8) {
        self.post_status(status as u16 | 0x0100);
    }

    /// Post a command's *completion* (error + status + INTRQ) -- unless the
    /// host has already written its next command, in which case the
    /// completion is dropped and `false` returned. Once a new command is
    /// latched the old one's status is dead (ATA-5 §9.1), and writing it
    /// anyway lands on the new command: for PACKET the FPGA has just opened
    /// the CDB phase (DRQ=1) on the Command write, so a stale 0x50 here
    /// clears DRQ before the host sees it and the CDB never arrives.
    /// g1bios_replay's disc probe does exactly that -- a 1-sector CD_READ it
    /// never drains, then straight into the next PACKET; its Command write
    /// clears our FIFO, which satisfied the drain wait.
    fn complete(&self, error: u8, status: u8) -> bool {
        // COMMAND_PEND can latch without the host having written a command:
        // measured at the end of the DMA read that delivers a MIL-CD's
        // 1ST_READ.BIN, where the flag came up with the Command register
        // reading 0x00 and every other register still holding the finished
        // read's values (byte count 0x3800, SECNR 0x22). Dropping the
        // completion there drops the one the host is still waiting on: its
        // read syscall never returns, the bootstrap never jumps, and the
        // console sits on the disc's licence screen forever.
        //
        // 0x00 is not a command the host issues -- ATA-5 has it as NOP, which
        // nothing in this boot path uses -- so treat it as the spurious latch
        // it is and post the completion anyway. A real command is a non-zero
        // opcode, and for that the guard below still applies.
        if self.command_pending() && (self.read(REG_CMD_FEAT) & 0xFF) == 0 {
            info!(
                "{}: COMMAND_PEND with no command (cmd=0x00); completing anyway",
                if self.is_primary() { "primary" } else { "secondary" }
            );
            self.set_error(error);
            // The plain form: if that latch came with a Command write, the
            // number has moved and a numbered write would be dropped.
            self.write(REG_STATUS, status as u16 | 0x0100);
            return true;
        }
        if self.command_pending() {
            // Log what the "next command" actually is. A real one is a
            // plausible ATA opcode; a command register reading 0x00 means
            // COMMAND_PEND latched without the host writing a command, and
            // the completion we are about to drop was the one the host was
            // still waiting for.
            let cmd_feat = self.read(REG_CMD_FEAT);
            info!("{}: host already issued the next command; completion dropped (hw={:#06x} cmd={:#04x} feat={:#04x} st={:#04x} rfifo={})",
                  if self.is_primary() { "primary" } else { "secondary" },
                  self.read(REG_HW_INFO), cmd_feat & 0xFF, cmd_feat >> 8,
                  self.read(REG_STATUS) & 0xFF, self.read(REG_READ_FIFO_LEVEL));
            return false;
        }
        self.set_error(error);
        self.set_status_intrq(status);
        true
    }

    /// Acknowledge the command `take_command` took: clear its COMMAND_PEND,
    /// so the FPGA releases MCU_IRQn.
    ///
    /// Only that command's. This used to write 0, which cleared the latch
    /// whatever it stood for, and the handlers that post Status first and
    /// acknowledge second gave the host exactly the opening: it sees BSY
    /// drop and writes its next command before the acknowledge lands. That
    /// command was then never served -- BSY up, no latch, no interrupt edge
    /// -- until the host timed out (R89 A-c). The FPGA now compares the
    /// number written here with the latest Command write's and keeps the
    /// latch if they differ, so the command stays pending for `sweep`.
    /// Calling this twice, or late, is harmless for the same reason.
    #[inline(never)]
    fn ack_command(&self) {
        self.write(REG_HW_INFO, ACK_TOKEN.load(core::sync::atomic::Ordering::Relaxed));
    }

    /// Pulse the IDE-side INTRQ line. Real ATAPI drives INTRQ high
    /// when the device wants the host's attention (data ready, command
    /// done); the host clears it by reading the STATUS register —
    /// which the FPGA observes and clears the latched flop. Without
    /// this the BIOS waits on the IRQ pin for a transition that never
    /// happens and times out.
    #[allow(dead_code)]
    fn assert_intrq(&self) {
        self.write(REG_INTRQ_PENDING, 0x0001);
    }

    /// Arm or disarm DMA mode in the FPGA. While armed (1), the FPGA
    /// asserts DMARQ whenever the read FIFO has at least one word and
    /// this device is selected — driving the host's DMA controller
    /// through the data-in phase. Disarm (0) once the host has drained
    /// the transfer.
    /// Arm the FPGA's DMA path in a direction, or disarm it.
    ///
    /// The direction matters because DMARQ means opposite things each way:
    /// reading it says "I have data", writing it says "I have room".
    ///
    /// Reading, the FPGA only raises DMARQ with a whole 16-word Holly burst
    /// in the FIFO (a grant with fewer let the host outrun the refill and
    /// read stale words). So data staged for a DMA read must total a multiple
    /// of 32 bytes or its tail never drains -- true of every whole-sector
    /// path today; pad here if a caller ever stages anything else.
    ///
    /// This sets the whole transfer mode: PIO flow control goes off with it.
    /// A command moves its data by DMA or by PIO, never both, so the two are
    /// one setting with five values, and writing it as one is what clears a
    /// leftover from a transfer that ended without its own cleanup. (The
    /// FPGA can write the fields separately -- the mask in bits 8-12 -- if a
    /// caller ever needs that.) The packet phase bit is not touched.
    fn set_dma_mode(&self, mode: DmaMode) {
        self.write(REG_DMA_MODE, XFER_MASK | mode as u16);
    }

    /// Arm the fabric capture: it triggers on the next matching event and
    /// records until full. No-op unless the FPGA was built with capture=True.
    fn capture_arm(&self) {
        self.write(REG_CAP_CTRL, 1);
    }

    /// Dump whatever was recorded, complete or not -- a partial capture is
    /// exactly what a stalled transfer produces.
    fn capture_dump(&self, label: &'static str) {
        let ctrl = self.read(REG_CAP_CTRL);
        // Bit 2 means the buffer filled. The event field is wptr >> 1 in eight
        // bits, so a full 512-entry capture reports 256 there and wraps to 0 --
        // trust the done bit over the count.
        let count = if ctrl & 0x04 != 0 {
            CAP_DEPTH
        } else {
            ((ctrl >> 8) as usize) * 2
        };
        info!("{}: capture ctrl={:#06x} events={}", label, ctrl, count);
        // Always read a few entries even when the count field says zero: it is
        // wptr >> 1, so a single captured event reports 0 and would otherwise
        // be invisible. cap_run still being set means it triggered.
        let count = count.max(8);
        for i in 0..count.min(CAP_DEPTH) {
            self.write(REG_CAP_DATA, i as u16);
            let v = self.read(REG_CAP_DATA);
            if i < 16 || i + 2 >= count {
                info!(
                    "  i={} byte={:#04x} addr={} csn={} iscmd={} bsy={} drq={}",
                    i, v & 0xFF, (v >> 8) & 7, (v >> 11) & 3,
                    (v >> 13) & 1, (v >> 14) & 1, (v >> 15) & 1,
                                );
            }
        }
    }

    /// Arm hardware backpressure for a PIO data phase, or release it. For
    /// data-out: while set, the FPGA drops the host-visible DRQ whenever the
    /// write FIFO is full. Without it a multi-sector WRITE SECTORS overruns
    /// the FIFO -- the MCU polls far too slowly to drop DRQ in time by
    /// itself. For data-in: DRQ shows only while the read FIFO has data. Like
    /// `set_dma_mode` this sets the whole transfer mode (DMA goes off) and
    /// leaves the packet phase bit alone.
    fn set_pio_flow_control(&self, read: bool, write: bool) {
        self.write(
            REG_DMA_MODE,
            XFER_MASK | (if read { 8 } else { 0 }) | (if write { 4 } else { 0 }),
        );
    }

    /// Update the ATAPI Interrupt Reason register (lives in the
    /// physical sector-count register, low byte). Bits:
    ///   bit 0 (C/D): 1 = command/packet phase, 0 = data
    ///   bit 1 (I/O): 1 = device→host (data-in), 0 = host→device
    ///   bit 2 (REL): bus release (we don't use)
    /// Standard combinations:
    ///   0x01 — host is writing the 12-byte packet to us
    ///   0x02 — we have data-in ready for the host
    ///   0x03 — command finished, host should read STATUS
    ///
    /// The FPGA takes the low byte only from an MCU write to this slot; the
    /// high byte (the host's previous sector count, LBA48) is not ours. A
    /// bitstream from before that zeroes it here, which no host of the
    /// packet device reads back.
    fn set_irr(&self, ir: u8) {
        self.write(REG_SECTOR_COUNT, ir as u16);
    }

    /// Force the Sega GD-ROM signature back into LBA_LOW/MID/HIGH and
    /// the sector count — used by DEVICE_RESET (soft reset). Matches
    /// the values the FPGA loads in its hardware reset state machine.
    fn load_atapi_signature(&self) {
        // LBA_LOW = disc format | status 0x02 (standby), LBA_MID=0x14,
        // (the FPGA's own reset value is 0x81 -- GD-ROM | pause -- not 0x82;
        // see `reassert_disc_format`.)
        // packed into addr 6. The format follows the mounted image rather
        // than being pinned to GD-ROM: the FPGA's reset state machine can
        // only load one fixed value, so a MIL-CD's CD-ROM XA format has to
        // be (re)asserted from here.
        self.write(REG_LBA_LOW_MID, 0x1400 | secnr(GD_STANDBY) as u16);
        // LBA_HIGH=0xEB low byte (LBA_LOW_PREV in high byte we leave alone).
        let high_lowp = self.read(REG_LBA_HIGH_LOWP);
        self.write(REG_LBA_HIGH_LOWP, (high_lowp & 0xFF00) | 0x00EB);
        self.write(REG_SECTOR_COUNT, 0x0001);
    }

    /// Set the GD-ROM "SECNR" byte the host reads back in Sector Count
    /// after a command: disc type in the high nibble, drive status low.
    /// Lives in the LBA_LOW slot's low byte; the byte-count half is kept.
    pub fn set_secnr(&self, v: u8) {
        let low_mid = self.read(REG_LBA_LOW_MID) & 0xFF00;
        self.write(REG_LBA_LOW_MID, low_mid | v as u16);
    }

    /// Re-apply the mounted disc's format to SECNR, keeping the drive status
    /// nibble that is currently latched.
    ///
    /// The FPGA's registers reset to a fixed GD-ROM signature -- LBA_LOW is
    /// `8'h81` in `fpga/amaranth/out/OpenGDEMUCore.v`, reloaded by its reset
    /// handling -- so every host bus reset silently reverts a MIL-CD's
    /// CD-ROM XA format to GD-ROM. Nothing used to put it back: the format is
    /// otherwise only written at `ide_task` startup and by DEVICE RESET, and a
    /// host *bus* reset is neither. The console then saw a disc that had
    /// announced itself as a CD go back to claiming GD-ROM mid-boot, which is
    /// the contradiction it refuses to boot across. Only the format nibble is
    /// touched, because the status nibble is the FPGA's to choose.
    fn reassert_disc_format(&self) {
        let low_mid = self.read(REG_LBA_LOW_MID);
        let status = (low_mid & 0x0F) as u8;
        self.write(REG_LBA_LOW_MID, (low_mid & 0xFF00) | (disc_format() | status) as u16);
    }

    /// Words in the write FIFO (host -> MCU), for "has the host sent enough".
    ///
    /// The SMC samples the FPGA's counter asynchronously, so one read can be
    /// torn across a carry (3 -> 4 read as 7, 255 -> 256 as 511) and claim
    /// words that are not there yet (R89 A-a). The lower of two reads cannot:
    /// a tear in both, in the same direction, would take two carries inside
    /// two bus cycles.
    fn write_fifo_level(&self) -> u16 {
        self.read(REG_WRITE_FIFO_LEVEL).min(self.read(REG_WRITE_FIFO_LEVEL))
    }

    #[allow(dead_code)]
    fn read_fifo_level(&self) -> u16 {
        self.read(REG_READ_FIFO_LEVEL)
    }

    /// Words in the read FIFO (MCU -> host), for "how much room is there":
    /// the higher of two reads, for the reason `write_fifo_level` takes the
    /// lower. A level torn low (0x800 -> 0x7FF read as 0) would report room
    /// that is not there, and the FPGA does not refuse a push into a full
    /// FIFO.
    fn read_fifo_level_hi(&self) -> u16 {
        self.read(REG_READ_FIFO_LEVEL).max(self.read(REG_READ_FIFO_LEVEL))
    }

    /// Log what the host has done on the bus since the last Command
    /// write: register reads, the live status byte it would see, whether
    /// INTRQ is still pending and the last Device Control write.
    fn log_host_activity(&self, label: &str) {
        let st = self.read(REG_HOST_STATUS_RDS);
        let data = self.read(REG_HOST_DATA_RDS);
        let pst = self.read(REG_HOST_PREV_STATUS_RDS);
        let pdata = self.read(REG_HOST_PREV_DATA_RDS);
        let dc = self.read(REG_HOST_DEVCTL);
        let hw = self.read(REG_HW_INFO);
        info!(
            "{}: host status_rds={} altstat_rds={} data_rds={} (prev cmd {}/{}/{}) status={:#04x} intrq={} nIEN={} devctl={:#04x} x{} rfifo={}",
            label, st & 0xFF, st >> 8, data, pst & 0xFF, pst >> 8, pdata,
            self.read(REG_STATUS) & 0xFF,
            self.read(REG_INTRQ_PENDING) & 1,
            (hw & HW_NIEN) != 0,
            dc & 0xFF, dc >> 8,
            self.read(REG_READ_FIFO_LEVEL),
        );
    }

    /// Pop one 16-bit word from the IDE-side write FIFO (data the host
    /// pushed into the Data register, e.g. an ATAPI command packet).
    fn pop_write_fifo(&self) -> u16 {
        self.read(REG_DATA_FIFO)
    }

    /// Push one 16-bit word into the IDE-side read FIFO (data the host
    /// will pop on its next Data register read).
    fn push_read_fifo(&self, word: u16) {
        self.write(REG_DATA_FIFO, word);
    }

    /// Return the host-requested ATAPI byte count (Cylinder Low / High,
    /// our LBA_MID / LBA_HIGH). Hosts use this to ask for a specific
    /// transfer length on data-in/data-out PIO transfers.
    #[allow(dead_code)]
    fn atapi_byte_count(&self) -> u16 {
        let low_mid = self.read(REG_LBA_LOW_MID);
        let high_lowp = self.read(REG_LBA_HIGH_LOWP);
        let mid = (low_mid >> 8) as u8;
        let high = (high_lowp & 0xFF) as u8;
        ((high as u16) << 8) | (mid as u16)
    }

    fn set_atapi_byte_count(&self, count: u16) {
        // LBA_MID = byte count low, LBA_HIGH = byte count high. Preserve
        // the LBA_LOW byte (low byte of addr 6) and LBA_LOW_PREV (high
        // byte of addr 7) so we don't disturb the sliding-window state.
        let low_mid = self.read(REG_LBA_LOW_MID) & 0x00FF;
        self.write(REG_LBA_LOW_MID, low_mid | (((count & 0xFF) as u16) << 8));
        let high_lowp = self.read(REG_LBA_HIGH_LOWP) & 0xFF00;
        self.write(REG_LBA_HIGH_LOWP, high_lowp | ((count >> 8) & 0xFF));
    }
}

#[derive(defmt::Format)]
struct CommandSnapshot {
    /// Device/Head register. Bit 6 = LBA mode, bits 3..0 = LBA[27:24].
    device: u8,
    command: u8,
    features: u8,
    lba_low: u8,
    lba_mid: u8,
    lba_high: u8,
    /// LBA[31:24] under LBA48 -- the FPGA's sliding-window shadow of
    /// LBA_LOW, holding the host's *first* of its two writes.
    lba_low_prev: u8,
    /// LBA[39:32].
    lba_mid_prev: u8,
    /// LBA[47:40].
    lba_high_prev: u8,
    /// Full 16 bits. LBA28 commands use only the low byte.
    sector_count: u16,
}

impl CommandSnapshot {
    /// The 28-bit address, whose top nibble comes from the Device register.
    fn lba28(&self) -> u64 {
        ((self.device as u64 & 0x0F) << 24)
            | ((self.lba_high as u64) << 16)
            | ((self.lba_mid as u64) << 8)
            | (self.lba_low as u64)
    }

    /// The 48-bit address. The Device register carries no address bits here;
    /// the top three bytes are the shadow registers instead.
    fn lba48(&self) -> u64 {
        ((self.lba_high_prev as u64) << 40)
            | ((self.lba_mid_prev as u64) << 32)
            | ((self.lba_low_prev as u64) << 24)
            | ((self.lba_high as u64) << 16)
            | ((self.lba_mid as u64) << 8)
            | (self.lba_low as u64)
    }

    /// Starting LBA for this command, in whichever mode it was issued.
    fn lba(&self, ext: bool) -> u64 {
        if ext { self.lba48() } else { self.lba28() }
    }

    /// Sectors requested. Zero means the maximum, which differs between the
    /// two modes: 256 for LBA28's one byte, 65536 for LBA48's two.
    fn count(&self, ext: bool) -> u32 {
        if ext {
            if self.sector_count == 0 { 65536 } else { self.sector_count as u32 }
        } else if self.sector_count & 0xFF == 0 {
            256
        } else {
            (self.sector_count & 0xFF) as u32
        }
    }
}

/// Long-running task that services both IDE devices' command queues.
/// `gdi` is the GD-ROM image map for the primary device; the secondary
/// (HDD) currently ABRTs CD_READ.
///
/// The FPGA OR-merges per-device `command_pend` (with a 4-cycle delay)
/// onto a single active-low `MCU_IRQn` line. We drive this off
/// `embassy_atsam3::gpio::pioa_wait_irq()` — falling-edge interrupt on
/// PA22 — so the task is event-driven rather than polling. After each
/// wake-up we sweep both devices in case both arrived in the same edge.

/// Per-device state we keep on the MCU side (the FPGA register file
/// only models the wire-level register file; ATAPI semantics like
/// UNIT_ATTENTION pending live here).
struct DeviceState {
    /// Set on power-up and on hardware reset (DC_RSTn pulse). The first
    /// PACKET command other than REQUEST_SENSE/REQ_ERROR completes with
    /// CHECK_CONDITION, sense key 6 (UNIT_ATTENTION), per ATAPI; the
    /// host clears it by issuing REQ_SENSE next.
    unit_attention: bool,
    /// Sense key to report on the next REQ_SENSE / REQ_ERROR. Reflects
    /// the most-recent CHECK_CONDITION cause (6 = UA after reset, 0 =
    /// nothing pending). Reset to 0 once REQ_SENSE drains it.
    pending_sense_key: u8,
    /// ASC / ASCQ that go with `pending_sense_key`, for REQ_ERROR bytes 8/9
    /// (stock GDEMU 6.0 fills them, D7). Set and cleared with the key; 0 where
    /// stock's value is not known (MEDIUM ERROR, ILLEGAL REQUEST).
    pending_asc: (u8, u8),
}

impl DeviceState {
    const fn new() -> Self {
        Self {
            unit_attention: true,
            pending_sense_key: 0x06, // UNIT_ATTENTION pending at boot
            pending_asc: discimg::atapi::ASC_MEDIUM_CHANGED, // the boot mount
        }
    }

    /// UNIT ATTENTION, medium changed (6/28/00, what stock reports after a
    /// mount), for the next command to serve.
    fn medium_changed(&mut self) {
        self.unit_attention = true;
        self.pending_sense_key = 0x06;
        self.pending_asc = discimg::atapi::ASC_MEDIUM_CHANGED;
    }

    /// NOT READY: 2/3A/00 with no medium, 2/04/01 while it becomes ready
    /// (the swap window), as stock reports them (D7).
    fn not_ready(&mut self) {
        self.pending_sense_key = 0x02;
        self.pending_asc = if media_present() {
            discimg::atapi::ASC_BECOMING_READY
        } else {
            discimg::atapi::ASC_NO_MEDIUM
        };
    }

    /// Nothing pending.
    fn clear_sense(&mut self) {
        self.pending_sense_key = 0;
        self.pending_asc = (0, 0);
    }
}

#[embassy_executor::task]
pub async fn ide_task(
    primary: IdeDevice,
    secondary: IdeDevice,
) {
    info!("IDE task started (gdi: {})", gdi_ref().is_some());
    // The interlock starts out as what the boot mount actually achieved. It
    // used to start `true` whatever happened, so a card with no usable image
    // answered TEST UNIT READY with "ready" and refused disk writes with
    // "eject first" over an empty drive (R89 A4).
    MEDIA_PRESENT.store(gdi_ref().is_some(), core::sync::atomic::Ordering::Relaxed);

    // Assert the mounted disc's format before serving anything. The FPGA's
    // reset state machine loads a fixed GD-ROM SECNR, and until now nothing
    // but the CDDA paths ever wrote the register -- so a MIL-CD was served
    // to a host that had been told it was looking at a GD-ROM.
    primary.set_secnr(secnr(GD_STANDBY));
    info!("disc format: SECNR=0x{:02x}", secnr(GD_STANDBY));

    let mut primary_state = DeviceState::new();
    let mut secondary_state = DeviceState::new();
    RSTN_FALLS_SEEN.store(primary.rstn_falls(), core::sync::atomic::Ordering::Relaxed);

    // Always do a sweep at startup in case the FPGA already had
    // `command_pend` set when we attached (e.g. host issued a command
    // during MCU init).
    sweep(&primary, &mut primary_state, "primary", Role::Packet).await;
    sweep(&secondary, &mut secondary_state, "secondary", Role::Disk).await;

    loop {
        // The FPGA's command line, or a button press: the button signals
        // the same waker rather than having this loop poll, so an idle bus
        // stays idle and a swap still lands immediately.
        let _ = embassy_atsam3::gpio::pioa_wait_irq().await;
        service_hard_reset(&primary, &mut primary_state).await;
        // Until neither has a command waiting: MCU_IRQn is the OR of the two
        // latches and the interrupt is its falling edge, so a command that
        // reaches the primary while the secondary's is still being served
        // makes no new edge and would sit until the next one (R89 A-c).
        loop {
            sweep(&primary, &mut primary_state, "primary", Role::Packet).await;
            sweep(&secondary, &mut secondary_state, "secondary", Role::Disk).await;
            if !primary.command_pending() && !secondary.command_pending() {
                break;
            }
        }
        service_swap_request(&mut primary_state, "primary").await;
    }
}

/// RSTn falling edges already accounted for (low 8 bits of slot 16's high byte).
static RSTN_FALLS_SEEN: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// After a *hard* reset (the G1 RSTn line, not SRST) while the media is
/// ejected, mount it again so the console can reboot to the menu without a
/// power cycle.
///
/// The hard reset is told from SRST by the RSTn falling-edge counter: SRST,
/// which a host may issue to either device, never moves it, and
/// remounting then would re-mount the GD image under a write app that has
/// just initialised the disk. The counter is a free-running 8-bit count, so
/// a reset cannot be missed between two looks (only 256 of them could).
/// Runs before the sweeps so their reset handling re-asserts the disc format
/// of the freshly mounted image.
async fn service_hard_reset(primary: &IdeDevice, state: &mut DeviceState) {
    let falls = primary.rstn_falls();
    if RSTN_FALLS_SEEN.swap(falls, core::sync::atomic::Ordering::Relaxed) == falls {
        return;
    }
    // No NOT READY window survives a reset: the host is rebooting and a
    // stale `READY_AT` (see `APPLY_SWAP_WINDOW`) would keep answering the
    // booting BIOS with NOT READY on the paths that do not remount.
    unsafe { READY_AT = None };
    // A write session does not survive a hard reset (never SRST: that does
    // not move the counter). Ending it first leaves the drive ejected, so
    // the remount below is the same one F5 does.
    if wsession().hard_reset() {
        info!("primary: hard reset ended the write session");
    }
    // GDEMU.INI `reset_goto=N`: a host hard reset switches to image N. An N
    // with no image leaves the media as it is. The file may have been
    // rewritten while the card was ejected for a write session (or by hand,
    // between boots), and `mount_named` does not re-read it: do it here.
    crate::gdi::reload_ini(disk_sectors());
    let goto = crate::gdi::ini().reset_goto;
    if goto > 0 {
        if media_present() && crate::gdi::mounted_number() == Some(goto) {
            return;
        }
        match crate::gdi::folder_for_number(disk_sectors(), goto) {
            Some(name) if crate::gdi::folder_has_image(disk_sectors(), name.as_str()) => {
                info!("primary: hard reset -- reset_goto image {}", goto);
                let ok = mount_named(state, "primary", name.as_str()).await;
                // The host is rebooting anyway: no tray-cycle window.
                unsafe { READY_AT = None };
                if ok {
                    return;
                }
            }
            _ => warn!("primary: reset_goto {} has no image, leaving the media", goto),
        }
    }
    if media_present() {
        return;
    }
    info!("primary: hard reset while ejected -- mounting the selected image again");
    let ok = load_media(state).await;
    info!("primary: remounted after hard reset (mounted: {})", ok);
}

/// Which kind of device an `IdeDevice` presents itself as.
///
/// The two share a bus, and the host tells them apart by the signature in
/// LBA_LOW/MID/HIGH plus which IDENTIFY they answer. A packet device ABRTs
/// IDENTIFY DEVICE so the host retries with IDENTIFY PACKET DEVICE; a plain
/// ATA disk does the opposite. Getting this backwards means the host either
/// never finds the disk or tries to send it ATAPI packets.
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum Role {
    /// GD-ROM: ATAPI packet device, signature 0x81/0x14/0xEB.
    Packet,
    /// Hard disk: plain ATA, signature 0x01/0x00/0x00.
    Disk,
}

async fn sweep(
    dev: &IdeDevice,
    state: &mut DeviceState,
    label: &'static str,
    role: Role,
) {
    // Drain all currently-pending commands. The IRQ asserts while
    // command_pend is set, but we ack as part of `handle`, so the
    // line goes high again after each — re-checking here also covers
    // the race where a second command arrives before we returned to
    // the wait. That includes one written before `handle` acknowledged:
    // the acknowledge is by number and leaves that command's latch set,
    // so MCU_IRQn never rises and there is no edge -- only this level.
    while dev.command_pending() {
        // Command turnaround timing. The host arms a transfer and then polls
        // for DRQ with a finite patience; if we take longer than that to
        // respond it gives up and retries, and the retry's BSY is what both
        // sides then see. These two numbers say whether the MCU was late and,
        // if so, whether it was busy with the previous command or slow on
        // this one.
        let cmd_at = Instant::now();
        let gap_us = unsafe { LAST_CMD_DONE }
            .map(|t: Instant| (cmd_at - t).as_micros())
            .unwrap_or(0);
        unsafe { CMD_SEEN_AT = Some(cmd_at) };

        let snap = dev.take_command();
        // A reset between commands is normal (the BIOS does one at boot);
        // one that landed mid-transfer already aborted the wait that saw
        // it. Either way, log it and clear the flag before the handler runs
        // so a wait inside it only trips on a *new* reset.
        {
            let hw = dev.read(REG_HW_INFO);
            if hw & HW_RESET_SEEN != 0 {
                info!("{}: host reset seen (nIEN={}, in_reset={})", label,
                      hw & HW_NIEN != 0, hw & HW_IN_RESET != 0);
                dev.clear_reset_seen();
                crate::cdda::stop();
                // The FPGA reloaded its fixed GD-ROM signature; put the
                // mounted disc's real format back before answering anything.
                dev.reassert_disc_format();
                info!("{}: disc format re-asserted after reset: SECNR low nibble kept, fmt={:#04x}",
                      label, disc_format());
            }
        }
        // Mirror command snapshot to UART — defmt/RTT goes away when
        // the DC power-cycles the board, but UART stays alive for the
        // host's screen/cat session. Keep this short so tail logs are
        // readable.
        info!(
            "{}: cmd 0x{:02x} feat 0x{:02x} sc {} lba {:02x}{:02x}{:02x}",
            label,
            snap.command,
            snap.features,
            snap.sector_count,
            snap.lba_high,
            snap.lba_mid,
            snap.lba_low,
        );
        handle(dev, state, label, role, &snap).await;
        let done = Instant::now();
        info!("{}: cmd {:#04x} gap={}us total={}us", label, snap.command,
               gap_us, (done - cmd_at).as_micros());
        dev.log_host_activity(label);
        unsafe { LAST_CMD_DONE = Some(done) };
    }
}

async fn handle(
    dev: &IdeDevice,
    state: &mut DeviceState,
    label: &'static str,
    role: Role,
    snap: &CommandSnapshot,
) {
    info!("{}: cmd={:?}", label, snap);
    match snap.command {
        CMD_NOP => {
            // ATA NOP. Real-world drives just succeed; the BIOS issues
            // it as a kind of "are you alive" probe. Returning ABRT
            // makes the BIOS retry forever.
            dev.set_error(0);
            dev.set_status_intrq(STATUS_IDLE);
            dev.ack_command();
        }
        // The secondary only: every write command finishes its CMD25 and the
        // card's busy wait before it completes, so there is nothing to flush.
        // Mirrors NOP. The primary keeps ABRTing these, as before.
        CMD_FLUSH_CACHE | CMD_FLUSH_CACHE_EXT if role == Role::Disk => {
            dev.set_error(0);
            dev.set_status_intrq(STATUS_IDLE);
            dev.ack_command();
        }
        // DEVICE RESET and PACKET belong to the packet device. The disk ABRTs
        // them below, as ATA has a non-packet device do: it used to load the
        // ATAPI signature into the disk's registers and stop CD audio on
        // 0x08, and hold BSY for ten seconds waiting for a CDB on 0xA0 (the
        // FPGA only opens the CDB phase on the primary) (R89 A7).
        CMD_DEVICE_RESET if role == Role::Packet => {
            crate::cdda::stop();
            // Soft reset: reload ATAPI signature, return to idle, no error.
            dev.load_atapi_signature();
            dev.set_error(0);
            dev.set_status_intrq(STATUS_IDLE);
            dev.ack_command();
        }
        CMD_SET_FEATURES => {
            // We don't actually honor any feature bits yet — just ack so
            // the host's PIO-mode set / etc. can proceed.
            dev.set_error(0);
            dev.set_status_intrq(STATUS_IDLE);
            dev.ack_command();
        }
        CMD_IDENTIFY => match role {
            // ATA spec: a packet device shall ABRT IDENTIFY DEVICE so the
            // host falls back to IDENTIFY PACKET DEVICE.
            Role::Packet => {
                dev.set_error(ERR_ABRT);
                dev.set_status_intrq(STATUS_IDLE_ERR);
                dev.ack_command();
            }
            // A disk answers it -- this is how DreamShell / KOS discover an
            // IDE hard disk on the G1 bus.
            Role::Disk => {
                stage_identify_disk(dev);
                dev.set_error(0);
                dev.set_status_intrq(STATUS_DRDY | STATUS_DSC | STATUS_DRQ);
                dev.ack_command();
                // A PIO data-in command is not finished when the data is
                // staged -- DRQ has to drop once the host has taken it all,
                // or a host that polls for "DRQ clear, BSY clear" before
                // issuing the next command waits forever.
                finish_pio_in(dev).await;
            }
        },
        CMD_IDENTIFY_PACKET => match role {
            // The stock GDEMU 6.0 block: 80 bytes (40 words), not ATA's 512
            // (D16). Stage it, then signal DRQ=1 so the host can start
            // reading.
            Role::Packet => {
                stage_identify_packet(dev);
                dev.set_atapi_byte_count(discimg::atapi::IDENTIFY_PACKET_REPLY.len() as u16);
                dev.set_error(0);
                dev.set_status_intrq(STATUS_DRDY | STATUS_DSC | STATUS_DRQ);
                dev.ack_command();
                // Drop DRQ once the host has the 80 bytes (stock: status
                // 0x50 after them), as the disk IDENTIFY does: a host that
                // reads while DRQ is up (gdprobe) would otherwise read on
                // past the block.
                finish_pio_in(dev).await;
            }
            // Mirror image: a non-packet device ABRTs the packet IDENTIFY.
            Role::Disk => {
                dev.set_error(ERR_ABRT);
                dev.set_status_intrq(STATUS_IDLE_ERR);
                dev.ack_command();
            }
        },
        CMD_WRITE_SECTORS | CMD_WRITE_DMA | CMD_WRITE_SECTORS_EXT | CMD_WRITE_DMA_EXT
            if role == Role::Disk =>
        {
            let dma = matches!(snap.command, CMD_WRITE_DMA | CMD_WRITE_DMA_EXT);
            let ext = matches!(snap.command, CMD_WRITE_SECTORS_EXT | CMD_WRITE_DMA_EXT);
            handle_disk_write(dev, label, snap, dma, ext).await;
        }
        CMD_READ_SECTORS | CMD_READ_DMA | CMD_READ_SECTORS_EXT | CMD_READ_DMA_EXT
            if role == Role::Disk =>
        {
            let dma = matches!(snap.command, CMD_READ_DMA | CMD_READ_DMA_EXT);
            let ext = matches!(snap.command, CMD_READ_SECTORS_EXT | CMD_READ_DMA_EXT);
            handle_disk_read(dev, label, snap, dma, ext).await;
        }
        crate::fwupdate::CMD_FW if role == Role::Disk => {
            handle_fw_command(dev, label, snap);
        }
        CMD_PACKET if role == Role::Packet => {
            handle_packet(dev, state, label).await;
            // ack_command happens inside handle_packet after the packet
            // protocol fully completes.
        }
        other => {
            warn!("{}: unhandled cmd 0x{:02x}", label, other);
            dev.set_error(ERR_ABRT);
            dev.set_status_intrq(STATUS_IDLE_ERR);
            dev.ack_command();
        }
    }
}

// ---------------------------------------------------------------------------
// PACKET (0xA0) command — ATAPI packet protocol.
//
// Sequence:
//   1. Host writes 0xA0 to the Command register. FPGA latches command_pend
//      and, for the primary (ATAPI) device, opens the CDB phase itself:
//      IRR=0x01, DRQ=1, BSY=0 within one core clock.
//   2. We just acknowledge the command latch.
//   3. Host writes 6 16-bit words to the Data register. FPGA pushes them
//      into write_fifo. We poll write_fifo level until it hits 6.
//   4. We pop the 12-byte packet, parse opcode = byte 0.
//   5. For data-in commands: stage response into read_fifo, set
//      byte-count registers, DRQ=1, BSY=0. Host PIO-reads until done.
//   6. We clear DRQ, leave STATUS=0x50 idle. ack_command() releases the
//      command_pend latch so the next command can land.
// ---------------------------------------------------------------------------

const PACKET_LEN_WORDS: usize = 6;
const PACKET_LEN_BYTES: usize = 12;

// ATAPI command opcodes (subset).
const CMD_READ_SECTORS: u8 = 0x20;
const CMD_READ_DMA: u8 = 0xC8;
const CMD_WRITE_SECTORS: u8 = 0x30;
const CMD_WRITE_DMA: u8 = 0xCA;
/// The LBA48 forms. Same transfers, but the address comes from the sliding
/// window instead of the Device register, and the sector count is 16 bits.
const CMD_READ_SECTORS_EXT: u8 = 0x24;
const CMD_READ_DMA_EXT: u8 = 0x25;
const CMD_WRITE_SECTORS_EXT: u8 = 0x34;
const CMD_WRITE_DMA_EXT: u8 = 0x35;

/// Direction of an armed DMA transfer, as the FPGA's slot-19 register encodes it.
#[derive(Copy, Clone, PartialEq, Eq)]
#[repr(u16)]
enum DmaMode {
    Off = 0,
    /// Device to host: CD_READ, disk reads.
    ToHost = 1,
    /// Host to device: disk writes.
    ToDevice = 2,
}

/// The largest sector count a 28-bit address can name. Anything past this is
/// reachable only through the LBA48 command set.
const MAX_LBA28_SECTORS: u32 = 0x1000_0000;

/// Addressable sectors on the emulated disk, published in IDENTIFY. Set once
/// the SD card is up; the whole card is exposed, MBR included, so the host
/// sees a normal partitioned disk and can mount the same filesystem that
/// holds the GD-ROM images.
static DISK_SECTORS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

pub fn set_disk_sectors(n: u32) {
    DISK_SECTORS.store(n, core::sync::atomic::Ordering::Relaxed);
}

/// Size the emulated disk from the card's partition table, falling back to
/// `csd_blocks` if there is no usable MBR.
///
/// The CSD-derived figure is not trustworthy here: on this card it reports
/// 2,688,000 blocks (1.3 GB) for what the partition table says is a 500 GB
/// volume -- a ~360x undercount, most likely a CSD v2 bit-alignment problem in
/// the HSMCI response registers. Since disk reads range-check against this
/// number, believing it made everything past LBA 2,688,000 unreachable while
/// still appearing to work, because the root directory happens to land below
/// the cut. Take the end of the furthest partition instead; main.rs already
/// noted the partition table is the authoritative source.
pub fn size_disk_from_mbr(csd_blocks: u32) {
    let mut mbr = [0u8; 512];
    if unsafe { embassy_atsam3::hsmci::read_block(0, &mut mbr) }.is_err() {
        warn!("disk sizing: MBR read failed, using CSD value {}", csd_blocks);
        set_disk_sectors(csd_blocks);
        return;
    }
    if mbr[510] != 0x55 || mbr[511] != 0xAA {
        warn!("disk sizing: no MBR signature, using CSD value {}", csd_blocks);
        set_disk_sectors(csd_blocks);
        return;
    }
    // A card with no partition table: sector 0 is the volume's own boot
    // sector, and what sits where the partition entries would be is boot
    // code. Same test as `fat::looks_like_bpb` (private there). The volume
    // is the card, so take its sector count (16-bit field, else 32-bit).
    if (mbr[0] == 0xEB || mbr[0] == 0xE9) && mbr[0x0B] == 0 && mbr[0x0C] == 2 && mbr[0x0D] != 0 {
        let mut n = u16::from_le_bytes([mbr[0x13], mbr[0x14]]) as u32;
        if n == 0 {
            n = u32::from_le_bytes([mbr[0x20], mbr[0x21], mbr[0x22], mbr[0x23]]);
        }
        if n == 0 {
            n = csd_blocks;
        }
        info!("disk sizing: no partition table, {} blocks from the boot sector (CSD said {})",
              n, csd_blocks);
        set_disk_sectors(n);
        return;
    }
    let mut end = 0u32;
    for i in 0..4 {
        let e = &mbr[446 + i * 16..446 + i * 16 + 16];
        if e[4] == 0 {
            continue; // unused entry
        }
        let start = u32::from_le_bytes([e[8], e[9], e[10], e[11]]);
        let count = u32::from_le_bytes([e[12], e[13], e[14], e[15]]);
        end = end.max(start.saturating_add(count));
    }
    if end == 0 {
        warn!("disk sizing: no partitions, using CSD value {}", csd_blocks);
        set_disk_sectors(csd_blocks);
    } else {
        info!("disk sizing: {} blocks from partition table (CSD said {})", end, csd_blocks);
        set_disk_sectors(end);
    }
}

const ATAPI_TEST_UNIT_READY: u8 = 0x00;
const ATAPI_REQUEST_SENSE: u8 = 0x03;
const ATAPI_START_STOP_UNIT: u8 = 0x1B;

/// Whether the GD-ROM currently has media loaded.
///
/// This is the coherency interlock for the shared card. The emulated disk on
/// the secondary exposes the same FAT32 volume the GD-ROM images live on, so
/// the host must be able to tell us "I am about to touch that filesystem" and
/// have us let go first. Ejecting drops the staged-sector cache and makes
/// CD_READ report NOT READY; loading raises a media-change UNIT ATTENTION so
/// the host re-reads the TOC.
static MEDIA_PRESENT: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(true);

/// The currently mounted GD-ROM image.
///
/// Owned here rather than handed in as a `&'static` so it can be replaced: a
/// StaticCell initialises once, which meant an image added while ejected could
/// never be picked up. Only `ide_task` (and `main` before that task spawns)
/// touches this, on a single-threaded executor with no interrupt access.
static mut GDI: Option<crate::gdi::Gdi> = None;

/// Command turnaround instrumentation. Only `ide_task` touches these.
static mut CMD_SEEN_AT: Option<Instant> = None;
static mut LAST_CMD_DONE: Option<Instant> = None;

/// Replace the mounted image. Safe to call before `ide_task` starts or from
/// inside it; nothing else may.
pub fn mount_gdi(g: crate::gdi::Gdi) {
    unsafe { GDI = Some(g) };
}

pub fn gdi_ref() -> Option<&'static crate::gdi::Gdi> {
    unsafe { (*core::ptr::addr_of!(GDI)).as_ref() }
}

/// Prefetch IP.BIN for whatever is currently mounted, if anything.
pub async fn prefetch_mounted() {
    if let Some(g) = gdi_ref() {
        prefetch_ip_bin(g).await;
    }
}

/// Re-read the card and mount whatever image is there now. Called on LOAD so
/// a GDI added or removed while ejected is actually picked up.
async fn rescan_media() -> bool {
    invalidate_cd_read_cache();
    match crate::gdi::load_default(disk_sectors()) {
        Ok(g) => {
            mount_gdi(g);
            if let Some(g) = gdi_ref() {
                prefetch_ip_bin(g).await;
            }
            true
        }
        Err(e) => {
            warn!("rescan_media: no usable image after load: {:?}", e);
            unsafe { GDI = None };
            false
        }
    }
}

/// The LOAD half of a tray cycle: mount whatever is selected now and tell
/// the host the medium may have changed, so it re-reads the TOC rather
/// than trusting what it cached before. Returns whether an image mounted.
async fn load_media(state: &mut DeviceState) -> bool {
    crate::cdda::stop();
    let ok = rescan_media().await;
    unsafe { READY_AT = None };
    MEDIA_PRESENT.store(ok, core::sync::atomic::Ordering::Relaxed);
    // The format nibble follows the mounted image (GDI -> GD-ROM, CDI -> CD-ROM
    // XA); without this a swap between the two shows the old one until reset.
    IdeDevice::primary().reassert_disc_format();
    state.medium_changed();
    ok
}

/// Swap discs without the host's help: eject, mount the selection, load.
/// This is what the GDEMU-style commands imply -- the menu that sends them
/// returns straight to the BIOS, which expects to find the new disc there.
/// A swap the button has asked for, as the folder to mount.
///
/// A name, not an index: which discs exist is the menu's business, and the
/// firmware should not have to enumerate the card to be told "mount that
/// one". The button cannot perform the swap itself either -- that needs the
/// device state the IDE task owns -- so it leaves the name here and the task
/// picks it up between commands.
static SWAP_PENDING: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);
static mut SWAP_NAME: heapless::String<{ gamecfg::MAX_NEXT }> = heapless::String::new();

/// Ask for the folder called `name` to be mounted. Called from the button.
pub fn request_swap(name: &str) {
    // SAFETY: written only here and read only by the IDE task after the flag
    // is observed, and the button is the only writer.
    let buf = unsafe { &mut *core::ptr::addr_of_mut!(SWAP_NAME) };
    buf.clear();
    for c in name.chars() {
        if buf.push(c).is_err() {
            break;
        }
    }
    SWAP_PENDING.store(true, core::sync::atomic::Ordering::Release);
}

/// Perform a pending button swap, if one is waiting.
///
/// Mounts by name, which reads one directory rather than enumerating the
/// card. That is the whole point: enumeration returns a sixteen-entry vector
/// by value and this part has about two kilobytes of stack.
async fn service_swap_request(state: &mut DeviceState, label: &'static str) {
    if !SWAP_PENDING.swap(false, core::sync::atomic::Ordering::Acquire) {
        return;
    }
    // Ejected means the host is (or may be) writing the card: `handle_disk_write`
    // holds a CMD25 open across its waits, and a swap would both read the card
    // under it and set MEDIA_PRESENT mid-transfer so the next write ABRTs. The
    // button is dead while ejected (`button::on_press`); a request that slipped
    // in just before the eject is dropped here.
    if !media_present() {
        info!("{}: button swap dropped -- media is ejected", label);
        return;
    }
    let name = unsafe { &*core::ptr::addr_of!(SWAP_NAME) };
    let ok = mount_named(state, label, name.as_str()).await;
    info!("{}: button swapped to '{}' (mounted: {})", label, name.as_str(), ok);
}

/// Mount the folder called `name`, replacing whatever is mounted.
///
/// Stops CD-DA, drops the media and the sector cache, mounts by name,
/// prefetches IP.BIN, then raises UNIT ATTENTION so the host re-reads the
/// TOC. On failure the drive is left empty (`GDI = None`, no medium).
/// Returns whether an image mounted.
async fn mount_named(state: &mut DeviceState, label: &'static str, name: &str) -> bool {
    crate::cdda::stop();
    MEDIA_PRESENT.store(false, core::sync::atomic::Ordering::Relaxed);
    invalidate_cd_read_cache();

    let ok = match crate::gdi::load_by_name(disk_sectors(), name) {
        Ok(g) => {
            mount_gdi(g);
            if let Some(g) = gdi_ref() {
                prefetch_ip_bin(g).await;
            }
            true
        }
        Err(e) => {
            warn!("{}: mount '{}' failed: {:?}", label, name, e);
            unsafe { GDI = None };
            false
        }
    };
    MEDIA_PRESENT.store(ok, core::sync::atomic::Ordering::Relaxed);
    let delay = if APPLY_SWAP_WINDOW { crate::gdi::ini().swap_delay_ms() } else { 0 };
    unsafe {
        READY_AT = if ok && delay != 0 {
            Some(Instant::now() + Duration::from_millis(delay as u64))
        } else {
            None
        };
    }
    // Same as `load_media`: the format nibble must follow the new image.
    IdeDevice::primary().reassert_disc_format();
    state.medium_changed();
    info!("{}: mounted '{}' ({})", label, name, ok);
    ok
}

async fn swap_media(state: &mut DeviceState, label: &'static str) -> bool {
    MEDIA_PRESENT.store(false, core::sync::atomic::Ordering::Relaxed);
    invalidate_cd_read_cache();
    let ok = load_media(state).await;
    info!("{}: swapped media (mounted: {})", label, ok);
    ok
}


pub(crate) fn media_present() -> bool {
    MEDIA_PRESENT.load(core::sync::atomic::Ordering::Relaxed)
}

/// Whether GDEMU.INI `open_time` / `detect_time` open a NOT READY window after
/// a swap. OFF: the keys are parsed (`gdi::ini()`) but not applied.
///
/// The window as written is only half a tray cycle (review R45 item 1), and
/// half is worse than none: openMenu's `cdrom_reinit` can see OK inside it
/// and its next CD_READ then fails. It waits for stock-GDEMU probe data --
/// the SENSE sequence and the timing a real GDEMU shows after a swap. To turn
/// it on consistently, all of these are needed together:
///   * gate GET_TOC, REQ_SES, GET_SCD and REQ_STAT as well as TUR, CD_PLAY
///     and CD_READ;
///   * SECNR / REQ_STAT report OPEN -> BUSY during the window, not STANDBY;
///   * one UNIT ATTENTION when the window *ends* (not only before it);
///   * every gated path sets the sense key, and REQ_ERROR (0x13) reports it;
///   * also open the window on `swap_media` / `load_media`, not just
///     `mount_named`.
/// `service_hard_reset` already clears `READY_AT` on a reset.
const APPLY_SWAP_WINDOW: bool = false;

/// When a swapped-in disc stops reading NOT READY (GDEMU.INI `open_time` +
/// `detect_time`, applied only for keys the card set, and only while
/// [`APPLY_SWAP_WINDOW`]). Only `ide_task` touches
/// this. Non-blocking on purpose: the IDE task keeps answering commands during
/// the window, so the host sees NOT READY rather than a stalled bus.
static mut READY_AT: Option<Instant> = None;

/// Mounted and past the swap window: what TEST UNIT READY, CD_READ and CD_PLAY
/// answer on. (`media_present` is the ownership interlock and ignores it.)
fn media_ready() -> bool {
    if !media_present() {
        return false;
    }
    match unsafe { READY_AT } {
        Some(t) if Instant::now() < t => false,
        Some(_) => {
            unsafe { READY_AT = None };
            true
        }
        None => true,
    }
}
// Sega GD-ROM ATAPI extensions.
const SEGA_REQ_STAT: u8 = 0x10;
const SEGA_REQ_MODE: u8 = 0x11;
const SEGA_SET_MODE: u8 = 0x12;
const SEGA_REQ_ERROR: u8 = 0x13;
const SEGA_GET_TOC: u8 = 0x14;
const SEGA_REQ_SES: u8 = 0x15;
const SEGA_CD_OPEN: u8 = 0x16;
const SEGA_CD_PLAY: u8 = 0x20;
const SEGA_CD_SEEK: u8 = 0x21;
const SEGA_CD_SCAN: u8 = 0x22;
const SEGA_CD_READ: u8 = 0x30;
const SEGA_CD_READ2: u8 = 0x31;
const SEGA_GET_SCD: u8 = 0x40;
/// Sega vendor handshake. The BIOS issues 0x70 and 0x71 during disc
/// authentication and refuses to mount the disc unless the device
/// returns the exact 6-byte sequence `BA 06 0D CA 6A 1F` (per
/// iceGDROM's reverse engineering — ide.c:197-199 in that repo).
/// OpenGDEMU image control. Vendor space Sega leaves alone, so it cannot
/// collide with a real GD-ROM command. Data-in replies honour an allocation
/// length in packet[3..=4] (big-endian, 0 = everything).
///
/// Discs are named, never numbered. The console reads the card's filesystem
/// itself, so the firmware does not enumerate anything and there is no list
/// for an index to be an index *into* -- see `handle_image_ctl` for what that
/// replaced and why.
///
///   packet[1] = 2  MOUNT     -> mount the name staged by MOUNT_NAME, or the
///                               configured default if packet[2] = 0xFF or no
///                               name is staged. Swaps immediately, no
///                               eject/load needed; UNIT ATTENTION follows.
///                               The staged name is cleared once used.
///   packet[1] = 3  VERSION   -> "OGDM", u8 protocol, feature byte
///                               (`OGDM_FEATURES`), 3 reserved bytes (zero),
///                               24-byte firmware string.
///   packet[1] = 5  MOUNT_NAME-> packet[2] = chunk index, packet[3..12] = up
///                               to nine bytes of the folder name. Chunk 0
///                               starts a new name; MOUNT and WSESSION_END 2
///                               consume it (a BEGIN clears it too).
///   packet[1] = 6  WSESSION_BEGIN  packet[2..6] = "NGWS". Eject, start a
///                               write session (F7 protected range, F8
///                               checksums). Bad key: ILLEGAL REQUEST.
///   packet[1] = 7  WSESSION_END    packet[2] = 0 leave empty, 1 mount the
///                               selection/default, 2 mount the MOUNT_NAME
///                               name. Not in a session, bad selector or
///                               failed mount: ILLEGAL REQUEST.
///   packet[1] = 8  WSESSION_STATUS data-in, 48 bytes ("NGST"; see
///                               `discimg::wsession::WriteSession::status`).
const OGDM_IMAGE_CTL: u8 = 0xE0;
/// Bumped when the 0xE0 reply layouts or field meanings change.
///
/// 2: VERSION's max-images, count and selected bytes became reserved zeros.
///    The firmware keeps no catalog -- it mounts by name, opening one
///    directory -- so none of the three described anything. The reply is the
///    same 32 bytes; only the meaning of 5..8 changed.
///
/// 3: VERSION's byte 5 became a feature bitfield (see `OGDM_FEATURES`).
const OGDM_PROTOCOL_VERSION: u8 = 3;
/// VERSION reply byte 5: what this firmware can do, one bit each.
///   bit0 = write session (sub-commands 6/7/8, `discimg::wsession`)
///   bit1 = write checksum (STATUS reports the FNV-1a of the last write)
///   bit2 = DMA advertised in the secondary's IDENTIFY DEVICE
/// Bytes 6..8 stay reserved zeros.
const OGDM_FEATURES: u8 = 0x07;
/// MOUNT's packet[2], meaning "forget the runtime choice and mount whatever
/// GDEMU.CFG names" -- what a power cycle would do.
const NO_SELECTION: u8 = 0xFF;
/// Disc format: the high nibble of SECNR.
///
/// Values and packing cross-checked against flycast, which is the closest
/// thing to a spec for this register: `core/hw/gdrom/gdrom_if.h` enumerates
/// `CdDA=0x00, CdRom=0x10, CdRom_XA=0x20, CdRom_Extra=0x30, CdRom_CDI=0x40,
/// GdRom=0x80`, and `gdromv3.h` declares SECNR as `Status:4` then
/// `DiscFormat:4` -- disc format high, drive status low, exactly as this
/// firmware already assumed. (`CdRom_CDI=0x40` is CD-**I**, an unrelated
/// format; it has nothing to do with DiscJuggler `.cdi` files.)
const DISC_FMT_CDROM_XA: u8 = 0x20;
const DISC_FMT_GDROM: u8 = 0x80;

/// Drive status: the low nibble of SECNR (flycast `gdromv3.h:301`).
pub const GD_PAUSE: u8 = 0x01;
pub const GD_STANDBY: u8 = 0x02;
pub const GD_PLAY: u8 = 0x03;

/// The disc format to report for whatever is mounted.
///
/// A GDI is a GD-ROM. A CDI is a MIL-CD, and a MIL-CD is a CD: its data
/// track is Mode 2, which is what a real drive reports as CD-ROM XA --
/// flycast's `GuessDiscType` returns `CdRom_XA` for any image with a Mode 2
/// track (`core/imgread/common.cpp`). Reporting GD-ROM for one contradicts
/// the disc's own IP.BIN, which says `CD-ROM1/1` in its media field, and the
/// BIOS refuses to boot across that contradiction: it reads IP.BIN, the
/// ISO9660 PVD and the root directory, then abandons the boot without ever
/// reading the `1ST_READ.BIN` the directory just pointed it at.
pub fn disc_format() -> u8 {
    match gdi_ref() {
        Some(g) if g.is_cdi => DISC_FMT_CDROM_XA,
        _ => DISC_FMT_GDROM,
    }
}

/// SECNR byte for a drive status: mounted disc's format high, `status` low.
pub fn secnr(status: u8) -> u8 {
    disc_format() | status
}
const SEGA_VENDOR_70: u8 = 0x70;
const SEGA_VENDOR_71: u8 = 0x71;

async fn handle_packet(
    dev: &IdeDevice,
    state: &mut DeviceState,
    label: &'static str,
) {
    // FEATURES bit 0 = DMA mode requested. The data-in path now
    // supports DMA via FPGA register 19 (REG_DMA_MODE) — see
    // `stream_from_sd` for the arm/disarm. Carry the flag through.
    let snap = dev.snapshot_command();
    let dma_requested = (snap.features & 0x01) != 0;
    // The host's PIO byte-count limit (ATAPI 10.8.10) lives in LBA Mid/High
    // at command time. Read it now: the data-in setters overwrite those
    // registers with our own byte counts.
    let byte_limit = (snap.lba_mid as u16) | ((snap.lba_high as u16) << 8);

    // Step 2: the FPGA opened the CDB phase (IRR C/D=1, DRQ=1, BSY=0,
    // packet word count armed) on the Command write itself -- by the time
    // we get here the host may already have written all six words and
    // the FPGA raised BSY on the last one. Writing Status here would
    // reopen DRQ over a CDB that is already in the write FIFO, so only
    // release the command latch.
    dev.ack_command();

    // Step 3: wait for 6 words on the write FIFO.
    let packet = match recv_packet(dev).await {
        Ok(p) => p,
        Err(()) => {
            // A reset, or a new command to this device, while the CDB was
            // owed: Status is no longer this command's to write (R89 A1, A-b).
            if dev.aborted() {
                return;
            }
            warn!("{}: timeout waiting for ATAPI packet", label);
            dev.set_error(ERR_ABRT);
            dev.set_status_intrq(STATUS_IDLE_ERR);
            return;
        }
    };

    let opcode = packet[0];
    info!("{}: ATAPI opcode 0x{:02x} packet={:?}", label, opcode, packet);
    info!(
        "{}: ATAPI 0x{:02x}  [{:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x} {:02x}]",
        label, opcode,
        packet[0], packet[1], packet[2], packet[3],
        packet[4], packet[5], packet[6], packet[7],
        packet[8], packet[9], packet[10], packet[11],
    );

    // ATAPI: when UNIT_ATTENTION is pending (set on reset), every PACKET
    // command other than REQUEST_SENSE / REQ_ERROR completes immediately
    // with CHECK_CONDITION and sense key 6, and the UA flag is cleared.
    // The host is expected to follow up with REQ_SENSE to read the sense
    // data, then retry the original command. Without this the BIOS sees
    // a "ready" drive after reset and never proceeds past TOC validation.
    // (Verified via dcload-serial against a stock GDEMU: TEST_UNIT_READY
    // first response = STATUS=0x11, ERR=0x60.)
    // GDEMU_CMD is exempt too: a menu must be able to switch images while a
    // UA is pending or the drive is empty, and it leaves the UA for the host.
    if state.unit_attention
        && opcode != ATAPI_REQUEST_SENSE
        && opcode != SEGA_REQ_ERROR
        && opcode != GDEMU_CMD
    {
        info!("{}: UNIT_ATTENTION served on opcode 0x{:02x}", label, opcode);
        dev.set_error(0x60); // sense key 6 << 4
        dev.set_irr(0x03);
        // 6/28/00 for the REQ_SENSE / REQ_ERROR that should follow (set
        // here, not left over: a GDEMU_CMD in between clears stale keys).
        state.medium_changed();
        state.unit_attention = false;
        dev.set_status_intrq(STATUS_DRDY | STATUS_DSC | STATUS_ERR); // 0x51
        return;
    }

    // The sense key describes the last command only: anything but the two
    // sense readers starts from "no sense" (paths that fail set it again).
    // REQ_ERROR reports it now (R117), so a stale key would show there.
    if opcode != ATAPI_REQUEST_SENSE && opcode != SEGA_REQ_ERROR {
        state.clear_sense();
    }

    // Step 4: dispatch.
    match opcode {
        ATAPI_TEST_UNIT_READY => {
            // No data, just "I'm here" -- unless the tray is open.
            dev.set_irr(0x03);
            if media_ready() {
                dev.set_error(0);
                dev.set_status_intrq(STATUS_IDLE);
            } else {
                // NOT READY / MEDIUM NOT PRESENT.
                state.not_ready();
                dev.set_error(0x20);
                dev.set_status_intrq(STATUS_IDLE_ERR);
            }
        }
        ATAPI_START_STOP_UNIT => {
            // CDB byte 4: bit 1 = LoEj, bit 0 = Start. LoEj clear is just a
            // spin up/down request, which means nothing here.
            let loej = packet[4] & 0x02 != 0;
            let start = packet[4] & 0x01 != 0;
            if loej && start && write_session_active() {
                // Only WSESSION_END or a hard reset ends a session.
                warn!("{}: LOAD refused during a write session", label);
                finish_illegal(dev, state);
                return;
            }
            if loej {
                if start {
                    // Re-read the card: the whole point of the eject window is
                    // that the host may have added or removed images while it
                    // was open, so trusting the previously parsed GDI would
                    // defeat it.
                    let ok = load_media(state).await;
                    info!("{}: LOAD -- media present (image: {})", label, ok);
                } else {
                    info!("{}: EJECT -- releasing media", label);
                    MEDIA_PRESENT.store(false, core::sync::atomic::Ordering::Relaxed);
                    // Staged sectors are no longer trustworthy: the host is
                    // free to rewrite the filesystem they came from -- and
                    // for the same reason the image list, if anything built
                    // one, may no longer describe what is on the card.
                    invalidate_cd_read_cache();
                    crate::cdda::stop();
                }
            }
            dev.set_irr(0x03);
            dev.set_error(0);
            dev.set_status_intrq(STATUS_IDLE);
        }
        ATAPI_REQUEST_SENSE => {
            // 18 bytes of fixed-format sense data. Allocation length is
            // packet byte 4. Byte 2 reports the current sense key — we
            // patch the canned reply with state.pending_sense_key so the
            // host learns about the UA condition. Reading sense data
            // clears any pending UNIT_ATTENTION condition per ATAPI spec.
            let alloc = packet[4] as u16;
            let mut sense = REQUEST_SENSE_REPLY;
            sense[2] = state.pending_sense_key;
            let n = core::cmp::min(alloc, sense.len() as u16);
            send_data_in(dev, &sense[..n as usize]).await;
            state.unit_attention = false;
            state.clear_sense();
        }
        SEGA_REQ_STAT => {
            let alloc = packet[4] as u16;
            let reply = build_req_stat_reply();
            let n = core::cmp::min(alloc, reply.len() as u16);
            send_data_in(dev, &reply[..n as usize]).await;
        }
        SEGA_REQ_MODE => {
            // packet[2] = offset, packet[4] = length into the 32-byte block
            // stock GDEMU 6.0 returns for (0, 32) (R117): the BIOS reads
            // (0, 10) and (18, 8) = "Rev 6.42". A window past the block, or
            // an empty one, still aborts (stock not probed there).
            match discimg::atapi::req_mode_reply(packet[2], packet[4]) {
                Some(reply) => send_data_in(dev, reply).await,
                None => {
                    warn!("{}: REQ_MODE off={} len={} unsupported", label, packet[2], packet[4]);
                    dev.set_error(ERR_ABRT);
                    dev.set_status_intrq(STATUS_IDLE_ERR);
                }
            }
        }
        SEGA_SET_MODE => {
            // Sega SET_MODE is a data-OUT command: host writes
            // `packet[4]` bytes (typically 10) to the drive. Without
            // draining that phase the host hangs waiting for DRQ to
            // clear and the BIOS's post-CD_READ "configure drive" step
            // never completes — game launch never proceeds. Read and
            // log the bytes; we don't yet act on them.
            let len = packet[4] as usize;
            let data = recv_data_out(dev, len).await;
            // Cut off by a reset or a new command: no completion (R89 A-b).
            if dev.aborted() {
                return;
            }
            info!(
                "{}: SET_MODE got {} bytes: {=[u8]:02x}", label, data.len(), data.as_slice()
            );
            dev.set_irr(0x03);
            dev.set_status_intrq(STATUS_IDLE);
        }
        SEGA_CD_PLAY => {
            // Ejected, GDI is still set but the host may be writing the card:
            // cdda's SD reads would cut into an open CMD25. NOT READY /
            // MEDIUM NOT PRESENT, as TEST UNIT READY and CD_READ say.
            // (CD_SEEK only pauses or stops, which reads nothing.)
            if !media_ready() {
                info!("{}: CD_PLAY while ejected", label);
                state.not_ready();
                dev.set_irr(0x03);
                dev.set_error(0x20);
                dev.set_status_intrq(STATUS_IDLE_ERR);
                return;
            }
            // [1] parameter type, [2..5] start, [6] repeat count (low
            // nibble, 15 = forever), [8..11] end. Type 1 = FAD (24-bit BE),
            // 2 = MSF, 3 = resume from pause. Layout per iceGDROM cdda.c.
            let ptype = packet[1] & 0x0F;
            let repeat = packet[6] & 0x0F;
            let be24 = |b: &[u8]| ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
            let msf = |b: &[u8]| ((b[0] as u32) * 60 + b[1] as u32) * 75 + b[2] as u32;
            let ok = match ptype {
                1 => crate::cdda::play(be24(&packet[2..5]), be24(&packet[8..11]), repeat).is_ok(),
                2 => crate::cdda::play(msf(&packet[2..5]), msf(&packet[8..11]), repeat).is_ok(),
                3 => { crate::cdda::resume(); true }
                _ => false,
            };
            if !ok {
                warn!("{}: CD_PLAY rejected: {=[u8]:02x}", label, packet[..12]);
            }
            dev.set_irr(0x03);
            if ok {
                dev.set_secnr(secnr(GD_PLAY));
                dev.set_error(0);
                dev.set_status_intrq(STATUS_IDLE);
            } else {
                dev.set_error(ERR_ABRT);
                dev.set_status_intrq(STATUS_IDLE_ERR);
            }
        }
        SEGA_CD_SEEK => {
            // Parameter type 4 = pause; anything else (seek to FAD/MSF,
            // stop) ends playback, as iceGDROM does.
            if packet[1] & 0x0F == 4 {
                crate::cdda::pause();
                dev.set_secnr(secnr(GD_PAUSE));
            } else {
                crate::cdda::stop();
                dev.set_secnr(secnr(GD_STANDBY));
            }
            dev.set_irr(0x03);
            dev.set_error(0);
            dev.set_status_intrq(STATUS_IDLE);
        }
        SEGA_CD_OPEN | SEGA_CD_SCAN => {
            // No-data commands — just succeed. Status phase, no error.
            // Must assert INTRQ for the host's command-complete handler;
            // without this the BIOS waits forever after issuing CD_SEEK
            // post-REQ_SES and falls into CD_SCD spam.
            dev.set_irr(0x03);
            dev.set_error(0);
            dev.set_status_intrq(STATUS_IDLE);
        }
        SEGA_REQ_ERROR => {
            // F0, then the pending sense key (stock GDEMU 6.0, R117; this
            // was all zeros) and its ASC/ASCQ (D7). Reading it clears the
            // condition, as REQUEST SENSE does.
            let alloc = packet[4] as u16;
            let (asc, ascq) = state.pending_asc;
            let reply = discimg::atapi::req_error_reply(state.pending_sense_key, asc, ascq);
            let n = core::cmp::min(alloc, reply.len() as u16);
            send_data_in(dev, &reply[..n as usize]).await;
            state.unit_attention = false;
            state.clear_sense();
        }
        SEGA_GET_TOC => {
            // Sega GET_TOC packet: byte 1 selects the session/area
            // (0 = SDA, 1 = HDA), bytes 3..4 = allocation length BE.
            // Build a real TOC from the parsed GDI when we have one;
            // fall back to ABRT if we don't (the BIOS shouldn't ever
            // ask GET_TOC before a disc is present, but be safe).
            if let Some(g) = gdi_ref() {
                let area = packet[1] & 0x01;
                let alloc = ((packet[3] as u16) << 8) | packet[4] as u16;
                let mut toc = [0u8; 408];
                g.build_toc(area, &mut toc);
                // Dump first 32 bytes of TOC + the trailing 12 bytes
                // (first/last/leadout) so we can compare against a known
                // reference at the bench.
                info!(
                    "{} GET_TOC area={} hd={:02x}{:02x}{:02x}{:02x} {:02x}{:02x}{:02x}{:02x} t3={:02x}{:02x}{:02x}{:02x} f={:02x}{:02x}{:02x}{:02x} l={:02x}{:02x}{:02x}{:02x} lo={:02x}{:02x}{:02x}{:02x}",
                    label, area,
                    toc[0], toc[1], toc[2], toc[3],
                    toc[4], toc[5], toc[6], toc[7],
                    toc[8], toc[9], toc[10], toc[11],
                    toc[396], toc[397], toc[398], toc[399],
                    toc[400], toc[401], toc[402], toc[403],
                    toc[404], toc[405], toc[406], toc[407],
                );
                let n = core::cmp::min(alloc as usize, toc.len());
                let n_aligned = n & !1; // host expects 16-bit-aligned PIO; trim odd trailing byte
                send_data_in(dev, &toc[..n_aligned]).await;
            } else {
                warn!("{}: GET_TOC with no GDI loaded", label);
                dev.set_error(ERR_ABRT);
                dev.set_status_intrq(STATUS_IDLE_ERR);
            }
        }
        SEGA_REQ_SES => {
            // Sega REQ_SES: packet[2] is a 0-indexed session number;
            // BIOS asks for sessions 0..num_sessions and our reply
            // describes that session's first track + start FAD. Session
            // 0 = SDA, session 1 = HDA on a GD-ROM. Sessions outside
            // the disc's range get ABRT (the BIOS uses that to learn
            // num_sessions). Per iceGDROM rv32/source/ide.c:437.
            let alloc = packet[4] as u16;
            let session = packet[2];
            match build_req_ses_reply(gdi_ref(), session) {
                Some(reply) => {
                    info!(
                        "{}: REQ_SES({}) -> {=[u8]:02x}", label, session, reply.as_slice()
                    );
                    let n = core::cmp::min(alloc as usize, reply.len());
                    send_data_in(dev, &reply[..n]).await;
                }
                None => {
                    info!(
                        "{}: REQ_SES({}) out of range, ABRT", label, session
                    );
                    dev.set_irr(0x03);
                    dev.set_error(ERR_ABRT);
                    dev.set_status_intrq(STATUS_IDLE_ERR);
                }
            }
        }
        SEGA_GET_SCD => {
            let alloc = ((packet[3] as u16) << 8) | packet[4] as u16;
            let mut reply = [0u8; 100];
            let len = build_get_scd_reply(packet[1] & 0x0F, &mut reply);
            let n = core::cmp::min(alloc, len as u16);
            send_data_in(dev, &reply[..n as usize]).await;
        }
        SEGA_CD_READ | SEGA_CD_READ2 => {
            handle_cd_read(dev, state, label, &packet, dma_requested, byte_limit).await;
        }
        OGDM_IMAGE_CTL => {
            handle_image_ctl(dev, state, label, &packet).await;
        }
        GDEMU_CMD => {
            handle_gdemu_cmd(dev, state, label, &packet).await;
        }
        SEGA_VENDOR_70 => {
            // Per iceGDROM rv32/source/ide.c — 0x70 is a no-data probe
            // (treated identically to TEST_UNIT_READY — `finish_packet_ok`).
            // Earlier this branch returned the 6-byte magic; that was
            // wrong, the magic is for 0x71 only.
            dev.set_irr(0x03);
            dev.set_error(0);
            dev.set_status_intrq(STATUS_IDLE);
        }
        SEGA_VENDOR_71 => {
            // Sega disc-auth challenge. The reply is a fixed 6-byte
            // magic and the BIOS expects the full 6 bytes regardless
            // of what's in packet[4] (per iceGDROM, which ignores the
            // alloc-len field for this command).
            send_data_in(dev, &VENDOR_HANDSHAKE_REPLY).await;
        }
        _ => {
            warn!("{}: unhandled ATAPI opcode 0x{:02x}", label, opcode);
            dev.set_irr(0x03);
            dev.set_error(ERR_ABRT);
            dev.set_status_intrq(STATUS_IDLE_ERR);
        }
    }
    // Every arm posts its completion status and INTRQ together; there is
    // deliberately no catch-all assert_intrq here. A second INTRQ after
    // the host has already read Status for the first one rides into its
    // next command as a spurious interrupt.
}

/// Sega CD_READ packet layout:
///
/// ```text
///   byte 0     : 0x30
///   byte 1     : data type / parameters
///   byte 2..4  : starting FAD (frame addr; LBA + 150), big-endian 24-bit
///   byte 5..7  : reserved
///   byte 8..10 : sector count, big-endian 24-bit
///   byte 11    : reserved
/// ```
///
/// FAD = LBA + 150 in CD-frame addressing (the 150-frame offset is the
/// 2-second pre-gap before track 1 in MSF/CD-DA terms).
async fn handle_cd_read(
    dev: &IdeDevice,
    state: &mut DeviceState,
    label: &'static str,
    packet: &[u8],
    dma: bool,
    byte_limit: u16,
) {
    let starting_fad = ((packet[2] as u32) << 16)
        | ((packet[3] as u32) << 8)
        | packet[4] as u32;
    let count = ((packet[8] as u32) << 16)
        | ((packet[9] as u32) << 8)
        | packet[10] as u32;
    let lba = starting_fad.saturating_sub(150);

    info!(
        "{}: CD_READ FAD={} ({} sectors), LBA={}, dma={}",
        label, starting_fad, count, lba, dma
    );
    info!(
        "{}: CD_READ FAD={} ({}sect) LBA={} dma={}",
        label, starting_fad, count, lba, dma
    );

    if !media_ready() {
        // NOT READY / MEDIUM NOT PRESENT -- the host ejected and has not
        // loaded again, so it may be rewriting the filesystem right now.
        info!("{}: CD_READ while ejected", label);
        dev.set_irr(0x03);
        dev.set_error(0x20);
        dev.set_status_intrq(STATUS_IDLE_ERR);
        return;
    }
    let Some(gdi) = gdi_ref() else {
        warn!("{}: CD_READ but no GDI loaded", label);
        dev.set_error(ERR_ABRT);
        dev.set_status_intrq(STATUS_IDLE_ERR);
        return;
    };

    // For now: read the (cooked) 2048-byte data area of each sector.
    // Real GD-ROM mode 1 sectors on disk are 2352 bytes raw; for the
    // user-data subset we'd want a proper sync/header skip — punt for
    // now and read the whole 2352-byte sector as-is, sending the first
    // 2048 bytes to the host. Good enough to validate the SD path.
    const SECTOR_BYTES: usize = 2048;
    let total = (count as usize).saturating_mul(SECTOR_BYTES);
    // No upper bound on the request size any more: stream_from_sd stages the
    // transfer a chunk at a time rather than buffering it whole, so a request
    // is limited by the disc, not by our RAM. Games ask for far more than the
    // BIOS does -- the first real content read observed here was 572 sectors
    // (1.1 MB), which the old 64 KB guard aborted outright.
    //
    // Still reject a request that runs off the end of the track; better an
    // ABRT the host can see than a read that silently returns garbage.
    let track_sectors = gdi.track_for_lba(lba).map(|t| t.sector_count).unwrap_or(0);
    let available = track_sectors.saturating_sub(lba.saturating_sub(
        gdi.track_for_lba(lba).map(|t| t.start_lba).unwrap_or(0),
    ));
    if total == 0 || count > available {
        warn!(
            "{}: CD_READ rejected: {} sectors from LBA {} ({} available in track)",
            label, count, lba, available
        );
        dev.set_error(ERR_ABRT);
        dev.set_status_intrq(STATUS_IDLE_ERR);
        return;
    }
    #[cfg(all(feature = "sd-multiblock", not(feature = "dma-pattern")))]
    unsafe { STREAM_OK = false };
    if stream_from_sd(dev, gdi, lba, count as u32, total, dma, byte_limit).await {
        // The card failed under the read: MEDIUM ERROR for the REQUEST SENSE
        // that follows the error status the stream posted.
        state.pending_sense_key = 0x03;
    }
    // Only after a transfer that went all the way through (see `STREAM_OK`).
    #[cfg(all(feature = "sd-multiblock", not(feature = "dma-pattern")))]
    if unsafe { STREAM_OK } {
        read_ahead(dev, gdi, lba, count as u32);
    }
}

/// Set by `stream_from_sd` / `stream_pio_from_sd` when, and only when, the
/// request was staged in full, the host took every byte and the completion
/// was posted. Cleared before each request. Read-ahead is gated on it so it
/// never runs after a failed SD read, an abandoned or reset transfer.
#[cfg(all(feature = "sd-multiblock", not(feature = "dma-pattern")))]
static mut STREAM_OK: bool = false;

/// Streaming history for [`read_ahead`].
#[cfg(all(feature = "sd-multiblock", not(feature = "dma-pattern")))]
static mut READ_AHEAD: sdfast::readahead::ReadAhead = sdfast::readahead::ReadAhead::new();

/// Feature `sd-multiblock`: fetch the sectors a streaming host will ask for
/// next into the idle CD_READ_BUF, once this request has been completed.
///
/// Runs after `stream_from_sd` returns, i.e. after the completion status and
/// INTRQ are already posted, so it adds nothing to this command's latency; it
/// can only delay the *next* command's pickup by one CMD18 (~1-2 ms), which a
/// miss would have cost anyway. Only done when the request continued the
/// previous one (`sdfast::readahead`), and only for mapped tracks: the
/// filesystem fallback is far too slow to run speculatively.
///
/// The result is an ordinary cache entry (`CD_READ_BUF_LBA`/`_LEN`), which
/// `stage_chunk` already honours, so a wrong guess costs one wasted read and
/// nothing else.
#[cfg(all(feature = "sd-multiblock", not(feature = "dma-pattern")))]
fn read_ahead(dev: &IdeDevice, gdi: &'static crate::gdi::Gdi, lba: u32, count: u32) {
    let ra = unsafe { &mut *core::ptr::addr_of_mut!(READ_AHEAD) };
    let Some(cand) = ra.record(lba, count, stage_sectors_for(gdi, lba)) else {
        return;
    };
    // The host has already moved on, or is resetting the bus: its next
    // command is the priority, not a guess about it. "Its" includes the other
    // device on the bus: the read below cannot be interrupted, and a command
    // waiting for the secondary would sit behind it just the same.
    if dev.command_pending() || dev.peer().command_pending() || dev.reset_seen() {
        return;
    }
    // CDDA is fed from the same SD card by `cdda_task`, and the one thing
    // that must not stall behind a speculative read is its ring.
    if crate::cdda::snapshot().status == crate::cdda::Status::Playing {
        return;
    }
    let Some(track) = gdi.track_for_lba(cand.lba) else { return };
    if !track.is_mapped() {
        return;
    }
    let remaining = track
        .sector_count
        .saturating_sub(cand.lba.saturating_sub(track.start_lba));
    let Some(p) = cand.clamped(stage_sectors_for(gdi, cand.lba), remaining) else {
        return;
    };
    let want = p.count as usize * 2048;
    if unsafe { CD_READ_BUF_LBA == p.lba && CD_READ_BUF_LEN >= want } {
        return;
    }
    invalidate_cd_read_cache();
    if let Some(len) = fill_cd_read_buf_raw(track, p.lba, p.count) {
        unsafe {
            CD_READ_BUF_LBA = p.lba;
            CD_READ_BUF_LEN = len;
        }
    }
}

/// Maximum CD_READ payload we hold in RAM up-front. Sized to fit the
/// BIOS's 7-sector IP.BIN read (14 KB) — freed the 16 KB defmt-rtt
/// ring buffer to make room. Larger requests get truncated to this
/// and short-transferred.
const CD_READ_BUF_BYTES: usize = 14 * 1024;
#[link_section = ".sram1"]
static mut CD_READ_BUF: [u8; CD_READ_BUF_BYTES] = [0; CD_READ_BUF_BYTES];

/// Tracks the LBA range currently sitting in CD_READ_BUF. Set by
/// `prefetch_ip_bin()` at MCU startup and refreshed inside
/// `stream_from_sd`. If a CD_READ matches this exactly we skip the
/// SD read entirely, giving the host's DMA controller data to pull
/// the moment it issues DMA_STATUS=1 — which is what the BIOS does
/// right after the CDB. With SD-read latency in the path the BIOS's
/// DMA channel times out at ~10 KB; cached, it drains all 14 KB.
static mut CD_READ_BUF_LBA: u32 = u32::MAX;
static mut CD_READ_BUF_LEN: usize = 0;

/// Drop the staged-sector cache.
///
/// Called on eject/load: once the host can rewrite the filesystem those bytes
/// came from, a cache keyed on LBA is no longer meaningful.
fn invalidate_cd_read_cache() {
    unsafe {
        CD_READ_BUF_LBA = u32::MAX;
        CD_READ_BUF_LEN = 0;
    }
}

/// Lend CD_READ_BUF to the image loader (it reads the .gdi text through it).
/// Only valid while no CD_READ is in flight, i.e. at boot and from the
/// LOAD path inside `ide_task`; whatever was staged is forgotten.
pub fn loader_scratch() -> &'static mut [u8] {
    invalidate_cd_read_cache();
    unsafe { &mut *core::ptr::addr_of_mut!(CD_READ_BUF) }
}

/// How many sectors of the track holding `lba` fit in CD_READ_BUF per
/// staging round. Mapped raw tracks are read whole-sector (2352 bytes plus
/// up to a block of lead-in) and compacted in place, so fewer fit than the
/// 2048-byte payloads they turn into: 5 instead of 7.
fn stage_sectors_for(gdi: &crate::gdi::Gdi, lba: u32) -> u32 {
    let payload_max = (CD_READ_BUF_BYTES / 2048) as u32;
    match gdi.track_for_lba(lba) {
        Some(t) if t.is_mapped() && t.sector_size as usize > 2048 => {
            let sec = t.sector_size as usize;
            let mut n = payload_max as usize;
            while n > 1 && (n * sec + 511 + 511) / 512 * 512 > CD_READ_BUF_BYTES {
                n -= 1;
            }
            n as u32
        }
        _ => payload_max,
    }
}

/// Fast path for `fill_cd_read_buf_from_sd`: multi-block reads straight
/// from the card using the track's extent map, then compact each stored
/// sector's 2048-byte payload down to the front of the buffer. The
/// compaction always moves data towards lower addresses (payload `s` starts
/// at or before its raw source), so it is safe in place. `None` means the
/// track is unmapped, the request does not fit, or the card read failed;
/// the caller then falls back to the filesystem path.
fn fill_cd_read_buf_raw(
    track: &crate::gdi::Track,
    start_lba: u32,
    count: u32,
) -> Option<usize> {
    let sec_size = track.sector_size as usize;
    let byte0 = track.byte_offset_of(start_lba)?;
    let head = (byte0 % 512) as usize;
    let raw_len = head + count as usize * sec_size;
    let nblocks = (raw_len + 511) / 512;
    if nblocks * 512 > CD_READ_BUF_BYTES {
        return None;
    }
    let buf = unsafe { &mut *core::ptr::addr_of_mut!(CD_READ_BUF) };
    let first = byte0 - head as u32;
    let mut done = 0usize;
    while done < nblocks {
        let (block, _, contig) = track.locate_byte(first + (done as u32) * 512)?;
        let n = (nblocks - done)
            .min(contig as usize)
            .min(embassy_atsam3::hsmci::MAX_BLOCKS_PER_READ);
        let dst = &mut buf[done * 512..(done + n) * 512];
        if let Err(e) = unsafe { embassy_atsam3::hsmci::read_blocks_dma(block, dst) } {
            warn!("cd_read raw: SD read of {} blocks at {} failed: {:?}", n, block, e);
            return None;
        }
        done += n;
    }
    let uoff = track.user_data_offset();
    for s in 0..count as usize {
        let src = head + s * sec_size + uoff;
        buf.copy_within(src..src + 2048, s * 2048);
    }
    Some(count as usize * 2048)
}

/// Read `count` sectors starting at `start_lba` into CD_READ_BUF.
/// On success updates CD_READ_BUF_LBA / CD_READ_BUF_LEN. Used both
/// from `stream_from_sd` (cache miss path) and from the boot-time
/// `prefetch_ip_bin` so the BIOS's first CD_READ is a cache hit.
async fn fill_cd_read_buf_from_sd(
    gdi: &'static crate::gdi::Gdi,
    start_lba: u32,
    count: u32,
) -> Result<usize, ()> {
    let track = gdi.track_for_lba(start_lba).ok_or(())?;
    // The buffer is about to be overwritten; a failed read must not leave
    // the old range advertised over new bytes.
    invalidate_cd_read_cache();
    if let Some(len) = fill_cd_read_buf_raw(track, start_lba, count) {
        unsafe {
            CD_READ_BUF_LBA = start_lba;
            CD_READ_BUF_LEN = len;
        }
        return Ok(len);
    }
    let sec_size = track.sector_size as usize;
    // Include the track's base offset within its file. GDI gives every track
    // its own file so that base is 0, but CDI concatenates all tracks into
    // one, and without this every CDI read past track 1 lands in the wrong
    // track's data.
    let track_offset =
        (start_lba - track.start_lba) * sec_size as u32 + track.file_offset;
    let track_filename = track.filename.clone();

    let block_dev = crate::blockdev::HsmciBlockDevice::new();
    block_dev.set_num_blocks(gdi.num_blocks);
    let mgr = embedded_sdmmc::VolumeManager::new(block_dev, crate::blockdev::DummyClock);
    let buf = unsafe { &mut CD_READ_BUF[..(count as usize * 2048)] };

    let volume = mgr.open_volume(embedded_sdmmc::VolumeIdx(0)).map_err(|_| ())?;
    let mut dir = volume.open_root_dir().map_err(|_| ())?;
    for part in gdi.image_dir.iter() {
        dir.change_dir(part.clone()).map_err(|_| ())?;
    }
    let file = dir
        .open_file_in_dir(&track_filename, embedded_sdmmc::Mode::ReadOnly)
        .map_err(|_| ())?;
    file.seek_from_start(track_offset).map_err(|_| ())?;

    let mut sector_buf = [0u8; 2352];
    for s in 0..count {
        let sb = &mut sector_buf[..sec_size];
        let mut tot = 0usize;
        while tot < sb.len() {
            let n = file.read(&mut sb[tot..]).map_err(|_| ())?;
            if n == 0 { break; }
            tot += n;
        }
        // Where the 2048-byte user data sits inside a stored sector. 2352 is
        // raw (12 sync + 4 header); 2336 is the CDI data form, which drops
        // sync and header but keeps the 8-byte Mode 2 subheader -- taking the
        // first 2048 bytes there would hand the host the subheader and shift
        // every sector by 8. 2048 is already cooked.
        let payload: &[u8] = match sec_size {
            2352 => &sector_buf[16..16 + 2048],
            2336 => &sector_buf[8..8 + 2048],
            _ => &sector_buf[..2048.min(sec_size)],
        };
        let dst = (s as usize) * 2048;
        buf[dst..dst + payload.len()].copy_from_slice(payload);
    }
    let len = (count as usize) * 2048;
    unsafe {
        CD_READ_BUF_LBA = start_lba;
        CD_READ_BUF_LEN = len;
    }
    Ok(len)
}

/// FNV-1a 32. Used to fingerprint exactly the bytes we hand to the FIFO so a
/// DC-side checksum over what actually landed in host RAM can be compared
/// against it -- byte counts alone cannot distinguish "lost the tail" from
/// "reported the level wrong".
/// Overwrite the staged chunk with a 16-bit word counter keyed to the word's
/// position in the *whole* transfer, so the pattern is continuous across chunk
/// refills. `byte_off` is where this chunk starts within the request.
#[cfg(feature = "dma-pattern")]
fn fill_pattern(byte_off: usize, len: usize) {
    let buf = unsafe { &mut CD_READ_BUF[..len] };
    for i in (0..len).step_by(2) {
        let w = (((byte_off + i) / 2) & 0xFFFF) as u16;
        buf[i] = (w & 0xFF) as u8;
        buf[i + 1] = (w >> 8) as u8;
    }
}

const FNV_INIT: u32 = 2166136261;
fn fnv1a(mut h: u32, data: &[u8]) -> u32 {
    for &b in data {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    h
}

/// Put `count` sectors starting at `lba` into CD_READ_BUF, skipping the SD
/// read when the prefetch cache already holds exactly that range. Returns
/// false if the read failed.
///
/// The cache only ever matches the first chunk of a request (it is seeded by
/// `prefetch_ip_bin`), which is the one that matters: it is what lets the
/// BIOS's first CD_READ start pushing with no SD latency in front of it.
async fn stage_chunk(gdi: &'static crate::gdi::Gdi, lba: u32, count: u32) -> bool {
    let want = (count as usize) * 2048;
    let hit = unsafe { CD_READ_BUF_LBA == lba && CD_READ_BUF_LEN >= want };
    if hit {
        info!("  cache hit: LBA={} ({} sectors)", lba, count);
        return true;
    }
    fill_cd_read_buf_from_sd(gdi, lba, count).await.is_ok()
}

/// Pre-fill CD_READ_BUF with the first 7 sectors of HDA at MCU boot.
/// The BIOS's very first CD_READ on game-launch is a DMA-mode read of
/// IP.BIN (LBA = HDA start, 7 sectors). With this cache pre-loaded
/// the SD-read latency disappears for that read and the host's DMA
/// channel can drain all 14 KB before timing out.
/// Complete a no-data command successfully.
fn finish_ok(dev: &IdeDevice) {
    dev.set_irr(0x03);
    dev.set_error(0);
    dev.set_status_intrq(STATUS_IDLE);
}

/// Complete a no-data command with CHECK CONDITION / ILLEGAL REQUEST.
fn finish_illegal(dev: &IdeDevice, state: &mut DeviceState) {
    state.pending_sense_key = 0x05;
    dev.set_irr(0x03);
    dev.set_error(ERR_ABRT | 0x50); // sense key 5 << 4
    dev.set_status_intrq(STATUS_IDLE_ERR);
}

/// Send `data`, cut to the CDB's allocation length (packet[3..=4], 0 = all).
async fn send_alloc(dev: &IdeDevice, packet: &[u8; 12], data: &[u8]) {
    let alloc = ((packet[3] as usize) << 8) | packet[4] as usize;
    let n = if alloc == 0 { data.len() } else { alloc.min(data.len()) };
    send_data_in(dev, &data[..n]).await;
}

/// OpenGDEMU's native 0xE0 -- see `OGDM_IMAGE_CTL` for the layouts.
async fn handle_image_ctl(
    dev: &IdeDevice,
    state: &mut DeviceState,
    label: &'static str,
    packet: &[u8; 12],
) {
    match packet[1] {
        // 0 (LIST) and 4 (INFO) are gone. Enumerating the card is the
        // menu's job now -- it can read the filesystem itself, and doing it
        // here cost this part a sixteen-entry catalog in RAM plus a
        // by-value vector on a 2 KB stack. 1 (SELECT by index) went with
        // them: an index only means something against a list the firmware
        // keeps.
        SUB_MOUNT_NAME => {
            // A chunk of the folder name to mount. The CDB is twelve bytes
            // and a folder name is not, so the name arrives nine bytes at a
            // time; chunk zero starts a new name. Chunking through the
            // existing command path is simpler than giving this one command
            // a data-out phase of its own.
            let chunk = packet[2] as usize;
            let name = unsafe { &mut *core::ptr::addr_of_mut!(MOUNT_NAME) };
            if chunk == 0 {
                name.clear();
            }
            for b in &packet[3..12] {
                if *b == 0 {
                    break;
                }
                if name.push(*b as char).is_err() {
                    break;
                }
            }
            finish_ok(dev);
        }
        SUB_MOUNT => {
            if write_session_active() {
                warn!("{}: MOUNT refused during a write session", label);
                finish_illegal(dev, state);
                return;
            }
            // Mount whatever name was staged, or the configured default.
            let name = unsafe { &*core::ptr::addr_of!(MOUNT_NAME) };
            if packet[2] == NO_SELECTION || name.is_empty() {
                clear_mount_name();
                crate::gdi::select_default();
                // A failed mount is ILLEGAL REQUEST here as it is for a name
                // (it used to report OK over an empty drive, R89 A8).
                if swap_media(state, label).await {
                    finish_ok(dev);
                } else {
                    finish_illegal(dev, state);
                }
                return;
            }
            let ok = mount_named(state, label, name.as_str()).await;
            clear_mount_name();
            if ok {
                finish_ok(dev);
            } else {
                finish_illegal(dev, state);
            }
        }
        discimg::wsession::SUB_WSESSION_BEGIN => {
            // DESIGN-GAP: a bad key is refused before anything changes; an SD
            // read error while taking the partition start also refuses the
            // BEGIN (nothing ejected) -- better than running unprotected.
            if packet[2..6] != discimg::wsession::BEGIN_KEY[..] {
                finish_illegal(dev, state);
                return;
            }
            // CD_READ_BUF is free to borrow: the drive is about to be ejected.
            let sec: Option<&mut [u8; 512]> = loader_scratch().first_chunk_mut::<512>();
            let part = match sec.map(crate::fat::partition_start) {
                Some(Ok(p)) if !gpt_card(p) => p,
                Some(Ok(_)) => {
                    warn!("{}: WSESSION_BEGIN refused: GPT card (protective MBR)", label);
                    finish_illegal(dev, state);
                    return;
                }
                _ => {
                    warn!("{}: WSESSION_BEGIN: cannot read the partition table", label);
                    finish_illegal(dev, state);
                    return;
                }
            };
            crate::cdda::stop();
            MEDIA_PRESENT.store(false, core::sync::atomic::Ordering::Relaxed);
            invalidate_cd_read_cache();
            unsafe { READY_AT = None };
            clear_mount_name();
            let _ = wsession().begin(&packet[2..6], part);
            info!("{}: write session begun, protecting LBA < {}", label, part);
            finish_ok(dev);
        }
        discimg::wsession::SUB_WSESSION_END => {
            use discimg::wsession::Remount;
            let Ok(how) = wsession().end(packet[2]) else {
                warn!("{}: WSESSION_END refused (no session or bad selector)", label);
                finish_illegal(dev, state);
                return;
            };
            info!("{}: write session ended", label);
            let ok = match how {
                Remount::None => {
                    state.medium_changed();
                    true
                }
                Remount::Default => {
                    // The write session may have rewritten GDEMU.INI.
                    crate::gdi::reload_ini(disk_sectors());
                    load_media(state).await
                }
                Remount::Named => {
                    crate::gdi::reload_ini(disk_sectors());
                    let name = unsafe { &*core::ptr::addr_of!(MOUNT_NAME) };
                    // DESIGN-GAP: Named with no name staged = Default (same
                    // as MOUNT's no-name case).
                    let ok = if name.is_empty() {
                        load_media(state).await
                    } else {
                        mount_named(state, label, name.as_str()).await
                    };
                    clear_mount_name();
                    ok
                }
            };
            if ok {
                finish_ok(dev);
            } else {
                finish_illegal(dev, state);
            }
        }
        discimg::wsession::SUB_WSESSION_STATUS => {
            let st = wsession().status(media_present());
            send_alloc(dev, packet, &st).await;
        }
        SUB_VERSION => {
            let mut out = [0u8; 32];
            out[..4].copy_from_slice(b"OGDM");
            out[4] = OGDM_PROTOCOL_VERSION;
            out[5] = OGDM_FEATURES;
            // Bytes 6..8 are reserved, and zero (byte 5 is the feature bits
            // above). They used to carry an
            // image limit, a count and a selected index -- all three
            // describing a catalog the firmware kept. It does not keep one:
            // it mounts by name, opening the one directory it needs, so
            // there is no limit, no count and no index. The host is the side
            // that can read the filesystem; what is on the card is its
            // question to answer. The bytes stay in the layout rather than
            // shortening the reply, so the offset of the version string does
            // not move.
            // Behind a resident boot region they carry its interface
            // generation and flags (docs/design/resident-app-contract.md 3-3).
            #[cfg(feature = "resident")]
            {
                let gf = crate::resident::ogdm_gen_flags();
                out[6] = gf as u8;
                out[7] = (gf >> 8) as u8;
            }
            let fw = env!("CARGO_PKG_VERSION").as_bytes();
            let n = fw.len().min(24);
            out[8..8 + n].copy_from_slice(&fw[..n]);
            send_alloc(dev, packet, &out).await;
        }
        sub => {
            warn!("{}: IMAGE_CTL sub-command {} not supported", label, sub);
            finish_illegal(dev, state);
        }
    }
}

/// Original GDEMU's image-selection command, which menus (GDmenu, openMenu)
/// send to the GD-ROM device. CDB byte 1 is the sub-code:
///
///   52 82 lo hi  mount image number `lo | hi << 8`; 0 = back to the default.
///                No data phase. A number with no folder, or a failed
///                mount, is ILLEGAL REQUEST; an unknown number leaves the
///                current media untouched.
///   52 81 55     next image          (wraps from the last to the first)
///   52 81 44     previous image
///   52 00        data-in, 8 bytes: two little-endian u32, bootloader then
///                firmware version, each packed `major << 24 | minor << 16 |
///                patch << 8` with every part in BCD (`00 00 09 01` is 1.9.0,
///                `00 00 14 05` is 5.14.0, per openMenu's `gdemu_sdk.c`; the
///                firmware's 5.20.0 is `00 00 20 05`, not 0x14). Cut by the
///                CDB's allocation length like the 0xE0 version reply.
///
/// Anything else is ILLEGAL REQUEST. Images are numbered folders in the root
/// of the card (`discimg::slot`).
const GDEMU_CMD: u8 = 0x52;
const GDEMU_MOUNT: u8 = 0x82;
const GDEMU_STEP: u8 = 0x81;
const GDEMU_STEP_NEXT: u8 = 0x55;
const GDEMU_STEP_PREV: u8 = 0x44;
const GDEMU_VERSION: u8 = 0x00;

/// The GDEMU protocol level NetGD emulates, as (major, minor, patch).
/// Reported as the firmware version of `52 00`, BCD-encoded
/// (`discimg::gdemu::version_word`); the bootloader reports 0.0.0, or the
/// resident's version behind one (`resident::bootloader_version_word`).
const GDEMU_COMPAT_VERSION: (u8, u8, u8) = (5, 20, 0);

/// Resolve a GDEMU target folder -- number `n`, or one step from the mounted
/// folder -- into `MOUNT_NAME`. Done in a plain function and through a static
/// so the 100-odd byte name never lives in the async frame: the IDE task's
/// stack is the scarce thing, and this runs ahead of the deepest path.
#[inline(never)]
fn stage_folder(n: Option<u16>, dir: Option<discimg::slot::Step>) -> bool {
    let blocks = disk_sectors();
    let found = match (n, dir) {
        (Some(n), _) => crate::gdi::folder_for_number(blocks, n),
        (None, Some(dir)) => crate::gdi::step_folder(blocks, dir),
        (None, None) => None,
    };
    let Some(found) = found else { return false };
    // A folder with nothing to mount would empty the drive and leave the
    // button dead (F1: it is off while no medium is mounted). `step_folder`
    // already vets its pick; a number named outright is vetted here.
    if n.is_some() && !crate::gdi::folder_has_image(blocks, found.as_str()) {
        return false;
    }
    let name = unsafe { &mut *core::ptr::addr_of_mut!(MOUNT_NAME) };
    name.clear();
    let _ = name.push_str(found.as_str());
    true
}

async fn handle_gdemu_cmd(
    dev: &IdeDevice,
    state: &mut DeviceState,
    label: &'static str,
    packet: &[u8; 12],
) {
    match packet[1] {
        GDEMU_MOUNT | GDEMU_STEP if write_session_active() => {
            warn!("{}: GDEMU mount refused during a write session", label);
            finish_illegal(dev, state);
        }
        GDEMU_MOUNT => {
            let n = (packet[2] as u16) | ((packet[3] as u16) << 8);
            if n == 0 {
                crate::gdi::select_default();
                // "A failed mount is ILLEGAL REQUEST" holds for the default
                // too (it used to report OK over an empty drive, R89 A8).
                if swap_media(state, label).await {
                    finish_ok(dev);
                } else {
                    finish_illegal(dev, state);
                }
                return;
            }
            // Resolve first: an unknown number, or a folder with no image in
            // it, must not disturb what is mounted.
            if !stage_folder(Some(n), None) {
                warn!("{}: GDEMU mount: no mountable folder numbered {}", label, n);
                finish_illegal(dev, state);
                return;
            }
            let name = unsafe { &*core::ptr::addr_of!(MOUNT_NAME) };
            let ok = mount_named(state, label, name.as_str()).await;
            // `0x52` stages through the same buffer as MOUNT_NAME: leave
            // nothing behind for a later WSESSION_END 2 to mount.
            clear_mount_name();
            if ok {
                finish_ok(dev);
            } else {
                finish_illegal(dev, state);
            }
        }
        GDEMU_STEP => {
            let dir = match packet[2] {
                GDEMU_STEP_NEXT => discimg::slot::Step::Next,
                GDEMU_STEP_PREV => discimg::slot::Step::Prev,
                _ => {
                    finish_illegal(dev, state);
                    return;
                }
            };
            if !stage_folder(None, Some(dir)) {
                warn!("{}: GDEMU step: no numbered folder with an image", label);
                finish_illegal(dev, state);
                return;
            }
            let name = unsafe { &*core::ptr::addr_of!(MOUNT_NAME) };
            let ok = mount_named(state, label, name.as_str()).await;
            // `0x52` stages through the same buffer as MOUNT_NAME: leave
            // nothing behind for a later WSESSION_END 2 to mount.
            clear_mount_name();
            if ok {
                finish_ok(dev);
            } else {
                finish_illegal(dev, state);
            }
        }
        GDEMU_VERSION => {
            let (ma, mi, pa) = GDEMU_COMPAT_VERSION;
            let fw = discimg::gdemu::version_word(ma, mi, pa);
            let mut out = [0u8; 8];
            // Bootloader in the first word (the resident's version when
            // there is one, else 0.0.0), firmware in the second.
            #[cfg(all(feature = "resident", not(feature = "sam3u2e")))]
            out[..4].copy_from_slice(&crate::resident::bootloader_version_word().to_le_bytes());
            out[4..].copy_from_slice(&fw.to_le_bytes());
            send_alloc(dev, packet, &out).await;
        }
        sub => {
            warn!("{}: GDEMU sub-command 0x{:02x} not supported", label, sub);
            finish_illegal(dev, state);
        }
    }
}

/// Sub-commands of [`OGDM_IMAGE_CTL`].
const SUB_MOUNT: u8 = 2;
const SUB_VERSION: u8 = 3;
const SUB_MOUNT_NAME: u8 = 5;

/// The write session (F6-F8). Only `ide_task` touches it.
static mut WSESSION: discimg::wsession::WriteSession = discimg::wsession::WriteSession::new();

fn wsession() -> &'static mut discimg::wsession::WriteSession {
    unsafe { &mut *core::ptr::addr_of_mut!(WSESSION) }
}

fn write_session_active() -> bool {
    wsession().active()
}

/// The folder name staged by [`SUB_MOUNT_NAME`] (or by `0x52` through
/// `stage_folder`), mounted by [`SUB_MOUNT`] or WSESSION_END 2. Cleared when
/// consumed and by WSESSION_BEGIN.
static mut MOUNT_NAME: heapless::String<{ gamecfg::MAX_NEXT }> = heapless::String::new();

/// After `fat::partition_start` filled the loader scratch with sector 0:
/// whether that is a GPT protective MBR (entry 0 of type 0xEE). The F7
/// boundary cannot be taken from it, so BEGIN refuses. `part == 0` means no
/// partition entry or a superfloppy (`partition_start` checked the BPB).
fn gpt_card(part: u32) -> bool {
    if part == 0 {
        return false;
    }
    match loader_scratch().first_chunk_mut::<512>() {
        Some(sec) => discimg::wsession::is_gpt_protective(sec, false),
        None => false,
    }
}

fn clear_mount_name() {
    unsafe { (*core::ptr::addr_of_mut!(MOUNT_NAME)).clear() };
}



pub async fn prefetch_ip_bin(gdi: &'static crate::gdi::Gdi) {
    let lba = gdi.boot_area_lba;
    // The BIOS asks for 7 sectors; cache whatever one staging round holds
    // so its first chunk needs no card access at all.
    let count: u32 = 7.min(stage_sectors_for(gdi, lba));
    info!("prefetching IP.BIN: LBA={} count={}", lba, count);
    match fill_cd_read_buf_from_sd(gdi, lba, count).await {
        Ok(n) => info!("  IP.BIN cached: {} bytes", n),
        Err(()) => warn!("prefetch_ip_bin: SD read failed"),
    }
}

async fn stream_from_sd(
    dev: &IdeDevice,
    gdi: &'static crate::gdi::Gdi,
    start_lba: u32,
    count: u32,
    total_bytes: usize,
    dma: bool,
    byte_limit: u16,
) -> bool {
    // Returns whether the card failed under the read (an error status has
    // been posted then, unless the command was already dead).
    //
    // CD_READ_BUF is a staging window, not a size limit. Requests larger
    // than it are streamed: stage a chunk, push it, refill, repeat. The
    // host tolerates the refill gaps because negating DMARQ is flow
    // control, not an error -- ATA/ATAPI-5 10.2.3 has the host pause and
    // wait indefinitely for the device to reassert it, and the FPGA
    // reasserts automatically once the FIFO refills (ide_dmarq is
    // combinational on read_fifo.r_rdy). Confirmed on hardware: the DC sat
    // through a 2 s stall without giving up.
    let stage_sectors = stage_sectors_for(gdi, start_lba);
    if count > stage_sectors {
        info!(
            "  CD_READ streaming: {} sectors in chunks of {}",
            count, stage_sectors
        );
    }
    if !dma {
        return stream_pio_from_sd(dev, gdi, start_lba, count, total_bytes, byte_limit).await;
    }

    // Stage the first chunk before arming DMA so the FIFO is primed and
    // DMARQ is already high when the host's channel starts.
    let first = count.min(stage_sectors);
    let staged = stage_chunk(gdi, start_lba, first).await;
    // The card read can take long enough for the host to give up and reset
    // the device or write a new command. Nothing is armed yet, and nothing
    // below may be written over what the host has moved on to (R89 A1).
    if dev.aborted() {
        info!("  CD_READ dead before its data phase; no completion");
        return false;
    }
    if !staged {
        warn!("stream_from_sd: SD read failed");
        dev.set_irr(0x03);
        dev.set_error(ERR_MEDIUM);
        dev.set_status_intrq(STATUS_IDLE_ERR);
        return true;
    }

    #[cfg(feature = "dma-pattern")]
    fill_pattern(0, (first as usize) * 2048);

    let staged_bytes = (first as usize) * 2048;
    let buf = unsafe { &mut CD_READ_BUF[..staged_bytes] };
    let mut hash = fnv1a(FNV_INIT, &buf[..]);
    info!("  staged {} of {} bytes; arming DMA", staged_bytes, total_bytes);
    if total_bytes >= 16 {
        info!(
            "  hdr [0..16]: {:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
            buf[8], buf[9], buf[10], buf[11], buf[12], buf[13], buf[14], buf[15]
        );
    }

    // Pre-prime the FIFO before notifying the host. Without this the
    // host's DMA controller starts on an empty FIFO, only sees DMARQ
    // assert as we trickle words in, and drops the channel partway
    // through the burst when a momentary push/pull mismatch underruns
    // the FIFO. Filling the FIFO first means DMARQ is already high
    // when the host's DMA channel arms, and there's a 4 KB cushion
    // before any underrun risk.
    let push_reg_pre = unsafe { dev.base.add(REG_DATA_FIFO) };
    let prime_words = (dev.fifo_depth_words() as usize).saturating_sub(8);
    let prime_bytes = (prime_words * 2).min(buf.len() & !1);
    let mut primed = 0usize;
    while primed + 1 < prime_bytes {
        let val = (buf[primed] as u16) | ((buf[primed + 1] as u16) << 8);
        unsafe { write_volatile(push_reg_pre, val) };
        primed += 2;
    }

    // Verify FIFO actually received data (sanity check for debugging DMA issues)
    let level_after_prime = dev.read_fifo_level();
    info!(
        "  FIFO primed: {} bytes -> level={}",
        primed,
        level_after_prime
    );
    if level_after_prime == 0 && primed > 0 {
        warn!("FIFO level is 0 after pushing {} bytes - SMC/FPGA issue?", primed);
    }

    // Phase 2: set up the data-in envelope and arm DMA, then notify
    // the host (INTRQ). The host reads STATUS, sees the data-phase
    // setup, and its DMA controller starts pulling on DMARQ.
    // The ATAPI byte-count limit must be even -- it counts bytes on a 16-bit
    // bus, and an odd value is not representable as whole words. Clamping a
    // large request with min(0xFFFF) produced 65535, and the host then never
    // started its DMA at all: a 572-sector (1.1 MB) read sat with our FIFO
    // full and zero bytes pulled, while a 16-sector read (byte count 32768,
    // even) worked. Clamp to 0xFFFE and force even.
    dev.set_atapi_byte_count((total_bytes.min(0xFFFE) & !1) as u16);
    dev.set_irr(0x02);
    dev.set_error(0);
    if dma {
        // Hold BSY for the whole DMA data phase and do not interrupt at its
        // start. The BIOS polls Status while its G1 DMA runs, and a status
        // of 0x50 (BSY=0, DRQ=0) reads as "command finished": it aborts the
        // DMA mid-burst and moves on. Transfers that fit one FIFO prime
        // drained before the next poll and looked fine; anything needing an
        // SD refill mid-transfer (a cold 9-sector IP.BIN read, the game's
        // 572-sector 1ST_READ.BIN) was cut at whatever word the poll landed
        // on -- 5308 and 4678 words, not burst multiples. DRQ=1 is not the
        // answer either: the BIOS then treats the phase as PIO and refuses.
        // iceGDROM does exactly this (packet_data_dma leaves BSY set from
        // the CDB and only finish_packet writes 0x50 + INTRQ).
        // Do NOT interrupt at the start of the DMA data phase. Tried and
        // measured: interrupting here while keeping BSY set (so the host can
        // never read 0x50 and mistake it for completion) does not help and
        // makes things much worse -- the host pulled 44 bytes of an 8228-byte
        // prime, push_buf_to_fifo hit its 10 s timeout twice, and the console
        // never got past IP.BIN's first chunk and fell back to the CD audio
        // player. The interrupt itself is the hazard here, not the cleared
        // BSY, so the "don't interrupt, hold BSY, let the host poll" rule
        // below is the real constraint.
        dev.set_status(STATUS_BSY | STATUS_DRDY | STATUS_DSC);
        dev.set_dma_mode(DmaMode::ToHost);
    } else {
        dev.set_status_intrq(STATUS_DRDY | STATUS_DSC | STATUS_DRQ);
    }

    // Push the balance of the primed chunk, then stage and push the rest of
    // the request a chunk at a time. push_buf_to_fifo blocks on FIFO
    // fullness, so this naturally paces to whatever rate the host drains at.
    let first_rest = buf.len() - primed;
    let mut pushed_bytes = primed + push_buf_to_fifo(dev, &buf[primed..]).await;
    let mut pushed_all = pushed_bytes == primed + first_rest;
    let mut sectors_done = first;
    let mut sd_failed = false;
    // A reset or a new command to this device ends the loop through
    // `push_buf_to_fifo`, which then pushes nothing and returns short.
    while pushed_all && sectors_done < count {
        let this = (count - sectors_done).min(stage_sectors);
        if !stage_chunk(gdi, start_lba + sectors_done, this).await {
            warn!("stream_from_sd: SD read failed mid-stream at sector {}", sectors_done);
            sd_failed = true;
            break;
        }
        #[cfg(feature = "dma-pattern")]
        fill_pattern((sectors_done as usize) * 2048, (this as usize) * 2048);

        let chunk = unsafe { &CD_READ_BUF[..(this as usize) * 2048] };
        hash = fnv1a(hash, chunk);
        let n = push_buf_to_fifo(dev, chunk).await;
        pushed_bytes += n;
        pushed_all = n == chunk.len();
        sectors_done += this;
    }
    if !pushed_all {
        info!("  host stopped draining mid-stream (lvl={})", dev.read_fifo_level());
    }
    if sectors_done < count {
        warn!("  short transfer: {} of {} sectors staged", sectors_done, count);
    }

    let level_before_drain = dev.read_fifo_level();
    info!("  staged; awaiting drain (lvl={})", level_before_drain);

    // Wait for the host to finish draining. Use coarse 1 ms granularity
    // and cap at 200 ms — if the host hasn't drained by then it's not
    // going to, and we want STATUS to transition out of DRQ promptly so
    // the BIOS can move on instead of hanging at "Please wait".
    //
    // For DMA mode, we must wait at least a minimum time (100ms) even if
    // the FIFO appears empty, to give the DC time to trigger its G1 DMA
    // controller. Without this, we may complete the command before the
    // DC's DMA has even started. The DC needs to:
    //   1. See INTRQ / poll for IRR=0x02
    //   2. Configure G1 DMA registers
    //   3. Trigger DMA (write to DMA_STATUS)
    // All of this takes time on the DC side. Only start checking FIFO level
    // after the minimum wait has elapsed.
    //
    // The floor is skipped when the host has demonstrably finished. An empty
    // FIFO is ambiguous on its own -- it can mean "host hasn't started" --
    // but an empty FIFO *after* we primed it and pushed every byte is not:
    // the host took them. Waiting out the floor there holds BSY for 100 ms
    // after the transfer is over, and the BIOS soft-resets the drive rather
    // than wait for the completion. That is what killed the boot: the read
    // of 1ST_READ.BIN drained fully in 49 ms and then hit
    // `host reset during CD_READ; no completion`, three times, before the
    // console gave up and returned to its menu.
    let host_took_everything = pushed_all && primed > 0 && dev.read_fifo_level() == 0;
    let min_wait_ms = if dma && !host_took_everything { 100u32 } else { 0u32 };
    // Progress is logged whenever the level actually moves. Measured: the
    // host does NOT resume after a stall -- raising this cap to 2000 ms
    // changed nothing, the level stayed frozen for the full two seconds. So a
    // stall here means the host abandoned the DMA, not that it paused.
    let mut last_lvl = u16::MAX;
    let mut stalled_ms = 0u32;
    // Feature `fast-complete`: the floor above guards against an empty FIFO
    // meaning "the host has not started yet". A FIFO seen *falling* to empty
    // does not mean that -- the host demonstrably pulled everything -- so the
    // floor is waived once the level has been observed to drop. The floor is
    // otherwise paid on every DMA CD_READ whose tail was still in the FIFO
    // when the last push returned, i.e. nearly all of them.
    //
    // Two guards, because a wrong "empty" here completes the command while
    // the host's DMA is still reading (a short or corrupt transfer):
    //  * `peak` starts at the level after the prime, not 0. The host can drain
    //    the primed bytes before the first sample, and a peak that starts at 0
    //    would then never see a drop.
    //  * The level is read by the SMC asynchronously to the FPGA's counter, so
    //    a single read can be torn at a power-of-two boundary (0x400 -> 0x3FF
    //    reading as 0). The waiver therefore needs two zero reads in a row, a
    //    millisecond apart.
    #[cfg(feature = "fast-complete")]
    let (mut peak, mut pulled, mut zeros) = (level_after_prime, false, 0u8);
    for i in 0..200u32 {
        // The host's Command write emptied the FIFO; that is not a drain.
        if dev.aborted() {
            break;
        }
        let lvl = dev.read_fifo_level();
        #[cfg(feature = "fast-complete")]
        {
            if lvl > peak {
                peak = lvl;
            } else if lvl < peak {
                pulled = true;
            }
            zeros = if lvl == 0 { zeros.saturating_add(1) } else { 0 };
        }
        #[cfg(feature = "fast-complete")]
        let waived_done = pulled && zeros >= 2;
        #[cfg(not(feature = "fast-complete"))]
        let waived_done = false;
        if (i >= min_wait_ms && lvl == 0) || waived_done { break; }
        if lvl != last_lvl {
            if stalled_ms >= 20 {
                info!("  drain: resumed after {} ms stall (lvl={})", stalled_ms, lvl);
            }
            last_lvl = lvl;
            stalled_ms = 0;
        } else {
            stalled_ms += 1;
        }
        Timer::after(Duration::from_millis(1)).await;
    }
    if dev.aborted() {
        // Reset, or a new command to this device (R89 A1): the byte count,
        // interrupt reason and Status are no longer ours. The FPGA dropped
        // DMA mode itself on a reset but not on a Command write.
        info!("  host reset or new command during CD_READ; no completion");
        dev.disarm();
        return false;
    }
    let leftover = dev.read_fifo_level();
    // Count against what we actually managed to stage, not the request: on a
    // mid-stream SD failure `sectors_done` stops short of `count`, and
    // charging the shortfall against total_bytes would overstate delivery.
    // Count against bytes that actually reached the FIFO. Using the staged
    // total overstates delivery whenever push_buf_to_fifo bailed out early --
    // that is what made a transfer the host never touched report
    // "delivered=6152" instead of 0.
    let staged_total = pushed_bytes;
    let delivered = staged_total.saturating_sub((leftover as usize) * 2);
    info!(
        "  drain done (lvl={}, delivered={}/{} of {} requested) fnv1a={:08x}",
        leftover, delivered, staged_total, total_bytes, hash
    );
    // Disarm DMA mode now that the host has drained as much as it
    // wanted. If we leave it armed and the FIFO has stray data left,
    // DMARQ would re-assert and confuse the host.
    if dma {
        dev.set_dma_mode(DmaMode::Off);
    }
    // ATAPI short-transfer signal: byte_count register at command
    // completion reflects the actual bytes delivered to the host (vs
    // the full request we set up at data-phase start). Without this
    // the BIOS keeps thinking the transfer is still in flight.
    dev.set_atapi_byte_count((delivered.min(0xFFFE) & !1) as u16);
    dev.set_irr(0x03);
    if sd_failed {
        // The card failed mid-stream: the host got fewer sectors than it
        // asked for and must be told so, as the PIO path does. This used to
        // complete with a clean status (R89 A2).
        dev.complete(ERR_MEDIUM, STATUS_IDLE_ERR);
    } else {
        dev.complete(0, STATUS_IDLE);
    }
    #[cfg(all(feature = "sd-multiblock", not(feature = "dma-pattern")))]
    {
        unsafe { STREAM_OK = pushed_all && sectors_done == count };
    }
    info!("CD_READ complete ({} bytes)", delivered);
    sd_failed
}

/// PIO CD_READ: the payload goes to the host in ATAPI DRQ blocks.
///
/// Each block is announced by its own byte count + DRQ + INTRQ, the host
/// reads exactly that many words, and the device shows BSY until the next
/// block (or the completion status) is ready. Presenting the whole request
/// as one DRQ block with byte count 0xFFFE did not work: Sega Rally 2's
/// driver read 32767 words per DRQ block it saw, straight through our
/// refill gaps (there is no IORDY pacing in PIO mode), and got junk.
/// A block never exceeds the host's byte-count limit from the command
/// (0xF000 for that driver) nor what is staged in CD_READ_BUF, so a block is
/// at most 5-7 sectors (see `stage_sectors_for`). The FPGA's read flow control raises BSY the instant
/// the host pops a block's last word, so the host cannot mistake the gap
/// before we re-arm DRQ for command completion.
async fn stream_pio_from_sd(
    dev: &IdeDevice,
    gdi: &'static crate::gdi::Gdi,
    start_lba: u32,
    count: u32,
    total_bytes: usize,
    byte_limit: u16,
) -> bool {
    let stage_sectors = stage_sectors_for(gdi, start_lba);
    // ATAPI: a limit of 0 (or 0xFFFF) means the host will take anything.
    let limit = match (byte_limit & !1) as usize {
        0 => 0xFFFE,
        l => l,
    };
    let prime_max = (dev.fifo_depth_words() as usize).saturating_sub(8) * 2;

    let t0 = Instant::now();
    let mut hash = FNV_INIT;
    let mut sent = 0usize;
    let mut blocks = 0u32;
    let mut chunk_start = 0u32;
    let mut chunk_sectors = 0u32;
    let mut ok = true;
    let mut sd_failed = false;

    dev.set_irr(0x02);
    dev.set_error(0);
    dev.set_pio_flow_control(true, false);
    while sent < total_bytes {
        // Stage the chunk holding the next block. Blocks are cut at chunk
        // boundaries, so a block always lies inside the staged window.
        let sector = (sent / 2048) as u32;
        if chunk_sectors == 0 || sector >= chunk_start + chunk_sectors {
            chunk_start = sector;
            chunk_sectors = (count - sector).min(stage_sectors);
            if !stage_chunk(gdi, start_lba + sector, chunk_sectors).await {
                warn!("stream_pio_from_sd: SD read failed at sector {}", sector);
                sd_failed = true;
                ok = false;
                break;
            }
            let chunk = unsafe { &CD_READ_BUF[..(chunk_sectors as usize) * 2048] };
            hash = fnv1a(hash, chunk);
        }
        // Before announcing another block: a reset, or a new command to this
        // device, since the last one (the card read above gives it time).
        // The Status, byte count and DRQ written below would land on the new
        // command's CDB phase (R89 A1).
        if dev.aborted() {
            ok = false;
            break;
        }
        let off = sent - (chunk_start as usize) * 2048;
        let block = (total_bytes - sent)
            .min(limit)
            .min((chunk_sectors as usize) * 2048 - off)
            & !1;
        let data = unsafe { &CD_READ_BUF[off..off + block] };

        // Announce the block only once the FIFO holds as much of it as fits;
        // the balance streams behind the host's reads. push_buf_to_fifo
        // would otherwise wait on room the host has no reason to make yet.
        dev.set_status(STATUS_BSY | STATUS_DRDY | STATUS_DSC);
        dev.set_atapi_byte_count(block as u16);
        let prime = block.min(prime_max);
        if push_buf_to_fifo(dev, &data[..prime]).await != prime {
            ok = false;
            break;
        }
        dev.set_status_intrq(STATUS_DRDY | STATUS_DSC | STATUS_DRQ);
        if push_buf_to_fifo(dev, &data[prime..]).await != block - prime {
            warn!("  host stopped draining block {} (lvl={})", blocks, dev.read_fifo_level());
            ok = false;
            break;
        }
        // The host owes us exactly `block` bytes of reads; the FPGA shows BSY
        // from the last pop until we re-arm DRQ above or complete below.
        if let Err(why) = wait_host(dev, || dev.read_fifo_level() == 0, 10_000).await {
            if why == Stall::Timeout {
                warn!("  host left block {} undrained (lvl={}, {} of {} bytes sent)",
                      blocks, dev.read_fifo_level(), sent, total_bytes);
                dev.log_host_activity("pio block stall");
            }
            ok = false;
            break;
        }
        sent += block;
        blocks += 1;
    }
    let leftover = dev.read_fifo_level() as usize * 2;
    let delivered = sent.saturating_sub(leftover);
    if dev.reset_seen() || dev.command_pending() {
        // The host reset the device, or moved on (its Command write is what
        // emptied the FIFO). Nothing may be written to Status now -- see
        // `complete` -- but flow control is still armed after a Command
        // write, and would show BSY over the new command's CDB phase.
        info!("  host reset or abandoned CD_READ after {} of {} bytes; no completion",
              delivered, total_bytes);
        dev.disarm();
        return false;
    }
    // Hold BSY (and drop DRQ) *before* releasing flow control: the FPGA's
    // synthetic BSY only lasts while pio_rd_fc is set, so releasing first
    // would show 0x58 with an empty FIFO until the completion write below.
    dev.set_status(STATUS_BSY | STATUS_DRDY | STATUS_DSC);
    dev.set_pio_flow_control(false, false);
    info!(
        "  PIO done: {} blocks, delivered={}/{} in {} ms fnv1a={:08x}",
        blocks, delivered, total_bytes, (Instant::now() - t0).as_millis(), hash
    );
    dev.set_atapi_byte_count((delivered.min(0xFFFE) & !1) as u16);
    dev.set_irr(0x03);
    if ok {
        dev.complete(0, STATUS_IDLE);
        #[cfg(all(feature = "sd-multiblock", not(feature = "dma-pattern")))]
        {
            unsafe { STREAM_OK = true };
        }
    } else {
        dev.complete(if sd_failed { ERR_MEDIUM } else { ERR_ABRT }, STATUS_IDLE_ERR);
    }
    info!("CD_READ complete ({} bytes)", delivered);
    sd_failed
}


/// Push `data` into the read FIFO via SAM3U DMAC, throttling on FIFO
/// fullness. Returns the bytes pushed: short if the host stopped draining
/// (FIFO full for 5 s), reset the bus, or wrote a command to either device
/// (see `wait_host`), so the caller can wrap up instead of stalling. A
/// caller that gets a short count asks `IdeDevice::aborted` whether it may
/// still write Status.
///
/// DMAC pushes ~16+ MB/s steady-state, matching the DC's host DMA
/// pull rate — without it the CPU `write_volatile` loop jittered with
/// embassy task scheduling and the first CD_READ DMA underran
/// inconsistently (8/13/14 KB delivered). With DMAC the FIFO stays
/// near-full and DMARQ never blips low mid-transfer.
async fn push_buf_to_fifo(dev: &IdeDevice, data: &[u8]) -> usize {
    let mut i = 0;
    let push_reg = unsafe { dev.base.add(REG_DATA_FIFO) };
    // Leave 4 words of headroom — DMAC writes immediately on arm,
    // we don't want to race the FPGA's r_rdy edge.
    let room = || dev.fifo_depth_words().saturating_sub(dev.read_fifo_level_hi() + 4) as usize;
    while i + 1 < data.len() {
        // A full FIFO drains in ~130 us at host DMA rate; sleeping here
        // cost a whole 1 ms tick per refill and was most of a CD_READ.
        // 5 s, not 10: the host's own patience is far shorter than either.
        // The longest legitimate stall measured is the console descrambling a
        // MIL-CD's 1ST_READ.BIN as it pulls, at 2.16 s -- and when the host
        // has genuinely gone the extra five seconds buy nothing but a job
        // that takes longer to fail.
        if let Err(why) = wait_host(dev, || room() > 0, 5_000).await {
            if why == Stall::Timeout {
                warn!("push_buf_to_fifo: FIFO never drained after {} bytes", i);
            }
            return i;
        }
        let room_words = room();
        let words_left = (data.len() - i) / 2;
        let n = room_words.min(words_left);
        // Source pointer needs to be halfword-aligned and stable for
        // the duration of the DMAC burst. CD_READ_BUF is byte-aligned
        // but `i` is always advanced by 2, so the running pointer is
        // halfword-aligned. Each transfer reads `n` halfwords from the
        // RAM buffer to the SMC FIFO write register at a fixed address.
        let src_ptr = unsafe { data.as_ptr().add(i) } as *const u16;
        unsafe {
            embassy_atsam3::dmac::push_halfwords(src_ptr, push_reg, n as u32);
        }
        // Wait for the DMAC burst to complete before the next FIFO-
        // level read; otherwise we'd see stale level and overshoot.
        // Future streaming refactor will overlap DMAC with the next
        // SD read instead of sleeping here.
        embassy_atsam3::dmac::wait_done();
        i += n * 2;
    }
    // Trailing odd byte (rare; CD sectors are always even).
    if i < data.len() {
        let val = data[i] as u16;
        unsafe { write_volatile(push_reg, val) };
    }
    i
}

#[allow(dead_code)]
async fn stream_pattern(dev: &IdeDevice, byte_count: usize) {
    // Push a sliding incrementing pattern, refilling the FIFO whenever
    // it has room. The primary FIFO is 4096 words = 8 KB, so a single 2 KB
    // sector we fit it all up-front.
    dev.set_atapi_byte_count(byte_count.min(0xFFFF) as u16);
    dev.set_error(0);
    dev.set_status_intrq(STATUS_DRDY | STATUS_DSC | STATUS_DRQ);

    let mut byte_idx: usize = 0;
    let mut waited = 0u32;
    while byte_idx < byte_count {
        // FIFO has room? Push another word.
        if dev.read_fifo_level() < (dev.fifo_depth_words() - 4) {
            let lo = (byte_idx as u8) as u16;
            let hi = ((byte_idx + 1) as u8) as u16;
            dev.push_read_fifo(lo | (hi << 8));
            byte_idx += 2;
            waited = 0;
        } else {
            if waited >= 10_000 {
                warn!("stream_pattern: FIFO never drained, abandoning");
                break;
            }
            Timer::after(Duration::from_micros(100)).await;
            waited += 1;
        }
    }

    // Wait for the host to finish draining what we staged.
    let mut empty_waited = 0u32;
    while dev.read_fifo_level() > 0 {
        if empty_waited >= 10_000 {
            warn!("stream_pattern: FIFO drain timeout at end");
            break;
        }
        Timer::after(Duration::from_micros(100)).await;
        empty_waited += 1;
    }
    dev.set_status_intrq(STATUS_IDLE);
}

/// Words deep in the FPGA read FIFO (MCU->IDE): primary 4096 (8 KB),
/// secondary 1024 (2 KB, four sectors). Deeper is better for DMA: the host
/// can pull at ~60x the rate the MCU can push, so a deeper FIFO rides out
/// the gap between push iterations without dropping DMARQ.
/// Mirrors READ_FIFO_DEPTH / SECONDARY_READ_FIFO_DEPTH in
/// fpga/amaranth/hw/ide_device.py. Keep them in step.
const PRIMARY_FIFO_DEPTH_WORDS: u16 = 4096;
const SECONDARY_FIFO_DEPTH_WORDS: u16 = 1024;

async fn recv_packet(dev: &IdeDevice) -> Result<[u8; PACKET_LEN_BYTES], ()> {
    // Wait for the host to write all 6 words. Yield-only: a sleep here is
    // a 1 ms tick (see `poll_until`), which was most of a PACKET command's
    // latency. A real drive has no CDB timeout; Sega Rally 2's WinCE
    // driver polls at ~50 ms and took >100 ms to get round to the CDB of
    // its first PIO read, which the old 100 ms cap turned into ABRT. Only
    // a reset or a command to the other device abandons the wait early.
    // Timestamp the host's first few Status/Alt Status reads (1 ms
    // resolution) so a host that stops before the CDB tells us whether it
    // polled for DRQ and gave up, or peeked once and went to sleep.
    let t0 = Instant::now();
    let mut last_st = dev.read(REG_HOST_STATUS_RDS);
    let mut marks: [(u32, u16); 8] = [(u32::MAX, 0); 8];
    let mut n_marks = 0usize;
    let r = wait_host(dev, || {
        let st = dev.read(REG_HOST_STATUS_RDS);
        if st != last_st {
            last_st = st;
            if n_marks < marks.len() {
                marks[n_marks] = ((Instant::now() - t0).as_millis() as u32, st);
                n_marks += 1;
            }
        }
        dev.write_fifo_level() >= PACKET_LEN_WORDS as u16
    }, 10_000).await;
    if r.is_err() {
        warn!("recv_packet: host reads (ms, st|alt<<8) before giving up: {:?}", &marks[..n_marks]);
        return Err(());
    }
    let mut bytes = [0u8; PACKET_LEN_BYTES];
    for i in 0..PACKET_LEN_WORDS {
        let w = dev.pop_write_fifo();
        bytes[2 * i] = (w & 0xFF) as u8;
        bytes[2 * i + 1] = (w >> 8) as u8;
    }
    Ok(bytes)
}

/// Drain `len` bytes of an ATAPI data-out phase from the host. Sets
/// up DRQ + IRR=0 (host-to-drive direction), pulses INTRQ, then waits
/// for the FPGA's write FIFO to receive the expected words. Returns
/// the bytes (or empty on timeout). Caller is responsible for the
/// command-complete (IRR=0x03 + STATUS_IDLE) status phase afterward.
async fn recv_data_out(dev: &IdeDevice, len: usize) -> heapless::Vec<u8, 64> {
    let mut out = heapless::Vec::<u8, 64>::new();
    // Nothing to receive: no data phase at all. Raising DRQ for it would
    // have the host see "send me data" for a transfer of nothing, plus an
    // interrupt ahead of the caller's completion one (R89 A8).
    if len == 0 {
        return out;
    }
    let words = (len + 1) / 2;
    dev.set_atapi_byte_count(len as u16);
    // Interrupt Reason: data-out phase (I/O=0, C/D=0).
    dev.set_irr(0x00);
    dev.set_error(0);
    dev.set_status_intrq(STATUS_DRDY | STATUS_DSC | STATUS_DRQ);

    if let Err(why) = wait_host(dev, || dev.write_fifo_level() >= words as u16, 10_000).await {
        if why == Stall::Timeout {
            warn!("recv_data_out: host never wrote {} words", words);
        }
        return out;
    }
    for _ in 0..words {
        let w = dev.pop_write_fifo();
        let _ = out.push((w & 0xFF) as u8);
        if out.len() < len {
            let _ = out.push((w >> 8) as u8);
        }
    }
    out
}

async fn send_data_in(dev: &IdeDevice, data: &[u8]) {
    // An allocation length of 0 is "transfer nothing", not an error: go
    // straight to the status phase. Opening a data phase for it raised DRQ
    // over an empty FIFO and interrupted twice (R89 A8).
    if data.is_empty() {
        finish_ok(dev);
        return;
    }
    // Stage the response in the read FIFO. Pad to even byte count by
    // emitting a zero high byte if the response is odd-length.
    let mut i = 0;
    while i < data.len() {
        let lo = data[i] as u16;
        let hi = if i + 1 < data.len() { data[i + 1] as u16 } else { 0 };
        dev.push_read_fifo(lo | (hi << 8));
        i += 2;
    }
    let staged_level = dev.read_fifo_level();
    // Host reads so far (Status/Alt Status/Data) belong to the CDB phase;
    // anything above these afterwards is the host reacting to the data.
    let pre_st = dev.read(REG_HOST_STATUS_RDS);
    let pre_data = dev.read(REG_HOST_DATA_RDS);
    info!("send_data_in staged {} bytes; read_fifo level={} host reads so far st={} alt={} data={}",
          data.len(), staged_level, pre_st & 0xFF, pre_st >> 8, pre_data);
    dev.set_atapi_byte_count(data.len() as u16);
    // Interrupt Reason: data-in phase (I/O=1, C/D=0).
    dev.set_irr(0x02);
    dev.set_error(0);
        // Real ATAPI fires INTRQ when DRQ goes 0->1 (data ready). The host
    // reads STATUS to clear, then drains the data via the Data reg.
    dev.set_status_intrq(STATUS_DRDY | STATUS_DSC | STATUS_DRQ);

    // Wait for the host to drain the FIFO, then clear DRQ. Cap the wait
    // at ~1 s — if the host never reads, we abandon the transfer rather
    // than hang the IDE task.
    // A new command to either device ends the wait too: the FPGA clears the
    // FIFO on the command write anyway, and a host that took what it wanted
    // and moved on should not cost the next command ten seconds of BSY.
    // Sample the host's reads while waiting so a reply it never drains
    // still tells us when (if ever) it looked at Status afterwards.
    let t0 = Instant::now();
    let mut last_st = pre_st;
    let mut last_change_us: u32 = 0;
    let mut first_status_us: u32 = u32::MAX;
    let r = wait_host(dev, || {
        let st = dev.read(REG_HOST_STATUS_RDS);
        if st != last_st {
            last_st = st;
            last_change_us = (Instant::now() - t0).as_micros() as u32;
            // First Status read after INTRQ: microseconds = an ISR,
            // milliseconds = a per-frame poll.
            if first_status_us == u32::MAX && (st & 0xFF) > (pre_st & 0xFF) {
                first_status_us = last_change_us;
            }
        }
        dev.read_fifo_level() == 0 || dev.command_pending()
    }, 10_000).await;
    let waited_us = (Instant::now() - t0).as_micros() as u32;
    info!("send_data_in: waited {} us; first Status read at +{} us; last change at +{} us (st={} alt={})",
          waited_us, first_status_us, last_change_us, last_st & 0xFF, last_st >> 8);
    match r {
        Err(Stall::Timeout) => {
            warn!("send_data_in: host never drained FIFO, abandoning");
            dev.log_host_activity("send_data_in");
        }
        // The FPGA holds the post-reset state; a completion write here
        // would only leave a stale status and IRQ for the host to find.
        // After a new command the interrupt reason is that command's too.
        Err(Stall::Reset) | Err(Stall::OwnCommand) => return,
        _ => {}
    }
    // Command complete. IRR = 0x03 (C/D|I/O = 1|1) tells the host the
    // status phase begins and we're done.
    dev.set_irr(0x03);
    dev.complete(0, STATUS_IDLE);
}

/// Read `n` consecutive SD blocks at `block` into `dst` (`n * 512` bytes):
/// CMD18 for runs of at least `multi_min` blocks, CMD17 per block below that.
fn fetch_run(block: u32, n: u32, dst: &mut [u8], multi_min: u32) -> bool {
    if n >= multi_min {
        return unsafe { embassy_atsam3::hsmci::read_blocks_dma(block, dst) }.is_ok();
    }
    for i in 0..n as usize {
        let one: &mut [u8; 512] = (&mut dst[i * 512..(i + 1) * 512]).try_into().unwrap();
        if unsafe { embassy_atsam3::hsmci::read_block(block + i as u32, one) }.is_err() {
            return false;
        }
    }
    true
}

/// ATA READ SECTORS / READ DMA on the emulated disk, LBA28 or LBA48.
///
/// Sectors map straight onto SD card blocks -- the whole card is exposed, so
/// LBA n is card block n and the host sees the same filesystem that holds the
/// GD-ROM images. `ext` picks where the address and count come from; see
/// [`CommandSnapshot::lba`].
async fn handle_disk_read(
    dev: &IdeDevice,
    label: &'static str,
    snap: &CommandSnapshot,
    dma: bool,
    ext: bool,
) {
    if snap.device & 0x40 == 0 {
        // CHS addressing. Nothing we serve is CHS-shaped and every host we
        // care about uses LBA, so refuse rather than silently mis-address.
        warn!("{}: disk read in CHS mode, refusing", label);
        dev.set_error(ERR_ABRT);
        dev.set_status_intrq(STATUS_IDLE_ERR);
        dev.ack_command();
        return;
    }
    // The card is addressed in 32-bit blocks, so a 48-bit LBA above that is
    // unreachable however big the card is. Refuse rather than truncate: a
    // silently wrapped address reads the wrong sector and looks like
    // corruption much later.
    let Ok(lba) = u32::try_from(snap.lba(ext)) else {
        warn!("{}: disk read at LBA {} beyond 32-bit block addressing", label, snap.lba(ext));
        dev.set_error(ERR_ABRT);
        dev.set_status_intrq(STATUS_IDLE_ERR);
        dev.ack_command();
        return;
    };
    let count = snap.count(ext);

    let total = disk_sectors();
    if total == 0 || lba.saturating_add(count) > total {
        warn!("{}: disk read {} sectors at LBA {} past end ({})", label, count, lba, total);
        dev.set_error(ERR_ABRT);
        dev.set_status_intrq(STATUS_IDLE_ERR);
        dev.ack_command();
        return;
    }
    info!("{}: disk read LBA={} count={} dma={}", label, lba, count, dma);

    // Sectors are fetched in runs of up to `chunk` through CD_READ_BUF with
    // CMD18 rather than one CMD17 each: per-block command overhead, not the
    // bus, is what capped a 64-sector read at ~35 ms. The buffer is the
    // GD-ROM's staging cache, but `ide_task` services one device at a time,
    // so borrowing it here is safe; the cost is that the next CD_READ misses
    // and re-reads from SD, which the host never notices since it does not
    // interleave disk and GD-ROM traffic. Short runs stay on CMD17 -- CMD18's
    // CMD12 tail makes it slower than single-block reads below ~8 blocks.
    const CHUNK: u32 = (CD_READ_BUF_BYTES / 512) as u32;
    const MULTI_MIN: u32 = 8;
    invalidate_cd_read_cache();
    let buf = unsafe { &mut CD_READ_BUF[..] };

    // Stage the first block *before* raising DRQ. The host starts reading the
    // moment DRQ goes high, so announcing readiness over an empty FIFO hands
    // it whatever the data path last held -- the first attempt at this
    // returned 256 copies of IDENTIFY word 0. send_data_in has the same
    // ordering for the same reason.
    let mut ok = true;
    let mut run = count.min(CHUNK);
    if !fetch_run(lba, run, &mut buf[..run as usize * 512], MULTI_MIN) {
        warn!("{}: SD read failed at block {}", label, lba);
        dev.set_error(ERR_ABRT);
        dev.set_status_intrq(STATUS_IDLE_ERR);
        dev.ack_command();
        return;
    }
    let mut fetched = run; // sectors read from SD into buf
    for i in (0..512).step_by(2) {
        dev.push_read_fifo((buf[i] as u16) | ((buf[i + 1] as u16) << 8));
    }
    let mut sent = 1u32; // sectors pushed to the FIFO

    dev.set_error(0);
    if dma {
        dev.set_status_intrq(STATUS_DRDY | STATUS_DSC);
        dev.set_dma_mode(DmaMode::ToHost);
    } else {
        dev.set_pio_flow_control(true, false);
        dev.set_status_intrq(STATUS_DRDY | STATUS_DSC | STATUS_DRQ);
    }
    dev.ack_command();

    // Remaining blocks stream behind it, paced by push_buf_to_fifo.
    while sent < count && ok {
        if sent == fetched {
            run = (count - fetched).min(CHUNK);
            if !fetch_run(lba + fetched, run, &mut buf[..run as usize * 512], MULTI_MIN) {
                warn!("{}: SD read failed at block {}", label, lba + fetched);
                ok = false;
                break;
            }
            fetched += run;
        }
        let off = ((sent - (fetched - run)) as usize) * 512;
        if dma {
            // The whole rest of the run in one paced push; DMARQ does the
            // flow control.
            let rest = &buf[off..run as usize * 512];
            if push_buf_to_fifo(dev, rest).await != rest.len() {
                warn!("{}: host stopped draining disk read at sector {}", label, sent);
                ok = false;
                break;
            }
            sent = fetched;
        } else {
            // PIO: wait for the host to take the previous sector, then
            // re-assert DRQ with an interrupt for the next one. The read
            // direction survives without this today because
            // push_buf_to_fifo blocks on room, so the host is paced by
            // accident -- but a host that waits for the per-sector interrupt
            // would hang forever, and ATA says it is entitled to.
            if let Err(why) = wait_host(dev, || dev.read_fifo_level() == 0, 10_000).await {
                if why == Stall::Timeout {
                    warn!("{}: host stopped draining disk read at sector {}", label, sent);
                }
                ok = false;
                break;
            }
            dev.set_status(STATUS_BSY | STATUS_DRDY | STATUS_DSC);
            let sector = &buf[off..off + 512];
            if push_buf_to_fifo(dev, sector).await != sector.len() {
                warn!("{}: host stopped draining disk read at sector {}", label, sent);
                ok = false;
                break;
            }
            dev.set_status_intrq(STATUS_DRDY | STATUS_DSC | STATUS_DRQ);
            sent += 1;
        }
    }

    // Let the host finish taking what is queued before clearing DRQ.
    // Give up the moment the host issues its next command. FatFs reads
    // exactly what it needs and leaves the rest of a sector in the FIFO, so
    // this loop would otherwise spin its full second on a transfer the host
    // has already finished with -- and the *next* command sits unserviced
    // behind it. That is what stalled the first WRITE SECTORS: the host armed
    // it, waited, timed out, and retried, and the retry's BSY was what we
    // then saw as 0xd8. The FPGA clears the FIFO on a command write anyway,
    // so there is nothing to preserve here.
    // A command to either device ends the wait: the data is all delivered,
    // and FatFs in particular reads what it needs and leaves the rest.
    if wait_host(dev, || dev.read_fifo_level() == 0 || dev.command_pending(), 10_000).await
        == Err(Stall::Timeout)
    {
        warn!("{}: host left {} words in the read FIFO (dma={}, bus={:#04x}, strobes={})",
              label, dev.read_fifo_level(), dma, dev.read(17), dev.read(15));
    }
    // Reset, or a new command to this device, at any point above: only let
    // go of DMA mode / flow control (a Command write leaves them armed) and
    // post nothing (R89 A1, A-b).
    if dev.aborted() {
        dev.disarm();
        return;
    }
    // BSY before flow control is released, so the host never sees DRQ with
    // an empty FIFO (see stream_pio_from_sd) -- unless the host has already
    // moved on, in which case Status belongs to its new command.
    if !dev.command_pending() {
        dev.set_status(STATUS_BSY | STATUS_DRDY | STATUS_DSC);
    }
    if dma {
        dev.set_dma_mode(DmaMode::Off);
    } else {
        dev.set_pio_flow_control(false, false);
    }
    dev.complete(if ok { 0 } else { ERR_ABRT }, if ok { STATUS_IDLE } else { STATUS_IDLE_ERR });
}

/// ATA WRITE SECTORS on the emulated disk.
///
/// Only accepted while the GD-ROM is ejected. The disk exposes the same FAT32
/// volume the images live on, so a host writing it while a CD_READ is
/// resolving file offsets against that volume would pull the ground out from
/// under an in-flight transfer. Requiring the eject makes the host state its
/// intent first, and is why START STOP UNIT exists here at all. Refusing with
/// ABRT is a normal, recoverable answer -- the host learns to eject first.
/// Poll `cond` until true, spinning on a yield rather than sleeping.
///
/// The time driver runs off the SAM3U's 32.768 kHz RTT with a /32 prescaler,
/// so ~1 ms is the finest sleep available and `Duration::from_micros(100)` is
/// really a whole millisecond. In the per-sector transfer loops that quantum
/// *is* the transfer rate: a 64-sector write spent ~64 ms asleep and a
/// millisecond moving data, which is most of why WRITE DMA measured no faster
/// than PIO. An earlier version yielded 400 times and then slept a tick to
/// keep count of the timeout; a host that takes longer than those 400 spins
/// to deliver a sector -- a PIO write of 256 words does -- paid the tick on
/// every sector, and 64-sector writes still cost 0.8 ms/sector. The timeout
/// is now measured, not counted, so nothing here ever sleeps. Returns false
/// if it gave up.
async fn poll_until(mut cond: impl FnMut() -> bool, timeout_ms: u32) -> bool {
    let start = Instant::now();
    while !cond() {
        if (Instant::now() - start).as_millis() >= timeout_ms as u64 {
            return false;
        }
        yield_now().await;
    }
    true
}

/// Why a host-paced wait gave up.
#[derive(Clone, Copy, PartialEq, Eq, defmt::Format)]
enum Stall {
    Timeout,
    /// The host wrote a command to the *other* device.
    PeerCommand,
    /// The host reset the bus (hard reset or SRST) mid-transfer.
    Reset,
    /// The host wrote a new command to *this* device (DEVICE RESET, or the
    /// next PACKET over a transfer it gave up on).
    OwnCommand,
}

/// `poll_until` for the waits where the host owes us something -- draining a
/// FIFO, filling one -- with one extra exit: the host issuing a command to
/// the other device. ATA's non-overlapped protocol forbids that until this
/// command has completed, and no Dreamcast host does it (KOS serialises the
/// GD-ROM and ATA drivers on one semaphore), but `ide_task` services one
/// command at a time, so the alternative to bailing here is holding the
/// other device's BSY until this wait times out. Callers treat it like the
/// host abandoning the transfer: give up, report ABRT, let the sweep move
/// on to the command that is actually waiting.
///
/// A new command to *this* device ends the wait as well, and is reported
/// even when `cond` holds: the FPGA empties both FIFOs on a Command write,
/// which by itself satisfies "drained" and "has room", and a multi-chunk
/// transfer used to carry on into the new command -- the next block's BSY,
/// byte count and DRQ written over its CDB phase, stale sectors pushed into
/// its FIFO (R89 A1). So `Ok` means the old command was still the current
/// one. The caller must then post nothing (see `IdeDevice::aborted`); the
/// command stays latched and the sweep serves it next. Every caller runs
/// after its own `ack_command`, or the command being served would count.
async fn wait_host(dev: &IdeDevice, mut cond: impl FnMut() -> bool, timeout_ms: u32)
    -> Result<(), Stall>
{
    let peer = dev.peer();
    let mut peer_cmd = false;
    let mut reset = false;
    let done = poll_until(
        || cond()
            || { reset = dev.reset_seen(); reset }
            || dev.aborted()
            || { peer_cmd = peer.command_pending(); peer_cmd },
        timeout_ms,
    ).await;
    if reset {
        // The FPGA already dropped the transfer (FIFOs, DMA mode, status);
        // the flag stays set for the command loop to log and clear.
        warn!("{}: host reset the bus mid-transfer, abandoning this command",
              if dev.is_primary() { "primary" } else { "secondary" });
        Err(Stall::Reset)
    } else if dev.aborted() {
        info!("{}: host wrote a new command mid-transfer, abandoning this one",
              if dev.is_primary() { "primary" } else { "secondary" });
        Err(Stall::OwnCommand)
    } else if peer_cmd {
        warn!("{}: host issued a command to the other device mid-transfer, abandoning this one",
              if dev.is_primary() { "primary" } else { "secondary" });
        Err(Stall::PeerCommand)
    } else if done {
        Ok(())
    } else {
        Err(Stall::Timeout)
    }
}

/// The two firmware-update steps that normal firmware handles; everything
/// after them runs from RAM, out of the separate `fwupdater` program.
fn handle_fw_command(dev: &IdeDevice, label: &'static str, snap: &CommandSnapshot) {
    let key = snap.lba(true);

    if write_session_active() {
        warn!("{}: firmware update refused during a write session", label);
        dev.set_error(ERR_ABRT);
        dev.set_status_intrq(STATUS_IDLE_ERR);
        dev.ack_command();
        return;
    }

    // The format-2 updater is not in a resident build (contract N5): it would
    // erase the resident. `handle` below refuses every step there.
    #[cfg(not(feature = "resident"))]
    if crate::fwupdate::should_enter(snap.features, key) {
        warn!("{}: entering the firmware updater -- no further logging, no way back", label);
        // Stop the one thing that keeps running off its own interrupt even
        // with the executor parked. The updater masks interrupts itself, but
        // the CDDA feed pokes the FPGA and the updater is about to own that
        // register file.
        crate::cdda::stop();
        // SAFETY: this never returns, so no other task resumes and nothing
        // else touches the MCU again. The IDE task is the only caller, and
        // it is the task being abandoned here.
        unsafe { crate::fwupdate::run(dev.base) }
    }

    if crate::fwupdate::handle(snap.features, key, label) {
        dev.set_error(0);
        dev.set_status_intrq(STATUS_IDLE);
    } else {
        dev.set_error(ERR_ABRT);
        dev.set_status_intrq(STATUS_IDLE_ERR);
    }
    dev.ack_command();
}

async fn handle_disk_write(
    dev: &IdeDevice,
    label: &'static str,
    snap: &CommandSnapshot,
    dma: bool,
    ext: bool,
) {
    // What STATUS reports for a write refused before `write_begin`: the
    // request's own LBA (clamped when it does not fit 32 bits) and count.
    let count = snap.count(ext);
    let req_lba = u32::try_from(snap.lba(ext)).unwrap_or(u32::MAX);
    if media_present() {
        warn!("{}: disk write refused -- eject the GD-ROM first", label);
        wsession().write_refused(req_lba, count);
        dev.set_error(ERR_ABRT);
        dev.set_status_intrq(STATUS_IDLE_ERR);
        dev.ack_command();
        return;
    }
    if snap.device & 0x40 == 0 {
        warn!("{}: disk write in CHS mode, refusing", label);
        wsession().write_refused(req_lba, count);
        dev.set_error(ERR_ABRT);
        dev.set_status_intrq(STATUS_IDLE_ERR);
        dev.ack_command();
        return;
    }
    // The card is addressed in 32-bit blocks, so a 48-bit LBA above that is
    // unreachable however big the card is. Refuse rather than truncate: a
    // silently wrapped address reads the wrong sector and looks like
    // corruption much later.
    let Ok(lba) = u32::try_from(snap.lba(ext)) else {
        warn!("{}: disk write at LBA {} beyond 32-bit block addressing", label, snap.lba(ext));
        wsession().write_refused(req_lba, count);
        dev.set_error(ERR_ABRT);
        dev.set_status_intrq(STATUS_IDLE_ERR);
        dev.ack_command();
        return;
    };

    let total = disk_sectors();
    if total == 0 || lba.saturating_add(count) > total {
        warn!("{}: disk write {} sectors at LBA {} past end ({})", label, count, lba, total);
        wsession().write_refused(lba, count);
        dev.set_error(ERR_ABRT);
        dev.set_status_intrq(STATUS_IDLE_ERR);
        dev.ack_command();
        return;
    }
    // F7 (inside a write session, below the first partition) and F8 (the
    // checksum restarts here).
    if !wsession().write_begin(lba, count) {
        warn!("{}: disk write {} sectors at LBA {} refused -- below the first partition", label, count, lba);
        dev.set_error(ERR_ABRT);
        dev.set_status_intrq(STATUS_IDLE_ERR);
        dev.ack_command();
        return;
    }
    info!("{}: disk write LBA={} count={} dma={}", label, lba, count, dma);

    // The card is about to change underneath the CD read cache, which may hold
    // a range of this very volume. Invalidate now, not only on success: a write
    // that fails half way, or is cut off by a host reset (both return before
    // the end of this function), has already altered blocks.
    invalidate_cd_read_cache();

    // DRQ up means "send me a sector". Unlike the read direction there is
    // nothing to stage first: the host is the producer here.
    dev.set_error(0);
    if dma {
        // No DRQ in DMA mode: the host is driven by DMARQ, which the FPGA
        // raises from write-FIFO room rather than from the status register.
        // BSY stays set for the whole command. The host does not poll status
        // during a DMA burst -- DMARQ paces it, and the FPGA does not gate
        // DMARQ on BSY -- but it *does* poll once its length counter runs
        // out. Leaving BSY clear there let it call the command complete and
        // issue the next one while we were still draining, and that command's
        // fifo_clear discarded the tail of this one.
        dev.set_status_intrq(STATUS_BSY | STATUS_DRDY | STATUS_DSC);
        dev.set_dma_mode(DmaMode::ToDevice);
    } else {
        dev.set_pio_flow_control(false, true);
        dev.capture_arm();
        dev.set_status_intrq(STATUS_DRDY | STATUS_DSC | STATUS_DRQ);
    }
    dev.ack_command();
    if let Some(t) = unsafe { CMD_SEEN_AT } {
        info!("{}: DRQ armed {}us after the command was noticed", label,
              (Instant::now() - t).as_micros());
    }

    // One CMD25 for the whole command, fed a sector at a time as the host
    // delivers them. CMD24 per sector cost a command/response round trip
    // plus a full program-busy wait on every block, which capped writes at
    // ~340 KB/s whatever the IDE side did. Opened after DRQ so the CMD25
    // setup overlaps the host's first sector instead of delaying it.
    //
    // `write_multi_begin` refuses more than 0xFFFF blocks and an LBA48 count
    // of 0 is 65536, so the command is cut into CMD25 runs of WRITE_RUN
    // sectors while the host-side transfer carries on unbroken. A command of
    // at most WRITE_RUN sectors is still exactly one CMD25.
    const WORDS: usize = 256;
    const WRITE_RUN: u32 = 32768;
    let mut block = [0u8; 512];
    let mut ok = true;
    let mut done = 0u32; // sectors completed before the current run
    let mut fed = 0u32; // sectors the card accepted (F8 statistics)
    for (run_lba, run_n) in discimg::ata::runs(lba, count, WRITE_RUN) {
        let mut mw = match unsafe { embassy_atsam3::hsmci::write_multi_begin(run_lba, run_n) } {
            Ok(mw) => mw,
            Err(e) => {
                warn!("{}: CMD25 at block {} failed: {:?}", label, run_lba, e);
                ok = false;
                break;
            }
        };
        for i in 0..run_n {
            let sector = done + i;
            // PIO flow control is DRQ, per sector. Leaving it asserted for the
            // whole command gives the host no reason to pause, and since the FPGA
            // asserts w_en regardless of w_rdy a host that runs ahead of our SD
            // writes silently overruns the 512-word FIFO and loses data. Dropping
            // DRQ while we program the card is what makes it wait.
            //
            // DMA needs none of this: DMARQ is driven from write-FIFO room, so
            // the flow control is in the FPGA and the host is already paced.
            if !dma && sector > 0 {
                // Not over a reset or a new command that arrived while the
                // card was programming the previous sector (R89 A1).
                if dev.aborted() {
                    ok = false;
                    break;
                }
                dev.set_status_intrq(STATUS_DRDY | STATUS_DSC | STATUS_DRQ);
            }

            if let Err(why) = wait_host(dev, || dev.write_fifo_level() >= WORDS as u16, 10_000).await {
                if why == Stall::Timeout {
                    // Sample what the FPGA holds while the host is polling. If
                    // this shows 0x58 while the DC reads 0xd8 off the bus, the
                    // two read paths genuinely disagree. COMMAND_PEND
                    // distinguishes the two things that can set BSY: an IDE
                    // command write sets both, an MCU status write sets only BSY.
                    warn!("{}: host stopped sending at sector {}: wfifo={} lvl_at_read={} reads={} status={:#04x} hw={:#06x}",
                          label, sector, dev.write_fifo_level(), dev.read(22), dev.read(23),
                          dev.read(REG_STATUS) & 0xFF, dev.read(REG_HW_INFO));
                    dev.capture_dump(label);
                }
                ok = false;
                break;
            }

            // DRQ down *and BSY up*: the host must not start the next sector --
            // or the next command -- while we drain the FIFO and program the
            // card. Dropping to a plain idle status here left BSY clear, so a
            // host that had just sent its last word saw the command as complete
            // and issued the next one, whose `fifo_clear` wiped the block out
            // from under this loop; the rest of the drain then returned the
            // FIFO's stale last word and that garbage reached the card.
            dev.set_status(STATUS_BSY | STATUS_DRDY | STATUS_DSC);
            // The tail of a sector came back as one repeated word, which is what
            // popping a drained FIFO yields. Check the level actually covers the
            // block before draining it, and fail the write if it does not: the
            // block would be the FIFO's stale last word, and it used to go to
            // the card and be reported as written (R89 A8). The wait above saw
            // a full sector, so a short level here means the FIFO was emptied
            // under us (a Command write or a reset). The higher of two reads,
            // so that a read torn low cannot fail a good write.
            let lvl_before =
                dev.read(REG_WRITE_FIFO_LEVEL).max(dev.read(REG_WRITE_FIFO_LEVEL));
            if lvl_before < WORDS as u16 {
                warn!("{}: sector {} of {}: level {} before pop (short!), failing the write",
                      label, sector, count, lvl_before);
                ok = false;
                break;
            }
            for i in 0..WORDS {
                let w = dev.pop_write_fifo();
                block[i * 2] = (w & 0xFF) as u8;
                block[i * 2 + 1] = (w >> 8) as u8;
            }
            // F8: the bytes exactly as they left the FIFO.
            wsession().write_data(&block);
            if let Err(e) = unsafe { mw.feed(&block) } {
                warn!("{}: SD write failed at block {}: {:?}", label, lba + sector, e);
                ok = false;
                break;
            }
            fed += 1;
        }
        // CMD12 + the last block's program time. A short transfer (host stopped
        // sending) is closed the same way; the card keeps what it got.
        if let Err(e) = unsafe { mw.end() } {
            warn!("{}: CMD12 after block {} failed: {:?}", label, run_lba + run_n - 1, e);
            ok = false;
        }
        if !ok {
            break;
        }
        done += run_n;
    }

    if dma {
        dev.set_dma_mode(DmaMode::Off);
    } else {
        dev.set_pio_flow_control(false, false);
    }
    // Every interrupted wait above cleared `ok`, a reset included.
    wsession().write_end(ok, fed);
    if dev.aborted() {
        // A completion write now would overwrite status/INTRQ inside the
        // post-reset BSY window, or land on the host's new command. The
        // CMD25 has been closed all the same.
        return;
    }
    dev.complete(if ok { 0 } else { ERR_ABRT }, if ok { STATUS_IDLE } else { STATUS_IDLE_ERR });
    if ok {
        // Anything cached from this volume may now be stale.
        invalidate_cd_read_cache();
    }
}

/// Wait for the host to drain a staged PIO data-in transfer, then drop DRQ
/// and raise the completion interrupt.
///
/// The ATAPI path folds this into `send_data_in`; plain ATA commands like
/// IDENTIFY DEVICE need the same ending without the packet-protocol byte
/// count / interrupt-reason bookkeeping.
async fn finish_pio_in(dev: &IdeDevice) {
    if wait_host(dev, || dev.read_fifo_level() == 0 || dev.command_pending(), 10_000).await
        == Err(Stall::Timeout)
    {
        warn!("finish_pio_in: host never drained FIFO, abandoning");
    }
    // No completion over a reset or the host's next command (R89 A-b).
    if dev.aborted() {
        return;
    }
    dev.complete(0, STATUS_IDLE);
}

// ---------------------------------------------------------------------------
// Canned responses
// ---------------------------------------------------------------------------

/// 6-byte Sega vendor disc-authentication response. The BIOS sends
/// opcode 0x70 / 0x71 during boot/launch and refuses to mount the disc
/// unless the device replies with this exact byte sequence.
static VENDOR_HANDSHAKE_REPLY: [u8; 6] = [0xBA, 0x06, 0x0D, 0xCA, 0x6A, 0x1F];

/// 18-byte ATAPI fixed-format sense data: "no sense, ready, no error".
static REQUEST_SENSE_REPLY: [u8; 18] = [
    0xF0, // valid + response code 0x70 (current error)
    0x00, // segment
    0x00, // sense key = NO SENSE
    0x00, 0x00, 0x00, 0x00, // information
    0x0A, // additional sense length = 10
    0x00, 0x00, 0x00, 0x00, // command-specific
    0x00, // ASC
    0x00, // ASCQ
    0x00, // FRU code
    0x00, 0x00, 0x00, // sense-key specific
];

fn audio_state(s: crate::cdda::Status) -> discimg::atapi::AudioState {
    use discimg::atapi::AudioState;
    match s {
        crate::cdda::Status::Playing => AudioState::Playing,
        crate::cdda::Status::Paused => AudioState::Paused,
        crate::cdda::Status::Finished => AudioState::Finished,
        crate::cdda::Status::Error => AudioState::Error,
        crate::cdda::Status::Standby => AudioState::Standby,
    }
}

/// The mounted image's tracks, or none.
fn mounted_tracks() -> &'static [crate::gdi::Track] {
    match gdi_ref() {
        Some(g) => g.tracks(),
        None => &[],
    }
}

/// 10-byte Sega REQ_STAT reply (Marcus Comstedt's layout): drive status,
/// disc format | repeat count, CTRL/ADR, TNO, index, FAD MSB..LSB, max
/// retry, 0. The disc format is SECNR's ([`disc_format`]; D-3, stock sends
/// 0x2_ for a CDI, R117).
fn build_req_stat_reply() -> [u8; 10] {
    let s = crate::cdda::snapshot();
    let state = audio_state(s.status);
    let tracks = mounted_tracks();
    let (fad, track) = discimg::atapi::head_position(tracks, state, s.fad);
    discimg::atapi::req_stat_reply(state, disc_format(), s.repeat, fad, track)
}

/// Build a 6-byte REQ_SES reply describing one session (0-indexed).
/// REQ_SES reply format (verified byte-for-byte against a stock
/// GDEMU via `tools/g1bios_replay`):
///   byte 0 = drive status, as REQ_STAT byte 0 (stock GDEMU 6.0 sent 0x02
///            in standby, R117; the g1bios_replay capture's 0x01 was
///            presumably taken paused)
///   byte 1 = 0x00
///   byte 2 = track number
///   bytes 3..6 = 24-bit FAD, big-endian
/// Subcommand semantics (packet[2]):
///   0 → "disc summary": session count + lead-out FAD
///   1 → first SDA track + its starting FAD (150 for a normal GDI)
///   2 → on a MIL-CD, the first track of session 2 + its starting FAD
///   past the last session → ABRT
/// The BIOS learns the session count from subcommand 0's track byte, not by
/// probing upwards until something aborts.
/// Stock GDEMU does NOT report an HDA session — even a dual-area
/// GD-ROM image gets reported as a single session here. Earlier
/// 0-indexed-iceGDROM-style mapping was wrong.
fn build_req_ses_reply(_gdi: Option<&'static crate::gdi::Gdi>, session: u8) -> Option<[u8; 6]> {
    // The per-subcommand reasoning (session count vs first track number,
    // MIL-CD's second session) lives with the builder, in discimg::atapi.
    let g = gdi_ref()?;
    let status = discimg::atapi::drive_status(audio_state(crate::cdda::snapshot().status));
    discimg::atapi::req_ses_reply(g.tracks(), g.is_cdi, g.boot_area_lba, g.sessions, session, status)
}

/// Build a 100-byte CD_SCD format-0 reply with a bit-expanded
/// Q-subcode payload. The 4-byte header is `[0, audio_status, 0, 100]`
/// and the remaining 96 bytes are 12 bytes of Q-subchannel expanded
/// MSB-first into one byte per bit (`0x40` for 1, `0x00` for 0: Q is bit 6,
/// P and R..W zero, as stock GDEMU 6.0 sends it, R117), with
/// a CRC-CCITT (poly 0x1021, init 0) computed across the bit stream
/// and substituted into Q[10..12] partway through expansion. Sending
/// all-zero payload bytes makes the BIOS see ADR=0 / CTRL=0 / MSF=0
/// and re-poll forever, never proceeding to CD_READ. Per iceGDROM
/// `firmware-rv32/source/cdda.c`.
///
/// Format 1 (the one games poll while CDDA plays) is the plain 14-byte
/// form: the same header with length 14, then CTRL/ADR, TNO, index and the
/// relative / absolute positions as binary FADs (D-2, R117). Returns the
/// reply length.
fn build_get_scd_reply(format: u8, r: &mut [u8; 100]) -> usize {
    let s = crate::cdda::snapshot();
    let state = audio_state(s.status);
    let (fad, track) = discimg::atapi::head_position(mounted_tracks(), state, s.fad);
    discimg::atapi::get_scd_reply(format, state, fad, track, r)
}

// ---------------------------------------------------------------------------
/// Stage an ATA `IDENTIFY DEVICE` response for the secondary (hard disk).
///
/// 256 words. Only the fields a host actually needs to enumerate and address
/// the disk are filled; everything else stays zero, which is legal.
fn stage_identify_disk(dev: &IdeDevice) {
    let mut buf = [0u16; 256];

    // Word 0: general config. Bit 6 = non-removable, bit 15 clear = ATA.
    buf[0] = 0x0040;
    // Words 1/3/6: obsolete CHS geometry. Hosts that ignore LBA still read
    // these, and zeroes make some of them reject the drive outright.
    buf[1] = 0x3FFF;   // cylinders (clamped legacy value)
    buf[3] = 16;       // heads
    buf[6] = 63;       // sectors per track
    put_ata_string(&mut buf[10..20], b"OPENGDEMU0000000000  ");   // serial
    put_ata_string(&mut buf[23..27], b"1.0     ");                // firmware
    put_ata_string(&mut buf[27..47], b"OpenGDEMU SD Disk                       ");
    buf[47] = 0x8001;  // max sectors per READ/WRITE MULTIPLE
    buf[49] = discimg::ata::WORD49_LBA;  // capabilities: LBA supported
    buf[53] = 0x0007;  // words 54-58, 64-70 and 88 are valid
    let sectors = disk_sectors();
    // Words 60/61 are the LBA28 capacity and the spec caps them at the
    // 28-bit maximum. Reporting the true size of a 500 GB card here is not
    // just cosmetic: a host that believes it will happily issue an LBA28
    // read of a sector no 28-bit address can name, and the address it
    // actually puts on the bus is the low 28 bits -- the wrong sector,
    // returned as if it were the right one.
    buf[60] = (sectors.min(MAX_LBA28_SECTORS) & 0xFFFF) as u16;
    buf[61] = (sectors.min(MAX_LBA28_SECTORS) >> 16) as u16;
    buf[80] = 0x0070;  // ATA-4/5/6
    buf[82] = 0x0000;
    // Word 83 bit 10: 48-bit addressing supported. Bits 14/15 must read
    // 01b or the host treats the whole word as meaningless.
    buf[83] = 0x4400;
    // Word 86 bit 10: and it is enabled. Hosts check this one, not 83.
    buf[86] = 0x0400;
    buf[88] = 0x0000;  // no UDMA advertised
    // Word 49 bit 8 (DMA) and word 63 = 0x0407 (MWDMA 0-2, mode 2 selected).
    // KOS's g1ata only uses DMA when word 63 bit 2 is set. SET FEATURES
    // (0x03, set transfer mode) is accepted as-is by `handle`.
    discimg::ata::advertise_mwdma(&mut buf);
    // Words 100-103: the real capacity, as a 48-bit little-endian count.
    buf[100] = (sectors & 0xFFFF) as u16;
    buf[101] = (sectors >> 16) as u16;
    buf[102] = 0;
    buf[103] = 0;

    for (i, w) in buf.iter().enumerate() {
        let _ = i;
        dev.push_read_fifo(*w);
    }
}

/// ATA identify strings are byte-swapped within each 16-bit word and space
/// padded. `dst` is sized in words; `src` is taken as-is and truncated or
/// space-filled to fit.
fn put_ata_string(dst: &mut [u16], src: &[u8]) {
    for (i, w) in dst.iter_mut().enumerate() {
        let a = *src.get(i * 2).unwrap_or(&b' ');
        let b = *src.get(i * 2 + 1).unwrap_or(&b' ');
        *w = ((a as u16) << 8) | (b as u16);
    }
}

/// Addressable sector count reported by the emulated disk.
///
/// Placeholder until the disk is backed by a real image or partition on the
/// SD card: 512 MB of 512-byte sectors, comfortably inside LBA28.
pub fn disk_sectors() -> u32 {
    DISK_SECTORS.load(core::sync::atomic::Ordering::Relaxed)
}

// IDENTIFY PACKET DEVICE response (40 words = 80 bytes, stock GDEMU 6.0)
// ---------------------------------------------------------------------------

/// Stage `discimg::atapi::IDENTIFY_PACKET_REPLY` (D16). Byte `2i` goes in the
/// low half of word `i`, as `send_data_in` packs every reply: the host reads
/// the data register as 16-bit words and stores each little-endian
/// (gdprobe's `gd_identify`), so it sees the bytes in the order the constant
/// lists them -- "SE", "CD-ROM DRIVE", "6.42" not byte-swapped, as stock
/// sends them.
fn stage_identify_packet(dev: &IdeDevice) {
    for w in discimg::atapi::IDENTIFY_PACKET_REPLY.chunks_exact(2) {
        dev.push_read_fifo(u16::from_le_bytes([w[0], w[1]]));
    }
}

// `STATUS_BSY` etc. are documented constants used only by the FPGA-side
// register file — keep around for reference / future use.
#[allow(dead_code)]
const _STATUS_BSY_DOC: u8 = STATUS_BSY;
