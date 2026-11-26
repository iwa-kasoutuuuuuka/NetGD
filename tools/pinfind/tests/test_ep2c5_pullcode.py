"""Tests for the weak pull-up code mode of tools/pinfind/ep2c5gate.py and
for tools/pinfind/mk_pullcode.py, with SYNTHETIC .pin / fit.rpt snippets.

    python -m unittest tools/pinfind/tests/test_ep2c5_pullcode.py

The snippets only mimic the layout of Quartus II 13.0 reports; they are not
copied from a real compile.
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

import ep2c5gate  # noqa: E402
import mk_pullcode  # noqa: E402

BALLS = ep2c5gate.load_balls()
PORT_BALLS = sorted(b for b, r in BALLS.items() if r in ep2c5gate.PORT_ROLES)
CLOCKS = sorted(b for b, r in BALLS.items() if r in ("GND+", "USER"))
DIRS = {"TDO": "output", "TDI": "input", "TMS": "input", "TCK": "input",
        "DATA0": "input"}

PIN_HEAD = """ -- synthetic
CHIP  "pullcode"  ASSIGNED TO AN: EP2C5F256C8

Pin Name/Usage               : Location  : Dir.   : I/O Standard      : Voltage : I/O Bank  : User Assignment
-------------------------------------------------------------------------------------------------------------
"""

OPTIONS = """+-----------------------------------------------------------+
; Fitter Device Options                                     ;
+-----------------------------------------------+-----------+
; Option                                        ; Setting   ;
+-----------------------------------------------+-----------+
; Enable user-supplied start-up clock (CLKUSR)  ; Off       ;
; Enable device-wide reset (DEV_CLRn)           ; Off       ;
; Enable device-wide output enable (DEV_OE)     ; Off       ;
; Enable INIT_DONE output                       ; Off       ;
; Configuration scheme                          ; Passive Serial ;
; Error detection CRC                           ; Off       ;
; nCEO                                          ; Unreserved ;
; ASDO,nCSO                                     ; Unreserved ;
; Reserve all unused pins                       ; As input tri-stated ;
+-----------------------------------------------+-----------+
"""


def synth_code(with_clocks=False):
    """{ball: codeword} for the ball table's port balls (clocks uncoded)."""
    coded = [b for b in PORT_BALLS if with_clocks or b not in CLOCKS]
    words = mk_pullcode.assign_codes(len(coded))
    code = {b: "" for b in PORT_BALLS}
    code.update(zip(coded, words))
    return code


def code_csv(code):
    s = io.StringIO()
    w = csv.writer(s, lineterminator="\n")
    w.writerow(["ball", "pad", "bank", "io_type", "kind", "in_code", "code", "note"])
    for i, (b, c) in enumerate(sorted(code.items())):
        w.writerow([b, i, 1, "Row I/O", "clock" if b in CLOCKS else "io",
                    "yes" if c else "no", c, ""])
    return s.getvalue()


def pin_text(k, code, override=None):
    override = override or {}
    out = [PIN_HEAD]
    for ball, role in BALLS.items():
        if ball in code:
            row = [f"p_{ball}", ball, "input", "3.3-V LVTTL", "1", "Y"]
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


def pu(code, b, k):
    return "On" if code[b] and code[b][k] == "1" else "Off"


def fit_text(k, code, inpu=None, pkgpu=None, pkgbh=None, drop_input=(),
             drop_package=False, extra=""):
    inpu, pkgpu, pkgbh = inpu or {}, pkgpu or {}, pkgbh or {}
    L = ["+------+", "; Input Pins ;", "+------+",
         "; Name ; Pin # ; I/O Bank ; Bus Hold ; Weak Pull Up ; I/O Standard ;",
         "+------+"]
    for b in sorted(code):
        if b in drop_input:
            continue
        L.append(f"; p_{b} ; {b} ; 1 ; no ; {inpu.get(b, pu(code, b, k))} ; 3.3-V LVTTL ;")
    L += ["+------+", ""]
    if not drop_package:
        L += ["+------+", "; All Package Pins ;", "+------+",
              "; Location ; Pad Number ; I/O Bank ; Pin Name/Usage ; Dir. ; I/O Standard ;"
              " Voltage ; I/O Type ; User Assignment ; Bus Hold ; Weak Pull Up ;",
              "+------+"]
        for ball, role in BALLS.items():
            if ball in code:
                L.append(f"; {ball} ; 1 ; 1 ; p_{ball} ; input ; 3.3-V LVTTL ; ; Row I/O ; Y ;"
                         f" {pkgbh.get(ball, 'no')} ; {pkgpu.get(ball, pu(code, ball, k))} ;")
            else:
                d = "output" if role == "TDO" else ""
                n = ("#" + role) if role == "TDO" else role
                L.append(f"; {ball} ; ; ; {n} ; {d} ; ; ; -- ; ; -- ; -- ;")
        L += ["+------+", ""]
    L += [OPTIONS, extra]
    return "\n".join(L)


