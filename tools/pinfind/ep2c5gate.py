#!/usr/bin/env python3
"""Safety gate for the stage-0 EP2C5 bitstream (fpga/stage0-ep2c5).

usage: ep2c5gate.py OUTPUT_DIR_OR_PIN_FILE [--fit FIT_RPT] [--device EP2C5F256C8]
                    [--user stage0_in ...] [--pins 256]

Reads the Quartus II fitter's <rev>.pin file and <rev>.fit.rpt and exits
non-zero unless the bitstream cannot drive any pin after configuration:

  .pin   * the chip line names the expected device
         * every one of the 256 balls is listed and has the role given by
           ep2c5f256_balls.txt (expected ball table); nothing else is
           accepted at a ball (an I/O ball shown as GND/GND*/TDO fails)
         * no row is an output or bidir, except TDO, nSTATUS and CONF_DONE
           at their own balls, which the bitstream does not control
         * every unused I/O is plain RESERVED_INPUT: no GND* / VCC*
           (unused pin driving ground/VCC), no weak pull-up, no bus hold,
           no RESERVED_OUTPUT / RESERVED_BIDIR
         * every dual-purpose pin (~NAME~ / ...) is RESERVED_INPUT
         * the only user pins are the --user names, and they are inputs
  fit.rpt
         * no Output Pins / Bidir Pins table rows
         * Input Pins: weak pull-up off, bus hold off
         * Dual Purpose and Dedicated Pins: dual-purpose pins not kept for
           configuration and not driven
         * device options: unused pins as input tri-stated, INIT_DONE,
           DEV_CLRn, DEV_OE, CLKUSR and CRC error detection off,
           configuration scheme Passive Serial, Input Pins table present, nCEO as regular IO

Standard library only. Prints every pin it looked at, then PASS or FAIL.
"""
import argparse
import glob
import os
import re
import sys

# Dedicated pins the user design cannot change. Their direction in the .pin
# file is fixed by the device (TDO is a JTAG output, nSTATUS/CONF_DONE are
# open-drain configuration bidirs), so they are allowed any direction.
DEDICATED_ANY_DIR = {"TDO", "nSTATUS", "CONF_DONE"}
# Expected ball -> role table (256 balls); exemptions apply only there.
BALL_TABLE = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                          "ep2c5f256_balls.txt")
DEDICATED_INPUT = {"TDI", "TMS", "TCK", "nCONFIG", "nCE", "DCLK", "DATA0",
                   "MSEL0", "MSEL1", "MSEL2", "MSEL3"}
POWER_DIRS = {"gnd", "power"}
OK_INPUT_DIRS = {"input", ""}

BAD_USAGE = re.compile(r"(GND\*|VCC\*|PULL|BUS_HOLD|BUS-HOLD|OUTPUT|BIDIR)",
                       re.IGNORECASE)
DUAL = re.compile(r"^~([^~]+)~\s*(?:/\s*(.*))?$")


class Gate:
    def __init__(self):
        self.errors = []
        self.notes = []

    def fail(self, msg):
        self.errors.append(msg)

    def note(self, msg):
        self.notes.append(msg)


# ---------------------------------------------------------------- .pin ----

def parse_pin(text):
    """Return (chip_line, device, rows); rows are dicts name/loc/dir/std/bank/user."""
    chip = None
    device = None
    rows = []
    in_table = False
    for line in text.splitlines():
        s = line.strip()
        if s.startswith("--"):
            continue
        m = re.match(r'^CHIP\s+"?([^"\s]*)"?\s+ASSIGNED TO AN:\s*(\S+)', s)
        if m:
            chip, device = m.group(1), m.group(2)
            continue
        if s.startswith("Pin Name/Usage"):
            in_table = True
            continue
        if not in_table or not s or set(s) <= set("-"):
            continue
        parts = [p.strip() for p in s.split(":")]
        if len(parts) < 3:
            continue
        while len(parts) < 7:
            parts.append("")
        rows.append({"name": parts[0], "loc": parts[1], "dir": parts[2].lower(),
                     "std": parts[3], "volt": parts[4], "bank": parts[5],
                     "user": parts[6]})
    return chip, device, rows


