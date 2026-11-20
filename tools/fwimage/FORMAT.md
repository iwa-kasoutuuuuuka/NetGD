# OGFW image format, versions 2 and 3

What `mcu/fwupdater` (vendor opcode `0xF9`, after the arm and enter keys)
accepts, and what `fwimage.py` writes. Format 2 is for boards without a
resident boot region (every board today); format 3 is for boards with one
(`docs/design/fw-bootloader.md` sections 2-5 and 2-6). Everything below is
format 2 unless it says otherwise; "Format 3" has the differences. The authoritative implementation of the
checks is `mcu/fwupdater/src/logic.rs`; `fwimage.py` repeats them, and both
are tested against `vectors/*.txt`.

## File

A `.ogfw` file is one or more **images**, back to back. Each image is a
512-byte header block followed by `len` payload bytes:

```
[header 512][payload len][header 512][payload len]...
```

A 4E update has two images (code, bitstream); a 2E update has one (code and
bitstream share the single flash plane, so the code image already contains
the bitstream).

## Header block (little-endian 32-bit words; bytes 36..511 are zero)

| word | offset | field          | value |
|------|--------|----------------|-------|
| 0    | 0      | `magic`        | `0x5746474F` ("OGFW") |
| 1    | 4      | `format`       | `2` (format 1, which had no CRC or target, is refused; `3`: see "Format 3") |
| 2    | 8      | `kind`         | `0` code, `1` bitstream |
| 3    | 12     | `target_cidr`  | CHIPID_CIDR of the part: 2E `0x281A0761`, 4E `0x28100961`. Bits [4:0] (silicon revision) are ignored on compare |
| 4    | 16     | `dest`         | flash address of the first byte (see below) |
| 5    | 20     | `len`          | payload bytes: non-zero, multiple of 512, at most `0x20000` |
| 6    | 24     | `payload_crc`  | CRC-32 of the `len` payload bytes (below) |
| 7    | 28     | `image_version`| free-form build number, informational |
| 8    | 32     | `header_crc`   | CRC-32 of bytes 0..31 |

### Destination rules

* `kind = 0` (code): `dest` must be `0x00080000` (start of plane 0). The
  payload starts with the Cortex-M vector table.
* `kind = 1` (bitstream): `dest` must be `0x00100000` (start of plane 1), and
  the board must be a SAM3U4E.
* There is no way to write anywhere else, and no partial update of the code
  plane.

### CRC

CRC-32 as in zlib/PNG: reflected polynomial `0xEDB88320`, initial value and
final xor `0xFFFFFFFF`; Python `zlib.crc32(data) & 0xFFFFFFFF`. Check value:
`crc32(b"123456789") == 0xCBF43926`. The payload is the bytes as they will sit
in flash, padded to a multiple of 512 with `0xFF`.

### Vector-table check (code images)

On the first 256 bytes, before anything is erased. With words `v0..v3` =
initial SP, Reset, NMI, HardFault:

* `v0` is a multiple of 4 and `0x20000000 < v0 <= sram0_end` of the running
  part (2E `0x20004000`, 4E `0x20008000`);
* `v1..v3` are odd (Thumb) and `v & ~1` lies in `[dest, dest + len)`.

## Format 3 (boards with a resident boot region)

Lock region 0 (`0x00080000`, 8 KB, the *layout*) holds the resident code and
is never written by an update. The app starts right behind it at `APP_BASE =
0x00082000`.

### Differences from format 2

