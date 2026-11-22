#!/usr/bin/env python3
"""Routed-netlist equivalence check of the main core (build.sh --gate-sim).

Runs one top-level testbench (gatesim_tb.v) twice under Icarus Verilog and
compares the two pin-level traces line by line:

  * gate: Supra's post-route netlist `<work>/OpenGDEMU_routed.v` (what the
    bitstream implements: after fixio.py and place-and-route), with Supra's
    own simulation models (alta_slice, alta_dio, alta_bram9k, alta_pllve ...),
    read in place from $SUPRA_HOME. Only the modules the netlist needs are
    written to a derived file under --work (not committed);
  * RTL: the very files build.sh handed to Supra, from the same work
    directory: OpenGDEMU.v (the wrapper), OpenGDEMUCore_ag16k.v (the
    generated Amaranth core) and Audio_PLL.v (the alta_pllve wrapper, same
    PLL model as the gate run).

The testbench's stimulus never depends on what it reads, so both runs see the
same input waveforms; a difference in the traces is a difference in
behaviour, and its first line says where it starts. The traces hold every
output and inout pad (sampled 4 ns after each clock edge, one line per
change), every value the testbench reads, and the PLL output's edge count per
1024 core cycles. The testbench also checks some values against what the
fpga/amaranth tests expect (`EXPECT FAIL` lines); those must not fail in the
RTL run either, or the testbench itself is wrong.

    python3 -I fpga/agm/gatesim.py --work fpga/agm/build --supra "$SUPRA_HOME" \\
        [--tb FILE] [--clk-to-q NS] [--timeout 1800] [--only rtl|gate] [--audio-alive]

Output in <work>/gatesim/: rtl.trace, gate.trace, gatesim.out (summary and
the first differences), and the compiled models / simulations.
"""
import argparse
import difflib
import importlib.util
import os
import re
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
DESIGN = "OpenGDEMU"
RTL_FILES = ("OpenGDEMU.v", "OpenGDEMUCore_ag16k.v", "Audio_PLL.v")


