#!/usr/bin/env python3
"""Build and inspect NetGD firmware update images ("OGFW").

    fwimage.py build --elf opengdemu --target 2e|4e -o out.ogfw [--image-version N] [--format 2|3]
    fwimage.py seal in.elf -o sealed.elf
    fwimage.py inspect out.ogfw [--target 2e|4e]

The format is what `mcu/fwupdater` expects over vendor opcode 0xF9; see
FORMAT.md. The checks here (header, CRC, vector table) are the same ones
`mcu/fwupdater/src/logic.rs` makes on the board, and both are tested against
the shared cases in `vectors/`.

Format 2 is what boards without a resident boot region accept; format 3 is
for boards with one (docs/design/fw-bootloader.md section 2-6). `build`
writes format 2 unless told otherwise.

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
FORMAT_VERSION = 2  # the default format of `build`
FORMAT_2 = 2
FORMAT_3 = 3
FORMATS = (FORMAT_2, FORMAT_3)
HEADER_BYTES = 512
HEADER_WORDS = 9      # format 2: magic .. header_crc
HEADER_WORDS_3 = 10   # format 3: + layout

KIND_CODE = 0
KIND_BITSTREAM = 1
KIND_NAMES = {KIND_CODE: "code", KIND_BITSTREAM: "bitstream"}

PLANE0_BASE = 0x00080000
PLANE1_BASE = 0x00100000
PLANE_LEN = 0x00020000
PAGE = 256
BLOCK = 512
LOCK_REGION = 8192

# Format 3: the resident region (lock region 0) and the app behind it.
LAYOUT = 0x2000
APP_BASE = PLANE0_BASE + LAYOUT
DESC_OFFSET = 0x100
DESC_LEN = 32
DESC_MAGIC = b"NGAP"
DESC_VERSION = 1
TRAILER_LEN = 4

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
    format: int = FORMAT_2
    layout: int = 0  # format 3 only; format 2 has no such word

    def pack(self):
        words = [MAGIC, self.format, self.kind, self.target_cidr, self.dest,
                 self.length, self.payload_crc, self.image_version]
        body = struct.pack("<8I", *words)
        if self.format == FORMAT_3:
            blk = body + struct.pack("<I", header_crc3(body, self.layout)) + struct.pack("<I", self.layout)
        else:
            blk = body + struct.pack("<I", crc32(body))
        return blk + bytes(HEADER_BYTES - len(blk))


def header_crc3(body32, layout):
    """Format 3: CRC-32 of bytes 0..31 followed by the layout word (bytes
    36..39), i.e. of every field except the CRC word itself."""
    return crc32(body32 + struct.pack("<I", layout))


def parse_header(block, formats=FORMATS):
    """Intrinsic checks only (magic, format, header CRC, kind): the
    equivalent of logic::parse_header. `formats` is what this reader accepts:
    (2,) for a board without a resident region, (3,) for one with."""
    if len(block) < HEADER_WORDS * 4:
        raise FwImageError("BadHeader", "short header")
    w = struct.unpack_from("<9I", block)
    if w[0] != MAGIC or w[1] not in formats:
        raise FwImageError("BadHeader", "magic/format")
    layout = 0
    if w[1] == FORMAT_3:
        if len(block) < HEADER_WORDS_3 * 4:
            raise FwImageError("BadHeader", "short header")
        layout = struct.unpack_from("<I", block, 36)[0]
        ok = header_crc3(bytes(block[:32]), layout) == w[8]
    else:
        ok = crc32(bytes(block[:32])) == w[8]
    if not ok:
        raise FwImageError("BadHeader", "header CRC")
    if w[2] not in KIND_NAMES:
        raise FwImageError("BadHeader", "unknown kind")
    return Header(w[2], w[3], w[4], w[5], w[6], w[7], w[1], layout)


def check_header(h, part):
    """Is this image for `part` ('2e'/'4e') and aimed where it may go?
    Equivalent of logic::check_header (format 2) / check_header_v3."""
    p = PARTS[part]
    if part_from_cidr(h.target_cidr) != part:
        raise FwImageError("BadTarget", f"image is for CIDR {h.target_cidr:#010x}")
    v3 = h.format == FORMAT_3
    if v3 and h.layout != LAYOUT:
        raise FwImageError("BadDest", f"layout {h.layout:#x}, this build has {LAYOUT:#x}")
    if h.kind == KIND_CODE:
        ok = h.dest == (APP_BASE if v3 else PLANE0_BASE)
    else:
        ok = h.dest == PLANE1_BASE and p.has_plane1
    if not ok:
        raise FwImageError("BadDest", f"{h.dest:#x} for {KIND_NAMES[h.kind]} on {p.name}")
    limit = PLANE_LEN - LAYOUT if (v3 and h.kind == KIND_CODE) else PLANE_LEN
    if h.length == 0 or h.length % BLOCK or h.length > limit:
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


# --- app descriptor and trailer (format 3, design 2-5) ----------------------


def pack_descriptor(length=0, bit_addr=0, bit_len=0, bit_raw=0, kind_version=0):
    """The 32-byte "NGAP" descriptor. `length` 0 means unsealed."""
    body = DESC_MAGIC + struct.pack("<6I", DESC_VERSION, length, bit_addr, bit_len, bit_raw, kind_version)
    return body + struct.pack("<I", crc32(body))


def check_descriptor(desc, image_len):
    """Equivalent of logic::check_descriptor. `desc` is the 32 bytes at +0x100,
    `image_len` the length of the whole image (trailer included).
    Raises 'Unsealed' for a length-0 descriptor (never produced by `build`,
    only by a linked ELF that was not sealed) and 'BadDescriptor' otherwise.
    Order: magic, version, length 0 -> Unsealed, CRC, length == image_len."""
    if len(desc) < DESC_LEN or bytes(desc[:4]) != DESC_MAGIC:
        raise FwImageError("BadDescriptor", "no NGAP magic")
    w = struct.unpack_from("<8I", bytes(desc[:DESC_LEN]))
    if w[1] != DESC_VERSION:
        raise FwImageError("BadDescriptor", f"descriptor version {w[1]}")
    if w[2] == 0:
        raise FwImageError("Unsealed", "descriptor length is 0")
    if crc32(bytes(desc[:28])) != w[7]:
        raise FwImageError("BadDescriptor", "descriptor CRC")
    if w[2] != image_len or w[2] % BLOCK:
        raise FwImageError("BadDescriptor", f"descriptor length {w[2]} != image length {image_len}")
    return w


def check_trailer(image):
    """Equivalent of logic::check_trailer: the last 4 bytes are the CRC-32 of
    everything before them."""
    if len(image) < TRAILER_LEN:
        raise FwImageError("BadTrailer", "image too short")
    want = struct.unpack_from("<I", image, len(image) - TRAILER_LEN)[0]
    if crc32(bytes(image[:-TRAILER_LEN])) != want:
        raise FwImageError("BadTrailer", "image CRC")


def seal_payload(buf):
    """Fill descriptor length + CRC and the trailer of an app image whose
    length is already a multiple of 512 and whose last 4 bytes are reserved.
    Returns new bytes. The descriptor's other fields are the app's own."""
    buf = bytearray(buf)
    n = len(buf)
    if n % BLOCK or n < DESC_OFFSET + DESC_LEN + TRAILER_LEN:
        raise FwImageError("BadLength", f"app image of {n} bytes")
    if bytes(buf[DESC_OFFSET:DESC_OFFSET + 4]) != DESC_MAGIC:
        raise FwImageError("BadDescriptor", f"no NGAP descriptor at +{DESC_OFFSET:#x}")
    if struct.unpack_from("<I", buf, DESC_OFFSET + 4)[0] != DESC_VERSION:
        raise FwImageError("BadDescriptor", "descriptor version")
    struct.pack_into("<I", buf, DESC_OFFSET + 8, n)
    struct.pack_into("<I", buf, DESC_OFFSET + 28, crc32(bytes(buf[DESC_OFFSET:DESC_OFFSET + 28])))
    struct.pack_into("<I", buf, n - TRAILER_LEN, crc32(bytes(buf[:n - TRAILER_LEN])))
    return bytes(buf)


# --- images -----------------------------------------------------------------


@dataclass(frozen=True)
class Image:
    header: Header
    payload: bytes

    def pack(self):
        return self.header.pack() + self.payload


def make_image(kind, target, dest, payload, image_version=0, fmt=FORMAT_2):
    """Pad `payload` to a whole number of blocks with 0xFF (erased flash) and
    wrap it in a header. Format 3 code: four bytes are reserved after the
    padding for the trailer, the descriptor is sealed (it must exist at
    +0x100) and the trailer CRC is written."""
    payload = bytes(payload)
    if fmt == FORMAT_3 and kind == KIND_CODE:
        pad = -(len(payload) + TRAILER_LEN) % BLOCK
        payload = seal_payload(payload + b"\xff" * pad + bytes(TRAILER_LEN))
    else:
        payload += b"\xff" * (-len(payload) % BLOCK)
    h = Header(kind, PARTS[target].cidr, dest, len(payload), crc32(payload), image_version,
               fmt, LAYOUT if fmt == FORMAT_3 else 0)
    return Image(h, payload)


def iter_images(data, formats=FORMATS):
    """Split a .ogfw file into its images, verifying header CRC and payload
    CRC. This is the interface a sender uses."""
    off = 0
    n = 0
    while off < len(data):
        if len(data) - off < HEADER_BYTES:
            raise FwImageError("BadHeader", "short header block")
        h = parse_header(data[off:off + HEADER_BYTES], formats)
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
    h = img.header
    check_header(h, part)
    if h.kind == KIND_CODE:
        v = struct.unpack_from("<4I", img.payload)
        check_vectors(v, h.dest, h.length, part)
        if h.format == FORMAT_3:
            check_descriptor(img.payload[DESC_OFFSET:DESC_OFFSET + DESC_LEN], h.length)
            check_trailer(img.payload)


def validate_file(images, part):
    kinds = [i.header.kind for i in images]
    if len(set(kinds)) != len(kinds):
        raise FwImageError("BadHeader", "two images of the same kind")
    if len({i.header.format for i in images}) > 1:
        raise FwImageError("BadHeader", "images of different formats")
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


def flash_plane_bytes(segs, base, top=None):
    """The bytes of the segments inside the plane at `base`, as one run from
    `base` to the highest byte used, gaps 0xFF. None if the plane is unused."""
    top = base + PLANE_LEN if top is None else top
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


def elf_sections(elf):
    """{name: (addr, file_offset, size, sh_type)} from the section headers.
    Empty if the ELF has none."""
    shoff = struct.unpack_from("<I", elf, 32)[0]
    shentsize, shnum, shstrndx = struct.unpack_from("<HHH", elf, 46)
    if not shoff or not shnum:
        return {}
    if shoff + shnum * shentsize > len(elf) or shentsize < 40:
        raise FwImageError("BadElf", "section headers past end of file")
    sh = [struct.unpack_from("<10I", elf, shoff + i * shentsize) for i in range(shnum)]
    if shstrndx >= shnum:
        raise FwImageError("BadElf", "bad section name table")
    stroff = sh[shstrndx][4]
    out = {}
    for name, typ, _fl, addr, off, size, *_ in sh:
        end = elf.find(b"\0", stroff + name)
        if end < 0:
            raise FwImageError("BadElf", "unterminated section name")
        if name or typ:
            out[elf[stroff + name:end].decode("ascii", "replace")] = (addr, off, size, typ)
    return out


SHT_NOBITS = 8
SEC_DESC = ".app_desc"
SEC_CRC = ".image_crc"


def app_bytes_from_elf(segs):
    """Format 3: the app image (from APP_BASE) out of the ELF's plane-0 load
    segments, gaps 0xFF, as `flash_plane_bytes` would. Anything loaded below
    APP_BASE (the ELF is still linked at 0x80000) is refused."""
    for addr, data in segs:
        end = addr + len(data)
        if addr < APP_BASE and end > PLANE0_BASE:
            raise FwImageError("BadDest", f"load address {addr:#x} is below APP_BASE {APP_BASE:#x}; "
                                          "link the app for the resident layout (format 3)")
    return flash_plane_bytes(segs, APP_BASE, PLANE0_BASE + PLANE_LEN)


def images_from_elf(elf, target, image_version=0, fmt=FORMAT_2):
    segs = elf_load_segments(elf)
    for addr, data in segs:
        in_flash = (PLANE0_BASE <= addr < PLANE0_BASE + PLANE_LEN) or \
                   (PLANE1_BASE <= addr < PLANE1_BASE + PLANE_LEN)
        # RAM-only or NOLOAD content is not part of the image; anything else
        # not in flash would be silently dropped, so say so.
        if not in_flash and addr < SRAM0_BASE:
            raise FwImageError("BadElf", f"load segment at {addr:#x} is neither flash nor RAM")
    if fmt == FORMAT_3:
        code = app_bytes_from_elf(segs)
        code_dest = APP_BASE
    else:
        code = flash_plane_bytes(segs, PLANE0_BASE)
        code_dest = PLANE0_BASE
    if code is None:
        raise FwImageError("BadElf", "no loadable bytes in the code plane")
    if fmt == FORMAT_3:
        # a linker script may reserve the trailer as `.image_crc`; then the
        # image ends right after it and nothing is appended
        sec = elf_sections(elf).get(SEC_CRC)
        if sec and sec[0] + TRAILER_LEN - APP_BASE == len(code) and len(code) % BLOCK == 0:
            code = code[:-TRAILER_LEN]
        if bytes(code[DESC_OFFSET:DESC_OFFSET + 4]) != DESC_MAGIC:
            raise FwImageError("BadDescriptor", f"no NGAP descriptor at {APP_BASE + DESC_OFFSET:#x}")
    bits = flash_plane_bytes(segs, PLANE1_BASE)
    if bits is not None and not PARTS[target].has_plane1:
        raise FwImageError("BadDest", f"ELF has plane-1 content but {PARTS[target].name} has no plane 1")
    images = [make_image(KIND_CODE, target, code_dest, code, image_version, fmt)]
    if bits is not None:
        images.append(make_image(KIND_BITSTREAM, target, PLANE1_BASE, bits, image_version, fmt))
    validate_file(images, target)
    return images


def seal_elf(elf):
    """Development flashing (design 2-5 (1)): write the descriptor length and
    CRC and the trailer CRC into the ELF itself, so `probe-rs run` flashes a
    sealed image. Needs `.app_desc` and `.image_crc` sections (PROGBITS) in
    the ELF; `.image_crc` must be the last 4 bytes of an image that is a
    multiple of 512 long and starts at APP_BASE. Returns the new ELF bytes."""
    secs = elf_sections(elf)
    if SEC_DESC not in secs or SEC_CRC not in secs:
        raise FwImageError("BadElf", f"ELF has no {SEC_DESC} / {SEC_CRC} section (not linked for the resident layout)")
    d_addr, d_off, d_size, d_typ = secs[SEC_DESC]
    c_addr, c_off, c_size, c_typ = secs[SEC_CRC]
    if SHT_NOBITS in (d_typ, c_typ):
        raise FwImageError("BadElf", "section without file content (NOBITS)")
    if d_addr != APP_BASE + DESC_OFFSET or d_size != DESC_LEN:
        raise FwImageError("BadDescriptor", f"{SEC_DESC} must be {DESC_LEN} bytes at {APP_BASE + DESC_OFFSET:#x}")
    if c_size != TRAILER_LEN:
        raise FwImageError("BadElf", f"{SEC_CRC} must be {TRAILER_LEN} bytes")
    n = c_addr + TRAILER_LEN - APP_BASE
    if n % BLOCK or n <= DESC_OFFSET + DESC_LEN:
        raise FwImageError("BadLength", f"image would be {n} bytes (not a multiple of {BLOCK})")
    app = app_bytes_from_elf(elf_load_segments(elf))
    if app is None or len(app) != n:
        raise FwImageError("BadElf", f"{SEC_CRC} is not at the end of the loaded image")
    sealed = seal_payload(app[:n - TRAILER_LEN] + bytes(TRAILER_LEN))
    out = bytearray(elf)
    out[d_off:d_off + DESC_LEN] = sealed[DESC_OFFSET:DESC_OFFSET + DESC_LEN]
    out[c_off:c_off + TRAILER_LEN] = sealed[n - TRAILER_LEN:]
    return bytes(out)


# --- CLI --------------------------------------------------------------------


def cmd_build(args):
    with open(args.elf, "rb") as f:
        elf = f.read()
    images = images_from_elf(elf, args.target, args.image_version, args.format)
    out = b"".join(i.pack() for i in images)
    with open(args.output, "wb") as f:
        f.write(out)
    print(f"{args.output}: {len(out)} bytes for {PARTS[args.target].name}, format {args.format}")
    describe(images)
    if args.target == "4e" and len(images) == 1:
        print("warning: no plane-1 content in the ELF; the file has a code image only", file=sys.stderr)


def cmd_seal(args):
    with open(args.elf, "rb") as f:
        elf = f.read()
    sealed = seal_elf(elf)
    with open(args.output, "wb") as f:
        f.write(sealed)
    desc = elf_sections(sealed)[SEC_DESC]
    d = sealed[desc[1]:desc[1] + DESC_LEN]
    print(f"{args.output}: sealed, image length {struct.unpack_from('<I', d, 8)[0]}, "
          f"descriptor crc {struct.unpack_from('<I', d, 28)[0]:#010x}")


def describe(images):
    for n, i in enumerate(images):
        h = i.header
        extra = ""
        if h.kind == KIND_CODE:
            sp, reset = struct.unpack_from("<2I", i.payload)
            extra = f"  SP={sp:#010x} Reset={reset:#010x}"
        fmt = f"fmt={h.format}" + (f" layout={h.layout:#x}" if h.format == FORMAT_3 else "")
        print(f"  image {n}: {KIND_NAMES[h.kind]:9} {fmt} dest={h.dest:#09x} len={h.length} "
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
        extra = ""
        if images[0].header.format == FORMAT_3:
            extra = ", app descriptor and trailer CRC"
        print(f"  OK for {PARTS[t].name}: header, CRC and vector table{extra} pass the updater's checks")


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    b = sub.add_parser("build", help="make an .ogfw from an opengdemu ELF")
    b.add_argument("--elf", required=True)
    b.add_argument("--target", required=True, choices=sorted(PARTS))
    b.add_argument("-o", "--output", required=True)
    b.add_argument("--image-version", type=lambda s: int(s, 0), default=0)
    b.add_argument("--format", type=int, choices=FORMATS, default=FORMAT_VERSION,
                   help="2: boards without a resident region (default); 3: with one (app linked at 0x82000)")
    b.set_defaults(fn=cmd_build)
    s = sub.add_parser("seal", help="write descriptor length/CRC and the image CRC into the ELF (development flashing)")
    s.add_argument("elf")
    s.add_argument("-o", "--output", required=True)
    s.set_defaults(fn=cmd_seal)
    i = sub.add_parser("inspect", help="parse and check an .ogfw (format 2 or 3)")
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
