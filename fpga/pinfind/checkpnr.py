#!/usr/bin/env python3
"""After place-and-route: check that every port of the pin-finder is on the
ball the ball list says, that Supra placed nothing by itself, and that no
user-capable ball of the package is left out of the design.

Run by build.sh (WSL, `python3 -I`); can be run by hand on a finished work
directory:

    python3 -I fpga/pinfind/checkpnr.py --gen fpga/pinfind/build \\
        --work fpga/pinfind/build/supra --pinmap "$SUPRA_HOME/etc/AG16KDDF256.pinmap" \\
        [--placement FILE --placement-regex REGEX] [--unused-ok PIN_xx,...]

Exit 0 only if all of these hold (R143 item 1 and 2):

  1. balls.txt in --gen is the list params.txt was generated from (SHA-256),
     and PinFind.asf assigns exactly the balls portmap.py derives from it
     (one per port, none twice, nothing else).
  2. Supra's log has no automatic placement ("... placed at pin ...", or an
     I/O reported as not assigned after the .asf was read).
  3. The ACTUAL placement, read from Supra's own output, puts every port on
     its expected ball, and has no other I/O.
  4. Every ball of Supra's pinmap that may be user I/O (every PIN_ line but
     the dedicated nCONFIG / nSTATUS / CONF_DONE / MSEL1 / JTAG / NC balls)
     is a port of the design, unless it is named with --unused-ok. Supra's
     treatment of unused pins is unknown (Quartus's old default was "output
     driving ground"), so the design leaves none unused instead of relying on it.

Where the actual placement comes from (3). The format of Supra's reports is
NOT known to this script's author (no Supra on the machine it was written
on). The script therefore only accepts a source that lists every port with
exactly one ball, and stops otherwise:

  * built-in: a Quartus-style pin table (`<port> : <ball> : ...`, as in a
    `.pin` file) in any file of the work directory, or `// Location: PIN_xx`
    comments directly above the I/O cell of a port in the routed netlist;
  * or a source a human named after looking at the reports:
    --placement FILE --placement-regex 'REGEX' with named groups (?P<port>)
    and (?P<ball>), applied to each line.

Never point --placement at the .asf, the .qsf, a Tcl file, or a report that
only repeats the constraints: that would "confirm" the request, not the
result. Lines containing `set_location_assignment` are ignored for that reason.
"""
import argparse
import glob
import hashlib
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(HERE, "..", "..", "tools", "pinfind"))

import portmap  # noqa: E402
from mklist import BALL, DEDICATED, parse_pinmap, read_list, user_capable  # noqa: E402

AUTO_PLACED = re.compile(r"placed at pin|is not assigned,|auto[- ]?placed|automatically placed", re.I)
BALL_TOKEN = re.compile(r"(?<![A-Za-z0-9_])(?:PIN_)?([A-HJ-NPRT](?:1[0-6]|[1-9]))(?![A-Za-z0-9_])")
SKIP_FILES = (".asf", ".qsf", ".tcl", ".sdc", ".gz", ".rbf", ".bin", ".prg", ".vqm")


def norm_port(q):
    """`{P[3]}`, `\\P[3] `, `P[3]` -> `P[3]`."""
    q = q.strip().strip("{}").strip()
    if q.startswith("\\"):
        q = q[1:]
    return q.strip()


def norm_ball(b):
    b = b.strip().upper()
    if not b.startswith("PIN_"):
        b = "PIN_" + b
    return b if BALL.match(b) else None


def params_of(gen):
    p = {}
    with open(os.path.join(gen, "params.txt"), encoding="utf-8") as f:
        for line in f:
            w = line.split(None, 1)
            if len(w) == 2:
                p[w[0]] = w[1].strip()
    return p


def expected_ports(gen, p, balls_path):
    raw = open(balls_path, "rb").read()
    sha = hashlib.sha256(raw.replace(b"\r\n", b"\n")).hexdigest()
    if sha != p.get("balls_sha256"):
        raise SystemExit(f"checkpnr: {balls_path} (sha256 {sha}) is not the list params.txt was made from "
                         f"({p.get('balls_sha256')}); run gen.py again")
    balls = read_list(balls_path)
    plan = portmap.plan(balls, p.get("clock", "osc") if p.get("clock") != "none" else "osc",
                        probes=int(p.get("n_probe", 0)) > 0, stage0=p.get("stage0") == "1")
    return portmap.ports(plan)


