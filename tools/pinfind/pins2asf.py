#!/usr/bin/env python3
"""Turn pin-finder results into a pin file (.asf) for the real AG16K core (fpga/agm).

usage:
  pins2asf.py --balls fpga/pinfind/build/balls.txt --mcu rtt.log
              [--la la.txt --la-map la_map.csv] [--mcu-map mcu_map.csv]
              [--set CLK_48_MHz=PIN_J4 ...] [--clock-auto]
              [--ports fpga/agm/pins/PROVISIONAL.asf | fpga/agm/src/OpenGDEMU.v]
              [-o fpga/agm/pins/gdemu60.asf] [--allow-partial]

Inputs
  --balls     the SAME ball list the pin-finder bitstream was generated from
              (index -> ball name). Its SHA-256 is written into the output.
  --mcu       the RTT log of the `pinfind` firmware. Lines used (anywhere in a
              line, so probe-rs / defmt prefixes do not matter):
                PF hit pin=PB9 idx=37 n=12       ball 37 was seen on MCU pin PB9
                PF clk c=3 ... hz=48000000       clock probe 3 runs at 48 MHz
  --mcu-map   CSV `mcu_pin,port` replacing entries of the built-in table (below).
  --la        output of `cargo run --example la_csv` (common/pinfind) for a
              logic-analyser capture on the GD-ROM connector:
                PF ch=D3 idx=12 n=4
  --la-map    CSV `channel,port`: which core port each analyser channel was
              clipped to (e.g. `D3,DC_DATA[3]`), from the connector pinout.
  --set       manual assignment PORT=BALL (method 1 / continuity results, clocks).
  --clock-auto  assign CLK_48_MHz / CLK_11_2896_MHz to the clock probe whose
              measured frequency is within 3 % of 48 MHz / 11.2896 MHz.
  --ports     the core's port list: an .asf with set_location_assignment lines
              (default: fpga/agm/pins/PROVISIONAL.asf, which is also what
              fpga/agm/build.sh checks a real pin file against) or the top-level
              Verilog.

A source pin counts only if one index has >= --min-share percent (default 80)
of its frames. Two balls for one port, or two ports on one ball, is an error.

Output: one `set_location_assignment -to <port> <ball>` per port, spelled the
way fpga/agm/build.sh expects. If any port is unassigned the file gets the
NETGD-PROVISIONAL-PINS marker (so build.sh never names it as a real pinout)
and the exit status is 2, unless --allow-partial (status 0, still marked).

Built-in MCU pin -> port table. The SMC pins are fixed by the SAM3U's
peripheral multiplexing (mcu/embassy-atsam3/src/smc.rs), so they hold on any
board that uses NCS3 and a 16-bit bus; IRQn and RSTn are the v5.5 board's
choice (docs/PCB.md) and only an assumption for 6.0.
"""
import argparse
import csv
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from mklist import norm, read_list  # noqa: E402

MARKER = "NETGD-PROVISIONAL-PINS"
REPO = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))

MCU_MAP = {}
for i, pin in enumerate(range(9, 17)):
    MCU_MAP[f"PB{pin}"] = f"MCU_DATA[{i}]"
for i, pin in enumerate(range(25, 32)):
    MCU_MAP[f"PB{pin}"] = f"MCU_DATA[{8 + i}]"
MCU_MAP["PB6"] = "MCU_DATA[15]"
MCU_MAP["PB7"] = "MCU_NBS[0]"        # A0/NBS0
MCU_MAP["PB8"] = "MCU_ADDR[1]"       # A1
for i in range(7):
    MCU_MAP[f"PC{i}"] = f"MCU_ADDR[{2 + i}]"
MCU_MAP["PB19"] = "MCU_NRE"
MCU_MAP["PB23"] = "MCU_NWE"
MCU_MAP["PC15"] = "MCU_NBS[1]"
MCU_MAP["PC17"] = "MCU_NCS"          # NCS3
MCU_MAP["PA22"] = "MCU_IRQn"         # v5.5 board choice
MCU_MAP["PA30"] = "MCU_RSTn"         # v5.5 board choice (docs/PCB.md "Logic Reset")

