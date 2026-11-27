#!/usr/bin/env python3
"""Safety gate for the stage-0 EP2C5 bitstream (fpga/stage0-ep2c5).

usage: ep2c5gate.py OUTPUT_DIR_OR_PIN_FILE [--fit FIT_RPT] [--device EP2C5F256C8]
                    [--user stage0_in ...] [--pins 256]
                    [--pullcode K --code ep2c5_pullcode.csv] [--rbf FILE.rbf]

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

--pullcode K (weak pull-up code variant K of fpga/pullcode-ep2c5): every
ball listed in --code must be the user input p_<BALL> (no RESERVED_INPUT
I/O left), and the Weak Pull Up column of both the Input Pins and the All
Package Pins table must be On exactly where the ball's codeword has bit K
set (Off on balls with no codeword). Bus hold must be off everywhere, and
all the stage-0 checks above still apply (no Output/Bidir rows, dual-purpose
pins not kept, device options).

--scan (the scan core of fpga/scan-ep2c5, table --table ep2c5_scan.csv):
every ball listed in the table must be its port p_<BALL>; the balls whose
dir is "output" (the 16 MCU_DATA balls) must be output or bidir, every other
one an input. The Output Pins + Bidir Pins tables must list exactly those
output ports (nothing else may drive), the Input Pins table exactly the
input ports. Weak Pull Up must be On exactly where the table says "on"
(Input Pins, Output/Bidir Pins and All Package Pins), bus hold off
everywhere, the Fitter Summary must show 0 PLLs, and the dual-purpose and
device-option checks of stage-0 still apply.

--scan2 K (variant K = 0, 1, 2 of the scan2 core, fpga/scan2-ep2c5, table
--table ep2c5_scan2_vK.csv): all the --scan checks, with the bus hold
expected per ball from the table's optional "bushold" column (On exactly
where it says "on", Off elsewhere). Before reading the reports, the table
itself is checked against --base (default ep2c5_scan.csv, the scan core):
same balls, directions, kinds and signals; the outputs are exactly the 16
MCU_DATA balls; and the balls whose pull-up / bus hold differ from the
base are exactly the variant's set: v0 none; v1 the 33 DC_* balls plus
C4 and C5 with pull-up Off; v2 the 33 DC_* balls with pull-up Off and bus
hold On.

--v55g (the v5.5 core + g1gate image of fpga/v55g-ep2c5, table --table
ep2c5_v55g.csv from mk_v55g.py, --sta sta.rpt): the table follows
--padmap (v5.5 signal -> F256 ball) plus G_MASTERn R14 / G_WRAPn N8; .pin:
every port at its ball with its direction, nothing else used; fit.rpt:
the Output + Bidir Pins tables list exactly the 41 table outputs, Current
Strength 4mA on each of the 24 G1 pins, weak pull-up exactly per table,
bus hold off, inputs exactly per table, unused pins without pull-up and
not driven, PLL policy (exactly one PLL = Audio_PLL, inclk0
CLK_11_2896_MHz on a dedicated pin), stage-0 device options; sta.rpt:
every slack summary >= 0 and no "Timing requirements not met".

--rbf prints the bitstream's size and SHA-256 (a missing file fails).

Standard library only. Prints every pin it looked at, then PASS or FAIL.
"""
import argparse
import csv
import hashlib
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
# Ball-table roles that are user I/O or dedicated clock inputs (pull-up code
# ports). USER is the stage-0 input ball (H1, itself a clock input).
PORT_ROLES = {"RESERVED_INPUT", "USER", "GND+"}
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


def load_code(path, k):
    """Return {ball: pull-up expected in variant k} from the code csv.

    A row with an empty codeword is a port with pull-up off in every variant.
    """
    out = {}
    words = []
    with open(path, encoding="utf-8") as f:
        for r in csv.DictReader(f):
            ball = r["ball"].strip()
            code = (r.get("code") or "").strip()
            if code:
                if len(code) != 10 or set(code) - {"0", "1"} or code.count("1") != 5:
                    raise SystemExit(f"ep2c5gate: bad codeword {code!r} for {ball}")
                words.append(code)
            if ball in out:
                raise SystemExit(f"ep2c5gate: ball {ball} twice in {path}")
            out[ball] = bool(code) and code[k] == "1"
    if len(words) != len(set(words)):
        raise SystemExit("ep2c5gate: codewords not distinct")
    return out


def load_scan(path, with_hold=False):
    """Return ({ball: pull-up expected}, {output balls}) from ep2c5_scan.csv.

    with_hold: also return {ball: bus hold expected} from the optional
    "bushold" column (missing column or empty cell = off).
    """
    code, outs, hold = {}, set(), {}
    with open(path, encoding="utf-8") as f:
        for r in csv.DictReader(f):
            ball = r["ball"].strip()
            if ball in code:
                raise SystemExit(f"ep2c5gate: ball {ball} twice in {path}")
            d, pu = r["dir"].strip(), r["pullup"].strip()
            if d not in ("input", "output") or pu not in ("on", "off"):
                raise SystemExit(f"ep2c5gate: bad dir/pullup {d!r}/{pu!r} for {ball}")
            bh = (r.get("bushold") or "off").strip()
            if bh not in ("on", "off") or (bh == "on" and (pu == "on" or d != "input")):
                raise SystemExit(f"ep2c5gate: bad bushold {bh!r} for {ball}")
            code[ball] = pu == "on"
            hold[ball] = bh == "on"
            if d == "output":
                outs.add(ball)
    if with_hold:
        return code, outs, hold
    return code, outs


