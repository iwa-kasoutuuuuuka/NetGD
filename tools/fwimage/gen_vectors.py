#!/usr/bin/env python3
"""Regenerate vectors/*.txt, the cases shared by the Rust tests in
mcu/fwupdater/src/logic.rs and test_fwimage.py. The expected results are
written here by hand, not computed by fwimage.py, so the two implementations
are checked against a third statement of the rules."""
import os
import struct
import zlib

import fwimage as F

here = os.path.dirname(os.path.abspath(__file__))
out = os.path.join(here, "vectors")
os.makedirs(out, exist_ok=True)

# --- crc32 -------------------------------------------------------------------
inputs = [b"", b"\x00", b"123456789", b"\xff" * 4, bytes(range(256)), b"OGFW" * 64]
with open(os.path.join(out, "crc32.txt"), "w", newline="\n") as f:
    f.write("# hex-data (- for empty)  crc32  (zlib.crc32; 123456789 -> cbf43926)\n")
    for d in inputs:
        f.write(f"{d.hex() or '-'} {zlib.crc32(d):08x}\n")

# --- vector table ------------------------------------------------------------
D = 0x80000
L = 0x20000
V = [
    # part dest len sp reset nmi hardfault expect
    ("2e", D, L, 0x20004000, 0x80101, 0x80103, 0x80105, "OK"),
    ("4e", D, L, 0x20008000, 0x80101, 0x80103, 0x80105, "OK"),
    ("4e", D, L, 0x20004000, 0x80101, 0x80103, 0x80105, "OK"),
    ("2e", D, L, 0x20008000, 0x80101, 0x80103, 0x80105, "BadVector"),  # 4E SP on a 2E
    ("2e", D, L, 0x20004004, 0x80101, 0x80103, 0x80105, "BadVector"),
    ("2e", D, L, 0x20000000, 0x80101, 0x80103, 0x80105, "BadVector"),  # SP at the very bottom
    ("2e", D, L, 0x20003FFC, 0x80101, 0x80103, 0x80105, "OK"),
    ("2e", D, L, 0x20003FFE, 0x80101, 0x80103, 0x80105, "BadVector"),  # unaligned
    ("2e", D, L, 0xFFFFFFFF, 0x80101, 0x80103, 0x80105, "BadVector"),  # erased flash
    ("2e", D, L, 0x20080000, 0x80101, 0x80103, 0x80105, "BadVector"),  # SRAM1
    ("2e", D, L, 0x20004000, 0x80100, 0x80103, 0x80105, "BadVector"),  # Reset not Thumb
    ("2e", D, L, 0x20004000, 0xFFFFFFFF, 0x80103, 0x80105, "BadVector"),
    ("2e", D, L, 0x20004000, 0x00000101, 0x80103, 0x80105, "BadVector"),  # reset below flash
    ("2e", D, L, 0x20004000, 0xA0001, 0x80103, 0x80105, "BadVector"),  # reset past the image
    ("2e", D, 0x1000, 0x20004000, 0x81001, 0x80103, 0x80105, "BadVector"),  # past a short image
    ("2e", D, 0x1000, 0x20004000, 0x80FFF, 0x80103, 0x80105, "OK"),
    ("2e", D, L, 0x20004000, 0x80101, 0x0, 0x80105, "BadVector"),  # NMI missing
    ("2e", D, L, 0x20004000, 0x80101, 0x80103, 0x80104, "BadVector"),  # HardFault not Thumb
    # format 3: the app is linked at APP_BASE, handlers must be inside [0x82000, 0xA0000)
    ("2e", 0x82000, 0x1E000, 0x20004000, 0x82101, 0x82103, 0x82105, "OK"),
    ("2e", 0x82000, 0x1E000, 0x20004000, 0x9FFFF, 0x82103, 0x82105, "OK"),
    ("2e", 0x82000, 0x1E000, 0x20004000, 0xA0001, 0x82103, 0x82105, "BadVector"),
    ("2e", 0x82000, 0x1E000, 0x20004000, 0x80101, 0x82103, 0x82105, "BadVector"),  # still linked at 0x80000
    ("2e", 0x82000, 0x1E000, 0x20004000, 0x81FFF, 0x82103, 0x82105, "BadVector"),  # in the resident region
    ("4e", 0x82000, 0x200, 0x20008000, 0x82101, 0x82103, 0x82105, "OK"),
    ("4e", 0x82000, 0x200, 0x20008000, 0x82201, 0x82103, 0x82105, "BadVector"),
]
with open(os.path.join(out, "vtable.txt"), "w", newline="\n") as f:
    f.write("# part dest len sp reset nmi hardfault expect   (hex)\n")
    for p, d, l, sp, rs, nmi, hf, e in V:
        f.write(f"{p} {d:x} {l:x} {sp:x} {rs:x} {nmi:x} {hf:x} {e}\n")