HIT = re.compile(r"PF hit pin=(P[ABC]\d{1,2}) idx=(\d+) n=(\d+)")
CLK = re.compile(r"PF clk c=(\d+)\b.*?\bhz=(\d+)")
LA = re.compile(r"PF ch=(\S+) idx=(\d+) n=(\d+)")


def bracify(port):
    # Spelling used by fpga/agm/pins/PROVISIONAL.asf and checked by build.sh:
    # the clocks bare, everything else in braces.
    return port if port.startswith("CLK_") else "{%s}" % port


def ports_from_file(path):
    with open(path, encoding="utf-8") as f:
        text = f.read()
    if path.endswith(".asf"):
        out = []
        for m in re.finditer(r"^set_location_assignment -to (\S+) \S+", text, re.M):
            out.append(m.group(1).strip("{}"))
        return out
    m = re.search(r"\bmodule\s+\w+\s*\((.*?)\);", text, re.S)
    if not m:
        raise ValueError(f"{path}: no module header")
    body = re.sub(r"//[^\n]*", "", m.group(1))
    out = []
    for d in re.finditer(r"\b(?:input|output|inout)\s*(?:wire\s*|reg\s*)?(\[\s*(\d+)\s*:\s*(\d+)\s*\])?\s*(\w+)", body):
        name = d.group(4)
        if d.group(1):
            hi, lo = int(d.group(2)), int(d.group(3))
            for b in range(min(hi, lo), max(hi, lo) + 1):
                out.append(f"{name}[{b}]")
        else:
            out.append(name)
    return out


def read_csv_map(path):
    out = {}
    with open(path, newline="", encoding="utf-8") as f:
        for row in csv.reader(f):
            if not row or row[0].strip().startswith("#"):
                continue
            if len(row) < 2:
                raise ValueError(f"{path}: expected 'key,port' rows")
            out[row[0].strip()] = row[1].strip()
    return out


def tally(lines, pattern):
    """{source: {idx: count}} from text lines. The firmware prints cumulative
    counts every few seconds, so the largest value of a (source, idx) pair is
    its count; repeated summaries are not added up."""
    t = {}
    for line in lines:
        for m in pattern.finditer(line):
            src, idx, n = m.group(1), int(m.group(2)), int(m.group(3))
            d = t.setdefault(src, {})
            d[idx] = max(d.get(idx, 0), n)
    return t


