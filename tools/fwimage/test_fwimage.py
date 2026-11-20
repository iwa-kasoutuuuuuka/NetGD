"""Tests for fwimage.py.   python -m unittest discover -s tools/fwimage -v

Real-ELF tests run when these point at ELFs built with the commands in
CLAUDE.md (each build overwrites the previous one, so copy them aside):

    FWIMAGE_ELF_4E=...  cargo build --release
    FWIMAGE_ELF_2E=...  DEFMT_LOG=off cargo build --release --features sam3u2e
"""

import contextlib
import io
import os
import struct
import sys
import tempfile
import unittest
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import fwimage as F  # noqa: E402


def vector_lines(name):
    with open(os.path.join(HERE, "vectors", name), encoding="utf-8") as f:
        return [l.split() for l in f if l.strip() and not l.startswith("#")]


def make_elf(segments):
    """Minimal ELF32-LE/ARM with one PT_LOAD per (paddr, bytes)."""
    n = len(segments)
    phoff = 52
    data_off = phoff + 32 * n
    ehdr = b"\x7fELF" + bytes([1, 1, 1, 0]) + bytes(8)
    ehdr += struct.pack("<HHIIIIIHHHHHH", 2, F.EM_ARM, 1, 0, phoff, 0, 0x05000000, 52, 32, n, 40, 0, 0)
    ph = b""
    body = b""
    for paddr, data in segments:
        ph += struct.pack("<8I", F.PT_LOAD, data_off + len(body), paddr, paddr, len(data), len(data), 5, 4)
        body += data
    return ehdr + ph + body


def vtable(sp=0x20004000, reset=0x80101, nmi=0x80103, hf=0x80105):
    return struct.pack("<4I", sp, reset, nmi, hf)


class SharedVectors(unittest.TestCase):
    def test_crc32(self):
        rows = vector_lines("crc32.txt")
        self.assertGreaterEqual(len(rows), 4)
        for data, want in rows:
            d = b"" if data == "-" else bytes.fromhex(data)
            self.assertEqual(F.crc32(d), int(want, 16), data)
        # the one everybody knows
        self.assertEqual(F.crc32(b"123456789"), 0xCBF43926)

    def test_vtable(self):
        rows = vector_lines("vtable.txt")
        self.assertGreaterEqual(len(rows), 10)
        for part, dest, ln, sp, rs, nmi, hf, want in rows:
            w = [int(x, 16) for x in (sp, rs, nmi, hf)]
            try:
                F.check_vectors(w, int(dest, 16), int(ln, 16), part)
                got = "OK"
            except F.FwImageError as e:
                got = e.kind
            self.assertEqual(got, want, (part, dest, ln, sp, rs, nmi, hf))

    def test_header(self):
        rows = vector_lines("header.txt")
        self.assertGreaterEqual(len(rows), 10)
        for part, hexs, want in rows:
            blk = bytes.fromhex(hexs)
            try:
                F.check_header(F.parse_header(blk), part)
                got = "OK"
            except F.FwImageError as e:
                got = e.kind
            self.assertEqual(got, want, (part, hexs))


class Images(unittest.TestCase):
    def code(self, n=0x2000, **kw):
        body = bytearray(vtable(**kw).ljust(n, b"\xa5"))
        return bytes(body)

    def test_header_roundtrip(self):
        h = F.Header(F.KIND_CODE, 0x281A0761, 0x80000, 0x20000, 0xDEADBEEF, 42)
        blk = h.pack()
        self.assertEqual(len(blk), 512)
        self.assertEqual(blk[:4], b"OGFW")
        self.assertEqual(F.parse_header(blk), h)
        self.assertEqual(blk[36:], bytes(512 - 36))

    def test_make_image_pads_and_crcs(self):
        img = F.make_image(F.KIND_CODE, "2e", F.PLANE0_BASE, self.code(0x2001))
        self.assertEqual(img.header.length, 0x2200)
        self.assertEqual(img.payload[0x2001:], b"\xff" * 0x1FF)
        self.assertEqual(img.header.payload_crc, zlib.crc32(img.payload))
        self.assertEqual(img.header.target_cidr, 0x281A0761)

    def test_file_roundtrip_and_tamper(self):
        a = F.make_image(F.KIND_CODE, "4e", F.PLANE0_BASE, self.code())
        b = F.make_image(F.KIND_BITSTREAM, "4e", F.PLANE1_BASE, b"\x01\x02\x03" * 700)
        data = a.pack() + b.pack()
        got = list(F.iter_images(data))
        self.assertEqual(got, [a, b])
        F.validate_file(got, "4e")
        bad = bytearray(data)
        bad[512 + 100] ^= 1  # payload
        with self.assertRaises(F.FwImageError) as cm:
            list(F.iter_images(bytes(bad)))
        self.assertEqual(cm.exception.kind, "CrcMismatch")
        bad = bytearray(data)
        bad[9] ^= 1  # header
        with self.assertRaises(F.FwImageError) as cm:
            list(F.iter_images(bytes(bad)))
        self.assertEqual(cm.exception.kind, "BadHeader")
        with self.assertRaises(F.FwImageError):
            list(F.iter_images(data[:-512]))  # truncated payload
        with self.assertRaises(F.FwImageError):
            list(F.iter_images(b""))

    def test_wrong_part_refused(self):
        a = F.make_image(F.KIND_CODE, "4e", F.PLANE0_BASE, self.code())
        with self.assertRaises(F.FwImageError) as cm:
            F.validate_image(a, "2e")
        self.assertEqual(cm.exception.kind, "BadTarget")

    def test_bad_vectors_refused(self):
        a = F.make_image(F.KIND_CODE, "2e", F.PLANE0_BASE, self.code(sp=0x20008000))
        with self.assertRaises(F.FwImageError) as cm:
            F.validate_image(a, "2e")
        self.assertEqual(cm.exception.kind, "BadVector")

    def test_duplicate_kind_refused(self):
        a = F.make_image(F.KIND_CODE, "2e", F.PLANE0_BASE, self.code())
        with self.assertRaises(F.FwImageError):
            F.validate_file([a, a], "2e")


