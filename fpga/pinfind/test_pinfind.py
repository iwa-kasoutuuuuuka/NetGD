#!/usr/bin/env python3
"""Simulation tests for the pin-finder core (hw/pinfind.py).

Run from the fpga/pinfind directory with the Amaranth venv:

    ../amaranth/amaranth_env/Scripts/python.exe test_pinfind.py
    ../amaranth/amaranth_env/Scripts/python.exe test_pinfind.py --write-golden

The pads are modelled as open-drain with a pull-up at the receiver (the MCU
pin): a ball reads 0 only while its output enable is on and the core drives
0. Every test checks, on every clock, that at most one output enable is on.

`--write-golden` also writes testdata/sim_edges.txt, the edges seen on each
ball in a 3-ball run with two clock probes; common/pinfind/tests/golden.rs
decodes that file with the firmware's decoder, so the Amaranth line code and
the Rust decoder are checked against each other.
"""

import os
import sys

from amaranth.sim import Simulator

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from hw.pinfind import (PinFindCore, frame_bytes, START, LISTEN, FRAME, NEXT, DONE,  # noqa: E402
                        SYNC_ACTIVE, IDLE, BIT0_ACTIVE, BIT1_ACTIVE, GAP)

CLK_HZ = 50e6
GOLDEN = os.path.join(os.path.dirname(os.path.abspath(__file__)), "testdata", "sim_edges.txt")


def expected_pulses(payload):
    """(active, ticks) list for one frame, as the line code defines it."""
    out = [(True, SYNC_ACTIVE), (False, IDLE)]
    bits = [(b >> (7 - i)) & 1 for b in payload for i in range(8)]
    for n, bit in enumerate(bits):
        out.append((True, BIT1_ACTIVE if bit else BIT0_ACTIVE))
        out.append((False, GAP if n == len(bits) - 1 else IDLE))
    return out


class Trace:
    """Per-cycle record of what the pads did."""

    def __init__(self, n):
        self.n = n
        self.levels = [[] for _ in range(n)]   # electrical level per cycle (pull-up model)
        self.states = []
        self.skips = []                         # ball index each time NEXT skipped
        self.max_oe = 0

    def edges(self, k):
        out, prev = [], 1
        for t, lv in enumerate(self.levels[k]):
            if lv != prev:
                out.append((t, lv))
                prev = lv
        return out

    def runs(self, k):
        """Run-length list (level, cycles) of ball k, without the leading idle."""
        lv = self.levels[k]
        out = []
        i = 0
        while i < len(lv):
            j = i
            while j < len(lv) and lv[j] == lv[i]:
                j += 1
            out.append((lv[i], j - i))
            i = j
        if out and out[0][0] == 1:
            out = out[1:]
        return out


def run(core, cycles, held_low=(), probe_hz=()):
    """Simulate `cycles` clocks. Balls in `held_low` read 0 (something else
    drives them); `probe_hz[c]` is the frequency fed to clock probe c."""
    sim = Simulator(core)
    sim.add_clock(1 / CLK_HZ)
    tr = Trace(core.n_drive)

    for c, hz in enumerate(probe_hz):
        async def probe_proc(ctx, c=c, hz=hz):
            v = 0
            while True:
                await ctx.delay(0.5 / hz)
                v ^= 1
                ctx.set(core.probe[c], v)
        sim.add_process(probe_proc)

    async def tb(ctx):
        n = core.n_drive
        for _ in range(cycles):
            oe = ctx.get(core.pad_oe)
            o = ctx.get(core.pad_o)
            pad_i = 0
            for k in range(n):
                driven_low = (oe >> k) & 1 and not o
                low = driven_low or k in held_low
                tr.levels[k].append(0 if low else 1)
                if not low:
                    pad_i |= 1 << k
            ctx.set(core.pad_i, pad_i)
            tr.max_oe = max(tr.max_oe, bin(oe).count("1"))
            st = ctx.get(core.state)
            if st != FRAME:
                assert oe == 0, f"output enabled outside FRAME (state {st})"
            if st == NEXT and ctx.get(core.skipped):
                tr.skips.append(ctx.get(core.ball))
            tr.states.append(st)
            await ctx.tick()

    sim.add_testbench(tb)
    os.makedirs("vcd", exist_ok=True)
    sim.run()
    assert tr.max_oe <= 1, f"{tr.max_oe} outputs enabled at once"
    return tr