SCAN2_V1_EXTRA = ("C4", "C5")


def check_scan2_table(g, k, path, base):
    """--scan2 K: the variant table differs from the scan table only as allowed."""
    def rows_of(p):
        with open(p, encoding="utf-8") as f:
            return {r["ball"].strip(): r for r in csv.DictReader(f)}
    v, b = rows_of(path), rows_of(base)
    if set(v) != set(b):
        g.fail(f"scan2 table: balls differ from {os.path.basename(base)}: "
               f"{sorted(set(v) ^ set(b))}")
        return
    for ball in sorted(v):
        for c in ("dir", "kind", "signal", "raw", "d_bit", "clk", "dc"):
            if (v[ball].get(c) or "").strip() != (b[ball].get(c) or "").strip():
                g.fail(f"scan2 table: {ball} {c} {v[ball].get(c)!r} != base {b[ball].get(c)!r}")
        if (b[ball].get("bushold") or "off").strip() != "off":
            g.fail(f"scan2 table: base has bus hold on {ball}")
    outs = {x for x in v if v[x]["dir"].strip() == "output"}
    want_outs = {x for x in v if re.fullmatch(r"MCU_DATA\[\d+\]", v[x]["signal"].strip())}
    if outs != want_outs or len(outs) != 16:
        g.fail(f"scan2 table: outputs {sorted(outs)} are not the 16 MCU_DATA balls")
    dc = {x for x in v if v[x]["signal"].strip().startswith("DC_")}
    if len(dc) != 33:
        g.fail(f"scan2 table: {len(dc)} DC_* balls, expected 33")
    want = {0: set(), 1: dc | set(SCAN2_V1_EXTRA), 2: dc}[k]
    diff = set()
    for ball in v:
        pu = v[ball]["pullup"].strip()
        bh = (v[ball].get("bushold") or "off").strip()
        if pu != b[ball]["pullup"].strip() or bh != "off":
            diff.add(ball)
        if ball in want:
            exp = ("off", "on" if k == 2 else "off")
            if (pu, bh) != exp:
                g.fail(f"scan2 table v{k}: {ball} pull-up {pu} / bus hold {bh}, expected "
                       f"{exp[0]} / {exp[1]}")
    if diff != want:
        g.fail(f"scan2 table v{k}: I/O settings differ from the base on "
               f"{sorted(diff - want)} (unexpected) / not on {sorted(want - diff)}")
    g.note(f"scan2 table v{k}: {len(diff)} balls differ from the base "
           f"({'pull-up off' if k == 1 else 'pull-up off, bus hold on' if k == 2 else 'none'})")


def code_port(ball):
    return "p_" + ball


def bh_ok(cell, want_on):
    """Bus Hold cell matches the expectation (On = yes/on, Off = no/off/--/empty)."""
    c = cell.strip().lower()
    return c in ("yes", "on") if want_on else c in ("no", "off", "--", "")


def check_pin(g, text, device, users, npins, balls=None, code=None, outs=frozenset()):
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
    if code is not None:
        for b in sorted(code):
            if balls.get(b) not in PORT_ROLES:
                g.fail(f".pin: code ball {b} is not an I/O or clock ball "
                       f"(table: {balls.get(b)})")
        for b, role in sorted(balls.items()):
            if role in PORT_ROLES and b not in code:
                g.fail(f".pin: I/O / clock ball {b} has no entry in the code table")
    for r in rows:
        name, d = r["name"], r["dir"]
        where = f"{r['loc']} {name!r} dir={d or '-'}"
        role = balls.get(r["loc"])
        if role is None:
            g.fail(f".pin: ball not in the expected EP2C5F256C8 table: {where}")
            continue
        if code is not None and r["loc"] in code:
            if name != code_port(r["loc"]):
                g.fail(f".pin: code ball is not its input port {code_port(r['loc'])}: {where}")
            elif r["loc"] in outs:
                if d not in ("output", "bidir"):
                    g.fail(f".pin: scan output port not an output: {where}")
            elif d != "input":
                g.fail(f".pin: code port not an input: {where}")
            seen_users.add(name)
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


