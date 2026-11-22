"""Tests for tools/pinfind (mklist.py, pins2asf.py) and fpga/pinfind/gen.py's ball handling.

    python -m unittest discover -s tools/pinfind/tests

The pinmap here is SYNTHETIC: every ball name of a 16 x 16 BGA in the layout
`PIN_<ball> PIN_<other>`, a few lines with extra words for special functions,
and DDR_/NC_ lines. It is not Supra's file (which is not in the repository)
and only mimics the shape ag16k-supra-build.md describes.
"""
import contextlib
import io
import os
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, ".."))

import mklist  # noqa: E402
import pins2asf  # noqa: E402

ROWS = "ABCDEFGHJKLMNPRT"
REPO = os.path.abspath(os.path.join(HERE, "..", "..", ".."))


def fake_pinmap():
    lines = ["# synthetic test pinmap", "DDR_DQ0 X1", "NC_1 X2"]
    for r in ROWS:
        for c in range(1, 17):
            ball = f"PIN_{r}{c}"
            if ball == "PIN_C1":
                lines.append(f"{ball} PIN_Z99 ASDO")
            elif ball == "PIN_D2":
                lines.append(f"{ball} PIN_Z98 NCSO")
            elif ball in ("PIN_A16",):
                continue   # not bonded in this fake package
            else:
                lines.append(f"{ball} PIN_Z{r}{c}")
    return "\n".join(lines) + "\n"


def run_main(mod, argv):
    out, err = io.StringIO(), io.StringIO()
    code = 0
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        try:
            mod.main(argv)
        except SystemExit as e:
            code = e.code if isinstance(e.code, int) else 1
    return code, out.getvalue(), err.getvalue()


class MklistTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.pm = os.path.join(self.tmp.name, "AG16KDDF256.pinmap")
        with open(self.pm, "w") as f:
            f.write(fake_pinmap())

    def tearDown(self):
        self.tmp.cleanup()

    def make(self, *extra):
        out = os.path.join(self.tmp.name, "balls.txt")
        code, _, err = run_main(mklist, [self.pm, "-o", out, *extra])
        self.assertEqual(code, 0, err)
        return mklist.read_list(out)

    def test_config_jtag_ddr_clock_never_driven(self):
        b = self.make()
        never = set(mklist.CONFIG) | {c for c, _ in mklist.CLOCKS}
        self.assertFalse(never & set(b["drive"]), never & set(b["drive"]))
        # Dedicated balls are not in the design at all; the PS balls that
        # become user I/O (G1 G2 C1 D2) and the DDR balls are hold inputs.
        self.assertFalse(set(mklist.DEDICATED) & set(b["hold"]))
        for x in ("PIN_G1", "PIN_G2", "PIN_C1", "PIN_D2", "PIN_F10", "PIN_M10", "PIN_L11", "PIN_M11"):
            self.assertIn(x, b["hold"])
        self.assertEqual(b["clock"], [c for c, _ in mklist.CLOCKS])
        # Specials and unbonded balls are left out.
        for x in ("PIN_C1", "PIN_D2", "PIN_A16"):
            self.assertNotIn(x, b["drive"])
        # 256 balls - 1 unbonded - 17 config/JTAG/DDR (C1 D2 included) - 11 clock.
        self.assertEqual(len(b["drive"]), 256 - 1 - len(mklist.CONFIG) - len(mklist.CLOCKS))
        # Order follows the pinmap.
        self.assertEqual(b["drive"][:3], ["PIN_A1", "PIN_A2", "PIN_A3"])

    def test_exclude_moves_to_hold_and_keeps_order(self):
        b = self.make("--exclude", "A2,pin_b5", "--exclude", "C1")
        self.assertEqual(b["hold"][:2], ["PIN_A2", "PIN_B5"])
        self.assertNotIn("PIN_A2", b["drive"])
        self.assertEqual(b["drive"][:2], ["PIN_A1", "PIN_A3"])

    def test_design_covers_every_user_capable_ball(self):
        # What build.sh checks against the real pinmap: nothing that may be
        # user I/O is left out of the design.
        b = self.make()
        plain, special = mklist.parse_pinmap(fake_pinmap())
        design = set(b["drive"]) | set(b["hold"]) | set(b["clock"])
        self.assertEqual(set(mklist.user_capable(plain, special)) - design, set())
        self.assertFalse(set(mklist.DEDICATED) & design)

    def test_no_hold_ddr_and_leave_out(self):
        b = self.make("--no-hold-ddr", "--leave-out", "C1")
        for x in ("PIN_F10", "PIN_M10", "PIN_L11", "PIN_M11", "PIN_C1"):
            self.assertNotIn(x, b["hold"])
            self.assertNotIn(x, b["drive"])
        self.assertIn("PIN_G1", b["hold"])
        text = open(os.path.join(self.tmp.name, "balls.txt")).read()
        self.assertIn("--unused-ok PIN_C1", text)

    def test_max_drive(self):
        b = self.make("--max-drive", "4")
        self.assertEqual(b["drive"], ["PIN_A1", "PIN_A2", "PIN_A3", "PIN_A4"])
        self.assertIn("PIN_A5", b["hold"])

    def test_list_does_not_contain_pinmap_text(self):
        out = os.path.join(self.tmp.name, "balls.txt")
        run_main(mklist, [self.pm, "-o", out])
        text = open(out).read()
        self.assertNotIn("PIN_Z", text)     # second-column names are not copied
        self.assertNotIn("DDR_DQ0", text)
        self.assertNotIn("ASDO", text)

    def test_read_list_rejects_bad_files(self):
        p = os.path.join(self.tmp.name, "bad.txt")
        for body in ("drive 1 PIN_A1\n", "drive 0 PIN_A1\ndrive 1 PIN_A1\n", "foo 0 PIN_A1\n",
                     "drive 0 PIN_I1\n"):
            with open(p, "w") as f:
                f.write(body)
            with self.assertRaises(ValueError, msg=body):
                mklist.read_list(p)

    def test_too_many_drive_balls(self):
        with self.assertRaises(ValueError):
            mklist.make_list(["PIN_X%d" % i for i in range(300)], [], [], None)


