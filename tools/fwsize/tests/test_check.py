import os, struct, sys, tempfile, unittest
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
import check


def make_elf(secs):
    """secs: list of (type, flags, size). Minimal ELF32 LE, headers only."""
    shoff = 0x34
    hdr = bytearray(0x34)
    hdr[0:4] = b"\x7fELF"; hdr[4] = 1; hdr[5] = 1
    struct.pack_into("<I", hdr, 0x20, shoff)
    struct.pack_into("<HH", hdr, 0x2E, 40, len(secs) + 1)
    sh = bytearray(40)  # null section
    for typ, flags, size in secs:
        sh += struct.pack("<IIIIIIIIII", 0, typ, flags, 0, 0, size, 0, 0, 0, 0)
    return bytes(hdr) + bytes(sh)


class T(unittest.TestCase):
    def run_check(self, secs, *args):
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "x.elf")
            open(p, "wb").write(make_elf(secs))
            return check.main([p, *args]), check.berkeley_sizes(p)

    def test_classify(self):
        rc, sz = self.run_check([(1, 2, 100), (1, 3, 10), (8, 3, 50), (1, 0, 999)])
        self.assertEqual(sz, (100, 10, 50))
        self.assertEqual(rc, 0)

    def test_limit(self):
        self.assertEqual(self.run_check([(1, 2, 100)], "--limit", "100")[0], 0)
        self.assertEqual(self.run_check([(1, 2, 101)], "--limit", "100")[0], 1)

    def test_report(self):
        line, ok = check.report("a", 90, 10, 0, 120)
        self.assertEqual(line, "a: text+data 100 / limit 120, margin 20")
        self.assertTrue(ok)


if __name__ == "__main__":
    unittest.main()
