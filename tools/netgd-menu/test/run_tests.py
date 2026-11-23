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
import hashlib
import os
import re
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import zlib


def _refuse_msys_python_with_native_gcc():
    """Stop early when an MSYS/Cygwin Python (os.name == "posix") would drive a native Windows gcc.

    That Python makes POSIX paths (/tmp/..., /h/...) and names the test program without .exe, and its runtime rewrites every
    argument that starts with "/" before a native program sees it ("/02/DISC.GDI" becomes "C:/<msys root>/02/DISC.GDI"), so
    nearly every test fails with misleading messages.  A Cygwin Python with a Cygwin gcc is consistent and is let through.
    """
    if sys.platform not in ("cygwin", "msys"):
        return
    try:
        target = subprocess.run(["gcc", "-dumpmachine"], capture_output=True, text=True, stdin=subprocess.DEVNULL,
                                timeout=60).stdout
    except Exception:
        return
    if "mingw" in target or "windows" in target:
        sys.exit("run_tests.py: this Python is an MSYS/Cygwin build (%s, sys.platform=%s) but gcc is a native Windows "
                 "compiler (%s).\nRun the tests with a native Windows Python instead, e.g.  py -3 %s"
                 % (sys.executable, sys.platform, target.strip(), os.path.relpath(os.path.abspath(__file__))))


_refuse_msys_python_with_native_gcc()

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)                      # tools/netgd-menu
RECV = os.path.join(os.path.dirname(ROOT), "netgd-recv")
sys.path.insert(0, os.path.join(os.path.dirname(ROOT), "netgd-send"))     # netgd_send.manifest builds the manifests of the tests
sys.path.insert(0, os.path.join(RECV, "test"))                             # fatcheck.py

from netgd_send import art as artmod  # noqa: E402
from netgd_send import manifest as mf  # noqa: E402
import fatcheck  # noqa: E402

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
# the same core built with -DNM_HANDOFF_TUR=1: the menu polls TEST UNIT READY before the hand-off (R104 finding 3); EXE is the default
# build, which hands over right after 52 82
EXE_TUR = os.path.join(ROOT, "build", "menu_host_tur.exe" if os.name == "nt" else "menu_host_tur")
RECV_EXE = os.path.join(RECV, "build", "test_host.exe" if os.name == "nt" else "test_host")

MENU_SRCS = ["src/nm_text.c", "src/nm_slot.c", "src/nm_cfgparse.c", "src/nm_ipbin.c", "src/nm_list.c", "src/nm_input.c",
             "src/nm_screen.c", "src/nm_scan.c", "src/nm_launch.c", "src/nm_disc.c", "src/nm_art.c", "src/ngtp_frame.c", "src/ngtp_srv.c", "src/nm_ui.c",
             "src/nm_netup.c"]
# the netgd-recv core and its test doubles (a disk image file, a packet-level firmware model); never main.c, xport_pc.c,
# dev_kos.c, fwxfer_kos.c, plat_kos.c
RECV_SRCS = ["src/crc32.c", "src/manifest.c", "src/ngblk.c", "src/fwctl.c", "src/ngfs.c", "src/recv_core.c", "src/diskio.c",
             "third_party/fatfs/ff.c", "third_party/fatfs/ffunicode.c", "third_party/fatfs/ffsystem.c",
             "test/dev_file.c", "test/fwctl_sim.c"]
TEST_SRCS = ["test/menu_host.c", "test/plat_host.c", "test/net_host.c"]
SRCS = MENU_SRCS + TEST_SRCS


def build():
    os.makedirs(os.path.join(ROOT, "build"), exist_ok=True)
    srcs = [os.path.join(ROOT, s) for s in SRCS] + [os.path.join(RECV, s) for s in RECV_SRCS]
    newest = max(os.path.getmtime(s) for s in srcs)
    for d in (os.path.join(ROOT, "src"), os.path.join(ROOT, "test"), os.path.join(RECV, "src"), os.path.join(RECV, "test")):
        for f in os.listdir(d):
            if f.endswith(".h"):
                newest = max(newest, os.path.getmtime(os.path.join(d, f)))
    for exe, extra in ((EXE, []), (EXE_TUR, ["-DNM_HANDOFF_TUR=1"])):
        if os.path.exists(exe) and os.path.getmtime(exe) >= newest:
            continue
        cmd = ["gcc", "-O1", "-g", "-Wall", "-Werror", "-Wno-unused-function", "-Wno-format-truncation", "-DNG_HOST"] + extra + [
               "-Isrc", "-I../netgd-recv/src", "-I../netgd-recv/third_party/fatfs", "-I../netgd-recv/test", "-Itest",
               "-o", exe] + SRCS + ["../netgd-recv/" + s for s in RECV_SRCS] + (["-lws2_32"] if os.name == "nt" else [])
        r = run_child(cmd, cwd=ROOT, timeout=max(CHILD_TIMEOUT, 600))
        if r.returncode:
            raise RuntimeError("build failed:" + chr(10) + r.stdout + r.stderr)
    build_recv_tool()


RECV_TOOL_SRCS = ["src/crc32.c", "src/manifest.c", "src/ngblk.c", "src/fwctl.c", "src/ngfs.c", "src/recv_core.c",
                  "src/xport_pc.c", "src/diskio.c", "third_party/fatfs/ff.c", "third_party/fatfs/ffunicode.c",
                  "third_party/fatfs/ffsystem.c", "test/dev_file.c", "test/fwctl_sim.c", "test/plat_host.c", "test/test_host.c"]


def build_recv_tool():
    """netgd-recv's test_host (mkimage / addfile / mkdirs / dump make and read the card images)."""
    srcs = [os.path.join(RECV, s) for s in RECV_TOOL_SRCS]
    newest = max(os.path.getmtime(s) for s in srcs)
    for d in ("src", "test"):
        for f in os.listdir(os.path.join(RECV, d)):
            if f.endswith(".h"):
                newest = max(newest, os.path.getmtime(os.path.join(RECV, d, f)))
    if os.path.exists(RECV_EXE) and os.path.getmtime(RECV_EXE) >= newest:
        return
    os.makedirs(os.path.join(RECV, "build"), exist_ok=True)
    cmd = ["gcc", "-O1", "-g", "-Wall", "-Wno-unused-function", "-Wno-format-truncation", "-DNG_HOST", "-Isrc",
           "-Ithird_party/fatfs", "-Itest", "-o", RECV_EXE] + RECV_TOOL_SRCS
    r = run_child(cmd, cwd=RECV, timeout=max(CHILD_TIMEOUT, 600))
    if r.returncode:
        raise RuntimeError("build of netgd-recv test_host failed:\n" + r.stdout + r.stderr)