| item | format 2 | format 3 |
|------|----------|----------|
| `format` (w1) | 2 | 3. Who accepts which: a board without a resident region (updater built without feature `resident`) takes **2 only** (3 is `BadHeader`); a board with one (`resident`) takes **3 only** (2 is `BadHeader`); `netgd-fwupd` (sender) and `fwimage.py inspect` read both and leave the decision to the board |
| `kind` 0 | code, `dest` = `0x00080000` | **app**, `dest` = `APP_BASE` exactly. Anything else, in particular anything inside `0x80000..0x82000`, is `BadDest` whatever the length |
| `kind` 1 | bitstream, `dest` `0x00100000`, 4E only | the same. If the app carries a non-zero `bs_stamp` (NGAX, below) **the last 4 bytes of the payload are the CRC-32 of the bytes before them** (same rule as the app trailer) and `bs_stamp` equals it; otherwise the payload is opaque |
| w8 `header_crc` | CRC-32 of bytes 0..31 | CRC-32 of bytes 0..31 **followed by** w9 (bytes 36..39), i.e. of every field except the CRC word, so a flipped layout bit is `BadHeader` |
| w9 (offset 36) | 0 (unused) | `layout`: size of the resident region the image was built for, `0x2000`. Any other value is `BadDest` (an image for a 16 KB resident region must never be written to an 8 KB one) |
| `len` limit | `0x20000` | code: `0x20000 - layout` = `0x1E000`; bitstream: `0x20000` |
| payload | the bytes as they sit in flash | the same; for `kind` 0 the app descriptor is at +0x100 and **the last 4 bytes are the image's own CRC-32** (the resident code checks it in flash) |
| `payload_crc` | CRC-32 of the payload | the same (it covers the trailer too; it protects the transfer) |
| vector check | handlers inside `[dest, dest+len)` | the same formula; `dest` is `APP_BASE`, so an ELF still linked at `0x80000` is refused by `fwimage` before a file exists |

Order of the checks, which is also the order the vectors rely on: header
(magic, format, CRC, kind) - target - layout - dest - length - payload CRC -
vector table - descriptor - trailer. Layout, descriptor and trailer faults
reuse the updater's existing status names where there is one (`BadDest` for
the layout); the descriptor and trailer faults (`BadDescriptor`, `Unsealed`,
`BadTrailer` in the tools) have no wire status yet (stage S5).

### App descriptor "NGAP" (32 bytes at app offset 0x100, little endian)

| offset | field |
|-------:|-------|
| 0  | magic `"NGAP"` (bytes `4E 47 41 50`) |
| 4  | descriptor version, `1` |
| 8  | length of the whole image **including the trailer**, a multiple of 512; **0 = not sealed** |
| 12 | bitstream address (absolute: inside the image on a 2E, `0x00100000` on a 4E, 0 for SD) |
| 16 | bitstream length, compressed |
| 20 | bitstream length, unpacked |
| 24 | top 8 bits: bitstream kind (0 none/SD, 1 LZMA); the rest: app version |
| 28 | CRC-32 of bytes 0..27 |

The linker leaves length and CRC 0; `fwimage` fills them (`seal` into the ELF,
`build` into the image). The checks, in order: magic and version
(`BadDescriptor`), length 0 (`Unsealed`), descriptor CRC (`BadDescriptor`),
length equals the image length and is a multiple of 512 (`BadDescriptor`).
Words 12..27 are the app's own and not checked here. `build` never produces an
`Unsealed` image; only a development resident build accepts one.

**The descriptor version stays 1 for ever.** The resident refuses any other
version (`bootcore::app::DESC_VERSION`), and a locked resident cannot be
changed on a deployed board, so a version 2 could never be installed. A new
field the app or the tools need goes into the extension block "NGAX" (below),
which the resident does not read.

### Trailer

The last four bytes of a code image are the CRC-32 (same algorithm) of all
bytes before them, little endian. An image shorter than four bytes is
`BadTrailer`.

### Extension block "NGAX" (app offset 0x120, 32 bytes, the resident does not read it)

Right behind the descriptor; covered by the image trailer CRC. Constants in
`bootcore::app` (`AX_OFF`, `AX_LEN`, `AX_MAGIC`, `Ax`, `parse_ax`, `check_ax`,
`pack_ax`) and `fwimage.py` (`AX_OFFSET`, `parse_ax`, `check_ax`, `pack_ax`).
ELF section `.app_ax` (32 bytes at `APP_BASE + 0x120`, PROGBITS) is what
`fwimage seal` patches.

