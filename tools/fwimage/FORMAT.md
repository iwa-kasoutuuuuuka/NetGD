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
| `kind` 1 | bitstream, `dest` `0x00100000`, 4E only | the same |
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

### Trailer

The last four bytes of a code image are the CRC-32 (same algorithm) of all
bytes before them, little endian. An image shorter than four bytes is
`BadTrailer`.

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
  format 3 also verifies descriptor and trailer.

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