# --- header ------------------------------------------------------------------
def hdr(kind=0, cidr=0x281A0761, dest=0x80000, length=0x20000, crc=0x12345678, ver=7,
        magic=F.MAGIC, fmt=F.FORMAT_VERSION, fix_crc=True, flip=None):
    body = struct.pack("<8I", magic, fmt, kind, cidr, dest, length, crc, ver)
    c = zlib.crc32(body) & 0xFFFFFFFF
    if not fix_crc:
        c ^= 1
    blk = bytearray(body + struct.pack("<I", c))
    if flip is not None:
        blk[flip] ^= 0x01
    return bytes(blk).hex()

C2, C4 = 0x281A0761, 0x28100961
H = [
    ("2e", hdr(), "OK"),
    ("4e", hdr(cidr=C4), "OK"),
    ("4e", hdr(kind=1, cidr=C4, dest=0x100000), "OK"),
    ("4e", hdr(cidr=0x28100962), "OK"),  # other silicon revision
    ("2e", hdr(cidr=0x281A0765), "OK"),
    ("2e", hdr(length=0x1000), "OK"),
    ("2e", hdr(cidr=C4), "BadTarget"),  # 4E image on a 2E board
    ("4e", hdr(cidr=C2), "BadTarget"),
    ("2e", hdr(cidr=0x28190561), "BadTarget"),  # 3U1E
    ("2e", hdr(kind=1, dest=0x100000), "BadDest"),  # plane 1 on a 2E
    ("4e", hdr(kind=1, cidr=C4, dest=0x80000), "BadDest"),  # bitstream into the code plane
    ("4e", hdr(kind=0, cidr=C4, dest=0x100000), "BadDest"),  # code into the bitstream plane
    ("2e", hdr(dest=0x80100), "BadDest"),  # not the plane base
    ("2e", hdr(dest=0x20000000), "BadDest"),
    ("2e", hdr(length=0), "BadLength"),
    ("2e", hdr(length=0x20200), "BadLength"),
    ("2e", hdr(length=0x20001), "BadLength"),
    ("2e", hdr(magic=0x5746474E), "BadHeader"),
    ("2e", hdr(fmt=1), "BadHeader"),  # the old format has no CRC or target
    ("2e", hdr(fix_crc=False), "BadHeader"),
    ("2e", hdr(flip=20), "BadHeader"),  # a flipped length bit breaks the header CRC
    ("2e", hdr(kind=2), "BadHeader"),
    ("2e", hdr(fmt=3), "BadHeader"),  # format 3 is for boards with a resident region
]
with open(os.path.join(out, "header.txt"), "w", newline="\n") as f:
    f.write("# part header-bytes(36, hex) expect   (checked: parse_header then check_header)\n")
    for p, h, e in H:
        f.write(f"{p} {h[:72]} {e}\n")

# =============================================================================
# Format 3 (resident boot region). Expected results are written by hand.
# =============================================================================
AB = 0x82000  # APP_BASE
LAY = 0x2000


