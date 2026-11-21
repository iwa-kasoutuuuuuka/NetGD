"""Hardware-free tests of netgd-menu's core (docs/design/menu-app.md 11).  Run from anywhere:

    python tools/netgd-menu/test/run_tests.py
    python tools/netgd-menu/test/run_tests.py -k selftest

Needs gcc and Python 3.  Builds build/menu_host from the menu core, the netgd-recv core and FatFs
(never the KOS files main_kos.c / plat_kos.c / net_kos.c, nor netgd-recv's dev_kos.c / fwxfer_kos.c).
Scratch files go to a folder under the system temp directory, removed afterwards (NETGD_TEST_WORK=<dir>
to choose another place).  Every child process has a time limit (NETGD_CHILD_TIMEOUT, default 120 s): a hang is a
test failure, never a stuck run.  Nothing here has run on a Dreamcast.
"""

import atexit
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)                      # tools/netgd-menu
RECV = os.path.join(os.path.dirname(ROOT), "netgd-recv")

CHILD_TIMEOUT = float(os.environ.get("NETGD_CHILD_TIMEOUT", "120"))


class ChildTimeout(AssertionError):
    pass


def run_child(cmd, **kw):
    kw.setdefault("capture_output", True)
    kw.setdefault("text", True)
    kw.setdefault("stdin", subprocess.DEVNULL)
    kw.setdefault("timeout", CHILD_TIMEOUT)
    try:
        return subprocess.run(cmd, **kw)
    except subprocess.TimeoutExpired as e:
        out = e.stdout or ""
        if isinstance(out, bytes):
            out = out.decode("utf-8", "replace")
        raise ChildTimeout("no exit after %g s: %s\n--- last output ---\n%s"
                           % (kw["timeout"], " ".join(map(str, cmd)), out[-2000:])) from None


def _work_root():
    env = os.environ.get("NETGD_TEST_WORK")
    if env:
        os.makedirs(env, exist_ok=True)
        return os.path.abspath(env), False
    return tempfile.mkdtemp(prefix="ngm-", dir=os.path.realpath(tempfile.gettempdir())), True


WORK_ROOT, _ours = _work_root()
if _ours:
    atexit.register(shutil.rmtree, WORK_ROOT, True)

EXE = os.path.join(ROOT, "build", "menu_host.exe" if os.name == "nt" else "menu_host")
RECV_EXE = os.path.join(RECV, "build", "test_host.exe" if os.name == "nt" else "test_host")

MENU_SRCS = ["src/nm_text.c", "src/nm_slot.c", "src/nm_cfgparse.c", "src/nm_ipbin.c", "src/nm_list.c", "src/nm_input.c"]
RECV_SRCS = []          # filled in by the steps that link the netgd-recv core (nm_scan, nm_launch, ngtp_srv)
TEST_SRCS = ["test/menu_host.c"]
SRCS = MENU_SRCS + TEST_SRCS


def build():
    os.makedirs(os.path.join(ROOT, "build"), exist_ok=True)
    srcs = [os.path.join(ROOT, s) for s in SRCS] + [os.path.join(RECV, s) for s in RECV_SRCS]
    newest = max(os.path.getmtime(s) for s in srcs)
    for d in (os.path.join(ROOT, "src"), os.path.join(ROOT, "test"), os.path.join(RECV, "src"), os.path.join(RECV, "test")):
        for f in os.listdir(d):
            if f.endswith(".h"):
                newest = max(newest, os.path.getmtime(os.path.join(d, f)))
    if os.path.exists(EXE) and os.path.getmtime(EXE) >= newest:
        return
    cmd = ["gcc", "-O1", "-g", "-Wall", "-Werror", "-Wno-unused-function", "-Wno-format-truncation", "-DNG_HOST",
           "-Isrc", "-I../netgd-recv/src", "-I../netgd-recv/third_party/fatfs", "-I../netgd-recv/test", "-Itest",
           "-o", EXE] + SRCS + ["../netgd-recv/" + s for s in RECV_SRCS]
    r = run_child(cmd, cwd=ROOT, timeout=max(CHILD_TIMEOUT, 600))
    if r.returncode:
        raise RuntimeError("build failed:\n" + r.stdout + r.stderr)


class Base(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        build()


class TestSelftest(Base):
    def test_selftest(self):
        r = run_child([EXE, "selftest"])
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("SELFTEST OK", r.stdout)


if __name__ == "__main__":
    print("scratch: %s; child limit %g s" % (WORK_ROOT, CHILD_TIMEOUT), file=sys.stderr)
    unittest.main(verbosity=2)