class PullcodeGateTest(unittest.TestCase):
    CODE = synth_code()

    def run_gate(self, k, pin=None, fit=None, code=None, gate_k=None):
        code = code or self.CODE
        pin = pin if pin is not None else pin_text(k, code)
        fit = fit if fit is not None else fit_text(k, code)
        with tempfile.TemporaryDirectory() as d:
            for name, text in (("pullcode.pin", pin), ("pullcode.fit.rpt", fit),
                               ("code.csv", code_csv(code))):
                with open(os.path.join(d, name), "w", encoding="utf-8") as f:
                    f.write(text)
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                rc = ep2c5gate.main([d, "--pullcode", str(k if gate_k is None else gate_k),
                                     "--code", os.path.join(d, "code.csv"), "-q"])
            return rc, out.getvalue()

    def coded(self, k, bit, n=0):
        return [b for b in sorted(self.CODE) if self.CODE[b] and self.CODE[b][k] == bit][n]

    def test_good_passes_every_k(self):
        for k in range(10):
            rc, out = self.run_gate(k)
            self.assertEqual(rc, 0, f"k={k}\n{out}")

    def test_wrong_k_fails(self):
        rc, out = self.run_gate(0, gate_k=1)
        self.assertEqual(rc, 1, out)

    def test_input_pins_pullup_missing_fails(self):
        b = self.coded(3, "1")
        rc, out = self.run_gate(3, fit=fit_text(3, self.CODE, inpu={b: "Off"}))
        self.assertEqual(rc, 1, out)
        self.assertIn(f"p_{b}", out)

    def test_input_pins_extra_pullup_fails(self):
        b = self.coded(3, "0")
        self.assertEqual(self.run_gate(3, fit=fit_text(3, self.CODE, inpu={b: "On"}))[0], 1)

    def test_package_pullup_mismatch_fails(self):
        b = self.coded(5, "1")
        rc, out = self.run_gate(5, fit=fit_text(5, self.CODE, pkgpu={b: "Off"}))
        self.assertEqual(rc, 1, out)
        self.assertIn(f"package pin {b}", out)

    def test_package_bus_hold_fails(self):
        b = self.coded(5, "0")
        self.assertEqual(self.run_gate(5, fit=fit_text(5, self.CODE, pkgbh={b: "yes"}))[0], 1)

    def test_uncoded_clock_pullup_fails(self):
        c = CLOCKS[0]
        self.assertEqual(self.CODE[c], "")
        self.assertEqual(self.run_gate(0, fit=fit_text(0, self.CODE, inpu={c: "On"}))[0], 1)

    def test_input_pin_missing_fails(self):
        b = self.coded(0, "1")
        rc, out = self.run_gate(0, fit=fit_text(0, self.CODE, drop_input={b}))
        self.assertEqual(rc, 1, out)
        self.assertIn("not in the Input Pins table", out)

    def test_package_table_missing_fails(self):
        rc, out = self.run_gate(0, fit=fit_text(0, self.CODE, drop_package=True))
        self.assertEqual(rc, 1, out)
        self.assertIn("All Package Pins table missing", out)

    def test_code_ball_left_reserved_fails(self):
        b = self.coded(0, "0")
        pin = pin_text(0, self.CODE, override={b: ("RESERVED_INPUT", "")})
        self.assertEqual(self.run_gate(0, pin=pin)[0], 1)

    def test_code_port_bidir_fails(self):
        b = self.coded(0, "0")
        pin = pin_text(0, self.CODE, override={b: (f"p_{b}", "bidir")})
        self.assertEqual(self.run_gate(0, pin=pin)[0], 1)

    def test_output_table_fails(self):
        extra = "+------+\n; Output Pins ;\n+------+\n; Name ; Pin # ;\n+------+\n; led ; A6 ;\n+------+\n"
        self.assertEqual(self.run_gate(0, fit=fit_text(0, self.CODE, extra=extra))[0], 1)

    def test_bidir_table_fails(self):
        extra = "+------+\n; Bidir Pins ;\n+------+\n; Name ; Pin # ;\n+------+\n; d0 ; A6 ;\n+------+\n"
        self.assertEqual(self.run_gate(0, fit=fit_text(0, self.CODE, extra=extra))[0], 1)

    def test_option_init_done_on_fails(self):
        fit = fit_text(0, self.CODE).replace(
            "; Enable INIT_DONE output                       ; Off       ;",
            "; Enable INIT_DONE output                       ; On        ;")
        self.assertEqual(self.run_gate(0, fit=fit)[0], 1)

    def test_code_table_missing_port_ball_fails(self):
        code = dict(self.CODE)
        del code[self.coded(0, "1")]
        self.assertEqual(self.run_gate(0, code=code)[0], 1)

    def test_bad_codeword_weight_rejected(self):
        code = dict(self.CODE)
        code[self.coded(0, "1")] = "1111000000"
        with self.assertRaises(SystemExit):
            self.run_gate(0, code=code)

    def test_duplicate_codeword_rejected(self):
        code = dict(self.CODE)
        a, b = self.coded(0, "1", 0), self.coded(0, "1", 1)
        code[b] = code[a]
        with self.assertRaises(SystemExit):
            self.run_gate(0, code=code)


