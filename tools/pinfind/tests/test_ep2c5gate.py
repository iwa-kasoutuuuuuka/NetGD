"""Tests for tools/pinfind/ep2c5gate.py with SYNTHETIC .pin / fit.rpt snippets.

    python -m unittest discover -s tools/pinfind/tests

The snippets only mimic the layout of Quartus II 13.0 reports; they are not
copied from a real compile.
"""
import contextlib
import io
import os
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, ".."))

import ep2c5gate  # noqa: E402

PIN_HEAD = """ -- synthetic
CHIP  "stage0"  ASSIGNED TO AN: EP2C5F256C8

Pin Name/Usage               : Location  : Dir.   : I/O Standard      : Voltage : I/O Bank  : User Assignment
-------------------------------------------------------------------------------------------------------------
"""


def pin_row(name, loc, d="", std="", bank="", user=""):
    return f"{name:<29}: {loc:<10}: {d:<7}: {std:<18}:         : {bank:<10}: {user}\n"


GOOD_ROWS = [
    ("GND", "A1", "gnd"),
    ("VCCIO8", "A2", "power"),
    ("RESERVED_INPUT", "A3", "", "", "3"),
    ("~ASDO~ / RESERVED_INPUT", "C1", "input", "3.3-V LVTTL", "1"),
    ("~nCSO~ / RESERVED_INPUT", "D2", "input", "3.3-V LVTTL", "1"),
    ("~LVDS41p/nCEO~ / RESERVED_INPUT", "N12", "input", "3.3-V LVTTL", "4"),
    ("TDO", "J5", "output"),
    ("nSTATUS", "M12", "bidir"),
    ("CONF_DONE", "L12", ""),
    ("nCONFIG", "H5", ""),
    ("DCLK", "H1", ""),
    ("DATA0", "H2", ""),
    ("GND+", "E8", ""),
    ("stage0_in", "B5", "input", "3.3-V LVTTL", "3", "N"),
]

FIT = """+------------------------------------------------+
; Input Pins                                     ;
+-----------+-------+----------+--------------+-----------+
; Name      ; Pin # ; I/O Bank ; Bus Hold     ; Weak Pull Up ;
+-----------+-------+----------+--------------+-----------+
; stage0_in ; B5    ; 3        ; no           ; Off       ;
+-----------+-------+----------+--------------+-----------+

+----------------------------------+
; Dual Purpose and Dedicated Pins  ;
+----------+-----------+--------------------+-------------------+------------------+
; Location ; Pin Name  ; Reserved As        ; User Signal Name  ; Pin Type         ;
+----------+-----------+--------------------+-------------------+------------------+
; C1       ; ASDO      ; As input tri-stated ; ~ASDO~           ; Dual Purpose Pin ;
; N12      ; nCEO      ; Use as regular IO  ; ~LVDS41p/nCEO~    ; Dual Purpose Pin ;
+----------+-----------+--------------------+-------------------+------------------+

+-----------------------------------------------------------+
; Fitter Device Options                                     ;
+-----------------------------------------------+-----------+
; Option                                        ; Setting   ;
+-----------------------------------------------+-----------+
; Enable device-wide reset (DEV_CLRn)           ; Off       ;
; Enable device-wide output enable (DEV_OE)     ; Off       ;
; Enable INIT_DONE output                       ; Off       ;
; Error detection CRC                           ; Off       ;
; Reserve ASDO pin after configuration.         ; As input tri-stated ;
; Reserve nCEO pin after configuration.         ; Use as regular IO ;
; Reserve all unused pins                       ; As input tri-stated ;
+-----------------------------------------------+-----------+
"""


def build_pin(rows):
    return PIN_HEAD + "".join(pin_row(*r) for r in rows)