def check_package(g, tables, code, required, outs=frozenset(), hold=None):
    """All Package Pins: Bus Hold off everywhere; Weak Pull Up as expected.

    code: {ball: pull-up expected} for the balls that must be input ports
    (pull-up code mode), or None (stage-0: no pull-up anywhere).
    """
    t, rows = find_table(tables, "All Package Pins")
    if not t or not rows:
        if required:
            g.fail("fit.rpt: All Package Pins table missing")
        return
    hdr = rows[0]
    iloc, iname, idir = col(hdr, "Location"), col(hdr, "Pin Name/Usage"), col(hdr, "Dir.")
    ibh, ipu = col(hdr, "Bus Hold"), col(hdr, "Weak Pull Up")
    if None in (iloc, iname, idir, ibh, ipu):
        g.fail("fit.rpt: All Package Pins table lacks Location/Pin Name/Dir./Bus Hold/Weak Pull Up")
        return
    seen = set()
    npu = 0
    for r in rows[1:]:
        if len(r) <= max(iloc, iname, idir, ibh, ipu):
            g.fail(f"fit.rpt: short All Package Pins row: {r}")
            continue
        loc, name, d = r[iloc], r[iname], r[idir].lower()
        bh, pu = r[ibh].lower(), r[ipu].lower()
        want_bh = bool(hold and hold.get(loc))
        if not bh_ok(bh, want_bh):
            g.fail(f"fit.rpt: package pin {loc} {name} bus hold {r[ibh]}, "
                   f"expected {'on' if want_bh else 'off'}")
        if pu == "on":
            npu += 1
        if code is not None and loc in code:
            seen.add(loc)
            want = "on" if code[loc] else "off"
            want_dir = ("output", "bidir") if loc in outs else ("input",)
            if name != code_port(loc) or d not in want_dir:
                g.fail(f"fit.rpt: package pin {loc} is {name!r} dir={d}, "
                       f"expected {'/'.join(want_dir)} {code_port(loc)}")
            if pu != want:
                g.fail(f"fit.rpt: package pin {loc} weak pull-up {r[ipu]}, expected {want}")
        elif pu not in ("off", "--", ""):
            g.fail(f"fit.rpt: package pin {loc} {name} weak pull-up {r[ipu]}, expected off")
        if d in ("output", "bidir") and loc not in outs \
                and name not in ("#TDO", "^nSTATUS", "^CONF_DONE"):
            g.fail(f"fit.rpt: package pin {loc} {name} dir={d}")
    if code is not None:
        for b in sorted(set(code) - seen):
            g.fail(f"fit.rpt: code ball {b} missing from All Package Pins")
        g.note(f"fit.rpt: All Package Pins weak pull-up On on {npu} balls "
               f"(expected {sum(code.values())})")


def check_drivers(g, tables, code, outs):
    """Scan mode: Output Pins + Bidir Pins list exactly the output ports."""
    want = {code_port(b): b for b in outs}
    seen = set()
    for kind in ("Output Pins", "Bidir Pins"):
        t, rows = find_table(tables, kind)
        if not t or len(rows) < 2:
            continue
        hdr = rows[0]
        ipu, ibh, iloc = col(hdr, "Weak Pull Up"), col(hdr, "Bus Hold"), col(hdr, "Pin #")
        if None in (ipu, ibh, iloc):
            g.fail(f"fit.rpt: {kind} table lacks Pin #/Weak Pull Up/Bus Hold column")
            continue
        for r in rows[1:]:
            name = r[0]
            loc = r[iloc] if iloc < len(r) else "?"
            if name not in want or loc != want[name]:
                g.fail(f"fit.rpt: {kind} row {name} at {loc} is not a scan output port")
                continue
            if name in seen:
                g.fail(f"fit.rpt: scan output {name} listed twice")
            seen.add(name)
            exp = "on" if code[want[name]] else "off"
            if ipu >= len(r) or r[ipu].lower() != exp:
                g.fail(f"fit.rpt: output {name} weak pull-up "
                       f"{r[ipu] if ipu < len(r) else '?'}, expected {exp}")
            if ibh >= len(r) or not bh_ok(r[ibh], False):
                g.fail(f"fit.rpt: output {name} bus hold {r[ibh] if ibh < len(r) else '?'}")
    for name in sorted(set(want) - seen):
        g.fail(f"fit.rpt: scan output {name} not in the Output Pins / Bidir Pins tables")
    g.note(f"fit.rpt: {len(seen)} output/bidir ports (expected {len(want)})")


def check_pll(g, tables, required):
    """Fitter Summary 'Total PLLs' must be 0 / N."""
    val = None
    for t, rows in tables.items():
        if not t.lower().startswith("fitter summary"):
            continue
        for r in rows:
            if len(r) >= 2 and r[0].strip().lower() == "total plls":
                val = r[1].strip()
    if val is None:
        if required:
            g.fail("fit.rpt: PLL count (Fitter Summary 'Total PLLs') not found")
        return
    if not re.match(r"^0\s*/", val):
        g.fail(f"fit.rpt: PLLs used: {val!r}, expected 0")
    else:
        g.note(f"fit.rpt: Total PLLs {val}")


