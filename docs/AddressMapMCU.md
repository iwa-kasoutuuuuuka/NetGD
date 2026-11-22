# Address Map for MCU

This is the SAM3U side of the FPGA register window: what the firmware sees when it reads and writes the
FPGA over the Static Memory Controller. The Dreamcast-side (G1) register decode is not covered here.

Sources: `mcu/netgd/src/ide.rs` (the `REG_*`, `HW_*`, `STATUS_*` constants and their use),
`mcu/netgd/src/cdda.rs` and `main.rs` (slots 12, 14-18, 29-31), `fpga/amaranth/hw/ide_device.py` (the
MCU-side `Switch` on `mcu_addr`), `fpga/amaranth/hw/cdda.py` (slots 29-31 bit layout), `fpga/amaranth/hw/OpenGDEMU.py`
and `mcu/embassy-atsam3/src/smc.rs` (window placement). Where the sources disagree, the section
"Discrepancies" at the end says so.

## Addressing

- The window is chip select 3: base `0x6300_0000` (`FPGA_BASE` in `smc.rs`).
- `MCU_ADDR[8:1]` is wired to SAM3U `A[8:1]` (`OpenGDEMU.v`, `OpenGDEMU.qsf`). The SMC address is therefore a
  byte address, and every slot is one 16-bit word: slot `n` is at byte `0x6300_0000 + 2n`.
- `io_MCU_ADDR[7]` selects the device:
  - Primary (GD-ROM): slots `0x00`-`0x7F`, bytes `0x6300_0000`-`0x6300_00FE`. Only slots `0x00`-`0x1F` and `0x20` (LBA_LOW alone) are decoded.
  - Secondary (ATA disk): slots `0x80`-`0xFF`, bytes `0x6300_0100`-`0x6300_01FE`. In Rust this is
    `fpga_window().add(0x80)` (a `*mut u16`).
- Within a device, the 7-bit slot index is `io_MCU_ADDR[6:0]`.
- Data is 16 bits wide. `MCU_NBS` (byte enables) is not used by the decoder, so every access is a full word.

Write behaviour, which matters when a bus access is held for several FPGA cycles:

- Register writes are level-sampled (`cs & wr`). Writing the same value twice is harmless.
- Slot 9 (data FIFO) and slot 31 (CDDA push) are edge-sampled: one push or pop per access, however long the
  strobe is held.
- A write to slot 0 with bit 15 clear loads COMMAND_PEND from bit 1 (`ide_device.py`, case 0), so anything that
  wants to keep COMMAND_PEND while touching the reset bit has to write it back. With bit 15 set the write is an
  acknowledge by number and can only clear COMMAND_PEND; see slot 0 below.
- Because a write is a level several FPGA cycles wide and the MCU-side assignments come last in `ide_device.py`,
  an MCU write beats a host-side change to the same flop made during the strobe. Slots 0 and 19 are written
  so that this cannot lose a host event (the acknowledge yields to a Command write; slot 19 has a write mask).

Bit names follow ATA. "RO" means reads only, "W" means writes only (reads return 0), "R/W" means both. Byte
offsets are the slot number times two, for comparison with the old version of this file.

## Summary

