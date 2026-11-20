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
]
with open(os.path.join(out, "header.txt"), "w", newline="\n") as f:
    f.write("# part header-bytes(36, hex) expect   (checked: parse_header then check_header)\n")
    for p, h, e in H:
        f.write(f"{p} {h[:72]} {e}\n")
print("vectors written to", out)
