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
import re
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

MENU_SRCS = ["src/nm_text.c", "src/nm_slot.c", "src/nm_cfgparse.c", "src/nm_ipbin.c", "src/nm_list.c", "src/nm_input.c",
             "src/nm_screen.c", "src/nm_scan.c", "src/nm_launch.c", "src/nm_ui.c"]
# the netgd-recv core and its test doubles (a disk image file, a packet-level firmware model); never main.c, xport_pc.c,
# dev_kos.c, fwxfer_kos.c, plat_kos.c
RECV_SRCS = ["src/crc32.c", "src/manifest.c", "src/ngblk.c", "src/fwctl.c", "src/ngfs.c", "src/recv_core.c", "src/diskio.c",
             "third_party/fatfs/ff.c", "third_party/fatfs/ffunicode.c", "third_party/fatfs/ffsystem.c",
             "test/dev_file.c", "test/fwctl_sim.c"]
TEST_SRCS = ["test/menu_host.c", "test/plat_host.c"]
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
        return build_recv_tool()
    cmd = ["gcc", "-O1", "-g", "-Wall", "-Werror", "-Wno-unused-function", "-Wno-format-truncation", "-DNG_HOST",
           "-Isrc", "-I../netgd-recv/src", "-I../netgd-recv/third_party/fatfs", "-I../netgd-recv/test", "-Itest",
           "-o", EXE] + SRCS + ["../netgd-recv/" + s for s in RECV_SRCS]
    r = run_child(cmd, cwd=ROOT, timeout=max(CHILD_TIMEOUT, 600))
    if r.returncode:
        raise RuntimeError("build failed:\n" + r.stdout + r.stderr)
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
        r = run_child([EXE, "selftest"])
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

    def ui(self, img, script, *opts, sectors=None):
        cmd = [EXE, "ui", img, str(sectors or self.SECTORS), "--script", script] + list(opts)
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
        # the firmware was asked its version (E0 03) and the GDEMU version (52 00), nothing else
        self.assertEqual([p.split()[0] for p in run.packets], ["E0", "52"], run.text)

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

    def test_no_controller_is_reported(self):
        img = self.card({"/02/DISC.GDI": GDI3})
        run = self.ui(img, "scan dump", "--slots", "2", "--nocontroller")
        self.assertEqual(run.rows[17], "No controller")


class TestLaunch(UiBase):
    """Design 11.2 L3-L7, L13, L14: the 0x52 mount, the READY wait and the hand-off."""

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
        self.assertEqual([p[:11] for p in run.packets[:2]], ["E0 03 00 00", "52 00 00 00"])
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


if __name__ == "__main__":
    print("scratch: %s; child limit %g s" % (WORK_ROOT, CHILD_TIMEOUT), file=sys.stderr)
    unittest.main(verbosity=2)
