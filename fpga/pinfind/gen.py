#!/usr/bin/env python3
"""Generate the AG16K pin-finder design (Verilog, pin file, constraints) from a ball list.

Run from fpga/pinfind with the Amaranth venv:

    ../amaranth/amaranth_env/Scripts/python.exe gen.py --balls build/balls.txt [options]

`build/balls.txt` comes from `tools/pinfind/mklist.py <Supra pinmap>`; nothing
here contains Supra data. Output (default `build/`, git-ignored):

    PinFindCore.v    the Amaranth core (hw/pinfind.py)
    PinFind.v        top level: tristate pads, clock source, core; pf_clock inlined
    PinFind.asf      ball assignment for Supra (set_location_assignment -to PORT PIN_xx)
    PinFind.sdc      clock constraints (minimal)
    params.txt       every parameter + the ball list's SHA-256 (read by build.sh
                     and worth keeping next to the RTT log)

Clock sources (see src/pf_clock.v):

    --clock osc            AGM alta_osc on-chip oscillator (default; unverified)
    --clock ring           LUT ring oscillator (fallback; unverified)
    --clock ball:PIN_G1    a ball as the clock; G1 = DCLK, which the `pinfind`
                           firmware toggles after configuration

Ticks are 2^prescale clocks. Defaults per source: osc 8, ring 11, ball 5.
The receiver does not need to know the clock rate; the prescale only keeps a
tick within what it can time (MCU: >= ~2 us; see README_ja.md).
"""
import argparse
import hashlib
import os
import sys

os.environ.setdefault("AMARANTH_USE_YOSYS", "builtin")

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(HERE, "..", "..", "tools", "pinfind"))

from amaranth.back import verilog  # noqa: E402

from hw.pinfind import PinFindCore  # noqa: E402
from mklist import norm, read_list  # noqa: E402

DEFAULT_PRESCALE = {"osc": 8, "ring": 11, "ball": 5}