def asf_ports(gen):
    out = []
    for line in open(os.path.join(gen, "PinFind.asf"), encoding="utf-8"):
        m = re.match(r"^\s*set_location_assignment\s+-to\s+(\S+)\s+(\S+)\s*$", line)
        if m:
            out.append((norm_port(m.group(1)), m.group(2)))
    return out


def auto_placed(work):
    hits = []
    for f in [os.path.join(work, "run.out")] + glob.glob(os.path.join(work, "alta_logs", "*")):
        if not os.path.isfile(f):
            continue
        for n, line in enumerate(open(f, encoding="utf-8", errors="replace"), 1):
            if AUTO_PLACED.search(line):
                hits.append(f"{os.path.relpath(f, work)}:{n}: {line.strip()}")
    return hits


def from_regex(path, regex, ports):
    rx = re.compile(regex)
    got = {}
    for line in open(path, encoding="utf-8", errors="replace"):
        if "set_location_assignment" in line:
            continue
        m = rx.search(line)
        if not m:
            continue
        q, b = norm_port(m.group("port")), norm_ball(m.group("ball"))
        if b is None:
            continue
        got.setdefault(q, set()).add(b)
    return got


def from_pin_table(path, ports):
    """Quartus .pin style: `<name> : <location> : ...`."""
    got = {}
    for line in open(path, encoding="utf-8", errors="replace"):
        if "set_location_assignment" in line or ":" not in line:
            continue
        cols = [c.strip() for c in line.split(":")]
        if len(cols) < 3:
            continue
        q = norm_port(cols[0])
        if q not in ports:
            continue
        b = norm_ball(cols[1])
        if b:
            got.setdefault(q, set()).add(b)
    return got


def from_location_comments(path, ports):
    """`// Location: PIN_xx` followed (within the cell) by a pad connection
    naming exactly one port."""
    got = {}
    lines = open(path, encoding="utf-8", errors="replace").read().splitlines()
    port_rx = re.compile(r"\.padio\s*\(\s*(\\?\S+?)\s*\)|\.(?:i|o)\s*\(\s*(\\?\S+?)\s*\)")
    for i, line in enumerate(lines):
        m = re.search(r"//\s*Location:\s*(PIN_\w+)", line)
        if not m:
            continue
        b = norm_ball(m.group(1))
        found = set()
        for nxt in lines[i + 1:i + 16]:
            if "Location:" in nxt:
                break
            for pm in port_rx.finditer(nxt):
                q = norm_port(pm.group(1) or pm.group(2))
                if q in ports:
                    found.add(q)
            if nxt.strip().endswith(");"):
                break
        if b and len(found) == 1:
            got.setdefault(found.pop(), set()).add(b)
    return got


def complete(got, ports):
    return set(got) >= set(ports) and all(len(got[q]) == 1 for q in ports)


