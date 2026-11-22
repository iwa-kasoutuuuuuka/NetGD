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
        self.tripped = False

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


def run(core, cycles, held_low=(), probe_hz=(), expect_trip=False):
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
            if ctx.get(core.tripped):
                tr.tripped = True
            await ctx.tick()

    sim.add_testbench(tb)
    os.makedirs("vcd", exist_ok=True)
    sim.run()
    assert tr.max_oe <= 1, f"{tr.max_oe} outputs enabled at once"
    assert tr.tripped == expect_trip, f"drive watchdog tripped={tr.tripped}, expected {expect_trip}"
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
    core = PinFindCore(200, prescale=0, frames=1, listen=1, start=1, max_loops=1)
    # Only simulate until ball 2 has talked; check the encoding of index 1 and 2.
    tr = run(core, 3 * 80 + 50)
    check_frames_exact(tr, 1, 0, 1, 1)


def test_max_loops_is_bounded():
    import warnings
    warnings.filterwarnings("ignore", message=".*created but never used")
    for bad in (0, 9, -1):
        try:
            PinFindCore(2, max_loops=bad)
        except ValueError:
            continue
        raise AssertionError(f"max_loops={bad} accepted")


def test_drive_watchdog_cuts_a_long_frame():
    # A cap shorter than one frame: the watchdog must drop the output enable
    # within cap << prescale clocks, go to DONE, and never drive again.
    core = PinFindCore(3, prescale=1, frames=2, listen=2, start=2, max_loops=1, drive_cap=5)
    tr = run(core, 2000, expect_trip=True)
    assert 0 < sum(1 for lv in tr.levels[0] if lv == 0) <= 5 << 1, "ball 0 driven beyond the cap"
    done_at = tr.states.index(DONE)
    for k in range(3):
        assert all(lv == 1 for lv in tr.levels[k][done_at:]), f"ball {k} driven after the watchdog"
    assert all(lv == 1 for lv in tr.levels[1]) and all(lv == 1 for lv in tr.levels[2]), "scan went on"


def test_drive_watchdog_default_never_trips_with_probes():
    # The default cap covers the longest frames (with probes, all frames).
    core = PinFindCore(2, n_probe=3, prescale=0, frames=3, listen=2, start=2, max_loops=1,
                       div_bits=2, win_bits=2)
    tr = run(core, 2 * 3 * 450 + 400, probe_hz=(12.5e6, 6e6, 1e6))
    assert tr.states[-1] == DONE


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


def _gen_env():
    import tempfile
    here = os.path.dirname(os.path.abspath(__file__))
    tools = os.path.join(here, "..", "..", "tools", "pinfind")
    sys.path.insert(0, os.path.join(tools, "tests"))
    sys.path.insert(0, tools)
    import mklist
    import gen
    from test_pinfind_tools import fake_pinmap
    return tempfile, mklist, gen, fake_pinmap


def _quiet(f, *a):
    import contextlib
    import io
    with contextlib.redirect_stderr(io.StringIO()):
        return f(*a)


def _locs(out):
    import re
    asf = open(os.path.join(out, "PinFind.asf")).read()
    return [(q.strip("{}"), b) for q, b in re.findall(r"^set_location_assignment -to (\S+) (\S+)$", asf, re.M)]


def _fake_list(tmp, *args):
    tempfile, mklist, gen, fake_pinmap = _gen_env()
    pm = os.path.join(tmp, "fake.pinmap")
    with open(pm, "w") as f:
        f.write(fake_pinmap())
    balls = os.path.join(tmp, "balls.txt")
    _quiet(mklist.main, [pm, "-o", balls, *args])
    plain, special = mklist.parse_pinmap(fake_pinmap())
    return balls, set(mklist.user_capable(plain, special))


