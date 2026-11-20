"""Hardware-free tests of netgd-recv's core (design 9.1).  Run from anywhere:

    python tools/netgd-recv/test/run_tests.py                 # everything, quick power-cut sweeps
    python tools/netgd-recv/test/run_tests.py --full-sweep    # ... with every cut position
    python tools/netgd-recv/test/run_tests.py -k sweep        # one group

Needs gcc and Python 3.  Builds build/test_host from the same sources as the KOS app
(everything except main.c, dev_kos.c, fwxfer_kos.c, plat_kos.c), makes FAT32 images with FatFs'
f_mkfs, drives the app against a job folder made by `netgd-send prepare`, and checks the
result with an independent FAT reader (fatcheck.py), with FatFs (test_host dump) and with
`netgd-send collect`.

Power-cut sweeps: the QUICK mode (default) cuts at every sector write of the metadata phases
(folder, manifest, checkpoint file, allocation, the end of each file, read-back, commit,
clean-up) and at about a dozen positions spread over the plain data writes.  The FULL mode cuts
at every write (the "medium" image: every 13th).  Full is used when --full-sweep is given, when
NETGD_SWEEP=full, or under CI (CI=true, unless NETGD_SWEEP=quick).

Time limits (a hang is a failure, never a stuck run):
    NETGD_CHILD_TIMEOUT   seconds for one child process (test_host, gcc, netgd-send); default 120
    NETGD_TEST_TIMEOUT    seconds for one test; default 1800.  A sweep stops with a failure when it
                          gets there; anything else that is still running 60 s later is aborted
                          with a stack dump (the whole run exits with a non-zero code).

Scratch files go to a folder under the system temp directory, removed afterwards
(NETGD_TEST_WORK=<dir> to choose another place; it must be a path without spaces or non-ASCII
characters, the /pc/ rule of dcload).  Keep it off slow disks: every simulated sector write is
flushed to the image file, and a run starts well over a thousand short child processes.
"""

import atexit
import faulthandler
import hashlib
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
SEND = os.path.join(os.path.dirname(ROOT), "netgd-send")
sys.path.insert(0, SEND)
sys.path.insert(0, HERE)

from netgd_send import job as jobmod  # noqa: E402
import fatcheck  # noqa: E402

CHILD_TIMEOUT = float(os.environ.get("NETGD_CHILD_TIMEOUT", "120"))
TEST_TIMEOUT = float(os.environ.get("NETGD_TEST_TIMEOUT", "1800"))
_sweep_env = os.environ.get("NETGD_SWEEP", "").lower()
FULL_SWEEP = _sweep_env == "full" or (_sweep_env != "quick" and os.environ.get("CI", "").lower() == "true")


class ChildTimeout(AssertionError):
    """A child process did not finish in CHILD_TIMEOUT seconds (reported as a test failure)."""


def run_child(cmd, **kw):
    """subprocess.run with captured text output and a time limit: no child can hang the run."""
    kw.setdefault("capture_output", True)
    kw.setdefault("text", True)
    kw.setdefault("stdin", subprocess.DEVNULL)
    kw.setdefault("timeout", CHILD_TIMEOUT)
    try:
        return subprocess.run(cmd, **kw)
    except subprocess.TimeoutExpired as e:      # subprocess.run has killed the child already
        out = e.stdout or ""
        if isinstance(out, bytes):
            out = out.decode("utf-8", "replace")
        raise ChildTimeout("no exit after %g s (hang?): %s\n--- last output ---\n%s"
                           % (kw["timeout"], " ".join(map(str, cmd)), out[-2000:])) from None


def _pc_safe(path):
    try:
        jobmod.to_pc_path(os.path.join(path, "x"))
        return True
    except Exception:
        return False


def _work_root():
    """Where the scratch folders go: NETGD_TEST_WORK, else the temp directory, else next to this file."""
    env = os.environ.get("NETGD_TEST_WORK")
    if env:
        os.makedirs(env, exist_ok=True)
        return os.path.abspath(env), False
    tmp = os.path.realpath(tempfile.gettempdir())
    if _pc_safe(tmp):
        return tempfile.mkdtemp(prefix="ngr-", dir=tmp), True
    return HERE, False                          # e.g. a user name with a space: fall back to the source tree


WORK_ROOT, _work_root_is_ours = _work_root()
if _work_root_is_ours:
    atexit.register(shutil.rmtree, WORK_ROOT, True)

