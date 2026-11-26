"""Tests for ep2c5gate.py --scan and tools/pinfind/mk_scan.py, with SYNTHETIC
.pin / fit.rpt snippets (layout of Quartus II 13.0 reports, not copied).

    python -m unittest tools/pinfind/tests/test_ep2c5_scan.py
"""
import contextlib
import csv
import io
import os
import re
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, ".."))
sys.path.insert(0, HERE)

import ep2c5gate  # noqa: E402
import mk_scan  # noqa: E402
from test_ep2c5_pullcode import BALLS, DIRS, OPTIONS, PIN_HEAD  # noqa: E402

PADMAP = os.path.join(HERE, "..", "ep2c5_padmap.csv")
ROWS = mk_scan.build(PADMAP)
TAB = {r["ball"]: r for r in ROWS}
OUTS = [r["ball"] for r in ROWS if r["dir"] == "output"]
SUMMARY = """+------+
; Fitter Summary ;
+------+------+
; Fitter Status ; Successful ;
; Total logic elements ; 2,000 / 4,608 ( 43 % ) ;
; Total PLLs ; {pll} / 2 ( 0 % ) ;
+------+------+
"""


def pin_text(override=None):
    override = override or {}
    out = [PIN_HEAD]
    for ball, role in BALLS.items():
        if ball in TAB:
            row = [f"p_{ball}", ball, TAB[ball]["dir"], "3.3-V LVTTL", "1", "Y"]
        elif role == "GND" or role.startswith(("GND_PLL", "GNDA_PLL")):
            row = [role, ball, "gnd", "", "", ""]
        elif role.startswith("VCC"):
            row = [role, ball, "power", "", "", ""]
        else:
            row = [role, ball, DIRS.get(role, ""), "", "", ""]
        if ball in override:
            row[0], row[2] = override[ball]
        out.append(f"{row[0]:<29}: {row[1]:<10}: {row[2]:<7}: {row[3]:<18}:"
                   f"         : {row[4]:<10}: {row[5]}\n")
    return "".join(out)


def pu(b):
    return "On" if TAB[b]["pullup"] == "on" else "Off"


def fit_text(inpu=None, outpu=None, pkgpu=None, extra_out=(), drop_out=(), pll=0,
             summary=True):
    inpu, outpu, pkgpu = inpu or {}, outpu or {}, pkgpu or {}
    L = ["+------+", "; Input Pins ;", "+------+",
         "; Name ; Pin # ; I/O Bank ; Bus Hold ; Weak Pull Up ; I/O Standard ;", "+------+"]
    for b in sorted(TAB):
        if TAB[b]["dir"] == "input":
            L.append(f"; p_{b} ; {b} ; 1 ; no ; {inpu.get(b, pu(b))} ; 3.3-V LVTTL ;")
    L += ["+------+", "", "+------+", "; Output Pins ;", "+------+",
          "; Name ; Pin # ; I/O Bank ; TRI Primitive ; Bus Hold ; Weak Pull Up ; I/O Standard ;",
          "+------+"]
    for b in OUTS:
        if b not in drop_out:
            L.append(f"; p_{b} ; {b} ; 4 ; yes ; no ; {outpu.get(b, pu(b))} ; 3.3-V LVTTL ;")
    for name, b in extra_out:
        L.append(f"; {name} ; {b} ; 4 ; no ; no ; Off ; 3.3-V LVTTL ;")
    L += ["+------+", "", "+------+", "; All Package Pins ;", "+------+",
          "; Location ; Pad Number ; I/O Bank ; Pin Name/Usage ; Dir. ; I/O Standard ;"
          " Voltage ; I/O Type ; User Assignment ; Bus Hold ; Weak Pull Up ;", "+------+"]
    for ball, role in BALLS.items():
        if ball in TAB:
            L.append(f"; {ball} ; 1 ; 1 ; p_{ball} ; {TAB[ball]['dir']} ; 3.3-V LVTTL ; ;"
                     f" Row I/O ; Y ; no ; {pkgpu.get(ball, pu(ball))} ;")
        else:
            d = "output" if role == "TDO" else ""
            n = ("#" + role) if role == "TDO" else role
            L.append(f"; {ball} ; ; ; {n} ; {d} ; ; ; -- ; ; -- ; -- ;")
    L += ["+------+", ""]
    if summary:
        L.append(SUMMARY.format(pll=pll))
    L.append(OPTIONS)
    return "\n".join(L)


def table_csv():
    s = io.StringIO()
    w = csv.DictWriter(s, fieldnames=mk_scan.COLS, lineterminator="\n")
    w.writeheader()
    w.writerows(ROWS)
    return s.getvalue()


