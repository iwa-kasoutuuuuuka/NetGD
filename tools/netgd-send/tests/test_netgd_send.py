"""Hardware-free tests for netgd-send (design 9.1).  Run: python -m unittest discover -s tests -v"""

import io
import os
import sys
import tempfile
import unittest
import zlib
from contextlib import redirect_stdout, redirect_stderr

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))

from netgd_send import image as imgmod  # noqa: E402
from netgd_send import job as jobmod  # noqa: E402
from netgd_send import manifest as mf  # noqa: E402
from netgd_send.__main__ import main  # noqa: E402

CHUNK = 1024  # tiny "chunk" so boundaries are testable (must be a multiple of 512)
MIB = 1024 * 1024


def pattern(n, seed):
    base = bytes(range(256))
    base = base[seed % 256:] + base[:seed % 256]
    return (base * (n // 256 + 1))[:n]


class Tmp(unittest.TestCase):
    def setUp(self):
        # /pc/ paths must be ASCII without spaces, so stay inside the repository tree
        self._td = tempfile.TemporaryDirectory(dir=HERE, prefix="_work_")
        self.addCleanup(self._td.cleanup)
        self.root = self._td.name

    def path(self, *p):
        return os.path.join(self.root, *p)

    def write(self, rel, data):
        p = self.path(*rel.split("/"))
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, "wb") as f:
            f.write(data)
        return p

    def make_gdi(self, names=("track01.bin", "track02.raw", "track03.bin"), subdir="src", nl="\r\n"):
        """3 tracks: data 2352 (4 sectors), audio 2352 (3 sectors), data 2048 (5 sectors)."""
        specs = [(1, 0, 4, 2352, names[0], 4 * 2352), (2, 150, 0, 2352, names[1], 3 * 2352),
                 (3, 45000, 4, 2048, names[2], 5 * 2048)]
        lines = [str(len(specs))]
        for num, lba, ctrl, sec, nm, size in specs:
            self.write("%s/%s" % (subdir, nm), pattern(size, num))
            q = '"%s"' % nm if " " in nm else nm
            lines.append("%d %d %d %d %s 0" % (num, lba, ctrl, sec, q))
        return self.write("%s/disc.gdi" % subdir, (nl.join(lines) + nl).encode("ascii"))

    def make_cdi(self, version=0x80000004, offset=None, size=4096):
        data = bytearray(pattern(size, 9))
        off = size - 100 if offset is None else offset
        data[-8:] = version.to_bytes(4, "little") + off.to_bytes(4, "little")
        return self.write("cdsrc/game.cdi", bytes(data))


class TestHash(unittest.TestCase):
    def test_crc32_vectors(self):
        self.assertEqual(mf.crc32(b""), 0)
        self.assertEqual(mf.crc32(b"123456789"), 0xCBF43926)
        self.assertEqual(mf.crc32(b"a"), 0xE8B7BE43)
        self.assertEqual(mf.crc32(b"The quick brown fox jumps over the lazy dog"), 0x414FA339)
        self.assertEqual(mf.crc32(b"6789", mf.crc32(b"12345")), 0xCBF43926)  # chainable
        self.assertEqual(mf.crc_hex(0xAB), "000000AB")


class TestChunking(Tmp):
    def hf(self, size):
        p = self.write("f.bin", pattern(size, 3))
        return p, mf.hash_file(p, CHUNK)

    def test_boundaries(self):
        for size, offs in [(0, []), (1000, []), (1024, []), (1025, [1024]), (2048, [1024]),
                           (2049, [1024, 2048]), (3000, [1024, 2048])]:
            p, (sz, crc, cps) = self.hf(size)
            self.assertEqual(sz, size)
            self.assertEqual([o for o, _ in cps], offs, "size %d" % size)
            data = pattern(size, 3)
            self.assertEqual(crc, zlib.crc32(data) & 0xFFFFFFFF)
            for off, c in cps:  # cumulative CRC of the prefix
                self.assertEqual(c, zlib.crc32(data[:off]) & 0xFFFFFFFF)

    def test_bad_chunk(self):
        with self.assertRaises(ValueError):
            mf.hash_file(self.path("x"), 1000)

    def test_sidecar_roundtrip(self):
        txt = mf.render_chunks(CHUNK, [[(1024, 5)], [], [(1024, 6), (2048, 7)]])
        chunk, table = mf.parse_chunks(txt)
        self.assertEqual(chunk, CHUNK)
        self.assertEqual(table, {(1, 1024): 5, (3, 1024): 6, (3, 2048): 7})


class TestParseGdi(unittest.TestCase):
    def test_quoted_names_bom_crlf_tabs(self):
        text = "ï»¿2 \r\n1 0 4 2352 \"Track 01.bin\" 0 \r\n\r\n2\t450\t0\t2352\ttrack02.raw\t0\r\n"
        count, rows, nl = imgmod.parse_gdi(text)
        self.assertEqual(count, 2)
        self.assertEqual(rows[0][4], "Track 01.bin")
        self.assertEqual(rows[1][4], "track02.raw")
        self.assertEqual(nl, "\r\n")


class TestNames(unittest.TestCase):
    def test_is_dos83(self):
        for ok in ("track01.bin", "DISC.GDI", "A", "a.b", "12345678.123", "T_1-2.raw"):
            self.assertTrue(imgmod.is_dos83(ok), ok)
        for bad in ("", "123456789.bin", "a.1234", "a b.bin", "a.b.c", ".bin", "trk.", "tr\u00e4ck.bin", "a+b.bin", "a,b.bin"):
            self.assertFalse(imgmod.is_dos83(bad), bad)

    def test_keeps_valid_and_renames_long(self):
        items = [("a", "Track 01.bin", "track01.bin"), ("b", "track02.raw", "track02.raw"),
                 ("c", "very_long_name_here.bin", "track03.bin")]
        out = imgmod.assign_83_names(items, reserved=("disc.gdi",))
        self.assertEqual(out, {"a": "track01.bin", "b": "track02.raw", "c": "track03.bin"})

    def test_collisions(self):
        # an original called TRACK01.BIN is kept; the long-named file that wanted track01.bin must not steal it
        items = [("a", "Game (Track 1).bin", "track01.bin"), ("b", "TRACK01.BIN", "track02.bin"),
                 ("c", "Game (Track 3).bin", "track03.bin"), ("d", "disc.gdi", "track04.gdi")]
        out = imgmod.assign_83_names(items, reserved=("DISC.GDI",))
        self.assertEqual(out["b"], "TRACK01.BIN")
        names = [v.lower() for v in out.values()]
        self.assertEqual(len(set(names)), 4)
        self.assertNotIn("track01.bin", [out["a"].lower()])
        self.assertNotIn("disc.gdi", names)
        self.assertTrue(all(imgmod.is_dos83(v) for v in out.values()))
        self.assertTrue(out["a"].endswith(".bin") and out["d"].endswith(".gdi"))

    def test_deterministic(self):
        items = [("a", "x" * 20 + ".bin", "track01.bin")] * 1 + [("b", "y" * 20 + ".bin", "track01.bin")]
        self.assertEqual(imgmod.assign_83_names(items), imgmod.assign_83_names(items))

    def test_slot_folder(self):
        self.assertEqual(imgmod.slot_folder(1), "01")
        self.assertEqual(imgmod.slot_folder(99), "99")
        self.assertEqual(imgmod.slot_folder(100), "100")
        self.assertEqual(imgmod.slot_folder(1234), "1234")
        for bad in (0, -1, 10000):
            with self.assertRaises(imgmod.ImageError):
                imgmod.slot_folder(bad)

    def test_folder_name_rules(self):
        self.assertEqual(imgmod.folder_name(2, None, "x.gdi"), "02")
        self.assertEqual(imgmod.folder_name(None, "Crazy Taxi (US)", "x.gdi"), "Crazy Taxi (US)")
        self.assertEqual(imgmod.folder_name(None, "GAMES/RPG/FF", "x.gdi"), "GAMES/RPG/FF")
        self.assertEqual(imgmod.folder_name(None, None, "C:/a/Crazy Taxi.gdi"), "Crazy Taxi")
        for bad in ("\u30b2\u30fc\u30e0", "caf\u00e9", "a:b", "a*b", " lead", "trail ", "..", "a/b/c/d", "x" * 128, "a\tb"):
            with self.assertRaises((imgmod.ImageError, mf.ManifestError), msg=repr(bad)):
                imgmod.folder_name(None, bad, "x.gdi")
        with self.assertRaises(imgmod.ImageError):  # non-ASCII stem and no --name
            imgmod.folder_name(None, None, "/x/\u30b2\u30fc\u30e0.gdi")
        with self.assertRaises(imgmod.ImageError):
            imgmod.folder_name(1, "x", "x.gdi")


class TestImages(Tmp):
    def test_gdi_ok(self):
        g = imgmod.check_gdi(self.make_gdi())
        self.assertEqual([t.size for t in g.tracks], [4 * 2352, 3 * 2352, 5 * 2048])
        self.assertEqual(g.newline, "\r\n")

    def test_gdi_two_tracks(self):
        p = self.write("t/disc.gdi", b"2\n1 0 4 2048 a.bin 0\n2 600 4 2352 b.bin 0\n")
        self.write("t/a.bin", b"\0" * 2048 * 2)
        self.write("t/b.bin", b"\0" * 2352)
        self.assertEqual(len(imgmod.check_gdi(p).tracks), 2)

    def test_gdi_errors(self):
        def case(text, files, msg):
            d = "c%d" % case.n
            case.n += 1
            for n, size in files.items():
                self.write("%s/%s" % (d, n), b"\0" * size)
            p = self.write("%s/disc.gdi" % d, text.encode())
            with self.assertRaises(imgmod.ImageError, msg=msg):
                imgmod.check_gdi(p)
        case.n = 0
        case("1\n1 0 4 2352 a.bin 0\n", {"a.bin": 2352 * 2 + 1}, "size not multiple")
        case("1\n1 0 4 2352 a.bin 0\n", {}, "missing file")
        case("2\n1 0 4 2352 a.bin 0\n", {"a.bin": 2352}, "count mismatch")
        case("1\n1 0 4 2000 a.bin 0\n", {"a.bin": 2000}, "bad sector size")
        case("1\n1 0 0 2048 a.bin 0\n", {"a.bin": 2048}, "audio with 2048")
        case("1\n1 0 1 2352 a.bin 0\n", {"a.bin": 2352}, "bad type")
        case("1\n1 0 4 2352 a.bin 16\n", {"a.bin": 2352}, "offset")
        case("2\n1 100 4 2352 a.bin 0\n2 100 4 2352 b.bin 0\n", {"a.bin": 2352, "b.bin": 2352}, "lba order")
        case("2\n1 0 4 2352 a.bin 0\n3 100 4 2352 b.bin 0\n", {"a.bin": 2352, "b.bin": 2352}, "numbering")
        case("1\n1 0 4 2352 a.bin 0\n", {"a.bin": 0}, "empty file")
        case("x\n", {}, "bad count")
        case("1\ngarbage\n", {}, "bad line")

    def test_gdi_rewrite(self):
        p = self.make_gdi(names=("Game (Track 1).bin", "Game (Track 2).raw", "Game (Track 3).bin"))
        g = imgmod.check_gdi(p)
        names, text = imgmod.plan_gdi_names(g)
        self.assertEqual(names, {1: "track01.bin", 2: "track02.raw", 3: "track03.bin"})
        self.assertEqual(text, "3\r\n1 0 4 2352 track01.bin 0\r\n2 150 0 2352 track02.raw 0\r\n3 45000 4 2048 track03.bin 0\r\n")

    def test_cdi(self):
        c = imgmod.check_cdi(self.make_cdi())
        self.assertEqual(c.version, "2")
        imgmod.check_cdi(self.make_cdi(version=0x80000006, offset=300))
        for kw in (dict(version=0x12345678), dict(offset=0), dict(offset=4096), dict(size=8, offset=1)):
            with self.assertRaises(imgmod.ImageError, msg=str(kw)):
                imgmod.check_cdi(self.make_cdi(**kw))
        with self.assertRaises(imgmod.ImageError):
            imgmod.detect_type("x.iso")


class TestManifest(Tmp):
    def test_render_exact_and_roundtrip(self):
        m = mf.Manifest("Crazy Taxi (US)", "gdi",
                        [mf.FileEntry(1234, 0x89ABCDEF, "disc.gdi"), mf.FileEntry(2352, 0x1, "track01.bin")], "disc.gdi")
        txt = m.render()
        self.assertEqual(txt, "NETGD-MANIFEST 1\nimage Crazy Taxi (US)\ntype gdi\nfile 1234 89ABCDEF disc.gdi\n"
                              "file 2352 00000001 track01.bin\ndescriptor disc.gdi\nend\n")
        m2 = mf.parse_manifest(txt)
        self.assertEqual(m2.render(), txt)
        self.assertEqual(m2.files, m.files)
        self.assertEqual(m.crc(), zlib.crc32(txt.encode()) & 0xFFFFFFFF)

    def test_parse_rejects(self):
        good = "NETGD-MANIFEST 1\nimage 01\ntype cdi\nfile 10 00000001 disc.cdi\ndescriptor disc.cdi\nend\n"
        mf.parse_manifest(good)
        bads = [good.replace("MANIFEST 1", "MANIFEST 2"), good.replace("end\n", ""), good + "x\n",
                good.replace("disc.cdi\nend", "other.cdi\nend"), good.replace("type cdi", "type iso"),
                good.replace("00000001", "1"), good.replace("image 01", "image a:b"),
                good.replace("file 10", "file 4294967296")]
        for b in bads:
            with self.assertRaises(mf.ManifestError, msg=b):
                mf.parse_manifest(b)

    def test_duplicate_names(self):
        m = mf.Manifest("01", "gdi", [mf.FileEntry(1, 0, "disc.gdi"), mf.FileEntry(1, 0, "DISC.GDI")], "disc.gdi")
        with self.assertRaises(mf.ManifestError):
            m.render()

    def test_pc_path(self):
        p = jobmod.to_pc_path(self.path("a", "b.bin"))
        self.assertTrue(p.startswith("/pc/") and ":" not in p and "\\" not in p, p)
        bads = ["E:\\my games\\a.bin", "E:\\\u30b2\\a.bin"]
        # UNC paths only exist on Windows (dc-tool-ip's /pc/ is a Windows drive);
        # on POSIX "\\srv\share" is just an odd relative file name.
        if os.name == "nt":
            bads.append("\\\\srv\\share\\a.bin")
        for bad in bads:
            with self.assertRaises(jobmod.JobError, msg=bad):
                jobmod.to_pc_path(bad)


class TestPrepare(Tmp):
    def prep(self, image, job="job", **kw):
        return jobmod.prepare(image, self.path(job), chunk_mib=1, **kw)

    def test_gdi_prepare_layout(self):
        gdi = self.make_gdi(names=("Game (Track 1).bin", "Game (Track 2).raw", "Game (Track 3).bin"))
        r = self.prep(gdi, name="CRAZYTAX")
        self.assertEqual(r.number, "0001")
        self.assertEqual([f.name for f in r.manifest.files], ["disc.gdi", "track01.bin", "track02.raw", "track03.bin"])
        self.assertEqual(r.manifest.descriptor, "disc.gdi")
        e = self.path("job", "0001")
        for n in ("manifest.ngm", "manifest.ngc", "src.txt", "disc.gdi", "track01.bin", "track02.raw", "track03.bin"):
            self.assertTrue(os.path.isfile(os.path.join(e, n)), n)
        with open(os.path.join(e, "src.txt")) as f:
            srcs = f.read().split("\n")[:-1]
        self.assertEqual(len(srcs), 4)
        self.assertTrue(all(s.startswith("/pc/") and " " not in s for s in srcs))
        self.assertTrue(srcs[1].endswith("/job/0001/track01.bin"))
        # file CRCs match the placed data
        t1 = pattern(4 * 2352, 1)
        self.assertEqual(r.manifest.files[1].crc, zlib.crc32(t1) & 0xFFFFFFFF)
        self.assertEqual(r.manifest.files[1].size, len(t1))
        with open(os.path.join(e, "disc.gdi"), "rb") as f:
            self.assertIn(b"track02.raw", f.read())
        with open(self.path("job", "job.ngj")) as f:
            self.assertEqual(f.read(), "NETGD-JOB 1\nstatfs\nput 0001 new\nfinish default\n")
        # the manifest the DC will read parses back to the same thing
        with open(os.path.join(e, "manifest.ngm"), "rb") as f:
            self.assertEqual(mf.parse_manifest(f.read().decode()).render(), r.manifest.render())

    def test_cdi_prepare(self):
        r = self.prep(self.make_cdi(), slot=7, mode="resume", mount=True)
        self.assertEqual(r.manifest.image, "07")
        self.assertEqual([f.name for f in r.manifest.files], ["disc.cdi"])
        with open(self.path("job", "job.ngj")) as f:
            self.assertEqual(f.read(), "NETGD-JOB 1\nstatfs\nput 0001 resume\nfinish mount 07\n")

    def test_determinism(self):
        gdi = self.make_gdi()
        a = self.prep(gdi, job="j1", slot=3)
        b = self.prep(gdi, job="j2", slot=3)
        self.assertEqual(a.manifest.render(), b.manifest.render())
        self.assertEqual(a.manifest.crc(), b.manifest.crc())
        with open(self.path("j1", "0001", "manifest.ngc"), "rb") as f1, open(self.path("j2", "0001", "manifest.ngc"), "rb") as f2:
            self.assertEqual(f1.read(), f2.read())

    def test_second_image_and_reprepare(self):
        self.prep(self.make_gdi(), slot=2)
        r2 = self.prep(self.make_cdi(), slot=3)
        self.assertEqual(r2.number, "0002")
        r3 = self.prep(self.make_cdi(), slot=3, mode="overwrite")  # same folder: entry is reused
        self.assertEqual(r3.number, "0002")
        job = jobmod.read_job(self.path("job"))
        self.assertEqual(job.puts, [["0001", "new"], ["0002", "overwrite"]])

    def test_reprepare_clears_stale_result(self):
        self.prep(self.make_cdi(), slot=2)
        self.write("job/result.ngr", b"NGD:END\n")
        self.write("job/cancel.req", b"")
        self.prep(self.make_cdi(), slot=2, mode="resume")
        self.assertFalse(os.path.exists(self.path("job", "result.ngr")))
        self.assertTrue(os.path.exists(self.path("job", "result.prev.ngr")))
        self.assertFalse(os.path.exists(self.path("job", "cancel.req")))

    def test_rejects_bad_input_without_writing(self):
        gdi = self.make_gdi()
        with self.assertRaises((imgmod.ImageError, mf.ManifestError)):
            self.prep(gdi, name="\u30b2\u30fc\u30e0")
        self.assertFalse(os.path.exists(self.path("job")))
        with self.assertRaises(jobmod.JobError):
            jobmod.prepare(gdi, self.path("job"), chunk_mib=0)

    def test_chunk_warning(self):
        r = jobmod.prepare(self.make_cdi(), self.path("job"), slot=1, chunk_mib=2)
        self.assertTrue(r.warnings)
        r = jobmod.prepare(self.make_cdi(), self.path("job8"), slot=1, chunk_mib=8)
        self.assertFalse(r.warnings)


NGD = "NGD:"


class TestCollect(Tmp):
    def setUp(self):
        super().setUp()
        # 3 MiB + 1 byte track so chunk = 1 MiB gives resume points at 1 MiB and 2 MiB, 3 MiB
        self.size = 3 * MIB + 1
        self.img = self.write("big/game.cdi", self.cdi_bytes(self.size))
        self.r = jobmod.prepare(self.img, self.path("job"), slot=2, chunk_mib=1)
        self.m = self.r.manifest
        self.total = self.m.total_bytes()
        with open(self.path("job", "0001", "manifest.ngc"), "rb") as f:
            _c, self.table = mf.parse_chunks(f.read().decode())

    @staticmethod
    def cdi_bytes(size):
        d = bytearray(pattern(size, 5))
        d[-8:] = (0x80000004).to_bytes(4, "little") + (size - 50).to_bytes(4, "little")
        return bytes(d)

    def put_result(self, text):
        self.write("job/result.ngr", text.encode())

    def collect(self):
        buf = io.StringIO()
        with redirect_stdout(buf):
            code = main(["collect", "--job", self.path("job")])
        return code, buf.getvalue()

    GOOD_HEAD = "NGD:HELLO app=1.0.0 fw=OGDM/3 caps=7 free=100 total=200\n"

    def success_text(self, with_files=True):
        t = self.GOOD_HEAD
        t += "NGD:PUT 0001 file=1/1 bytes=%d/%d rate=900000\n" % (self.total, self.total)
        if with_files:
            f = self.m.files[0]
            t += "NGD:FILE 0001 file=1 size=%d crc=%08X\n" % (f.size, f.crc)
        t += "NGD:DONE 0001 ok\nNGD:FINISH ok mounted=\"02\"\nNGD:END ok=1\n"
        return t

    def test_success(self):
        self.put_result(self.success_text())
        code, out = self.collect()
        self.assertEqual(code, 0, out)
        self.assertIn("put 0001 OK", out)
        self.assertIn("RESULT: success", out)

    def test_success_without_file_lines(self):
        self.put_result(self.success_text(with_files=False))
        self.assertEqual(self.collect()[0], 0)

    def test_no_result(self):
        code, out = self.collect()
        self.assertEqual(code, 2)
        self.assertIn("no result.ngr", out)

    def test_partial_then_resume(self):
        # app died after ~2.5 MiB: next resume point is 2 MiB
        self.put_result(self.GOOD_HEAD + "NGD:PUT 0001 file=1/1 bytes=%d/%d rate=1\n" % (2 * MIB + MIB // 2, self.total))
        code, out = self.collect()
        self.assertEqual(code, 2, out)
        self.assertIn("INCOMPLETE", out)
        self.assertIn("next resume point: disc.cdi @ %d" % (2 * MIB), out)
        # status shows the same
        buf = io.StringIO()
        with redirect_stdout(buf):
            self.assertEqual(main(["status", "--job", self.path("job")]), 0)
        self.assertIn("resume point: disc.cdi @ %d" % (2 * MIB), buf.getvalue())
        # second run resumes from 2 MiB with the right checkpoint CRC and completes
        crc = self.table[(1, 2 * MIB)]
        self.write("job/result.ngr", (self.GOOD_HEAD + "NGD:RESUME 0001 file=1 offset=%d crc=%08X\n" % (2 * MIB, crc)
                                      + self.success_text()[len(self.GOOD_HEAD):]).encode())
        code, out = self.collect()
        self.assertEqual(code, 0, out)
        self.assertIn("resumed from disc.cdi @ %d" % (2 * MIB), out)

    def test_resume_with_wrong_crc_or_offset(self):
        for line in ("NGD:RESUME 0001 file=1 offset=%d crc=DEADBEEF\n" % MIB,
                     "NGD:RESUME 0001 file=1 offset=%d crc=%08X\n" % (MIB + 512, 0),
                     "NGD:RESUME 0001 file=5 offset=0\n"):
            self.put_result(self.GOOD_HEAD + line + self.success_text()[len(self.GOOD_HEAD):])
            code, out = self.collect()
            self.assertEqual(code, 1, out)
            self.assertIn("PROBLEM", out)

    def test_failed_error_line(self):
        self.put_result(self.GOOD_HEAD + "NGD:PUT 0001 file=1/1 bytes=%d/%d rate=1\n" % (MIB + 5, self.total)
                        + "NGD:ERROR 0001 code=ENOSPC need=734003200 free=10485760\nNGD:END ok=0\n")
        code, out = self.collect()
        self.assertEqual(code, 1)
        self.assertIn("ENOSPC", out)
        self.assertIn("need=734003200", out)
        self.assertIn("next resume point: disc.cdi @ %d" % MIB, out)

    def test_failed_chunk_reported(self):
        self.put_result(self.GOOD_HEAD + "NGD:ERROR 0001 code=EBUS file=1 offset=%d\nNGD:END ok=0\n" % (MIB + 100))
        code, out = self.collect()
        self.assertEqual(code, 1)
        self.assertIn("EBUS", out)
        self.assertIn("next resume point: disc.cdi @ %d" % MIB, out)

    def test_corrupted_crc_from_app(self):
        f = self.m.files[0]
        t = self.success_text().replace("crc=%08X" % f.crc, "crc=%08X" % (f.crc ^ 1))
        self.put_result(t)
        code, out = self.collect()
        self.assertEqual(code, 1)
        self.assertIn("CRC32", out)
        self.assertIn("FAILED", out)

    def test_wrong_size_or_totals(self):
        self.put_result(self.success_text().replace("bytes=%d/%d" % (self.total, self.total), "bytes=5/6"))
        self.assertEqual(self.collect()[0], 1)
        f = self.m.files[0]
        self.put_result(self.success_text().replace("size=%d" % f.size, "size=%d" % (f.size - 1)))
        self.assertEqual(self.collect()[0], 1)

    def test_torn_last_line_and_garbage(self):
        # last line cut in the middle (4 KiB writes): ignored, DONE missing -> incomplete
        t = self.GOOD_HEAD + "NGD:PUT 0001 file=1/1 bytes=1048576/%d rate=1\nNGD:DONE 0001 o" % self.total
        self.put_result(t)
        code, out = self.collect()
        self.assertEqual(code, 2, out)
        self.assertIn("torn", out)
        # malformed NGD line
        self.put_result(self.success_text().replace("NGD:HELLO", "NGD:PUT 0001 file=oops\nNGD:HELLO"))
        code, out = self.collect()
        self.assertEqual(code, 1)
        self.assertIn("malformed", out)

    def test_finish_failure(self):
        self.put_result(self.success_text().replace('NGD:FINISH ok mounted="02"', "NGD:FINISH error code=EFW"))
        code, out = self.collect()
        self.assertEqual(code, 1)
        self.assertIn("finish FAILED", out)

    def test_finish_missing_is_incomplete(self):
        t = "".join(l for l in self.success_text().splitlines(True) if "FINISH" not in l and "END" not in l)
        self.put_result(t)
        self.assertEqual(self.collect()[0], 2)

    def test_two_puts_one_failed(self):
        img2 = self.write("big2/g2.cdi", self.cdi_bytes(4096))
        jobmod.prepare(img2, self.path("job"), slot=3, chunk_mib=1)  # entry 0002 (also drops result)
        m2 = parse = jobmod._load_entry(self.path("job"), "0002")[0]
        text = (self.success_text().replace("NGD:FINISH", "NGD:XX").replace("NGD:END ok=1\n", "")
                + "NGD:ERROR 0002 code=ECRC\nNGD:FINISH error code=ESTATE\nNGD:END ok=0\n")
        self.put_result(text)
        code, out = self.collect()
        self.assertEqual(code, 1)
        self.assertIn("put 0001 OK", out)
        self.assertIn("put 0002 FAILED", out)
        self.assertEqual(m2.image, "03")

    def test_collect_without_job(self):
        buf = io.StringIO()
        with redirect_stderr(buf):
            code = main(["collect", "--job", self.path("nothing")])
        self.assertEqual(code, 1)
        self.assertIn("job.ngj", buf.getvalue())


class TestCli(Tmp):
    def test_prepare_cli_and_status(self):
        gdi = self.make_gdi()
        buf = io.StringIO()
        with redirect_stdout(buf):
            code = main(["prepare", gdi, "--job", self.path("job"), "--slot", "5", "--chunk-mib", "8"])
        self.assertEqual(code, 0)
        self.assertIn("SD folder  : 05", buf.getvalue())
        self.assertIn("disc.gdi", buf.getvalue())
        buf = io.StringIO()
        with redirect_stdout(buf):
            self.assertEqual(main(["status", "--job", self.path("job")]), 0)
        self.assertIn("PENDING", buf.getvalue())

    def test_prepare_cli_error_exit(self):
        err = io.StringIO()
        with redirect_stderr(err):
            code = main(["prepare", self.path("nope.gdi"), "--job", self.path("job")])
        self.assertEqual(code, 1)
        self.assertIn("error:", err.getvalue())


if __name__ == "__main__":
    unittest.main()
