#!/usr/bin/env python3
"""Check a Quartus .pin report for the stage-0 rule: the configured FPGA
drives nothing.

Every row must be power or ground, a pin reserved as an input
(RESERVED_INPUT), or one of the dedicated configuration / JTAG pins.
Fails on any other name (a user signal, GND+ = unused pin driving
ground, a reserved output), on an output or bidir direction outside the
dedicated pins, and on a report with no RESERVED_INPUT rows.
Exit code 0 = pass. Usage: check_pins.py output_files/stage0.pin
"""
import re
import sys

POWER = re.compile(r"^(GND|VCC\w*|GND_PLL\w*|VCCA_PLL\w*|VCCD_PLL\w*|VCCIO\w*|VCCINT|NC)$")
# Dedicated pins of a passive-serial Cyclone II. TDO drives only during JTAG.
DEDICATED = {"nCONFIG", "nSTATUS", "CONF_DONE", "DCLK", "DATA0", "nCE",
             "MSEL0", "MSEL1", "MSEL2", "MSEL3", "TCK", "TDI", "TMS", "TDO"}
DEDICATED_DRIVE = {"nSTATUS", "CONF_DONE", "DCLK", "DATA0", "TDO"}


def rows(text):
    for line in text.splitlines():
        parts = [p.strip() for p in line.split(":")]
        if len(parts) < 3 or not re.match(r"^[A-Z]+\d+$", parts[1]):
            continue
        yield parts[0], parts[1], parts[2].lower()


def check(text):
    bad, n_res, n = [], 0, 0
    for name, loc, d in rows(text):
        n += 1
        base = name.strip("~")
        if POWER.match(name):
            continue
        if name == "RESERVED_INPUT" or name.endswith("/ RESERVED_INPUT"):
            n_res += 1
            if d not in ("", "input"):
                bad.append("%s %s: reserved input with direction %r" % (loc, name, d))
            continue
        if base in DEDICATED:
            if d in ("output", "bidir") and base not in DEDICATED_DRIVE:
                bad.append("%s %s: dedicated pin with direction %r" % (loc, name, d))
            continue
        bad.append("%s %s (%s): not allowed in stage-0" % (loc, name, d or "-"))
    if n == 0:
        bad.append("no pin rows found")
    elif n_res == 0:
        bad.append("no RESERVED_INPUT rows: unused pins are not reserved as inputs")
    return bad, n, n_res


def main(argv):
    with open(argv[1], encoding="utf-8", errors="replace") as f:
        bad, n, n_res = check(f.read())
    print("rows=%d reserved_input=%d problems=%d" % (n, n_res, len(bad)))
    for b in bad:
        print("FAIL " + b)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