def dominant(counts, min_share):
    total = sum(counts.values())
    idx, n = max(counts.items(), key=lambda kv: kv[1])
    return idx if total and n * 100 >= min_share * total else None


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--balls", required=True)
    ap.add_argument("--mcu", action="append", default=[])
    ap.add_argument("--mcu-map")
    ap.add_argument("--la", action="append", default=[])
    ap.add_argument("--la-map")
    ap.add_argument("--set", action="append", default=[], metavar="PORT=BALL")
    ap.add_argument("--clock-auto", action="store_true")
    ap.add_argument("--ports", default=os.path.join(REPO, "fpga", "agm", "pins", "PROVISIONAL.asf"))
    ap.add_argument("--min-share", type=int, default=80)
    ap.add_argument("-o", "--output")
    ap.add_argument("--allow-partial", action="store_true")
    a = ap.parse_args(argv)

    balls = read_list(a.balls)
    drive, clock = balls["drive"], balls["clock"]
    ports = ports_from_file(a.ports)
    portset = set(ports)
    assign = {}          # port -> (ball, why)
    notes = []
    errors = []

    def put(port, ball, why):
        if port not in portset:
            errors.append(f"{why}: {port} is not a port of the core")
            return
        if port in assign and assign[port][0] != ball:
            errors.append(f"{port}: {assign[port][0]} ({assign[port][1]}) and {ball} ({why})")
            return
        assign[port] = (ball, why)

    mcu_map = dict(MCU_MAP)
    if a.mcu_map:
        mcu_map.update(read_csv_map(a.mcu_map))

    mcu_lines = [line for p in a.mcu for line in open(p, encoding="utf-8", errors="replace")]
    for pin, counts in sorted(tally(mcu_lines, HIT).items()):
        idx = dominant(counts, a.min_share)
        if idx is None:
            errors.append(f"MCU {pin}: no dominant index in {counts}")
            continue
        if idx >= len(drive):
            errors.append(f"MCU {pin}: index {idx} is beyond the ball list ({len(drive)} drive balls): wrong list?")
            continue
        port = mcu_map.get(pin)
        if port is None:
            notes.append(f"MCU {pin} <-> {drive[idx]} (no core port for this MCU pin)")
            continue
        put(port, drive[idx], f"MCU {pin}")

    if a.clock_auto:
        hz = {}
        for line in mcu_lines:
            for m in CLK.finditer(line):
                hz.setdefault(int(m.group(1)), []).append(int(m.group(2)))
        for port, target in (("CLK_48_MHz", 48_000_000), ("CLK_11_2896_MHz", 11_289_600)):
            hits = [c for c, v in hz.items() if v and abs(sorted(v)[len(v) // 2] - target) <= target * 0.03]
            if len(hits) == 1 and hits[0] < len(clock):
                put(port, clock[hits[0]], f"clock probe {hits[0]}")
            elif len(hits) > 1:
                errors.append(f"{port}: several clock probes near {target} Hz: {hits}")
            else:
                notes.append(f"{port}: no clock probe within 3 % of {target} Hz")

    if a.la:
        if not a.la_map:
            sys.exit("pins2asf.py: --la needs --la-map (channel -> port)")
        la_map = read_csv_map(a.la_map)
        la_lines = [line for p in a.la for line in open(p, encoding="utf-8", errors="replace")]
        for ch, counts in sorted(tally(la_lines, LA).items()):
            idx = dominant(counts, a.min_share)
            if idx is None:
                errors.append(f"LA {ch}: no dominant index in {counts}")
                continue
            if idx >= len(drive):
                errors.append(f"LA {ch}: index {idx} beyond the ball list")
                continue
            port = la_map.get(ch)
            if port is None:
                notes.append(f"LA {ch} <-> {drive[idx]} (channel not in --la-map)")
                continue
            put(port, drive[idx], f"LA {ch}")

    for s in a.set:
        port, _, ball = s.partition("=")
        put(port.strip(), norm(ball), "--set")

    by_ball = {}
    for port, (ball, why) in assign.items():
        by_ball.setdefault(ball, []).append(f"{port} ({why})")
    for ball, ps in by_ball.items():
        if len(ps) > 1:
            errors.append(f"{ball} is assigned to {len(ps)} ports: {', '.join(ps)}")

    if errors:
        for e in errors:
            print(f"pins2asf.py: error: {e}", file=sys.stderr)
        sys.exit(1)

    missing = [p for p in ports if p not in assign]
    out = []
    if missing:
        out.append(f"# {MARKER}")
        out.append(f"# INCOMPLETE: {len(missing)} of {len(ports)} ports unassigned; build.sh treats this file as provisional.")
    out.append("# Generated by tools/pinfind/pins2asf.py from pin-finder results. Not verified on hardware.")
    out.append(f"# ball list sha256 {balls['sha256']}")
    for n in notes:
        out.append(f"# note: {n}")
    for p in missing:
        out.append(f"# unassigned: {p}")
    for p in ports:
        if p in assign:
            ball, why = assign[p]
            out.append(f"set_location_assignment -to {bracify(p)} {ball}")
    text = "\n".join(out) + "\n"
    if a.output:
        with open(a.output, "w", newline="\n") as f:
            f.write(text)
    else:
        sys.stdout.write(text)
    print(f"pins2asf.py: {len(assign)} of {len(ports)} ports assigned"
          + (f", {len(missing)} missing" if missing else ""), file=sys.stderr)
    for n in notes:
        print(f"pins2asf.py: note: {n}", file=sys.stderr)
    if missing and not a.allow_partial:
        sys.exit(2)


if __name__ == "__main__":
    main()