| offset | field |
|-------:|-------|
| 0  | magic `"NGAX"` (bytes `4E 47 41 58`) |
| 4  | block length: 32 now; a multiple of 4, at least 32; may grow (nobody but the app and the tools care) |
| 8  | `min_gen`: the least resident interface generation (`IFACE_GEN`) the app needs; 1 |
| 12 | `bs_stamp`: CRC-32 of the plane-1 bitstream without its last four bytes, which hold the same value (4E, "V3"); 0 = not checked |
| 16 | `bs_iface`: the FPGA's interface/version register value the app expects; 0 = not checked |
| 20..27 | reserved, 0 |
| 28 | reserved, 0 (no CRC of its own: the trailer covers it) |

Check order (`check_ax`, vectors `ax.txt`): magic (`NoAx`), length (`BadAx`:
below 32 or not a multiple of 4), then `bs_stamp` against the last four bytes
of the plane-1 image as a little-endian word (`StampMismatch`; a stamp of 0 is
never checked). `min_gen` is not part of that check: the app compares it with
the resident's generation (`Ax::gen_ok`) and, when the resident is older,
still starts and leaves out what needs the newer one (design 3-4).

**V3 (bitstream stamp, 4E only).** The plane-1 image ends in its own CRC-32;
the app compares one word (`bs_stamp` against the image's last four bytes)
before it loads the FPGA, and the PC checks the combination before anything is
sent. Linker: `.flash1` is followed by padding and the 4-byte section
`.bs_crc`, ending on a multiple of 512, like `.image_pad`/`.image_crc`.
`fwimage build --format 3` and `fwimage seal`: bitstream CRC into `.bs_crc`,
the same value into `bs_stamp` (a different non-zero value already there is
`StampMismatch`; `.bs_crc` without an NGAX block is `BadAx`), then descriptor
length and CRC, then the trailer, in that order. `validate_file` and `inspect`
check a file's code image against its bitstream image (`BadTrailer` for a
bitstream that is not self-consistent, `StampMismatch` for the wrong pair); a
code-only file passes with a note, because the board keeps its stored
bitstream. A `bs_stamp` on a part without a plane 1 is `BadAx`. No new header
field is needed.

## Resident and application contract (interface generation 1)

The spec is `docs/design/resident-app-contract.md`; this section is the part
that is code: names and layouts shared by `mcu/bootcore` (Rust, `no_std`, no
dependencies), the application and `fwimage.py`. **Frozen with the resident:**
once a resident is locked on a board only SWD changes it, so everything here
changes only by raising the generation (design 3-4).

### Resident information block (`0x00081FF0`, 16 bytes, end of lock region 0)

`bootcore::info` (`INFO_ADDR`, `INFO_MAGIC`, `IFACE_GEN`, `Info`,
`resident_info`, `pack_info`, `version`, `check_region`) and `fwimage.py`
(`INFO_OFFSET`, `parse_info`, `pack_info`, `check_region`). Read with plain
loads (no EEFC).

| offset | field |
|-------:|-------|
| 0  | magic `"NGRS"` (bytes `4E 47 52 53`) |
| 4  | byte 0: interface generation (`IFACE_GEN`, 1); byte 1: flags; bytes 2..3: `layout >> 8` (`0x0020`) |
| 8  | resident version `major << 16 \| minor << 8 \| patch` |
| 12 | CRC-32 of `0x80000..0x81FFB` (the last four bytes of the region), the app trailer rule: `check_trailer` accepts the whole 8192 bytes |

Flags: bit 0 `INFO_F_DEV` (development build: no SLB, accepts unsealed apps),
bit 1 `INFO_F_SELF_LOCK`, bit 2 `INFO_F_BUTTON`, bit 3 `INFO_F_LED`, bit 4
`INFO_F_WDT_MR` (writes `WDT_MR`, development only). Unused bytes of the region
are `0xFF`. No magic, or generation 0: the version is unknown. The app does not
check the CRC at boot (about 1 ms); only when asked (`netgd-fwupd`). Vectors
`info.txt`.

### GPBR0 (app -> resident) and GPBR1 (resident -> app)

