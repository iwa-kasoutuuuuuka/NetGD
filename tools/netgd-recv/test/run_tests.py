"""Hardware-free tests of netgd-recv's core (design 9.1).  Run from anywhere:

    python tools/netgd-recv/test/run_tests.py            # everything (about a minute)
    python tools/netgd-recv/test/run_tests.py -k sweep   # one group

Needs gcc and Python 3.  Builds build/test_host from the same sources as the KOS app
(everything except main.c, dev_kos.c, fwctl.c, plat_kos.c), makes FAT32 images with FatFs'
f_mkfs, drives the app against a job folder made by `netgd-send prepare`, and checks the
result with an independent FAT reader (fatcheck.py), with FatFs (test_host dump) and with
`netgd-send collect`.
"""

import hashlib
import os
import re
import shutil
import subprocess
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
SEND = os.path.join(os.path.dirname(ROOT), "netgd-send")
sys.path.insert(0, SEND)
sys.path.insert(0, HERE)

from netgd_send import job as jobmod  # noqa: E402
import fatcheck  # noqa: E402

EXE = os.path.join(ROOT, "build", "test_host.exe" if os.name == "nt" else "test_host")
SECTORS = 2 * 1024 * 1024          # 1 GiB virtual card
SRCS = ["src/crc32.c", "src/manifest.c", "src/ngblk.c", "src/ngfs.c", "src/recv_core.c", "src/xport_pc.c",
        "src/diskio.c", "third_party/fatfs/ff.c", "third_party/fatfs/ffunicode.c", "third_party/fatfs/ffsystem.c",
        "test/dev_file.c", "test/fwctl_sim.c", "test/plat_host.c", "test/test_host.c"]


def build():
    os.makedirs(os.path.join(ROOT, "build"), exist_ok=True)
    newest = max(os.path.getmtime(os.path.join(ROOT, s)) for s in SRCS)
    for d in ("src", "test"):
        for f in os.listdir(os.path.join(ROOT, d)):
            if f.endswith(".h"):
                newest = max(newest, os.path.getmtime(os.path.join(ROOT, d, f)))
    if os.path.exists(EXE) and os.path.getmtime(EXE) >= newest:
        return
    cmd = ["gcc", "-O1", "-g", "-Wall", "-Wno-unused-function", "-Wno-format-truncation", "-DNG_HOST", "-Isrc",
           "-Ithird_party/fatfs", "-Itest", "-o", EXE] + SRCS
    r = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
    if r.returncode:
        raise RuntimeError("build failed:\n" + r.stdout + r.stderr)


