#!/usr/bin/env python3
"""Merge the ibuf + obuf pair that Supra's af_map.tcl emits for each inout bit
into one bidirectional I/O cell, so that `af` puts it on ONE pad.

usage: fixio.py in.vqm out.vqm [--expect N] [--prim alta_io]

Without this, Supra 2026.03 places the input half and the output half of every
inout on two different balls, and location assignments do not apply to them
(docs/research/ag16k-supra-build.md 4-1). The fix is a text rewrite of the
.vqm: for a pad that has both

    cycloneive_io_ibuf X (.i(PAD), .ibar(1'h0), .o(N));
    cycloneive_io_obuf Y (.i(D), .o(PAD), .oe(E));

the two cells become

    alta_io \\PAD~io (.padio(PAD), .datain(D), .oe(E), .combout(N));

--expect N fails the build if the number of merged bits is not N (32 for
NetGD: MCU_DATA[15:0] and DC_DATA[15:0]).

Not verified beyond reading Supra's post-route netlist: no equivalence check,
no gate-level simulation, nothing on hardware.
"""
import argparse
import re
import sys

ap = argparse.ArgumentParser()
ap.add_argument("src")
ap.add_argument("dst")
ap.add_argument("--expect", type=int)
ap.add_argument("--prim", default="alta_io")
a = ap.parse_args()

s = open(a.src).read()
pat = re.compile(r"  cycloneive_io_(ibuf|obuf) (\S+) \((.*?)\n  \);\n", re.S)
bufs = {}
for m in pat.finditer(s):
    pins = dict(re.findall(r"\.(\w+)\(([^)]*)\)", m.group(3)))
    pad = pins["i"] if m.group(1) == "ibuf" else pins["o"]
    if m.group(1) in bufs.setdefault(pad, {}):
        sys.exit("fixio.py: two %ss on pad %s" % (m.group(1), pad))
    bufs[pad][m.group(1)] = (m, pins)

n = 0
out = s
for pad, d in bufs.items():
    if len(d) == 2:
        (mi, pi), (mo, po) = d["ibuf"], d["obuf"]
        if "oe" not in po:
            sys.exit("fixio.py: pad %s has an input and an output buffer but no .oe" % pad)
        cell = "  %s \\%s~io  (\n    .padio(%s),\n    .datain(%s),\n    .oe(%s),\n    .combout(%s)\n  );\n" % (
            a.prim, pad, pad, po["i"], po["oe"], pi["o"])
        out = out.replace(mi.group(0), cell).replace(mo.group(0), "")
        n += 1
open(a.dst, "w", newline="\n").write(out)
print("fixio.py: %d inout bits merged into %s" % (n, a.prim))
if a.expect is not None and n != a.expect:
    sys.exit("fixio.py: expected %d inout bits, merged %d" % (a.expect, n))