`SYS_GPBR0` = `0x400E1290`, `SYS_GPBR1` = `0x400E1294`. `bootcore::boot`
(`GPBR_RECOVER_MAGIC`, `GPBR1_MARK`, `REPORT_*`, `RESULT_*`, `Trigger`,
`report`, `Report::parse`, `request_allowed`) and `fwimage.py` (`pack_report`,
`parse_report`). Vectors `report.txt`.

* **GPBR0 = request.** The app writes `GPBR_RECOVER_MAGIC` (`0x4352474E`,
  `"NGRC"` as a little-endian word) with interrupts off and resets with
  `RSTC_CR = 0xA5000005` (PROCRST | PERRST). The resident, before it touches
  the card, writes 0 if it reads the magic; any other value is left alone.
* **GPBR1 = report.** The resident writes it on every boot just before it
  jumps; the app reads it first thing and treats a word without the mark as no
  report (a cold GPBR is undefined).

```
 31        24 23        16 15         8 7          0
+------------+------------+------------+------------+
|    0x4E    |    0xB1    |   flags    |   result   |
+------------+------------+------------+------------+
flags (bit in the word)
  bit 8   REPORT_BTN       the button was held, so recovery was entered
  bit 9   REPORT_REQ       GPBR0 held the request, so recovery was entered
  bit 10  REPORT_WROTE     this boot wrote flash
  bit 11  REPORT_LOCKED    the last GLB read lock bit 0 of plane 0 as 1
  bit 12  REPORT_SLB       this boot issued SLB
  bit 13  REPORT_UNSEALED  an unsealed app was accepted (development resident only)
  bits 14-15 reserved, written 0, ignored when read
result  RESULT_NONE 0 no recovery tried | RESULT_RECOVERED 1 done (written, or already the same image)
        2..9 recovery failed and a valid old app was started; also the LED blink count:
        RESULT_NO_CARD 2, RESULT_NO_FILE 3, RESULT_READ 4, RESULT_HEADER 5,
        RESULT_LOCKED 6, RESULT_WRITE 7, RESULT_CRC 8, RESULT_VERIFY 9
```

`RecoverError::result_code` (`bootcore::update`) maps every recovery error to a
result (exhaustive `match`, so a new error needs a code); `sd_recover_ex`
returns `Recovered { len, wrote }` for `REPORT_WROTE`. "No card" (2) is decided
by the caller (card initialisation failed). A failure after the application's
first page was erased leaves no valid app: the resident keeps polling the card
and writes no report; the jump happens only after a recovery that succeeded or
with an app that is still valid.

**Rule X6 (`request_allowed`).** The app must not ask for recovery again when
the report says `REQ`: a recovery that did not help (an FPGA that never starts)
would otherwise reset for ever. With no report (a probe start, no resident, an
older resident) it must not ask at all: nothing would answer, and the reset
would start the same app again. With a report without `REQ` (a button boot, a
plain boot), it may ask once.

### Boot procedure the resident follows (reference: `mcu/bootcore/tests/common/sim.rs`)

1. Clocks, `FWS_RUN`; `lock_resident`: GLB, `lock_step` (SLB only if region 0
   reads unlocked and the build is not `dev`), FWS 6, SLB, FWS 4, GLB,
   `lock_report`. Never stops the boot.
2. Button: **one** read; only if pressed, two more 20 ms apart (`button_held`).
3. `verify_flash_app`, `decide`; on `Recover` clear GPBR0 first if the request
   was the cause; `sd_recover_ex`; `report` into GPBR1; hand-off with FWS 4,
   VTOR, MSP.

### Clock and flash wait-state constants (`bootcore::part`)

One definition for the resident (what it leaves) and the app (what it builds
when it has to; `mcu/embassy-atsam3/src/clocks.rs`, `efc.rs`):
`XTAL_HZ` 12 000 000, `MOSC_XTST` 8, `PLLA_MUL` 8 (`MULA` field = 7), `PLLA_DIV` 1,
`PLLA_COUNT` 0x3F, `MCK_HZ` 96 000 000 (PLLA, prescaler 1), `FWS_RUN` 4 (also the
hand-off value, both planes), `FWS_WRITE` 6 (only while an EWP or SLB runs),
`EEFC_FMR_RUN` `0x400`.