EXE = os.path.join(ROOT, "build", "test_host.exe" if os.name == "nt" else "test_host")
SECTORS = 2 * 1024 * 1024          # 1 GiB virtual card
SRCS = ["src/crc32.c", "src/manifest.c", "src/ngblk.c", "src/fwctl.c", "src/ngfs.c", "src/recv_core.c", "src/xport_pc.c",
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
    r = run_child(cmd, cwd=ROOT, timeout=max(CHILD_TIMEOUT, 600))
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
        # per-test time limit: sweeps check self.deadline themselves; whatever else is still running
        # 60 s after it is aborted with a stack dump (the run exits with code 1)
        self.deadline = time.monotonic() + TEST_TIMEOUT
        faulthandler.dump_traceback_later(TEST_TIMEOUT + 60, exit=True)
        self.addCleanup(faulthandler.cancel_dump_traceback_later)
        # short folder names: the app's /pc/ path buffers are 160..200 bytes
        name = re.sub(r"\W", "_", self.id().split(".")[-1])
        self.work = os.path.join(WORK_ROOT, "_work_" + re.sub(r"^test_", "", name)[:40])
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
        with open(p, newline="") as f:
            t = f.read()
        t = re.sub(r"^(put \d{4}) \w+$", r"\1 " + mode, t, flags=re.M)
        with open(p, "w", newline="") as f:
            f.write(t)

    def expected(self, gdi_path, prep):
        """name(lower) -> bytes of the files netgd-send put in the job folder entry"""
        out = {}
        edir = os.path.join(self.jobdir(prep["job"]), prep["num"])
        for name, _pc in prep["files"]:
            with open(os.path.join(edir, name), "rb") as f:
                out[name.lower()] = f.read()
        return out

    # --- the app
    sectors = SECTORS
    cluster = 4096

    def mkimage(self, name="card.img", sectors=None, cluster=None):
        sectors = sectors or self.sectors
        cluster = cluster or self.cluster
        img = os.path.join(self.work, name)
        r = run_child([EXE, "mkimage", img, str(sectors), str(cluster)])
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        return img

    def run_app(self, img, job, *opts, sectors=None):
        sectors = sectors or self.sectors
        pcjob = jobmod.to_pc_path(self.jobdir(job))
        root = self.drive if self.drive else "/"
        cmd = [EXE, "run", img, str(sectors), root, pcjob] + list(opts)
        r = run_child(cmd)
        return Run(r.stdout, r.returncode)

    def collect(self, job):
        r = run_child([sys.executable, os.path.join(SEND, "netgd-send.py"), "collect", "--job", self.jobdir(job)])
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
        with open(p, "rb") as f:
            return hashlib.sha1(f.read()).hexdigest()


def prep_info(self, r, job):
    return {"job": job, "num": r.number, "files": r.files}


class TestBasics(Base):
    def test_selftest(self):
        r = run_child([EXE, "selftest"])
        self.assertEqual(r.returncode, 0, r.stdout)

    def test_a_hanging_child_is_a_failure(self):
        """The time limit of run_child works: a child that never returns is killed and reported."""
        t0 = time.monotonic()
        with self.assertRaises(ChildTimeout) as cm:
            run_child([EXE, "hang"], timeout=2)
        self.assertLess(time.monotonic() - t0, 60)
        self.assertIsInstance(cm.exception, AssertionError)     # unittest counts it as a FAIL
        self.assertIn("hanging", str(cm.exception))             # what the child printed before is kept
        self.assertEqual(run_child([EXE, "selftest"]).returncode, 0)

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
            d = run_child([EXE, "dump", img, str(SECTORS), outd])
            self.assertEqual(d.returncode, 0, d.stdout)
            for n, data in exp.items():
                self.assertEqual(crc32(data), int(re.search(r"CRC /GAME1/%s ([0-9A-F]+)" % n, d.stdout, re.I).group(1), 16))

    def test_mount_on_finish(self):
        img = self.mkimage()
        gdi = self.make_gdi("src")
        self.prepare(gdi, "job", "Crazy Taxi (US)", mount=True)
        run = self.run_app(img, "job", "--readback", "0")
        self.assertEqual(run.rc, 0, run.text)
        self.assertEqual(run.sim["endmode"], "2")
        self.assertTrue(run.has('NGD:FINISH ok mounted="Crazy Taxi (US)"'), run.text)
        self.assertEqual(run.n("unverified"), 0)
        fc = self.fs(img)
        self.assertEqual(fc.errors, [])
        self.assertIn("/Crazy Taxi (US)/DISC.GDI", fc.files)

    def test_firmware_without_session_and_checksum_is_refused(self):
        img = self.mkimage()
        before = self.file_hash(img)
        gdi = self.make_gdi("src")
        self.prepare(gdi, "job", "GAME1")
        for caps in (0, 1, 2, 4, 5, 6):         # anything but bit0 (SESSION) AND bit1 (CHECKSUM)
            run = self.run_app(img, "job", "--caps", str(caps))
            self.assertEqual(run.rc, 1, run.text)
            self.assertTrue(run.has("NGD:ERROR session code=EFW"), run.text)
            self.assertEqual((run.n("begin"), run.n("end"), run.n("cmds")), (0, 0, 0), "caps %d" % caps)
            self.assertEqual(self.file_hash(img), before)

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
        r = run_child([EXE, "fragment", img, str(sectors), "65536"])
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
        exp = {}
        for n in ("disc.gdi", "track01.bin", "track02.raw", "track03.bin"):
            with open(os.path.join(self.jobdir("j"), "0001", n), "rb") as f:
                exp[n.lower()] = f.read()
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
            time.sleep(0.05)
    shutil.copyfile(src, dst)


def cut_points(ref, total, stride):
    """The sector-write counts after which the power is cut.

    Full mode: 0, stride, 2*stride, .. total.  Quick mode: every write of the metadata phases --
    from the start to just after the allocation (folder, NETGD.MF, NETGD.CKP, the FAT chains), around
    the end of every file, and from the last file to the end (read-back, commit, removal of the
    checkpoint) -- two writes on each side of the first checkpoints, and about a dozen positions
    spread over the rest (plain data sectors and the later, identical checkpoint rewrites)."""
    if FULL_SWEEP:
        return list(range(0, total + 1, stride))
    ph = ref.phase
    pts = set(range(0, min(total, ph["alloc_done"][0] + 2) + 1))
    files = ph.get("file_done", [])
    for w in files:
        pts.update(range(w - 2, w + 3))
    if files:
        pts.update(range(files[-1] - 2, total + 1))
    for w in ph.get("checkpoint", [])[:2]:
        pts.update(range(w - 2, w + 3))
    pts.update(range(0, total + 1, max(stride, total // 12, 1)))
    pts.add(total)
    return sorted(p for p in pts if 0 <= p <= total)


def sweep(test, base_img, job, folder, exp, old=None, stride=1, ckp="4096", extra=()):
    """Cut the power after a number of sector writes (see cut_points); check the invariants; resume;
    check again."""
    ref_img = os.path.join(test.work, "ref.img")
    copy(base_img, ref_img)
    ref = test.run_app(ref_img, job, "--ckp", ckp, *extra)
    test.assertEqual(ref.rc, 0, ref.text)
    total = ref.n("writes")
    alloc_w = ref.phase["alloc_done"][0]
    stats = {"mode": "full" if FULL_SWEEP else "quick", "cuts": 0, "visible_new": 0, "visible_old": 0,
             "invisible": 0, "resumed": 0, "lost_max": 0, "total": total}
    fresh_leak_ok = old is None
    t0 = time.monotonic()
    for n in cut_points(ref, total, stride):
        if time.monotonic() > test.deadline:
            test.fail("sweep not finished after %d s (NETGD_TEST_TIMEOUT): %d cuts done, stopped before cut %d of %d"
                      % (TEST_TIMEOUT, stats["cuts"], n, total))
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
    stats["seconds"] = round(time.monotonic() - t0, 1)
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


class TestHardening(Base):
    """Review R48 findings: UNIT ATTENTION, session always ended, path checks, read flips, FSInfo, ..."""

    def card_with(self, name="GAMEA", seed=1, sectors=None, cluster=None):
        img = self.mkimage(sectors=sectors, cluster=cluster)
        tag = name.replace("/", "_")
        gdi = self.make_gdi("s_" + tag, seed=seed)
        self.prepare(gdi, "j_" + tag, name)
        run = self.run_app(img, "j_" + tag, sectors=sectors)
        self.assertEqual(run.rc, 0, run.text)
        return img

    def write_job(self, job, text):
        os.makedirs(self.jobdir(job), exist_ok=True)
        with open(os.path.join(self.jobdir(job), "job.ngj"), "w", newline="") as f:
            f.write(text)

    # --- 1. UNIT ATTENTION
    def test_unit_attention_twice_in_a_row(self):
        img = self.mkimage()
        gdi = self.make_gdi("s")
        r = self.prepare(gdi, "j", "UA1", mode="resume")
        exp = self.expected(gdi, prep_info(self, r, "j"))
        for k in range(2):      # each run starts with the UA the previous END / mount left pending
            run = self.run_app(img, "j", "--ua")
            self.assertEqual(run.rc, 0, "run %d\n%s" % (k, run.text))
            self.assertTrue(run.has("NGD:HELLO"), run.text)
            self.assertEqual(run.n("ua"), 1, "the UA of the previous END must have been retried over (run %d)" % k)
            self.assertEqual((run.n("begin"), run.n("end")), (1, 1))
        self.assert_image(self.fs(img), "UA1", exp)

    def test_mount_default_fallback_after_failed_end2(self):
        img = self.mkimage()
        gdi = self.make_gdi("s")
        self.prepare(gdi, "j", "UA2", mount=True)
        run = self.run_app(img, "j", "--ua", "--failend2")
        self.assertTrue(run.has("NGD:FINISH fail code=EFW"), run.text)
        self.assertEqual(run.n("defmounts"), 1, "MOUNT 0xFF must get through the UA the failed END left: " + run.text)
        self.assertEqual(run.n("end"), 1)
        self.assertTrue(run.has("NGD:END ok=0"))

    # --- 2. the session is always ended
    def test_non_fat32_card_ends_the_session(self):
        img = self.mkimage()
        with open(img, "r+b") as f:
            f.seek(446 + 4)
            f.write(b"\x07")                      # NTFS/exFAT type byte
        gdi = self.make_gdi("s")
        self.prepare(gdi, "j", "NOFAT")
        run = self.run_app(img, "j")
        self.assertEqual(run.rc, 1, run.text)
        self.assertTrue(run.has("NGD:ERROR session code=EFS"), run.text)
        self.assertEqual((run.n("begin"), run.n("end"), run.n("endmode")), (1, 1, 1), run.text)

    def test_unreadable_card_ends_the_session(self):
        img = self.mkimage()
        gdi = self.make_gdi("s")
        self.prepare(gdi, "j", "NOREAD")
        run = self.run_app(img, "j", "--rflip-all")          # no two reads of a sector ever agree
        self.assertEqual(run.rc, 1, run.text)
        self.assertTrue(run.has("NGD:ERROR session code=EIO"), run.text)
        self.assertEqual((run.n("begin"), run.n("end")), (1, 1), run.text)
        self.assertGreater(run.n("rdmis"), 0)

    # --- 3. path validation
    def test_folder_arguments_are_validated(self):
        img = self.card_with("01", seed=1)
        base = self.file_hash(img)
        args = ("/01", "01.", "01 ", "//", "01//", "01/", "../01", "a//b")
        self.write_job("pj", "NETGD-JOB 1\nstatfs\n" + "".join(
            "%s %s\n" % (op, arg) for op in ("delete", "verify", "list") for arg in args) + "finish none\n")
        run = self.run_app(img, "pj")
        bad = run.has("NGD:ERROR delete") + run.has("NGD:ERROR verify") + run.has("NGD:ERROR list")
        self.assertEqual(len(bad), 3 * len(args), run.text)
        self.assertTrue(all("code=EMANIFEST" in l for l in bad), run.text)
        self.assertEqual(run.has("NGD:DELETE"), [], run.text)
        self.assertEqual(self.file_hash(img), base, "a rejected argument must not touch the card")
        self.assertEqual(sorted(self.fs(img).visible_images()), ["/01"])
        # the plain name is still protected (folder 01), root list is fine
        self.write_job("pj", "NETGD-JOB 1\ndelete 01\nlist /\nverify 01\nfinish none\n")
        run = self.run_app(img, "pj")
        self.assertTrue(run.has("NGD:ERROR delete code=EPROTECTED"), run.text)
        self.assertTrue(run.has("NGD:LIST complete 01"), run.text)
        self.assertTrue(run.has('NGD:VERIFY "01" ok'), run.text)

    def test_finish_mount_name_is_validated(self):
        img = self.card_with("GAMEA", seed=1)
        for name in ("A" * 97, "GAMEA.", "GAMEA ", "/GAMEA", "GAMEA/"):
            self.write_job("fj", "NETGD-JOB 1\nstatfs\nfinish mount %s\n" % name)
            run = self.run_app(img, "fj")
            self.assertTrue(run.has("NGD:FINISH fail code=ENOENT"), "%r: %s" % (name, run.text))
            self.assertEqual(run.sim["endmode"], "1", "%r mounts the default instead" % name)
            self.assertEqual(run.n("namedmounts"), 0)
        self.write_job("fj", "NETGD-JOB 1\nstatfs\nfinish mount GAMEA\n")
        run = self.run_app(img, "fj")
        self.assertTrue(run.has('NGD:FINISH ok mounted="GAMEA"'), run.text)
        self.assertEqual(run.sim["endmode"], "2")
        self.assertEqual(run.sim["endname"], "GAMEA")

    def test_protected_default_path_forms(self):
        img = self.card_with("GAMES/X", seed=1)
        cfgfile = os.path.join(self.work, "cfg")
        for cfg in ("image=/GAMES/X/\n", "image = GAMES//X\r\n", "image=games/x.\n"):
            with open(cfgfile, "w", newline="") as f:
                f.write("# test\n" + cfg)
            r = run_child([EXE, "addfile", img, str(SECTORS), "/GDEMU.CFG", cfgfile])
            self.assertEqual(r.returncode, 0, r.stdout)
            before = self.file_hash(img)
            self.write_job("dj", "NETGD-JOB 1\ndelete GAMES/X\nfinish none\n")
            run = self.run_app(img, "dj")
            self.assertTrue(run.has("NGD:ERROR delete code=EPROTECTED"), "%r: %s" % (cfg, run.text))
            self.assertEqual(self.file_hash(img), before)
            # an overwrite put of the same folder is refused as well
            g = self.make_gdi("sn", seed=7)
            self.prepare(g, "jo", "GAMES/X", mode="overwrite")
            run = self.run_app(img, "jo")
            self.assertTrue(run.has("NGD:ERROR 0001 code=EPROTECTED"), "%r: %s" % (cfg, run.text))
            self.assertEqual(self.file_hash(img), before)
        # with force.req it goes
        open(os.path.join(self.jobdir("dj"), "force.req"), "w").close()
        run = self.run_app(img, "dj")
        self.assertTrue(run.has('NGD:DELETE "GAMES/X" ok'), run.text)

    # --- 4. read-side bit flips
    def test_read_bit_flips_single_sector(self):
        gdi = self.make_gdi("s", sizes=(60 * 2352, 20 * 2352, 30 * 2048))
        r = self.prepare(gdi, "j", "RFLIP")
        exp = self.expected(gdi, prep_info(self, r, "j"))
        seen = 0
        for k in (1, 2, 5, 9, 30, 100):
            img = self.mkimage()
            self.set_mode("j", "new")
            run = self.run_app(img, "j", "--rflip", str(k))
            self.assertEqual(run.rc, 0, "rflip %d\n%s" % (k, run.text))
            seen += run.n("rdmis")
            fc = self.fs(img)
            self.assertEqual(fc.errors, [])
            self.assert_image(fc, "RFLIP", exp, "rflip %d" % k)
        self.assertGreaterEqual(seen, 5, "single-sector read flips were not noticed")

    # --- 5. FSInfo
    def test_leaked_clusters_do_not_fake_free_space(self):
        sectors = 40 * 2048
        img = self.mkimage(sectors=sectors, cluster=512)
        fc = self.fs(img)
        n = fc.ncl
        # a power cut inside f_expand: FAT entries (both copies) say "allocated", FSInfo still says "all free"
        first, last = 100, 100 + int(n * 0.8)
        blob = struct.pack("<%dI" % (last - first), *([0x0FFFFFFF] * (last - first)))
        with open(img, "r+b") as f:
            for copy_no in range(fc.nfats):
                f.seek((fc.fat_lba + copy_no * fc.fatsz) * 512 + first * 4)
                f.write(blob)
        before = self.file_hash(img)
        gdi = self.make_gdi("s")
        self.prepare(gdi, "j", "LEAKY")
        run = self.run_app(img, "j", sectors=sectors)
        self.assertEqual(run.rc, 1, run.text)
        m = re.search(r"code=ENOSPC need=(\d+) free=(\d+)", run.text)
        self.assertTrue(m, "a put on a card with leaked clusters must say ENOSPC: " + run.text)
        self.assertLess(int(m.group(2)), 10 * 1024 * 1024, "free space must come from a FAT scan, not FSInfo")
        self.assertEqual(self.file_hash(img), before)
        self.assertEqual((run.n("begin"), run.n("end")), (1, 1))

    # --- mount-time FAT scan: 256-sector double reads instead of FatFs's one-sector window
    def fat_ref(self, img):
        """(free bytes, FAT sectors the scan has to read): counted straight from FAT 1 of the image"""
        import array
        with open(img, "rb") as f:
            mbr = f.read(512)
            pstart = struct.unpack_from("<I", mbr, 446 + 8)[0]
            f.seek(pstart * 512)
            bpb = f.read(512)
            spc, rsv, nfats = struct.unpack_from("<BHB", bpb, 13)
            tot, fatsz = struct.unpack_from("<I", bpb, 32)[0], struct.unpack_from("<I", bpb, 36)[0]
            ncl = (tot - rsv - nfats * fatsz) // spc
            f.seek((pstart + rsv) * 512)
            raw = f.read((ncl + 2) * 4)
        a = array.array("I")
        self.assertEqual(a.itemsize, 4)
        a.frombytes(raw + b"\0" * ((ncl + 2) * 4 - len(raw)))
        if sys.byteorder != "little":
            a.byteswap()
        free = sum(1 for v in a[2:] if not v & 0x0FFFFFFF)
        return free * spc * 512, ((ncl + 2) * 4 + 511) // 512

    def poke_fat(self, img, first, values):
        """overwrite FAT entries first.. in every FAT copy"""
        with open(img, "r+b") as f:
            pstart = struct.unpack_from("<I", f.read(512), 446 + 8)[0]
            f.seek(pstart * 512)
            bpb = f.read(512)
            rsv, nfats = struct.unpack_from("<HB", bpb, 14)
            fatsz = struct.unpack_from("<I", bpb, 36)[0]
            blob = struct.pack("<%dI" % len(values), *values)
            for c in range(nfats):
                f.seek((pstart + rsv + c * fatsz) * 512 + first * 4)
                f.write(blob)

    def statfs(self, img, *opts, sectors=None):
        self.write_job("sj", "NETGD-JOB 1\nstatfs\nfinish none\n")
        return self.run_app(img, "sj", *opts, sectors=sectors)

    def free_of(self, run):
        hello = re.search(r"^NGD:HELLO .* free=(\d+) ", run.text, re.M)
        st = re.search(r"^NGD:STATFS free=(\d+) ", run.text, re.M)
        self.assertTrue(hello and st, run.text)
        self.assertEqual(hello.group(1), st.group(1), run.text)
        return int(st.group(1))

    def test_mount_free_count_equals_the_fat(self):
        sectors = 40 * 2048
        # fragmented: free space only in holes
        img = self.mkimage("frag.img", sectors=sectors, cluster=512)
        r = run_child([EXE, "fragment", img, str(sectors), "65536"])
        self.assertEqual(r.returncode, 0, r.stdout)
        ref = self.fat_ref(img)[0]
        self.assertGreater(ref, 0)
        self.assertEqual(self.free_of(self.statfs(img, sectors=sectors)), ref)
        # leaked clusters, a pattern of holes and odd values, the last cluster free and the one before it
        # used; FSInfo knows nothing of any of it
        img = self.mkimage("leak.img", sectors=sectors, cluster=512)
        n = self.fs(img).ncl
        self.poke_fat(img, 100, [0x0FFFFFFF] * int(n * 0.6))
        self.poke_fat(img, 100 + int(n * 0.7), [0x0FFFFFFF, 0, 0, 0x0FFFFFF7, 0xF0000000, 0x00000100, 1] * 500)
        self.poke_fat(img, n, [0x0FFFFFFF, 0])                 # entries ncl and ncl + 1: the last two clusters
        ref = self.fat_ref(img)[0]
        run = self.statfs(img, sectors=sectors)
        self.assertEqual(self.free_of(run), ref, run.text)
        # a card of the usual shape with an image on it
        img = self.card_with("GAMEA", seed=1)
        self.assertEqual(self.free_of(self.statfs(img)), self.fat_ref(img)[0])

    def test_mount_fat_chunk_bit_flip_is_reread(self):
        img = self.mkimage()                                    # 1 GiB, 4 KiB clusters: 2048 FAT sectors = 8 chunks
        self.poke_fat(img, 5000, [0x0FFFFFFF, 0, 0] * 30000)
        ref, scan = self.fat_ref(img)
        chunks = (scan + 255) // 256
        self.assertGreaterEqual(chunks, 8)
        base = self.file_hash(img)
        clean = self.statfs(img)
        self.assertEqual(clean.n("rdmis"), 0)
        self.assertEqual(self.free_of(clean), ref)
        self.assertEqual(self.file_hash(img), base, "statfs writes nothing")
        for k in (1, 2, 3, 2 * chunks - 1, 2 * chunks):
            run = self.statfs(img, "--rflipn", str(k))
            self.assertEqual(run.rc, 0, "rflipn %d\n%s" % (k, run.text))
            self.assertEqual(run.n("rdmis"), 1, "flip in multi-sector read %d was not noticed\n%s" % (k, run.text))
            self.assertEqual(self.free_of(run), ref, "rflipn %d" % k)
            self.assertEqual(run.n("rcmds"), clean.n("rcmds") + 2, "one mismatch: that chunk is read twice again")

    def test_mount_fat_persistent_flip_is_eio(self):
        img = self.mkimage()
        base = self.file_hash(img)
        gdi = self.make_gdi("s")
        self.prepare(gdi, "j", "NOFAT")
        run = self.run_app(img, "j", "--rflipn-all")           # no two reads of a FAT chunk ever agree
        self.assertEqual(run.rc, 1, run.text)
        self.assertTrue(run.has("NGD:ERROR session code=EIO"), run.text)
        self.assertEqual(run.has("NGD:HELLO"), [], run.text)
        self.assertEqual(run.n("rdmis"), 4, run.text)           # NGBLK_READ_TRIES pairs, then give up
        self.assertEqual((run.n("begin"), run.n("end")), (1, 1), run.text)
        self.assertEqual(self.file_hash(img), base, "a card whose FAT cannot be read must not be written")

    def test_mount_read_commands_on_a_large_fat(self):
        # as many clusters as a 128 GB card with 32 KiB clusters has (3.9 million: a FAT of 15 MiB = 30,500 sectors),
        # in an image file that stays below 2 GiB: 512-byte clusters
        sectors = 4000000
        img = self.mkimage("big.img", sectors=sectors, cluster=512)
        scan = self.fat_ref(img)[1]
        self.assertGreater(scan, 30000)
        n = scan * 128 - 5000
        self.poke_fat(img, 1000, ([0x0FFFFFFF, 0, 0, 0x0FFFFFFF, 0x0FFFFFFF, 0, 0x0FFFFFFF] * (n // 7 + 1))[:n])
        ref = self.fat_ref(img)[0]
        run = self.statfs(img, "--mountms", "0", sectors=sectors)
        self.assertEqual(run.rc, 0, run.text)
        self.assertEqual(self.free_of(run), ref)
        chunks = (scan + 255) // 256
        # two reads per 256-sector chunk, plus the single-sector double reads of the MBR, the boot sector
        # and FSInfo.  With FatFs's own scan: two commands per FAT SECTOR (2 * scan, 60,000 and more).
        self.assertGreaterEqual(run.n("rcmds"), 2 * chunks)
        self.assertLess(run.n("rcmds"), 2 * chunks + 40, run.text)
        self.assertLess(run.n("rcmds") * 100, 2 * scan)
        print("large FAT: %d sectors to scan, %d read commands in the whole run (one-sector double reads: %d)"
              % (scan, run.n("rcmds"), 2 * scan), file=sys.stderr)
        # progress: always a closing line; pct lines in between (here not spaced out), rising, before HELLO
        self.assertTrue(run.has("NGD:MOUNT pct=100 fat_sectors=%d ms=" % scan), run.text)
        pcts = [int(m) for m in re.findall(r"^NGD:MOUNT pct=(\d+)$", run.text, re.M)]
        self.assertEqual(len(pcts), chunks - 1)
        self.assertEqual(pcts, sorted(pcts))
        self.assertTrue(pcts[0] >= 0 and pcts[-1] < 100)
        self.assertLess(run.ngd.index(run.has("NGD:MOUNT pct=100")[0]), run.ngd.index(run.has("NGD:HELLO")[0]))
        # with the default spacing (1 s) a scan this quick prints the closing line only
        run = self.statfs(img, sectors=sectors)
        self.assertEqual(len(run.has("NGD:MOUNT")), 1, run.text)
        # netgd-send takes the new lines as chatter
        res = jobmod.parse_result(run.text)
        self.assertEqual(res.malformed, [])
        self.assertEqual(int(res.hello["free"]), ref)

    # --- 9. cancel during the read-back
    def test_cancel_during_readback_then_resume(self):
        gdi = self.make_gdi("s", sizes=(300 * 2352, 61 * 2352, 130 * 2048))
        r = self.prepare(gdi, "j", "RB1", mode="resume")
        exp = self.expected(gdi, prep_info(self, r, "j"))
        img0 = self.mkimage("c0.img")
        ref = self.run_app(img0, "j", "--readback", "0", "--pollms", "0")
        self.assertEqual(ref.rc, 0, ref.text)
        p0 = ref.n("polls")
        img = self.mkimage("c1.img")
        run = self.run_app(img, "j", "--readback", "1", "--pollms", "0", "--stopafter", str(p0))
        self.assertEqual(run.rc, 2, run.text)
        self.assertTrue(run.has("NGD:STOP"), run.text)
        self.assertIn("readback", run.phase)
        self.assertNotIn("verified", run.phase)
        self.assertEqual(run.sim["endmode"], "1")
        fc = self.fs(img)
        self.assertEqual(fc.errors, [])
        self.assertEqual(fc.visible_images(), {}, "not published before the read-back is done")
        self.assertEqual(self.collect("j")[0], 2)
        run = self.run_app(img, "j")
        self.assertEqual(run.rc, 0, run.text)
        fc = self.fs(img)
        self.assertEqual((fc.errors, fc.lost_clusters()), ([], 0))
        self.assert_image(fc, "RB1", exp)
        self.assertEqual(self.collect("j")[0], 0)

    def test_readback_progress_line_format(self):
        img = self.mkimage()
        gdi = self.make_gdi("s", sizes=(300 * 2352, 61 * 2352, 130 * 2048))
        self.prepare(gdi, "j", "RB2")
        run = self.run_app(img, "j", "--pollms", "0")
        self.assertEqual(run.rc, 0, run.text)
        # (at most one line per second: a fast host may print none; the format is what matters when present)
        for l in run.has("NGD:READBACK"):
            self.assertRegex(l, r"^NGD:READBACK 0001 file=\d/4 bytes=\d+/\d+$")

    # --- 10. direct-write preconditions
    def test_ckp_claiming_contiguity_on_a_fragmented_file(self):
        sectors = 40 * 2048
        img = self.mkimage(sectors=sectors, cluster=512)
        r = run_child([EXE, "fragment", img, str(sectors), "65536"])
        self.assertEqual(r.returncode, 0, r.stdout)
        gdi = self.make_gdi("s", sizes=(300 * 2352, 100 * 2352, 100 * 2048))
        r = self.prepare(gdi, "j", "FRAGLIE", mode="resume")
        exp = self.expected(gdi, prep_info(self, r, "j"))
        open(os.path.join(self.jobdir("j"), "fragok.req"), "w").close()
        run = self.run_app(img, "j", "--pollms", "0", "--stopafter", "0", sectors=sectors)
        self.assertEqual(run.rc, 2, run.text)
        fc = self.fs(img)
        self.assertGreater(fc.runs("/FRAGLIE/track01.bin"), 1, "the file must be fragmented for this test")
        ck = [p for p in fc.files if p.upper() == "/FRAGLIE/NETGD.CKP"][0]
        lba = fc.data_lba + (fc.files[ck]["first"] - 2) * fc.spc
        with open(img, "r+b") as f:
            f.seek(lba * 512)
            b = bytearray(f.read(512))
            self.assertEqual(bytes(b[:4]), b"NGCK")
            self.assertEqual(b[14] | b[15] << 8, 1, "the checkpoint says FRAG")
            b[14] = b[15] = 0                                   # now it claims "contiguous"
            struct.pack_into("<I", b, 508, crc32(bytes(b[:508])))
            f.seek(lba * 512)
            f.write(bytes(b))
        os.remove(os.path.join(self.jobdir("j"), "fragok.req"))
        run = self.run_app(img, "j", sectors=sectors)
        self.assertEqual(run.rc, 0, run.text)
        self.assertTrue(run.has("NGD:NOTE 0001 file=2 slow-path"), run.text)
        fc = self.fs(img)
        self.assertEqual(fc.errors, [])
        self.assert_image(fc, "FRAGLIE", exp)
        self.assertEqual(self.collect("j")[0], 0)

    # --- 11. small ones
    def test_list_is_not_truncated(self):
        img = self.mkimage()
        r = run_child([EXE, "mkdirs", img, str(SECTORS), "D", "40"])
        self.assertEqual(r.returncode, 0, r.stdout)
        self.write_job("lj", "NETGD-JOB 1\nlist /\nfinish none\n")
        run = self.run_app(img, "lj")
        names = sorted(l.split()[-1] for l in run.has("NGD:LIST"))
        self.assertEqual(names, ["D%03d" % i for i in range(40)], run.text)


if __name__ == "__main__":
    if "--full-sweep" in sys.argv:
        sys.argv.remove("--full-sweep")
        FULL_SWEEP = True
    print("power-cut sweeps: %s; scratch: %s; limits: child %g s, test %g s"
          % ("FULL" if FULL_SWEEP else "quick (--full-sweep or NETGD_SWEEP=full for every cut)", WORK_ROOT,
             CHILD_TIMEOUT, TEST_TIMEOUT), file=sys.stderr)
    unittest.main(verbosity=2)