class FromElf(unittest.TestCase):
    def test_two_planes(self):
        code = vtable() + b"\x11" * 0x300
        elf = make_elf([
            (0x20000000, b"\xee" * 64),  # RAM load (not part of the image)
            (0x80000, code),
            (0x80000 + 0x400, b"\x22" * 100),  # gap before it must be 0xFF
            (0x100000, b"\x33" * 1000),
        ])
        imgs = F.images_from_elf(elf, "4e", 9)
        self.assertEqual([i.header.kind for i in imgs], [F.KIND_CODE, F.KIND_BITSTREAM])
        c = imgs[0].payload
        self.assertEqual(len(c), 0x600)  # 0x464 rounded up to 512
        self.assertEqual(c[:len(code)], code)
        self.assertEqual(c[len(code):0x400], b"\xff" * (0x400 - len(code)))
        self.assertEqual(c[0x400:0x464], b"\x22" * 100)
        self.assertEqual(imgs[1].header.dest, F.PLANE1_BASE)
        self.assertEqual(imgs[1].payload[:1000], b"\x33" * 1000)
        self.assertEqual(imgs[0].header.image_version, 9)

    def test_2e_rejects_plane1(self):
        elf = make_elf([(0x80000, vtable()), (0x100000, b"x" * 8)])
        with self.assertRaises(F.FwImageError) as cm:
            F.images_from_elf(elf, "2e")
        self.assertEqual(cm.exception.kind, "BadDest")

    def test_4e_elf_with_2e_sp_is_fine_but_2e_with_4e_sp_is_not(self):
        F.images_from_elf(make_elf([(0x80000, vtable(sp=0x20004000))]), "4e")
        with self.assertRaises(F.FwImageError) as cm:
            F.images_from_elf(make_elf([(0x80000, vtable(sp=0x20008000))]), "2e")
        self.assertEqual(cm.exception.kind, "BadVector")

    def test_not_an_elf(self):
        for junk in (b"", b"MZ" + bytes(100), b"\x7fELF\x02\x01\x01" + bytes(100)):
            with self.assertRaises(F.FwImageError):
                F.elf_load_segments(junk)

    def test_overlap_and_straddle(self):
        with self.assertRaises(F.FwImageError):
            F.images_from_elf(make_elf([(0x80000, vtable() + bytes(64)), (0x80010, b"z" * 8)]), "2e")
        with self.assertRaises(F.FwImageError):
            F.images_from_elf(make_elf([(0x80000 + 0x1FF00, vtable() + bytes(0x200))]), "2e")

    def test_cli(self):
        with tempfile.TemporaryDirectory() as d:
            elf = os.path.join(d, "x.elf")
            out = os.path.join(d, "x.ogfw")
            with open(elf, "wb") as f:
                f.write(make_elf([(0x80000, vtable() + bytes(0x300))]))
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                self.assertEqual(F.main(["build", "--elf", elf, "--target", "2e", "-o", out]), 0)
                self.assertEqual(F.main(["inspect", out]), 0)
                self.assertEqual(F.main(["inspect", out, "--target", "2e"]), 0)
            self.assertIn("OK for SAM3U2E", buf.getvalue())
            with contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(F.main(["inspect", out, "--target", "4e"]), 1)  # BadTarget


def real(env):
    p = os.environ.get(env)
    return p if p and os.path.exists(p) else None


class RealElfs(unittest.TestCase):
    def check(self, env, target, planes):
        path = real(env)
        if not path:
            self.skipTest(f"{env} not set")
        with open(path, "rb") as f:
            imgs = F.images_from_elf(f.read(), target, 1)
        self.assertEqual(len(imgs), planes)
        # round trip through a file image
        again = list(F.iter_images(b"".join(i.pack() for i in imgs)))
        self.assertEqual(again, imgs)
        F.validate_file(again, target)
        code = imgs[0]
        sp, reset = struct.unpack_from("<2I", code.payload)
        self.assertEqual(code.header.target_cidr, F.PARTS[target].cidr)
        self.assertLessEqual(sp, F.PARTS[target].sram0_end)
        self.assertEqual(reset & 1, 1)
        self.assertLessEqual(code.header.length, F.PLANE_LEN)
        # the other part must refuse it
        other = "4e" if target == "2e" else "2e"
        with self.assertRaises(F.FwImageError) as cm:
            F.validate_file(imgs, other)
        self.assertEqual(cm.exception.kind, "BadTarget")
        return imgs

    def test_real_4e(self):
        imgs = self.check("FWIMAGE_ELF_4E", "4e", 2)
        self.assertEqual(imgs[1].header.dest, F.PLANE1_BASE)

    def test_real_2e(self):
        self.check("FWIMAGE_ELF_2E", "2e", 1)


if __name__ == "__main__":
    unittest.main()