| Slot | Byte | Name | Access | Purpose |
|------|------|------|--------|---------|
| 0 | 0x00 | HW_INFO | R/W | Device id, COMMAND_PEND, RESET_SEEN, nIEN, IN_RESET, command number |
| 1 | 0x02 | STATUS | R/W (bits 8-15 W) | ATA status; write bit 8 to raise INTRQ; bits 9-15 tie the write to a command number |
| 2 | 0x04 | DEVICE | R/W | Device register (bit 4 = DEV) |
| 3 | 0x06 | ERROR | R/W | Bits 0-1, ABRT and sense key |
| 4 | 0x08 | COMMAND / FEATURES | R/W | Latched command (low), features (high) |
| 5 | 0x0A | SECTOR_COUNT | R/W (low byte W) | Sector count; ATAPI interrupt reason in the low byte |
| 6 | 0x0C | LBA_LOW / LBA_MID | R/W | LBA bits 7:0 and 15:8 |
| 7 | 0x0E | LBA_HIGH / LBA_LOW_PREV | R/W | LBA bits 23:16; previous LBA_LOW |
| 8 | 0x10 | LBA_MID_PREV / LBA_HIGH_PREV | R/W | Previous LBA_MID and LBA_HIGH |
| 9 | 0x12 | DATA_FIFO | R/W (edge) | Data register FIFOs: read pops host-to-MCU, write pushes MCU-to-host |
| 10 | 0x14 | READ_FIFO_LEVEL | RO | Words waiting for the host (MCU to host) |
| 11 | 0x16 | WRITE_FIFO_LEVEL | RO | Words waiting for the MCU (host to MCU) |
| 12 | 0x18 | ID | RO | Always `0xC0DE` |
| 13 | 0x1A | INTRQ_PENDING | R/W | IDE INTRQ flop, bit 0 |
| 14 | 0x1C | FREE_RUN | RO | Core clock counter |
| 15 | 0x1E | IDE_STROBES | RO | Host strobe counter |
| 16 | 0x20 | RSTN_EDGES | RO | Reset edge counters (low: rises, high: falls) |
| 17 | 0x22 | BUS_STATE | RO | Live IDE control lines |
| 18 | 0x24 | AUDIO_ALIVE | RO | Audio-clock heartbeat counter |
| 19 | 0x26 | DMA_MODE / flow control | R/W (masked) | DMA direction, PIO flow control, packet phase |
| 20 | 0x28 | CAP_CTRL | R/W | In-fabric capture control (not built in shipped image) |
| 21 | 0x2A | CAP_DATA | R/W | In-fabric capture read index and value |
| 22 | 0x2C | LVL_AT_READ | RO | Debug: write FIFO level at the last slot 11 read |
| 23 | 0x2E | LVL_READ_COUNT | RO | Debug: number of slot 11 reads |
| 24 | 0x30 | HOST_STATUS_RDS | RO | Status reads (low), Alt Status reads (high), since last Command write |
| 25 | 0x32 | HOST_DATA_RDS | RO | Data register reads since last Command write |
| 26 | 0x34 | HOST_DEVCTL | RO | Last Device Control byte (low), write count (high) |
| 27 | 0x36 | PREV_STATUS_RDS | RO | Slot 24 as of the most recent Command write |
| 28 | 0x38 | PREV_DATA_RDS | RO | Slot 25 as of the most recent Command write |
| 29 | 0x3A | CDDA_CTRL | R/W | CDDA enable, clear underflow, flush (primary device only) |
| 30 | 0x3C | CDDA_LEVEL | RO | CDDA sample FIFO level, in words (primary device only) |
| 31 | 0x3E | CDDA_PUSH | W (edge) | One PCM word per write, left then right (primary device only) |

## Slot details

### Slot 0 (0x00): HW_INFO

| Bit | Name | Access | Meaning |
|-----|------|--------|---------|
| 0 | DEV_ID | RO | 0 = primary (GD-ROM), 1 = secondary (ATA disk) |
| 1 | COMMAND_PEND | R/W | Set by a host Command write. The MCU clears it by acknowledging (`ack_command`, below). The FPGA raises MCU_IRQn while it is set. |
| 2 | RESET_SEEN | R/W | Sticky. Set by a hard reset (RSTn low) or SRST. Write 1 to clear. |
| 3 | nIEN | RO | Live copy of Device Control bit 1. |
| 4 | IN_RESET | RO | Live: RSTn is low or SRST is set. |
| 14:8 | CMD_SEQ | R, W = number to acknowledge | Counts host Command writes to this device, modulo 128. Not cleared by a reset. |
| 15 | ACK_BY_SEQ | R = 1, W = form of the write | Reads 1 on a bitstream that has CMD_SEQ. Written as 1, it makes the write an acknowledge by number. |

