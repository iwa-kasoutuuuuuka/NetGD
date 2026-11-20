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


def make_elf(segments, sections=()):
    """Minimal ELF32-LE/ARM with one PT_LOAD per (paddr, bytes), and optional
    section headers: `sections` is [(name, addr, size)], each lying inside one
    of the segments (the section's file bytes are the segment's)."""
    n = len(segments)
    phoff = 52
    data_off = phoff + 32 * n
    ph = b""
    body = b""
    seg_off = []
    for paddr, data in segments:
        seg_off.append(data_off + len(body))
        ph += struct.pack("<8I", F.PT_LOAD, data_off + len(body), paddr, paddr, len(data), len(data), 5, 4)
        body += data
    shoff = shnum = shstrndx = 0
    tail = b""
    if sections:
        names = b"\0"
        shdrs = [bytes(40)]
        for name, addr, size in sections:
            off = next(so + (addr - pa) for (pa, d), so in zip(segments, seg_off) if pa <= addr < pa + len(d))
            shdrs.append(struct.pack("<10I", len(names), 1, 2, addr, off, size, 0, 0, 4, 0))
            names += name.encode() + b"\0"
        names += b".shstrtab\0"
        str_off = data_off + len(body)
        shdrs.append(struct.pack("<10I", len(names) - 10, 3, 0, 0, str_off, len(names), 0, 0, 1, 0))
        tail = names
        shoff = str_off + len(names)
        shnum = len(shdrs)
        shstrndx = shnum - 1
        tail += b"".join(shdrs)
    ehdr = b"\x7fELF" + bytes([1, 1, 1, 0]) + bytes(8)
    ehdr += struct.pack("<HHIIIIIHHHHHH", 2, F.EM_ARM, 1, 0, phoff, shoff, 0x05000000, 52, 32, n, 40, shnum, shstrndx)
    return ehdr + ph + body + tail


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
                F.check_header(F.parse_header(blk, (2,)), part)
                got = "OK"
            except F.FwImageError as e:
                got = e.kind
            self.assertEqual(got, want, (part, hexs))

    def test_header3(self):
        rows = vector_lines("header3.txt")
        self.assertGreaterEqual(len(rows), 20)
        for part, hexs, want in rows:
            blk = bytes.fromhex(hexs)
            try:
                F.check_header(F.parse_header(blk, (3,)), part)
                got = "OK"
            except F.FwImageError as e:
                got = e.kind
            self.assertEqual(got, want, (part, hexs))

    def test_descriptor(self):
        rows = vector_lines("desc.txt")
        self.assertGreaterEqual(len(rows), 10)
        for desc, n, want in rows:
            try:
                F.check_descriptor(bytes.fromhex(desc), int(n, 16))
                got = "OK"
            except F.FwImageError as e:
                got = e.kind
            self.assertEqual(got, want, (desc, n))

    def test_trailer(self):
        rows = vector_lines("trailer.txt")
        self.assertGreaterEqual(len(rows), 8)
        for data, want in rows:
            try:
                F.check_trailer(b"" if data == "-" else bytes.fromhex(data))
                got = "OK"
            except F.FwImageError as e:
                got = e.kind
            self.assertEqual(got, want, data)

    def test_image3(self):
        rows = vector_lines("image3.txt")
        self.assertGreaterEqual(len(rows), 10)
        for part, hexs, want in rows:
            try:
                F.validate_file(list(F.iter_images(bytes.fromhex(hexs), (3,))), part)
                got = "OK"
            except F.FwImageError as e:
                got = e.kind
            self.assertEqual(got, want, (part, hexs[:80]))


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


AB = F.APP_BASE


def app_blob(size, sp=0x20004000, base=AB, desc=True):
    """An app as linked: vectors, an unsealed NGAP descriptor at +0x100."""
    b = bytearray(b"\xa5" * size)
    b[:16] = struct.pack("<4I", sp, base + 0x101, base + 0x103, base + 0x105)
    if desc:
        b[0x100:0x120] = F.pack_descriptor(0)
    return bytes(b)