def check_fit(g, text, users, code=None, outs=frozenset(), scan=False, hold=None):
    tables = rpt_tables(text)
    if scan:
        check_drivers(g, tables, code, outs)
    else:
        for kind in ("Output Pins", "Bidir Pins"):
            t, rows = find_table(tables, kind)
            if t and len(rows) > 1:
                g.fail(f"fit.rpt: {kind} table has {len(rows) - 1} row(s): "
                       + ", ".join(r[0] for r in rows[1:]))
    check_pll(g, tables, required=scan)
    pkg_code = code
    if code is not None and outs:
        users = users - {code_port(b) for b in outs}
        code = {b: v for b, v in code.items() if b not in outs}
    t, rows = find_table(tables, "Input Pins")
    if t and rows:
        hdr = rows[0]
        ipu = col(hdr, "Weak Pull Up")
        ibh = col(hdr, "Bus Hold")
        iloc = col(hdr, "Pin #")
        seen = set()
        for r in rows[1:]:
            if code is not None:
                loc = r[iloc] if iloc is not None and iloc < len(r) else ""
                seen.add(loc)
                if loc not in code or r[0] != code_port(loc):
                    g.fail(f"fit.rpt: input {r[0]} at {loc or '?'} is not a code port")
                elif ipu is None or r[ipu].lower() != ("on" if code[loc] else "off"):
                    g.fail(f"fit.rpt: input {r[0]} weak pull-up "
                           f"{r[ipu] if ipu is not None else '?'}, expected "
                           f"{'On' if code[loc] else 'Off'}")
            elif ipu is not None and r[ipu].lower() not in ("off", ""):
                g.fail(f"fit.rpt: input {r[0]} weak pull-up {r[ipu]}")
            loc_bh = r[iloc] if iloc is not None and iloc < len(r) else ""
            want_bh = bool(hold and hold.get(loc_bh))
            if ibh is not None and not bh_ok(r[ibh], want_bh):
                g.fail(f"fit.rpt: input {r[0]} bus hold {r[ibh]}, "
                       f"expected {'on' if want_bh else 'off'}")
            if r[0] not in users:
                g.fail(f"fit.rpt: unexpected input pin {r[0]}")
        if code is not None:
            for b in sorted(set(code) - seen):
                g.fail(f"fit.rpt: code ball {b} not in the Input Pins table")
            if ipu is None or ibh is None or iloc is None:
                g.fail("fit.rpt: Input Pins table lacks Pin #/Weak Pull Up/Bus Hold column")
        if ipu is None or ibh is None:
            g.note("fit.rpt: Input Pins table lacks Weak Pull Up/Bus Hold column")
    else:
        g.fail("fit.rpt: Input Pins table missing (cannot check weak pull-up / bus hold)")
    check_dual_options(g, tables)
    check_package(g, tables, pkg_code, required=pkg_code is not None, outs=outs, hold=hold)


def check_dual_options(g, tables):
    """Dual-purpose pins not kept / driven; the stage-0 device options."""
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


# ---------------------------------------------------------------- v55g ----

V55G_DC_PREFIX = "DC_"
# The 24 G1 outputs / bidirs that g1gate drives (ep2c5_v55g.csv "strength").
V55G_STRENGTH = "4ma"


def load_v55g(path):
    """Return {signal: row} from ep2c5_v55g.csv (mk_v55g.py)."""
    out = {}
    with open(path, encoding="utf-8") as f:
        for r in csv.DictReader(f):
            s = r["signal"].strip()
            if s in out:
                raise SystemExit(f"ep2c5gate: signal {s} twice in {path}")
            if r["dir"] not in ("input", "output", "bidir") or r["pullup"] not in ("on", "off"):
                raise SystemExit(f"ep2c5gate: bad dir / pullup for {s} in {path}")
            out[s] = r
    balls = [r["ball"] for r in out.values()]
    if len(set(balls)) != len(balls):
        raise SystemExit(f"ep2c5gate: a ball twice in {path}")
    return out


def check_v55g_table(g, tab, padmap):
    """The table is the v5.5 pin list carried by the pad map, plus R14 / N8."""
    with open(padmap, encoding="utf-8") as f:
        pm = {r["v55_signal"].strip(): r for r in csv.DictReader(f) if r["v55_signal"].strip()}
    want = set(pm) | {"G_MASTERn", "G_WRAPn"}
    if set(tab) != want:
        g.fail(f"v55g table: signals differ from the pad map: {sorted(set(tab) ^ want)}")
    for s, r in sorted(tab.items()):
        if s in pm and pm[s]["f256_ball"] != r["ball"]:
            g.fail(f"v55g table: {s} at {r['ball']}, pad map says {pm[s]['f256_ball']}")
        dc_out = s.startswith(V55G_DC_PREFIX) and pm.get(s, {}).get("v55_dir") in ("output", "bidir")
        if dc_out != (r["strength"].lower() == V55G_STRENGTH):
            g.fail(f"v55g table: {s} strength {r['strength']!r}")
        if dc_out and r["dir"] != "bidir":
            g.fail(f"v55g table: DC output {s} is not bidir (no readback)")
        if s.startswith(V55G_DC_PREFIX) and r["pullup"] != "off":
            g.fail(f"v55g table: weak pull-up on {s}")
    if tab.get("G_MASTERn", {}).get("ball") != "R14" or tab.get("G_WRAPn", {}).get("ball") != "N8":
        g.fail("v55g table: G_MASTERn / G_WRAPn not at R14 / N8")
    for s in ("G_MASTERn", "G_WRAPn"):
        if tab.get(s, {}).get("pullup") != "on" or tab.get(s, {}).get("dir") != "input":
            g.fail(f"v55g table: {s} must be an input with weak pull-up")
    if tab.get("CLK_11_2896_MHz", {}).get("ball") != "J2" or tab.get("CLK_48_MHz", {}).get("ball") != "H16":
        g.fail("v55g table: clocks not at J2 / H16")
    outs = sorted(s for s, r in tab.items() if r["dir"] != "input")
    n_dc = sum(1 for s in outs if s.startswith(V55G_DC_PREFIX))
    if n_dc != 24 or len(outs) != 41:
        g.fail(f"v55g table: {len(outs)} outputs ({n_dc} DC), expected 41 (24 DC)")
    g.note(f"v55g table: {len(tab)} ports, {len(outs)} output/bidir ({n_dc} G1 at 4mA)")


