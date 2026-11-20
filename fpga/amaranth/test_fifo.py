#!/usr/bin/env python3
"""Equivalence of the two FIFO memory styles (hw/fifo.py).

The AG16K build swaps the asynchronous-read FIFOs of the Cyclone II build
for registered-read ones that map to block RAM. The claim is that nothing
outside the FIFO can tell: every output is the same in every cycle. These
tests check that claim directly by running both styles side by side on the
same stimulus and comparing on every clock edge.

  fifo_lockstep_*   `SyncReadFIFO` against `ResetInserter(SyncFIFO)` under
                    random push/pop/clear traffic, at each depth the design
                    uses and two small ones that wrap constantly. Compares
                    r_data (also while empty), r_rdy, w_rdy and level.
  core_lockstep     two whole `OpenGDEMUCore`s, one per style, driven with
                    the same random MCU and IDE bus activity; compares
                    every output port.

Run from the fpga/amaranth directory:

    amaranth_env/bin/python3 test_fifo.py

Independent of NETGD_FPGA_TARGET: it always builds both.
"""

import random
import sys

from amaranth import *
from amaranth.hdl.xfrm import ResetInserter
from amaranth.lib.fifo import SyncFIFO
from amaranth.sim import Simulator

from hw.fifo import SyncReadFIFO
from hw.OpenGDEMU import OpenGDEMUCore


class FifoPair(Elaboratable):
    def __init__(self, depth):
        self.depth = depth
        self.clear = Signal()
        self.w_en = Signal()
        self.w_data = Signal(16)
        self.r_en = Signal()
        self.ref = ResetInserter(self.clear)(SyncFIFO(width=16, depth=depth))
        self.new = SyncReadFIFO(width=16, depth=depth)

    def elaborate(self, platform):
        m = Module()
        m.submodules.ref = self.ref
        m.submodules.new = self.new
        for f in (self.ref, self.new):
            m.d.comb += [
                f.w_en.eq(self.w_en),
                f.w_data.eq(self.w_data),
                f.r_en.eq(self.r_en),
            ]
        m.d.comb += self.new.clear.eq(self.clear)
        return m