Bits 5-7 read 0.

Writes come in two forms, chosen by bit 15. Bit 2 (clear RESET_SEEN) works the same in both.

- **Plain, bit 15 = 0.** COMMAND_PEND is loaded from bit 1. Writing 0 clears it whatever command it stands
  for. This is the only form a bitstream from before CMD_SEQ has.
- **By number, bit 15 = 1.** With bit 1 = 0, COMMAND_PEND is cleared only if bits 14:8 equal CMD_SEQ, and not
  in the FPGA cycle of a host Command write. With bit 1 = 1 it is left alone. This form never sets it.

The firmware reads slot 0 before and after it snapshots a command (`take_command`), keeps the upper byte,
and writes that byte back to acknowledge (`ack_command`). A command the host writes after the snapshot has
the next number, so the acknowledge leaves its COMMAND_PEND set and the command is served next. With the
plain form that command was lost: COMMAND_PEND cleared, BSY still set, and no MCU_IRQn edge.

The two ages of firmware and bitstream mix freely. An old bitstream reads bits 15:8 as 0, so the firmware
writes back 0, which is the plain acknowledge. Old firmware writes 0 to a new bitstream, which is also the
plain form.

CMD_SEQ changes on the FPGA clock, so one MCU read that straddles a Command write can return a mix of the
two values. `take_command` accepts a number only when two reads either side of the snapshot agree.

### Slot 1 (0x02): STATUS

Read: bit 0 ERR, bit 3 DRQ, bit 4 DSC, bit 5 DF, bit 6 DRDY, bit 7 BSY. Bits 1-2 read 0.

Write: the same bits set the status the host sees. Bit 8 (write only, not stored) raises INTRQ in the same
FPGA cycle as the new status. Writing bit 8 as 0 does nothing.

Firmware uses bit 8 for every completion and every DRQ transition (`set_status_intrq`). Posting status and
INTRQ as two writes left a window where the host could read Status and issue its next command before the
interrupt, so the interrupt landed on that command.

Bits 9-15 (write only) tie the write to a command:

| Bit | Meaning |
|-----|---------|
| 14:9 | The low six bits of CMD_SEQ (slot 0 bits 13:8) of the command this status belongs to. |
| 15 | 0 = plain write, always applied. 1 = apply the write only if bits 14:9 equal the low six bits of the current CMD_SEQ, and not in the FPGA cycle of a host Command write. |

A write that fails the comparison is dropped whole: no status bit changes and bit 8 raises nothing.

Why: the host can write its next command between the firmware deciding a status and the write reaching the
FPGA, or inside the write strobe (several FPGA cycles wide, and applied after the IDE-side logic). The plain
write then cleared the BSY the Command write had just raised, with that command not yet looked at; on a
PACKET it cleared the DRQ of the CDB phase. The firmware's `post_status` builds bits 9-15 from the upper byte
`take_command` kept (bit 15 as read, bits 14:8 shifted up one), so every `set_status` / `set_status_intrq`
of a command carries its number. Nothing needs to follow a dropped write: the new command stays latched and
is served next.

Six bits, because bit 8 is taken: a stale write is applied only if exactly 64 (or 128) Command writes came
in between. Compatibility is the same as slot 0. Old firmware writes bit 15 = 0. A bitstream without the
check ignores bits 9-15. A bitstream without CMD_SEQ reads as 0 in slot 0, so the firmware writes 0 here.

`complete` writes the plain form in one case: COMMAND_PEND set with the Command register reading 0x00, which
it treats as a latch without a command and completes over.

### Slot 2 (0x04): DEVICE