def check_v55g_pin(g, text, device, tab):
    chip, dev, rows = parse_pin(text)
    if dev is None or dev.upper() != device.upper():
        g.fail(f".pin: device {dev}, expected {device}")
    balls = load_balls()
    if len(rows) != 256:
        g.fail(f".pin: {len(rows)} rows, expected 256")
    at = {r["ball"]: s for s, r in tab.items()}
    seen = set()
    for b in sorted(set(balls) - {r["loc"] for r in rows}):
        g.fail(f".pin: expected ball {b} missing")
    for r in rows:
        name, d, loc = r["name"], r["dir"], r["loc"]
        where = f"{loc} {name!r} dir={d or '-'}"
        role = balls.get(loc)
        if role is None:
            g.fail(f".pin: unknown ball {where}")
            continue
        if loc in at:
            s = at[loc]
            if name != s:
                g.fail(f".pin: ball {loc} is {name!r}, expected {s}")
                continue
            if d != tab[s]["dir"]:
                g.fail(f".pin: {s} dir {d}, expected {tab[s]['dir']}")
            if role not in PORT_ROLES:
                g.fail(f".pin: {s} on a non-I/O ball ({role})")
            seen.add(s)
            continue
        if name in tab:
            g.fail(f".pin: {name} at {loc}, expected {tab[name]['ball']}")
            continue
        m = DUAL.match(name)
        if m:
            if role != "RESERVED_INPUT" or (m.group(2) or "").strip() != "RESERVED_INPUT" \
                    or d not in OK_INPUT_DIRS:
                g.fail(f".pin: dual-purpose pin not input tri-stated: {where}")
            continue
        if role == "USER":
            # stage-0's input ball H1 is a dedicated clock input; unused it
            # is listed as GND+ (input only in silicon)
            if name not in ("GND+", "RESERVED_INPUT") or d not in OK_INPUT_DIRS:
                g.fail(f".pin: unused clock ball not GND+ / RESERVED_INPUT: {where}")
            continue
        if role == "RESERVED_INPUT":
            if name != "RESERVED_INPUT" or d not in OK_INPUT_DIRS:
                g.fail(f".pin: unused I/O ball not RESERVED_INPUT: {where}")
            continue
        if name != role:
            g.fail(f".pin: ball {loc} is {name!r}, expected {role!r}")
        elif "*" in name or BAD_USAGE.search(name):
            g.fail(f".pin: driving / pulled pin: {where}")
        elif role in DEDICATED_ANY_DIR:
            pass
        elif role in DEDICATED_INPUT or role in ("NC", "GND+"):
            if d not in OK_INPUT_DIRS:
                g.fail(f".pin: pin not an input: {where}")
        elif role.startswith(("VCC", "GND")):
            if d not in POWER_DIRS | OK_INPUT_DIRS:
                g.fail(f".pin: supply pin with direction: {where}")
        else:
            g.fail(f".pin: unknown role {role!r}: {where}")
    for s in sorted(set(tab) - seen):
        g.fail(f".pin: {s} not listed at {tab[s]['ball']}")
    return rows


def _summary_value(tables, label):
    for t, rows in tables.items():
        if not t.lower().startswith(("fitter summary", "flow summary")):
            continue
        for r in rows:
            if len(r) >= 2 and r[0].strip().lower() == label.lower():
                return r[1].strip()
    return None