class Format3(unittest.TestCase):
    def test_build_seals_and_roundtrips(self):
        elf = make_elf([(AB, app_blob(0x1234)), (0x100000, b"\x33" * 700)])
        imgs = F.images_from_elf(elf, "4e", 5, fmt=3)
        code, bits = imgs
        self.assertEqual((code.header.format, code.header.layout, code.header.dest), (3, 0x2000, AB))
        self.assertEqual(code.header.length, 0x1400)  # 0x1234 + 4 rounded up to 512
        self.assertEqual(code.payload[0x1234:0x13FC], b"\xff" * (0x13FC - 0x1234))
        d = struct.unpack_from("<8I", code.payload, 0x100)
        self.assertEqual(d[0], 0x5041474E)  # "NGAP" read as a little-endian word
        self.assertEqual(d[2], 0x1400)
        self.assertEqual(d[7], F.crc32(code.payload[0x100:0x11C]))
        self.assertEqual(struct.unpack_from("<I", code.payload, 0x13FC)[0], F.crc32(code.payload[:0x13FC]))
        self.assertEqual(bits.header.format, 3)
        self.assertEqual(bits.header.length, 1024)
        data = b"".join(i.pack() for i in imgs)
        again = list(F.iter_images(data, (3,)))
        self.assertEqual(again, imgs)
        F.validate_file(again, "4e")
        with self.assertRaises(F.FwImageError) as cm:  # a format-2 reader refuses it
            list(F.iter_images(data, (2,)))
        self.assertEqual(cm.exception.kind, "BadHeader")

    def test_default_is_format_2(self):
        elf = make_elf([(0x80000, vtable() + bytes(0x300))])
        self.assertEqual(F.images_from_elf(elf, "2e")[0].header.format, 2)

    def test_old_link_address_refused(self):
        elf = make_elf([(0x80000, app_blob(0x400, base=0x80000))])
        with self.assertRaises(F.FwImageError) as cm:
            F.images_from_elf(elf, "2e", fmt=3)
        self.assertEqual(cm.exception.kind, "BadDest")
        self.assertIn("APP_BASE", str(cm.exception))
        # content straddling the boundary is refused too
        elf = make_elf([(AB - 0x100, app_blob(0x800))])
        with self.assertRaises(F.FwImageError) as cm:
            F.images_from_elf(elf, "2e", fmt=3)
        self.assertEqual(cm.exception.kind, "BadDest")

    def test_missing_descriptor_refused(self):
        elf = make_elf([(AB, app_blob(0x400, desc=False))])
        with self.assertRaises(F.FwImageError) as cm:
            F.images_from_elf(elf, "2e", fmt=3)
        self.assertEqual(cm.exception.kind, "BadDescriptor")

    def test_length_limit(self):
        top = 0x20000 - 0x2000
        F.images_from_elf(make_elf([(AB, app_blob(top - 4))]), "2e", fmt=3)  # fills the area exactly
        with self.assertRaises(F.FwImageError) as cm:
            F.images_from_elf(make_elf([(AB, app_blob(top - 3))]), "2e", fmt=3)  # trailer no longer fits
        self.assertEqual(cm.exception.kind, "BadLength")
        with self.assertRaises(F.FwImageError) as cm:
            F.images_from_elf(make_elf([(AB, app_blob(top + 0x200))]), "2e", fmt=3)  # runs past the flash
        self.assertEqual(cm.exception.kind, "BadElf")

    def test_tampering_is_caught(self):
        img = F.images_from_elf(make_elf([(AB, app_blob(0x800))]), "2e", fmt=3)[0]
        for off, kind in ((0x300, "BadTrailer"), (0x11C, "BadDescriptor"), (0x108, "BadDescriptor")):
            p = bytearray(img.payload)
            p[off] ^= 1
            bad = F.Image(F.Header(0, img.header.target_cidr, AB, len(p), F.crc32(bytes(p)), 0, 3, 0x2000), bytes(p))
            with self.assertRaises(F.FwImageError) as cm:
                F.validate_image(bad, "2e")
            self.assertEqual(cm.exception.kind, kind, hex(off))

    def test_mixed_formats_refused(self):
        a = F.make_image(F.KIND_CODE, "4e", AB, app_blob(0x400), fmt=3)
        b = F.make_image(F.KIND_BITSTREAM, "4e", F.PLANE1_BASE, b"x" * 512, fmt=2)
        with self.assertRaises(F.FwImageError):
            F.validate_file([a, b], "4e")

    def test_header_roundtrip_and_layout_under_crc(self):
        h = F.Header(F.KIND_CODE, 0x281A0761, AB, 0x400, 0xDEADBEEF, 42, 3, 0x2000)
        blk = h.pack()
        self.assertEqual(struct.unpack_from("<2I", blk, 32), (F.header_crc3(blk[:32], 0x2000), 0x2000))
        self.assertEqual(F.parse_header(blk), h)
        self.assertEqual(blk[40:], bytes(512 - 40))
        bad = bytearray(blk)
        bad[36] ^= 1
        with self.assertRaises(F.FwImageError):
            F.parse_header(bytes(bad))

    # --- seal -------------------------------------------------------------

    def sealable_elf(self, n=0x800):
        app = bytearray(app_blob(n))
        app[n - 4:] = bytes(4)
        return make_elf([(AB, bytes(app)), (0x20000000, b"\xee" * 16)],
                        [(".app_desc", AB + 0x100, 32), (".image_crc", AB + n - 4, 4)])

    def test_seal(self):
        elf = self.sealable_elf()
        self.assertEqual(set(F.elf_sections(elf)), {".app_desc", ".image_crc", ".shstrtab"})
        sealed = F.seal_elf(elf)
        self.assertEqual(len(sealed), len(elf))
        # only the descriptor length/CRC and the trailer changed
        diff = [i for i in range(len(elf)) if elf[i] != sealed[i]]
        self.assertTrue(all(0 <= i - F.elf_sections(elf)[".app_desc"][1] < 32 or
                            0 <= i - F.elf_sections(elf)[".image_crc"][1] < 4 for i in diff))
        # the sealed ELF builds an image whose own checks pass, and that
        # image's payload equals what the ELF holds
        img = F.images_from_elf(sealed, "2e", fmt=3)[0]
        self.assertEqual(img.header.length, 0x800)
        F.validate_image(img, "2e")
        segs = F.elf_load_segments(sealed)
        self.assertEqual(img.payload, [d for a, d in segs if a == AB][0])
        # idempotent
        self.assertEqual(F.seal_elf(sealed), sealed)
        # and `build` of the unsealed ELF gives the same payload as the sealed one
        self.assertEqual(F.images_from_elf(elf, "2e", fmt=3)[0].payload, img.payload)

    def test_seal_refuses(self):
        for elf in (
            make_elf([(AB, app_blob(0x800))]),  # no sections
            make_elf([(AB, app_blob(0x800))], [(".app_desc", AB + 0x100, 32)]),  # no .image_crc
            make_elf([(AB, app_blob(0x800))], [(".app_desc", AB + 0x100, 32), (".image_crc", AB + 0x7FC, 3)]),
            make_elf([(AB, app_blob(0x800))], [(".app_desc", AB + 0x104, 32), (".image_crc", AB + 0x7FC, 4)]),
            make_elf([(AB, app_blob(0x7F0))], [(".app_desc", AB + 0x100, 32), (".image_crc", AB + 0x7EC, 4)]),  # 0x7F0 long
            make_elf([(AB, app_blob(0x800, desc=False))], [(".app_desc", AB + 0x100, 32), (".image_crc", AB + 0x7FC, 4)]),
        ):
            with self.assertRaises(F.FwImageError):
                F.seal_elf(elf)

    def test_cli_format3_and_seal(self):
        with tempfile.TemporaryDirectory() as d:
            elf = os.path.join(d, "x.elf")
            sealed = os.path.join(d, "s.elf")
            out = os.path.join(d, "x.ogfw")
            old = os.path.join(d, "old.elf")
            with open(elf, "wb") as f:
                f.write(self.sealable_elf())
            with open(old, "wb") as f:
                f.write(make_elf([(0x80000, vtable() + bytes(0x300))]))
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                self.assertEqual(F.main(["seal", elf, "-o", sealed]), 0)
                self.assertEqual(F.main(["build", "--elf", sealed, "--target", "2e", "--format", "3", "-o", out]), 0)
                self.assertEqual(F.main(["inspect", out]), 0)
            self.assertIn("fmt=3 layout=0x2000", buf.getvalue())
            self.assertIn("descriptor and trailer", buf.getvalue())
            with contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(F.main(["build", "--elf", old, "--target", "2e", "--format", "3", "-o", out]), 1)
                self.assertEqual(F.main(["seal", old, "-o", sealed]), 1)
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(F.main(["build", "--elf", old, "--target", "2e", "-o", out]), 0)  # default: format 2
                self.assertEqual(F.main(["inspect", out]), 0)


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

    def test_real_elfs_not_yet_format3(self):
        # Until stage S2 relinks the app at APP_BASE the shipped ELFs must be
        # refused for format 3, and say why.
        for env, tgt in (("FWIMAGE_ELF_2E", "2e"), ("FWIMAGE_ELF_4E", "4e")):
            path = real(env)
            if not path:
                continue
            with open(path, "rb") as f:
                elf = f.read()
            try:
                F.images_from_elf(elf, tgt, 1, fmt=3)
            except F.FwImageError as e:
                self.assertIn(e.kind, ("BadDest", "BadDescriptor"), env)
                if e.kind == "BadDest":
                    self.assertIn("APP_BASE", str(e))
            else:
                self.fail(f"{env}: format 3 accepted an ELF linked at 0x80000")


if __name__ == "__main__":
    unittest.main()