def hdr3(kind=0, cidr=C2, dest=AB, length=0x1E000, crc=0x12345678, ver=7, layout=LAY,
         magic=F.MAGIC, fmt=3, fix_crc=True, flip=None):
    """40 bytes (hex). Header CRC covers bytes 0..31 then the layout word."""
    body = struct.pack("<8I", magic, fmt, kind, cidr, dest, length, crc, ver)
    c = zlib.crc32(body + struct.pack("<I", layout)) & 0xFFFFFFFF
    if not fix_crc:
        c ^= 1
    blk = bytearray(body + struct.pack("<II", c, layout))
    if flip is not None:
        blk[flip] ^= 0x01
    return bytes(blk).hex()


H3 = [
    ("2e", hdr3(), "OK"),
    ("4e", hdr3(cidr=C4), "OK"),
    ("2e", hdr3(length=0x200), "OK"),
    ("2e", hdr3(length=0x1E000), "OK"),
    ("4e", hdr3(kind=1, cidr=C4, dest=0x100000, length=0x20000), "OK"),  # a bitstream may use the whole plane
    ("4e", hdr3(cidr=0x28100962), "OK"),  # other silicon revision
    ("2e", hdr3(cidr=C4), "BadTarget"),
    ("4e", hdr3(cidr=C2), "BadTarget"),
    ("2e", hdr3(cidr=0x28190561), "BadTarget"),  # 3U1E
    ("2e", hdr3(dest=0x80000), "BadDest"),  # format-2 address: inside the resident region
    ("2e", hdr3(dest=0x81F00), "BadDest"),  # anywhere before APP_BASE
    ("2e", hdr3(dest=0x82100), "BadDest"),  # not exactly APP_BASE
    ("2e", hdr3(dest=0x100000), "BadDest"),
    ("2e", hdr3(layout=0x4000), "BadDest"),  # an image for a 16 KB resident region
    ("2e", hdr3(layout=0), "BadDest"),
    ("2e", hdr3(kind=1, dest=0x100000, length=0x1000), "BadDest"),  # plane 1 on a 2E
    ("4e", hdr3(kind=1, cidr=C4, dest=AB), "BadDest"),  # bitstream into the app area
    ("2e", hdr3(length=0x1E200), "BadLength"),  # one block over 0x20000 - layout
    ("2e", hdr3(length=0x20000), "BadLength"),
    ("2e", hdr3(length=0), "BadLength"),
    ("2e", hdr3(length=0x1001), "BadLength"),
    ("4e", hdr3(kind=1, cidr=C4, dest=0x100000, length=0x20200), "BadLength"),
    ("2e", hdr(), "BadHeader"),  # a format-2 header (padded with a zero word): a resident board refuses it
    ("2e", hdr3(fmt=4), "BadHeader"),
    ("2e", hdr3(magic=0x5746474E), "BadHeader"),
    ("2e", hdr3(fix_crc=False), "BadHeader"),
    ("2e", hdr3(flip=20), "BadHeader"),  # a flipped length bit
    ("2e", hdr3(flip=36), "BadHeader"),  # a flipped layout bit: the layout word is under the CRC
    ("2e", hdr3(kind=2), "BadHeader"),
]
with open(os.path.join(out, "header3.txt"), "w", newline="\n") as f:
    f.write("# part header-bytes(40, hex) expect   (format 3: parse_header(formats=3) then check_header)\n")
    for p, h, e in H3:
        if len(h) == 72:
            h += "00000000"  # a 36-byte format-2 header, padded
        f.write(f"{p} {h[:80]} {e}\n")


# --- app descriptor ----------------------------------------------------------
def desc(magic=b"NGAP", ver=1, length=0x400, f12=0, f16=0, f20=0, f24=0, crc_xor=0, crc=None):
    body = magic + struct.pack("<6I", ver, length, f12, f16, f20, f24)
    c = (zlib.crc32(body) & 0xFFFFFFFF) if crc is None else crc
    return (body + struct.pack("<I", c ^ crc_xor)).hex()


