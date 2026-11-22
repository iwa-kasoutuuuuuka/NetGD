import os
import struct
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
import check


def make_elf(secs, machine=40):
    """secs: list of (name, type, flags, addr, size). Headers only."""
    names = b"\0"
    offs = []
    for s in secs:
        offs.append(len(names))
        names += s[0].encode() + b"\0"
    strtab_name = len(names)
    names += b".shstrtab\0"
    shoff = 0x34 + len(names)
    n = len(secs) + 2
    hdr = bytearray(0x34)
    hdr[0:4] = b"\x7fELF"
    hdr[4] = 1
    hdr[5] = 1
    struct.pack_into("<H", hdr, 0x12, machine)
    struct.pack_into("<I", hdr, 0x20, shoff)
    struct.pack_into("<HHH", hdr, 0x2E, 40, n, n - 1)
    sh = bytearray(40)
    for (_name, typ, flags, addr, size), o in zip(secs, offs):
        sh += struct.pack("<IIIIIIIIII", o, typ, flags, addr, 0, size, 0, 0, 0, 0)
    sh += struct.pack("<IIIIIIIIII", strtab_name, 3, 0, 0, 0x34, len(names), 0, 0, 0, 0)
    return bytes(hdr) + names + bytes(sh)


# Hand-computed layout. Code ends 100,000 B above FLASH origin 0x82000, so
# .image_pad starts there; used = 100,000 + 4 (CRC) = 100,004, rounded up to
# 196 * 512 = 100,352, so pad = 348 B and the CRC sits at +348.
BASE = 0x82000
PAD_ADDR = BASE + 100000


def secs(pad_addr=PAD_ADDR, pad_size=348):
    return [
        (".text", 1, 6, BASE, 99000),
        (".data", 1, 3, 0x20000000, 160),
        (".bss", 8, 3, 0x20000100, 5000),
        (".image_pad", 1, 2, pad_addr, pad_size),
        (".image_crc", 1, 2, pad_addr + pad_size, 4),
    ]


def write(d, s, **kw):
    p = os.path.join(d, "x.elf")
    with open(p, "wb") as f:
        f.write(make_elf(s, **kw))
    return p


class T(unittest.TestCase):
    def test_resident_usage(self):
        with tempfile.TemporaryDirectory() as d:
            used, image = check.resident_usage(write(d, secs()))
        self.assertEqual(used, 100004)
        self.assertEqual(image, 100352)
        line, ok = check.report_resident("t", used, image)
        self.assertEqual(
            line,
            "t: flash used 100004 / limit 122880, margin 22876 (padded image 100352)")
        self.assertTrue(ok)

    def test_gate_boundary(self):
        with tempfile.TemporaryDirectory() as d:
            self.assertEqual(check.main([write(d, secs()), "--gate-2e"]), 0)
            # used = 122,880 exactly (pad starts 122,876 above origin, size 0)
            self.assertEqual(
                check.main([write(d, secs(BASE + 122876, 0)), "--gate-2e"]), 0)
            # one byte more
            self.assertEqual(
                check.main([write(d, secs(BASE + 122877, 0)), "--gate-2e"]), 1)

    def test_berkeley(self):
        with tempfile.TemporaryDirectory() as d:
            p = write(d, secs())
            # text = .text + pad + crc, data = .data, bss = .bss
            self.assertEqual(check.berkeley_sizes(p), (99000 + 348 + 4, 160, 5000))
            self.assertEqual(check.main([p, "--limit", "99512"]), 0)
            self.assertEqual(check.main([p, "--limit", "99511"]), 1)

    def test_errors(self):
        with tempfile.TemporaryDirectory() as d:
            self.assertEqual(check.main([write(d, secs(), machine=3), "--gate-2e"]), 2)
            self.assertEqual(check.main([write(d, secs()[:3]), "--gate-2e"]), 2)
            p = write(d, secs())
            with open(p, "r+b") as f:
                f.truncate(os.path.getsize(p) - 50)
            self.assertEqual(check.main([p]), 2)
            self.assertEqual(check.main([os.path.join(d, "none")]), 2)


if __name__ == "__main__":
    unittest.main()