def build_args(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--balls", required=True, help="ball list from tools/pinfind/mklist.py")
    ap.add_argument("--out", default=os.path.join(HERE, "build"))
    ap.add_argument("--clock", default="osc", help="osc | ring | ball:PIN_xx")
    ap.add_argument("--drive", choices=["od", "pp"], default="od",
                    help="od: drive low only, idle released (default; needs a pull-up at the "
                         "receiver: the MCU's own, or --io-attrs); pp: drive both levels")
    ap.add_argument("--prescale", type=int)
    ap.add_argument("--frames", type=int, default=3, help="frames per ball (default 3)")
    ap.add_argument("--listen", type=int, default=16, help="listen ticks before driving (default 16)")
    ap.add_argument("--start", type=int, default=256, help="ticks to wait after configuration (default 256)")
    ap.add_argument("--max-loops", type=int, default=8,
                    help="stop with every output off after N passes (default 8; 0 = never, not advised)")
    ap.add_argument("--no-skip-low", action="store_true",
                    help="drive a ball even if it read low while listening (only for a logic-analyser "
                         "run with nothing else powered on that side)")
    ap.add_argument("--no-probes", action="store_true", help="leave the clock balls out of the design")
    ap.add_argument("--div-bits", type=int, default=10)
    ap.add_argument("--win-bits", type=int, default=6)
    ap.add_argument("--ring-stages", type=int, default=31)
    ap.add_argument("--clkctrl", choices=["auto", "on", "off"], default="auto",
                    help="explicit cycloneive_clkctrl on the core clock (auto: on for osc/ring)")
    ap.add_argument("--io-attrs", action="store_true",
                    help="add Quartus-style I/O assignments to the .asf (minimum current strength, weak "
                         "pull-up on scanned balls, unused balls as tri-stated inputs). Whether Supra "
                         "accepts them is UNVERIFIED: check run.out")
    return ap.parse_args(argv)


def generate(a):
    balls = read_list(a.balls)
    drive, hold, clock = balls["drive"], balls["hold"], balls["clock"]
    if a.no_probes:
        clock = []
    if a.clock == "osc" or a.clock == "ring":
        src, clk_ball = a.clock, None
    elif a.clock.startswith("ball:"):
        src, clk_ball = "ball", norm(a.clock[5:])
        if clk_ball in drive:
            sys.exit(f"gen.py: clock ball {clk_ball} is a drive ball; regenerate the list with "
                     f"mklist.py --exclude {clk_ball} (indices would shift otherwise)")
        if clk_ball in clock:
            sys.exit(f"gen.py: {clk_ball} is a clock probe; use --no-probes or another clock ball")
        hold = [b for b in hold if b != clk_ball]
    else:
        sys.exit("gen.py: --clock osc | ring | ball:PIN_xx")
    if src == "ring" and a.ring_stages % 2 == 0:
        sys.exit("gen.py: --ring-stages must be odd")
    prescale = DEFAULT_PRESCALE[src] if a.prescale is None else a.prescale
    clkctrl = (src != "ball") if a.clkctrl == "auto" else (a.clkctrl == "on")

    core = PinFindCore(len(drive), len(clock), prescale=prescale, frames=a.frames,
                       listen=a.listen, start=a.start, max_loops=a.max_loops,
                       skip_if_low=not a.no_skip_low, div_bits=a.div_bits, win_bits=a.win_bits)
    rtl = verilog.convert(core, name="PinFindCore", ports=core.ports(), emit_src=False)

    os.makedirs(a.out, exist_ok=True)
    w = lambda name, text: open(os.path.join(a.out, name), "w", newline="\n").write(text)
    w("PinFindCore.v", rtl)

    nd, nh, nc = len(drive), len(hold), len(clock)
    defines = [f"`define PF_CLK_{ {'osc': 'OSC', 'ring': 'RING', 'ball': 'PIN'}[src] }"]
    if clkctrl:
        defines.append("`define PF_CLKCTRL")
    if a.drive == "pp":
        defines.append("`define PF_PUSH_PULL")
    ports = [f"    inout  [{nd - 1}:0] P"]
    if nh:
        ports.append(f"    input  [{nh - 1}:0] HOLD")
    if nc:
        ports.append(f"    input  [{nc - 1}:0] PROBE")
    if src == "ball":
        ports.append("    input  CLKIN")
    pf_clock = open(os.path.join(HERE, "src", "pf_clock.v")).read()
    top = f"""// Generated by fpga/pinfind/gen.py -- do not edit. Ball list sha256 {balls['sha256']}
{chr(10).join(defines)}

// P[k]   : drive ball k (index k in the ball list), scanned one at a time.
// HOLD[h]: balls kept as never-driven inputs (in the design only so that they
//          are inputs for sure). PROBE[c]: dedicated clock inputs, counted.
module PinFind (
{(','+chr(10)).join(ports)}
);
    wire clk;
    pf_clock #(.STAGES({a.ring_stages})) u_clk (.pin({'CLKIN' if src == 'ball' else "1'b0"}), .clk(clk));

    wire [{nd - 1}:0] oe;
    wire o;
    PinFindCore core (
        .clk(clk), .rst(1'b0),
        .pad_i(P), .pad_oe(oe), .pad_o(o){', .probe(PROBE)' if nc else ''}
    );

    genvar k;
    generate
        for (k = 0; k < {nd}; k = k + 1) begin : pad
`ifdef PF_PUSH_PULL
            assign P[k] = oe[k] ? o : 1'bz;
`else
            // Open drain: only ever pull low; high is the receiver's pull-up.
            assign P[k] = (oe[k] & ~o) ? 1'b0 : 1'bz;
`endif
        end
    endgenerate
endmodule

{pf_clock}"""
    w("PinFind.v", top)

    asf = [f"# Generated by fpga/pinfind/gen.py. Ball list sha256 {balls['sha256']}",
           "# Supra order: set_location_assignment -to <port> <ball>"]
    asf += [f"set_location_assignment -to {{P[{k}]}} {b}" for k, b in enumerate(drive)]
    asf += [f"set_location_assignment -to {{HOLD[{k}]}} {b}" for k, b in enumerate(hold)]
    asf += [f"set_location_assignment -to {{PROBE[{k}]}} {b}" for k, b in enumerate(clock)]
    if src == "ball":
        asf.append(f"set_location_assignment -to CLKIN {clk_ball}")
    asf.append("set_global_assignment -name ON_CHIP_BITSTREAM_DECOMPRESSION OFF")
    if a.io_attrs:
        asf.append("# --io-attrs: Quartus syntax, UNVERIFIED in Supra")
        asf.append('set_global_assignment -name RESERVE_ALL_UNUSED_PINS "AS INPUT TRI-STATED"')
        for k in range(nd):
            asf.append(f'set_instance_assignment -name CURRENT_STRENGTH_NEW "MINIMUM CURRENT" -to {{P[{k}]}}')
            asf.append(f"set_instance_assignment -name WEAK_PULL_UP_RESISTOR ON -to {{P[{k}]}}")
    w("PinFind.asf", "\n".join(asf) + "\n")

    sdc = ["# Generated by fpga/pinfind/gen.py. Minimal: the design is slow and has no I/O timing."]
    if src == "ball":
        sdc.append("create_clock -name pf_clkin -period 100.000 [get_ports {CLKIN}]")
    for k in range(nc):
        sdc.append(f"create_clock -name pf_probe{k} -period 20.000 [get_ports {{PROBE[{k}]}}]")
    w("PinFind.sdc", "\n".join(sdc) + "\n")

    params = {
        "balls_sha256": balls["sha256"], "n_drive": nd, "n_hold": nh, "n_probe": nc,
        "clock": a.clock, "clkctrl": int(clkctrl), "drive": a.drive, "prescale": prescale,
        "frames": a.frames, "listen": a.listen, "start": a.start, "max_loops": a.max_loops,
        "skip_if_low": int(not a.no_skip_low), "div_bits": a.div_bits, "win_bits": a.win_bits,
        "ring_stages": a.ring_stages, "io_attrs": int(a.io_attrs),
        "total_pins": nd + nh + nc + (1 if src == "ball" else 0),
        "core_sha256": hashlib.sha256(rtl.encode()).hexdigest(),
    }
    w("params.txt", "".join(f"{k} {v}\n" for k, v in params.items()))
    print(f"gen.py: {nd} drive, {nh} hold, {nc} probe, clock {a.clock}, prescale {prescale}, "
          f"drive {a.drive} -> {a.out}", file=sys.stderr)
    return params


if __name__ == "__main__":
    generate(build_args())