### Flash trait duties (`bootcore::flash::Flash`)

`set_fws(FWS_WRITE)` before `ewp`/`slb`, `set_fws(FWS_RUN)` right after (the
portable code does it; the host model counts an EWP/SLB at any other FWS, and
every flash read, hence the hand-off, at FWS 6 as a violation). `ewp` and
`BlockRead::read_block` rewind the watchdog (`WDT_CR = 0xA5000001`) as their
first act; the resident never writes `WDT_MR`. Measured bound in the model: at
most 254 008 flash bytes read between two rewinds (`tests/resident.rs`, limit
262 144).

### What is frozen with the resident (design 3-4)

`APP_BASE`, `layout`, the descriptor's position (+0x100), length (32), version
(1) and words 0/4/8/28, the trailer rule, the vector check, GPBR0's word,
GPBR1's layout, the information block's position and layout, the OGFW format-3
header, the file name `NETGD.OGF`, the LED counts, and design section 2 as a
whole. Everything else (NGAX, the app's descriptor words 12..27) is the app's.

### `fwimage.py`

* `build --elf E --target 2e|4e --format 3 -o out.ogfw`: refuses an ELF with
  load content below `APP_BASE` (`BadDest`: "link the app for the resident
  layout"), requires the descriptor magic at `APP_BASE + 0x100`
  (`BadDescriptor`), pads with `0xFF` leaving four bytes, fills descriptor
  length and CRC, writes the trailer, then computes `payload_crc` and the header
  CRC. If the ELF has a 4-byte section `.image_crc` ending at the end of the
  loaded image the trailer goes there instead of being appended. `--format 2`
  (the default for now) is unchanged.
* `seal in.elf -o out.elf` (development flashing: `probe-rs run` writes the
  ELF as it is, so the CRCs must be in it): needs PROGBITS sections
  `.app_desc` (32 bytes at `APP_BASE + 0x100`) and `.image_crc` (4 bytes, the
  last of an image whose length is a multiple of 512 starting at `APP_BASE`);
  writes descriptor length + CRC and the trailer into them, nothing else. The
  result passes `inspect`'s checks once built into an image, and sealing twice
  changes nothing.
* `inspect` reads both formats, prints `fmt=`/`layout=` per image, and for
  format 3 also verifies descriptor and trailer, prints the NGAX block, and
  checks the code image's `bs_stamp` against the bitstream image in the file.
  Given an 8192-byte file with `"NGRS"` at `0x1FF0` it checks a resident
  binary instead (region CRC, information block, vector table).
* `seal-resident resident.elf -o resident.bin [--target 2e|4e] [--distribution]`:
  the resident's ELF -> the 8192 bytes of lock region 0 (unused bytes `0xFF`).
  Refuses (`BadDest`) any flash load content outside `0x80000..0x82000`, a bad
  vector table (`BadVector`; the SP is checked against `--target`, default the
  smaller 2E SRAM, so a pass fits both parts), a missing or foreign information
  block (`BadResident`: magic, generation == `IFACE_GEN`, layout `0x2000`), and
  with `--distribution` a development build, one without `SELF_LOCK`, or one
  that writes `WDT_MR`. Fills the last four bytes with the CRC-32 of the first
  8188 (= the information block's CRC word). Idempotent.
* Format 3 with V3: see "Extension block" above; `seal` and `build` take the
  ELF sections `.bs_crc` and `.app_ax`.

### Shared vectors (`vectors/`, generated by `gen_vectors.py`, expectations written by hand)

Read by `fwimage.py` (`test_fwimage.py`), `mcu/fwupdater/src/logic.rs` and
`tools/netgd-fwupd/tests/test_all.c`; all three must agree on every row.

| file | row | checked by |
|------|-----|------------|
| `crc32.txt` | data, CRC | CRC-32 |
| `vtable.txt` | part, dest, len, SP, Reset, NMI, HardFault, expect | vector check (includes format-3 `dest = 0x82000` rows) |
| `header.txt` | part, 36 header bytes, expect | **format-2** reader: parse then check (a `format = 3` row is `BadHeader`) |
| `header3.txt` | part, 40 header bytes, expect | **format-3** reader (a format-2 header is `BadHeader`) |
| `desc.txt` | 32 descriptor bytes, image length, expect (`OK`, `Unsealed`, `BadDescriptor`) | descriptor check |
| `trailer.txt` | image bytes (trailer included), expect (`OK`, `BadTrailer`) | trailer check |
| `image3.txt` | board part, whole one-image file, expect (`OK`, `CrcMismatch`, `BadVector`, ...) | the format-3 reader end to end, in the order given above; each row fails in exactly one way |
| `report.txt` | `E` button request wrote locked slb unsealed result word / `R` word, flags, result (parse only, reserved bits set) / `N` word (no mark) | GPBR1 report encode and parse (`bootcore::boot`, `fwimage.py`) |
| `info.txt` | `I` 16 bytes, gen, flags, layout>>8, version, crc / `N` 16 bytes (version unknown) | resident information block parse |
| `ax.txt` | 32-byte NGAX block, bitstream trailer word, expect (`OK`, `NoAx`, `BadAx`, `StampMismatch`) | `check_ax`; the bitstream's own trailer rule is `trailer.txt` |

`report.txt`, `info.txt` and `ax.txt` are read by `fwimage.py` and
`mcu/bootcore` (`tests/contract.rs`); the app side should use bootcore's
functions rather than its own.

## TODO (S3, when a resident binary exists)

CI disassembly checks (contract F5, not done: there is no resident to
disassemble yet): the resident has no store to `WDT_MR` (`0x400E1254`), GPBR
addresses only `0x400E1290` and `0x400E1294`, one `EEFC_FCR` write site, no
write to the FPGA pins, PCK0, SMC, DMAC, UART or SRAM1; the app has no
`EEFC_FCR` address (`0x400E0804`, `0x400E0A04`). Add to
`.github/workflows/build.yaml` after the existing disassembly step.

## Updater status codes (ERROR register, with ERR)

`0x14` BadHeader (magic, format, header CRC, kind), `0x24` BadDest, `0x34`
BadLength, `0x44` NoHeader, `0x54` Overrun, `0x64` FlashError, `0x74`
CrcMismatch, `0x84` BadTarget (also: not a 2E/4E), `0x94` Locked, `0xA4`
BadVector, `0xB4` NotVerified (reset refused).

## Sending (for `netgd-recv` / a future BBA sender)

After the arm and enter steps of `fwupdate.rs`, for each image
(`fwimage.iter_images(data)` yields them, CRC-checked):

1. `F9 / feat 0x55`: PIO-write the 512-byte header block.
2. `F9 / feat 0x56, sector count = n`: PIO-write `n` x 512 payload bytes,
   at most 256 blocks per command, until `len` is sent. Wait for BSY to clear
   after each block.
3. `F9 / feat 0x57`: verify and commit. This CRCs the whole image, **then**
   programs page 0, then CRCs the whole flash image again. It takes a while
   (programming one page plus a read of the image).

After every image has verified: `F9 / feat 0x58` resets the board. It is
refused (`0xB4`) unless an image verified and none is half-written. A reset
regardless is `F9 / feat 0x59` with the 48-bit key `0x4F4746525354` in the
LBA registers (LBA48 order); do not use it unless the board is already lost.

On any error other than a failed header, restart that image from step 1.
Pages 1.. have already been rewritten by then, see below.

## What is still not safe

Pages 1..N are programmed as they arrive; page 0 (vector table) is written
last, only after the image as received passed its CRC. From the first page
erase until page 0 is rewritten (the whole transfer, tens of seconds), the
old vector table is in flash but points into partly overwritten code. A
power cut, a console reset, or a vanished host in that window leaves a board
that only SWD can recover. The same applies to the few milliseconds of the
page-0 write itself. Closing the window needs a resident boot region
(`docs/research/LOG.md` R37, countermeasure B).