def find_placement(work, ports, placement=None, regex=None):
    """Return (source description, {port: ball}) or raise SystemExit."""
    if placement:
        if not regex:
            raise SystemExit("checkpnr: --placement needs --placement-regex")
        if placement.lower().endswith(SKIP_FILES):
            raise SystemExit(f"checkpnr: {placement} is an input or a constraints file, not a placement result")
        got = from_regex(placement, regex, ports)
        if not complete(got, ports):
            missing = [q for q in ports if q not in got]
            multi = [q for q in ports if len(got.get(q, ())) > 1]
            raise SystemExit(f"checkpnr: --placement {placement}: {len(missing)} port(s) not found "
                             f"(e.g. {missing[:5]}), {len(multi)} with several balls (e.g. {multi[:5]})")
        return f"{placement} (--placement-regex)", {q: next(iter(got[q])) for q in ports}
    sources = []
    for f in sorted(glob.glob(os.path.join(work, "**", "*"), recursive=True)):
        if not os.path.isfile(f) or f.lower().endswith(SKIP_FILES) or os.path.getsize(f) > 64 << 20:
            continue
        for name, fn in (("pin table", from_pin_table), ("Location comments", from_location_comments)):
            got = fn(f, ports)
            if complete(got, ports):
                sources.append((f"{os.path.relpath(f, work)} ({name})", {q: next(iter(got[q])) for q in ports}))
    if not sources:
        hint = []
        for f in sorted(glob.glob(os.path.join(work, "**", "*"), recursive=True)):
            if os.path.isfile(f) and not f.lower().endswith(SKIP_FILES) and os.path.getsize(f) < 64 << 20:
                n = sum(1 for _ in BALL_TOKEN.finditer(open(f, encoding="utf-8", errors="replace").read()))
                if n:
                    hint.append(f"    {os.path.relpath(f, work)}: {n} ball-like tokens")
        raise SystemExit(
            "checkpnr: could not read the ACTUAL placement of every port from Supra's outputs, so the "
            "port -> ball assignment is NOT confirmed. Do not load this bitstream.\n"
            "  Find the report that lists the ball each I/O was placed on (not the constraints), then run\n"
            "  build.sh again with --placement FILE --placement-regex 'REGEX with (?P<port>..) and (?P<ball>..)'.\n"
            "  Files with ball names:\n" + ("\n".join(hint[:20]) or "    (none)"))
    first = sources[0][1]
    for desc, m in sources[1:]:
        if m != first:
            raise SystemExit(f"checkpnr: placement sources disagree: {sources[0][0]} vs {desc}")
    return " + ".join(d for d, _ in sources), first


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--gen", required=True)
    ap.add_argument("--work", required=True)
    ap.add_argument("--balls", help="ball list (default: <gen>/balls.txt)")
    ap.add_argument("--pinmap", required=True, help="Supra's AG16KDDF256.pinmap (read only, never copied)")
    ap.add_argument("--placement")
    ap.add_argument("--placement-regex")
    ap.add_argument("--unused-ok", action="append", default=[],
                    help="user-capable balls a human accepts as unused (comma-separated)")
    ap.add_argument("--report", help="write a summary here")
    a = ap.parse_args(argv)

    p = params_of(a.gen)
    exp = expected_ports(a.gen, p, a.balls or os.path.join(a.gen, "balls.txt"))
    errors, notes = [], []

    asf = asf_ports(a.gen)
    if asf != exp:
        extra = sorted(set(asf) - set(exp))
        missing = sorted(set(exp) - set(asf))
        errors.append(f"PinFind.asf does not match the ball list: {len(missing)} missing (e.g. {missing[:4]}), "
                      f"{len(extra)} unexpected (e.g. {extra[:4]}); run gen.py again")
    if str(len(exp)) != p.get("total_pins"):
        errors.append(f"params.txt total_pins {p.get('total_pins')} != {len(exp)} ports")
    if len({b for _, b in exp}) != len(exp):
        errors.append("a ball is on two ports")

    hits = auto_placed(a.work)
    if hits:
        errors.append("Supra placed I/O by itself (automatic placement):\n    " + "\n    ".join(hits[:20]))

    ports = [q for q, _ in exp]
    src, actual = find_placement(a.work, ports, a.placement, a.placement_regex)
    wrong = [(q, b, actual.get(q)) for q, b in exp if actual.get(q) != b]
    if wrong:
        errors.append(f"actual placement differs from the ball list ({src}):\n    " +
                      "\n    ".join(f"{q}: expected {b}, placed {got}" for q, b, got in wrong[:30]))
    else:
        notes.append(f"placement: all {len(exp)} ports on their expected balls ({src})")

    plain, special = parse_pinmap(open(a.pinmap, "rb").read().decode("utf-8", "replace"))
    design = {b for _, b in exp}
    ok = {norm_ball(b) for e in a.unused_ok for b in e.split(",") if b.strip()}
    ok.discard(None)
    unused = [b for b in user_capable(plain, special) if b not in design]
    bad = [b for b in unused if b not in ok]
    if bad:
        errors.append("user-capable balls of the pinmap are not in the design, and what Supra does with an "
                      "unused pin is not known: " + " ".join(bad) +
                      "\n    Put them in the list as hold (mklist.py), or, after confirming in Supra's report "
                      "that each one is not user I/O or is a tri-stated input, name it with --unused-ok.")
    for b in unused:
        if b in ok:
            notes.append(f"unused in the design, accepted by --unused-ok: {b}")
    for b in sorted(ok - set(unused)):
        notes.append(f"--unused-ok {b}: it is in the design (or not in the pinmap); nothing to accept")
    in_pinmap = set(plain) | {b for b, _ in special}
    for b in sorted(design - in_pinmap):
        notes.append(f"{b} is in the design but not in the pinmap (Supra accepted it)")
    for b in sorted(design & set(DEDICATED)):
        errors.append(f"{b} ({DEDICATED[b]}) is a dedicated configuration ball and must not be a design port")
    notes.append(f"unused user-capable balls: {len(unused)} ({len(unused) - len(bad)} accepted)")

    lines = [f"checkpnr: {n}" for n in notes] + [f"checkpnr: ERROR {e}" for e in errors]
    text = "\n".join(lines) + "\n"
    if a.report:
        with open(a.report, "w", encoding="utf-8", newline="\n") as f:
            f.write(text)
    sys.stdout.write(text)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