def test_gen_outputs_are_consistent():
    """gen.py on a synthetic ball list (tools/pinfind/tests): every port of
    the top level gets exactly one ball, no ball twice, all clock modes, the
    PS balls G1/G2 and the DDR balls are inputs, clock balls are inputs
    unless probed, DCLK is toggled only when G1 is the clock."""
    tempfile, mklist, gen, fake_pinmap = _gen_env()
    import portmap
    with tempfile.TemporaryDirectory() as tmp:
        balls, user = _fake_list(tmp, "--exclude", "T16", "--max-drive", "150")
        lst = mklist.read_list(balls)
        for clock, extra in (("osc", []), ("ring", []), ("ball:PIN_G1", []), ("osc", ["--probes"]),
                             ("ball:PIN_J4", [])):
            out = os.path.join(tmp, clock.replace(":", "_") + "".join(extra))
            p = _quiet(gen.generate, gen.build_args(["--balls", balls, "--out", out, "--clock", clock] + extra))
            locs = _locs(out)
            assert len(locs) == p["total_pins"], (clock, len(locs), p["total_pins"])
            assert len({b for _, b in locs}) == len(locs), "a ball assigned twice"
            assert len({q for q, _ in locs}) == len(locs), "a port assigned twice"
            # The .asf is exactly what portmap (and so build.sh's checkpnr.py) expects.
            plan = portmap.plan(lst, clock, probes="--probes" in extra)
            assert locs == portmap.ports(plan), clock
            # Every user-capable ball of the pinmap is in the design; nothing dedicated is.
            assert {b for _, b in locs} == user, (clock, user ^ {b for _, b in locs})
            top = open(os.path.join(out, "PinFind.v")).read()
            assert f"inout  [{p['n_drive'] - 1}:0] P" in top
            assert ("CLKIN" in top) == clock.startswith("ball")
            port_of = {b: q for q, b in locs}
            for b in ("PIN_H3", "PIN_F3", "PIN_F16", "PIN_F15", "PIN_H1", "PIN_H2", "PIN_J1", "PIN_J2", "PIN_H4"):
                assert b not in port_of, (clock, b)
            assert port_of["PIN_G2"].startswith("HOLD["), port_of["PIN_G2"]
            if clock == "ball:PIN_G1":
                assert port_of["PIN_G1"] == "CLKIN"
            else:
                assert port_of["PIN_G1"].startswith("HOLD["), port_of["PIN_G1"]
            for b in ("PIN_C1", "PIN_D2", "PIN_F10", "PIN_M10", "PIN_L11", "PIN_M11"):
                assert port_of[b].startswith("HOLD["), (b, port_of[b])
            assert p["toggle_dclk"] == (1 if clock == "ball:PIN_G1" else 0), clock
            assert p["n_drive"] == 150
            if "--probes" in extra:
                assert p["n_probe"] == 11 and port_of["PIN_J4"] == "PROBE[0]"
            else:
                assert p["n_probe"] == 0
                if clock == "ball:PIN_J4":
                    assert port_of["PIN_J4"] == "CLKIN"
                else:
                    assert port_of["PIN_J4"].startswith("HOLD[")
            # First-run defaults (R143): 2 frames, 1 loop, T >= 20 us.
            assert (p["frames"], p["max_loops"], p["stage0"]) == (2, 1, 0)
            assert p["prescale"] == {"osc": 10, "ring": 12, "ball": 6}[clock.split(":")[0]]
            assert p["drive_cap_ticks"] > 0


def test_gen_stage0_has_no_outputs():
    tempfile, mklist, gen, fake_pinmap = _gen_env()
    with tempfile.TemporaryDirectory() as tmp:
        balls, user = _fake_list(tmp)
        out = os.path.join(tmp, "s0")
        p = _quiet(gen.generate, gen.build_args(["--balls", balls, "--out", out, "--stage0"]))
        top = open(os.path.join(out, "PinFind.v")).read()
        import re
        assert not re.search(r"^\s*(inout|output)\b|\bassign\b", top, re.M), "stage 0 drives something"
        assert "PinFindCore" not in top and not os.path.exists(os.path.join(out, "PinFindCore.v"))
        assert (p["stage0"], p["toggle_dclk"], p["n_probe"], p["clock"]) == (1, 0, 0, "none")
        locs = _locs(out)
        assert len(locs) == p["total_pins"] and len({b for _, b in locs}) == len(locs)
        assert {b for _, b in locs} == user
        assert all(q.startswith(("P[", "HOLD[")) for q, _ in locs)


def test_gen_refuses_unsafe_options():
    tempfile, mklist, gen, fake_pinmap = _gen_env()
    bad = [
        ["--drive", "pp", "--no-skip-low"],
        ["--max-loops", "0"], ["--max-loops", "9"],
        ["--frames", "0"], ["--frames", "9"],
        ["--stage0", "--probes"], ["--stage0", "--clock", "ring"],
        ["--prescale", "21"],
    ]
    for extra in bad:
        a = gen.build_args(["--balls", "unused.txt"] + extra)
        assert gen.check_args(a), f"accepted {extra}"
    assert gen.check_args(gen.build_args(["--balls", "x"])) is None
    assert gen.check_args(gen.build_args(["--balls", "x", "--drive", "pp"])) is None
    assert gen.check_args(gen.build_args(["--balls", "x", "--no-skip-low"])) is None