# desc-hex(32 bytes) image-len(hex) expect.  Order of checks: magic, version,
# length 0 -> Unsealed, CRC, length == image length (and a multiple of 512).
D = [
    (desc(), 0x400, "OK"),
    (desc(length=0x1E000), 0x1E000, "OK"),
    (desc(length=0x200, f12=0x100000, f16=0x9000, f20=0x20000, f24=0x01000007), 0x200, "OK"),
    (desc(length=0), 0x400, "Unsealed"),  # linked, not sealed
    (desc(length=0, crc=0), 0x400, "Unsealed"),
    (desc(length=0, crc_xor=1), 0x400, "Unsealed"),  # length 0 is decided before the CRC
    (desc(magic=b"NGAQ"), 0x400, "BadDescriptor"),
    (desc(magic=b"\xff\xff\xff\xff", length=0xFFFFFFFF, crc=0xFFFFFFFF), 0x400, "BadDescriptor"),  # erased flash
    (desc(magic=b"NGAQ", length=0), 0x400, "BadDescriptor"),  # magic is decided before length 0
    (desc(ver=0), 0x400, "BadDescriptor"),
    (desc(ver=2), 0x400, "BadDescriptor"),
    (desc(crc_xor=1), 0x400, "BadDescriptor"),
    (desc(), 0x600, "BadDescriptor"),  # length differs from the image's
    (desc(length=0x401), 0x401, "BadDescriptor"),  # not a multiple of 512
    (desc(length=0x200), 0x400, "BadDescriptor"),
]
with open(os.path.join(out, "desc.txt"), "w", newline="\n") as f:
    f.write("# descriptor-hex(32 bytes) image-len(hex) expect   (OK, Unsealed, BadDescriptor)\n")
    for d, n, e in D:
        f.write(f"{d} {n:x} {e}\n")


# --- trailer -----------------------------------------------------------------
def with_crc(body, xor=0):
    return (body + struct.pack("<I", (zlib.crc32(body) & 0xFFFFFFFF) ^ xor)).hex()


flipped_body = bytearray(b"123456789")
flipped_body[3] ^= 0x04
T = [
    (with_crc(b""), "OK"),  # 4 bytes: the CRC of nothing
    (with_crc(b"123456789"), "OK"),
    (with_crc(bytes(range(256))), "OK"),
    (with_crc(b"\xff" * 508), "OK"),
    (b"123456789".hex() + "26" + "39" + "f4" + "cb", "OK"),  # the check value, spelled out
    (with_crc(b"123456789", xor=1), "BadTrailer"),
    (with_crc(b"123456789", xor=0x80000000), "BadTrailer"),
    (bytes(flipped_body).hex() + "263 9f4cb".replace(" ", ""), "BadTrailer"),  # body bit flipped, CRC of the original
    (b"123456789".hex() + "cbf43926", "BadTrailer"),  # right value, big-endian
    ("ffffffff", "BadTrailer"),  # erased trailer
    ("00000001", "BadTrailer"),
    ("-", "BadTrailer"),  # empty
    ("010203", "BadTrailer"),  # shorter than a trailer
]
with open(os.path.join(out, "trailer.txt"), "w", newline="\n") as f:
    f.write("# image-hex(trailer included, - for empty) expect   (OK, BadTrailer): last 4 bytes LE = crc32 of the rest\n")
    for d, e in T:
        f.write(f"{d or '-'} {e}\n")


# --- whole single-image files, as the format-3 (resident) reader sees them ---
def app_payload(sp, size=0x400, reset=AB + 0x101, nmi=AB + 0x103, hf=AB + 0x105, fill=0xA5):
    p = bytearray(bytes([fill]) * size)
    p[0:16] = struct.pack("<4I", sp, reset, nmi, hf)
    p[0x100:0x120] = bytes.fromhex(desc(length=0))  # unsealed, as linked
    return p


def seal_blob(p):
    """What make_image(fmt=3) does, written out here: descriptor length and
    CRC, then the trailer."""
    n = len(p)
    p = bytearray(p)
    p[0x108:0x10C] = struct.pack("<I", n)
    p[0x11C:0x120] = struct.pack("<I", zlib.crc32(bytes(p[0x100:0x11C])) & 0xFFFFFFFF)
    p[n - 4:n] = struct.pack("<I", zlib.crc32(bytes(p[:n - 4])) & 0xFFFFFFFF)
    return p