R/W, full byte. Bit 4 (DEV) is the device-select bit. Bit 6 is the LBA-mode flag (the firmware refuses CHS
on the disk path; the FPGA only stores it). An MCU write changes only this device's copy.

### Slot 3 (0x06): ERROR

R/W. Three parts of the byte are stored:

- Bits 1-0: MCU-owned, shown to the host as written (stock reads ERROR `0x01` after DEVICE RESET). Cleared by a
  reset and, with the sense key, by the host's PACKET Command write. Old firmware writes them as 0, so nothing
  changes for it; on a bitstream from before this they are dropped and read back 0.
- Bit 2: ABRT.
- Bits 7-4: sense key (ATAPI).

Bit 3 reads 0. The ERR status bit lives in slot 1, not here. The firmware writes whole bytes such as `0x20`
(NOT READY, sense 2) and `0x60` (UNIT ATTENTION, sense 6) on the packet path.

### Slot 4 (0x08): COMMAND (low) / FEATURES (high)

R/W. Holds the opcode the host last wrote to the Command register, and the Features byte. An MCU write
updates only the readback; it does not set COMMAND_PEND.

### Slot 5 (0x0A): SECTOR_COUNT

R/W, 16 bits. Reset value `0x0001`.

- Low byte: the sector count, or on the packet device the ATAPI interrupt reason (C/D bit 0, I/O bit 1).
- High byte: the previous sector count, used by LBA48.

An MCU write changes the low byte only. The high byte belongs to the host (its previous Sector Count write)
and is read back by the host with HOB set. `set_irr` in the firmware therefore no longer zeroes it. On a
bitstream from before this change an MCU write is a whole-word write and does zero it.

### Slot 6 (0x0C): LBA_LOW (low) / LBA_MID (high)

R/W. On the GD-ROM, LBA_LOW is Sega's SECNR: disc format in the high nibble, drive status in the low nibble.
Reset values:

- Primary: LBA_LOW `0x81` (GD-ROM, ready), LBA_MID `0x14`.
- Secondary: LBA_LOW `0x01`, LBA_MID `0x00`, the non-packet ATA signature.

On the packet path, LBA_MID and LBA_HIGH together carry the host's ATAPI byte count.

### Slot 7 (0x0E): LBA_HIGH (low) / LBA_LOW_PREV (high)

R/W. Reset LBA_HIGH is `0xEB` on the primary and `0x00` on the secondary.

### Slot 8 (0x10): LBA_MID_PREV (low) / LBA_HIGH_PREV (high)

R/W.

Each host write to an LBA register pushes the old value into the matching `_PREV` field. The LBA48 address is
then the current byte plus the previous one, the way the host's two writes arrive. The firmware reads the
`_PREV` values for LBA48 commands.

### Slot 9 (0x12): DATA_FIFO

R/W, edge-sampled.

- Read: pops one word from the write FIFO (host to MCU). Used for ATAPI command packets and disk write data.
- Write: pushes one word into the read FIFO (MCU to host). The host reads it through its Data register.

FIFO depths are set in the FPGA:

| | Read FIFO (MCU to host) | Write FIFO (host to MCU) |
|-|-------------------------|--------------------------|
| Primary | 4096 words | 32 words |
| Secondary | 1024 words | 512 words |

Words pushed past the FIFO's capacity are dropped without any error. `push_buf_to_fifo` checks slot 10 before
each push and keeps four words of headroom.

### Slot 10 (0x14): READ_FIFO_LEVEL

RO. Words currently in the read FIFO (MCU to host).

### Slot 11 (0x16): WRITE_FIFO_LEVEL

RO. Words currently in the write FIFO (host to MCU). Reading this slot also updates slots 22 and 23.

### Slot 12 (0x18): ID

RO, always `0xC0DE`. `main.rs` reads it on both devices after the FPGA loads, as a bus sanity check.

### Slot 13 (0x1A): INTRQ_PENDING

R/W, bit 0.

