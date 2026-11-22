#!/usr/bin/env python3
"""Flash-size gate for the firmware ELF (standard library only).

Computes Berkeley-style text+data of a 32-bit little-endian ELF (the same
number `arm-none-eabi-size` prints as text + data), prints the margin against
a limit, appends it to $GITHUB_STEP_SUMMARY if set, and exits 1 when over.

The limit for the tightest build (2E, resident layout) lives here only:
  app area = 0x82000..0xA0000 (link/sam3u2e_resident.x FLASH, 0x1E000)
           = 122,880 bytes
  .image_pad rounds the image (+4-byte CRC trailer) up to 512 bytes, so a
  build at <= 122,880 - 512 = 122,368 always seals; docs/research/flash-budget-2e.md.
(The linker itself already errors "region FLASH overflowed" past 122,880;
this gate fails earlier, keeps the 512 B seal reserve and prints the margin.)

usage: check.py ELF [--label TEXT] [--gate-2e | --limit N]   (neither: sizes only)
"""
import argparse
import os
import struct
import sys

LIMIT_2E_RESIDENT = 122368

SHT_NOBITS = 8
SHF_WRITE = 1
SHF_ALLOC = 2


def berkeley_sizes(path):
    """Return (text, data, bss) like binutils `size` (Berkeley format)."""
    with open(path, "rb") as f:
        b = f.read()
    if b[:4] != b"\x7fELF" or b[4] != 1 or b[5] != 1:
        raise ValueError("%s: not a 32-bit little-endian ELF" % path)
    shoff, = struct.unpack_from("<I", b, 0x20)
    shentsize, shnum = struct.unpack_from("<HH", b, 0x2E)
    text = data = bss = 0
    for i in range(shnum):
        _n, typ, flags, _a, _o, size = struct.unpack_from(
            "<IIIIII", b, shoff + i * shentsize)
        if not flags & SHF_ALLOC:
            continue
        if typ == SHT_NOBITS:
            bss += size
        elif flags & SHF_WRITE:
            data += size
        else:
            text += size
    return text, data, bss


def report(label, text, data, bss, limit):
    total = text + data
    if limit is None:
        return "%s: text+data %d (text %d, data %d, bss %d)" % (
            label, total, text, data, bss), True
    margin = limit - total
    return "%s: text+data %d / limit %d, margin %d" % (
        label, total, limit, margin), margin >= 0


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("elf")
    ap.add_argument("--label", default=None)
    g = ap.add_mutually_exclusive_group()
    g.add_argument("--limit", type=int, default=None)
    g.add_argument("--gate-2e", action="store_true",
                   help="use LIMIT_2E_RESIDENT")
    a = ap.parse_args(argv)
    t, d, s = berkeley_sizes(a.elf)
    line, ok = report(a.label or os.path.basename(a.elf), t, d, s,
                      LIMIT_2E_RESIDENT if a.gate_2e else a.limit)
    print(line if ok else line + "  ** OVER LIMIT **")
    summ = os.environ.get("GITHUB_STEP_SUMMARY")
    if summ:
        with open(summ, "a") as f:
            f.write("%s%s\n\n" % (line, "" if ok else " **OVER LIMIT**"))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
