#!/usr/bin/env python3
"""Generate the AG16K pin-finder design (Verilog, pin file, constraints) from a ball list.

Run from fpga/pinfind with the Amaranth venv:

    ../amaranth/amaranth_env/Scripts/python.exe gen.py --balls build/balls.txt [options]

`build/balls.txt` comes from `tools/pinfind/mklist.py <Supra pinmap>`; nothing
here contains Supra data. Output (default `build/`, git-ignored):

    PinFindCore.v    the Amaranth core (hw/pinfind.py); not in stage 0
    PinFind.v        top level: tristate pads, clock source, core; pf_clock inlined
    PinFind.asf      ball assignment for Supra (set_location_assignment -to PORT PIN_xx)
    PinFind.sdc      clock constraints (minimal)
    balls.txt        a copy of the ball list (build.sh checks the placement against it)
    params.txt       every parameter + the ball list's SHA-256 (read by build.sh,
                     copied next to the bitstream, read by the firmware's build.rs)

The defaults are the first-run settings of README_ja.md (R143): open drain,
listen-before-drive, 2 frames per ball, one loop, no clock probes, ticks of at
least ~20 us. Stages (README_ja.md section 0):

    --stage0               no outputs at all (every ball an input, no clock, no
                           core): checks CONF_DONE, current and that no MCU pin
                           changes across configuration
    (stage 1)              default options with a short list (mklist.py --max-drive N)
    (stage 2)              default options with the whole list
    --probes               (stage 3) also count the dedicated clock inputs

Clock sources (see src/pf_clock.v):

    --clock osc            AGM alta_osc on-chip oscillator (default; unverified)
    --clock ring           LUT ring oscillator (fallback; unverified)
    --clock ball:PIN_G1    a ball as the clock; G1 = DCLK, which the `pinfind`
                           firmware toggles after configuration (only in this
                           case: build.rs reads params.txt)

Ticks are 2^prescale clocks. Defaults per source: osc 10, ring 12, ball 6
(T >= 20 us for open-drain rise times if osc <= 51 MHz, ring <= 204 MHz, and
the firmware's loop <= 3.2 M iterations/s; README_ja.md section 1).
"""
import argparse
import hashlib
import os
import shutil
import sys

os.environ.setdefault("AMARANTH_USE_YOSYS", "builtin")

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(HERE, "..", "..", "tools", "pinfind"))

from mklist import read_list  # noqa: E402
import portmap  # noqa: E402

DEFAULT_PRESCALE = {"osc": 10, "ring": 12, "ball": 6}
MAX_LOOPS = 8
MAX_FRAMES = 8