def run_fifo_lockstep(depth, cycles, seed):
    dut = FifoPair(depth)
    sim = Simulator(dut)
    sim.add_clock(1 / 48e6)
    rng = random.Random(seed)
    stats = {"push": 0, "pop": 0, "clear": 0, "empty_cmp": 0, "full": 0}

    async def tb(ctx):
        # Phases bias the traffic so the FIFO spends time full, empty and
        # hovering at one word (where the bypass register does the work).
        phase_len = max(64, min(4 * depth, cycles // 8))
        p_w, p_r = 0.5, 0.5
        for cyc in range(cycles):
            if cyc == 0:
                # Start by filling it, so "full" is reached at any depth.
                p_w, p_r = 1.0, 0.0
            elif cyc >= depth + 8 and cyc % phase_len == 0:
                p_w, p_r = rng.choice([
                    (0.9, 0.1), (0.1, 0.9), (0.5, 0.5), (1.0, 1.0),
                    (0.95, 0.9), (0.3, 1.0), (1.0, 0.0), (0.0, 1.0),
                ])
            # w_en / r_en are driven regardless of w_rdy / r_rdy, as the
            # design does: pushes into a full FIFO and pops of an empty one
            # must be ignored identically.
            w = rng.random() < p_w
            r = rng.random() < p_r
            c = cyc >= depth + 8 and rng.random() < 0.002
            ctx.set(dut.w_en, w)
            ctx.set(dut.w_data, rng.getrandbits(16))
            ctx.set(dut.r_en, r)
            ctx.set(dut.clear, c)
            ref = (ctx.get(dut.ref.r_data), ctx.get(dut.ref.r_rdy),
                   ctx.get(dut.ref.w_rdy), ctx.get(dut.ref.level))
            new = (ctx.get(dut.new.r_data), ctx.get(dut.new.r_rdy),
                   ctx.get(dut.new.w_rdy), ctx.get(dut.new.level))
            assert ref == new, (
                f"depth {depth} cycle {cyc}: async (r_data, r_rdy, w_rdy, "
                f"level) = {ref}, sync-read = {new}")
            stats["push"] += w and ref[2]
            stats["pop"] += r and ref[1]
            stats["clear"] += c
            stats["empty_cmp"] += not ref[1]
            stats["full"] += not ref[2]
            await ctx.tick()

    sim.add_testbench(tb)
    sim.run()
    # The run must actually have visited the corners it claims to cover.
    assert stats["push"] > depth and stats["pop"] > depth, stats
    assert stats["empty_cmp"] > 0 and stats["full"] > 0 and stats["clear"] > 0, stats
    return stats


OUTPUTS = [
    "io_MCU_DATA_OUT", "io_MCU_IRQ", "io_IDE_DATA_OUT", "io_IDE_DMARQ",
    "io_IDE_IORDY", "io_IDE_INTRQ", "io_DC_SCK", "io_DC_SDAT", "io_DC_LRCK",
]


def run_core_lockstep(cycles, seed):
    """Random bus traffic into both cores at once.

    Not a protocol model -- the directed suites do that, per target. This
    is the opposite: strobes of random length at random addresses, weighted
    towards the FIFO slots and the Data register, including overlapping
    MCU and IDE activity, DMA cycles, command writes (which clear the
    FIFOs) and resets. Whatever state that drives the cores into, the two
    must agree on every output in every cycle.
    """
    cores = [OpenGDEMUCore(target="cyclone2"), OpenGDEMUCore(target="ag16k")]
    sims = []
    traces = []
    for idx, core in enumerate(cores):
        rng = random.Random(seed)
        sim = Simulator(core)
        sim.add_clock(1 / 48e6, domain="sync")
        trace = []
        traces.append(trace)

        async def tb(ctx, core=core, rng=rng, trace=trace):
            def drive(name, value):
                ctx.set(getattr(core, name), value)

            drive("io_IDE_RSTn", 1)
            drive("io_IDE_CSn", 0b11)
            drive("io_IDE_DMACKn", 1)
            mcu_left = 0
            ide_left = 0
            audio = 0
            for cyc in range(cycles):
                # --- MCU bus: one access = CS + RD or WR held a few cycles.
                if mcu_left == 0:
                    drive("io_MCU_CS", 0)
                    drive("io_MCU_RD", 0)
                    drive("io_MCU_WR", 0)
                    if rng.random() < 0.6:
                        slot = rng.choice([9] * 12 + [10, 11, 1, 2, 3, 19, 19,
                                                      29, 30, 31, 31]
                                          + [rng.randrange(128)])
                        dev = 0x80 if rng.random() < 0.3 else 0
                        wr = rng.random() < 0.7
                        data = rng.getrandbits(16)
                        if slot == 19 and wr:
                            # Keep DMA / flow-control modes in play.
                            data = rng.choice([0, 1, 1, 1, 2, 2, 4, 8, 9, 0x10])
                        drive("io_MCU_ADDR", dev | slot)
                        drive("io_MCU_BS", 0b11)
                        drive("io_MCU_DATA_IN", data)
                        drive("io_MCU_CS", 1)
                        drive("io_MCU_WR", 1 if wr else 0)
                        drive("io_MCU_RD", 0 if wr else 1)
                        mcu_left = rng.randrange(1, 7)
                else:
                    mcu_left -= 1
                # --- IDE bus: register or DMA cycle, strobe 1-8 cycles.
                if ide_left == 0:
                    drive("io_IDE_WR", 0)
                    drive("io_IDE_RD", 0)
                    drive("io_IDE_RD_H", 0)
                    drive("io_IDE_WR_H", 0)
                    r = rng.random()
                    if r < 0.0005:
                        drive("io_IDE_RSTn", 0)
                    elif r < 0.01:
                        drive("io_IDE_RSTn", 1)
                    if rng.random() < 0.5:
                        dma = rng.random() < 0.5
                        drive("io_IDE_DMACKn", 0 if dma else 1)
                        if dma:
                            drive("io_IDE_CSn", 0b11)
                        else:
                            drive("io_IDE_CSn",
                                  0b01 if rng.random() < 0.05 else 0b10)
                            # Data mostly; Device and Command sometimes.
                            drive("io_IDE_ADDR",
                                  rng.choice([0] * 20 + [6, 7] + [rng.randrange(8)]))
                        wr = rng.random() < 0.3
                        data = rng.getrandbits(16)
                        if rng.random() < 0.5:
                            # Device register value selecting either drive.
                            data = (data & ~0x10) | (rng.getrandbits(1) << 4)
                        drive("io_IDE_DATA_IN", data)
                        drive("io_IDE_WR", 1 if wr else 0)
                        drive("io_IDE_WR_H", 1 if wr else 0)
                        drive("io_IDE_RD", 0 if wr else 1)
                        drive("io_IDE_RD_H", 0 if wr else 1)
                        ide_left = rng.randrange(1, 9)
                    elif rng.random() < 0.3:
                        drive("io_IDE_DMACKn", 1)
                else:
                    ide_left -= 1
                # --- audio clock, roughly a quarter of the core clock.
                if cyc % 2 == 0:
                    audio ^= 1
                    drive("io_AUDIO_CLK", audio)
                drive("io_audio_tick", (cyc >> 5) & 1)

                trace.append(tuple(ctx.get(getattr(core, n)) for n in OUTPUTS)
                             + (ctx.get(core.dev0.dbg_rfifo_level),
                                ctx.get(core.dev1.dbg_rfifo_level)))
                await ctx.tick()

        sim.add_testbench(tb)
        sims.append(sim)
    for sim in sims:
        sim.run()

    ref, new = traces
    assert len(ref) == len(new) == cycles
    names = OUTPUTS + ["dev0.rfifo_level", "dev1.rfifo_level"]
    for cyc, (a, b) in enumerate(zip(ref, new)):
        if a != b:
            diff = [f"{n}: {x:#x} vs {y:#x}" for n, x, y in zip(names, a, b) if x != y]
            raise AssertionError(f"cores diverge at cycle {cyc}: {', '.join(diff)}")

    # Evidence the traffic reached the FIFOs and the bus, not just idled.
    data_out = {t[names.index("io_IDE_DATA_OUT")] for t in ref}
    mcu_out = {t[names.index("io_MCU_DATA_OUT")] for t in ref}
    lvl0 = max(t[-2] for t in ref)
    lvl1 = max(t[-1] for t in ref)
    dmarq = sum(t[names.index("io_IDE_DMARQ")] for t in ref)
    assert len(data_out) > 100 and len(mcu_out) > 100, (len(data_out), len(mcu_out))
    assert lvl0 > 16 and lvl1 > 16, (lvl0, lvl1)
    assert dmarq > 0
    return {"ide_data_values": len(data_out), "mcu_data_values": len(mcu_out),
            "max_rfifo0": lvl0, "max_rfifo1": lvl1, "dmarq_cycles": dmarq}


# Depths in use: primary/secondary read FIFO, primary/secondary write FIFO,
# CDDA (not a power of two). 5 and 16 wrap every few cycles.
FIFO_CASES = [(4096, 60000), (1024, 30000), (512, 20000), (32, 20000),
              (768, 30000), (16, 20000), (5, 20000)]

TESTS = (
    [(f"fifo_lockstep_depth_{d}", lambda d=d, n=n: run_fifo_lockstep(d, n, seed=d))
     for d, n in FIFO_CASES]
    + [(f"core_lockstep_seed_{s}", lambda s=s: run_core_lockstep(60000, seed=s))
       for s in (1, 2, 3)]
)


def main() -> None:
    failures = 0
    for name, fn in TESTS:
        try:
            stats = fn()
            print(f"  ok    {name}  {stats}")
        except AssertionError as e:
            print(f"  FAIL  {name}: {e}")
            failures += 1
    print()
    print(f"all {len(TESTS)} test(s) passed" if not failures
          else f"{failures} test(s) failed")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