def _fake_supra_work(tmp, out, placement, run_out=""):
    """A work directory with a pin table in the Quartus .pin style (SYNTHETIC:
    the real Supra report format is unknown) and a run.out."""
    work = os.path.join(tmp, "work")
    os.makedirs(work, exist_ok=True)
    with open(os.path.join(work, "run.out"), "w") as f:
        f.write("Info: synthetic\n" + run_out)
    with open(os.path.join(work, "PinFind.pin"), "w") as f:
        f.write("Pin Name/Usage : Location : Dir\n")
        for q, b in placement:
            f.write(f"{q} : {b} : input\n")
    return work


def _checkpnr(gen_out, work, pinmap, *extra):
    import contextlib
    import io
    import checkpnr
    buf = io.StringIO()
    try:
        with contextlib.redirect_stdout(buf):
            code = checkpnr.main(["--gen", gen_out, "--work", work, "--pinmap", pinmap, *extra])
    except SystemExit as e:
        return 1, str(e)
    return code, buf.getvalue()


def test_checkpnr_placement_and_coverage():
    """build.sh's post-route check on synthetic Supra outputs: passes only on
    an exact placement, with no auto-placement, every user-capable ball used."""
    tempfile, mklist, gen, fake_pinmap = _gen_env()
    with tempfile.TemporaryDirectory() as tmp:
        balls, user = _fake_list(tmp, "--max-drive", "20")
        pinmap = os.path.join(tmp, "fake.pinmap")
        out = os.path.join(tmp, "gen")
        _quiet(gen.generate, gen.build_args(["--balls", balls, "--out", out]))
        locs = _locs(out)

        work = _fake_supra_work(os.path.join(tmp, "ok"), out, locs)
        code, text = _checkpnr(out, work, pinmap)
        assert code == 0, text
        assert "all %d ports on their expected balls" % len(locs) in text, text

        # Two balls swapped by the placer.
        bad = list(locs)
        bad[0], bad[1] = (bad[0][0], bad[1][1]), (bad[1][0], bad[0][1])
        code, text = _checkpnr(out, _fake_supra_work(os.path.join(tmp, "swap"), out, bad), pinmap)
        assert code == 1 and "differs" in text, text

        # Supra placed something by itself.
        work = _fake_supra_work(os.path.join(tmp, "auto"), out, locs,
                                "Warn: IO P[3] is not assigned, placed at pin PIN_A9\n")
        code, text = _checkpnr(out, work, pinmap)
        assert code == 1 and "automatic placement" in text, text

        # The normal pre-.asf "Error: Pin ...'s locatiion is not assigned." is not auto-placement.
        work = _fake_supra_work(os.path.join(tmp, "pre"), out, locs,
                                "Error: Pin P[3]'s locatiion is not assigned.\n")
        assert _checkpnr(out, work, pinmap)[0] == 0

        # No placement source at all: fail, never "assume the .asf".
        work = os.path.join(tmp, "none", "work")
        os.makedirs(work)
        open(os.path.join(work, "run.out"), "w").write("Info\n")
        import shutil
        shutil.copy(os.path.join(out, "PinFind.asf"), work)
        code, text = _checkpnr(out, work, pinmap)
        assert code == 1 and "NOT confirmed" in text, text

        # A placement source named by a human, with a regex.
        rep_file = os.path.join(work, "io.rpt")
        with open(rep_file, "w") as f:
            for q, b in locs:
                f.write(f"IO {q} placed {b} (x,y)\n")
        code, text = _checkpnr(out, work, pinmap, "--placement", rep_file,
                               "--placement-regex", r"^IO (?P<port>\S+) placed (?P<ball>PIN_\w+)")
        assert code == 0, text
        # ... but never the .asf.
        code, text = _checkpnr(out, work, pinmap, "--placement", os.path.join(work, "PinFind.asf"),
                               "--placement-regex", r"-to (?P<port>\S+) (?P<ball>\S+)")
        assert code == 1, text

        # A user-capable ball left out of the design fails unless accepted.
        balls2 = os.path.join(tmp, "balls2.txt")
        _quiet(mklist.main, [pinmap, "-o", balls2, "--max-drive", "20", "--leave-out", "C1"])
        out2 = os.path.join(tmp, "gen2")
        _quiet(gen.generate, gen.build_args(["--balls", balls2, "--out", out2]))
        locs2 = _locs(out2)
        work2 = _fake_supra_work(os.path.join(tmp, "lo"), out2, locs2)
        code, text = _checkpnr(out2, work2, pinmap)
        assert code == 1 and "PIN_C1" in text and "not in the design" in text, text
        code, text = _checkpnr(out2, work2, pinmap, "--unused-ok", "C1")
        assert code == 0, text

        # A stale ball list (not the one params.txt was made from).
        with open(os.path.join(out, "balls.txt"), "a") as f:
            f.write("# edited\n")
        code, text = _checkpnr(out, _fake_supra_work(os.path.join(tmp, "st"), out, locs), pinmap)
        assert code == 1 and "not the list" in text, text