class Base(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        build()


class TestSelftest(Base):
    def test_selftest(self):
        for exe in (EXE, EXE_TUR):
            r = run_child([exe, "selftest"])
            self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
            self.assertIn("SELFTEST OK", r.stdout)


GDI3 = (b"3\r\n1 0 4 2352 track01.bin 0\r\n2 450 0 2352 track02.raw 0\r\n"
        b"3 45000 4 2048 track03.bin 0\r\n")


def ipbin(title):
    """a 2048-byte first sector of track 3 with an IP.BIN header and this title"""
    b = bytearray(2048)
    b[0:16] = b"SEGA SEGAKATANA "
    b[0x80:0x100] = title.encode("ascii").ljust(128)
    return bytes(b)


def exp_row(sel, number, title, kind="", star=False):
    """one list row as nm_ui.c draws it: cursor, number, title (38), kind, default mark"""
    return ((">" if sel else " ") + ("%04d" % number if number else "    ") + "  " + title.ljust(38) + " "
            + kind.ljust(3) + " " + ("*" if star else " ")).rstrip()


class UiBase(Base):
    """A card image built with netgd-recv's test_host, and the menu core run on it with a script."""
    SECTORS = 100000

    def setUp(self):
        self.work = os.path.join(WORK_ROOT, "_w_" + self.id().split(".")[-1][:40])
        shutil.rmtree(self.work, ignore_errors=True)
        os.makedirs(self.work)
        self.addCleanup(shutil.rmtree, self.work, True)

    def card(self, files, name="card.img", sectors=None):
        """files: {"/02/DISC.GDI": bytes, ...}; the folders are made as needed"""
        sectors = sectors or self.SECTORS
        img = os.path.join(self.work, name)
        r = run_child([RECV_EXE, "mkimage", img, str(sectors), "512"])
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        for i, (path, data) in enumerate(sorted(files.items())):
            src = os.path.join(self.work, "src%d.bin" % i)
            with open(src, "wb") as f:
                f.write(data)
            r = run_child([RECV_EXE, "addfile", img, str(sectors), path, src])
            self.assertEqual(r.returncode, 0, "addfile %s: %s" % (path, r.stdout + r.stderr))
        return img

    exe = EXE                    # a subclass can run the TEST UNIT READY build instead (EXE_TUR)

    def ui(self, img, script, *opts, sectors=None):
        cmd = [self.exe, "ui", img, str(sectors or self.SECTORS), "--script", script] + list(opts)
        r = run_child(cmd)
        return UiRun(r.stdout, r.returncode)


class UiRun(object):
    def __init__(self, text, rc):
        self.text, self.rc = text, rc
        self.packets = [l[3:] for l in text.splitlines() if l.startswith("FW ")]
        m = re.search(r"^SIM (.*)$", text, re.M)
        self.sim = dict(re.findall(r"(\w+)=(\S*)", m.group(1))) if m else {}
        self.screens = []
        for blk in re.finditer(r"^SCREEN (.*)\n((?:S\d\d\|.*\n){20})END$", text, re.M):
            head = dict(re.findall(r"(\w+)=(-?\d+)", blk.group(1)))
            rows = [l[4:] for l in blk.group(2).splitlines()]
            self.screens.append((head, rows))
        self.handoff = re.findall(r"^HANDOFF (\w+)$", text, re.M)

    def n(self, key):
        return int(self.sim[key])

    @property
    def head(self):
        return self.screens[-1][0]

    @property
    def rows(self):
        return self.screens[-1][1]


class TestListAndInfo(UiBase):
    def library(self):
        return {
            "/02/DISC.GDI": GDI3, "/02/GAME.CFG": b'title = "Crazy Taxi"\r\n',
            "/03/DISC.CDI": b"cdi", "/03/NAME.TXT": b"Name From TXT\r\nsecond\r\n",
            "/04/DISC.GDI": GDI3, "/04/TRACK03.BIN": ipbin("  TEST  GAME   FOUR"),
            "/05/DISC.NGP": b"x",
            "/06/README.TXT": b"nothing to boot",
            "/MYGAME/DISC.GDI": GDI3,
            "/01/MENU.CDI": b"menu",
        }

    def test_l1_l2_list_titles_states_and_no_writes(self):
        img = self.card(self.library())
        run = self.ui(img, "scan dump", "--slots", "2,3,4")
        self.assertEqual(run.rc, 0, run.text)
        self.assertEqual(run.head["count"], "6", run.text)                      # 01 is the menu: not listed
        want = [exp_row(True, 2, "Crazy Taxi", "GDI"),
                exp_row(False, 3, "Name From TXT", "CDI"),
                exp_row(False, 4, "TEST GAME FOUR", "GDI"),
                exp_row(False, 5, "05 (incomplete)"),
                exp_row(False, 6, "(empty)"),
                exp_row(False, 0, "MYGAME", "GDI")]
        self.assertEqual(run.rows[2:8], want, "\n".join(run.rows))
        self.assertEqual(run.rows[8:17], [""] * 9)
        self.assertTrue(run.rows[0].startswith("NetGD Menu 0.1.0"), run.rows[0])
        self.assertTrue(run.rows[0].endswith("fw 5.20.0"), run.rows[0])
        self.assertEqual(run.rows[17], "6 games  Reset: stays in the game (reset_goto off)")
        # L2: reading only
        self.assertEqual((run.n("cmd_writes"), run.n("sector_writes"), run.n("begin"), run.n("unsess")), (0, 0, 0, 0), run.text)
        self.assertEqual(run.n("violations"), 0)
        # the stock-safe gate: the GDEMU version (52 00) first, then the NetGD version (E0 03), nothing else
        self.assertEqual([p[:5] for p in run.packets], ["52 00", "E0 03"], run.text)

    def test_default_image_mark_and_duplicate_numbers(self):
        files = {"/02/DISC.GDI": GDI3, "/002/DISC.GDI": GDI3, "/03/DISC.GDI": GDI3, "/GDEMU.CFG": b"image=03\n"}
        img = self.card(files)
        run = self.ui(img, "scan dump", "--slots", "2,3")
        self.assertEqual(run.rows[2:4], [exp_row(True, 2, "02", "GDI"), exp_row(False, 3, "03", "GDI", star=True)], "\n".join(run.rows))
        self.assertEqual(run.rows[4], "")
        self.assertEqual(run.rows[17], "2 games  duplicate number 02")

    def test_l8_no_firmware(self):
        img = self.card(self.library())
        run = self.ui(img, "step step dump S", "--nofw")
        self.assertEqual(run.head["state"], "1", run.text)                       # NM_S_NOFW
        self.assertIn("NetGD firmware not found", "\n".join(run.rows))
        self.assertEqual(run.n("rcmds"), 0, run.text)                            # the SD card was not touched
        self.assertEqual(run.n("cmd_writes"), 0)
        self.assertEqual(run.handoff, ["bios"])                                  # START: back to the BIOS

    def test_l9_no_sd_card(self):
        blank = os.path.join(self.work, "blank.img")
        with open(blank, "wb") as f:
            f.write(b"\0" * 512 * self.SECTORS)
        run = self.ui(blank, "step step dump")
        self.assertEqual(run.head["state"], "2", run.text)                       # NM_S_NOSD
        self.assertIn("SD card: no MBR (partition table) on the card", "\n".join(run.rows))
        # a card whose first partition is not FAT32
        img = self.card({"/02/DISC.GDI": GDI3})
        with open(img, "r+b") as f:
            f.seek(446 + 4)
            f.write(b"\x07")
        run = self.ui(img, "step step dump")
        self.assertIn("SD card: first partition type 0x07 is not FAT32", "\n".join(run.rows))
        run = self.ui(img, "step step dump", "--noimg")
        self.assertIn("SD card: no disk", "\n".join(run.rows))
        self.assertEqual(run.n("cmd_writes"), 0)

    def test_l10_read_bit_flips(self):
        img = self.card(self.library())
        base = self.ui(img, "scan dump", "--slots", "2,3,4")
        for k in (2, 3, 7, 20):
            run = self.ui(img, "scan dump", "--slots", "2,3,4", "--rflip", str(k))
            self.assertEqual(run.rows[2:8], base.rows[2:8], "flip at read %d\n%s" % (k, "\n".join(run.rows)))
        # reads go bad for good after the folder list was made: the scan stops, the list stays, nothing hangs
        run = self.ui(img, "step step rflipall scan dump", "--slots", "2,3,4")
        self.assertEqual(run.rc, 0, run.text)
        self.assertEqual(run.head["scan"], "3", run.text)                        # NM_SCAN_ABORTED
        self.assertTrue(run.rows[17].startswith("SD read error (scan stopped)"), run.rows[17])
        self.assertEqual(run.head["count"], "6")
        self.assertEqual(run.n("cmd_writes"), 0)

    def test_l11_many_folders_paging_and_input_while_scanning(self):
        img = self.card({})
        r = run_child([RECV_EXE, "mkdirs", img, str(self.SECTORS), "0", "300"])      # folders 0000 .. 0299
        self.assertEqual(r.returncode, 0, r.stdout)
        # input is accepted while the scan is still running (one folder per frame)
        run = self.ui(img, "step step D D D dump")
        self.assertEqual(run.head["scan"], "1", run.text)                        # still scanning
        self.assertEqual(run.head["cursor"], "3")
        self.assertIn("Scanning ", run.rows[17])
        run = self.ui(img, "scan dump")
        self.assertEqual(run.head["count"], "298", run.text)         # 0000 is a named folder without an image, 0001 is the menu
        run = self.ui(img, "scan U dump")
        self.assertEqual((run.head["cursor"], run.head["top"]), ("0", "0"))      # no wrap at the top
        run = self.ui(img, "scan RT dump")
        self.assertEqual((run.head["cursor"], run.head["top"]), ("15", "15"))
        self.assertTrue(run.rows[2].startswith(">") and "0017" in run.rows[2], run.rows[2])
        run = self.ui(img, "scan RT RT RT RT RT RT RT RT RT RT RT RT RT RT RT RT RT RT RT RT RT dump")
        self.assertEqual(run.head["cursor"], "297", run.text)                    # clamped at the end
        self.assertEqual(int(run.head["top"]), 298 - 15)
        run = self.ui(img, "scan hold:D:1000 dump")
        self.assertEqual(run.head["cursor"], "10", run.text)                      # 1 + (1000 - 300) / 80 + 1 steps of auto-repeat
        self.assertEqual(run.n("cmd_writes"), 0)

    def test_l12_reset_goto_and_info_screen(self):
        files = {"/02/DISC.GDI": GDI3, "/GDEMU.INI": b"reset_goto=1\r\nopen_time=700\r\n"}
        run = self.ui(self.card(files), "scan dump X dump", "--slots", "2")
        self.assertEqual(run.screens[0][1][17], "1 game  Reset: back to this menu")
        info = "\n".join(run.screens[1][1])
        self.assertEqual(run.screens[1][0]["state"], "4")                        # NM_S_INFO
        self.assertIn("Reset: back to this menu", info)
        self.assertIn("GDEMU.INI      found", info)
        self.assertIn("open_time 700 ms  detect_time 250 ms (default)", info)
        self.assertIn("Hand-off      BIOS menu", info)
        self.assertIn("Firmware      sim-0.0", info)
        run = self.ui(self.card({"/02/DISC.GDI": GDI3, "/GDEMU.INI": b"reset_goto=7\n"}, "b.img"), "scan dump", "--slots", "2")
        self.assertEqual(run.rows[17], "1 game  Reset: goes to 7")
        run = self.ui(self.card({"/02/DISC.GDI": GDI3}, "c.img"), "scan dump", "--slots", "2")
        self.assertEqual(run.rows[17], "1 game  Reset: stays in the game (reset_goto off)")
        # A on the info screen switches the hand-off, B goes back
        run = self.ui(self.card({"/02/DISC.GDI": GDI3}, "d.img"), "scan X A dump B dump", "--slots", "2")
        self.assertIn("Hand-off      restart the console", "\n".join(run.screens[0][1]))
        self.assertEqual(run.screens[1][0]["state"], "3")

    def test_info_screen_stack_line(self):
        # A stack-hw firmware (VERSION caps bit3): X reads E0 09 once and the info screen shows the high-water mark.
        img = self.card({"/02/DISC.GDI": GDI3})
        run = self.ui(img, "scan X dump B X dump", "--slots", "2", "--caps", "15")
        for head, rows in run.screens:
            self.assertEqual(head["state"], "4")
            self.assertEqual(rows[9].rstrip(), " Stack         4816 of 7128 bytes used", "\n".join(rows))
        self.assertEqual(sum(1 for p in run.packets if p.startswith("E0 09")), 2, run.text)   # once per visit
        # Any other firmware: no line, and E0 09 is never sent (it would be ILLEGAL REQUEST).
        run = self.ui(self.card({"/02/DISC.GDI": GDI3}, "b.img"), "scan X dump", "--slots", "2")
        self.assertNotIn("Stack", "\n".join(run.rows))
        self.assertFalse([p for p in run.packets if p.startswith("E0 09")], run.text)

    def test_no_controller_is_reported(self):
        img = self.card({"/02/DISC.GDI": GDI3})
        run = self.ui(img, "scan dump", "--slots", "2", "--nocontroller")
        self.assertEqual(run.rows[17], "No controller")


NL = chr(10)


class TestNetStart(UiBase):
    """R104 finding 1: the screen before the blocking network start, and the second Y retrying the address"""

    def lib(self):
        return self.card({"/02/DISC.GDI": GDI3})

    def test_starting_network_is_on_the_screen_before_the_call(self):
        r = self.ui(self.lib(), "scan Y", "--slots", "2")
        m = re.search(r"^NETUP call=1 screen3=\|(.*)\|$", r.text, re.M)
        self.assertIsNotNone(m, r.text)
        self.assertIn("Starting network", m.group(1))

    def test_no_bba_is_reported_and_final(self):
        # KOS: no BBA is DEAD, like no link: Y:Network goes away and a second Y does not call nm_net_up
        r = self.ui(self.lib(), "scan Y dump t:100 Y dump", "--slots", "2", "--nobba")
        self.assertEqual(len(re.findall(r"^NETUP call=", r.text, re.M)), 1, r.text)
        self.assertIn("BBA not found", NL.join(r.screens[0][1]))
        self.assertIn("BBA not found", NL.join(r.rows))
        self.assertNotIn("Y:Network", NL.join(r.rows))

    def test_no_address_is_final_and_y_does_not_start_again(self):
        r = self.ui(self.lib(), "scan Y dump t:100 Y dump", "--slots", "2", "--noaddr", "1")
        self.assertEqual(len(re.findall(r"^NETUP call=", r.text, re.M)), 1, r.text)
        self.assertIn("No IP address", NL.join(r.screens[0][1]))
        self.assertIn("No IP address", NL.join(r.rows))
        self.assertNotIn("Y:Network", NL.join(r.rows))
        self.assertNotIn("127.0.0.1", NL.join(r.rows))

    def test_transfer_screen_ip_and_window(self):
        r = self.ui(self.lib(), "scan Y dump", "--slots", "2")
        self.assertEqual(len(re.findall(r"^NETUP call=", r.text, re.M)), 1, r.text)
        self.assertIn("127.0.0.1", NL.join(r.rows))
        self.assertIn("window 16 KiB x 1 (stock tcp)", NL.join(r.rows))

    def test_no_dhcp_text_and_steps_on_the_screen(self):
        r = self.ui(self.lib(), "scan Y", "--slots", "2")
        m = re.search(r"^NETLOG screen5=\|(.*)\| screen7=\|(.*)\|$", r.text, re.M)
        self.assertIsNotNone(m, r.text)
        self.assertIn("static IP, no DHCP", m.group(1))
        self.assertEqual(m.group(2), "  host: bring-up step")          # nm_net_log lines are drawn (and shown) while it runs

    def test_no_link_is_final_and_y_does_not_start_again(self):
        # KOS 2.3 aborts on a second net_init after one that returned -1 (bba_rx_thread assert): one start, then a final
        # message, no Y:Network hint, and a second Y only shows the message again (nm_net_up is not called)
        r = self.ui(self.lib(), "scan Y dump t:100 Y dump", "--slots", "2", "--nolink", "1")
        self.assertEqual(len(re.findall(r"^NETUP call=", r.text, re.M)), 1, r.text)
        first = NL.join(r.screens[0][1])
        self.assertIn("No link: check the cable and restart the console", first)
        self.assertNotIn("Y: try again", first)
        self.assertNotIn("Y:Network", first)
        self.assertIn("A:Start  X:Info  START:BIOS menu", first)
        self.assertIn("No link: check the cable and restart the console", NL.join(r.rows))
        self.assertNotIn("127.0.0.1", NL.join(r.rows))
        self.assertNotIn("Starting network", NL.join(r.rows))

    def test_y_network_is_offered_before_a_failure(self):
        r = self.ui(self.lib(), "scan dump", "--slots", "2", "--nolink", "1")
        self.assertIn("Y:Network", NL.join(r.rows))


class TestStockGate(UiBase):
    """The stock-safe write gate (as netgd-t2's t2_gate): 52 00 first; E0 03 (and every other E0) only when 52 00 says 5.20.0"""

    def lib(self):
        return self.card({"/02/DISC.GDI": GDI3})

    def test_stock_gdemu_gets_52_00_only(self):
        run = self.ui(self.lib(), "step step dump", "--stock", "--slots", "2")
        self.assertEqual(run.head["state"], "1", run.text)                      # NM_S_NOFW
        self.assertEqual([p[:5] for p in run.packets], ["52 00"], run.text)
        self.assertEqual(run.packets[0], "52 00 00 00 00 00 00 00 00 00 00 00")   # byte for byte the CDB of the stock record
        self.assertFalse([p for p in run.packets if p.startswith("E0")], run.text)
        rows = NL.join(run.rows)
        self.assertIn("NetGD firmware not found", rows)
        self.assertIn("GDEMU 5.20.5 is not NetGD: no NetGD command sent.", rows)
        self.assertTrue(run.rows[0].endswith("fw 5.20.5"), run.rows[0])
        self.assertEqual(run.n("rcmds"), 0, run.text)                           # the SD card was not touched

    def test_stock_with_unit_attention_and_start(self):
        run = self.ui(self.lib(), "step step dump S Y X A", "--stock", "--ua", "--slots", "2")
        self.assertEqual([p[:5] for p in run.packets], ["52 00", "52 00"], run.text)   # repeated once on UNIT ATTENTION
        self.assertEqual(run.handoff, ["bios"], run.text)
        self.assertFalse([p for p in run.packets if p.startswith("E0")], run.text)
        self.assertNotIn("NETUP", run.text)

    def test_other_gdemu_version_gets_no_e0(self):
        for v in ("0x05200500", "0x05210000", "0x06000000"):
            run = self.ui(self.lib(), "step step dump Y", "--gdemu-version", v, "--slots", "2")
            self.assertEqual(run.head["state"], "1", run.text)
            self.assertEqual([p[:5] for p in run.packets], ["52 00"], (v, run.text))
            self.assertIn("is not NetGD", NL.join(run.rows))

    def test_no_answer_to_52_00(self):
        run = self.ui(self.lib(), "step step dump", "--nofw")
        self.assertEqual(run.packets, ["(no answer)"], run.text)                 # one packet (52 00), no E0 after it
        self.assertEqual(run.n("packets"), 1)
        self.assertIn("did not answer 52 00", NL.join(run.rows))

    def test_netgd_is_asked_e0_03_after_52_00(self):
        run = self.ui(self.lib(), "scan dump", "--slots", "2")
        self.assertEqual([p[:5] for p in run.packets[:2]], ["52 00", "E0 03"], run.text)
        self.assertEqual(run.head["state"], "3", run.text)                      # NM_S_LIST


class TestLaunchDefault(UiBase):
    """R104 finding 3, variant A (the default build, NM_HANDOFF_TUR=0): READY is taken from the success of `52 82`; no TEST UNIT
    READY is sent, so the UNIT ATTENTION stays for the BIOS."""

    def games(self):
        return self.card({"/02/DISC.GDI": GDI3, "/03/DISC.CDI": b"cdi"})

    def test_no_test_unit_ready_before_the_hand_off(self):
        run = self.ui(self.games(), "scan D A t:2000", "--slots", "2,3")
        self.assertEqual(run.rc, 0, run.text)
        self.assertEqual([p[:11] for p in run.packets[:2]], ["52 00 00 00", "E0 03 00 00"])
        self.assertEqual([p[:2] for p in run.packets[2:]], ["52"], run.text)     # only the mount: no 00 (TEST UNIT READY)
        self.assertEqual(run.packets[2][:11], "52 82 03 00")
        self.assertEqual(run.handoff, ["bios"], run.text)
        self.assertIn("H04|  Starting 03 ...", run.text)

    def test_a_drive_that_would_never_be_ready_does_not_hold_the_hand_off(self):
        run = self.ui(self.games(), "scan D A t:2000", "--slots", "2,3", "--notready", "1000000")
        self.assertEqual(run.handoff, ["bios"], run.text)
        self.assertEqual([p for p in run.packets if p.startswith("00 ")], [])

    def test_unit_attention_is_left_for_the_bios(self):
        run = self.ui(self.games(), "scan D ua A t:2000", "--slots", "2,3")
        self.assertEqual(run.handoff, ["bios"], run.text)
        mounts = [p for p in run.packets if p.startswith("52 82")]
        self.assertEqual(len(mounts), 2, run.text)                                # the mount is still repeated once (not executed)
        self.assertEqual([p for p in run.packets if p.startswith("00 ")], [])

    def test_by_name_and_reboot_variants(self):
        run = self.ui(self.card({"/02/DISC.GDI": GDI3, "/SECOND_GAME_X/DISC.GDI": GDI3}), "scan X A B D A t:2000", "--slots", "2")
        self.assertEqual(run.handoff, ["reboot"], run.text)
        self.assertEqual([p for p in run.packets if p.startswith("00 ")], [])
        self.assertEqual(run.n("named_mounts"), 1)

    def test_no_image_still_shows_the_dialog(self):
        run = self.ui(self.games(), "scan A dump", "--slots", "3")               # 02 is not mountable in the model
        self.assertEqual(run.handoff, [])
        self.assertEqual(run.screens[0][0]["state"], "6", run.text)


class TestLaunch(UiBase):
    """Design 11.2 L3-L7, L13, L14: the 0x52 mount, the READY wait and the hand-off.  This is the NM_HANDOFF_TUR=1 build (the
    menu polls TEST UNIT READY); TestLaunchDefault below is the default build (hand over right after 52 82)."""
    exe = EXE_TUR

    def games(self, extra=None):
        files = {"/02/DISC.GDI": GDI3, "/03/DISC.CDI": b"cdi", "/MYGAME/DISC.GDI": GDI3}
        files.update(extra or {})
        return self.card(files)

    def fw(self, run):
        """the firmware packets after the two of the start-up (E0 03, 52 00), as strings"""
        return run.packets[2:]

    def test_l3_launch_by_number(self):
        run = self.ui(self.games(), "scan D A t:2000", "--slots", "2,3")
        self.assertEqual(run.rc, 0, run.text)
        self.assertEqual([p[:11] for p in run.packets[:2]], ["52 00 00 00", "E0 03 00 00"])
        self.assertEqual(self.fw(run)[0][:11], "52 82 03 00")                    # 52 82 lo hi
        self.assertEqual([p[:2] for p in self.fw(run)], ["52", "00", "00"])      # READY twice in a row, then nothing more
        self.assertEqual(run.handoff, ["bios"], run.text)
        self.assertIn("H04|  Starting 03 ...", run.text)                         # the screen at the hand-off
        self.assertEqual((run.n("gdemu_mounts"), run.n("cmd_writes"), run.n("begin")), (1, 0, 0))

    def test_l3_notready_polls_before_ready(self):
        run = self.ui(self.games(), "scan D A t:2000", "--slots", "2,3", "--notready", "5")
        self.assertEqual(run.handoff, ["bios"], run.text)
        tur = [p for p in self.fw(run) if p.startswith("00 ")]
        self.assertEqual(len(tur), 5 + 2, run.text)                              # five NOT READY, then two READY

    def test_l4_launch_by_name(self):
        files = {"/02/DISC.GDI": GDI3, "/SECOND_GAME_X/DISC.GDI": GDI3}
        run = self.ui(self.card(files), "scan D A t:2000", "--slots", "2")
        self.assertEqual(run.handoff, ["bios"], run.text)
        fw = self.fw(run)
        self.assertEqual(fw[0], "E0 05 00 53 45 43 4F 4E 44 5F 47 41")         # SECOND_GA
        self.assertEqual(fw[1], "E0 05 01 4D 45 5F 58 00 00 00 00 00")         # ME_X + NUL
        self.assertEqual(fw[2], "E0 02 00 00 00 00 00 00 00 00 00 00")         # mount the staged name
        self.assertEqual(run.n("named_mounts"), 1)
        self.assertIn("Starting SECOND_GAME_X ...", run.text)

    def test_l5_no_image(self):
        run = self.ui(self.games(), "scan A dump A t:100 dump", "--slots", "3")   # 02 is not mountable in the model
        self.assertEqual(run.rc, 0, run.text)
        self.assertEqual(run.handoff, [])
        self.assertEqual(run.screens[0][0]["state"], "6", run.text)               # NM_S_DIALOG
        self.assertIn("No image in folder 02", "\n".join(run.screens[0][1]))
        self.assertEqual(run.screens[1][0]["state"], "3")                         # back in the list
        self.assertEqual(run.screens[1][1][2][:1], ">")
        self.assertEqual(run.n("gdemu_mounts"), 0)

    def test_l5_empty_and_incomplete_folders_are_not_started(self):
        files = {"/02/README.TXT": b"x", "/03/DISC.NGP": b"x"}
        run = self.ui(self.card(files), "scan A dump B D A dump", "--slots", "2,3")
        self.assertIn("No image in folder 02", "\n".join(run.screens[0][1]))
        self.assertIn("Image in folder 03 is incomplete", "\n".join(run.screens[1][1]))
        self.assertEqual([p for p in self.fw(run) if p.startswith("52 8")], [])  # nothing was sent to the firmware

    def test_l6_drive_never_ready(self):
        files = {"/02/DISC.GDI": GDI3, "/GDEMU.INI": b"open_time=700\n"}
        run = self.ui(self.card(files), "scan A t:10300 dump t:600 dump A t:3000 dump t:9000 dump B dump",
                      "--slots", "2", "--notready", "1000000")
        self.assertEqual(run.rc, 0, run.text)
        self.assertEqual(run.handoff, [])
        self.assertEqual(run.screens[0][0]["state"], "5", run.text)               # NM_S_LAUNCH: 10 s + open_time not over yet
        self.assertIn("Waiting for the drive ... 10 s", "\n".join(run.screens[0][1]))
        self.assertEqual(run.screens[1][0]["state"], "6")                         # the dialog
        self.assertIn("Drive not ready", "\n".join(run.screens[1][1]))
        self.assertIn("did not become READY in 10 s", "\n".join(run.screens[1][1]))
        self.assertEqual(run.screens[2][0]["state"], "5")                         # A: waiting again, without mounting again
        self.assertEqual(run.screens[3][0]["state"], "6")                         # the second wait ran out as well
        self.assertEqual(run.screens[4][0]["state"], "3")                         # B: the list
        self.assertEqual(run.n("gdemu_mounts"), 1)                                # the mount was not repeated

    def test_l7_unit_attention(self):
        run = self.ui(self.games(), "scan D ua A t:2000", "--slots", "2,3")
        self.assertEqual(run.handoff, ["bios"], run.text)
        mounts = [p for p in self.fw(run) if p.startswith("52 82")]
        self.assertEqual(len(mounts), 2, run.text)                                # the first was not executed: sent again
        # the UNIT ATTENTION the mount raises is not repeated away inside TEST UNIT READY: the wait handles it
        tur = [p for p in self.fw(run) if p.startswith("00 ")]
        self.assertEqual(len(tur), 3, run.text)

    def test_l13_reboot_hand_off(self):
        run = self.ui(self.games(), "scan X A B A t:2000", "--slots", "2,3")
        self.assertEqual(run.handoff, ["reboot"], run.text)

    def test_l14_start_confirms_and_never_mounts(self):
        img = self.games()
        run = self.ui(img, "scan S dump B dump S A t:500", "--slots", "2,3")
        self.assertEqual(run.screens[0][0]["state"], "7", run.text)               # NM_S_CONFIRM_BIOS
        self.assertIn("Go back to the BIOS menu?", "\n".join(run.screens[0][1]))
        self.assertEqual(run.screens[1][0]["state"], "3")                         # B: stays
        self.assertEqual(run.handoff, ["bios"], run.text)
        self.assertEqual([p for p in run.packets if p.startswith("52 8") or p.startswith("E0 02") or p.startswith("E0 05")], [])
        self.assertEqual((run.n("gdemu_mounts"), run.n("named_mounts"), run.n("cmd_writes")), (0, 0, 0))
        self.assertIn("Returning to the BIOS menu ...", run.text)

    def test_nofw_start_goes_to_the_bios_at_once(self):
        run = self.ui(self.games(), "step step S", "--nofw")
        self.assertEqual(run.handoff, ["bios"])


NGT_HELLO, NGT_STATFS, NGT_LIST, NGT_PUT, NGT_DATA, NGT_DELETE, NGT_VERIFY, NGT_FINISH, NGT_PING = range(1, 10)
NGT_HELLO_OK, NGT_PONG, NGT_LINE, NGT_DONE, NGT_WANT, NGT_ERROR = 0x81, 0x89, 0x90, 0x91, 0x92, 0x7F
CODE = 4821


def ngt_frame(type_, payload=b"", flags=0, seq=1, crc=None):
    hdr = b"NGT1" + bytes([type_, flags, 0, 0]) + struct.pack("<II", seq, len(payload))
    if crc is None:
        crc = zlib.crc32(hdr + payload) & 0xFFFFFFFF
    return hdr + payload + struct.pack("<I", crc)


class NgtClient(object):
    """A raw NGTP client: what the PC side sends, byte for byte (netgd-send's own client is tested in its step)."""

    def __init__(self, port, timeout=30):
        self.sock = socket.create_connection(("127.0.0.1", port), timeout=timeout)
        self.seq = 0

    def close(self):
        try:
            self.sock.close()
        except OSError:
            pass

    def send_raw(self, data):
        self.sock.sendall(data)

    def send(self, type_, payload=b"", flags=0, seq=None):
        self.seq = self.seq + 1 if seq is None else seq
        self.send_raw(ngt_frame(type_, payload, flags, self.seq))
        return self.seq

    def _read(self, n):
        buf = b""
        while len(buf) < n:
            chunk = self.sock.recv(n - len(buf))
            if not chunk:
                raise EOFError("the server closed the connection")
            buf += chunk
        return buf

    def recv(self):
        hdr = self._read(16)
        assert hdr[:4] == b"NGT1", hdr
        n = struct.unpack_from("<I", hdr, 12)[0]
        body = self._read(n + 4)
        payload, crc = body[:n], struct.unpack("<I", body[n:])[0]
        assert zlib.crc32(hdr + payload) & 0xFFFFFFFF == crc, "bad CRC from the server"
        return hdr[4], hdr[5], struct.unpack_from("<I", hdr, 8)[0], payload

    def hello(self, code=CODE, version=1, name=b"rawtest"):
        self.send(NGT_HELLO, struct.pack("<HH", version, code) + name)
        return self.recv()

    def request(self, type_, payload=b"", source=None, abort_after=None):
        """send a request; answer WANTs from `source` ({file index: bytes}); returns (lines, ok, name) or ('ERROR', text)"""
        seq = self.send(type_, payload)
        lines, wants = [], 0
        while True:
            t, fl, sq, p = self.recv()
            if t == NGT_LINE:
                lines.append(p.decode("ascii"))
            elif t == NGT_WANT:
                idx, _z, off, n = struct.unpack("<HHQI", p)
                wants += 1
                data = source[idx][off:off + n]
                flags = 1 if (abort_after is not None and wants >= abort_after) else 0
                self.send_raw(ngt_frame(NGT_DATA, struct.pack("<HHQ", idx, 0, off) + data, flags, sq))
            elif t == NGT_DONE:
                return lines, p[0] == 1, p[1:].decode("ascii")
            elif t == NGT_ERROR:
                return "ERROR", p.decode("ascii")
            else:
                raise AssertionError("unexpected frame %#x" % t)


def free_tcp_udp_port():
    """A port number free for TCP and UDP: menu_host binds both to it.  An ephemeral TCP port is not enough on Windows, where
    excluded port ranges can apply to UDP only (bind then fails with WSAEACCES)."""
    for _ in range(200):
        t = socket.socket()
        u = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        try:
            t.bind(("127.0.0.1", 0))
            port = t.getsockname()[1]
            u.bind(("127.0.0.1", port))
            return port
        except OSError:
            continue
        finally:
            t.close()
            u.close()
    raise AssertionError("no port free for both TCP and UDP")


class NgtServe(object):
    """menu_host serve on a card image: the transfer screen on a real localhost socket"""

    def __init__(self, test, img, *opts, code=CODE, sectors=None):
        self.port = free_tcp_udp_port()
        self.test = test
        cmd = [EXE, "serve", img, str(sectors or UiBase.SECTORS), "--port", str(self.port), "--code", str(code)] + list(opts)
        self.proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding="utf-8", errors="replace",
                                     stdin=subprocess.DEVNULL)
        test.addCleanup(self._kill)
        self.lines = []
        self.ready = threading.Event()
        self.thread = threading.Thread(target=self._pump, daemon=True)
        self.thread.start()
        if not self.ready.wait(60):
            self._kill()
            test.fail("menu_host serve did not start listening:\n" + "\n".join(self.lines))
        self.clients = []

    def _pump(self):
        for line in self.proc.stdout:
            self.lines.append(line.rstrip("\n"))
            if line.startswith("LISTENING") or line.startswith("SERVE-END"):
                self.ready.set()
        self.ready.set()

    def _kill(self):
        for c in getattr(self, "clients", []):
            c.close()
        if self.proc.poll() is None:
            self.proc.kill()
        self.proc.wait()
        self.thread.join(5)
        self.proc.stdout.close()

    def client(self):
        c = NgtClient(self.port)
        self.clients.append(c)
        return c

    def finish(self, timeout=60, check=True):
        """wait for the process to end (the UI left the transfer screen); returns the SIM summary as a dict"""
        try:
            self.proc.wait(timeout)
        except subprocess.TimeoutExpired:
            self._kill()
            self.test.fail("serve did not end:\n" + "\n".join(self.lines))
        self.thread.join(10)
        text = "\n".join(self.lines)
        m = re.search(r"^SIM (.*)$", text, re.M)
        self.text = text
        self.sim = {k: v for k, v in re.findall(r"(\w+)=(\S*)", m.group(1))} if m else {}
        if check and self.sim and "POWERCUT" not in text:
            # in every scenario: no write outside a session, none the firmware would refuse, no mount command while a
            # session was open (N14), and no session left open when the transfer screen is left
            t = self.test
            t.assertEqual((int(self.sim["unsess"]), int(self.sim["violations"]), int(self.sim["mount_in_session"])), (0, 0, 0), text)
            t.assertEqual(int(self.sim["session"]), 0, text)
        return self.sim