def build_args(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--balls", required=True, help="ball list from tools/pinfind/mklist.py")
    ap.add_argument("--out", default=os.path.join(HERE, "build"))
    ap.add_argument("--stage0", action="store_true",
                    help="stage 0: no outputs, every ball an input (README_ja.md section 0)")
    ap.add_argument("--clock", default="osc", help="osc | ring | ball:PIN_xx")
    ap.add_argument("--drive", choices=["od", "pp"], default="od",
                    help="od: drive low only, idle released (default; needs a pull-up at the "
                         "receiver: the MCU's own, or an external one); pp: drive both levels")
    ap.add_argument("--prescale", type=int)
    ap.add_argument("--frames", type=int, default=2, help=f"frames per ball, 1..{MAX_FRAMES} (default 2)")
    ap.add_argument("--listen", type=int, default=16, help="listen ticks before driving (default 16)")
    ap.add_argument("--start", type=int, default=256, help="ticks to wait after configuration (default 256)")
    ap.add_argument("--max-loops", type=int, default=1,
                    help=f"stop with every output off after N passes, 1..{MAX_LOOPS} (default 1)")
    ap.add_argument("--no-skip-low", action="store_true",
                    help="drive a ball even if it read low while listening (only for a logic-analyser "
                         "run with nothing else powered on that side; refused with --drive pp)")
    pr = ap.add_mutually_exclusive_group()
    pr.add_argument("--probes", action="store_true",
                    help="count the dedicated clock inputs (stage 3); without it they are hold inputs")
    pr.add_argument("--no-probes", action="store_true", help="the default; kept for old command lines")
    ap.add_argument("--div-bits", type=int, default=10)
    ap.add_argument("--win-bits", type=int, default=6)
    ap.add_argument("--ring-stages", type=int, default=31)
    ap.add_argument("--clkctrl", choices=["auto", "on", "off"], default="auto",
                    help="explicit alta_gclksw on the core clock (auto = off). Supra 2026.03.b0 "
                         "rejects it on AG16K ('no corresponding resource', then af_run crashes) and "
                         "puts the clock on a global net by itself; cycloneive_clkctrl is unknown to its af_map")
    ap.add_argument("--io-attrs", action="store_true",
                    help="add Quartus-style I/O assignments to the .asf (minimum current strength, weak "
                         "pull-up on scanned balls, unused balls as tri-stated inputs). Whether Supra "
                         "accepts them is UNVERIFIED: check run.out")
    return ap.parse_args(argv)


def check_args(a):
    """Refuse combinations that remove protections (R143). Returns an error string or None."""
    if not 1 <= a.max_loops <= MAX_LOOPS:
        return f"--max-loops must be 1..{MAX_LOOPS}"
    if not 1 <= a.frames <= MAX_FRAMES:
        return f"--frames must be 1..{MAX_FRAMES}"
    if a.drive == "pp" and a.no_skip_low:
        return ("--drive pp together with --no-skip-low removes both protections against a fight "
                "(a released idle and listen-before-drive); use open drain with external pull-ups "
                "(README_ja.md section 5)")
    if a.prescale is not None and not 0 <= a.prescale <= 20:
        return "--prescale must be 0..20"
    if a.listen < 1 or a.start < 1:
        return "--listen and --start must be >= 1"
    if a.stage0 and (a.probes or a.drive != "od" or a.no_skip_low or a.io_attrs or a.clock != "osc"):
        return ("--stage0 takes no --probes / --drive / --no-skip-low / --io-attrs / --clock "
                "(it has no outputs and no clock)")
    return None


def keep_inputs(name, width):
    """Verilog that keeps an input bus alive through synthesis (it drives
    nothing). Whether af keeps the pads too is what build.sh checks."""
    if not width:
        return ""
    return f"    (* keep = 1 *) wire [{width - 1}:0] {name.lower()}_keep = {name};\n"


def generate(a):
    err = check_args(a)
    if err:
        sys.exit(f"gen.py: {err}")
    balls = read_list(a.balls)
    try:
        plan = portmap.plan(balls, a.clock, probes=a.probes, stage0=a.stage0)
    except ValueError as e:
        sys.exit(f"gen.py: {e}")
    src = plan["src"]
    drive, hold, probe, clk_ball = plan["drive"], plan["hold"], plan["probe"], plan["clkin"]
    if src == "ring" and a.ring_stages % 2 == 0:
        sys.exit("gen.py: --ring-stages must be odd")
    prescale = 0 if a.stage0 else (DEFAULT_PRESCALE[src] if a.prescale is None else a.prescale)
    clkctrl = a.clkctrl == "on"   # auto = off: see --clkctrl and src/pf_clock.v
    nd, nh, nc = len(drive), len(hold), len(probe)

    os.makedirs(a.out, exist_ok=True)

    def w(name, text):
        with open(os.path.join(a.out, name), "w", newline="\n", encoding="utf-8") as f:
            f.write(text)

    if os.path.abspath(a.balls) != os.path.abspath(os.path.join(a.out, "balls.txt")):
        shutil.copyfile(a.balls, os.path.join(a.out, "balls.txt"))

    ports = []
    if a.stage0:
        rtl = ""
        drive_cap = 0
        ports.append(f"    input  [{nd - 1}:0] P")
        if nh:
            ports.append(f"    input  [{nh - 1}:0] HOLD")
        top = f"""// Generated by fpga/pinfind/gen.py -- do not edit. Ball list sha256 {balls['sha256']}
// STAGE 0: no outputs. Every ball of the list is an input; there is no clock
// and no logic. The `keep` wires only stop synthesis from dropping the pads.
module PinFind (
{(','+chr(10)).join(ports)}
);
{keep_inputs('P', nd)}{keep_inputs('HOLD', nh)}endmodule
"""
        if os.path.exists(os.path.join(a.out, "PinFindCore.v")):
            os.remove(os.path.join(a.out, "PinFindCore.v"))
    else:
        from amaranth.back import verilog
        from hw.pinfind import PinFindCore
        core = PinFindCore(nd, nc, prescale=prescale, frames=a.frames,
                           listen=a.listen, start=a.start, max_loops=a.max_loops,
                           skip_if_low=not a.no_skip_low, div_bits=a.div_bits, win_bits=a.win_bits)
        drive_cap = core.drive_cap
        rtl = verilog.convert(core, name="PinFindCore", ports=core.ports(), emit_src=False)
        w("PinFindCore.v", rtl)

        defines = [f"`define PF_CLK_{ {'osc': 'OSC', 'ring': 'RING', 'ball': 'PIN'}[src] }"]
        if clkctrl:
            defines.append("`define PF_CLKCTRL")
        if a.drive == "pp":
            defines.append("`define PF_PUSH_PULL")
        ports.append(f"    inout  [{nd - 1}:0] P")
        if nh:
            ports.append(f"    input  [{nh - 1}:0] HOLD")
        if nc:
            ports.append(f"    input  [{nc - 1}:0] PROBE")
        if src == "ball":
            ports.append("    input  CLKIN")
        pf_clock = open(os.path.join(HERE, "src", "pf_clock.v"), encoding="utf-8").read()
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
{keep_inputs('HOLD', nh)}
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

    pm = portmap.ports(plan)
    asf = [f"# Generated by fpga/pinfind/gen.py. Ball list sha256 {balls['sha256']}",
           "# Supra order: set_location_assignment -to <port> <ball>"]
    asf += [f"set_location_assignment -to {{{q}}} {b}" if "[" in q else f"set_location_assignment -to {q} {b}"
            for q, b in pm]
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
        "format": 2,
        "balls_sha256": balls["sha256"], "stage0": int(a.stage0),
        "n_drive": nd, "n_hold": nh, "n_probe": nc,
        "clock": "none" if a.stage0 else a.clock, "clock_ball": clk_ball or "none",
        # The firmware toggles DCLK (PA24) only when G1 is the design's clock.
        "toggle_dclk": int(clk_ball == portmap.DCLK_BALL),
        "clkctrl": int(clkctrl and not a.stage0), "drive": "none" if a.stage0 else a.drive,
        "prescale": prescale, "frames": a.frames, "listen": a.listen, "start": a.start,
        "max_loops": a.max_loops, "skip_if_low": int(not a.no_skip_low),
        "drive_cap_ticks": drive_cap,
        "div_bits": a.div_bits, "win_bits": a.win_bits,
        "ring_stages": a.ring_stages, "io_attrs": int(a.io_attrs),
        "total_pins": len(pm),
        "core_sha256": hashlib.sha256(rtl.encode()).hexdigest() if rtl else "none",
    }
    w("params.txt", "".join(f"{k} {v}\n" for k, v in params.items()))
    what = "STAGE 0 (no outputs)" if a.stage0 else (
        f"clock {a.clock}, prescale {prescale}, drive {a.drive}, frames {a.frames}, loops {a.max_loops}")
    print(f"gen.py: {nd} drive, {nh} hold, {nc} probe, {what} -> {a.out}", file=sys.stderr)
    return params


if __name__ == "__main__":
    generate(build_args())