class MkPullcodeTest(unittest.TestCase):
    def test_codes_distinct_weight5_balanced(self):
        for n in (1, 50, 150, 158, 252):
            w = mk_pullcode.assign_codes(n)
            self.assertEqual(len(set(w)), n)
            self.assertTrue(all(len(x) == 10 and x.count("1") == 5 for x in w))
            per = [sum(x[k] == "1" for x in w) for k in range(10)]
            self.assertLessEqual(max(per) - min(per), 1, per)

    def test_too_many_balls(self):
        with self.assertRaises(SystemExit):
            mk_pullcode.assign_codes(253)

    def test_committed_tables_match_generator(self):
        csvp = os.path.join(HERE, "..", "ep2c5_pullcode.csv")
        with open(csvp, encoding="utf-8") as f:
            committed = list(csv.DictReader(f))
        rows = mk_pullcode.build(os.path.join(HERE, "..", "ep2c5_padmap.csv"))
        self.assertEqual(committed, rows)
        self.assertEqual(sorted(r["ball"] for r in rows), PORT_BALLS)

    def test_qsf_pullups_follow_bit_k(self):
        rows = mk_pullcode.build(os.path.join(HERE, "..", "ep2c5_padmap.csv"))
        for k in (0, 9):
            q = mk_pullcode.qsf_text(k, rows)
            on = set(re.findall(r"WEAK_PULL_UP_RESISTOR ON -to p_(\S+)", q))
            off = set(re.findall(r"WEAK_PULL_UP_RESISTOR OFF -to p_(\S+)", q))
            want = {r["ball"] for r in rows if r["code"] and r["code"][k] == "1"}
            self.assertEqual(on, want)
            self.assertEqual(on | off, {r["ball"] for r in rows})
            self.assertEqual(len(re.findall(r"ENABLE_BUS_HOLD_CIRCUITRY OFF", q)), len(rows))
            for opt in ("ENABLE_INIT_DONE_OUTPUT OFF", "CRC_ERROR_CHECKING OFF",
                        'CYCLONEII_RESERVE_NCEO_AFTER_CONFIGURATION "USE AS REGULAR IO"',
                        'CYCLONEII_CONFIGURATION_SCHEME "PASSIVE SERIAL"',
                        "ON_CHIP_BITSTREAM_DECOMPRESSION ON",
                        'RESERVE_ALL_UNUSED_PINS "AS INPUT TRI-STATED"'):
                self.assertIn(opt, q)

    def test_committed_sources_match_generator(self):
        root = os.path.normpath(os.path.join(HERE, "..", "..", ".."))
        out = os.path.join(root, "fpga", "pullcode-ep2c5")
        rows = mk_pullcode.build(os.path.join(HERE, "..", "ep2c5_padmap.csv"))
        with open(os.path.join(out, "pullcode.v"), encoding="utf-8") as f:
            self.assertEqual(f.read(), mk_pullcode.verilog_text(rows))
        for k in range(10):
            with open(os.path.join(out, f"k{k}", "pullcode.qsf"), encoding="utf-8") as f:
                self.assertEqual(f.read(), mk_pullcode.qsf_text(k, rows), k)
        v = mk_pullcode.verilog_text(rows)
        self.assertNotRegex(v, r"\b(output|inout|assign|always)\b")


if __name__ == "__main__":
    unittest.main()
