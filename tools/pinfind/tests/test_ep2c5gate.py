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


BALLS = ep2c5gate.load_balls()
DIRS = {"TDO": "output", "TDI": "input", "TMS": "input", "TCK": "input",
        "DATA0": "input", "USER": "input"}


def good_rows():
    """One row per expected ball, taken from the repository's ball table."""
    rows = []
    for ball, role in BALLS.items():
        if role == "USER":
            rows.append(["stage0_in", ball, "input", "3.3-V LVTTL", "1", "Y"])
        elif role in ("GND",) or role.startswith("GND_PLL") or role.startswith("GNDA_PLL"):
            rows.append([role, ball, "gnd"])
        elif role.startswith(("VCC",)):
            rows.append([role, ball, "power"])
        else:
            rows.append([role, ball, DIRS.get(role, "")])
    return rows


GOOD_ROWS = good_rows()


def ball_of(role, n=0):
    return [b for b, r in BALLS.items() if r == role][n]


def mutate(ball, **kw):
    """GOOD_ROWS with the row at `ball` replaced (name=..., d=...)."""
    out = []
    for r in GOOD_ROWS:
        r = list(r)
        if r[1] == ball:
            if "name" in kw:
                r[0] = kw["name"]
            if "d" in kw:
                r[2] = kw["d"]
        out.append(r)
    return out


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
; Enable user-supplied start-up clock (CLKUSR)  ; Off       ;
; Configuration scheme                          ; Passive Serial ;
; Error detection CRC                           ; Off       ;
; nCEO                                          ; Use as regular IO ;
; ASDO,nCSO                                     ; Unreserved ;
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

    def io(self, n=0):
        return ball_of("RESERVED_INPUT", n)

    def test_unused_driving_ground_fails(self):
        rc, out = self.run_gate(mutate(self.io(), name="GND*"))
        self.assertEqual(rc, 1, out)
        self.assertIn("GND*", out)

    def test_weak_pullup_reserved_fails(self):
        self.assertEqual(self.run_gate(mutate(self.io(), name="RESERVED_INPUT_WITH_WEAK_PULLUP"))[0], 1)

    def test_bus_hold_reserved_fails(self):
        self.assertEqual(self.run_gate(mutate(self.io(), name="RESERVED_INPUT_WITH_BUS_HOLD"))[0], 1)

    def test_user_output_fails(self):
        rows = mutate(self.io(), name="led", d="output")
        self.assertEqual(self.run_gate(rows, extra=["--user", "stage0_in", "--user", "led"])[0], 1)

    def test_unexpected_user_input_fails(self):
        self.assertEqual(self.run_gate(mutate(self.io(), name="other", d="input"))[0], 1)

    def test_nceo_as_output_fails(self):
        self.assertEqual(self.run_gate(mutate(self.io(), name="nCEO", d="output"))[0], 1)

    def test_dual_purpose_kept_fails(self):
        self.assertEqual(self.run_gate(mutate(self.io(), name="~ASDO~", d="output"))[0], 1)

    # --- D3: expected-ball table (the false-pass mutations) ---------------

    def test_io_ball_renamed_tdo_output_fails(self):
        rc, out = self.run_gate(mutate(self.io(), name="TDO", d="output"))
        self.assertEqual(rc, 1, out)
        self.assertIn("I/O ball not RESERVED_INPUT", out)

    def test_io_ball_renamed_conf_done_output_fails(self):
        rc, out = self.run_gate(mutate(self.io(), name="CONF_DONE", d="output"))
        self.assertEqual(rc, 1, out)

    def test_io_ball_renamed_nstatus_bidir_fails(self):
        self.assertEqual(self.run_gate(mutate(self.io(), name="nSTATUS", d="bidir"))[0], 1)

    def test_io_ball_changed_to_gnd_fails(self):
        rc, out = self.run_gate(mutate(self.io(), name="GND", d="gnd"))
        self.assertEqual(rc, 1, out)
        self.assertIn("not RESERVED_INPUT", out)

    def test_io_ball_changed_to_gnd_star_fails(self):
        self.assertEqual(self.run_gate(mutate(self.io(), name="GND*", d="gnd"))[0], 1)

    def test_tdo_on_wrong_ball_fails_both_ends(self):
        # TDO moved: the real TDO ball is now a plain I/O (wrong role), and
        # the other ball is not the TDO ball.
        rows = mutate(ball_of("TDO"), name="RESERVED_INPUT", d="")
        rc, out = self.run_gate(rows)
        self.assertEqual(rc, 1, out)

    def test_tdo_still_allowed_as_output_at_its_ball(self):
        self.assertEqual(self.run_gate(GOOD_ROWS)[0], 0)

    def test_supply_ball_renamed_fails(self):
        self.assertEqual(self.run_gate(mutate(ball_of("GND"), name="RESERVED_INPUT"))[0], 1)

    def test_ball_missing_fails(self):
        rows = [r for r in GOOD_ROWS if r[1] != self.io()]
        rc, out = self.run_gate(rows, extra=[])
        self.assertEqual(rc, 1, out)

    def test_unknown_ball_fails(self):
        rows = mutate(self.io())
        rows[0][1] = "ZZ99"
        self.assertEqual(self.run_gate(rows)[0], 1)

    def test_table_has_256_balls_one_each_special(self):
        self.assertEqual(len(BALLS), 256)
        for role in ("TDO", "CONF_DONE", "nSTATUS", "USER"):
            self.assertEqual(list(BALLS.values()).count(role), 1, role)

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

    def test_fit_nceo_driving_ground_fails(self):
        # What the first CI run produced with the non-CYCLONEII_ assignment.
        fit = FIT.replace("; nCEO                                          ; Use as regular IO ;",
                          "; nCEO                                          ; As output driving ground ;")
        self.assertNotEqual(fit, FIT)
        self.assertEqual(self.run_gate(GOOD_ROWS, fit=fit)[0], 1)

    # --- D4 / D5 -----------------------------------------------------------

    def test_fit_input_pins_table_missing_fails(self):
        blocks = FIT.split(chr(10) * 2)
        fit = (chr(10) * 2).join(x for x in blocks if "; Input Pins" not in x)
        self.assertNotIn("Input Pins", fit)
        rc, out = self.run_gate(GOOD_ROWS, fit=fit)
        self.assertEqual(rc, 1, out)
        self.assertIn("Input Pins table missing", out)

    def test_fit_clkusr_on_fails(self):
        fit = FIT.replace("(CLKUSR)  ; Off       ;", "(CLKUSR)  ; On        ;")
        self.assertNotEqual(fit, FIT)
        rc, out = self.run_gate(GOOD_ROWS, fit=fit)
        self.assertEqual(rc, 1, out)
        self.assertIn("clkusr", out)

    def test_fit_clkusr_missing_fails(self):
        fit = FIT.replace("; Enable user-supplied start-up clock (CLKUSR)  ; Off       ;" + chr(10), "")
        self.assertNotEqual(fit, FIT)
        self.assertEqual(self.run_gate(GOOD_ROWS, fit=fit)[0], 1)

    def test_fit_scheme_not_passive_serial_fails(self):
        fit = FIT.replace("; Passive Serial ;", "; Active Serial ;")
        self.assertNotEqual(fit, FIT)
        self.assertEqual(self.run_gate(GOOD_ROWS, fit=fit)[0], 1)

    def test_missing_option_fails(self):
        fit = FIT.replace("; Error detection CRC                           ; Off       ;\n", "")
        self.assertNotEqual(fit, FIT)
        self.assertEqual(self.run_gate(GOOD_ROWS, fit=fit)[0], 1)


if __name__ == "__main__":
    unittest.main()