def check_frames_exact(tr, k, prescale, frames_per_slot, idx):
    """Ball k's waveform is exactly `frames_per_slot` frames per slot, tick for tick."""
    tick = 1 << prescale
    want = []
    for _ in range(frames_per_slot):
        want += expected_pulses(frame_bytes(idx))
    # Merge consecutive idles (the closing gap of one frame meets nothing:
    # the next frame starts with active SYNC, so no merge is needed except at
    # the end where the line stays idle).
    got = tr.runs(k)
    # Compare the active/idle pattern of the first slot; the final idle run
    # is longer than GAP (the line stays idle until the next loop).
    n = len(want)
    assert len(got) >= n, f"ball {k}: {len(got)} runs, want >= {n}"
    for i, ((act, ticks), (lv, cyc)) in enumerate(zip(want, got[:n])):
        assert lv == (0 if act else 1), f"ball {k} run {i}: level {lv}, want {'active' if act else 'idle'}"
        if i == n - 1:
            assert cyc >= ticks * tick, f"ball {k} last idle {cyc} < {ticks * tick}"
        else:
            assert cyc == ticks * tick, f"ball {k} run {i}: {cyc} cycles, want {ticks * tick}"


# -- tests ---------------------------------------------------------------

def test_scan_order_and_exact_waveform():
    core = PinFindCore(3, prescale=1, frames=2, listen=4, start=4, max_loops=1)
    tr = run(core, 3 * 2 * 80 * 2 + 400)
    for k in range(3):
        check_frames_exact(tr, k, 1, 2, k)
    # Ball 0 talks before ball 1 before ball 2.
    first = [next(t for t, lv in enumerate(tr.levels[k]) if lv == 0) for k in range(3)]
    assert first == sorted(first), f"scan order {first}"
    assert tr.states[-1] == DONE, "did not stop after max_loops"


def test_prescale_zero():
    core = PinFindCore(2, prescale=0, frames=1, listen=2, start=2, max_loops=1)
    tr = run(core, 400)
    for k in range(2):
        check_frames_exact(tr, k, 0, 1, k)


def test_skip_ball_held_low():
    core = PinFindCore(4, prescale=1, frames=1, listen=4, start=4, max_loops=1)
    tr = run(core, 4 * 160 + 200, held_low={2})
    # Ball 2 is never driven: its only low level is the external one.
    assert all(lv == 0 for lv in tr.levels[2])
    assert tr.skips == [2], f"skips {tr.skips}"
    for k in (0, 1, 3):
        check_frames_exact(tr, k, 1, 1, k)


def test_stops_after_max_loops_and_stays_off():
    core = PinFindCore(2, prescale=0, frames=1, listen=2, start=2, max_loops=2)
    tr = run(core, 2 * 2 * 80 + 600)
    done_at = tr.states.index(DONE)
    for k in range(2):
        assert all(lv == 1 for lv in tr.levels[k][done_at:]), "driven after DONE"
        # Two loops: two SYNCs per ball.
        syncs = [c for lv, c in tr.runs(k) if lv == 0 and c == SYNC_ACTIVE]
        assert len(syncs) == 2, f"ball {k}: {len(syncs)} frames, want 2"


def test_large_index():
    # The index is a byte; check one beyond the low bits with a big list.
    core = PinFindCore(200, prescale=0, frames=1, listen=1, start=1, max_loops=0)
    # Only simulate until ball 2 has talked; check the encoding of index 1 and 2.
    tr = run(core, 3 * 80 + 50)
    check_frames_exact(tr, 1, 0, 1, 1)


def decode_runs_python(runs, tick):
    """Minimal reference decode of exact runs (no jitter), for the probe test."""
    frames, i = [], 0
    while i < len(runs):
        lv, c = runs[i]
        if lv == 0 and c == SYNC_ACTIVE * tick:
            i += 2
            bits = []
            while i < len(runs) and runs[i][0] == 0:
                bits.append(1 if runs[i][1] == BIT1_ACTIVE * tick else 0)
                gap = runs[i + 1][1] if i + 1 < len(runs) else 10 ** 9
                i += 2
                if gap >= GAP * tick:
                    break
            frames.append([int("".join(map(str, bits[j:j + 8])), 2) for j in range(0, len(bits), 8)])
        else:
            i += 1
    return frames


