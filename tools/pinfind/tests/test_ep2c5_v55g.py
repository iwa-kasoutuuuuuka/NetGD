"""Tests for ep2c5gate.py --v55g and tools/pinfind/mk_v55g.py, with SYNTHETIC
.pin / fit.rpt / sta.rpt snippets (layout of Quartus II 13.0 reports, not
copied).

    python -m unittest tools/pinfind/tests/test_ep2c5_v55g.py
"""
import contextlib
import io
import os
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, ".."))
sys.path.insert(0, HERE)

import ep2c5gate  # noqa: E402
import mk_v55g  # noqa: E402
from test_ep2c5_pullcode import BALLS, DIRS, OPTIONS, PIN_HEAD  # noqa: E402

PADMAP = os.path.join(HERE, "..", "ep2c5_padmap.csv")
TABLE = os.path.join(HERE, "..", "ep2c5_v55g.csv")
ROOT = os.path.normpath(os.path.join(HERE, "..", "..", ".."))
ROWS = mk_v55g.build(PADMAP)
TAB = {r["signal"]: r for r in ROWS}
AT = {r["ball"]: r for r in ROWS}


def pin_text(override=None):
    override = override or {}
    out = [PIN_HEAD.replace('"pullcode"', '"v55g"')]
    for ball, role in BALLS.items():
        if ball in AT:
            r = AT[ball]
            row = [r["signal"], ball, r["dir"], "3.3-V LVTTL", "1", "Y"]
        elif role == "GND" or role.startswith(("GND_PLL", "GNDA_PLL")):
            row = [role, ball, "gnd", "", "", ""]
        elif role.startswith("VCC"):
            row = [role, ball, "power", "", "", ""]
        elif role in ("RESERVED_INPUT", "USER"):
            row = ["RESERVED_INPUT", ball, "", "", "", ""]
        else:
            row = [role, ball, DIRS.get(role, ""), "", "", ""]
        if ball in override:
            row[0], row[2] = override[ball]
        out.append(f"{row[0]:<29}: {row[1]:<10}: {row[2]:<7}: {row[3]:<18}:"
                   f"         : {row[4]:<10}: {row[5]}\n")
    return "".join(out)


def table(title, header, rows):
    w = [len(h) for h in header]
    for r in rows:
        w = [max(a, len(str(c))) for a, c in zip(w, r)]
    line = "+" + "+".join("-" * (x + 2) for x in w) + "+"
    fmt = lambda r: "; " + " ; ".join(str(c).ljust(x) for c, x in zip(r, w)) + " ;"
    out = [line, f"; {title} ;", line, fmt(header), line]
    out += [fmt(r) for r in rows]
    out.append(line)
    return "\n".join(out) + "\n\n"


def fit_text(strength=None, pullup=None, extra_out=None, pll_in="CLK_11_2896_MHz",
             plls="1 / 2 ( 50 % )", drop=None):
    strength = strength or {}
    pullup = pullup or {}
    s = table("Fitter Summary", ["Fitter Status", "Successful"], [
        ["Device", "EP2C5F256C8"],
        ["Total logic elements", "4,100 / 4,608 ( 89 % )"],
        ["Total memory bits", "102,400 / 119,808 ( 85 % )"],
        ["Total PLLs", plls]])
    hdr = ["Name", "Pin #", "Bus Hold", "Weak Pull Up", "I/O Standard", "Current Strength"]
    outs, bids = [], []
    for r in ROWS:
        if r["dir"] == "input" or r["signal"] == drop:
            continue
        cur = strength.get(r["signal"], "4mA" if r["strength"] else "24mA")
        pu = pullup.get(r["signal"], "On" if r["pullup"] == "on" else "Off")
        row = [r["signal"], r["ball"], "no", pu, "3.3-V LVTTL", cur]
        (bids if r["dir"] == "bidir" else outs).append(row)
    if extra_out:
        outs.append([extra_out[0], extra_out[1], "no", "Off", "3.3-V LVTTL", "24mA"])
    s += table("Output Pins", hdr, outs)
    s += table("Bidir Pins", hdr, bids)
    ins = [[r["signal"], r["ball"], "no", pullup.get(r["signal"], "On" if r["pullup"] == "on" else "Off")]
           for r in ROWS if r["dir"] == "input"]
    s += table("Input Pins", ["Name", "Pin #", "Bus Hold", "Weak Pull Up"], ins)
    pk = []
    for ball, role in BALLS.items():
        if ball in AT:
            r = AT[ball]
            pu = pullup.get(r["signal"], "On" if r["pullup"] == "on" else "Off")
            pk.append([ball, r["signal"], r["dir"], "no", pu])
        elif extra_out and ball == extra_out[1]:
            pk.append([ball, extra_out[0], "output", "no", "Off"])
        else:
            nm = {"TDO": "#TDO", "nSTATUS": "^nSTATUS", "CONF_DONE": "^CONF_DONE"}.get(role, role)
            if role in ("RESERVED_INPUT", "USER"):
                nm = "RESERVED_INPUT"
            pk.append([ball, nm, DIRS.get(role, ""), "--", "--"])
    s += table("All Package Pins", ["Location", "Pin Name/Usage", "Dir.", "Bus Hold", "Weak Pull Up"], pk)
    s += table("PLL Summary", ["Name", "Audio_PLL:audio_pll_inst|altpll:altpll_component|pll"], [
        ["PLL location", "PLL_1"], ["Input frequency 0", "11.29 MHz"],
        ["Inclk0 signal", pll_in], ["Inclk1 signal", "--"],
        ["Inclk0 signal type", "Dedicated Pin"]])
    s += OPTIONS
    return s