def check_v55g_fit(g, text, tab, device):
    tables = rpt_tables(text)
    dev = _summary_value(tables, "Device")
    if dev is None or dev.upper() != device.upper():
        g.fail(f"fit.rpt: device {dev!r}, expected {device}")
    for label in ("Total logic elements", "Total memory bits", "Total registers",
                  "Total pins", "Total PLLs"):
        g.note(f"fit.rpt: {label}: {_summary_value(tables, label)}")
    # --- outputs / bidirs: exactly the table's, at their balls, 4 mA on G1
    want_out = {s for s, r in tab.items() if r["dir"] != "input"}
    seen = set()
    for kind in ("Output Pins", "Bidir Pins"):
        t, rows = find_table(tables, kind)
        if not t or len(rows) < 2:
            continue
        hdr = rows[0]
        iloc, ipu, ibh, icur = (col(hdr, "Pin #"), col(hdr, "Weak Pull Up"),
                                col(hdr, "Bus Hold"), col(hdr, "Current Strength"))
        if None in (iloc, ipu, ibh, icur):
            g.fail(f"fit.rpt: {kind} table lacks Pin # / Weak Pull Up / Bus Hold / Current Strength")
            continue
        for r in rows[1:]:
            name = r[0]
            if len(r) <= max(iloc, ipu, ibh, icur):
                g.fail(f"fit.rpt: short {kind} row {r}")
                continue
            if name not in want_out:
                g.fail(f"fit.rpt: unexpected {kind} row {name} at {r[iloc]}")
                continue
            want_kind = "Bidir Pins" if tab[name]["dir"] == "bidir" else "Output Pins"
            if kind != want_kind:
                g.fail(f"fit.rpt: {name} in {kind}, expected {want_kind}")
            if name in seen:
                g.fail(f"fit.rpt: {name} listed twice")
            seen.add(name)
            if r[iloc] != tab[name]["ball"]:
                g.fail(f"fit.rpt: {name} at {r[iloc]}, expected {tab[name]['ball']}")
            exp_pu = "on" if tab[name]["pullup"] == "on" else "off"
            if r[ipu].lower() != exp_pu:
                g.fail(f"fit.rpt: {name} weak pull-up {r[ipu]}, expected {exp_pu}")
            if not bh_ok(r[ibh], False):
                g.fail(f"fit.rpt: {name} bus hold {r[ibh]}")
            cur = r[icur].replace(" ", "").lower()
            if tab[name]["strength"]:
                if cur != tab[name]["strength"].lower():
                    g.fail(f"fit.rpt: {name} current strength {r[icur]}, expected "
                           f"{tab[name]['strength']}")
                else:
                    g.note(f"fit.rpt: {name} {r[iloc]} current strength {r[icur]}")
            else:
                g.note(f"fit.rpt: {name} {r[iloc]} current strength {r[icur]} (MCU side)")
    for s in sorted(want_out - seen):
        g.fail(f"fit.rpt: {s} missing from the Output / Bidir Pins tables")
    n4 = sum(1 for s in seen if tab[s]["strength"])
    g.note(f"fit.rpt: {len(seen)} output/bidir pins (expected {len(want_out)}), "
           f"{n4} G1 pins at 4mA")
    # --- inputs
    want_in = {s for s, r in tab.items() if r["dir"] == "input"}
    t, rows = find_table(tables, "Input Pins")
    seen_in = set()
    if t and len(rows) > 1:
        hdr = rows[0]
        iloc, ipu, ibh = col(hdr, "Pin #"), col(hdr, "Weak Pull Up"), col(hdr, "Bus Hold")
        if None in (iloc, ipu, ibh):
            g.fail("fit.rpt: Input Pins table lacks Pin # / Weak Pull Up / Bus Hold")
        else:
            for r in rows[1:]:
                name = r[0]
                if name not in want_in:
                    g.fail(f"fit.rpt: unexpected input {name}")
                    continue
                seen_in.add(name)
                if r[iloc] != tab[name]["ball"]:
                    g.fail(f"fit.rpt: input {name} at {r[iloc]}, expected {tab[name]['ball']}")
                exp_pu = "on" if tab[name]["pullup"] == "on" else "off"
                if r[ipu].lower() != exp_pu:
                    g.fail(f"fit.rpt: input {name} weak pull-up {r[ipu]}, expected {exp_pu}")
                if not bh_ok(r[ibh], False):
                    g.fail(f"fit.rpt: input {name} bus hold {r[ibh]}")
    else:
        g.fail("fit.rpt: Input Pins table missing")
    for s in sorted(want_in - seen_in):
        g.fail(f"fit.rpt: input {s} missing from the Input Pins table")
    # --- every package pin: pull-up / bus hold / direction
    at = {r["ball"]: s for s, r in tab.items()}
    t, rows = find_table(tables, "All Package Pins")
    if not t or len(rows) < 2:
        g.fail("fit.rpt: All Package Pins table missing")
    else:
        hdr = rows[0]
        iloc, iname, idir = col(hdr, "Location"), col(hdr, "Pin Name/Usage"), col(hdr, "Dir.")
        ibh, ipu = col(hdr, "Bus Hold"), col(hdr, "Weak Pull Up")
        if None in (iloc, iname, idir, ibh, ipu):
            g.fail("fit.rpt: All Package Pins table lacks columns")
        else:
            for r in rows[1:]:
                if len(r) <= max(iloc, iname, idir, ibh, ipu):
                    continue
                loc, name, d = r[iloc], r[iname], r[idir].lower()
                pu = r[ipu].lower()
                if not bh_ok(r[ibh], False):
                    g.fail(f"fit.rpt: package pin {loc} {name} bus hold {r[ibh]}")
                if loc in at:
                    s = at[loc]
                    exp_pu = "on" if tab[s]["pullup"] == "on" else "off"
                    if name != s or d != tab[s]["dir"]:
                        g.fail(f"fit.rpt: package pin {loc} is {name!r} {d}, expected "
                               f"{s} {tab[s]['dir']}")
                    if pu != exp_pu:
                        g.fail(f"fit.rpt: package pin {loc} {s} weak pull-up {r[ipu]}")
                else:
                    if pu not in ("off", "--", ""):
                        g.fail(f"fit.rpt: package pin {loc} {name} weak pull-up {r[ipu]}")
                    if d in ("output", "bidir") and name not in ("#TDO", "^nSTATUS", "^CONF_DONE"):
                        g.fail(f"fit.rpt: package pin {loc} {name} dir={d}")
    # --- PLL policy: exactly Audio_PLL, fed by CLK_11_2896_MHz on J2
    v = _summary_value(tables, "Total PLLs")
    if v is None or not re.match(r"^1\s*/", v):
        g.fail(f"fit.rpt: Total PLLs {v!r}, expected 1 (Audio_PLL only)")
    plls = [rows for t, rows in tables.items() if t.lower().startswith("pll summary")]
    if len(plls) != 1:
        g.fail(f"fit.rpt: {len(plls)} PLL Summary tables, expected 1")
    else:
        kv = {r[0].strip().lower(): r[1].strip() for r in plls[0] if len(r) >= 2}
        if "audio_pll" not in kv.get("name", "").lower():
            g.fail(f"fit.rpt: PLL is {kv.get('name')!r}, expected Audio_PLL")
        if kv.get("inclk0 signal") != "CLK_11_2896_MHz":
            g.fail(f"fit.rpt: PLL inclk0 {kv.get('inclk0 signal')!r}, expected CLK_11_2896_MHz")
        if kv.get("inclk0 signal type", "").lower() != "dedicated pin":
            g.fail(f"fit.rpt: PLL inclk0 type {kv.get('inclk0 signal type')!r}")
        if kv.get("inclk1 signal", "--") not in ("--", ""):
            g.fail(f"fit.rpt: PLL inclk1 {kv.get('inclk1 signal')!r}")
        g.note(f"fit.rpt: PLL {kv.get('name')} at {kv.get('pll location')}, inclk0 "
               f"{kv.get('inclk0 signal')} ({kv.get('inclk0 signal type')}), "
               f"{kv.get('input frequency 0')}")
    check_dual_options(g, tables)