def _pinfind_gatesim():
    """fpga/pinfind/gatesim.py (model discovery and extraction), loaded under
    another name: both files are called gatesim.py."""
    path = os.path.join(HERE, "..", "pinfind", "gatesim.py")
    spec = importlib.util.spec_from_file_location("pinfind_gatesim", path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def run(cmd, timeout=None, out=None):
    print("gatesim: " + " ".join(cmd), flush=True)
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        raise SystemExit(f"gatesim: {cmd[0]} did not finish in {timeout} s")
    if out:
        with open(out, "w", encoding="utf-8", newline="\n") as f:
            f.write(r.stdout)
    return r


def compile_and_run(name, tb, srcs, models, gdir, timeout, plusargs=()):
    exe = os.path.join(gdir, name + ".vvp")
    # The testbench first: the RTL files carry no `timescale and take its 1 ps.
    # Language rules differ per side, each the one its source needs:
    #   RTL -g2005: Yosys's Verilog (the generated core) starts its `always @*`
    #     blocks through a `reg \$auto$verilog_backend... = 0` initialiser,
    #     which is a time-0 event only under Verilog-2005 rules. Under
    #     SystemVerilog rules a block whose inputs never change is never
    #     evaluated and its outputs stay X (seen: dev0's read FIFO r_en, X
    #     from power-up until the first host Data read, so the level froze);
    #   gate -g2012: Supra's routed netlist declares some flop outputs `reg`
    #     and drives them from cell ports, which only SystemVerilog allows.
    gen = "-g2005" if name == "rtl" else "-g2012"
    r = run(["iverilog", gen, "-o", exe, "-s", "tb", tb] + srcs + [models])
    errs = [l for l in (r.stdout + r.stderr).splitlines() if "warning" not in l.lower() and l.strip()]
    if r.returncode != 0:
        sys.stderr.write("\n".join(errs[-40:]) + "\n")
        raise SystemExit(f"gatesim: iverilog failed for the {name} run; the check was NOT done")
    trace = os.path.join(gdir, name + ".trace")
    r = run(["vvp", "-n", exe] + list(plusargs), timeout=timeout, out=trace)
    if r.returncode != 0:
        sys.stderr.write(r.stderr[-2000:])
        raise SystemExit(f"gatesim: vvp failed for the {name} run")
    # Trace lines only; anything else (the PLL model's "locked at" message,
    # which names the instance differently in the two netlists, $finish) is
    # returned apart and reported, not compared.
    lines, other = [], []
    for l in open(trace, encoding="utf-8").read().splitlines():
        (lines if TRACE_LINE.match(l) else other).append(l)
    return lines, other


def summarize(lines):
    kinds = {}
    for l in lines:
        kinds[l[:1]] = kinds.get(l[:1], 0) + 1
    end = [l for l in lines if l.startswith("END")]
    fails = [l for l in lines if l.startswith("EXPECT FAIL")]
    xs = sum(1 for l in lines if l.startswith(("P", "A")) and re.search(r"[xX]", l.split(" ", 2)[2]))
    return kinds, end, fails, xs


TRACE_LINE = re.compile(r"^(?:[PACRWDIMN] \d|EXPECT FAIL |END )")
KIND = {"P": "pads (48 MHz side)", "A": "CDDA pins", "C": "CDCLK edge count", "R": "MCU read",
        "W": "MCU write", "D": "IDE register read", "I": "IDE write", "M": "DMA read",
        "N": "DMA write", "E": "testbench", "X": "testbench"}


def compare(rtl, gate, report, max_show=40):
    """Line-by-line comparison; returns the number of differing lines."""
    sm = difflib.SequenceMatcher(None, rtl, gate, autojunk=False)
    ndiff = 0
    shown = 0
    first_by_kind = {}
    for tag, i1, i2, j1, j2 in sm.get_opcodes():
        if tag == "equal":
            continue
        ndiff += max(i2 - i1, j2 - j1)
        for l in rtl[i1:i2]:
            first_by_kind.setdefault(l[:1], ("rtl ", l))
        for l in gate[j1:j2]:
            first_by_kind.setdefault(l[:1], ("gate", l))
        if shown < max_show:
            report.append(f"@@ rtl lines {i1 + 1}-{i2}, gate lines {j1 + 1}-{j2} ({tag})")
            for l in rtl[i1:min(i2, i1 + 8)]:
                report.append("- " + l)
            for l in gate[j1:min(j2, j1 + 8)]:
                report.append("+ " + l)
            shown += 1
    if first_by_kind:
        report.insert(0, "first difference by kind (- rtl / + gate):")
        for k, (side, l) in sorted(first_by_kind.items()):
            report.insert(1, f"  {KIND.get(k, k):20s} {side}: {l}")
    return ndiff


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--work", required=True, help="build.sh work directory (holds OpenGDEMU_routed.v)")
    ap.add_argument("--supra", required=True, help="SUPRA_HOME (models are read from here, in place)")
    ap.add_argument("--tb", default=os.path.join(HERE, "gatesim_tb.v"))
    ap.add_argument("--models", nargs="*", help="model files (default: found under --supra)")
    ap.add_argument("--netlist", help="netlist for the gate run (default <work>/OpenGDEMU_routed.v)")
    ap.add_argument("--clk-to-q", type=float,
                    help="ns of clock-to-Q delay added to every flop of the models (test override)")
    ap.add_argument("--only", choices=("rtl", "gate"), help="run one side only (no comparison)")
    ap.add_argument("--audio-alive", action="store_true",
                    help="also simulate ~6.5 ms more and check the xtal heartbeat (slot 18); slow")
    ap.add_argument("--timeout", type=int, default=1800, help="seconds per simulation")
    a = ap.parse_args(argv)

    for t in ("iverilog", "vvp"):
        if not shutil.which(t):
            raise SystemExit(f"gatesim: `{t}` (Icarus Verilog) is needed for --gate-sim")
    work = os.path.abspath(a.work)
    net = os.path.abspath(a.netlist or os.path.join(work, f"{DESIGN}_routed.v"))
    if not os.path.isfile(net):
        raise SystemExit(f"gatesim: {net} not found (run build.sh first)")
    rtl_srcs = [os.path.join(work, f) for f in RTL_FILES]
    for f in rtl_srcs:
        if not os.path.isfile(f):
            raise SystemExit(f"gatesim: {f} not found (the RTL run uses build.sh's own inputs)")
    pf = _pinfind_gatesim()
    supra = os.path.abspath(a.supra)
    models = a.models or pf.find_models(supra)
    if not models:
        raise SystemExit(f"gatesim: no simulation models (alta_* modules) under {supra}")
    if not a.models:
        for m in models:
            if not os.path.abspath(m).startswith(supra + os.sep):
                raise SystemExit("gatesim: model outside SUPRA_HOME")

    gdir = os.path.join(work, "gatesim")
    os.makedirs(gdir, exist_ok=True)
    lib = os.path.join(gdir, "models.v")
    # The netlist instantiates every primitive the RTL's Audio_PLL.v needs (alta_pllve).
    used = pf.needed_models(models, net, lib, None, a.clk_to_q)
    print(f"gatesim: models from {len(models)} file(s), {len(used)} module(s) used: {' '.join(used)}")
    with open(os.path.join(work, "Audio_PLL.v"), encoding="utf-8") as f:
        for prim in set(re.findall(r"^\s*(alta_\w+)\s", f.read(), re.M)):
            if prim not in used:
                raise SystemExit(f"gatesim: Audio_PLL.v needs {prim}, which the netlist does not use")

    report = []
    res = {}
    for side in ("rtl", "gate"):
        if a.only and a.only != side:
            continue
        srcs = rtl_srcs if side == "rtl" else [net]
        lines, other = compile_and_run(side, os.path.abspath(a.tb), srcs, lib, gdir, a.timeout,
                                       ["+audio_alive"] if a.audio_alive else [])
        kinds, end, fails, xs = summarize(lines)
        for l in other:
            if "$finish" not in l:
                report.append(f"{side} (not compared): {l.strip()}")
        res[side] = lines
        report.append(f"{side}: {len(lines)} trace lines "
                      + " ".join(f"{k}={n}" for k, n in sorted(kinds.items()))
                      + f"; {end[0] if end else 'NO END LINE'}; pad lines with x: {xs}")
        for l in fails[:10]:
            report.append(f"{side}: {l}")

    ok = True
    verdict = []
    if "rtl" in res:
        _, end, fails, _ = summarize(res["rtl"])
        if not end:
            ok = False; verdict.append("GATESIM FAIL: the RTL run did not reach the end of the testbench")
        if fails:
            ok = False; verdict.append(f"GATESIM FAIL: {len(fails)} testbench check(s) fail on the RTL itself (testbench wrong?)")
    if "gate" in res and not [l for l in res["gate"] if l.startswith("END")]:
        ok = False; verdict.append("GATESIM FAIL: the gate run did not reach the end of the testbench")
    if len(res) == 2:
        diffrep = []
        n = compare(res["rtl"], res["gate"], diffrep)
        if n:
            ok = False
            verdict.append(f"GATESIM DIFF: {n} trace line(s) differ between the routed netlist and the RTL")
        elif ok:
            verdict.append(f"GATESIM PASS: routed netlist and RTL traces identical ({len(res['rtl'])} lines)")
        report += diffrep
    elif ok:
        verdict.append(f"GATESIM ONLY {a.only}: no comparison")
    out = "\n".join(verdict + report) + "\n"
    with open(os.path.join(gdir, "gatesim.out"), "w", encoding="utf-8", newline="\n") as f:
        f.write(out)
    print(out[:6000])
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