def load_balls(path=None):
    """Return {ball: role} from the expected-ball table (BALL ROLE per line)."""
    path = path or BALL_TABLE
    table = {}
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.split("#", 1)[0].strip()
            if not line:
                continue
            ball, role = line.split(None, 1)
            table[ball] = role.strip()
    return table


def check_pin(g, text, device, users, npins, balls=None):
    chip, dev, rows = parse_pin(text)
    if dev is None:
        g.fail(".pin: no 'CHIP ... ASSIGNED TO AN:' line")
    elif device and dev.upper() != device.upper():
        g.fail(f".pin: device {dev}, expected {device}")
    else:
        g.note(f".pin: chip {chip} on {dev}")
    if not rows:
        g.fail(".pin: no pin rows parsed")
        return rows
    locs = [r["loc"] for r in rows]
    if len(set(locs)) != len(locs):
        g.fail(".pin: duplicate locations")
    if npins and len(rows) != npins:
        g.fail(f".pin: {len(rows)} rows, expected {npins}")
    if balls is None:
        balls = load_balls()
    listed = set(locs)
    for b in sorted(set(balls) - listed):
        g.fail(f".pin: expected ball {b} ({balls[b]}) missing")
    seen_users = set()
    for r in rows:
        name, d = r["name"], r["dir"]
        where = f"{r['loc']} {name!r} dir={d or '-'}"
        role = balls.get(r["loc"])
        if role is None:
            g.fail(f".pin: ball not in the expected EP2C5F256C8 table: {where}")
            continue
        m = DUAL.match(name)
        if m:
            usage = (m.group(2) or "").strip()
            if role != "RESERVED_INPUT":
                g.fail(f".pin: dual-purpose name on non-I/O ball (expected {role}): {where}")
            elif usage != "RESERVED_INPUT" or d not in OK_INPUT_DIRS:
                g.fail(f".pin: dual-purpose pin not input tri-stated: {where}")
            continue
        if role == "USER":
            if name not in users:
                g.fail(f".pin: unexpected user pin (ball expects USER): {where}")
            elif d != "input":
                g.fail(f".pin: user pin not an input: {where}")
            seen_users.add(name)
            continue
        if role == "RESERVED_INPUT":
            # Unused/released I/O ball: must be plain RESERVED_INPUT, or a
            # user input. Anything else (GND, GND*, TDO, output, ...) fails.
            if name in users:
                if d != "input":
                    g.fail(f".pin: user pin not an input: {where}")
                seen_users.add(name)
            elif name != "RESERVED_INPUT" or d not in OK_INPUT_DIRS:
                g.fail(f".pin: I/O ball not RESERVED_INPUT (expected RESERVED_INPUT): {where}")
            continue
        # Fixed ball: the .pin name must be exactly the expected role.
        if name != role:
            g.fail(f".pin: ball {r['loc']} is {name!r}, expected {role!r}")
            continue
        if "*" in name or BAD_USAGE.search(name):
            g.fail(f".pin: driving/pulled pin: {where}")
        elif role in DEDICATED_ANY_DIR:
            pass  # direction fixed by the device, exempt at this ball only
        elif role in DEDICATED_INPUT or role in ("NC", "GND+"):
            if d not in OK_INPUT_DIRS:
                g.fail(f".pin: pin not an input: {where}")
        elif role.startswith(("VCC", "GND")):
            if d not in POWER_DIRS | OK_INPUT_DIRS:
                g.fail(f".pin: supply pin with direction: {where}")
        else:
            g.fail(f".pin: unknown role {role!r}: {where}")
    for u in users:
        if u not in seen_users:
            g.note(f".pin: user pin {u} not listed (optimised away?)")
    return rows


