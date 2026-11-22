#!/usr/bin/env python3
"""Make the pin-finder's ball list from Supra's AG16KDDF256 pinmap.

usage: mklist.py $SUPRA_HOME/etc/AG16KDDF256.pinmap [-o fpga/pinfind/build/balls.txt]
                 [--exclude PIN_A1,PIN_B2 ...] [--exclude-file FILE] [--max-drive N]
                 [--no-hold-ddr] [--leave-out PIN_xx,...]

The pinmap stays in the Supra installation: this script only reads it and
writes ball names (facts about the package, no Supra text) to a file that is
NOT committed (fpga/pinfind/build/ is ignored). The bitstream generator
(fpga/pinfind/gen.py) and the host tools (tools/pinfind/pins2asf.py) read
that file; the index of a ball is its position in it, so keep the same file
for the build and for reading the results.

How the pinmap is read (docs/research/ag16k-supra-build.md section 2 and
fpga/agm/mkasf.py): a line whose first word is a ball name `PIN_<row><col>`
describes that ball. With exactly two words it is a plain user I/O (the
second word is the EP4CE15F23 pin Supra maps it to); with more words the
ball has a special function and is left out (its name is listed as a comment;
the rest of the line is not copied). Lines that
do not start with a ball name (DDR_*, NC_*, comments) are ignored.

Output, one ball per line, `<kind> <index> <ball>`:

  drive  -- scanned: the pin-finder drives it, one at a time (index = the
            number it sends). Only balls the pinmap lists as plain (two-word)
            user I/O can be drive balls.
  hold   -- kept in the design as an input that is never driven. Keeping a
            ball in the design makes it an input for sure, instead of whatever
            Supra does with unused balls (unknown; Quartus's old default was
            "output driving ground"). Hold balls are:
              * balls you excluded with --exclude (e.g. the 74HC04
                clock-buffer output once it is known) and the overflow of
                --max-drive;
              * the PS balls that become user I/O after configuration: DCLK
                G1 and DATA0 G2 (ag16k-config.md C9: the MCU drives them
                during configuration, so the design must keep them inputs),
                and ASDO C1 / NCSO D2 (normal I/O in PS mode). Always, even
                if the pinmap does not list them (Supra then refuses the
                assignment and build.sh stops: the safe failure);
              * every other ball the pinmap marks with a special function
                (more than two words) that is not a dedicated configuration /
                JTAG / NC ball or a clock input;
              * the package's DDR balls F10 M10 L11 M11 when the pinmap lists
                them as balls (--no-hold-ddr leaves them out, for when Supra
                refuses them as user I/O).
            --leave-out BALL removes a hold ball from the design (for a ball
            Supra refuses). build.sh refuses a design that leaves a
            user-capable ball unused unless that ball is named with
            --unused-ok (a human decision, recorded in the build report).
  clock  -- dedicated clock inputs (input-only, cannot be scanned). The
            pin-finder counts their frequency (gen.py --probes) or, without
            probes, keeps them as hold inputs.

Never listed, whatever the pinmap says (docs/research/ag16k-config.md 1-3,
read from the DDF256 manual): the dedicated configuration balls (nCONFIG H3,
nSTATUS F3, CONF_DONE F16, MSEL1 F15), JTAG (H1 H2 J1 J2) and NC H4. They are
not user I/O.
"""
import argparse
import hashlib
import os
import re
import sys

BALL = re.compile(r"^PIN_([A-HJ-NPRT])(\d{1,2})$")

