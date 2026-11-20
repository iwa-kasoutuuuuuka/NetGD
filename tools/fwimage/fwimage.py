#!/usr/bin/env python3
"""Build and inspect NetGD firmware update images ("OGFW").

    fwimage.py build --elf opengdemu --target 2e|4e -o out.ogfw [--image-version N]
    fwimage.py inspect out.ogfw [--target 2e|4e]

The format is what `mcu/fwupdater` expects over vendor opcode 0xF9; see
FORMAT.md. The checks here (header, CRC, vector table) are the same ones
`mcu/fwupdater/src/logic.rs` makes on the board, and both are tested against
the shared cases in `vectors/`.

Python standard library only. No sender over the BBA yet: that is for
`tools/netgd-recv` later, which will call `iter_images()` and push each
`Image` through the F9/55, F9/56, F9/57 sequence (FORMAT.md, "Sending").
"""

import argparse
import struct
import sys
import zlib
from dataclasses import dataclass

# --- format constants (keep in step with mcu/fwupdater/src/logic.rs) --------

MAGIC = 0x5746474F  # "OGFW" little-endian
FORMAT_VERSION = 2
HEADER_BYTES = 512
HEADER_WORDS = 9

KIND_CODE = 0
KIND_BITSTREAM = 1
KIND_NAMES = {KIND_CODE: "code", KIND_BITSTREAM: "bitstream"}

PLANE0_BASE = 0x00080000
PLANE1_BASE = 0x00100000
PLANE_LEN = 0x00020000
PAGE = 256
BLOCK = 512
LOCK_REGION = 8192

SRAM0_BASE = 0x20000000
CIDR_VERSION_MASK = 0x1F


@dataclass(frozen=True)
class PartInfo:
    name: str
    cidr: int
    sram0_end: int
    has_plane1: bool


PARTS = {
    "2e": PartInfo("SAM3U2E", 0x281A0761, SRAM0_BASE + 0x4000, False),
    "4e": PartInfo("SAM3U4E", 0x28100961, SRAM0_BASE + 0x8000, True),
}


def part_from_cidr(cidr):
    """Part key ('2e'/'4e') for a CHIPID_CIDR, ignoring the silicon revision."""
    for key, p in PARTS.items():
        if (cidr & ~CIDR_VERSION_MASK) == (p.cidr & ~CIDR_VERSION_MASK):
            return key
    return None


class FwImageError(Exception):
    """Raised with the same names the updater reports (BadHeader, ...)."""

    def __init__(self, kind, msg=""):
        super().__init__(f"{kind}: {msg}" if msg else kind)
        self.kind = kind


# --- CRC --------------------------------------------------------------------


def crc32(data):
    """CRC-32 as the updater computes it: reflected 0xEDB88320, init and
    final xor 0xFFFFFFFF -- identical to zlib.crc32."""
    return zlib.crc32(data) & 0xFFFFFFFF


# --- header -----------------------------------------------------------------


@dataclass(frozen=True)
class Header:
    kind: int
    target_cidr: int
    dest: int
    length: int
    payload_crc: int
    image_version: int = 0

    def pack(self):
        words = [MAGIC, FORMAT_VERSION, self.kind, self.target_cidr, self.dest,
                 self.length, self.payload_crc, self.image_version]
        body = struct.pack("<8I", *words)
        blk = body + struct.pack("<I", crc32(body))
        return blk + bytes(HEADER_BYTES - len(blk))


def parse_header(block):
    """Intrinsic checks only (magic, format, header CRC, kind): the
    equivalent of logic::parse_header."""
    if len(block) < HEADER_WORDS * 4:
        raise FwImageError("BadHeader", "short header")
    w = struct.unpack_from("<9I", block)
    if w[0] != MAGIC or w[1] != FORMAT_VERSION:
        raise FwImageError("BadHeader", "magic/format")
    if crc32(block[:32]) != w[8]:
        raise FwImageError("BadHeader", "header CRC")
    if w[2] not in KIND_NAMES:
        raise FwImageError("BadHeader", "unknown kind")
    return Header(w[2], w[3], w[4], w[5], w[6], w[7])