class ScanGateTest(unittest.TestCase):
    def run_gate(self, pin=None, fit=None):
        with tempfile.TemporaryDirectory() as d:
            for name, text in (("scan.pin", pin or pin_text()),
                               ("scan.fit.rpt", fit or fit_text()),
                               ("scan.csv", table_csv())):
                with open(os.path.join(d, name), "w", encoding="utf-8") as f:
                    f.write(text)
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                rc = ep2c5gate.main([d, "--scan", "--table", os.path.join(d, "scan.csv"), "-q"])
            return rc, out.getvalue()

    def test_good_passes(self):
        rc, out = self.run_gate()
        self.assertEqual(rc, 0, out)

    def test_extra_output_fails(self):
        rc, out = self.run_gate(fit=fit_text(extra_out=[("p_A3", "A3")]))
        self.assertEqual(rc, 1, out)

    def test_missing_output_fails(self):
        rc, out = self.run_gate(fit=fit_text(drop_out={OUTS[0]}))
        self.assertEqual(rc, 1, out)
        self.assertIn("not in the Output Pins", out)

    def test_input_as_output_in_pin_fails(self):
        self.assertEqual(self.run_gate(pin=pin_text({"A3": ("p_A3", "output")}))[0], 1)

    def test_data_ball_as_input_fails(self):
        b = OUTS[0]
        self.assertEqual(self.run_gate(pin=pin_text({b: (f"p_{b}", "input")}))[0], 1)

    def test_pullup_off_on_io_fails(self):
        self.assertEqual(self.run_gate(fit=fit_text(inpu={"A3": "Off"}))[0], 1)
        self.assertEqual(self.run_gate(fit=fit_text(pkgpu={"A3": "Off"}))[0], 1)

    def test_pullup_on_clock_fails(self):
        self.assertEqual(self.run_gate(fit=fit_text(inpu={"H16": "On"}))[0], 1)

    def test_pullup_on_output_fails(self):
        self.assertEqual(self.run_gate(fit=fit_text(outpu={OUTS[3]: "On"}))[0], 1)

    def test_pll_fails(self):
        rc, out = self.run_gate(fit=fit_text(pll=1))
        self.assertEqual(rc, 1, out)
        self.assertIn("PLLs used", out)

    def test_pll_missing_fails(self):
        self.assertEqual(self.run_gate(fit=fit_text(summary=False))[0], 1)


class MkScanTest(unittest.TestCase):
    def test_counts_and_data_balls(self):
        self.assertEqual(len(ROWS), 158)
        want = "M11 L11 T14 R13 R12 N12 T7 T8 K1 K4 K5 J4 P3 L4 M4 T12".split()
        self.assertEqual([r["ball"] for r in mk_scan.by(ROWS, "d_bit")], want)
        self.assertEqual(sorted(OUTS), sorted(want))
        self.assertEqual(sum(r["pullup"] == "on" for r in ROWS), 134)
        for r in ROWS:
            if r["kind"] == "clock":
                self.assertEqual(r["pullup"], "off")
        self.assertEqual([r["ball"] for r in mk_scan.by(ROWS, "clk")], mk_scan.CLOCKS)
        self.assertEqual(len(mk_scan.by(ROWS, "raw")), 142)
        self.assertEqual(len(mk_scan.by(ROWS, "dc")), 33)
        addr = [mk_scan.sig_ball(ROWS, s) for s in mk_scan.MCU_ADDR]
        self.assertEqual(addr, "R11 R5 T5 R4 T4 P4 P5 T3".split())

    def test_committed_files_match_generator(self):
        root = os.path.normpath(os.path.join(HERE, "..", "..", ".."))
        with open(os.path.join(HERE, "..", "ep2c5_scan.csv"), encoding="utf-8") as f:
            self.assertEqual(list(csv.DictReader(f)), ROWS)
        out = os.path.join(root, "fpga", "scan-ep2c5")
        with open(os.path.join(out, "scan.v"), encoding="utf-8") as f:
            self.assertEqual(f.read(), mk_scan.verilog_text(ROWS))
        with open(os.path.join(out, "scan.qsf"), encoding="utf-8") as f:
            q = f.read()
        self.assertEqual(q, mk_scan.qsf_text(ROWS))
        on = set(re.findall(r"WEAK_PULL_UP_RESISTOR ON -to p_(\S+)", q))
        self.assertEqual(on, {r["ball"] for r in ROWS if r["pullup"] == "on"})
        self.assertNotIn("PLL", re.sub(r"#.*", "", q))
        for opt in ("ENABLE_INIT_DONE_OUTPUT OFF", "CRC_ERROR_CHECKING OFF",
                    'CYCLONEII_RESERVE_NCEO_AFTER_CONFIGURATION "USE AS REGULAR IO"',
                    'CYCLONEII_CONFIGURATION_SCHEME "PASSIVE SERIAL"',
                    "ON_CHIP_BITSTREAM_DECOMPRESSION ON",
                    'RESERVE_ALL_UNUSED_PINS "AS INPUT TRI-STATED"'):
            self.assertIn(opt, q)
        self.assertEqual(len(re.findall(r"ENABLE_BUS_HOLD_CIRCUITRY OFF", q)), 158)

    def test_core_has_no_pll_or_write_path(self):
        root = os.path.normpath(os.path.join(HERE, "..", "..", ".."))
        with open(os.path.join(root, "fpga", "scan-ep2c5", "scan_core.v"), encoding="utf-8") as f:
            v = f.read()
        self.assertNotRegex(v, r"altpll|inout")
        self.assertIn("assign d_oe = ~ncs & ~nrd;", v)


if __name__ == "__main__":
    unittest.main()