# ------------------------------------------------------------- fit.rpt ----

def rpt_tables(text):
    """Map table title -> list of rows (each a list of cell strings).

    Quartus report tables look like
        +------+
        ; Title ;
        +------+------+
        ; Col1 ; Col2 ;
        +------+------+
        ; a    ; b    ;
        +------+------+
    The first row after the title is kept as the header (rows[0]).
    """
    tables = {}
    title = None
    rows = None
    for line in text.splitlines():
        s = line.rstrip()
        if s.startswith(";") and s.endswith(";"):
            cells = [c.strip() for c in s.strip(";").split(";")]
            if title is None:
                title = cells[0]
                rows = []
                tables.setdefault(title, rows)
            else:
                rows.append(cells)
        elif s.startswith("+"):
            continue
        else:
            title = None
            rows = None
    return tables


def find_table(tables, prefix):
    for t, rows in tables.items():
        if t.lower().startswith(prefix.lower()):
            return t, rows
    return None, None


def col(header, name):
    for i, h in enumerate(header):
        if h.lower() == name.lower():
            return i
    return None


# (option label regex, accepted values regex). A missing row fails.
REQUIRED_OPTIONS = [
    (r"^reserve all unused pins$", r"^as input tri-stated$"),
    (r"^enable init_done output$", r"^off$"),
    (r"^enable device-wide reset \(dev_clrn\)$", r"^off$"),
    (r"^enable device-wide output enable \(dev_oe\)$", r"^off$"),
    (r"^(error detection crc|enable error detection crc.*|crc error checking)$", r"^off$"),
    (r"^enable user-supplied start-up clock \(clkusr\)$", r"^off$"),
    (r"^configuration scheme$", r"^passive serial$"),
    # 13.0 labels this row just "nCEO" for Cyclone II; "USE AS REGULAR IO"
    # shows as "Unreserved" (the pin is then an unused pin, reserved as
    # input tri-stated, which the .pin check confirms).
    (r"^(nceo|reserve nceo pin after configuration\.?)$", r"^(unreserved|use as regular io)$"),
]
# Optional rows: checked only if present.
OPTIONAL_OPTIONS = [
    (r"^(asdo,ncso|reserve asdo pin after configuration\.?)$", r"^(unreserved|use as regular io|as input tri-stated)$"),
    (r"^reserve (ncso|flash_nce) pin after configuration\.?$", r"^(use as regular io|as input tri-stated)$"),
    (r"^reserve data0 pin after configuration$", r"^(use as regular io|as input tri-stated)$"),
]


