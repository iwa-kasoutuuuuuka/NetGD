#!/usr/bin/env python3
"""Flash-size gate for the firmware ELF (standard library only).

Gate (--gate-2e, resident layout, link/sam3u2e_resident.x):
  FLASH = ORIGIN 0x82000, LENGTH 0x1E000 = 122,880 bytes (the 2E app area).
  The ELF already contains `.image_pad` (rounds the image up so that the
  4-byte `.image_crc` trailer ends on a 512 B boundary) and `.image_crc`.
  Real usage before padding is therefore
      used = addr(.image_pad) - FLASH_ORIGIN + 4      (+4 = CRC trailer)
  and the sealed image ends at addr(.image_crc) + 4 - FLASH_ORIGIN.
  The padded size is round-up(used, 512) and 122,880 is a multiple of 512,
  so the image fits exactly when used <= 122,880. The linker fails with
  "region FLASH overflowed" at the same point; the gate adds the margin print.
  (Berkeley text+data would include the pad and move only in 512 B steps.)

Without --gate-2e it prints Berkeley text+data / bss (informational).

usage: check.py ELF [--label TEXT] [--gate-2e | --limit N]
  --limit N applies N to the Berkeley text+data number.
"""
import argparse
import os
import struct
import sys

FLASH_ORIGIN = 0x82000
FLASH_LENGTH = 0x1E000            # 122,880
EM_ARM = 40
SHT_NOBITS = 8
SHF_WRITE = 1
SHF_ALLOC = 2


class ElfError(Exception):
    pass


def read_sections(path):
    """Return a list of (name, type, flags, addr, size)."""
    with open(path, "rb") as f:
        b = f.read()
    if len(b) < 0x34 or b[:4] != b"\x7fELF" or b[4] != 1 or b[5] != 1:
        raise ElfError("%s: not a 32-bit little-endian ELF" % path)
    machine, = struct.unpack_from("<H", b, 0x12)
    if machine != EM_ARM:
        raise ElfError("%s: e_machine %d, expected ARM (40)" % (path, machine))
    shoff, = struct.unpack_from("<I", b, 0x20)
    shentsize, shnum, shstrndx = struct.unpack_from("<HHH", b, 0x2E)
    if shentsize < 40 or shnum == 0 or shoff + shnum * shentsize > len(b):
        raise ElfError("%s: section table out of bounds" % path)
    raw = [struct.unpack_from("<IIIIIIIIII", b, shoff + i * shentsize)
           for i in range(shnum)]
    names = b""
    if shstrndx < shnum:
        so, ss = raw[shstrndx][4], raw[shstrndx][5]
        if so + ss > len(b):
            raise ElfError("%s: section name table out of bounds" % path)
        names = b[so:so + ss]
    out = []
    for nm, typ, flags, addr, _off, size, *_ in raw:
        end = names.find(b"\0", nm)
        name = names[nm:end if end >= 0 else None].decode("ascii", "replace")
        out.append((name, typ, flags, addr, size))
    return out


def berkeley_sizes(path):
    """(text, data, bss) like binutils `size` (Berkeley format)."""
    text = data = bss = 0
    for _n, typ, flags, _a, size in read_sections(path):
        if not flags & SHF_ALLOC:
            continue
        if typ == SHT_NOBITS:
            bss += size
        elif flags & SHF_WRITE:
            data += size
        else:
            text += size
    return text, data, bss


def resident_usage(path):
    """(used, image): used = bytes before .image_pad plus the 4-byte CRC
    trailer; image = padded image size (end of .image_crc)."""
    secs = {s[0]: s for s in read_sections(path)}
    for need in (".image_pad", ".image_crc"):
        if need not in secs:
            raise ElfError("%s: no %s section (not a resident-layout ELF?)"
                           % (path, need))
    pad, crc = secs[".image_pad"], secs[".image_crc"]
    if pad[3] < FLASH_ORIGIN:
        raise ElfError("%s: .image_pad at 0x%X is below FLASH 0x%X"
                       % (path, pad[3], FLASH_ORIGIN))
    return pad[3] - FLASH_ORIGIN + 4, crc[3] + crc[4] - FLASH_ORIGIN


def report_resident(label, used, image, limit=FLASH_LENGTH):
    margin = limit - used
    line = ("%s: flash used %d / limit %d, margin %d (padded image %d)"
            % (label, used, limit, margin, image))
    return line, margin >= 0


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
                   help="resident layout: pre-pad usage vs 122,880")
    a = ap.parse_args(argv)
    label = a.label or os.path.basename(a.elf)
    try:
        if a.gate_2e:
            line, ok = report_resident(label, *resident_usage(a.elf))
        else:
            line, ok = report(label, *berkeley_sizes(a.elf), a.limit)
    except (ElfError, OSError) as e:
        print("error: %s" % e, file=sys.stderr)
        return 2
    print(line if ok else line + "  ** OVER LIMIT **")
    summ = os.environ.get("GITHUB_STEP_SUMMARY")
    if summ:
        with open(summ, "a") as f:
            f.write("%s%s\n\n" % (line, "" if ok else " **OVER LIMIT**"))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
