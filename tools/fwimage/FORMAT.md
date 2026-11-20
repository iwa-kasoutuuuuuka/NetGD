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

**Revised 2026-11-20 (findings B1..B12 of `docs/research/fable-debug-2026-11-20.md`,
LOG R89), still generation 1.** No resident exists yet and no board is locked,
so there is no "old generation 1" in the field to tell apart; `IFACE_GEN` and
the descriptor version (`DESC_VERSION`) both stay 1. What changed: GPBR0 gets
a second word written by the resident ("in progress"); GPBR1 gets flag bit 14
`LOCK_UNKNOWN` (was reserved) and result 10; `report.txt` has one more column;
`Flash::lock_bits` returns `Option<u32>`; an EWP timeout does not return; the
hand-off starts with the watchdog rewind; writing recovery attempts are bounded
per boot. Design section 10 has the old/new table.

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
(`GPBR_RECOVER_MAGIC`, `GPBR_RECOVER_BUSY`, `Request`, `GPBR1_MARK`,
`REPORT_*`, `RESULT_*`, `Trigger`, `report`, `Report::parse`,
`request_allowed`) and `fwimage.py` (`pack_report`, `parse_report`). Vectors
`report.txt`.

* **GPBR0 = request.** The app writes `GPBR_RECOVER_MAGIC` (`0x4352474E`,
  `"NGRC"` as a little-endian word) with interrupts off and resets with
  `RSTC_CR = 0xA5000005` (PROCRST | PERRST). The resident reads GPBR0 three
  ways (`Request::of`):

  | GPBR0 at reset | meaning | recovery? | written before the card is touched | written at hand-off |
  |----------------|---------|-----------|------------------------------------|---------------------|
  | `0x4352474E` (`GPBR_RECOVER_MAGIC`, written by the app) | a new request | yes | `0xBCADB8B1` | 0 |
  | `0xBCADB8B1` (`GPBR_RECOVER_BUSY` = `!MAGIC`, written by the resident) | a requested recovery was cut short by a reset | **not on that ground**; only if the app is invalid or the button is held (GPBR0 then stays as it is) | nothing | 0 |
  | anything else | nothing asked | only if the app is invalid or the button is held | nothing | nothing (not the resident's word) |

  Both of the resident's readings report `REQ`. With `GPBR_RECOVER_BUSY` and a
  valid app the resident starts the app at once, without touching the card,
  with `REQ` and `RESULT_INTERRUPTED` (10). Why not simply write 0 before
  recovering (the earlier draft): a reset in the middle (a card that hangs the
  resident until the watchdog fires, an EWP timeout, the reset button) would
  forget that the app had asked, the next boot would be a plain one, and the
  app would ask again: request, hang, 16 s, app, request, for ever.
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
  bit 14  REPORT_LOCK_UNKNOWN  the last GLB failed: whether region 0 is locked is not
                           known. LOCKED is 0 then and says nothing (never both)
  bit 15  reserved, written 0, ignored when read
result  RESULT_NONE 0 no recovery tried | RESULT_RECOVERED 1 done (written, or already the same image)
        2..9 recovery failed and a valid old app was started; also the LED blink count:
        RESULT_NO_CARD 2, RESULT_NO_FILE 3, RESULT_READ 4, RESULT_HEADER 5,
        RESULT_LOCKED 6 (also: lock bits unreadable and a valid app was kept),
        RESULT_WRITE 7, RESULT_CRC 8, RESULT_VERIFY 9
        RESULT_INTERRUPTED 10 GPBR0 held GPBR_RECOVER_BUSY: a requested recovery was cut
        short by a reset and was not tried again; always with REQ; never blinked
```

A reader that finds `LOCKED` and `LOCK_UNKNOWN` both set (never written) takes
it as unknown.

`RecoverError::result_code` (`bootcore::update`) maps every recovery error to a
result (exhaustive `match`, so a new error needs a code); `sd_recover_ex`
returns `Recovered { len, wrote }` for `REPORT_WROTE`, and `sd_recover_attempt`
returns `Attempt { result, wrote }`, where `wrote` also tells of a failed
attempt whether it issued flash commands. "No card" (2) is decided by the
caller (card initialisation failed). A failure after the application's first
page was erased leaves no valid app: the resident keeps polling the card and
writes no report; the jump happens only after a recovery that succeeded or
with an app that is still valid.

**Bounded rewriting (`after_failed_recovery`, `MAX_WRITING_ATTEMPTS` = 3).**
With no valid app the resident polls the card again after a failure
(`Fallback::RetryRecovery`). It counts, in RAM, the attempts of this boot
whose `wrote` is true; after the third one that failed it stops
(`Fallback::GiveUp`): it keeps showing the failure and rewinding the watchdog
and writes nothing more until a reset or power cycle. A card that validates
and then fails while the body is written would otherwise cost up to about 480
page writes at every poll, for as long as the board is powered. Attempts that
wrote nothing (no card, no file, a refused file) are not counted.

**"Already installed" (`sd_recover*` writes nothing).** Same app length and
trailer CRC, and for a file with a bitstream the CRC of plane 1 equals the
header's `payload_crc` **and the last word of plane 1 equals the file's**. A
stamped bitstream (V3) has the CRC residue `0x2144DF1C` as its `payload_crc`
whatever its content, so the CRC alone only says "some self-consistent
bitstream of this length". The check after writing is the same.

**Rule X6 (`request_allowed`).** The app must not ask for recovery again when
the report says `REQ`: a recovery that did not help (an FPGA that never starts)
would otherwise reset for ever. With no report (a probe start, no resident, an
older resident) it must not ask at all: nothing would answer, and the reset
would start the same app again. With a report without `REQ` (a button boot, a
plain boot), it may ask once.

### Boot procedure the resident follows (reference: `mcu/bootcore/tests/common/sim.rs`)

1. Clocks, `FWS_RUN`; `lock_resident`: GLB, `lock_step` (SLB only if the GLB
   worked, region 0 reads unlocked and the build is not `dev`), FWS 6, SLB,
   FWS 4, GLB, `lock_report` / `lock_unknown`. Never stops the boot. A GLB
   that fails (`lock_bits` = `None`: FCMDE, or no FRDY in time) is reported as
   `LOCK_UNKNOWN`, not as locked, and no SLB is issued.
2. Button: **one** read; only if pressed, two more 20 ms apart (`button_held`).
3. `verify_flash_app`, `decide`; on `Recover` store
   `Request::gpbr0_before_recovery` in GPBR0 first; `sd_recover_attempt`
   (it sets `FWS_RUN` itself before its first flash read: on a 4E nobody has
   set EEFC1's FWS yet); on failure `after_failed_recovery`.
4. Hand-off, in this order: **`WDT_CR` rewind first**; pins, HSMCI, SysTick
   back, FWS 4; GPBR0 = `Request::gpbr0_at_handoff`, GPBR1 = `report`; at
   least `WDT_RESTART_TO_JUMP_US` (100 us) after the rewind: VTOR, MSP, jump.
   (Later SAM datasheets forbid a `WDT_MR` write within three slow-clock
   periods, about 92 us, of a `WDT_CR` restart. The SAM3U datasheet does not
   say so; this is from memory of the SAM4/SAM E70 ones and is hardware check
   H9. The app's first act may be its single `WDT_MR` write.)

**Lock bits before a recovery (`lock_verdict`).** A target region that reads
locked: `Locked` (result 6), nothing written. Lock bits that cannot be read: if
the app in flash is valid, this attempt ends with `LockUnknown` (result 6) and
the app is untouched (a region locked in the middle of the image would
otherwise cost a working app); if there is no valid app the recovery goes
ahead, since there is nothing to lose and an EWP on a locked page is aborted
with FLOCKE. Nothing is remembered: the next attempt reads the lock bits
again, so a GLB failure never refuses recovery for good.

### Clock and flash wait-state constants (`bootcore::part`)

One definition for the resident (what it leaves) and the app (what it builds
when it has to; `mcu/embassy-atsam3/src/clocks.rs`, `efc.rs`):
`XTAL_HZ` 12 000 000, `MOSC_XTST` 8, `PLLA_MUL` 8 (`MULA` field = 7), `PLLA_DIV` 1,
`PLLA_COUNT` 0x3F, `MCK_HZ` 96 000 000 (PLLA, prescaler 1), `FWS_RUN` 4 (also the
hand-off value, both planes), `FWS_WRITE` 6 (only while an EWP or SLB runs),
`EEFC_FMR_RUN` `0x400`.

### Flash trait duties (`bootcore::flash::Flash`)

`set_fws(FWS_WRITE)` before `ewp`/`slb`, `set_fws(FWS_RUN)` right after (the
portable code does it; the host model counts an EWP/SLB at any other FWS,
every flash read, hence the hand-off, at FWS 6, and every flash read before
the first `set_fws` after reset as a violation). `ewp` and
`BlockRead::read_block` rewind the watchdog (`WDT_CR = 0xA5000001`) as their
first act; the resident never writes `WDT_MR`. Every command reads FSR once
before it writes FCR (stale FCMDE/FLOCKE clear on read) and takes its flags
from the FSR read that saw FRDY.

**An `ewp` that times out does not return.** The plane is unreadable while the
command is busy, and the caller of the RAM routine is code in that plane; a
return would fetch from a busy plane and fault. The RAM routine resets the chip
(`RSTC_CR = 0xA5000005`, written from RAM) or stops rewinding the watchdog.
That is a reset in the middle of a page write, which the power-cut sweep
covers. `FlashError::Timeout` is only ever returned by `slb` (flash stays
readable during SLB/GLB); `lock_bits` returns `None` on a timeout. Measured bound in the model: at
most 254 008 flash bytes read between two rewinds (`tests/resident.rs`, limit
262 144).

### What is frozen with the resident (design 3-4)

`APP_BASE`, `layout`, the descriptor's position (+0x100), length (32), version
(1) and words 0/4/8/28, the trailer rule, the vector check, GPBR0's two words
(request, in progress), GPBR1's layout (seven flags, results 0..10), the
information block's position and layout, the OGFW format-3 header, the file
name `NETGD.OGF`, the LED counts, the bound on writing attempts, and design
section 2 as a whole. Everything else (NGAX, the app's descriptor words 12..27) is the app's.

### `fwimage.py`

* `build` (both formats) refuses an ELF that is not a build for `--target`
  (finding B1 of R89: `--target 4e` used to take a SAM3U2E ELF, whose stack
  pointer `0x20004000` is a valid one on a 4E, with only a warning about the
  missing plane 1; on a format-2 4E board that file leaves the FPGA without a
  bitstream and only SWD recovers). Two checks on top of the updater's own,
  in `check_built_for`, both before the output file is written:
  * the initial SP must be **exactly** the end of the target's SRAM0 (2E
    `0x20004000`, 4E `0x20008000`), where every netgd linker script puts it:
    `BadTarget`. `--any-sp` for an ELF with a deliberately different stack;
  * a `--target 4e` ELF must have plane-1 content: `NoBitstream`.
    `--code-only` for a file with the code image alone (the board keeps the
    bitstream it has).

  A real 2E ELF needs both flags to become a 4E file. The updater and
  `inspect` are unchanged (they cannot know what the ELF was).
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
| `report.txt` | `E` button request wrote locked slb unsealed lock_unknown result word / `R` word, the seven flags, result (parse only: the reserved bit set, or LOCKED together with LOCK_UNKNOWN) / `N` word (no mark; includes the two GPBR0 words) | GPBR1 report encode and parse (`bootcore::boot`, `fwimage.py`) |
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

### SAM3U4E, format 2: the window between the two images

A 4E file has two images, code (plane 0) then bitstream (plane 1), and each is
verified and committed on its own (`0x57` after each). **Between the commit of
the code image and the commit of the bitstream image the board holds new code
with the old bitstream, and once the first bitstream page has been written,
new code with a partly written bitstream.** If the transfer stops there (power
cut, console reset, host gone, a flash error on plane 1), the next boot runs
the new code, the FPGA does not configure (or configures with a design the new
code does not match), and without the FPGA there is no ATA interface to send
`0xF9` through again: **only SWD recovers the board.** The updater's "reset
only after verify" rule does not help; it refuses the reset command, not a
power cut.

* The window is the whole bitstream transfer (about 50 KB, several seconds)
  plus its verify.
* Nothing in format 2 closes it: the two planes cannot be committed together,
  and sending the bitstream first only moves the window (new bitstream, old
  code).
* What reduces it: do not interrupt a 4E update; `netgd-fwupd` sends both
  images in one run and reports which one failed. A file without the plane-1
  image is refused by `build` unless `--code-only` is given, for the same
  reason (a 2E build flashed to a 4E has the same result).
* With a resident (format 3) the case is recoverable: the app compares its
  `bs_stamp` with the last word of plane 1 before it loads the FPGA, sees the
  mismatch, and asks the resident for an SD recovery, which rewrites both
  planes from `NETGD.OGF`. The same window exists during that recovery and is
  covered by its power-cut handling (the app is invalid until both are
  written).

The SAM3U2E has no plane 1 and no such window (its bitstream is inside the
code image or on the SD card).