def sim_int(sim, key):
    return int(sim[key])


class NetBase(UiBase):
    def pattern(self, n, seed):
        base = bytes((i * 7 + seed * 13) & 0xFF for i in range(251))
        return (base * (n // 251 + 1))[:n]

    def gdi_set(self, folder="02", sizes=(3000, 4096, 5000), name_prefix="track"):
        """-> (manifest text, {index: bytes}, {lowercase name: bytes}); the descriptor (disc.gdi) is file 0"""
        gdi = ("3\r\n1 0 4 2352 %s01.bin 0\r\n2 450 0 2352 %s02.raw 0\r\n3 45000 4 2048 %s03.bin 0\r\n"
               % (name_prefix, name_prefix, name_prefix)).encode()
        datas = [("disc.gdi", gdi)]
        for i, sz in enumerate(sizes):
            datas.append(("%s0%d.%s" % (name_prefix, i + 1, "raw" if i == 1 else "bin"), self.pattern(sz, i + 1)))
        files = [mf.FileEntry(len(d), zlib.crc32(d) & 0xFFFFFFFF, n) for n, d in datas]
        m = mf.Manifest(folder, "gdi", files, "disc.gdi")
        return m.render(), {i: d for i, (n, d) in enumerate(datas)}, {n: d for n, d in datas}

    def put_payload(self, text, mode=0, flags=0, tag="0001"):
        return struct.pack("<BBH", mode, flags, 0) + tag.encode() + text.encode()

    def sha(self, path):
        with open(path, "rb") as f:
            return hashlib.sha1(f.read()).hexdigest()

    def read(self, fc, folder, lower_name):
        real = [k for k in fc.folder_files(folder) if k.lower() == lower_name][0]
        return fc.read_file(folder + "/" + real)

    def clean_fs(self, img):
        fc = fatcheck.Fat32(img)
        self.assertEqual(fc.errors, [])
        self.assertEqual(fc.lost_clusters(), 0)
        return fc


class TestNgtpFrames(NetBase):
    def test_n3_wrong_code_and_three_strikes(self):
        img = self.card({"/02/DISC.GDI": GDI3})
        before = self.sha(img)
        srv = NgtServe(self, img, "--slots", "2")
        c = srv.client()
        t, fl, sq, p = c.hello(code=1111)
        self.assertEqual(t, NGT_ERROR)
        self.assertTrue(p.startswith(b"EAUTH"), p)
        self.assertRaises(EOFError, c.recv)                                     # and the connection is closed
        # the server listens again; the right code works
        c = srv.client()
        t, fl, sq, p = c.hello()
        self.assertEqual(t, NGT_HELLO_OK)
        self.assertEqual(struct.unpack_from("<HHI", p, 0), (1, 0, 65536))        # version, no session yet, max DATA
        self.assertEqual(p[8], 3)                                                # OGDM protocol of the model
        self.assertEqual(p[34:], b"0.1.0")
        c.send(NGT_PING)
        self.assertEqual(c.recv()[0], NGT_PONG)
        # HELLO twice is a protocol error
        c.send(NGT_HELLO, struct.pack("<HH", 1, CODE))
        self.assertTrue(c.recv()[3].startswith(b"ESTATE"))
        c.close()
        # three wrong codes: the code is made anew, the old one no longer works
        for _ in range(3):
            c = srv.client()
            self.assertTrue(c.hello(code=2222)[3].startswith(b"EAUTH"))
            c.close()
        c = srv.client()
        self.assertTrue(c.hello(code=CODE)[3].startswith(b"EAUTH"))
        c.close()
        srv.proc.kill()
        srv.finish()
        self.assertEqual(self.sha(img), before)

    def test_version_mismatch(self):
        srv = NgtServe(self, self.card({"/02/DISC.GDI": GDI3}), "--slots", "2")
        c = srv.client()
        t, fl, sq, p = c.hello(version=2)
        self.assertEqual(t, NGT_ERROR)
        self.assertTrue(p.startswith(b"EVERSION"), p)

    def test_n4_requests_in_the_wrong_state(self):
        img = self.card({"/02/DISC.GDI": GDI3})
        before = self.sha(img)
        srv = NgtServe(self, img)
        # a PUT before HELLO
        c = srv.client()
        text, src, files = self.gdi_set("03")
        c.send(NGT_PUT, self.put_payload(text))
        t, fl, sq, p = c.recv()
        self.assertEqual(t, NGT_ERROR)
        self.assertTrue(p.startswith(b"ESTATE"), p)
        self.assertRaises(EOFError, c.recv)
        # a DATA nobody asked for
        c = srv.client()
        c.hello()
        c.send(NGT_DATA, struct.pack("<HHQ", 0, 0, 0) + b"xx")
        self.assertTrue(c.recv()[3].startswith(b"ESTATE"))
        # an unknown request type
        c = srv.client()
        c.hello()
        c.send(0x55)
        self.assertTrue(c.recv()[3].startswith(b"EPROTO"))
        # a bad magic
        c = srv.client()
        c.send_raw(b"XXXX" + b"\0" * 12)                                         # exactly one header: nothing is left unread
        self.assertTrue(c.recv()[3].startswith(b"EPROTO"))
        # an over-long frame header
        c = srv.client()
        c.send_raw(b"NGT1" + bytes([NGT_PING, 0, 0, 0]) + struct.pack("<II", 1, 70000))
        self.assertTrue(c.recv()[3].startswith(b"EPROTO"))
        # the session was never opened by any of this: the SD card is as it was
        c = srv.client()
        c.hello()
        c.send(NGT_FINISH, bytes([1]))
        self.assertEqual(c.recv()[0], NGT_DONE)                                  # no session: just DONE
        srv.finish()
        self.assertEqual(self.sha(img), before)
        self.assertEqual((sim_int(srv.sim, "begin"), sim_int(srv.sim, "cmd_writes")), (0, 0))

    def test_n4_data_for_the_wrong_offset(self):
        img = self.card({"/02/DISC.GDI": GDI3})
        srv = NgtServe(self, img)
        c = srv.client()
        c.hello()
        text, src, files = self.gdi_set("03")
        seq = c.send(NGT_PUT, self.put_payload(text))
        while True:
            t, fl, sq, p = c.recv()
            if t == NGT_WANT:
                break
        idx, _z, off, n = struct.unpack("<HHQI", p)
        c.send_raw(ngt_frame(NGT_DATA, struct.pack("<HHQ", idx, 0, off + 512) + b"\0" * 16, 0, sq))   # not the offset asked for
        while True:
            t, fl, sq, p = c.recv()
            if t == NGT_ERROR:
                break
        self.assertTrue(p.startswith(b"ESTATE"), p)
        c = srv.client()
        c.hello()
        c.send(NGT_FINISH, bytes([1]))
        c.recv(), c.recv()
        srv.finish()
        fc = self.clean_fs(img)
        self.assertEqual(fc.visible_images().get("/03"), None, "half-written image must be invisible")
        self.assertEqual(sim_int(srv.sim, "violations"), 0)

    def test_n5_bad_crc(self):
        img = self.card({"/02/DISC.GDI": GDI3})
        srv = NgtServe(self, img)
        c = srv.client()
        c.send_raw(ngt_frame(NGT_HELLO, struct.pack("<HH", 1, CODE) + b"x", crc=0xDEADBEEF))
        t, fl, sq, p = c.recv()
        self.assertEqual(t, NGT_ERROR)
        self.assertTrue(p.startswith(b"EBADCRC"), p)
        self.assertRaises(EOFError, c.recv)
        c = srv.client()                                                         # and the server is still there
        self.assertEqual(c.hello()[0], NGT_HELLO_OK)
        c.send(NGT_FINISH, bytes([1]))
        self.assertEqual(c.recv()[0], NGT_DONE)
        srv.finish()
        self.assertEqual(sim_int(srv.sim, "begin"), 0)

    def test_n9_no_space(self):
        img = self.card({"/02/DISC.GDI": GDI3})
        mfst = mf.Manifest("03", "gdi", [mf.FileEntry(3 * 1024 ** 3, 0x12345678, "disc.gdi")], "disc.gdi").render()
        srv = NgtServe(self, img)
        c = srv.client()
        c.hello()
        r = c.request(NGT_PUT, self.put_payload(mfst))
        self.assertFalse(r[1], r)
        self.assertEqual(r[2], "ENOSPC")
        self.assertTrue(any(l.startswith("NGD:ERROR 0001 code=ENOSPC") for l in r[0]), r[0])
        c.send(NGT_FINISH, bytes([1]))
        c.recv(), c.recv()
        srv.finish()
        fc = self.clean_fs(img)
        self.assertEqual(fc.folder_files("/03"), {})                             # nothing was made on the card
        self.assertEqual((sim_int(srv.sim, "cmd_writes") > 0, sim_int(srv.sim, "unsess")), (False, 0))

    def test_n11_idle_session_is_closed(self):
        img = self.card({"/02/DISC.GDI": GDI3})
        srv = NgtServe(self, img, "--idle-ms", "400")
        c = srv.client()
        c.hello()
        r = c.request(NGT_STATFS)
        self.assertTrue(r[1], r)
        self.assertTrue(any(l.startswith("NGD:STATFS free=") for l in r[0]), r[0])
        sim = srv.finish()                                                       # no FINISH: the idle timer ends it
        self.assertEqual((sim_int(sim, "begin"), sim_int(sim, "end"), sim_int(sim, "endmode")), (1, 1, 1), srv.text)
        self.assertEqual((sim_int(sim, "unsess"), sim_int(sim, "violations")), (0, 0))

    def test_n13_remote_operations(self):
        files = {"/02/DISC.GDI": GDI3, "/03/DISC.CDI": b"cdi", "/04/DISC.NGP": b"x", "/05/README.TXT": b"x"}
        img = self.card(files)
        srv = NgtServe(self, img)
        c = srv.client()
        c.hello()
        lines, ok, name = c.request(NGT_STATFS)
        self.assertTrue(ok, name)
        m = [l for l in lines if l.startswith("NGD:STATFS")]
        self.assertEqual(len(m), 1)
        self.assertRegex(m[0], r"^NGD:STATFS free=\d+ total=\d+ cluster=512$")
        self.assertTrue(any(l.startswith("NGD:MOUNT pct=100") for l in lines), lines)    # the session opened lazily
        self.assertTrue(any(l.startswith("NGD:HELLO ") for l in lines), lines)
        lines, ok, name = c.request(NGT_LIST, b"/")
        self.assertTrue(ok, name)
        self.assertEqual(sorted(l for l in lines if l.startswith("NGD:LIST")),
                         ["NGD:LIST complete 02", "NGD:LIST complete 03", "NGD:LIST other 05", "NGD:LIST partial 04"])
        lines, ok, name = c.request(NGT_VERIFY, b"02")
        self.assertFalse(ok)
        self.assertEqual(name, "ENOENT")                                         # no NETGD.MF there: the same error recv gives
        lines, ok, name = c.request(NGT_DELETE, bytes([0]) + b"03")
        self.assertTrue(ok, (lines, name))
        self.assertIn('NGD:DELETE "03" ok', lines)
        lines, ok, name = c.request(NGT_DELETE, bytes([0]) + b"..")
        self.assertFalse(ok)
        c.send(NGT_FINISH, bytes([1]))
        self.assertTrue(any(l[0] == NGT_LINE for l in [c.recv()]))               # NGD:FINISH ok
        t, fl, sq, p = c.recv()
        self.assertEqual((t, p[0]), (NGT_DONE, 1))
        sim = srv.finish()
        self.assertEqual((sim_int(sim, "begin"), sim_int(sim, "end"), sim_int(sim, "endmode")), (1, 1, 1))
        fc = self.clean_fs(img)
        self.assertNotIn("/03/DISC.CDI", fc.files)
        self.assertIn("/02/DISC.GDI", fc.files)

    def test_put_over_ngtp_and_finish_mount(self):
        img = self.card({"/02/DISC.GDI": GDI3})
        text, src, files = self.gdi_set("03")
        srv = NgtServe(self, img)
        c = srv.client()
        c.hello()
        lines, ok, name = c.request(NGT_PUT, self.put_payload(text, tag="0007"), source=src)
        self.assertTrue(ok, (lines, name))
        self.assertTrue(any(l.startswith("NGD:FILE 0007 file=1 ") for l in lines), lines)
        self.assertTrue(any(l.startswith("NGD:PUT 0007") for l in lines), lines)
        c.send(NGT_FINISH, bytes([2]) + b"03")                                   # mount 03
        c.recv(), c.recv()
        sim = srv.finish()
        self.assertEqual((sim_int(sim, "end"), sim_int(sim, "endmode")), (1, 2), srv.text)
        self.assertEqual(sim["endname"], "03")
        fc = self.clean_fs(img)
        names = {n.lower(): v for n, v in fc.folder_files("/03").items()}
        self.assertEqual(sorted(names), sorted(list(files) + ["netgd.mf"]))
        for n, d in files.items():
            self.assertEqual(self.read(fc, "/03", n), d, n)
        self.assertEqual([p.lower() for p in fc.visible_images().get("/03")], ["/03/disc.gdi"])
        self.assertEqual((sim_int(sim, "unsess"), sim_int(sim, "violations")), (0, 0))

    def test_n8_force_on_a_protected_folder(self):
        for decision, ok_expected in (("allow", True), ("deny", False)):
            img = self.card({"/01/MENU.CDI": b"menu"}, name="f_%s.img" % decision)
            text, src, files = self.gdi_set("01")
            srv = NgtServe(self, img, "--confirm", decision)
            c = srv.client()
            c.hello()
            lines, ok, name = c.request(NGT_PUT, self.put_payload(text, mode=2, flags=1), source=src)
            self.assertEqual(ok, ok_expected, (decision, lines, name))
            if not ok_expected:
                self.assertEqual(name, "EPROTECTED")
            c.send(NGT_FINISH, bytes([1]))
            c.recv(), c.recv()
            srv.finish()
            fc = self.clean_fs(img)
            self.assertEqual("/01/DISC.GDI" in fc.files, ok_expected)
            self.assertEqual("/01/MENU.CDI" in fc.files, not ok_expected)       # refused: the old image is untouched
        # without the force flag a protected folder is refused without asking anybody
        img = self.card({"/01/MENU.CDI": b"menu"}, name="f_noflag.img")
        srv = NgtServe(self, img, "--confirm", "allow")
        c = srv.client()
        c.hello()
        lines, ok, name = c.request(NGT_PUT, self.put_payload(text, mode=2, flags=0), source=src)
        self.assertEqual((ok, name), (False, "EPROTECTED"))
        c.send(NGT_FINISH, bytes([1]))
        c.recv(), c.recv()
        srv.finish()

    def test_n10_stop_safely_and_resume(self):
        img = self.card({"/02/DISC.GDI": GDI3})
        text, src, files = self.gdi_set("03", sizes=(400000, 6000, 9000))
        srv = NgtServe(self, img, "--stopafter", "2")
        c = srv.client()
        c.hello()
        lines, ok, name = c.request(NGT_PUT, self.put_payload(text, mode=1), source=src)
        self.assertFalse(ok)
        self.assertEqual(name, "ESTOP")
        self.assertTrue(any(l.startswith("NGD:STOP 0001") for l in lines), lines)
        # the session is still open and the checkpoint is on the card: the same PUT again goes on
        lines, ok, name = c.request(NGT_PUT, self.put_payload(text, mode=1), source=src)
        self.assertTrue(ok, (lines, name))
        self.assertTrue(any(l.startswith("NGD:RESUME 0001") or l.startswith("NGD:FILE 0001") for l in lines), lines)
        c.send(NGT_FINISH, bytes([1]))
        c.recv(), c.recv()
        srv.finish()
        fc = self.clean_fs(img)
        self.assertEqual(self.read(fc, "/03", "track01.bin"), files["track01.bin"])

    def test_n6_pc_disconnects_midway_then_resumes(self):
        img = self.card({"/02/DISC.GDI": GDI3})
        text, src, files = self.gdi_set("03", sizes=(300000, 6000, 9000))
        srv = NgtServe(self, img)
        c = srv.client()
        c.hello()
        c.send(NGT_PUT, self.put_payload(text, mode=1))
        wants = 0
        while wants < 3:
            t, fl, sq, p = c.recv()
            if t == NGT_WANT:
                idx, _z, off, n = struct.unpack("<HHQI", p)
                c.send_raw(ngt_frame(NGT_DATA, struct.pack("<HHQ", idx, 0, off) + src[idx][off:off + n], 0, sq))
                wants += 1
        c.close()                                                                # the PC (or the LAN) goes away
        c = srv.client()                                                         # the server is waiting again, session open
        t, fl, sq, p = c.hello()
        self.assertEqual(t, NGT_HELLO_OK)
        self.assertEqual(struct.unpack_from("<H", p, 2)[0], 1)                  # flags: the session is open
        lines, ok, name = c.request(NGT_PUT, self.put_payload(text, mode=1), source=src)
        self.assertTrue(ok, (lines, name))
        c.send(NGT_FINISH, bytes([1]))
        c.recv(), c.recv()
        srv.finish()
        fc = self.clean_fs(img)
        for n, d in files.items():
            self.assertEqual(self.read(fc, "/03", n), d, n)

    def test_n7_power_cut_sweep(self):
        base = self.card({"/02/DISC.GDI": GDI3})
        text, src, files = self.gdi_set("03", sizes=(70000, 9000, 5000))
        # a whole run first: how many sector writes it takes
        full = os.path.join(self.work, "full.img")
        shutil.copy(base, full)
        srv = NgtServe(self, full)
        c = srv.client()
        c.hello()
        self.assertTrue(c.request(NGT_PUT, self.put_payload(text), source=src)[1])
        c.send(NGT_FINISH, bytes([1]))
        c.recv(), c.recv()
        total = sim_int(srv.finish(), "sector_writes")
        self.assertGreater(total, 100)
        cuts = sorted({max(1, int(total * f)) for f in (0.01, 0.05, 0.12, 0.2, 0.3, 0.42, 0.55, 0.68, 0.8, 0.9, 0.96, 0.99)})
        results = []
        lost_max = 0
        for cut in cuts:
            img = os.path.join(self.work, "cut%d.img" % cut)
            shutil.copy(base, img)
            srv = NgtServe(self, img, "--cut", str(cut))
            c = srv.client()
            try:
                c.hello()
                c.request(NGT_PUT, self.put_payload(text), source=src)
            except (EOFError, OSError, AssertionError):
                pass                                                              # the power went
            c.close()
            srv.finish()
            self.assertIn("POWERCUT", srv.text, "cut %d" % cut)
            fc = fatcheck.Fat32(img)
            self.assertEqual(fc.errors, [], "cut %d" % cut)
            vis = fc.visible_images().get("/03")
            if vis:                                                               # a visible image is a complete one
                for n, d in files.items():
                    self.assertEqual(self.read(fc, "/03", n), d, "cut %d %s" % (cut, n))
            results.append(bool(vis))
            # power back: the same PUT (resume) finishes it
            srv = NgtServe(self, img)
            c = srv.client()
            c.hello()
            lines, ok, name = c.request(NGT_PUT, self.put_payload(text, mode=1), source=src)
            self.assertTrue(ok, "cut %d: %s %s" % (cut, lines[-3:], name))
            c.send(NGT_FINISH, bytes([1]))
            c.recv(), c.recv()
            srv.finish()
            fc = fatcheck.Fat32(img)
            self.assertEqual(fc.errors, [], "cut %d: structure after recovery" % cut)
            # A cut during the allocation may leave clusters allocated but unused (netgd-recv's own sweep reports the same
            # "lost_max"); after the allocation at most the cluster of NETGD.CKP itself.  Half way through is past the allocation.
            lost = fc.lost_clusters()
            lost_max = max(lost_max, lost)
            if cut >= total // 2:
                self.assertLessEqual(lost, 1, "cut %d leaked %d clusters" % (cut, lost))
            self.assertFalse([p for p in fc.files if p.lower().endswith((".ngp", ".ckp"))], "cut %d: leftovers" % cut)
            for n, d in files.items():
                self.assertEqual(self.read(fc, "/03", n), d, "after resume, cut %d %s" % (cut, n))
        print("ngtp power cuts: %d positions of %d sector writes; image visible after the cut at %d of them; most clusters left over %d"
              % (len(cuts), total, sum(results), lost_max), file=sys.stderr)


SEND_PY = os.path.join(os.path.dirname(ROOT), "netgd-send", "netgd-send.py")


class TestPush(NetBase):
    """netgd-send push / remote against menu_host serve (design 11.2 N1, N2, N8, N10, N12, N14)."""

    def make_gdi(self, sub="src", sizes=(4 * 2352, 3 * 2352, 5 * 2048), stem="mygame"):
        d = os.path.join(self.work, sub)
        os.makedirs(d, exist_ok=True)
        specs = [(1, 0, 4, 2352, "track01.bin"), (2, 450, 0, 2352, "track02.raw"), (3, 45000, 4, 2048, "track03.bin")]
        lines = ["3"]
        for (num, lba, ctrl, sec, nm), size in zip(specs, sizes):
            with open(os.path.join(d, nm), "wb") as f:
                f.write(self.pattern(size, num))
            lines.append("%d %d %d %d %s 0" % (num, lba, ctrl, sec, nm))
        p = os.path.join(d, stem + ".gdi")
        with open(p, "wb") as f:
            f.write(("\r\n".join(lines) + "\r\n").encode("ascii"))
        return p, {nm: self.pattern(size, num) for (num, _l, _c, _s, nm), size in zip(specs, sizes)}

    def make_cdi(self, sub="cdi", stem="disc one", size=8192):
        d = os.path.join(self.work, sub)
        os.makedirs(d, exist_ok=True)
        data = bytearray(self.pattern(size, 9))
        data[-8:] = (0x80000005).to_bytes(4, "little") + (size - 100).to_bytes(4, "little")
        p = os.path.join(d, stem + ".cdi")
        with open(p, "wb") as f:
            f.write(bytes(data))
        return p, bytes(data)

    def push(self, srv, image, *extra, timeout=120):
        cmd = [sys.executable, SEND_PY, "push", image, "--host", "127.0.0.1", "--port", str(srv.port), "--code", str(CODE)] + list(extra)
        r = run_child(cmd, timeout=timeout)
        return r

    def test_n1_push_gdi_slot_auto(self):
        img = self.card({"/02/DISC.GDI": GDI3, "/04/DISC.GDI": GDI3})
        gdi, tracks = self.make_gdi()
        srv = NgtServe(self, img, "--slots", "2,3,4")
        r = self.push(srv, gdi, "--slot", "auto")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("using the free folder 03", r.stdout)
        sim = srv.finish()
        self.assertEqual((sim_int(sim, "begin"), sim_int(sim, "end"), sim_int(sim, "endmode")), (1, 1, 1), srv.text)
        fc = self.clean_fs(img)
        self.assertEqual(sorted(k.lower() for k in fc.folder_files("/03")),
                         sorted(["disc.gdi", "track01.bin", "track02.raw", "track03.bin", "name.txt", "netgd.mf"]))
        for n, d in tracks.items():
            self.assertEqual(self.read(fc, "/03", n), d, n)
        self.assertEqual(self.read(fc, "/03", "name.txt"), b"mygame\n")
        self.assertFalse([p for p in fc.files if p.lower().endswith((".ngp", ".ckp"))])
        self.assertEqual([p.lower() for p in fc.visible_images()["/03"]], ["/03/disc.gdi"])
        # the rewritten descriptor names the 8.3 files
        self.assertIn(b"track03.bin", self.read(fc, "/03", "disc.gdi"))

    def test_n2_push_cdi_by_name_and_mount(self):
        img = self.card({"/02/DISC.GDI": GDI3})
        cdi, data = self.make_cdi()
        srv = NgtServe(self, img)
        r = self.push(srv, cdi, "--name", "MYCDI", "--mount", "--title", "Café Disc")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        sim = srv.finish()
        self.assertEqual((sim_int(sim, "endmode"), sim["endname"]), (2, "MYCDI"), srv.text)
        fc = self.clean_fs(img)
        self.assertEqual(self.read(fc, "/MYCDI", "disc.cdi"), data)
        self.assertEqual(self.read(fc, "/MYCDI", "name.txt"), b"Caf? Disc\n")                 # ASCII only

    def test_push_uses_a_name_txt_next_to_the_image(self):
        img = self.card({"/02/DISC.GDI": GDI3})
        gdi, tracks = self.make_gdi()
        with open(os.path.join(os.path.dirname(gdi), "Name.TXT"), "wb") as f:
            f.write(b"The Real Title\r\nsecond line\r\n")
        srv = NgtServe(self, img)
        r = self.push(srv, gdi, "--slot", "5")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        srv.finish()
        fc = self.clean_fs(img)
        self.assertEqual(self.read(fc, "/05", "name.txt"), b"The Real Title\r\nsecond line\r\n")

    def test_wrong_code_is_refused(self):
        img = self.card({"/02/DISC.GDI": GDI3})
        gdi, tracks = self.make_gdi()
        srv = NgtServe(self, img)
        cmd = [sys.executable, SEND_PY, "push", gdi, "--host", "127.0.0.1", "--port", str(srv.port), "--code", "1"]
        r = run_child(cmd)
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn("EAUTH", r.stdout)
        srv.proc.kill()
        srv.finish()

    def test_n8_push_force_protected_folder(self):
        for decision, rc in (("allow", 0), ("deny", 1)):
            img = self.card({"/01/MENU.CDI": b"menu"}, name="p_%s.img" % decision)
            gdi, tracks = self.make_gdi("src_" + decision)
            srv = NgtServe(self, img, "--confirm", decision)
            r = self.push(srv, gdi, "--slot", "1", "--mode", "overwrite", "--force")
            self.assertEqual(r.returncode, rc, (decision, r.stdout, r.stderr))
            if rc:
                self.assertIn("EPROTECTED", r.stdout)
                srv.test.assertTrue(True)
                cl = srv.client()
                cl.hello()
                cl.send(NGT_FINISH, bytes([1]))
                cl.recv(), cl.recv()
            srv.finish()
            fc = self.clean_fs(img)
            self.assertEqual("/01/DISC.GDI" in fc.files, rc == 0)
            self.assertEqual("/01/MENU.CDI" in fc.files, rc != 0)

    def test_n10_stop_safely_exit_code_2_then_resume(self):
        img = self.card({"/02/DISC.GDI": GDI3})
        gdi, tracks = self.make_gdi(sizes=(200 * 2352, 3 * 2352, 5 * 2048))
        srv = NgtServe(self, img, "--stopafter", "2")
        r = self.push(srv, gdi, "--slot", "3")
        self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
        self.assertIn("NGD:STOP", r.stdout)
        self.assertIn("incomplete", r.stdout)
        r = self.push(srv, gdi, "--slot", "3")                                   # the same command again
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        sim = srv.finish()
        self.assertEqual(sim_int(sim, "begin"), 1)                               # one session over both runs
        fc = self.clean_fs(img)
        self.assertEqual(self.read(fc, "/03", "track01.bin"), tracks["track01.bin"])

    def test_auto_slot_resumes_the_partial_folder_when_run_again(self):
        """R104 finding 2: the same command (default --slot auto) goes on in the folder the stop left, not in the next one"""
        img = self.card({"/02/DISC.GDI": GDI3})
        gdi, tracks = self.make_gdi(sizes=(200 * 2352, 3 * 2352, 5 * 2048))
        srv = NgtServe(self, img, "--stopafter", "2")
        r = self.push(srv, gdi)
        self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
        self.assertIn("using the free folder 03", r.stdout)
        self.assertIn("--slot 03", r.stdout)                                     # the exact command to go on
        r = self.push(srv, gdi)                                                   # the same command again
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("trying to resume it", r.stdout)
        self.assertNotIn("using the free folder", r.stdout)
        srv.finish()
        fc = self.clean_fs(img)
        self.assertEqual(self.read(fc, "/03", "track01.bin"), tracks["track01.bin"])
        self.assertFalse([k for k in fc.files if k.startswith("/04") or k.startswith("04")], "a second numbered folder was started")
        self.assertFalse([k for k in fc.files if k.lower().endswith((".ngp", ".ckp"))], "leftovers")

    def test_auto_slot_skips_a_partial_folder_of_a_different_image(self):
        img = self.card({"/02/DISC.GDI": GDI3})
        gdi1, _t1 = self.make_gdi("srcA", sizes=(200 * 2352, 3 * 2352, 5 * 2048), stem="first")
        gdi2, tracks2 = self.make_gdi("srcB", sizes=(7 * 2352, 3 * 2352, 5 * 2048), stem="second")
        srv = NgtServe(self, img, "--stopafter", "2")
        r = self.push(srv, gdi1)
        self.assertEqual(r.returncode, 2, r.stdout + r.stderr)                   # 03 is partial now
        r = self.push(srv, gdi2)                                                  # another image: not 03
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("holds a different image", r.stdout)
        self.assertIn("using the free folder 04", r.stdout)
        srv.finish()
        fc = self.clean_fs(img)
        self.assertEqual(self.read(fc, "/04", "track01.bin"), tracks2["track01.bin"])

    def test_n12_after_finish_the_list_shows_the_new_folder(self):
        img = self.card({"/02/DISC.GDI": GDI3})
        gdi, tracks = self.make_gdi()
        srv = NgtServe(self, img, "--dump", "--slots", "2,3")
        r = self.push(srv, gdi, "--slot", "auto", "--title", "Pushed Game")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        srv.finish()
        rows = [l[4:] for l in srv.lines if re.match(r"S\d\d\|", l)]
        self.assertIn(exp_row(False, 3, "Pushed Game", "GDI"), rows, "\n".join(rows))
        self.assertIn("SERVE-END state=3", srv.text)                              # back in the list
        self.assertIn("2 games", "\n".join(rows))

    def test_remote_operations(self):
        img = self.card({"/02/DISC.GDI": GDI3, "/03/DISC.CDI": b"cdi"})
        srv = NgtServe(self, img)

        def remote(*args):
            return run_child([sys.executable, SEND_PY, "remote", args[0], *args[1:], "--host", "127.0.0.1",
                              "--port", str(srv.port), "--code", str(CODE)])

        r = remote("statfs")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertRegex(r.stdout, r"NGD:STATFS free=\d+ total=\d+ cluster=512")
        r = remote("list")
        self.assertEqual(r.returncode, 0, r.stdout)
        self.assertIn("NGD:LIST complete 02", r.stdout)
        r = remote("verify", "02")
        self.assertEqual(r.returncode, 1, r.stdout)                              # no NETGD.MF
        r = remote("delete", "03")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn('NGD:DELETE "03" ok', r.stdout)
        r = remote("finish")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        sim = srv.finish()
        self.assertEqual((sim_int(sim, "begin"), sim_int(sim, "end")), (1, 1))
        self.assertNotIn("/03/DISC.CDI", self.clean_fs(img).files)


def udp_ask(port, data, wait=2.0):
    """one datagram to 127.0.0.1:port; the reply, or None when nothing came within `wait` seconds"""
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        s.settimeout(wait)
        s.sendto(data, ("127.0.0.1", port))
        try:
            return s.recvfrom(512)[0]
        except (socket.timeout, OSError):
            return None
    finally:
        s.close()


def free_udp_port():
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


class TestDiscovery(NetBase):
    """step 9: the UDP discovery on the transfer screen and netgd-send's use of it"""
    make_gdi = TestPush.make_gdi

    def send(self, *args, timeout=120):
        return run_child([sys.executable, SEND_PY] + list(args), timeout=timeout)

    def test_reply_and_ignored_requests(self):
        srv = NgtServe(self, self.card({"/02/DISC.GDI": GDI3}))
        r = udp_ask(srv.port, b"NGTP-DISCOVER 1")
        self.assertIsNotNone(r)
        self.assertRegex(r.decode(), r"^NGTP-HERE 1 name=NetGD app=\S+ state=idle$")
        self.assertNotIn(b"4821", r)                                     # the pass code never goes out
        self.assertIsNotNone(udp_ask(srv.port, b"NGTP-DISCOVER 1 more=ignored\r\n"))
        self.assertIsNone(udp_ask(srv.port, b"NGTP-DISCOVER 2", 0.5))
        self.assertIsNone(udp_ask(srv.port, b"hello", 0.5))
        self.assertIsNone(udp_ask(srv.port, b"NGTP-HERE 1 name=NetGD app=0 state=idle", 0.5))
        self.assertIsNotNone(udp_ask(srv.port, b"NGTP-DISCOVER 1"))      # still alive after the junk

    def test_state_is_busy_while_a_pc_is_attached(self):
        srv = NgtServe(self, self.card({"/02/DISC.GDI": GDI3}))
        c = srv.client()
        deadline = time.time() + 10
        got = b""
        while time.time() < deadline and b"state=busy" not in got:
            got = udp_ask(srv.port, b"NGTP-DISCOVER 1", 0.5) or b""
        self.assertIn(b"state=busy", got)
        c.close()
        deadline = time.time() + 10
        while time.time() < deadline and b"state=idle" not in got:
            got = udp_ask(srv.port, b"NGTP-DISCOVER 1", 0.5) or b""
        self.assertIn(b"state=idle", got)

    def test_discover_command_lists_the_menu(self):
        srv = NgtServe(self, self.card({"/02/DISC.GDI": GDI3}))
        r = self.send("discover", "--broadcast", "127.0.0.1", "--port", str(srv.port), "--discover-timeout", "1")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertRegex(r.stdout, r"127\.0\.0\.1\s+NetGD\s+app \S+\s+idle")

    def test_discover_command_without_an_answer_exits_1(self):
        r = self.send("discover", "--broadcast", "127.0.0.1", "--port", str(free_udp_port()), "--discover-timeout", "0.5")
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn("no menu answered", r.stdout)

    def test_push_without_host_finds_the_menu(self):
        img = self.card({"/02/DISC.GDI": GDI3})
        gdi, tracks = self.make_gdi()
        srv = NgtServe(self, img, "--slots", "2")
        r = self.send("push", gdi, "--code", str(CODE), "--port", str(srv.port), "--broadcast", "127.0.0.1",
                      "--discover-timeout", "1", "--slot", "auto")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("found NetGD at 127.0.0.1", r.stdout)
        srv.finish()
        fc = self.clean_fs(img)
        self.assertEqual(self.read(fc, "/03", "track03.bin"), tracks["track03.bin"])

    def test_push_without_host_and_without_a_menu_fails_cleanly(self):
        gdi, _t = self.make_gdi()
        r = self.send("push", gdi, "--code", "1", "--port", str(free_udp_port()), "--broadcast", "127.0.0.1",
                      "--discover-timeout", "0.5")
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn("no menu answered", r.stdout)
        self.assertNotIn("connected:", r.stdout)

    def test_without_udp_the_host_argument_still_works(self):
        img = self.card({"/02/DISC.GDI": GDI3, "/03/DISC.CDI": b"cdi"})
        srv = NgtServe(self, img, "--no-udp")
        self.assertIsNone(udp_ask(srv.port, b"NGTP-DISCOVER 1", 0.5))
        r = self.send("remote", "statfs", "--host", "127.0.0.1", "--port", str(srv.port), "--code", str(CODE))
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        r = self.send("remote", "list", "--code", str(CODE), "--port", str(srv.port), "--broadcast", "127.0.0.1",
                      "--discover-timeout", "0.5")
        self.assertEqual(r.returncode, 1, r.stdout)                      # nothing to discover without a host
        self.assertIn("no menu answered", r.stdout)
        self.assertNotIn("Traceback", r.stderr)


def art_file(seed):
    """a NETGD.ART with a picture of its own (made by netgd-send's art module)"""
    return artmod.encode(bytes(((i * seed) >> 3) & 255 for i in range(128 * 128 * 3)))


def art_crc(data):
    return "%08X" % (zlib.crc32(data[16:16 + 128 * 128 * 2]) & 0xFFFFFFFF)


def art_lines(run):
    """every `ART ...` line a run printed (one per dump): a dict each"""
    return [dict(re.findall(r"(\w+)=(\S+)", l)) for l in run.text.splitlines() if l.startswith("ART ")]


class TestBoxArt(UiBase):
    """step 11: NETGD.ART in the folder of the game under the cursor"""

    def library(self, **extra):
        files = {"/02/DISC.GDI": GDI3, "/02/NETGD.ART": art_file(3),
                 "/03/DISC.GDI": GDI3, "/03/NETGD.ART": art_file(5),
                 "/04/DISC.GDI": GDI3}
        files.update(extra)
        return files

    def test_shown_only_after_the_cursor_stood_still_and_not_before(self):
        img = self.card(self.library())
        r = self.ui(img, "scan, t:100, dump, t:700, dump")
        self.assertEqual(r.rc, 0, r.text)
        early, late = art_lines(r)
        self.assertEqual((early["shown"], early["state"]), ("0", "1"), r.text)          # waiting (NM_ART_WAIT)
        self.assertEqual((late["shown"], late["dirty"], late["layout"]), ("1", "0", "1"), r.text)
        self.assertEqual(late["crc"], art_crc(self.library()["/02/NETGD.ART"]))
        self.assertEqual(r.n("cmd_writes"), 0)

    def test_the_list_keeps_clear_of_the_art_block(self):
        img = self.card(self.library())
        r = self.ui(img, "scan, t:700, dump")
        rows = r.rows
        self.assertEqual(art_lines(r)[0]["shown"], "1")
        # the rows of the list (2..) end at column 42; the selected row is inverted only that far (no way to see attributes
        # here: dirty=0 says that every cell of the block was blank and normal at each present)
        for line in rows[2:5]:
            self.assertLessEqual(len(line), 42, line)
        self.assertRegex(rows[2], r"^>0002  02 {1,30}GDI$")
        self.assertEqual(art_lines(r)[0]["dirty"], "0")

    def test_moving_hides_it_at_once_and_the_next_game_gets_its_own(self):
        lib = self.library()
        img = self.card(lib)
        r = self.ui(img, "scan, t:700, D, dump, t:700, dump")
        a_moved, a_next = art_lines(r)
        self.assertEqual(a_moved["shown"], "0", r.text)
        self.assertEqual((a_next["shown"], a_next["crc"], a_next["dirty"]), ("1", art_crc(lib["/03/NETGD.ART"]), "0"), r.text)

    def test_a_game_without_art_shows_none_and_the_layout_stays(self):
        img = self.card(self.library())
        r = self.ui(img, "scan, t:700, D, D, t:900, dump")
        a = art_lines(r)[0]
        self.assertEqual((a["shown"], a["layout"], a["state"]), ("0", "1", "4"), r.text)     # FAILED: no file, not retried
        self.assertRegex(r.rows[4], r"^>0004 ", r.rows[4])
        self.assertLessEqual(len(r.rows[4]), 42)
        self.assertNotIn("error", "\n".join(r.rows).lower())

    def test_files_the_menu_cannot_use_are_ignored(self):
        good = art_file(7)
        wrong_size = good[:6] + struct.pack("<HH", 64, 64) + good[10:]
        bad_magic = b"XGAR" + good[4:]
        version2 = good[:4] + struct.pack("<H", 2) + good[6:]
        truncated = good[:-2]
        lib = {"/02/DISC.GDI": GDI3, "/02/NETGD.ART": wrong_size,
               "/03/DISC.GDI": GDI3, "/03/NETGD.ART": bad_magic,
               "/04/DISC.GDI": GDI3, "/04/NETGD.ART": version2,
               "/05/DISC.GDI": GDI3, "/05/NETGD.ART": truncated,
               "/06/DISC.GDI": GDI3, "/06/NETGD.ART": b"",
               "/07/DISC.GDI": GDI3, "/07/NETGD.ART": good + b"trailing bytes are fine"}
        img = self.card(lib)
        script = "scan, " + ", ".join("t:700, dump, D" for _ in range(6)) + ", t:100, dump"
        r = self.ui(img, script)
        a = art_lines(r)
        self.assertEqual([x["shown"] for x in a[:6]], ["0", "0", "0", "0", "0", "1"], r.text)
        self.assertEqual(a[5]["crc"], art_crc(good))
        self.assertEqual(r.head["count"], "6")                                  # the list is whole and was never stopped
        self.assertEqual({x["dirty"] for x in a}, {"0"})
        self.assertEqual(r.n("cmd_writes"), 0)

    def test_read_errors_do_not_stop_the_list_and_a_later_visit_works(self):
        lib = self.library()
        img = self.card(lib)
        r = self.ui(img, "scan, rflipboth, t:900, dump, D, U, t:100, dump, rflipoff, D, U, t:900, dump")
        a = art_lines(r)
        self.assertEqual((a[0]["shown"], a[0]["state"]), ("0", "4"), r.text)        # the load failed: FAILED, nothing shown
        self.assertEqual(r.head["cursor"], "0")
        self.assertEqual((a[2]["shown"], a[2]["crc"]), ("1", art_crc(lib["/02/NETGD.ART"])), r.text)
        status = r.rows[17]
        self.assertNotIn("unreadable", status)
        self.assertNotIn("read error", status)

    def test_the_picture_goes_with_the_list_and_comes_back(self):
        img = self.card(self.library())
        r = self.ui(img, "scan, t:700, X, dump, B, dump")
        in_info, back = art_lines(r)
        self.assertEqual(in_info["shown"], "0", r.text)
        self.assertEqual((back["shown"], back["dirty"]), ("1", "0"), r.text)

    def test_launching_while_the_art_loads_or_is_shown(self):
        img = self.card(self.library())
        for script in ("scan, t:700, A, t:3000", "scan, t:330, A, t:3000", "scan, t:330, S, A, t:300"):
            r = self.ui(img, script, "--slots", "2,3,4")
            self.assertEqual(r.handoff, ["bios"], script + "\n" + r.text)
            self.assertEqual((r.n("cmd_writes"), r.n("violations")), (0, 0), script)

    def test_no_art_files_no_change_to_the_list(self):
        img = self.card({"/02/DISC.GDI": GDI3, "/03/DISC.GDI": GDI3})
        r = self.ui(img, "scan, t:900, dump")
        a = art_lines(r)[0]
        self.assertEqual((a["shown"], a["layout"]), ("0", "0"))
        self.assertEqual(r.rows[2], exp_row(True, 2, "02", "GDI"))                  # the full 38-column title as before


def solid_png(w, h, rgb):
    """a w x h PNG of one colour (colour type 2, 8 bit, no filter)"""
    def chunk(typ, body):
        return struct.pack(">I", len(body)) + typ + body + struct.pack(">I", zlib.crc32(typ + body) & 0xFFFFFFFF)
    raw = (b"\0" + bytes(rgb) * w) * h
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw))
            + chunk(b"IEND", b""))


