"""Which ball goes on which port of the pin-finder's top level.

Pure Python (no Amaranth), shared by gen.py (which writes PinFind.asf from it)
and checkpnr.py (which build.sh runs after place-and-route to compare it with
what Supra actually did). Keeping the rule in one place is what lets build.sh
check "port -> ball" against the ball list itself, not only against the .asf
gen.py happened to write.

Ports:
    P[k]      drive ball k of the list (inout; in stage 0 an input)
    HOLD[h]   balls that are inputs and never driven: the list's hold balls,
              then (without clock probes) its clock balls
    PROBE[c]  the list's clock balls, counted (only with --probes)
    CLKIN     the clock ball of `--clock ball:PIN_xx`
"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "tools", "pinfind"))

from mklist import norm  # noqa: E402

# The ball the MCU can clock after configuration (DCLK, PA24).
DCLK_BALL = "PIN_G1"


def parse_clock(clock):
    """'osc' | 'ring' | 'ball:PIN_xx' -> (src, ball or None)."""
    if clock in ("osc", "ring"):
        return clock, None
    if clock.startswith("ball:"):
        return "ball", norm(clock[5:])
    raise ValueError("--clock osc | ring | ball:PIN_xx")


def plan(balls, clock="osc", probes=False, stage0=False):
    """Return {'src', 'drive', 'hold', 'probe', 'clkin'} for a ball list
    (`mklist.read_list`). Raises ValueError for a combination that cannot be
    built safely."""
    drive = list(balls["drive"])
    hold = list(balls["hold"])
    clock_balls = list(balls["clock"])
    if stage0:
        # No clock, no core, no outputs: every ball of the list is an input.
        return {"src": "none", "drive": drive, "hold": hold + clock_balls, "probe": [], "clkin": None}
    src, clk_ball = parse_clock(clock)
    if src == "ball":
        if clk_ball in drive:
            raise ValueError(f"clock ball {clk_ball} is a drive ball; regenerate the list with "
                             f"mklist.py --exclude {clk_ball} (indices would shift otherwise)")
        if clk_ball in clock_balls and probes:
            raise ValueError(f"{clk_ball} is a clock probe; leave out --probes or use another clock ball")
        if clk_ball not in hold and clk_ball not in clock_balls:
            raise ValueError(f"clock ball {clk_ball} is not in the ball list (hold or clock); "
                             f"every ball in the design must come from the list")
        hold = [b for b in hold if b != clk_ball]
        clock_balls = [b for b in clock_balls if b != clk_ball]
    if probes:
        probe = clock_balls
    else:
        probe = []
        hold = hold + clock_balls
    return {"src": src, "drive": drive, "hold": hold, "probe": probe, "clkin": clk_ball}


def ports(p):
    """[(port, ball)] in .asf order, port names spelt as in PinFind.asf."""
    out = [(f"P[{k}]", b) for k, b in enumerate(p["drive"])]
    out += [(f"HOLD[{k}]", b) for k, b in enumerate(p["hold"])]
    out += [(f"PROBE[{k}]", b) for k, b in enumerate(p["probe"])]
    if p["clkin"]:
        out.append(("CLKIN", p["clkin"]))
    return out