STA_SUMMARY = re.compile(r"^(Slow|Fast) Model (Setup|Hold|Recovery|Removal|Minimum Pulse Width)"
                         r" Summary$", re.I)


def check_v55g_sta(g, text):
    if "Timing requirements not met" in text:
        g.fail("sta.rpt: timing requirements not met")
    n = 0
    clocks = set()
    for t, rows in rpt_tables(text).items():
        m = STA_SUMMARY.match(t.strip())
        if not m:
            continue
        for r in rows[1:]:
            if len(r) >= 2 and re.match(r"^-?\d+(\.\d+)?$", r[1]):
                n += 1
                clocks.add(r[0])
                slack = float(r[1])
                g.note(f"sta.rpt: {m.group(1)} {m.group(2)} {r[0]}: slack {slack}")
                if slack < 0:
                    g.fail(f"sta.rpt: {m.group(1)} {m.group(2)} {r[0]} slack {slack}")
    if n == 0:
        g.fail("sta.rpt: no slack summary found")
    if "CLK_48_MHz" not in clocks:
        g.fail("sta.rpt: CLK_48_MHz not in the slack summaries")


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


def report(g, a, rows, pin):
    if a.rbf:
        if os.path.exists(a.rbf):
            with open(a.rbf, "rb") as f:
                data = f.read()
            print(f"rbf: {a.rbf} size {len(data)} bytes sha256 "
                  f"{hashlib.sha256(data).hexdigest()}")
        else:
            g.fail(f"rbf not found: {a.rbf}")
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