- Write 1: asserts the IDE INTRQ line. It is gated by device select and nIEN on the IDE side.
- Write 0: clears it.
- Read: returns the flop.

The FPGA clears the flop itself when the host reads Status, and when the host writes Command.

### Slots 14-18: diagnostic counters

These are all RO.

| Slot | Name | Meaning |
|------|------|---------|
| 14 | FREE_RUN | Free-running counter on the core clock. A change between reads means the FPGA is clocked. |
| 15 | IDE_STROBES | Host strobes counted (writes, and falling edges of RD). Stays at 0 if the host never touches the bus. |
| 16 | RSTN_EDGES | Low byte: rising edges of RSTn. High byte: falling edges. |
| 17 | BUS_STATE | Bit 0 RSTn (1 = out of reset), bit 1 CSn[0] deasserted, bit 2 CSn[1] deasserted, bit 3 RD, bit 4 WR. |
| 18 | AUDIO_ALIVE | Counter driven by a divided audio-clock toggle. A change means the 11.2896 MHz crystal reaches the fabric. |

`main.rs` reads slots 14-18 once a second whether or not the `fpga-diag` feature is on. Only the logging is
gated.

### Slot 19 (0x26): DMA_MODE and flow control

R/W.

| Bits | Name | Meaning |
|------|------|---------|
| 1:0 | DMA_MODE | 0 = off, 1 = device to host (DMARQ means "data ready"), 2 = host to device (DMARQ means "room"). 3 is decoded as off. |
| 2 | PIO_WR_FC | During a PIO data-out phase, drop host-visible DRQ while the write FIFO is full. |
| 3 | PIO_RD_FC | During a PIO data-in phase, show DRQ only while the read FIFO has data. |
| 4 | PKT_PHASE | Packet phase. Set by the FPGA when a PACKET command opens the CDB phase; cleared by the FPGA on the last CDB word, on any Command write and on reset. |
| 12:8 | WRITE_MASK | Write only. Bit 8+n enables the write of bit n. All zero means "write bits 4:0", the whole-word write of older bitstreams. |

Reads return bits 4:0; bits 12:8 read 0. The FPGA clears all five bits on reset.

The firmware writes with mask `0x0F` (`XFER_MASK` in `ide.rs`): `set_dma_mode`, `set_pio_flow_control` and
`disarm` each set DMA_MODE and both flow-control bits together and never touch PKT_PHASE.

- DMA_MODE and the flow-control bits are written together on purpose. A command moves its data by DMA or by
  PIO, never both, so the three fields are one transfer mode, and setting the whole mode also clears whatever
  a transfer that ended without cleanup left behind. A caller that does need to change one field can use a
  narrower mask.
- PKT_PHASE is left out because the FPGA owns it. Before the mask, `disarm` after a transfer that the host had
  abandoned with a new PACKET command cleared PKT_PHASE under that command's CDB, and the sixth CDB word then
  did not raise BSY.

On an older bitstream the mask is ignored and every write is a whole-word write, as before.

### Slot 20 (0x28): CAP_CTRL and slot 21 (0x2A): CAP_DATA

Debug capture. Slot 20 arms and reports; slot 21 writes the read index, then reads the value.

- Slot 20 write bit 0 = 1: arm, and reset the pointers. Write 0: disarm.
- Slot 20 read: bit 0 armed, bit 1 running, bit 2 done, bits 15-8 the event count divided by 2.
- Slot 21 write: sets the index (bits 8:0). Read: returns the sample at that index.

This only records data when the device is built with `capture=True`. `OpenGDEMU.py` does not do that, so the
shipped image accepts the arm, and every sample reads 0. The firmware arms it on each WRITE DMA and dumps it on a
host stall (`capture_arm`, `capture_dump`).

### Slots 22 (0x2C) and 23 (0x2E): debug counters

RO. Slot 22 is the write FIFO level at the last slot 11 read. Slot 23 counts slot 11 reads. Both are always
present. `ide.rs` reads them directly as raw numbers, with no named constants.