def trailer_only(p):
    p = bytearray(p)
    p[-4:] = struct.pack("<I", zlib.crc32(bytes(p[:-4])) & 0xFFFFFFFF)
    return p


def file3(payload, cidr=C2, dest=AB, kind=0, ver=3, fmt=3, layout=LAY, pcrc=None):
    pcrc = (zlib.crc32(bytes(payload)) & 0xFFFFFFFF) if pcrc is None else pcrc
    if fmt == 3:
        h = bytes.fromhex(hdr3(kind=kind, cidr=cidr, dest=dest, length=len(payload), crc=pcrc, ver=ver, layout=layout))
    else:
        h = bytes.fromhex(hdr(kind=kind, cidr=cidr, dest=dest, length=len(payload), crc=pcrc, ver=ver, fmt=fmt))
    return (h.ljust(512, b"\0") + bytes(payload)).hex()


good2 = seal_blob(app_payload(0x20004000))
good4 = seal_blob(app_payload(0x20008000, size=0x200))
unsealed = trailer_only(app_payload(0x20004000))
bad_desc_crc = bytearray(good2)
bad_desc_crc[0x11C] ^= 1
bad_desc_crc = trailer_only(bad_desc_crc)
no_desc = bytearray(app_payload(0x20004000))
no_desc[0x100:0x120] = b"\xff" * 32
no_desc = trailer_only(no_desc)
body_flip = bytearray(good2)
body_flip[0x300] ^= 0x10
erased_trailer = bytearray(good2)
erased_trailer[-4:] = b"\xff\xff\xff\xff"
bad_vec = seal_blob(app_payload(0x20004000, reset=AB + 0x100))  # Reset not Thumb
bad_vec_out = seal_blob(app_payload(0x20004000, hf=AB + 0x400))  # HardFault at the end of the image
len_mismatch = bytearray(good2)
len_mismatch[0x108:0x10C] = struct.pack("<I", 0x600)
len_mismatch[0x11C:0x120] = struct.pack("<I", zlib.crc32(bytes(len_mismatch[0x100:0x11C])) & 0xFFFFFFFF)
len_mismatch = trailer_only(len_mismatch)

# board-part file-hex expect.  Payload CRC and header CRC are fixed up in every
# row except the one that is about them, so each row fails in exactly one way.
F3 = [
    ("2e", file3(good2), "OK"),
    ("4e", file3(good2, cidr=C4), "OK"),  # SP 0x20004000 is fine on a 4E
    ("4e", file3(good4, cidr=C4), "OK"),
    ("2e", file3(good4, cidr=C4), "BadTarget"),  # image for a 4E on a 2E board
    ("2e", file3(good4), "BadVector"),  # 4E SP (0x20008000) in a 2E image
    ("2e", file3(bad_vec), "BadVector"),
    ("2e", file3(bad_vec_out), "BadVector"),
    ("2e", file3(unsealed), "Unsealed"),
    ("2e", file3(bad_desc_crc), "BadDescriptor"),
    ("2e", file3(no_desc), "BadDescriptor"),
    ("2e", file3(len_mismatch), "BadDescriptor"),
    ("2e", file3(body_flip), "BadTrailer"),
    ("2e", file3(erased_trailer), "BadTrailer"),
    ("2e", file3(good2, pcrc=0x12345678), "CrcMismatch"),
    ("2e", file3(good2, dest=0x80000), "BadDest"),  # linked for the old layout
    ("2e", file3(good2, layout=0x4000), "BadDest"),
    ("2e", file3(good2, fmt=2, dest=0x80000, layout=0), "BadHeader"),  # a format-2 file: not for a resident board
]
with open(os.path.join(out, "image3.txt"), "w", newline="\n") as f:
    f.write("# board-part file-hex expect   one format-3 image (512-byte header + payload) as the resident-region\n"
            "# reader (format 3 only) checks it: header, payload CRC, vectors, descriptor, trailer, in that order\n")
    for p, h, e in F3:
        f.write(f"{p} {h} {e}\n")
print("vectors written to", out)
