#!/usr/bin/env python3
"""Optional gate-level check of the ROUTED pin-finder netlist (build.sh --gate-sim).

Simulates Supra's own post-route netlist (`<work>/PinFind_routed.v`) with
Supra's own simulation models, read in place from $SUPRA_HOME (only the modules
the netlist needs are extracted into a derived file under --work, which is not
committed), under Icarus Verilog, and asserts on every change of any pad (after
a 1 us power-up window in which the unclocked registers may still be X):

    * at most one drive pad is low at any time (stage 0: none ever);
    * no drive pad is ever X (an unknown level may be a low);
    * (not stage 0) at least one pad was driven low in the simulated time,
      so the check did not pass merely because nothing ran.

This is what catches an inverted output enable after fixio.py (R143 item 1:
every scanned ball low at once) if Supra's models describe the silicon.

Pads are modelled as wires with a pull-up (the MCU's pull-up); HOLD and PROBE
inputs are held high; CLKIN (a `ball:` clock) is a 1 MHz square wave.
alta_osc is simulated with its own model (OSC_DEL 20 ns = 25 MHz). A
zero-delay LUT ring does not oscillate in simulation, so for `--clock ring`
the derived copy of LUT models (alta_slice, cycloneive_lcell_comb) get a 0.4 ns delay (--lcell-delay):
a test clock override for gate-sim only (an X on a slice LUT input also gives 0
there, so the ring can start; that hides power-up X in a ring build).

The real prescale-10 / start-256 design needs ~1.2 s of simulated time per loop
(hours under Icarus). For gate-sim, generate a second design with a small
prescale and start (gen.py --prescale 1 --start 8): same logic, shorter ticks.
The pad-level property being checked does not depend on the tick length.

    python3 -I fpga/pinfind/gatesim.py --gen fpga/pinfind/build --work fpga/pinfind/build/supra \\
        --supra "$SUPRA_HOME" [--models FILE ...] [--ms 300] [--timeout 1800]
"""
import argparse
import glob
import os
import re
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

from checkpnr import params_of  # noqa: E402


def find_models(supra):
    """Files under $SUPRA_HOME that define alta_* or cycloneive_* modules."""
    out = []
    rx = re.compile(rb"^\s*module\s+(alta_|cycloneive_)", re.M)
    for f in glob.glob(os.path.join(supra, "**", "*.v"), recursive=True):
        try:
            if os.path.getsize(f) < 32 << 20 and rx.search(open(f, "rb").read()):
                out.append(f)
        except OSError:
            pass
    # the rodina simulation library first (etc/arch/rodinia/alta_sim.v is a copy of it)
    return sorted(out, key=lambda f: ("rodina" not in f, f))


def split_modules(text):
    """{name: source} of every `module ... endmodule` in preprocessed text."""
    mods = {}
    for m in re.finditer(r"^\s*module\s+(\S+?)\s*[#(;\s].*?^\s*endmodule\b", text, re.S | re.M):
        mods.setdefault(m.group(1), m.group(0))
    return mods


def needed_models(models, netlist, out_path, lcell_delay=None, clk2q=None):
    """Write only the modules the routed netlist needs (closure over instantiations).

    Supra's alta_sim.v also holds SystemVerilog-only models (the ADC) that Icarus
    cannot parse, and a second copy of itself under etc/; so each file is
    preprocessed (-E -DALTA_SIM, as Supra's own simulator would), its modules
    are split, the first definition of a name wins, and only the modules that
    are reachable from the netlist are kept. Written under --work, never into
    the Supra tree.
    """
    allmods = {}
    for f in models:
        r = subprocess.run(["iverilog", "-E", "-g2012", "-DALTA_SIM", "-o", "/dev/stdout", f],
                           capture_output=True, text=True)
        if r.returncode != 0:
            sys.stderr.write(f"gatesim: cannot preprocess {f}: {r.stderr[-500:]}\n")
            continue
        for n, src in split_modules(r.stdout).items():
            allmods.setdefault(n, src)
    names = set(allmods)
    text = open(netlist, encoding="utf-8", errors="replace").read()
    todo = [k for k in re.findall(r"\b(\w+)\b", text) if k in names]
    keep = []
    seen = set()
    while todo:
        n = todo.pop()
        if n in seen:
            continue
        seen.add(n)
        keep.append(n)
        todo += [k for k in re.findall(r"\b(\w+)\b", allmods[n]) if k in names and k not in seen]
    with open(out_path, "w", encoding="utf-8", newline="\n") as f:
        f.write("`timescale 1ns/10ps\n// modules of Supra's simulation library used by this netlist (derived; not committed)\n")
        for n in sorted(keep):
            src = allmods[n].strip()
            if clk2q:
                # TEST OVERRIDE: a clock-to-Q delay on every flop of the models, so that a flop clocked
                # through a few zero-delay assigns (a delta later) cannot see the new value of another.
                src = re.sub(r"Q <= (?!#)", f"Q <= #{clk2q} ", src)
            lut_line = {"cycloneive_lcell_comb": "assign combout = combout_rt & 1'b1;",
                        "alta_slice": "assign LutOut ="}.get(n)
            if lcell_delay and lut_line:
                # TEST OVERRIDE (ring clock only): a LUT delay so the ring oscillates.
                new = src.replace(lut_line, lut_line.replace("assign ", f"assign #{lcell_delay} ", 1), 1)
                if n == "alta_slice":
                    # a ring of inverters starting at X stays X: resolve an unknown LUT input to 0
                    new = new.replace("assign #%s LutOut =" % lcell_delay,
                                      "assign #%s LutOut = (^{A, B, D, pinC} === 1'bx) ? 1'b0 :" % lcell_delay, 1)
                if new == src:
                    raise SystemExit(f"gatesim: cannot add a delay to {n} (model changed?)")
                src = new
            f.write(src + "\n\n")
    return sorted(keep)