class TestWindow(NetBase):
    """The NGTP receive window: 16 KiB x 1 on stock KOS TCP (R178), 64 KiB x 2 in a tcpfix build (as netgd-t2)"""

    def push_counting(self, c, text, src):
        """a PUT; WANTs are answered only when no more frames come for 0.3 s: the largest WANT and most outstanding at once"""
        seq = c.send(NGT_PUT, self.put_payload(text, mode=1))
        pending, most, biggest = [], 0, 0
        while True:
            c.sock.settimeout(0.3 if pending else 30)
            try:
                peek = c.sock.recv(1, socket.MSG_PEEK)
            except socket.timeout:
                idx, off, n, sq = pending.pop(0)
                c.send_raw(ngt_frame(NGT_DATA, struct.pack("<HHQ", idx, 0, off) + src[idx][off:off + n], 0, sq))
                continue
            finally:
                c.sock.settimeout(30)
            self.assertTrue(peek, "the server closed")
            t, fl, sq, p = c.recv()
            if t == NGT_WANT:
                idx, _z, off, n = struct.unpack("<HHQI", p)
                pending.append((idx, off, n, sq))
                most = max(most, len(pending))
                biggest = max(biggest, n)
            elif t == NGT_DONE:
                self.assertEqual(p[0], 1, p)
                self.assertEqual(pending, [])
                return biggest, most
            elif t == NGT_ERROR:
                self.fail(p)
        del seq

    def run_push(self, *opts):
        img = self.card({"/02/DISC.GDI": GDI3})
        text, src, files = self.gdi_set("03", sizes=(300000, 6000, 9000))
        srv = NgtServe(self, img, *opts)
        c = srv.client()
        c.hello()
        biggest, most = self.push_counting(c, text, src)
        c.send(NGT_FINISH, bytes([1]))
        c.recv(), c.recv()
        srv.finish()
        fc = self.clean_fs(img)
        for n, d in files.items():
            self.assertEqual(self.read(fc, "/03", n), d, n)
        return srv, biggest, most

    def test_stock_default_is_16k_x_1(self):
        srv, biggest, most = self.run_push()
        self.assertIn("WINDOW want=16384 depth=1 tcpfix=0 rcvbuf=20480", srv.text)
        self.assertEqual((biggest, most), (16384, 1))

    def test_tcpfix_default_is_16k_x_1(self):
        # interim: 64 KiB x 2 is the shape of g2netbench p4, which hung the real Dreamcast (cause unknown)
        srv, biggest, most = self.run_push("--tcpfix")
        self.assertIn("WINDOW want=16384 depth=1 tcpfix=1 rcvbuf=20480", srv.text)
        self.assertEqual((biggest, most), (16384, 1))

    def test_tcpfix_explicit_64k_x_2_still_works(self):
        srv, biggest, most = self.run_push("--tcpfix", "--want", "65536", "--depth", "2")
        self.assertIn("WINDOW want=65536 depth=2 tcpfix=1 rcvbuf=20480", srv.text)
        self.assertEqual((biggest, most), (65536, 2))

    def test_b_stuck_does_not_stop_a_transfer(self):
        srv, biggest, most = self.run_push("--b-stuck")
        self.assertNotIn("NGD:STOP", srv.text)