class Pins2AsfTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.balls = os.path.join(self.tmp.name, "balls.txt")
        drive = [f"PIN_{r}{c}" for r in "ABCDE" for c in range(1, 16)]   # 75 balls
        lines = [f"drive {i} {b}" for i, b in enumerate(drive)]
        lines += ["clock 0 PIN_J4", "clock 1 PIN_T14"]
        with open(self.balls, "w") as f:
            f.write("\n".join(lines) + "\n")
        self.drive = drive

    def tearDown(self):
        self.tmp.cleanup()

    def write(self, name, text):
        p = os.path.join(self.tmp.name, name)
        with open(p, "w") as f:
            f.write(text)
        return p

    def full_inputs(self):
        """A complete, consistent set of results: MCU side from the built-in
        table, G1 side from a logic analyser, clocks from the probes."""
        mcu = []
        idx = 0
        mcu_ports = {}
        for pin, port in pins2asf.MCU_MAP.items():
            mcu.append(f"INFO  PF hit pin={pin} idx={idx} n=7  (pinfind.rs:1)")
            mcu_ports[port] = self.drive[idx]
            idx += 1
        mcu.append("PF hit pin=PA26 idx=60 n=3")            # wired, but no core port
        mcu.append("PF clk c=0 cnt=96 unit=1536 div=10 win=6 hz=47990000")
        mcu.append("PF clk c=1 cnt=22 unit=1536 div=10 win=6 hz=11280000")
        la, lamap = [], []
        dc_ports = [p for p in pins2asf.ports_from_file(os.path.join(REPO, "fpga/agm/pins/PROVISIONAL.asf"))
                    if p.startswith("DC_")]
        for n, port in enumerate(dc_ports):
            la.append(f"PF ch=D{n} idx={idx} n=5")
            lamap.append(f"D{n},{port}")
            idx += 1
        return ("\n".join(mcu) + "\n", "\n".join(la) + "\n", "\n".join(lamap) + "\n")

    def test_complete_result_gives_a_real_pin_file(self):
        mcu, la, lamap = self.full_inputs()
        out = os.path.join(self.tmp.name, "gdemu60.asf")
        code, _, err = run_main(pins2asf, [
            "--balls", self.balls, "--mcu", self.write("rtt.log", mcu),
            "--la", self.write("la.txt", la), "--la-map", self.write("lamap.csv", lamap),
            "--clock-auto", "-o", out])
        self.assertEqual(code, 0, err)
        text = open(out).read()
        self.assertNotIn(pins2asf.MARKER, text)
        lines = [l for l in text.splitlines() if l.startswith("set_location_assignment")]
        self.assertEqual(len(lines), 66)
        self.assertIn("set_location_assignment -to CLK_48_MHz PIN_J4", lines)
        self.assertIn("set_location_assignment -to CLK_11_2896_MHz PIN_T14", lines)
        self.assertIn("set_location_assignment -to {MCU_DATA[0]} PIN_A1", lines)   # PB9 = idx 0
        self.assertIn("PA26", text)   # reported as a note
        # The same check fpga/agm/build.sh makes: every port exactly once, same spelling.
        for port in open(os.path.join(REPO, "fpga/agm/pins/PROVISIONAL.asf"), encoding="utf-8").read().splitlines():
            if port.startswith("set_location_assignment -to "):
                key = port.rsplit(" ", 1)[0] + " "
                self.assertEqual(sum(1 for l in lines if l.startswith(key)), 1, key)

    def test_partial_result_is_marked_provisional(self):
        out = os.path.join(self.tmp.name, "part.asf")
        mcu = "PF hit pin=PB9 idx=3 n=9\n"
        code, _, _ = run_main(pins2asf, ["--balls", self.balls, "--mcu", self.write("r.log", mcu), "-o", out])
        self.assertEqual(code, 2)
        self.assertIn(pins2asf.MARKER, open(out).read())
        code, _, _ = run_main(pins2asf, ["--balls", self.balls, "--mcu", self.write("r.log", mcu),
                                         "-o", out, "--allow-partial"])
        self.assertEqual(code, 0)
        self.assertIn(pins2asf.MARKER, open(out).read())

    def test_conflicts_are_errors(self):
        # Two MCU pins that map to two ports land on one ball.
        mcu = "PF hit pin=PB9 idx=3 n=9\nPF hit pin=PB10 idx=3 n=9\n"
        code, _, err = run_main(pins2asf, ["--balls", self.balls, "--mcu", self.write("r.log", mcu)])
        self.assertEqual(code, 1)
        self.assertIn("assigned to 2 ports", err)
        # A pin that saw two balls about equally.
        mcu = "PF hit pin=PB9 idx=3 n=5\nPF hit pin=PB9 idx=4 n=5\n"
        code, _, err = run_main(pins2asf, ["--balls", self.balls, "--mcu", self.write("r.log", mcu)])
        self.assertEqual(code, 1)
        self.assertIn("no dominant index", err)
        # --set disagreeing with a measurement.
        mcu = "PF hit pin=PB9 idx=3 n=9\n"
        code, _, err = run_main(pins2asf, ["--balls", self.balls, "--mcu", self.write("r.log", mcu),
                                           "--set", "MCU_DATA[0]=PIN_E9"])
        self.assertEqual(code, 1)

    def test_repeated_summaries_are_not_added_up(self):
        # Cumulative summaries: 3 then 9 for idx 3, a stray 2 for idx 4.
        # Added up that would be 12 : 2 (86 %); as a maximum it is 9 : 2 (82 %).
        mcu = ["PF hit pin=PB9 idx=3 n=3", "PF hit pin=PB9 idx=3 n=9", "PF hit pin=PB9 idx=4 n=2"]
        t = pins2asf.tally(mcu, pins2asf.HIT)
        self.assertEqual(t, {"PB9": {3: 9, 4: 2}})

    def test_index_beyond_list_means_wrong_list(self):
        mcu = "PF hit pin=PB9 idx=200 n=9\n"
        code, _, err = run_main(pins2asf, ["--balls", self.balls, "--mcu", self.write("r.log", mcu)])
        self.assertEqual(code, 1)
        self.assertIn("wrong list", err)

    def test_ports_from_verilog_match_the_asf(self):
        v = pins2asf.ports_from_file(os.path.join(REPO, "fpga/agm/src/OpenGDEMU.v"))
        a = pins2asf.ports_from_file(os.path.join(REPO, "fpga/agm/pins/PROVISIONAL.asf"))
        self.assertEqual(sorted(v), sorted(a))
        self.assertEqual(len(v), 66)


if __name__ == "__main__":
    unittest.main()