### Slots 24 (0x30) to 28 (0x38): host access counters

RO. These count what the host did on the Dreamcast bus. The FPGA clears 24 and 25 on each Command write.

- 24: low byte = Status register reads, high byte = Alt Status reads (8 bits each).
- 25: Data register reads (16 bits).
- 26: low byte = last Device Control byte written, high byte = number of Device Control writes.
- 27: slot 24 as it stood at the most recent Command write.
- 28: slot 25 as it stood at the most recent Command write.

Slots 27 and 28 let the firmware read what the host did during the previous command, after the next one has
started.

### Slot 29 (0x3A): CDDA_CTRL

Primary device only. Layout from `hw/cdda.py`:

- Write: bit 0 enable output, bit 1 clear the underflow flag and counter, bit 2 flush the FIFO.
- Read: bit 0 enable, bit 1 underflow (sticky), bits 15-8 silent frames inserted since the last clear
  (saturating).

### Slot 30 (0x3C): CDDA_LEVEL

Primary device only. RO. Words in the CDDA sample FIFO (depth 768). The firmware keeps it topped up to 760.

### Slot 31 (0x3E): CDDA_PUSH

Primary device only. Write only. Each write pushes one 16-bit PCM word. Samples go in left then right. The
push is edge-sampled, so one write is one word.

## Discrepancies and open points

These were found while writing this page. Nothing was changed in the code to resolve them.

1. **Byte addresses in `OpenGDEMU.py`.** The class docstring gives the secondary window as `0x6300_0080`-`0x6300_00FF`,
   a 128-byte window. The wiring and firmware both use word slots: `MCU_ADDR[8:1]` and `fpga_window().add(0x80)`
   put the secondary at byte `0x6300_0100`, and each window is 128 words = 256 bytes. The table above follows the
   wiring.
2. **`ide_device.py` class docstring is out of date.** It says FIFO slots 9-11 are stubbed, that Data reads return
   `0xFFFF`, and that the layout "matches the prior design's `docs/AddressMapMCU.md`". The FIFOs are implemented
   now, and the old address map is replaced by this page.
3. **Secondary read FIFO depth comment in `ide.rs`.** The comment on `IdeDevice::read_fifo_depth` says the
   secondary FIFO holds 256 words. The constant `SECONDARY_FIFO_DEPTH_WORDS` is 1024, which matches
   `SECONDARY_READ_FIFO_DEPTH` in the Python.
4. **Capture (slots 20/21) is not in the shipped image.** The firmware depends on it for diagnostics only, and it
   reads zeros in the shipped build. If it is needed, build with `capture=True` and rebuild the bitstream. The
   Python notes say enabling it has changed fitter results before.
5. **Whole-word writes to shared slots.** Resolved in the FPGA: slot 5 takes only the low byte from the MCU,
   and slot 19 has a write mask. Both fall back to whole-word writes on a bitstream built before that; the
   shipped `OpenGDEMU.rbf` has to be rebuilt from `out/OpenGDEMUCore.v` to get the new behaviour.
6. **Slot 1 bit 8 is a pulse.** Reads never show it, so the MCU cannot check whether INTRQ was raised.

### Slot 32 (0x40): LBA_LOW alone

R/W. A write changes `lba_low` from bits 7:0 and nothing else; a read gives `lba_low` in the low byte and `0xA5`
in the high byte. Slot 6 stores `lba_low` and `lba_mid` together from one word, and the host writes BYTE COUNT
LOW into `lba_mid`, so the firmware's read-modify-write of slot 6 to set SECNR could lose a host write that fell
between its read and its write. `set_secnr` uses slot 32 when the high byte reads `0xA5`. A bitstream without the
slot reads 0 there and the firmware keeps the slot 6 path; old firmware never touches slot 32. Decoded on both
devices.
