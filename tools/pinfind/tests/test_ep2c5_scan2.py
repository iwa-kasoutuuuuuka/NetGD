"""Tests for ep2c5gate.py --scan2 K and tools/pinfind/mk_scan2.py, with
SYNTHETIC .pin / fit.rpt snippets (layout of Quartus II 13.0 reports, not
copied).

    python -m unittest tools/pinfind/tests/test_ep2c5_scan2.py
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
import mk_scan2  # noqa: E402
from test_ep2c5_pullcode import BALLS, DIRS, OPTIONS, PIN_HEAD  # noqa: E402

PADMAP = os.path.join(HERE, "..", "ep2c5_padmap.csv")
ROOT = os.path.normpath(os.path.join(HERE, "..", "..", ".."))
ROWS = {k: mk_scan2.build(PADMAP, k) for k in mk_scan2.VARIANTS}
BASE = mk_scan.build(PADMAP)
SUMMARY = """+------+
; Fitter Summary ;
+------+------+
; Fitter Status ; Successful ;
; Total PLLs ; {pll} / 2 ( 0 % ) ;
+------+------+
"""


def tab(k):
    return {r["ball"]: r for r in ROWS[k]}


def pin_text(k, override=None):
    t = tab(k)
    override = override or {}
    out = [PIN_HEAD]
    for ball, role in BALLS.items():
        if ball in t:
            row = [f"p_{ball}", ball, t[ball]["dir"], "3.3-V LVTTL", "1", "Y"]
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


def fit_text(k, inpu=None, inbh=None, pkgbh=None, extra_out=(), pll=0):
    t = tab(k)
    inpu, inbh, pkgbh = inpu or {}, inbh or {}, pkgbh or {}

    def pu(b):
        return "On" if t[b]["pullup"] == "on" else "Off"

    def bh(b):
        return "yes" if t[b]["bushold"] == "on" else "no"
    L = ["+------+", "; Input Pins ;", "+------+",
         "; Name ; Pin # ; I/O Bank ; Bus Hold ; Weak Pull Up ; I/O Standard ;", "+------+"]
    for b in sorted(t):
        if t[b]["dir"] == "input":
            L.append(f"; p_{b} ; {b} ; 1 ; {inbh.get(b, bh(b))} ; {inpu.get(b, pu(b))} ;"
                     " 3.3-V LVTTL ;")
    L += ["+------+", "", "+------+", "; Output Pins ;", "+------+",
          "; Name ; Pin # ; I/O Bank ; TRI Primitive ; Bus Hold ; Weak Pull Up ; I/O Standard ;",
          "+------+"]
    for b in sorted(t):
        if t[b]["dir"] == "output":
            L.append(f"; p_{b} ; {b} ; 4 ; yes ; no ; Off ; 3.3-V LVTTL ;")
    for name, b in extra_out:
        L.append(f"; {name} ; {b} ; 4 ; no ; no ; Off ; 3.3-V LVTTL ;")
    L += ["+------+", "", "+------+", "; All Package Pins ;", "+------+",
          "; Location ; Pad Number ; I/O Bank ; Pin Name/Usage ; Dir. ; I/O Standard ;"
          " Voltage ; I/O Type ; User Assignment ; Bus Hold ; Weak Pull Up ;", "+------+"]
    for ball, role in BALLS.items():
        if ball in t:
            L.append(f"; {ball} ; 1 ; 1 ; p_{ball} ; {t[ball]['dir']} ; 3.3-V LVTTL ; ;"
                     f" Row I/O ; Y ; {pkgbh.get(ball, bh(ball))} ; {pu(ball)} ;")
        else:
            d = "output" if role == "TDO" else ""
            n = ("#" + role) if role == "TDO" else role
            L.append(f"; {ball} ; ; ; {n} ; {d} ; ; ; -- ; ; -- ; -- ;")
    L += ["+------+", "", SUMMARY.format(pll=pll), OPTIONS]
    return "\n".join(L)


def table_csv(rows, cols=mk_scan2.COLS):
    s = io.StringIO()
    w = csv.DictWriter(s, fieldnames=cols, lineterminator="\n")
    w.writeheader()
    w.writerows(rows)
    return s.getvalue()


class Scan2GateTest(unittest.TestCase):
    def run_gate(self, k, pin=None, fit=None, rows=None, gate_k=None):
        with tempfile.TemporaryDirectory() as d:
            for name, text in (("scan2.pin", pin or pin_text(k)),
                               ("scan2.fit.rpt", fit or fit_text(k)),
                               ("v.csv", table_csv(rows or ROWS[k])),
                               ("base.csv", table_csv(BASE, mk_scan.COLS))):
                with open(os.path.join(d, name), "w", encoding="utf-8") as f:
                    f.write(text)
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                rc = ep2c5gate.main([d, "--scan2", str(k if gate_k is None else gate_k),
                                     "--table", os.path.join(d, "v.csv"),
                                     "--base", os.path.join(d, "base.csv"), "-q"])
            return rc, out.getvalue()

    def test_good_variants_pass(self):
        for k in mk_scan2.VARIANTS:
            rc, out = self.run_gate(k)
            self.assertEqual(rc, 0, out)

    def test_committed_base_table_passes(self):
        for k in mk_scan2.VARIANTS:
            g = ep2c5gate.Gate()
            ep2c5gate.check_scan2_table(g, k, os.path.join(HERE, "..", f"ep2c5_scan2_v{k}.csv"),
                                        os.path.join(HERE, "..", "ep2c5_scan.csv"))
            self.assertEqual(g.errors, [], k)

    def test_wrong_variant_fails(self):
        rc, out = self.run_gate(1, gate_k=2)
        self.assertEqual(rc, 1, out)
        self.assertIn("scan2 table v2", out)

    def test_bus_hold_missing_fails(self):
        b4 = "B4"
        rc, out = self.run_gate(2, fit=fit_text(2, inbh={b4: "no"}))
        self.assertEqual(rc, 1, out)
        self.assertIn("bus hold no, expected on", out)
        self.assertEqual(self.run_gate(2, fit=fit_text(2, pkgbh={b4: "no"}))[0], 1)

    def test_bus_hold_extra_fails(self):
        self.assertEqual(self.run_gate(1, fit=fit_text(1, inbh={"B4": "yes"}))[0], 1)
        self.assertEqual(self.run_gate(2, fit=fit_text(2, pkgbh={"C3": "yes"}))[0], 1)

    def test_pullup_on_released_ball_fails(self):
        self.assertEqual(self.run_gate(1, fit=fit_text(1, inpu={"C4": "On"}))[0], 1)
        self.assertEqual(self.run_gate(1, fit=fit_text(1, inpu={"B4": "On"}))[0], 1)

    def test_extra_output_fails(self):
        rc, out = self.run_gate(0, fit=fit_text(0, extra_out=[("p_B4", "B4")]))
        self.assertEqual(rc, 1, out)

    def test_input_as_output_fails(self):
        self.assertEqual(self.run_gate(0, pin=pin_text(0, {"B4": ("p_B4", "output")}))[0], 1)

    def test_pll_fails(self):
        self.assertEqual(self.run_gate(1, fit=fit_text(1, pll=1))[0], 1)

    def test_table_extra_difference_fails(self):
        rows = [dict(r) for r in ROWS[1]]
        for r in rows:
            if r["ball"] == "C3":
                r["pullup"] = "off"
        rc, out = self.run_gate(1, rows=rows, fit=fit_text(1, inpu={"C3": "Off"}))
        self.assertEqual(rc, 1, out)
        self.assertIn("(unexpected)", out)

    def test_table_extra_output_fails(self):
        rows = [dict(r) for r in ROWS[0]]
        for r in rows:
            if r["ball"] == "B4":
                r["dir"] = "output"
        rc, out = self.run_gate(0, rows=rows)
        self.assertEqual(rc, 1, out)


class MkScan2Test(unittest.TestCase):
    def test_variant_sets(self):
        dc = {r["ball"] for r in BASE if r["dc"] != ""}
        self.assertEqual(len(dc), 33)
        self.assertIn("B4", dc)
        for k in mk_scan2.VARIANTS:
            rows = ROWS[k]
            self.assertEqual(len(rows), 158)
            self.assertEqual([r["ball"] for r in rows if r["dir"] == "output"],
                             [r["ball"] for r in BASE if r["dir"] == "output"])
            diff = {r["ball"] for r, b in zip(rows, BASE)
                    if (r["pullup"], r["bushold"]) != (b["pullup"], "off")}
            want = {0: set(), 1: dc | {"C4", "C5"}, 2: dc}[k]
            self.assertEqual(diff, want, k)
            for r, b in zip(rows, BASE):
                for c in mk_scan.COLS:
                    if c != "pullup":
                        self.assertEqual(r[c], b[c])
        self.assertEqual(sum(r["bushold"] == "on" for r in ROWS[2]), 33)
        self.assertEqual(sum(r["pullup"] == "on" for r in ROWS[1]), 134 - 35)

    def test_committed_files_match_generator(self):
        out = os.path.join(ROOT, "fpga", "scan2-ep2c5")
        for k in mk_scan2.VARIANTS:
            with open(os.path.join(HERE, "..", f"ep2c5_scan2_v{k}.csv"), encoding="utf-8") as f:
                self.assertEqual(list(csv.DictReader(f)), ROWS[k])
            with open(os.path.join(out, f"scan2_v{k}.qsf"), encoding="utf-8") as f:
                q = f.read()
            self.assertEqual(q, mk_scan2.qsf_text(ROWS[k], k))
            on = set(re.findall(r"WEAK_PULL_UP_RESISTOR ON -to p_(\S+)", q))
            self.assertEqual(on, {r["ball"] for r in ROWS[k] if r["pullup"] == "on"})
            bh = set(re.findall(r"ENABLE_BUS_HOLD_CIRCUITRY ON -to p_(\S+)", q))
            self.assertEqual(bh, {r["ball"] for r in ROWS[k] if r["bushold"] == "on"})
            self.assertNotIn("PLL", re.sub(r"#.*", "", q))
            for opt in ("ENABLE_INIT_DONE_OUTPUT OFF", "CRC_ERROR_CHECKING OFF",
                        'CYCLONEII_RESERVE_NCEO_AFTER_CONFIGURATION "USE AS REGULAR IO"',
                        'CYCLONEII_CONFIGURATION_SCHEME "PASSIVE SERIAL"',
                        "ON_CHIP_BITSTREAM_DECOMPRESSION ON",
                        'RESERVE_ALL_UNUSED_PINS "AS INPUT TRI-STATED"',
                        "TOP_LEVEL_ENTITY scan2"):
                self.assertIn(opt, q)
        with open(os.path.join(out, "scan2.v"), encoding="utf-8") as f:
            self.assertEqual(f.read(), mk_scan2.verilog_text(ROWS[0]))

    def test_wrapper_outputs_only_mcu_data(self):
        v = mk_scan2.verilog_text(ROWS[0])
        outs = re.findall(r"^\s+output wire (p_\w+)", v, re.M)
        self.assertEqual(sorted(outs), sorted(f"p_{r['ball']}" for r in BASE
                                              if r["dir"] == "output"))
        self.assertIn(".c4(p_C4)", v)
        self.assertEqual(len(re.findall(r"= d_oe \? d_out\[\d+\] : 1'bz;", v)), 16)

    def test_core_has_no_pll_inout_or_extra_outputs(self):
        d = os.path.join(ROOT, "fpga", "scan2-ep2c5")
        for name in ("scan2_core.v", "g1x.v"):
            with open(os.path.join(d, name), encoding="utf-8") as f:
                v = re.sub(r"//.*", "", f.read())
            self.assertNotRegex(v, r"altpll|inout|1'bz")
        with open(os.path.join(d, "scan2_core.v"), encoding="utf-8") as f:
            v = f.read()
        self.assertIn("scan_core #(.WIN_LOG2(WIN_LOG2)) sc", v)
        # d_oe comes from scan_core unchanged
        self.assertIn(".d_out(d_s), .d_oe(d_oe));", v)


if __name__ == "__main__":
    unittest.main()