class TestStuckB(NetBase):
    """A pad that reports B all the time (real hardware: 400 of 400 samples): B counts only after a stable release, and the
    screens that are left with B offer START instead, only while B is stuck."""
    S_LIST, S_INFO, S_CONFIRM_BIOS, S_NET_IDLE = 3, 4, 7, 9

    def lib(self):
        return self.card({"/02/DISC.GDI": GDI3})

    def run_ui(self, script, *opts):
        return self.ui(self.lib(), script, "--slots", "2", *opts)

    def state(self, r, i=-1):
        return int(r.screens[i][0]["state"])

    def test_info_screen_has_a_way_out(self):
        r = self.run_ui("scan X dump B dump S dump", "--b-stuck")
        self.assertEqual([self.state(r, i) for i in range(3)], [self.S_INFO, self.S_INFO, self.S_LIST], r.text)
        self.assertIn("START:Back (B is stuck)", NL.join(r.screens[0][1]))

    def test_bios_confirm_has_a_way_out(self):
        r = self.run_ui("scan S dump B dump S dump", "--b-stuck")
        self.assertEqual([self.state(r, i) for i in range(3)], [self.S_CONFIRM_BIOS, self.S_CONFIRM_BIOS, self.S_LIST], r.text)
        self.assertIn("START:No, stay in the menu (B is stuck)", NL.join(r.screens[0][1]))
        self.assertEqual(r.handoff, [])

    def test_transfer_screen_has_a_way_out(self):
        r = self.run_ui("scan Y dump B dump S t:100 dump", "--b-stuck")
        self.assertEqual([self.state(r, i) for i in range(3)], [self.S_NET_IDLE, self.S_NET_IDLE, self.S_LIST], r.text)
        self.assertIn("START:Leave this screen (B is stuck)", NL.join(r.screens[0][1]))

    def test_healthy_pad_start_does_not_leave(self):
        for script, want in (("scan X S dump", self.S_INFO), ("scan S S dump", self.S_CONFIRM_BIOS), ("scan Y S dump", self.S_NET_IDLE)):
            r = self.run_ui(script)
            self.assertEqual(self.state(r), want, script)
            self.assertNotIn("B is stuck", NL.join(r.rows))

    def test_released_then_pressed_works(self):
        r = self.run_ui("scan X unstuck t:200 dump B dump", "--b-stuck")
        self.assertEqual((self.state(r, 0), self.state(r, 1)), (self.S_INFO, self.S_LIST), r.text)
        self.assertIn("B:Back", NL.join(r.screens[0][1]))

    def test_one_up_sample_is_not_a_press(self):
        r = self.run_ui("scan X unstuck step stuck step step dump", "--b-stuck")
        self.assertEqual(self.state(r), self.S_INFO, r.text)

    def test_replugged_stuck_pad_is_ignored(self):
        r = self.run_ui("scan unstuck t:200 X nocontroller step stuck controller t:100 dump", "--b-stuck")
        self.assertEqual(self.state(r), self.S_INFO, r.text)
        self.assertIn("B is stuck", NL.join(r.rows))

    def test_force_confirm_works_with_b_stuck(self):
        img = self.card({"/01/MENU.CDI": b"menu"})
        text, src, files = self.gdi_set("01")
        srv = NgtServe(self, img, "--b-stuck", "--confirm", "allow")
        c = srv.client()
        c.hello()
        lines, ok, name = c.request(NGT_PUT, self.put_payload(text, mode=2, flags=1), source=src)
        self.assertTrue(ok, (lines, name))
        c.send(NGT_FINISH, bytes([1]))
        c.recv(), c.recv()
        srv.finish()
        self.assertIn("/01/DISC.GDI", self.clean_fs(img).files)