def _supra_work(tmp, placement, sites=None, routed=True):
    """A work directory shaped like Supra 2026.03.b0's (first real build):
    alta_db/io.asf (ports in reverse order, no braces, IO_STANDARD lines) and
    PinFind_routed.v whose pad cells carry coord_x/y/z, in both naming styles
    (`\\P[0]~io (` with `defparam \\P[0]~io .coord_x`, `hold_keep_o_0(` with
    `defparam hold_keep_o_0.coord_x`). The contents are synthetic."""
    work = os.path.join(tmp, "work")
    os.makedirs(os.path.join(work, "alta_db"), exist_ok=True)
    with open(os.path.join(work, "run.out"), "w") as f:
        f.write("Error: Pin HOLD[0]'s locatiion is not assigned.\nInfo: Place virtual pin HOLD[0] at PIN_T16.\n")
    with open(os.path.join(work, "alta_db", "io.asf"), "w") as f:
        for q, b in reversed(placement):
            f.write(f"set_location_assignment -to {q} {b}\n")
        for q, _ in reversed(placement):
            f.write(f'set_instance_assignment -name IO_STANDARD -to {q} "3.3-V LVTTL"\n')
    if routed:
        sites = sites or {q: (i // 4, 7, i % 4) for i, (q, _) in enumerate(placement)}
        with open(os.path.join(work, "PinFind_routed.v"), "w") as f:
            f.write("module PinFind (P, HOLD);\nwire gnd;\n")
            for i, (q, _) in enumerate(placement):
                cell = f"\\{q}~io " if q.startswith("P[") else f"hold_keep_o_{i}"
                f.write(f"alta_dio {cell}(\n\t.padio({q}),\n\t.oe(gnd),\n\t.regout());\n")
                for k, v in zip("xyz", sites[q]):
                    f.write(f"defparam {cell}.coord_{k} = {v};\n")
            f.write("alta_slice \\core|x (\n\t.A(gnd),\n\t.Q());\ndefparam \\core|x .coord_x = 3;\n")
            f.write("endmodule\n")
    return work


def test_checkpnr_reads_supra_io_placement():
    """checkpnr.py's default source with Supra 2026.03.b0: alta_db/io.asf,
    cross-checked against the pad sites of the routed netlist."""
    tempfile, mklist, gen, fake_pinmap = _gen_env()
    with tempfile.TemporaryDirectory() as tmp:
        balls, user = _fake_list(tmp, "--max-drive", "20")
        pinmap = os.path.join(tmp, "fake.pinmap")
        out = os.path.join(tmp, "gen")
        _quiet(gen.generate, gen.build_args(["--balls", balls, "--out", out]))
        locs = _locs(out)

        work = _supra_work(os.path.join(tmp, "ok"), locs)
        code, text = _checkpnr(out, work, pinmap)
        assert code == 0, text
        assert "io.asf (Supra I/O placement)" in text and "distinct sites" in text, text
        sites_file = os.path.join(work, "checkpnr.sites")
        rows = [l.split() for l in open(sites_file) if not l.startswith("#")]
        assert len(rows) == len(locs) and rows[0][:2] == list(locs[0]), rows[:2]

        # Same balls on the same sites in another build: ok. A ball on another site: fail.
        work2 = _supra_work(os.path.join(tmp, "ref"), locs)
        assert _checkpnr(out, work2, pinmap, "--sites-ref", sites_file)[0] == 0
        moved = {q: (i // 4, 7, i % 4) for i, (q, _) in enumerate(locs)}
        moved[locs[0][0]], moved[locs[1][0]] = moved[locs[1][0]], moved[locs[0][0]]
        code, text = _checkpnr(out, _supra_work(os.path.join(tmp, "mv"), locs, moved), pinmap,
                               "--sites-ref", sites_file)
        assert code == 1 and "differ from --sites-ref" in text, text

        # Supra put two ports on other balls.
        bad = list(locs)
        bad[0], bad[1] = (bad[0][0], bad[1][1]), (bad[1][0], bad[0][1])
        code, text = _checkpnr(out, _supra_work(os.path.join(tmp, "swap"), bad), pinmap)
        assert code == 1 and "differs" in text, text
        # ... or on something that is not a PIN_ ball: kept, so it is a mismatch.
        bad = [(locs[0][0], "NC_PIN_226N_CLK8")] + locs[1:]
        code, text = _checkpnr(out, _supra_work(os.path.join(tmp, "nc"), bad), pinmap)
        assert code == 1 and "NC_PIN_226N_CLK8" in text, text

        # A port missing from io.asf, or an I/O that is not a port: stop.
        code, text = _checkpnr(out, _supra_work(os.path.join(tmp, "miss"), locs[1:]), pinmap)
        assert code == 1 and "missing" in text, text
        code, text = _checkpnr(out, _supra_work(os.path.join(tmp, "ext"), locs + [("X", "PIN_T16")]), pinmap)
        assert code == 1 and "other I/O" in text, text

        # Two ports on one site in the routed netlist; no routed netlist at all.
        same = {q: (0, 7, 0) if i < 2 else (i // 4, 8, i % 4) for i, (q, _) in enumerate(locs)}
        code, text = _checkpnr(out, _supra_work(os.path.join(tmp, "site"), locs, same), pinmap)
        assert code == 1 and "carry two ports" in text, text
        code, text = _checkpnr(out, _supra_work(os.path.join(tmp, "nor"), locs, routed=False), pinmap)
        assert code == 1 and "cannot be cross-checked" in text, text


def test_gatesim_testbench_reads_both_port_styles():
    """gatesim.py's port parser and testbench on two synthetic routed-netlist
    headers (bus ports; escaped per-bit ports). The real format is unknown."""
    import gatesim
    import tempfile
    p = {"n_drive": "3", "n_hold": "2", "n_probe": "0", "stage0": "0", "clock": "ball:PIN_G1"}
    with tempfile.TemporaryDirectory() as tmp:
        a = os.path.join(tmp, "a.v")
        open(a, "w").write("module PinFind (P, HOLD, CLKIN);\n inout [2:0] P;\n input [1:0] HOLD;\n"
                           " input CLKIN;\n alta_dio x ();\nendmodule\n")
        tb = gatesim.testbench(p, gatesim.top_ports(a), 5)
        assert ".P(pad)" in tb and ".HOLD({2{1'b1}})" in tb and ".CLKIN(clkin)" in tb, tb
        assert "pads low at once" in tb and "lows > 1" in tb
        assert "GATESIM DONE" not in tb
        tb = gatesim.testbench(p, gatesim.top_ports(a), 5, expect_done=True)
        assert "GATESIM DONE" in tb and "n_ever == 3" in tb and "5 * 900000.0" in tb, tb
        b = os.path.join(tmp, "b.v")
        open(b, "w").write("module PinFind (\\P[0] , \\P[1] , \\P[2] , \\HOLD[0] , \\HOLD[1] , CLKIN);\n"
                           " inout \\P[0] ;\n inout \\P[1] ;\n inout \\P[2] ;\n"
                           " input \\HOLD[0] ;\n input \\HOLD[1] ;\n input CLKIN;\nendmodule\n")
        tb = gatesim.testbench(p, gatesim.top_ports(b), 5)
        assert ".\\P[2] (pad[2])" in tb and ".\\HOLD[1] (1'b1)" in tb, tb
        p0 = dict(p, stage0="1", clock="none")
        tb = gatesim.testbench(p0, gatesim.top_ports(a), 5)
        assert "lows > 0" in tb and "clkin" not in tb
        r = os.path.join(tmp, "r.v")
        open(r, "w").write("module PinFind (P);\n alta_slice \\u_clk|stage[0].inv (\n .A(vcc));\n"
                           " alta_slice \\u_clk|stage[1].inv (\n .A(vcc));\nendmodule\n")
        seed = gatesim.ring_seed(r)
        assert seed == ["\\u_clk|stage[0].inv"], seed
        tb = gatesim.testbench(dict(p, clock="ring"), gatesim.top_ports(a), 5, xres=seed)
        assert "defparam dut.\\u_clk|stage[0].inv .GATESIM_XRES = 1'b1;" in tb, tb


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
    test_max_loops_is_bounded,
    test_drive_watchdog_cuts_a_long_frame,
    test_drive_watchdog_default_never_trips_with_probes,
    test_gen_outputs_are_consistent,
    test_gen_stage0_has_no_outputs,
    test_gen_refuses_unsafe_options,
    test_checkpnr_placement_and_coverage,
    test_checkpnr_reads_supra_io_placement,
    test_gatesim_testbench_reads_both_port_styles,
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
