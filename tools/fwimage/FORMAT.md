# OGFW image format, version 2

What `mcu/fwupdater` (vendor opcode `0xF9`, after the arm and enter keys)
accepts, and what `fwimage.py` writes. The authoritative implementation of the
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
| 1    | 4      | `format`       | `2` (format 1, which had no CRC or target, is refused) |
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