def top_ports(netlist, top="PinFind"):
    """{name: (direction, width or None for an escaped scalar)} of the top module."""
    text = open(netlist, encoding="utf-8", errors="replace").read()
    m = re.search(r"\bmodule\s+" + top + r"\b(.*?)\bendmodule\b", text, re.S)
    if not m:
        raise SystemExit(f"gatesim: no module {top} in {netlist}")
    body = m.group(1)
    ports = {}
    decl = re.compile(r"\b(input|output|inout)\b\s*(?:wire\s+)?(\[\s*(\d+)\s*:\s*(\d+)\s*\])?\s*([^;()]*?)\s*[;,)]")
    for d in decl.finditer(body):
        names = [n.strip() for n in d.group(5).split(",") if n.strip()]
        for n in names:
            width = None
            if d.group(2):
                width = abs(int(d.group(3)) - int(d.group(4))) + 1
            ports[n] = (d.group(1), width)
    if not ports:
        raise SystemExit(f"gatesim: could not read the ports of {top} in {netlist}")
    return ports


def bit_ref(ports, name, k):
    """How to connect bit k of bus `name`: ('bus', name) or ('esc', '\\\\name[k] ')."""
    if name in ports and ports[name][1]:
        return f".{name}"
    esc = f"\\{name}[{k}]"
    if esc in ports:
        return "." + esc + " "
    raise SystemExit(f"gatesim: port {name}[{k}] not found in the routed netlist")


SETTLE_NS = 1000