class TestR104LowItems(NetBase):
    """R104 'later' items A to D: a dead connection is replaced, one transition per frame, a session-end failure when the
    firmware says the session is closed, B after 'Stop safely' does not also leave the screen."""

    def test_a_silent_connection_is_replaced_by_a_new_one(self):
        img = self.card({"/02/DISC.GDI": GDI3})
        srv = NgtServe(self, img)
        c1 = srv.client()
        c1.hello()                                                    # then silent: its PC lost power, no FIN was sent
        c2 = srv.client()
        self.assertEqual(c2.hello()[0], NGT_HELLO_OK)                 # without the fix this never answers (the read times out)
        r = c2.request(NGT_STATFS)
        self.assertTrue(r[1], r)
        c2.send(NGT_FINISH, bytes([1]))
        c2.recv(), c2.recv()
        srv.finish()

    def test_a_new_connection_must_say_hello_again(self):
        img = self.card({"/02/DISC.GDI": GDI3})
        srv = NgtServe(self, img)
        c1 = srv.client()
        c1.hello()
        c2 = srv.client()
        c2.send(NGT_STATFS)                                           # no HELLO on the new connection: refused
        self.assertEqual(c2.recv()[0], NGT_ERROR)
        c3 = srv.client()
        self.assertEqual(c3.hello()[0], NGT_HELLO_OK)
        c3.send(NGT_FINISH, bytes([1]))
        c3.recv()                                                     # no session was opened: just DONE
        srv.finish()

    def test_b_a_and_start_together_start_the_game_only(self):
        img = self.card({"/02/DISC.GDI": GDI3, "/03/DISC.CDI": b"cdi"})
        run = self.ui(img, "scan A+S dump t:2000", "--slots", "2,3")
        self.assertNotEqual(run.screens[0][0]["state"], "7", run.text)       # not the 'go back to the BIOS menu?' screen
        self.assertEqual(run.handoff, ["bios"], run.text)
        self.assertEqual(run.n("gdemu_mounts"), 1)

    def test_b_y_and_x_together_open_the_transfer_screen_only(self):
        img = self.card({"/02/DISC.GDI": GDI3})
        run = self.ui(img, "scan Y+X dump", "--slots", "2")
        self.assertEqual(run.screens[0][0]["state"], "9", run.text)          # NM_S_NET_IDLE, not NM_S_INFO

    def test_b_a_priority_and_cursor_moves_when_no_transition(self):
        img = self.card({"/02/DISC.GDI": GDI3, "/03/DISC.CDI": b"cdi"})
        run = self.ui(img, "scan D+A t:2000", "--slots", "2,3")              # A wins: the game under the cursor, no move
        self.assertEqual(run.handoff, ["bios"], run.text)
        self.assertEqual([p[:11] for p in run.packets if p.startswith("52 82")], ["52 82 02 00"])
        run = self.ui(img, "scan D dump", "--slots", "2,3")
        self.assertEqual(run.screens[0][1][3][:1], ">")                        # a lone D still moves the cursor

    def test_c_session_end_fails_but_the_firmware_says_it_is_closed(self):
        img = self.card({"/02/DISC.GDI": GDI3})
        srv = NgtServe(self, img, "--idle-ms", "400", "--failend", "closed", "--dump")
        c = srv.client()
        c.hello()
        self.assertTrue(c.request(NGT_STATFS)[1])
        srv.finish(check=False)                                                  # the idle timer ends the session: the end fails
        self.assertIn("SERVE-END state=3", srv.text)                             # the list, not NM_S_FATAL (12)
        self.assertIn("session is closed", srv.text)

    def test_c_session_end_fails_and_the_session_is_still_open(self):
        img = self.card({"/02/DISC.GDI": GDI3})
        srv = NgtServe(self, img, "--idle-ms", "400", "--failend", "stuck", "--dump")
        c = srv.client()
        c.hello()
        self.assertTrue(c.request(NGT_STATFS)[1])
        srv.finish(check=False)
        self.assertIn("SERVE-END state=12", srv.text)
        self.assertIn("Could not close the session", srv.text)

    def test_d_b_that_stopped_an_operation_does_not_also_leave_the_screen(self):
        img = self.card({"/02/DISC.GDI": GDI3})
        text, src, files = self.gdi_set("03", sizes=(400000, 6000, 9000))
        srv = NgtServe(self, img, "--stopafter", "2", "--hold-b-ms", "2500")
        c = srv.client()
        c.hello()
        lines, ok, name = c.request(NGT_PUT, self.put_payload(text, mode=1), source=src)
        self.assertEqual((ok, name), (False, "ESTOP"))
        r = c.request(NGT_STATFS)                                                # B is still down: the screen must still be open
        self.assertTrue(r[1], r)
        time.sleep(2.7)                                                          # B is released, then a new press counts again
        lines, ok, name = c.request(NGT_PUT, self.put_payload(text, mode=1), source=src)
        self.assertTrue(ok, (lines, name))
        c.send(NGT_FINISH, bytes([1]))
        c.recv(), c.recv()
        srv.finish()