def pattern(n, seed):
    base = bytes((i * 7 + seed * 13) & 0xFF for i in range(251))      # 251 is prime: no 512-periodicity
    return (base * (n // 251 + 1))[:n]


def crc32(b):
    import zlib
    return zlib.crc32(b) & 0xFFFFFFFF


class Run(object):
    def __init__(self, text, rc):
        self.rc, self.text = rc, text
        self.ngd = [l for l in text.splitlines() if l.startswith("NGD:")]
        self.phase = {}
        for m in re.finditer(r"^PHASE (\S+) (\d+)$", text, re.M):
            self.phase.setdefault(m.group(1), []).append(int(m.group(2)))
        m = re.search(r"^SIM (.*)$", text, re.M)
        self.sim = dict(re.findall(r"(\w+)=(\S*)", m.group(1))) if m else {}

    def has(self, prefix):
        return [l for l in self.ngd if l.startswith(prefix)]

    def n(self, key):
        return int(self.sim[key])


class Base(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        build()

    def setUp(self):
        self.work = os.path.join(HERE, "_work_" + re.sub(r"\W", "_", self.id().split(".")[-1]))
        shutil.rmtree(self.work, ignore_errors=True)
        os.makedirs(self.work)
        self.addCleanup(shutil.rmtree, self.work, True)
        self.drive = os.path.splitdrive(self.work)[0] or ""

    # --- sources / jobs
    def make_gdi(self, sub, sizes=(4 * 2352, 3 * 2352, 5 * 2048), seed=1):
        d = os.path.join(self.work, sub)
        os.makedirs(d, exist_ok=True)
        specs = [(1, 0, 4, 2352, "track01.bin"), (2, 150, 0, 2352, "track02.raw"), (3, 45000, 4, 2048, "track03.bin")]
        lines = ["3"]
        for (num, lba, ctrl, sec, nm), size in zip(specs, sizes):
            with open(os.path.join(d, nm), "wb") as f:
                f.write(pattern(size, num + seed))
            lines.append("%d %d %d %d %s 0" % (num, lba, ctrl, sec, nm))
        p = os.path.join(d, "disc.gdi")
        with open(p, "wb") as f:
            f.write(("\r\n".join(lines) + "\r\n").encode("ascii"))
        return p

    def prepare(self, gdi, job, name, mode="new", chunk_mib=8, mount=False):
        return jobmod.prepare(gdi, os.path.join(self.work, job), name=name, mode=mode, chunk_mib=chunk_mib, mount=mount)

    def jobdir(self, job):
        return os.path.join(self.work, job)

    def set_mode(self, job, mode):
        p = os.path.join(self.jobdir(job), "job.ngj")
        t = open(p, newline="").read()
        t = re.sub(r"^(put \d{4}) \w+$", r"\1 " + mode, t, flags=re.M)
        open(p, "w", newline="").write(t)

    def expected(self, gdi_path, prep):
        """name(lower) -> bytes of the files netgd-send put in the job folder entry"""
        out = {}
        edir = os.path.join(self.jobdir(prep["job"]), prep["num"])
        for name, _pc in prep["files"]:
            out[name.lower()] = open(os.path.join(edir, name), "rb").read()
        return out

    # --- the app
    sectors = SECTORS
    cluster = 4096

    def mkimage(self, name="card.img", sectors=None, cluster=None):
        sectors = sectors or self.sectors
        cluster = cluster or self.cluster
        img = os.path.join(self.work, name)
        r = subprocess.run([EXE, "mkimage", img, str(sectors), str(cluster)], capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        return img

    def run_app(self, img, job, *opts, sectors=None):
        sectors = sectors or self.sectors
        pcjob = jobmod.to_pc_path(self.jobdir(job))
        root = self.drive if self.drive else "/"
        cmd = [EXE, "run", img, str(sectors), root, pcjob] + list(opts)
        r = subprocess.run(cmd, capture_output=True, text=True)
        return Run(r.stdout, r.returncode)

    def collect(self, job):
        r = subprocess.run([sys.executable, os.path.join(SEND, "netgd-send.py"), "collect", "--job", self.jobdir(job)],
                           capture_output=True, text=True)
        return r.returncode, r.stdout + r.stderr

    def fs(self, img):
        return fatcheck.Fat32(img)

    def assert_image(self, fc, folder, files, msg=""):
        """folder holds exactly these files (lowercase name -> bytes), plus NETGD.MF"""
        got = fc.folder_files("/" + folder)
        names = {n.lower() for n in got}
        want = set(files) | {"netgd.mf"}
        self.assertEqual(names, want, msg + " names in folder")
        for n, v in got.items():
            if n.lower() in files:
                self.assertEqual(fc.read_file("/%s/%s" % (folder, n)), files[n.lower()], msg + " content of " + n)

    def file_hash(self, p):
        return hashlib.sha1(open(p, "rb").read()).hexdigest()


def prep_info(self, r, job):
    return {"job": job, "num": r.number, "files": r.files}


class TestBasics(Base):
    def test_selftest(self):
        r = subprocess.run([EXE, "selftest"], capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stdout)

    def test_put_new_fast_and_slow(self):
        for slow in (False, True):
            img = self.mkimage()
            gdi = self.make_gdi("src%d" % slow, sizes=(300 * 2352, 61 * 2352, 130 * 2048))
            r = self.prepare(gdi, "job%d" % slow, "GAME1", mode="new")
            exp = self.expected(gdi, prep_info(self, r, "job%d" % slow))
            run = self.run_app(img, "job%d" % slow, *(["--slow"] if slow else []), "--ckp", "65536")
            self.assertEqual(run.rc, 0, run.text)
            self.assertTrue(run.has("NGD:DONE 0001 ok"), run.text)
            self.assertTrue(run.has("NGD:HELLO"))
            self.assertTrue(run.has("NGD:FINISH ok"))
            self.assertTrue(run.has("NGD:END ok=1"))
            fc = self.fs(img)
            self.assertEqual(fc.errors, [])
            self.assertEqual(fc.lost_clusters(), 0)
            self.assert_image(fc, "GAME1", exp)
            vis = fc.visible_images()
            self.assertEqual(list(vis), ["/GAME1"])
            self.assertFalse([p for p in fc.files if p.lower().endswith((".ngp", ".ckp"))])
            for p in fc.folder_files("/GAME1"):
                if not p.upper().startswith("NETGD"):
                    self.assertEqual(fc.runs("/GAME1/" + p) <= 1, True, "contiguous: " + p)
            # firmware model
            self.assertEqual((run.n("violations"), run.n("unsess"), run.n("begin"), run.n("end"), run.n("endmode")),
                             (0, 0, 1, 1, 1))
            self.assertEqual(run.n("fnvbad"), 0)
            self.assertEqual(run.n("unverified"), 0)
            # netgd-send understands what we wrote
            code, out = self.collect("job%d" % slow)
            self.assertEqual(code, 0, out)
            # and FatFs reads it back (test_host dump)
            outd = os.path.join(self.work, "dump%d" % slow)
            d = subprocess.run([EXE, "dump", img, str(SECTORS), outd], capture_output=True, text=True)
            self.assertEqual(d.returncode, 0, d.stdout)
            for n, data in exp.items():
                self.assertEqual(crc32(data), int(re.search(r"CRC /GAME1/%s ([0-9A-F]+)" % n, d.stdout, re.I).group(1), 16))

    def test_mount_on_finish_and_fnv_off(self):
        img = self.mkimage()
        gdi = self.make_gdi("src")
        self.prepare(gdi, "job", "Crazy Taxi (US)", mount=True)
        run = self.run_app(img, "job", "--caps", "4", "--readback", "0")    # old firmware: no checksum -> readback forced
        self.assertEqual(run.rc, 0, run.text)
        self.assertEqual(run.sim["endmode"], "2")
        self.assertTrue(run.has('NGD:FINISH ok mounted="Crazy Taxi (US)"'), run.text)
        self.assertGreater(run.n("unverified"), 0)
        fc = self.fs(img)
        self.assertEqual(fc.errors, [])
        self.assertIn("/Crazy Taxi (US)/DISC.GDI", fc.files)

    def test_existing_modes_and_protection(self):
        img = self.mkimage()
        g1 = self.make_gdi("s1", seed=1)
        g2 = self.make_gdi("s2", seed=2)
        r1 = self.prepare(g1, "j1", "GAMEA", mode="new")
        e1 = self.expected(g1, prep_info(self, r1, "j1"))
        self.assertEqual(self.run_app(img, "j1").rc, 0)
        # new on an existing folder
        r2 = self.prepare(g2, "j2", "GAMEA", mode="new")
        e2 = self.expected(g2, prep_info(self, r2, "j2"))
        run = self.run_app(img, "j2")
        self.assertEqual(run.rc, 1)
        self.assertTrue(run.has("NGD:ERROR 0001 code=EEXIST"), run.text)
        self.assert_image(self.fs(img), "GAMEA", e1)
        # resume with a different manifest: refuse
        self.set_mode("j2", "resume")
        run = self.run_app(img, "j2")
        self.assertTrue(run.has("NGD:ERROR 0001 code=EEXIST"), run.text)
        # resume of the identical, complete image: verifies, DONE
        self.set_mode("j1", "resume")
        run = self.run_app(img, "j1")
        self.assertEqual(run.rc, 0, run.text)
        self.assertEqual(len(run.has("NGD:FILE 0001")), 4)
        # overwrite
        self.set_mode("j2", "overwrite")
        run = self.run_app(img, "j2")
        self.assertEqual(run.rc, 0, run.text)
        fc = self.fs(img)
        self.assertEqual(fc.errors, [])
        self.assert_image(fc, "GAMEA", e2)
        # list / statfs / verify / delete jobs
        job = os.path.join(self.jobdir("j2"), "job.ngj")
        with open(job, "w", newline="") as f:
            f.write("NETGD-JOB 1\nstatfs\nlist /\nverify GAMEA\ndelete GAMEA\nfinish none\n")
        run = self.run_app(img, "j2")
        self.assertEqual(run.rc, 0, run.text)
        self.assertTrue(run.has("NGD:STATFS free="))
        self.assertTrue(run.has("NGD:LIST complete GAMEA"), run.text)
        self.assertTrue(run.has('NGD:VERIFY "GAMEA" ok'))
        self.assertTrue(run.has('NGD:DELETE "GAMEA" ok'))
        self.assertEqual(run.sim["endmode"], "0")
        fc = self.fs(img)
        self.assertEqual((fc.errors, fc.lost_clusters(), [p for p in fc.files if "GAMEA" in p]), ([], 0, []))
        # delete refuses to touch a folder that is not an image / the root
        with open(job, "w", newline="") as f:
            f.write("NETGD-JOB 1\ndelete /\nfinish none\n")
        run = self.run_app(img, "j2")
        self.assertTrue(run.has("NGD:ERROR delete code=EPROTECTED"), run.text)

    def test_protected_default_folder(self):
        img = self.mkimage()
        g1 = self.make_gdi("s1", seed=1)
        g2 = self.make_gdi("s2", seed=2)
        self.prepare(g1, "j1", "01", mode="new")                 # a *new* 01 is allowed
        self.assertEqual(self.run_app(img, "j1").rc, 0)
        self.prepare(g2, "j2", "01", mode="overwrite")
        run = self.run_app(img, "j2")
        self.assertTrue(run.has("NGD:ERROR 0001 code=EPROTECTED"), run.text)
        open(os.path.join(self.jobdir("j2"), "force.req"), "w").close()
        run = self.run_app(img, "j2")
        self.assertEqual(run.rc, 0, run.text)

    def test_enospc_writes_nothing(self):
        sectors = 40 * 2048
        img = self.mkimage(sectors=sectors, cluster=512)
        before = self.file_hash(img)
        gdi = self.make_gdi("s", sizes=(14 * 1024 * 1024 // 2352 * 2352, 12 * 1024 * 1024 // 2352 * 2352, 2048 * 100))
        self.prepare(gdi, "j", "BIG")
        run = self.run_app(img, "j", sectors=sectors)
        self.assertEqual(run.rc, 1)
        self.assertTrue(run.has("NGD:ERROR 0001 code=ENOSPC need="), run.text)
        self.assertEqual(self.file_hash(img), before, "ENOSPC must leave the card untouched")

    def test_fragmented(self):
        sectors = 40 * 2048
        img = self.mkimage(sectors=sectors, cluster=512)
        r = subprocess.run([EXE, "fragment", img, str(sectors), "65536"], capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stdout)
        gdi = self.make_gdi("s", sizes=(300 * 2352, 100 * 2352, 100 * 2048))
        self.prepare(gdi, "j", "FRAGGY")
        run = self.run_app(img, "j", sectors=sectors)
        self.assertEqual(run.rc, 1)
        self.assertTrue(run.has("NGD:ERROR 0001 code=EFRAG"), run.text)
        fc = self.fs(img)
        self.assertEqual(fc.errors, [])
        # the failed attempt left a folder without an image; a retry that allows fragments works
        self.set_mode("j", "resume")
        open(os.path.join(self.jobdir("j"), "fragok.req"), "w").close()
        run = self.run_app(img, "j", sectors=sectors)
        self.assertEqual(run.rc, 0, run.text)
        fc = self.fs(img)
        self.assertEqual(fc.errors, [])
        exp = {n.lower(): open(os.path.join(self.jobdir("j"), "0001", n), "rb").read()
               for n in ("disc.gdi", "track01.bin", "track02.raw", "track03.bin")}
        self.assert_image(fc, "FRAGGY", exp)
        self.assertGreater(fc.runs("/FRAGGY/track01.bin"), 1)
        code, out = self.collect("j")
        self.assertEqual(code, 0, out)

    def test_bus_bit_flip_is_rewritten(self):
        img = self.mkimage()
        gdi = self.make_gdi("s", sizes=(300 * 2352, 61 * 2352, 130 * 2048))
        r = self.prepare(gdi, "j", "FLIP")
        exp = self.expected(gdi, prep_info(self, r, "j"))
        # flip in several data commands, one at a time
        for k in (3, 20, 40):
            img = self.mkimage()
            self.set_mode("j", "new")
            run = self.run_app(img, "j", "--flip", str(k), "--readback", "0")
            self.assertEqual(run.rc, 0, run.text)
            self.assertGreaterEqual(run.n("fnvbad"), 1, "flip %d not noticed" % k)
            fc = self.fs(img)
            self.assertEqual(fc.errors, [])
            self.assert_image(fc, "FLIP", exp, "flip %d" % k)

    def test_stop_and_resume_with_collect(self):
        MIB = 1 << 20
        img = self.mkimage()
        gdi = self.make_gdi("s", sizes=(1000 * 2352, 500 * 2352, 600 * 2048))
        r = self.prepare(gdi, "j", "BIGGAME", mode="resume", chunk_mib=1)
        exp = self.expected(gdi, prep_info(self, r, "j"))
        run = self.run_app(img, "j", "--ckp", str(MIB), "--pollms", "0", "--stopafter", "20")
        self.assertEqual(run.rc, 2, run.text)
        self.assertTrue(run.has("NGD:STOP"))
        self.assertEqual(run.sim["endmode"], "1")           # default image comes back
        self.assertEqual(self.collect("j")[0], 2)
        fc = self.fs(img)
        self.assertEqual(fc.errors, [])
        self.assertEqual(fc.visible_images(), {}, "half-written image must be invisible")
        self.assertIn("/BIGGAME/DISC.NGP", fc.files)
        run = self.run_app(img, "j", "--ckp", str(MIB))
        self.assertEqual(run.rc, 0, run.text)
        res = run.has("NGD:RESUME 0001")
        self.assertEqual(len(res), 1, run.text)
        self.assertIn("offset=1048576", res[0])
        code, out = self.collect("j")
        self.assertEqual(code, 0, out)
        self.assertIn("resumed from", out)
        fc = self.fs(img)
        self.assertEqual((fc.errors, fc.lost_clusters()), ([], 0))
        self.assert_image(fc, "BIGGAME", exp)

    def test_source_crc_error(self):
        img = self.mkimage()
        gdi = self.make_gdi("s")
        self.prepare(gdi, "j", "BADSRC")
        p = os.path.join(self.jobdir("j"), "0001", "track02.raw")
        with open(p, "r+b") as f:        # corrupt after the manifest was made (hard link: the source too)
            f.seek(100)
            f.write(b"\xff\xff")
        run = self.run_app(img, "j")
        self.assertEqual(run.rc, 1)
        self.assertTrue(run.has("NGD:ERROR 0001 code=ECRC file=3"), run.text)
        fc = self.fs(img)
        self.assertEqual(fc.visible_images(), {})
        self.assertEqual(fc.errors, [])


def copy(src, dst):
    for i in range(20):
        try:
            shutil.copyfile(src, dst)
            return
        except OSError:
            import time
            time.sleep(0.05)
    shutil.copyfile(src, dst)


def sweep(test, base_img, job, folder, exp, old=None, stride=1, ckp="4096", extra=()):
    """Cut the power after every N-th sector write; check the invariants; resume; check again."""
    ref_img = os.path.join(test.work, "ref.img")
    copy(base_img, ref_img)
    ref = test.run_app(ref_img, job, "--ckp", ckp, *extra)
    test.assertEqual(ref.rc, 0, ref.text)
    total = ref.n("writes")
    alloc_w = ref.phase["alloc_done"][0]
    stats = {"cuts": 0, "visible_new": 0, "visible_old": 0, "invisible": 0, "resumed": 0, "lost_max": 0, "total": total}
    fresh_leak_ok = old is None
    for n in range(0, total + 1, stride):
        img = os.path.join(test.work, "cut.img")
        copy(base_img, img)
        r = test.run_app(img, job, "--ckp", ckp, "--cut", str(n), *extra)
        test.assertEqual(r.n("violations"), 0)
        test.assertEqual((r.n("begin"), r.n("end")), (1, 1), "session must always be closed: cut %d" % n)
        fc = test.fs(img)
        test.assertEqual(fc.errors, [], "cut %d: structure" % n)
        # (b) never mounted-but-corrupt: a visible image is the complete old one or the complete new one
        vis = fc.visible_images()
        if "/" + folder in vis:
            got = {k.lower(): fc.read_file("/%s/%s" % (folder, k)) for k in fc.folder_files("/" + folder)
                   if not k.upper().startswith("NETGD")}
            if got == exp:
                stats["visible_new"] += 1
            elif old is not None and got == old:
                stats["visible_old"] += 1
            else:
                test.fail("cut %d: a visible image that is neither old nor new (files %s)" % (n, sorted(got)))
        else:
            stats["invisible"] += 1
            test.assertFalse(fc.folder_files("/" + folder) and any(
                k.lower().endswith((".gdi", ".cdi")) for k in fc.folder_files("/" + folder)))
        # (c) recovery run completes
        if old is not None:
            test.set_mode(job, "resume")
        r2 = test.run_app(img, job, "--ckp", ckp, *extra)
        if r2.rc != 0 and old is not None:
            test.assertTrue(r2.has("NGD:ERROR 0001 code=EEXIST"), "cut %d: %s" % (n, r2.text))
            test.set_mode(job, "overwrite")
            r2 = test.run_app(img, job, "--ckp", ckp, *extra)
        if old is not None:
            test.set_mode(job, "overwrite")
        test.assertEqual(r2.rc, 0, "cut %d: recovery failed\n%s" % (n, r2.text))
        if r2.has("NGD:RESUME"):
            stats["resumed"] += 1
        fc = test.fs(img)
        test.assertEqual(fc.errors, [], "cut %d: structure after recovery" % n)
        test.assert_image(fc, folder, exp, "cut %d after recovery" % n)
        test.assertFalse([p for p in fc.files if p.lower().endswith((".ngp", ".ckp"))], "cut %d: leftovers" % n)
        lost = fc.lost_clusters()
        stats["lost_max"] = max(stats["lost_max"], lost)
        if fresh_leak_ok and n >= alloc_w:
            # at most the cluster of NETGD.CKP itself (f_unlink frees the chain after the entry)
            test.assertLessEqual(lost, 1, "cut %d (after allocation) leaked %d clusters" % (n, lost))
        stats["cuts"] += 1
    return stats


class TestPowerLoss(Base):
    sectors = 80 * 2048          # a small virtual card keeps the Python-side FAT parsing quick
    cluster = 1024

    def test_sweep_fresh_small(self):
        img = self.mkimage()
        gdi = self.make_gdi("s", sizes=(10 * 2352, 5 * 2352, 9 * 2048))
        r = self.prepare(gdi, "j", "PL1", mode="resume")
        exp = self.expected(gdi, prep_info(self, r, "j"))
        st = sweep(self, img, "j", "PL1", exp, ckp="4096")
        print("\nsweep fresh/small:", st)
        self.assertGreater(st["resumed"], 5)
        self.assertGreater(st["visible_new"], 0)

    def test_sweep_fresh_slowpath(self):
        img = self.mkimage()
        gdi = self.make_gdi("s", sizes=(6 * 2352, 3 * 2352, 5 * 2048))
        r = self.prepare(gdi, "j", "PL2", mode="resume")
        exp = self.expected(gdi, prep_info(self, r, "j"))
        st = sweep(self, img, "j", "PL2", exp, ckp="2048", extra=("--slow",))
        print("\nsweep fresh/slow:", st)

    def test_sweep_overwrite(self):
        img = self.mkimage()
        gold = self.make_gdi("old", sizes=(8 * 2352, 4 * 2352, 6 * 2048), seed=5)
        ro = self.prepare(gold, "jold", "PL3", mode="new")
        old_exp = self.expected(gold, prep_info(self, ro, "jold"))
        self.assertEqual(self.run_app(img, "jold", "--ckp", "4096").rc, 0)
        gnew = self.make_gdi("new", sizes=(10 * 2352, 5 * 2352, 9 * 2048), seed=9)
        rn = self.prepare(gnew, "jnew", "PL3", mode="overwrite")
        new_exp = self.expected(gnew, prep_info(self, rn, "jnew"))
        self.assertNotEqual(old_exp, new_exp)
        st = sweep(self, img, "jnew", "PL3", new_exp, old=old_exp, ckp="4096")
        print("\nsweep overwrite:", st)
        self.assertGreater(st["visible_old"], 0)
        self.assertGreater(st["visible_new"], 0)
        self.assertGreater(st["invisible"], 0)

    def test_sweep_medium_strided(self):
        img = self.mkimage()
        gdi = self.make_gdi("s", sizes=(120 * 2352, 40 * 2352, 70 * 2048))
        r = self.prepare(gdi, "j", "PL4", mode="resume")
        exp = self.expected(gdi, prep_info(self, r, "j"))
        st = sweep(self, img, "j", "PL4", exp, stride=13, ckp="32768")
        print("\nsweep medium:", st)


if __name__ == "__main__":
    unittest.main(verbosity=2)