class GateTest(unittest.TestCase):
    def run_gate(self, rows, fit=FIT, extra=()):
        with tempfile.TemporaryDirectory() as d:
            with open(os.path.join(d, "stage0.pin"), "w") as f:
                f.write(build_pin(rows))
            with open(os.path.join(d, "stage0.fit.rpt"), "w") as f:
                f.write(fit)
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                rc = ep2c5gate.main([d, "--pins", str(len(rows)), *extra])
            return rc, out.getvalue()

    def test_good_passes(self):
        rc, out = self.run_gate(GOOD_ROWS)
        self.assertEqual(rc, 0, out)
        self.assertIn("PASS", out)

    def test_unused_driving_ground_fails(self):
        rows = GOOD_ROWS + [("GND*", "A4", "", "", "3")]
        rc, out = self.run_gate(rows)
        self.assertEqual(rc, 1, out)
        self.assertIn("GND*", out)

    def test_weak_pullup_reserved_fails(self):
        rows = GOOD_ROWS + [("RESERVED_INPUT_WITH_WEAK_PULLUP", "A5", "", "", "3")]
        self.assertEqual(self.run_gate(rows)[0], 1)

    def test_bus_hold_reserved_fails(self):
        rows = GOOD_ROWS + [("RESERVED_INPUT_WITH_BUS_HOLD", "A5", "", "", "3")]
        self.assertEqual(self.run_gate(rows)[0], 1)

    def test_user_output_fails(self):
        rows = GOOD_ROWS + [("led", "A6", "output", "3.3-V LVTTL", "3", "N")]
        self.assertEqual(self.run_gate(rows, extra=["--user", "stage0_in", "--user", "led"])[0], 1)

    def test_unexpected_user_input_fails(self):
        rows = GOOD_ROWS + [("other", "A6", "input", "3.3-V LVTTL", "3", "N")]
        self.assertEqual(self.run_gate(rows)[0], 1)

    def test_nceo_as_output_fails(self):
        rows = [r for r in GOOD_ROWS if "nCEO" not in r[0]] + [("nCEO", "N12", "output")]
        self.assertEqual(self.run_gate(rows)[0], 1)

    def test_dual_purpose_kept_fails(self):
        rows = [r for r in GOOD_ROWS if "ASDO" not in r[0]] + [("~ASDO~", "C1", "output")]
        self.assertEqual(self.run_gate(rows)[0], 1)

    def test_wrong_device_fails(self):
        rc, out = self.run_gate(GOOD_ROWS, extra=["--device", "EP2C5T144C8"])
        self.assertEqual(rc, 1, out)

    def test_fit_output_pins_table_fails(self):
        fit = FIT + """
+----------------+
; Output Pins    ;
+------+-------+
; Name ; Pin # ;
+------+-------+
; led  ; A6    ;
+------+-------+
"""
        self.assertEqual(self.run_gate(GOOD_ROWS, fit=fit)[0], 1)

    def test_fit_unused_as_ground_fails(self):
        fit = FIT.replace("; Reserve all unused pins                       ; As input tri-stated ;",
                          "; Reserve all unused pins                       ; As output driving ground ;")
        self.assertNotEqual(fit, FIT)
        self.assertEqual(self.run_gate(GOOD_ROWS, fit=fit)[0], 1)

    def test_fit_init_done_on_fails(self):
        fit = FIT.replace("; Enable INIT_DONE output                       ; Off       ;",
                          "; Enable INIT_DONE output                       ; On        ;")
        self.assertNotEqual(fit, FIT)
        self.assertEqual(self.run_gate(GOOD_ROWS, fit=fit)[0], 1)

    def test_fit_input_pullup_fails(self):
        fit = FIT.replace("; no           ; Off       ;", "; no           ; On        ;")
        self.assertNotEqual(fit, FIT)
        self.assertEqual(self.run_gate(GOOD_ROWS, fit=fit)[0], 1)

    def test_missing_option_fails(self):
        fit = FIT.replace("; Error detection CRC                           ; Off       ;\n", "")
        self.assertNotEqual(fit, FIT)
        self.assertEqual(self.run_gate(GOOD_ROWS, fit=fit)[0], 1)


if __name__ == "__main__":
    unittest.main()