def check_fit(g, text, users):
    tables = rpt_tables(text)
    for kind in ("Output Pins", "Bidir Pins"):
        t, rows = find_table(tables, kind)
        if t and len(rows) > 1:
            g.fail(f"fit.rpt: {kind} table has {len(rows) - 1} row(s): "
                   + ", ".join(r[0] for r in rows[1:]))
    t, rows = find_table(tables, "Input Pins")
    if t and rows:
        hdr = rows[0]
        ipu = col(hdr, "Weak Pull Up")
        ibh = col(hdr, "Bus Hold")
        for r in rows[1:]:
            if ipu is not None and r[ipu].lower() not in ("off", ""):
                g.fail(f"fit.rpt: input {r[0]} weak pull-up {r[ipu]}")
            if ibh is not None and r[ibh].lower() not in ("no", "off", ""):
                g.fail(f"fit.rpt: input {r[0]} bus hold {r[ibh]}")
            if r[0] not in users:
                g.fail(f"fit.rpt: unexpected input pin {r[0]}")
        if ipu is None or ibh is None:
            g.note("fit.rpt: Input Pins table lacks Weak Pull Up/Bus Hold column")
    else:
        g.fail("fit.rpt: Input Pins table missing (cannot check weak pull-up / bus hold)")
    t, rows = find_table(tables, "Dual Purpose and Dedicated Pins")
    if t and rows:
        hdr = rows[0]
        ires = col(hdr, "Reserved As")
        itype = col(hdr, "Pin Type")
        iname = col(hdr, "Pin Name")
        for r in rows[1:]:
            typ = r[itype] if itype is not None else ""
            res = r[ires] if ires is not None else ""
            nm = r[iname] if iname is not None else r[0]
            g.note(f"fit.rpt dual/dedicated: {' | '.join(r)}")
            if "dual" in typ.lower():
                low = res.lower()
                if "output" in low or "programming" in low or "drive" in low:
                    g.fail(f"fit.rpt: dual-purpose pin {nm} reserved as {res!r}")
    else:
        g.note("fit.rpt: no Dual Purpose and Dedicated Pins table")
    opts = {}
    for t, rows in tables.items():
        if "device options" in t.lower() or t.lower().startswith("fitter settings"):
            for r in rows:
                if len(r) >= 2:
                    opts.setdefault(r[0].strip().lower(), r[1].strip().lower())
    if not opts:
        g.fail("fit.rpt: no device options table found")
    for label, ok in REQUIRED_OPTIONS:
        hits = [(k, v) for k, v in opts.items() if re.match(label, k)]
        if not hits:
            g.fail(f"fit.rpt: device option /{label}/ not found")
        for k, v in hits:
            if not re.match(ok, v):
                g.fail(f"fit.rpt: {k} = {v!r}")
            else:
                g.note(f"fit.rpt option: {k} = {v}")
    for label, ok in OPTIONAL_OPTIONS:
        for k, v in opts.items():
            if re.match(label, k):
                if not re.match(ok, v):
                    g.fail(f"fit.rpt: {k} = {v!r}")
                else:
                    g.note(f"fit.rpt option: {k} = {v}")


# ----------------------------------------------------------------- main ----

def locate(path, fit):
    if os.path.isdir(path):
        pins = sorted(glob.glob(os.path.join(path, "*.pin")))
        if len(pins) != 1:
            sys.exit(f"ep2c5gate: need exactly one .pin in {path}, found {pins}")
        pin = pins[0]
    else:
        pin = path
    if fit is None:
        base = os.path.splitext(pin)[0]
        fit = base + ".fit.rpt"
    return pin, fit


def read(path):
    with open(path, encoding="utf-8", errors="replace") as f:
        return f.read()


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("path", help="output_files dir or the .pin file")
    ap.add_argument("--fit", help="fit.rpt (default: next to the .pin)")
    ap.add_argument("--device", default="EP2C5F256C8")
    ap.add_argument("--user", action="append", default=None,
                    help="allowed user input pin name (repeatable; default stage0_in)")
    ap.add_argument("--pins", type=int, default=256,
                    help="expected .pin row count (0 = do not check)")
    ap.add_argument("-q", "--quiet", action="store_true")
    a = ap.parse_args(argv)
    users = set(a.user or ["stage0_in"])
    pin, fit = locate(a.path, a.fit)
    g = Gate()
    rows = check_pin(g, read(pin), a.device, users, a.pins)
    if os.path.exists(fit):
        check_fit(g, read(fit), users)
    else:
        g.fail(f"fit.rpt not found: {fit}")
    if not a.quiet:
        print(f"== {pin}: {len(rows)} pins")
        for r in rows:
            print(f"  {r['loc']:<5} {r['dir'] or '-':<7} {r['name']}"
                  + (f"  [{r['std']}]" if r["std"] else ""))
        for n in g.notes:
            print("note:", n)
    for e in g.errors:
        print("FAIL:", e)
    if g.errors:
        print(f"ep2c5gate: FAIL ({len(g.errors)} problem(s))")
        return 1
    print("ep2c5gate: PASS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