# ag16k-config.md 1-3 (DDF256 manual, Table-1) and ag16k-supra-build.md 2.
# Dedicated configuration / JTAG / NC balls: never in the design.
DEDICATED = {
    "PIN_H3": "nCONFIG", "PIN_F3": "nSTATUS", "PIN_F16": "CONF_DONE", "PIN_F15": "MSEL1",
    "PIN_H1": "TDI", "PIN_H2": "TCK", "PIN_J1": "TMS", "PIN_J2": "TDO",
    "PIN_H4": "NC",
}
# Configuration balls that are user I/O after configuration: always hold
# (ag16k-config.md C9 for DCLK / DATA0, 1-2 for ASDO / NCSO).
CONFIG_HOLD = {"PIN_G1": "DCLK", "PIN_G2": "DATA0", "PIN_C1": "ASDO", "PIN_D2": "NCSO"}
# DDR balls that reach the package: hold unless --no-hold-ddr.
DDR = {"PIN_F10": "DDR_VREF1", "PIN_M10": "DDR_VREF0", "PIN_L11": "DDR_/CLK", "PIN_M11": "DDR_CLK"}
# Every ball that is never a drive ball by name.
CONFIG = {**DEDICATED, **CONFIG_HOLD, **DDR}
# Dedicated clock inputs, in CLKn order.
CLOCKS = [
    ("PIN_J4", "CLK1"), ("PIN_K1", "CLK2"), ("PIN_L1", "CLK3"), ("PIN_G16", "CLK4"),
    ("PIN_G15", "CLK5"), ("PIN_H16", "CLK6"), ("PIN_H15", "CLK7"), ("PIN_F9", "CLK10"),
    ("PIN_E9", "CLK11"), ("PIN_T14", "CLK14"), ("PIN_R14", "CLK15"),
]


def norm(b):
    b = b.strip().upper()
    if not b.startswith("PIN_"):
        b = "PIN_" + b
    if not BALL.match(b):
        raise ValueError(f"not a ball name: {b!r}")
    return b


def parse_pinmap(text):
    """Return (plain, special) as ordered lists: plain = [ball], special = [(ball, rest)]."""
    plain, special, seen = [], [], set()
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith(("#", "//", ";")):
            continue
        w = line.split()
        if not BALL.match(w[0]):
            continue
        if w[0] in seen:
            raise ValueError(f"pinmap lists {w[0]} twice")
        seen.add(w[0])
        if len(w) == 2:
            plain.append(w[0])
        else:
            special.append((w[0], " ".join(w[1:])))
    return plain, special


def make_list(plain, special, exclude=(), max_drive=None, hold_ddr=True, leave_out=()):
    """Return (drive, hold, clock, notes)."""
    exclude = [norm(b) for b in exclude]
    leave_out = sorted({norm(b) for b in leave_out})
    known = set(plain) | {b for b, _ in special}
    notes = []
    for b in exclude:
        if b not in known:
            notes.append(f"--exclude {b}: not in the pinmap, ignored")
    clock_balls = {b for b, _ in CLOCKS}
    drive, hold = [], []
    for b in plain:
        if b in CONFIG:
            if b in DEDICATED:
                notes.append(f"{b} ({CONFIG[b]}) is plain in the pinmap but is a dedicated "
                             f"configuration/JTAG ball: left out")
            continue
        if b in clock_balls:
            continue
        if b in exclude:
            hold.append(b)
        else:
            drive.append(b)
    if max_drive is not None and len(drive) > max_drive:
        hold += drive[max_drive:]
        drive = drive[:max_drive]
    for b, name in CONFIG_HOLD.items():
        if b not in known:
            notes.append(f"{b} ({name}) is not in the pinmap; kept as hold anyway "
                         f"(build.sh stops if Supra refuses it)")
        hold.append(b)
    for b, _ in special:
        if b in CONFIG or b in clock_balls:
            continue
        hold.append(b)
    for b, name in DDR.items():
        if b not in known:
            continue
        if hold_ddr:
            hold.append(b)
        else:
            notes.append(f"{b} ({name}) left out by --no-hold-ddr: unused in the design "
                         f"(build.sh needs --unused-ok {b})")
    for b in leave_out:
        if b in hold:
            hold.remove(b)
            notes.append(f"{b} left out by --leave-out: unused in the design (build.sh needs --unused-ok {b})")
        else:
            notes.append(f"--leave-out {b}: not a hold ball, ignored")
    clock = [b for b, _ in CLOCKS if b in known]
    for b, name in CLOCKS:
        if b not in known:
            notes.append(f"clock ball {b} ({name}) is not in the pinmap: not in the design")
    if len(drive) > 256:
        raise ValueError(f"{len(drive)} drive balls; the index is one byte (max 256). Use --exclude or --max-drive")
    if not drive:
        raise ValueError("no drive balls left")
    if len(set(hold)) != len(hold):
        raise ValueError("a hold ball is listed twice")
    return drive, hold, clock, notes