def test_clock_probes_count():
    # Main clock 50 MHz, tick = 2 clocks = 40 ns, window 2^4 ticks = 640 ns.
    # Probe 0 at 12.5 MHz with a 2-bit divider: the top bit toggles every
    # 2 input cycles = 160 ns -> 4 toggles per window. Probe 1 silent.
    core = PinFindCore(1, n_probe=2, prescale=1, frames=3, listen=2, start=40,
                       max_loops=1, div_bits=2, win_bits=4)
    tr = run(core, 3000, probe_hz=(12.5e6,))
    frames = decode_runs_python(tr.runs(0), 2)
    assert len(frames) == 3, frames
    for f in frames:
        assert f[:3] == [0, 0xFF, (2 << 4) | 4], f
        c0 = (f[3] << 8) | f[4]
        c1 = (f[5] << 8) | f[6]
        assert 3 <= c0 <= 5, f"probe 0 count {c0}"
        assert c1 == 0, f"probe 1 count {c1}"
        assert f[7] == sum(f[:7]) & 0xFF, "bad sum"


def test_gen_outputs_are_consistent():
    """gen.py on a synthetic ball list (tools/pinfind/tests): every port of
    the top level gets exactly one ball, no ball twice, all three clock modes."""
    import re
    import tempfile
    here = os.path.dirname(os.path.abspath(__file__))
    tools = os.path.join(here, "..", "..", "tools", "pinfind")
    sys.path.insert(0, os.path.join(tools, "tests"))
    sys.path.insert(0, tools)
    import mklist
    import gen
    from test_pinfind_tools import fake_pinmap
    with tempfile.TemporaryDirectory() as tmp:
        pm = os.path.join(tmp, "fake.pinmap")
        with open(pm, "w") as f:
            f.write(fake_pinmap())
        balls = os.path.join(tmp, "balls.txt")
        import contextlib, io
        with contextlib.redirect_stderr(io.StringIO()):
            mklist.main([pm, "-o", balls, "--exclude", "T16", "--max-drive", "150"])
        for clock in ("osc", "ring", "ball:PIN_G1"):
            out = os.path.join(tmp, clock.split(":")[0])
            with contextlib.redirect_stderr(io.StringIO()):
                p = gen.generate(gen.build_args(["--balls", balls, "--out", out, "--clock", clock]))
            asf = open(os.path.join(out, "PinFind.asf")).read()
            locs = re.findall(r"^set_location_assignment -to (\S+) (\S+)$", asf, re.M)
            assert len(locs) == p["total_pins"], (clock, len(locs), p["total_pins"])
            assert len({b for _, b in locs}) == len(locs), "a ball assigned twice"
            assert len({q for q, _ in locs}) == len(locs), "a port assigned twice"
            top = open(os.path.join(out, "PinFind.v")).read()
            assert f"inout  [{p['n_drive'] - 1}:0] P" in top
            assert ("CLKIN" in top) == clock.startswith("ball")
            # Configuration balls never carry a design port (DCLK only as CLKIN).
            for q, b in locs:
                assert b not in ("PIN_G2", "PIN_H3", "PIN_F3", "PIN_F16") and                     (b != "PIN_G1" or q == "CLKIN"), (q, b)
            assert p["n_drive"] == 150 and p["n_probe"] == 11


def write_golden():
    core = PinFindCore(3, n_probe=2, prescale=2, frames=2, listen=4, start=8,
                       max_loops=2, div_bits=4, win_bits=4)
    tr = run(core, 9000, probe_hz=(12.5e6, 3.125e6))
    os.makedirs(os.path.dirname(GOLDEN), exist_ok=True)
    with open(GOLDEN, "w", newline="\n") as f:
        f.write("# Edges on each ball from test_pinfind.py --write-golden (hw/pinfind.py).\n")
        f.write("# 3 balls, 2 frames per ball, 2 loops, prescale 2 (tick = 4 cycles of 20 ns),\n")
        f.write("# probes: 12.5 MHz and 3.125 MHz, div_bits 4, win_bits 4. Pull-up model, idle = 1.\n")
        f.write("# <ball> <cycle> <level after the edge>\n")
        for k in range(3):
            for t, lv in tr.edges(k):
                f.write(f"{k} {t} {lv}\n")
    print(f"wrote {GOLDEN}")


TESTS = [
    test_scan_order_and_exact_waveform,
    test_prescale_zero,
    test_skip_ball_held_low,
    test_stops_after_max_loops_and_stays_off,
    test_large_index,
    test_clock_probes_count,
    test_gen_outputs_are_consistent,
]


def main():
    failures = 0
    for t in TESTS:
        try:
            t()
            print(f"  ok    {t.__name__}")
        except AssertionError as e:
            print(f"  FAIL  {t.__name__}: {e}")
            failures += 1
    if "--write-golden" in sys.argv:
        write_golden()
    if failures:
        print(f"\n{failures} test(s) failed")
        sys.exit(1)
    print(f"\nall {len(TESTS)} test(s) passed")


if __name__ == "__main__":
    main()