def testbench(p, ports, ms):
    nd, nh, nc = int(p["n_drive"]), int(p["n_hold"]), int(p["n_probe"])
    stage0 = p.get("stage0") == "1"
    ball_clock = p.get("clock", "").startswith("ball:")
    conns, decls = [], []
    decls.append(f"    wire [{nd - 1}:0] pad;")
    decls.append(f"    genvar g; generate for (g = 0; g < {nd}; g = g + 1) begin : pu pullup(pad[g]); end endgenerate")
    for name, width, sig in (("P", nd, "pad"), ("HOLD", nh, f"{{{nh}{{1'b1}}}}" if nh else ""),
                             ("PROBE", nc, f"{{{nc}{{1'b1}}}}" if nc else "")):
        if not width:
            continue
        if name in ports and ports[name][1]:
            conns.append(f"        .{name}({sig})")
        else:
            for k in range(width):
                ref = bit_ref(ports, name, k)
                s = f"pad[{k}]" if name == "P" else "1'b1"
                conns.append(f"        {ref}({s})")
    if ball_clock:
        decls.append("    reg clkin = 1'b0; always #500 clkin = ~clkin;   // 1 MHz (1 ns units)")
        conns.append("        .CLKIN(clkin)")
    return f"""`timescale 1ns/1ps
// Generated by fpga/pinfind/gatesim.py for one build; not committed.
module tb;
{chr(10).join(decls)}
    PinFind dut (
{(','+chr(10)).join(conns)}
    );
    integer k, lows, xs, max_lows, seen_low, n_edges;
    reg settled;
    reg [{nd - 1}:0] ever = {nd}'b0;   // pads that were seen low at least once
    integer n_ever;
    realtime t_last;
    // The registers of the netlist start unknown; the pads may be X for the
    // first {SETTLE_NS} ns (power-up, before the first clock edges). Every
    // check applies after that.
    initial begin max_lows = 0; seen_low = 0; n_edges = 0; settled = 0; #{SETTLE_NS} settled = 1; end
    always @(pad or settled) if (settled) begin
        n_edges = n_edges + 1; t_last = $realtime;
        lows = 0; xs = 0;
        for (k = 0; k < {nd}; k = k + 1) begin
            if (pad[k] === 1'b0) lows = lows + 1;
            if (pad[k] === 1'bx || pad[k] === 1'bz) xs = xs + 1;
        end
        for (k = 0; k < {nd}; k = k + 1) if (pad[k] === 1'b0) ever[k] = 1'b1;
        if (lows > 0) seen_low = 1;
        if (lows > max_lows) max_lows = lows;
        if (xs != 0) begin
            $display("GATESIM FAIL: %0d pad(s) X/Z at %0t ns", xs, $time); $finish;
        end
        if (lows > {0 if stage0 else 1}) begin
            $display("GATESIM FAIL: %0d pads low at once at %0t ns", lows, $time); $finish;
        end
    end
    initial begin
        #({ms} * 1000000);
        if ({0 if stage0 else 1} && !seen_low) begin
            $display("GATESIM INCONCLUSIVE: no pad was driven in {ms} ms (did the clock run? registers X?)");
        end else begin
            n_ever = 0;
            for (k = 0; k < {nd}; k = k + 1) if (ever[k]) n_ever = n_ever + 1;
            $display("GATESIM PASS: max %0d pad(s) low at once over {ms} ms (%0d pad changes after {SETTLE_NS} ns; %0d of {nd} pads were driven low; last change at %0.1f us)", max_lows, n_edges, n_ever, t_last / 1000.0);
        end
        $finish;
    end
endmodule
"""


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--gen", required=True)
    ap.add_argument("--work", required=True)
    ap.add_argument("--supra", required=True)
    ap.add_argument("--models", nargs="*", help="model files (default: found under --supra)")
    ap.add_argument("--ms", type=int, default=300, help="simulated time in ms (default 300)")
    ap.add_argument("--timeout", type=int, default=1800)
    ap.add_argument("--clk-to-q", type=float, help="ns of clock-to-Q delay added to every flop model (test override)")
    ap.add_argument("--lcell-delay", type=float,
                    help="ns added to every LUT model (default 0.4 for --clock ring, else none)")
    a = ap.parse_args(argv)

    p = params_of(a.gen)
    lcell_delay = a.lcell_delay if a.lcell_delay is not None else (0.4 if p.get("clock") == "ring" else None)
    for t in ("iverilog", "vvp"):
        if not shutil.which(t):
            raise SystemExit(f"gatesim: `{t}` (Icarus Verilog) is needed for --gate-sim")
    net = os.path.join(a.work, "PinFind_routed.v")
    if not os.path.isfile(net):
        raise SystemExit(f"gatesim: {net} not found")
    models = a.models or find_models(a.supra)
    if not models:
        raise SystemExit(f"gatesim: no simulation models (alta_* / cycloneive_* modules) under {a.supra}")
    for m in models:
        if not os.path.abspath(m).startswith(os.path.abspath(a.supra) + os.sep) and not a.models:
            raise SystemExit("gatesim: model outside SUPRA_HOME")
    ports = top_ports(net)
    tb = os.path.join(a.work, "gatesim_tb.v")
    with open(tb, "w", encoding="utf-8", newline="\n") as f:
        f.write(testbench(p, ports, a.ms))
    exe = os.path.join(a.work, "gatesim.vvp")
    lib = os.path.join(a.work, "gatesim_models.v")
    used = needed_models(models, net, lib, lcell_delay, a.clk_to_q)
    print(f"gatesim: models from {len(models)} file(s), {len(used)} module(s) used: {' '.join(used)}")
    cmd = ["iverilog", "-g2012", "-o", exe, "-s", "tb", tb, net, lib]
    print("gatesim: " + " ".join(cmd))
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        sys.stderr.write(r.stdout[-4000:] + r.stderr[-4000:])
        raise SystemExit("gatesim: iverilog failed (models or netlist not accepted); gate-level check NOT done")
    try:
        r = subprocess.run(["vvp", "-n", exe], capture_output=True, text=True, timeout=a.timeout)
    except subprocess.TimeoutExpired:
        raise SystemExit(f"gatesim: simulation did not finish in {a.timeout} s; use a smaller --gate-sim-ms")
    out = r.stdout + r.stderr
    with open(os.path.join(a.work, "gatesim.out"), "w", encoding="utf-8", newline="\n") as f:
        f.write(out)
    result = [l for l in out.splitlines() if l.startswith("GATESIM")]
    print("\n".join(result) or out[-2000:])
    if not result or not result[-1].startswith("GATESIM PASS"):
        raise SystemExit("gatesim: gate-level check did not pass (gatesim.out)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
