#!/usr/bin/env python3
"""Build .ogfw files with the real tools/fwimage/fwimage.py, for the C host test to parse and send
through the fake updater (cross-checks CRC, header packing and the format against the Python).

    python gen_golden.py OUTDIR
    FWIMAGE_ELF_2E=... FWIMAGE_ELF_4E=...   also build real_2e.ogfw / real_4e.ogfw from real ELFs
"""
import os
import random
import struct
import sys

sys.dont_write_bytecode = True   # do not leave __pycache__ in tools/fwimage

HERE = os.path.dirname(os.path.abspath(__file__))
FWIMAGE = os.path.join(HERE, "..", "..", "fwimage")
sys.path.insert(0, FWIMAGE)
import fwimage as F  # noqa: E402
from test_fwimage import make_elf  # noqa: E402


def body(rng, n, vt):
    return vt + bytes(rng.randrange(256) for _ in range(n - len(vt)))


def vtable(sp, base):
    return struct.pack("<4I", sp, base + 0x101, base + 0x103, base + 0x105)


def write_file(path, images):
    with open(path, "wb") as f:
        f.write(b"".join(i.pack() for i in images))


def main(out):
    os.makedirs(out, exist_ok=True)
    rng = random.Random(7)
    # 2E: one code image (plane 0 only)
    elf = make_elf([(F.PLANE0_BASE, body(rng, 0x5000, vtable(0x20004000, F.PLANE0_BASE)))])
    imgs = F.images_from_elf(elf, "2e", image_version=0x1234)
    write_file(os.path.join(out, "golden_2e.ogfw"), imgs)
    # 4E: code + bitstream
    elf = make_elf([(F.PLANE0_BASE, body(rng, 0x20000, vtable(0x20008000, F.PLANE0_BASE))),
                    (F.PLANE1_BASE, body(rng, 0x9000 + 100, b""))])
    imgs = F.images_from_elf(elf, "4e", image_version=77)
    write_file(os.path.join(out, "golden_4e.ogfw"), imgs)
    # real firmware ELFs, if given (see tools/fwimage/README_ja.md)
    for key, tgt in (("FWIMAGE_ELF_2E", "2e"), ("FWIMAGE_ELF_4E", "4e")):
        p = os.environ.get(key)
        if p:
            with open(p, "rb") as f:
                imgs = F.images_from_elf(f.read(), tgt, image_version=1)
            write_file(os.path.join(out, "real_%s.ogfw" % tgt), imgs)
            print("built real_%s.ogfw from %s" % (tgt, p))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1] if len(sys.argv) > 1 else "golden"))