def main_v55g(a):
    """--v55g: the v5.5 core + g1gate image (fpga/v55g-ep2c5).

    Table (ep2c5_v55g.csv, from mk_v55g.py) against the pad map; .pin: every
    port at its ball with its direction, nothing else used, unused balls
    RESERVED_INPUT; fit.rpt: Output + Bidir Pins exactly the table's 41
    (16 MCU_DATA + MCU_IRQn + 24 G1), Current Strength 4mA on each of the 24
    G1 pins, weak pull-up exactly where the table says (R14, N8, MCU
    strobes), bus hold off everywhere, inputs exactly the table's, PLL
    policy (1 PLL = Audio_PLL, inclk0 CLK_11_2896_MHz on a dedicated pin),
    stage-0 device options; sta.rpt: every slack summary >= 0.
    """
    table = a.table
    if os.path.basename(table) == "ep2c5_scan.csv":
        table = os.path.join(os.path.dirname(os.path.abspath(__file__)), "ep2c5_v55g.csv")
    tab = load_v55g(table)
    pin, fit = locate(a.path, a.fit)
    sta = a.sta or os.path.splitext(pin)[0] + ".sta.rpt"
    g = Gate()
    check_v55g_table(g, tab, a.padmap)
    rows = check_v55g_pin(g, read(pin), a.device, tab)
    if os.path.exists(fit):
        check_v55g_fit(g, read(fit), tab, a.device)
    else:
        g.fail(f"fit.rpt not found: {fit}")
    if os.path.exists(sta):
        check_v55g_sta(g, read(sta))
    else:
        g.fail(f"sta.rpt not found: {sta}")
    return report(g, a, rows, pin)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("path", help="output_files dir or the .pin file")
    ap.add_argument("--fit", help="fit.rpt (default: next to the .pin)")
    ap.add_argument("--device", default="EP2C5F256C8")
    ap.add_argument("--user", action="append", default=None,
                    help="allowed user input pin name (repeatable; default stage0_in)")
    ap.add_argument("--pins", type=int, default=256,
                    help="expected .pin row count (0 = do not check)")
    ap.add_argument("--pullcode", type=int, default=None, metavar="K",
                    help="pull-up code variant K (0..9); needs --code")
    ap.add_argument("--code", help="code table csv (tools/pinfind/ep2c5_pullcode.csv)")
    ap.add_argument("--scan", action="store_true",
                    help="scan core mode (fpga/scan-ep2c5); table from --table")
    ap.add_argument("--table", default=os.path.join(os.path.dirname(os.path.abspath(__file__)),
                                                    "ep2c5_scan.csv"),
                    help="scan table csv (default tools/pinfind/ep2c5_scan.csv)")
    ap.add_argument("--scan2", type=int, default=None, metavar="K",
                    help="scan2 core variant K (0..2, fpga/scan2-ep2c5); table from --table")
    ap.add_argument("--base", default=os.path.join(os.path.dirname(os.path.abspath(__file__)),
                                                   "ep2c5_scan.csv"),
                    help="--scan2: the scan table the variant table is checked against")
    ap.add_argument("--v55g", action="store_true",
                    help="v55g image (fpga/v55g-ep2c5); table --table ep2c5_v55g.csv, "
                         "--sta sta.rpt")
    ap.add_argument("--padmap", default=os.path.join(os.path.dirname(os.path.abspath(__file__)),
                                                     "ep2c5_padmap.csv"),
                    help="--v55g: the pad map the table must follow")
    ap.add_argument("--sta", help="--v55g: TimeQuest sta.rpt (default: next to the .pin)")
    ap.add_argument("--rbf", help="bitstream: print its size and SHA-256")
    ap.add_argument("-q", "--quiet", action="store_true")
    a = ap.parse_args(argv)
    if a.v55g:
        return main_v55g(a)
    code = None
    hold = None
    outs = frozenset()
    if a.scan2 is not None:
        if a.scan2 not in (0, 1, 2):
            ap.error("--scan2 K needs K in 0..2")
        a.scan = True
    if a.scan and a.pullcode is not None:
        ap.error("--scan and --pullcode are exclusive")
    if a.scan:
        code, outs, hold = load_scan(a.table, with_hold=True)
        outs = frozenset(outs)
        users = {code_port(b) for b in code}
    elif a.pullcode is not None:
        if not a.code or not 0 <= a.pullcode <= 9:
            ap.error("--pullcode K needs 0 <= K <= 9 and --code")
        code = load_code(a.code, a.pullcode)
        users = {code_port(b) for b in code}
    else:
        users = set(a.user or ["stage0_in"])
    pin, fit = locate(a.path, a.fit)
    g = Gate()
    if a.scan2 is not None:
        check_scan2_table(g, a.scan2, a.table, a.base)
        g.note(f"scan2 variant v{a.scan2}: bus hold On on {sum(hold.values())} balls")
    if a.scan:
        g.note(f"scan core: {len(code)} ports, {len(outs)} outputs "
               f"({' '.join(sorted(outs))}), {sum(code.values())} with weak pull-up On")
    elif code is not None:
        g.note(f"pull-up code variant k={a.pullcode}: {len(code)} ports, "
               f"{sum(code.values())} with weak pull-up On")
    rows = check_pin(g, read(pin), a.device, users, a.pins, code=code, outs=outs)
    if os.path.exists(fit):
        check_fit(g, read(fit), users, code=code, outs=outs, scan=a.scan, hold=hold)
    else:
        g.fail(f"fit.rpt not found: {fit}")
    if a.rbf:
        if os.path.exists(a.rbf):
            with open(a.rbf, "rb") as f:
                data = f.read()
            print(f"rbf: {a.rbf} size {len(data)} bytes sha256 "
                  f"{hashlib.sha256(data).hexdigest()}")
        else:
            g.fail(f"rbf not found: {a.rbf}")
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