def user_capable(plain, special):
    """Balls of the pinmap that Supra may treat as user I/O: every PIN_ line
    except the dedicated configuration / JTAG / NC balls. build.sh requires
    each of them to be in the design, or named with --unused-ok."""
    return [b for b in list(plain) + [b for b, _ in special] if b not in DEDICATED]


def render(src_name, src_sha, drive, hold, clock, special, notes, argv):
    out = [
        "# NetGD pin-finder ball list. Generated by tools/pinfind/mklist.py; do not edit, do not commit.",
        f"# source: {src_name} sha256 {src_sha}",
        f"# args: {' '.join(argv)}",
        f"# drive {len(drive)}  hold {len(hold)}  clock {len(clock)}",
        "# <kind> <index> <ball>   (the index is what the pin-finder sends on a drive ball)",
    ]
    # Only the ball name: nothing else from the pinmap goes into the list.
    for b, _ in special:
        out.append(f"# special function in the pinmap (hold unless dedicated/clock): {b}")
    for n in notes:
        out.append(f"# note: {n}")
    for kind, lst in (("drive", drive), ("hold", hold), ("clock", clock)):
        out += [f"{kind} {i} {b}" for i, b in enumerate(lst)]
    return "\n".join(out) + "\n"


def read_list(path):
    """Parse a ball list. Returns {'drive': [...], 'hold': [...], 'clock': [...], 'sha256': ...}."""
    data = open(path, "rb").read()
    lists = {"drive": [], "hold": [], "clock": []}
    for n, raw in enumerate(data.decode().splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        w = line.split()
        if len(w) != 3 or w[0] not in lists:
            raise ValueError(f"{path}:{n}: expected '<drive|hold|clock> <index> <ball>'")
        kind, idx, ball = w[0], int(w[1]), norm(w[2])
        if idx != len(lists[kind]):
            raise ValueError(f"{path}:{n}: {kind} index {idx} out of order (expected {len(lists[kind])})")
        lists[kind].append(ball)
    allb = lists["drive"] + lists["hold"] + lists["clock"]
    if len(set(allb)) != len(allb):
        raise ValueError(f"{path}: a ball is listed twice")
    lists["sha256"] = hashlib.sha256(data.replace(b"\r\n", b"\n")).hexdigest()
    return lists


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    ap = argparse.ArgumentParser(description="Make the pin-finder ball list from Supra's pinmap.")
    ap.add_argument("pinmap")
    ap.add_argument("-o", "--output", help="output file (default: stdout)")
    ap.add_argument("--exclude", action="append", default=[],
                    help="balls never to drive (comma-separated, repeatable); they become 'hold' inputs")
    ap.add_argument("--exclude-file", help="file with one ball per line (# comments)")
    ap.add_argument("--max-drive", type=int, help="scan only the first N balls (the rest become hold)")
    ap.add_argument("--no-hold-ddr", action="store_true",
                    help="leave the DDR balls F10 M10 L11 M11 out of the design (only if Supra refuses them)")
    ap.add_argument("--leave-out", action="append", default=[],
                    help="hold balls to leave out of the design (comma-separated; only if Supra refuses them)")
    a = ap.parse_args(argv)

    excl = [b for e in a.exclude for b in e.split(",") if b.strip()]
    if a.exclude_file:
        for line in open(a.exclude_file, encoding="utf-8"):
            line = line.split("#")[0].strip()
            if line:
                excl.append(line)
    raw = open(a.pinmap, "rb").read()
    plain, special = parse_pinmap(raw.decode("utf-8", "replace"))
    lo = [b for e in a.leave_out for b in e.split(",") if b.strip()]
    drive, hold, clock, notes = make_list(plain, special, excl, a.max_drive,
                                          hold_ddr=not a.no_hold_ddr, leave_out=lo)
    text = render(os.path.basename(a.pinmap), hashlib.sha256(raw).hexdigest(),
                  drive, hold, clock, special, notes, argv)
    if a.output:
        os.makedirs(os.path.dirname(os.path.abspath(a.output)), exist_ok=True)
        with open(a.output, "w", newline="\n") as f:
            f.write(text)
        print(f"mklist.py: {len(drive)} drive, {len(hold)} hold, {len(clock)} clock -> {a.output}", file=sys.stderr)
    else:
        sys.stdout.write(text)
    for n in notes:
        print(f"mklist.py: note: {n}", file=sys.stderr)


if __name__ == "__main__":
    main()