def sta_text(slack48="6.100", extra=""):
    s = table("Slow Model Setup Summary", ["Clock", "Slack", "End Point TNS"], [
        ["CLK_48_MHz", slack48, "0.000"], ["CLK_11_2896_MHz", "80.000", "0.000"]])
    s += table("Slow Model Hold Summary", ["Clock", "Slack", "End Point TNS"], [
        ["CLK_48_MHz", "0.499", "0.000"]])
    return s + extra


def run_gate(pin=None, fit=None, sta=None):
    with tempfile.TemporaryDirectory() as d:
        for name, text in (("v55g.pin", pin or pin_text()), ("v55g.fit.rpt", fit or fit_text()),
                           ("v55g.sta.rpt", sta or sta_text())):
            with open(os.path.join(d, name), "w", encoding="utf-8") as f:
                f.write(text)
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            rc = ep2c5gate.main([os.path.join(d, "v55g.pin"), "--v55g", "--table", TABLE, "-q"])
        return rc, buf.getvalue()


class TestGenerator(unittest.TestCase):
    def test_rows(self):
        self.assertEqual(len(ROWS), 68)
        outs = [r for r in ROWS if r["dir"] != "input"]
        self.assertEqual(len(outs), 41)
        dc = [r for r in outs if r["signal"].startswith("DC_")]
        self.assertEqual(len(dc), 24)
        self.assertTrue(all(r["strength"] == "4mA" and r["dir"] == "bidir" for r in dc))
        self.assertEqual(TAB["DC_INTRQ"]["ball"], "B4")
        self.assertEqual(TAB["DC_INTRQ"]["pullup"], "off")
        self.assertEqual(TAB["G_MASTERn"]["ball"], "R14")
        self.assertEqual(TAB["G_WRAPn"]["ball"], "N8")
        self.assertEqual(sorted(int(r["gidx"]) for r in dc), list(range(24)))

    def test_files_up_to_date(self):
        with tempfile.TemporaryDirectory() as d:
            t = os.path.join(d, "t.csv")
            with contextlib.redirect_stdout(io.StringIO()):
                mk_v55g.main(["--table", t, "--out", d])
            for got, want in ((t, TABLE),
                              (os.path.join(d, "v55g.qsf"),
                               os.path.join(ROOT, "fpga", "v55g-ep2c5", "v55g.qsf")),
                              (os.path.join(d, "v55g.qpf"),
                               os.path.join(ROOT, "fpga", "v55g-ep2c5", "v55g.qpf"))):
                with open(got, encoding="utf-8") as a, open(want, encoding="utf-8") as b:
                    self.assertEqual(a.read(), b.read(), f"{want} is stale: run mk_v55g.py")

    def test_qsf_strength(self):
        q = mk_v55g.qsf_text(ROWS)
        self.assertEqual(q.count('CURRENT_STRENGTH_NEW "4MA"'), 24)
        self.assertIn('RESERVE_ALL_UNUSED_PINS "AS INPUT TRI-STATED"', q)
        self.assertIn("DEVICE EP2C5F256C8", q)


class TestGate(unittest.TestCase):
    def test_pass(self):
        rc, out = run_gate()
        self.assertEqual(rc, 0, out)

    def test_strength_24ma(self):
        rc, out = run_gate(fit=fit_text(strength={"DC_DATA[3]": "24mA"}))
        self.assertEqual(rc, 1)
        self.assertIn("DC_DATA[3] current strength", out)

    def test_extra_output(self):
        rc, out = run_gate(pin=pin_text({"C4": ("dbg", "output")}),
                           fit=fit_text(extra_out=("dbg", "C4")))
        self.assertEqual(rc, 1)
        self.assertIn("unexpected Output Pins row dbg", out)

    def test_missing_output(self):
        rc, out = run_gate(fit=fit_text(drop="DC_IORDY"))
        self.assertEqual(rc, 1)
        self.assertIn("DC_IORDY missing", out)

    def test_b4_pullup(self):
        rc, out = run_gate(fit=fit_text(pullup={"DC_INTRQ": "On"}))
        self.assertEqual(rc, 1)
        self.assertIn("DC_INTRQ weak pull-up", out)

    def test_master_no_pullup(self):
        rc, out = run_gate(fit=fit_text(pullup={"G_MASTERn": "Off"}))
        self.assertEqual(rc, 1)

    def test_pll_policy(self):
        rc, out = run_gate(fit=fit_text(pll_in="CLK_48_MHz"))
        self.assertEqual(rc, 1)
        self.assertIn("PLL inclk0", out)
        rc, out = run_gate(fit=fit_text(plls="2 / 2 ( 100 % )"))
        self.assertEqual(rc, 1)

    def test_wrong_ball(self):
        b4 = TAB["DC_INTRQ"]["ball"]
        rc, out = run_gate(pin=pin_text({b4: ("RESERVED_INPUT", "")}))
        self.assertEqual(rc, 1)

    def test_timing(self):
        rc, out = run_gate(sta=sta_text(slack48="-0.200"))
        self.assertEqual(rc, 1)
        self.assertIn("slack -0.2", out)
        rc, out = run_gate(sta=sta_text(extra="Critical Warning: Timing requirements not met\n"))
        self.assertEqual(rc, 1)


if __name__ == "__main__":
    unittest.main()