class TestPushArt(NetBase):
    """step 11 end to end: netgd-send push --art -> the card -> the menu shows the picture"""
    make_gdi = TestPush.make_gdi
    push = TestPush.push

    def test_push_art_png_then_the_menu_shows_it(self):
        img = self.card({"/02/DISC.GDI": GDI3})
        gdi, _tracks = self.make_gdi()
        png = os.path.join(self.work, "cover.png")
        with open(png, "wb") as f:
            f.write(solid_png(40, 30, (255, 0, 0)))
        srv = NgtServe(self, img, "--slots", "2")
        r = self.push(srv, gdi, "--slot", "3", "--art", png)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        srv.finish()
        fc = self.clean_fs(img)
        self.assertIn("netgd.art", [k.lower() for k in fc.folder_files("/03")])
        data = self.read(fc, "/03", "netgd.art")
        self.assertEqual(len(data), 16 + 128 * 128 * 2)
        px = artmod.parse(data)
        self.assertEqual(px[64 * 128 + 64], 0xF800)                                  # red in the middle
        self.assertEqual(px[0], 0)                                                   # 40 x 30 is wider than high: black bars above
        # the menu shows exactly these bytes
        run = self.ui(img, "scan, D, t:900, dump")
        a = art_lines(run)[0]
        self.assertEqual((a["shown"], a["crc"], a["dirty"]), ("1", art_crc(data), "0"), run.text)

    def test_push_a_finished_art_file_and_without_art(self):
        img = self.card({"/02/DISC.GDI": GDI3})
        gdi, _tracks = self.make_gdi()
        finished = art_file(11)
        p = os.path.join(self.work, "NETGD.ART")
        with open(p, "wb") as f:
            f.write(finished)
        srv = NgtServe(self, img, "--slots", "2")
        r = self.push(srv, gdi, "--slot", "3", "--art", p, "--no-finish")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        r = self.push(srv, self.make_gdi("src2", stem="other")[0], "--slot", "4")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        srv.finish()
        fc = self.clean_fs(img)
        self.assertEqual(self.read(fc, "/03", "netgd.art"), finished)
        self.assertNotIn("netgd.art", [k.lower() for k in fc.folder_files("/04")])


if __name__ == "__main__":
    print("scratch: %s; child limit %g s" % (WORK_ROOT, CHILD_TIMEOUT), file=sys.stderr)
    unittest.main(verbosity=2)