def check_header(h, part):
    """Is this image for `part` ('2e'/'4e') and aimed where it may go?
    Equivalent of logic::check_header."""
    p = PARTS[part]
    if part_from_cidr(h.target_cidr) != part:
        raise FwImageError("BadTarget", f"image is for CIDR {h.target_cidr:#010x}")
    if h.kind == KIND_CODE:
        ok = h.dest == PLANE0_BASE
    else:
        ok = h.dest == PLANE1_BASE and p.has_plane1
    if not ok:
        raise FwImageError("BadDest", f"{h.dest:#x} for {KIND_NAMES[h.kind]} on {p.name}")
    if h.length == 0 or h.length % BLOCK or h.length > PLANE_LEN:
        raise FwImageError("BadLength", str(h.length))


def check_vectors(words, dest, length, part):
    """First four words of a code image: SP, Reset, NMI, HardFault.
    Equivalent of logic::check_vectors."""
    sp = words[0]
    if sp % 4 or sp <= SRAM0_BASE or sp > PARTS[part].sram0_end:
        raise FwImageError("BadVector", f"SP {sp:#010x}")
    end = dest + length
    for name, h in zip(("Reset", "NMI", "HardFault"), words[1:4]):
        a = h & ~1
        if not (h & 1) or a < dest or a >= end:
            raise FwImageError("BadVector", f"{name} {h:#010x}")


# --- images -----------------------------------------------------------------


@dataclass(frozen=True)
class Image:
    header: Header
    payload: bytes

    def pack(self):
        return self.header.pack() + self.payload


def make_image(kind, target, dest, payload, image_version=0):
    """Pad `payload` to a whole number of blocks with 0xFF (erased flash) and
    wrap it in a header."""
    payload = bytes(payload)
    pad = -len(payload) % BLOCK
    payload += b"\xff" * pad
    h = Header(kind, PARTS[target].cidr, dest, len(payload), crc32(payload), image_version)
    return Image(h, payload)


def iter_images(data):
    """Split a .ogfw file into its images, verifying header CRC and payload
    CRC. This is the interface a sender uses."""
    off = 0
    n = 0
    while off < len(data):
        h = parse_header(data[off:off + HEADER_BYTES])
        if len(data) - off < HEADER_BYTES:
            raise FwImageError("BadHeader", "short header block")
        off += HEADER_BYTES
        if h.length == 0 or h.length % BLOCK or off + h.length > len(data):
            raise FwImageError("BadLength", f"image {n}")
        payload = data[off:off + h.length]
        off += h.length
        if crc32(payload) != h.payload_crc:
            raise FwImageError("CrcMismatch", f"image {n} payload")
        yield Image(h, payload)
        n += 1
    if n == 0:
        raise FwImageError("BadHeader", "empty file")


def validate_image(img, part):
    """Everything the updater would check before and while writing."""
    check_header(img.header, part)
    if img.header.kind == KIND_CODE:
        v = struct.unpack_from("<4I", img.payload)
        check_vectors(v, img.header.dest, img.header.length, part)


def validate_file(images, part):
    kinds = [i.header.kind for i in images]
    if len(set(kinds)) != len(kinds):
        raise FwImageError("BadHeader", "two images of the same kind")
    for img in images:
        validate_image(img, part)


# --- ELF --------------------------------------------------------------------

PT_LOAD = 1
EM_ARM = 40


def elf_load_segments(elf):
    """[(paddr, bytes)] for every PT_LOAD with file content. Uses the load
    (physical) address: .data lives in flash at its LMA."""
    if elf[:4] != b"\x7fELF" or elf[4] != 1 or elf[5] != 1:
        raise FwImageError("BadElf", "not a 32-bit little-endian ELF")
    e_machine = struct.unpack_from("<H", elf, 18)[0]
    if e_machine != EM_ARM:
        raise FwImageError("BadElf", f"machine {e_machine}, expected ARM")
    e_phoff = struct.unpack_from("<I", elf, 28)[0]
    e_phentsize, e_phnum = struct.unpack_from("<HH", elf, 42)
    segs = []
    for i in range(e_phnum):
        p_type, p_offset, _vaddr, p_paddr, p_filesz, _memsz, _flags, _align = \
            struct.unpack_from("<8I", elf, e_phoff + i * e_phentsize)
        if p_type != PT_LOAD or p_filesz == 0:
            continue
        if p_offset + p_filesz > len(elf):
            raise FwImageError("BadElf", "segment past end of file")
        segs.append((p_paddr, elf[p_offset:p_offset + p_filesz]))
    return segs


def flash_plane_bytes(segs, base):
    """The bytes of the segments inside the plane at `base`, as one run from
    `base` to the highest byte used, gaps 0xFF. None if the plane is unused."""
    top = base + PLANE_LEN
    used = []
    for addr, data in segs:
        end = addr + len(data)
        if end <= base or addr >= top:
            continue
        if addr < base or end > top:
            raise FwImageError("BadElf", f"segment {addr:#x}..{end:#x} straddles a plane boundary")
        used.append((addr, data))
    if not used:
        return None
    hi = max(a + len(d) for a, d in used)
    buf = bytearray(b"\xff" * (hi - base))
    written = bytearray(hi - base)
    for addr, data in used:
        o = addr - base
        if any(written[o:o + len(data)]):
            raise FwImageError("BadElf", f"overlapping segments at {addr:#x}")
        buf[o:o + len(data)] = data
        written[o:o + len(data)] = b"\x01" * len(data)
    return bytes(buf)


def images_from_elf(elf, target, image_version=0):
    segs = elf_load_segments(elf)
    for addr, data in segs:
        in_flash = (PLANE0_BASE <= addr < PLANE0_BASE + PLANE_LEN) or \
                   (PLANE1_BASE <= addr < PLANE1_BASE + PLANE_LEN)
        # RAM-only or NOLOAD content is not part of the image; anything else
        # not in flash would be silently dropped, so say so.
        if not in_flash and addr < SRAM0_BASE:
            raise FwImageError("BadElf", f"load segment at {addr:#x} is neither flash nor RAM")
    code = flash_plane_bytes(segs, PLANE0_BASE)
    if code is None:
        raise FwImageError("BadElf", "no loadable bytes in plane 0")
    bits = flash_plane_bytes(segs, PLANE1_BASE)
    if bits is not None and not PARTS[target].has_plane1:
        raise FwImageError("BadDest", f"ELF has plane-1 content but {PARTS[target].name} has no plane 1")
    images = [make_image(KIND_CODE, target, PLANE0_BASE, code, image_version)]
    if bits is not None:
        images.append(make_image(KIND_BITSTREAM, target, PLANE1_BASE, bits, image_version))
    validate_file(images, target)
    return images


# --- CLI --------------------------------------------------------------------


def cmd_build(args):
    with open(args.elf, "rb") as f:
        elf = f.read()
    images = images_from_elf(elf, args.target, args.image_version)
    out = b"".join(i.pack() for i in images)
    with open(args.output, "wb") as f:
        f.write(out)
    print(f"{args.output}: {len(out)} bytes for {PARTS[args.target].name}")
    describe(images)
    if args.target == "4e" and len(images) == 1:
        print("warning: no plane-1 content in the ELF; the file has a code image only", file=sys.stderr)


def describe(images):
    for n, i in enumerate(images):
        h = i.header
        extra = ""
        if h.kind == KIND_CODE:
            sp, reset = struct.unpack_from("<2I", i.payload)
            extra = f"  SP={sp:#010x} Reset={reset:#010x}"
        print(f"  image {n}: {KIND_NAMES[h.kind]:9} dest={h.dest:#09x} len={h.length} "
              f"({h.length // PAGE} pages) crc={h.payload_crc:#010x} cidr={h.target_cidr:#010x} "
              f"version={h.image_version}{extra}")


def cmd_inspect(args):
    with open(args.file, "rb") as f:
        data = f.read()
    images = list(iter_images(data))
    describe(images)
    guess = {part_from_cidr(i.header.target_cidr) for i in images}
    targets = [args.target] if args.target else sorted(k for k in guess if k)
    if not targets:
        raise FwImageError("BadTarget", "image is not for a SAM3U2E or SAM3U4E")
    for t in targets:
        validate_file(images, t)
        print(f"  OK for {PARTS[t].name}: header, CRC and vector table pass the updater's checks")


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    b = sub.add_parser("build", help="make an .ogfw from an opengdemu ELF")
    b.add_argument("--elf", required=True)
    b.add_argument("--target", required=True, choices=sorted(PARTS))
    b.add_argument("-o", "--output", required=True)
    b.add_argument("--image-version", type=lambda s: int(s, 0), default=0)
    b.set_defaults(fn=cmd_build)
    i = sub.add_parser("inspect", help="parse and check an .ogfw")
    i.add_argument("file")
    i.add_argument("--target", choices=sorted(PARTS), help="check against this part (default: the one the image names)")
    i.set_defaults(fn=cmd_inspect)
    args = ap.parse_args(argv)
    try:
        args.fn(args)
    except FwImageError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    except OSError as e:
        print(f"error: {e}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
