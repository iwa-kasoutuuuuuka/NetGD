#!/usr/bin/env python3
"""Asynchronous-host tests: the Dreamcast and the MCU on their own timebases.

test_top.py drives the bus one core-clock tick at a time, which can only ever
put a strobe edge on a clock edge. The bugs that survived it -- a word read
twice at a burst boundary, a word skipped when two strobes merge, the host
outrunning a refill in progress -- all live in the *phase* between the G1 bus
and the 48 MHz core clock, and between the MCU's SMC writes and both of them.

So here nothing waits on the clock. The host is a coroutine that sleeps in
nanoseconds with multiword-DMA mode 2 numbers (ATA/ATAPI-5 table 50), the MCU
is a coroutine with the SAM3U SMC's 12-MCK write cycle, and the core clock
runs underneath them at 48 MHz. A 120 ns bus cycle is 5.76 core periods, so
the phase walks across every possible alignment within a few words.

Two limits to keep in mind when reading a pass: the RTL has no propagation
delay, so this cannot see the clock-to-out margin that static timing
reports; and the host model encodes what has been *observed* of Holly (32-byte
bursts once DMARQ is granted, DMACK dropped after each), not a datasheet.

Run:  amaranth_env/bin/python3 test_timing.py
"""

import os
import sys

from amaranth import *
from amaranth.sim import Simulator

from hw.fifo import resolve_target
from hw.OpenGDEMU import OpenGDEMUCore


NS = 1e-9
CORE_PERIOD = 1 / 48e6

# Multiword DMA mode 2 (ATA/ATAPI-5 table 50), nanoseconds.
MWDMA2 = dict(
    t0=120,   # cycle time
    tD=70,    # DIOR-/DIOW- asserted width
    tKR=25,   # DIOR- negated width (minimum; t0 - tD in practice)
    tI=0,     # DMACK- to DIOR-/DIOW- setup
    tJ=5,     # DIOR-/DIOW- to DMACK- hold
    tLR=35,   # DIOR- to DMARQ delay (device side); host uses it as a wait
)
# Holly moves 32 bytes per DMARQ grant and releases DMACK after each burst.
HOLLY_BURST_WORDS = 16

# SAM3U SMC at 96 MHz: 1 MCK setup, 8 MCK pulse, 12 MCK cycle (smc.rs).
MCK = 1e9 / 96e6
SMC_SETUP = 1 * MCK
SMC_PULSE = 8 * MCK
SMC_CYCLE = 12 * MCK

ADDR_DATA = 0
ADDR_SECTOR_COUNT = 2
ADDR_DEVICE = 6
ADDR_COMMAND = 7
CSN_CMD = 0b10
CSN_IDLE = 0b11
CSN_CTRL_ALT = 0b01
ADDR_ALT_STATUS = 6
ADDR_STATUS = 7

MCU_HW_INFO = 0
MCU_STATUS = 1
MCU_FIFO = 9
MCU_RFIFO_LEVEL = 10
MCU_DMA_MODE = 19
SECONDARY = 0x80


class SimTop(Elaboratable):
    """The Quartus wrapper's job, in simulation: negedge samples of the strobes.

    src/OpenGDEMU.v captures DC_RDn/DC_WRn on the falling clock edge and hands
    those to the core as io_IDE_RD_H/WR_H. That half-phase sample is part of
    the strobe detector, so a test without it is not testing the design.
    Pins here are active-high, as the core sees them after the wrapper's
    inversion.
    """

    def __init__(self):
        self.core = OpenGDEMUCore()
        self.ide_rd = Signal()
        self.ide_wr = Signal()

    def elaborate(self, platform):
        m = Module()
        m.submodules.core = core = self.core

        m.domains.neg = neg = ClockDomain("neg", clk_edge="neg")
        m.d.comb += [neg.clk.eq(ClockSignal("sync")), neg.rst.eq(ResetSignal("sync"))]
        rd_half = Signal()
        wr_half = Signal()
        m.d.neg += [rd_half.eq(self.ide_rd), wr_half.eq(self.ide_wr)]
        m.d.comb += [
            core.io_IDE_RD.eq(self.ide_rd),
            core.io_IDE_WR.eq(self.ide_wr),
            core.io_IDE_RD_H.eq(rd_half),
            core.io_IDE_WR_H.eq(wr_half),
        ]
        return m


# -- host (Dreamcast G1) model ------------------------------------------------

class Host:
    def __init__(self, ctx, top, timing=MWDMA2, abort_on_dmarq_drop=True):
        self.ctx, self.top, self.core, self.t = ctx, top, top.core, timing
        self.dev = top.core.dev1
        # (word index, read FIFO level, IORDY) at each DMA sample point.
        self.trace = []
        # Holly, reading: if DMARQ negates part-way through a 16-word burst
        # it ends the burst, counts only the whole 32-byte chunks and
        # re-requests the rest. Seen on hardware as every READ DMA stopping
        # exactly one chunk short ("done=480 of 512") once the FPGA started
        # negating DMARQ at level < 16 instead of at empty. Whether it does
        # the same when writing is unknown (a write grant with three words
        # free still delivered sixteen), so it is a switch.
        self.abort_on_dmarq_drop = abort_on_dmarq_drop
        self.aborted_bursts = 0

    async def delay(self, ns):
        await self.ctx.delay(ns * NS)

    def idle(self):
        c = self.ctx
        c.set(self.core.io_IDE_CSn, CSN_IDLE)
        c.set(self.top.ide_rd, 0)
        c.set(self.top.ide_wr, 0)
        c.set(self.core.io_IDE_DMACKn, 1)

    async def reset(self):
        self.idle()
        self.ctx.set(self.core.io_IDE_RSTn, 0)
        await self.delay(200)
        self.ctx.set(self.core.io_IDE_RSTn, 1)
        # post_reset_count is 24 bits on the real core; the tests below only
        # need BSY to clear, which the MCU does explicitly.
        await self.delay(500)

    async def pio_write(self, csn, addr, data):
        """PIO mode 4 register write: t0 120, t2 70, data setup/hold 25/10."""
        c = self.ctx
        c.set(self.core.io_IDE_CSn, csn)
        c.set(self.core.io_IDE_ADDR, addr)
        await self.delay(25)                       # t1 address setup
        c.set(self.core.io_IDE_DATA_IN, data)
        c.set(self.top.ide_wr, 1)
        await self.delay(70)                       # t2
        c.set(self.top.ide_wr, 0)
        await self.delay(10)                       # t4 data hold
        c.set(self.core.io_IDE_DATA_IN, 0)
        await self.delay(40)
        self.idle()

    async def pio_read(self, csn, addr):
        c = self.ctx
        c.set(self.core.io_IDE_CSn, csn)
        c.set(self.core.io_IDE_ADDR, addr)
        await self.delay(25)
        c.set(self.top.ide_rd, 1)
        await self.delay(70)
        value = c.get(self.core.io_IDE_DATA_OUT)   # latched on DIOR- negation
        c.set(self.top.ide_rd, 0)
        await self.delay(50)
        self.idle()
        return value

    async def wait_dmarq(self, timeout_ns=2_000_000):
        waited = 0
        while not self.ctx.get(self.core.io_IDE_DMARQ):
            await self.delay(10)
            waited += 10
            if waited > timeout_ns:
                raise AssertionError("DMARQ never asserted")

    async def dma_read_burst(self, words=HOLLY_BURST_WORDS, stall_log=None):
        """One Holly burst: DMACK- low, `words` DIOR- pulses, DMACK- high.

        Holly does not stop for DMARQ negating mid-burst (observed on the write
        side: a grant with three words free still delivered sixteen). Whether
        it honours IORDY in DMA is unknown, so the default is that it does not.
        """
        t, c = self.t, self.ctx
        got = []
        c.set(self.core.io_IDE_CSn, CSN_IDLE)
        c.set(self.core.io_IDE_DMACKn, 0)
        await self.delay(max(t["tI"], 5))
        for i in range(words):
            if i and self.abort_on_dmarq_drop and not c.get(self.core.io_IDE_DMARQ):
                self.aborted_bursts += 1
                await self.delay(t["tJ"])
                c.set(self.core.io_IDE_DMACKn, 1)
                return None
            c.set(self.top.ide_rd, 1)
            await self.delay(t["tD"])
            if stall_log is not None and not c.get(self.core.io_IDE_IORDY):
                stall_log.append(i)
            got.append(c.get(self.core.io_IDE_DATA_OUT))
            self.trace.append((len(self.trace), c.get(self.dev.dbg_rfifo_level),
                               c.get(self.core.io_IDE_IORDY)))
            c.set(self.top.ide_rd, 0)
            await self.delay(t["t0"] - t["tD"])
        await self.delay(t["tJ"])
        c.set(self.core.io_IDE_DMACKn, 1)
        return got

    async def dma_read(self, total_words, gap_ns=100):
        """Read `total_words` the way Holly does: wait DMARQ, burst, repeat."""
        got = []
        while len(got) < total_words:
            await self.wait_dmarq()
            await self.delay(self.t["tLR"])
            burst = await self.dma_read_burst(min(HOLLY_BURST_WORDS, total_words - len(got)))
            if burst is not None:
                got += burst
            await self.delay(gap_ns)
        return got


# -- MCU (SAM3U SMC) model ---------------------------------------------------

class Mcu:
    def __init__(self, ctx, top, dev=SECONDARY):
        self.ctx, self.core, self.dev = ctx, top.core, dev

    async def delay(self, ns):
        await self.ctx.delay(ns * NS)

    async def write(self, addr, data):
        c = self.ctx
        c.set(self.core.io_MCU_ADDR, self.dev | addr)
        c.set(self.core.io_MCU_DATA_IN, data)
        await self.delay(SMC_SETUP)
        c.set(self.core.io_MCU_CS, 1)
        c.set(self.core.io_MCU_WR, 1)
        await self.delay(SMC_PULSE)
        c.set(self.core.io_MCU_CS, 0)
        c.set(self.core.io_MCU_WR, 0)
        await self.delay(SMC_CYCLE - SMC_SETUP - SMC_PULSE)

    async def read(self, addr):
        c = self.ctx
        c.set(self.core.io_MCU_ADDR, self.dev | addr)
        await self.delay(SMC_SETUP)
        c.set(self.core.io_MCU_CS, 1)
        c.set(self.core.io_MCU_RD, 1)
        await self.delay(SMC_PULSE)
        value = c.get(self.core.io_MCU_DATA_OUT)
        c.set(self.core.io_MCU_CS, 0)
        c.set(self.core.io_MCU_RD, 0)
        await self.delay(SMC_CYCLE - SMC_SETUP - SMC_PULSE)
        return value

    async def push(self, words):
        """DMAC burst into the FIFO register: back-to-back SMC writes."""
        for w in words:
            await self.write(MCU_FIFO, w)

    async def push_buf(self, words, depth):
        """push_buf_to_fifo: fill to depth-4, wait for room, repeat."""
        i = 0
        while i < len(words):
            level = await self.read(MCU_RFIFO_LEVEL)
            room = max(0, depth - (level + 4))
            if room == 0:
                await self.delay(1000)
                continue
            n = min(room, len(words) - i)
            await self.push(words[i:i + n])
            i += n


def pattern(sector, n=256):
    return [((sector * 256 + i) * 0x9E37 + 0x1234) & 0xFFFF for i in range(n)]


def classify(got, want):
    """Describe the first mismatch the way fatcheck does."""
    for i, (g, w) in enumerate(zip(got, want)):
        if g != w:
            tags = []
            if i and g == want[i - 1]:
                tags.append("== previous word (insertion)")
            if i + 1 < len(want) and g == want[i + 1]:
                tags.append("== next word (skip)")
            try:
                tags.append(f"== want[{want.index(g)}]")
            except ValueError:
                pass
            bad = sum(1 for a, b in zip(got, want) if a != b)
            return (f"word {i} (sector {i // 256} off {i % 256}) got={g:04x} "
                    f"want={w:04x} {' '.join(tags)}; {bad} bad of {len(want)}")
    return None


# -- testbenches --------------------------------------------------------------
#
# Each test is a pair of coroutines, host and MCU, added to pysim as separate
# testbenches so they run on independent timebases and only meet through the
# DUT -- the MCU notices the command the way the firmware does, by polling
# COMMAND_PEND. Results come back through `res`; checks run after sim.run().

def want_words(sectors):
    want = []
    for s in range(sectors):
        want += pattern(s)
    return want


async def mcu_wait_command(mcu, timeout_ns=50_000):
    waited = 0
    while not (await mcu.read(MCU_HW_INFO)) & 0x2:
        waited += SMC_CYCLE
        assert waited < timeout_ns, "MCU never saw COMMAND_PEND"


def dma_read_refill_race(sectors=4, sd_latency_ns=40_000, depth=1024, t0=120):
    """READ DMA of several sectors with the MCU refilling one sector behind.

    The firmware stages sector 0 before raising DRDY, then fetches each further
    sector from the SD card (hundreds of microseconds) and DMACs it into the
    FIFO at one word per 125 ns SMC cycle. The host drains a sector in ~31 us,
    so the FIFO is empty at every sector boundary and the host waits on
    DMARQ. When the refill starts, DMARQ is granted on the first word and the
    host then reads a 16-word burst at 120 ns per word -- faster than the
    words are arriving.

    `sd_latency_ns` must exceed the ~31 us the host needs to drain a sector,
    or the refill lands while the host is still busy and the race never
    happens (the real SD read is ~220 us). At t0 = 120 ns the host and the
    125 ns SMC cycle are matched closely enough that the FIFO sits at exactly
    one word for the whole burst and the read survives on 5 ns per word; at
    t0 = 110 it underruns at word 9 of the first refilled sector, at 100 at
    word 5. The hardware failed at word 12 of every sector on one build and a
    few words per 30 MB on another -- the same race at different phases.
    """
    want = want_words(sectors)
    timing = dict(MWDMA2, t0=t0, tD=min(MWDMA2["tD"], t0 - MWDMA2["tKR"]))

    async def host_tb(ctx, top, res):
        host = Host(ctx, top, timing)
        await host.reset()
        await host.delay(2000)                                   # MCU boot
        await host.pio_write(CSN_CMD, ADDR_DEVICE, 0x10)         # DEV=1
        await host.pio_write(CSN_CMD, ADDR_SECTOR_COUNT, sectors)
        await host.pio_write(CSN_CMD, ADDR_COMMAND, 0xC8)        # READ DMA
        res["got"] = [v & 0xFFFF for v in await host.dma_read(len(want))]
        res["trace"] = host.trace
        res["aborted"] = host.aborted_bursts

    async def mcu_tb(ctx, top, res):
        mcu = Mcu(ctx, top)
        await mcu.delay(800)
        await mcu.write(MCU_STATUS, 0x50)                        # DRDY|DSC
        await mcu_wait_command(mcu)
        # handle_disk_read: stage sector 0, then DRDY|DSC + dma_mode=1.
        await mcu.push(pattern(0))
        await mcu.write(MCU_STATUS, 0x50)
        await mcu.write(MCU_DMA_MODE, 1)
        await mcu.write(MCU_HW_INFO, 0)
        for s in range(1, sectors):
            await mcu.delay(sd_latency_ns)
            await mcu.push_buf(pattern(s), depth)

    def check(res):
        empties = [(i, lvl, rdy) for i, lvl, rdy in res["trace"] if lvl == 0]
        low = min(lvl for _, lvl, _ in res["trace"][256:])
        print(f"        refill: min level after sector 0 = {low}, "
              f"{len(empties)} samples on an empty FIFO {empties[:6]}")
        problem = classify(res["got"], want)
        assert problem is None, problem
        assert res["aborted"] == 0, f'{res["aborted"]} burst(s) saw DMARQ negate mid-burst'

    return host_tb, mcu_tb, check


def dma_read_prestaged(sectors=2, abort=True):
    """Control: everything staged before the host starts. Must always pass."""
    want = want_words(sectors)

    async def host_tb(ctx, top, res):
        host = Host(ctx, top, abort_on_dmarq_drop=abort)
        await host.reset()
        await host.delay(2000)
        await host.pio_write(CSN_CMD, ADDR_DEVICE, 0x10)
        await host.pio_write(CSN_CMD, ADDR_COMMAND, 0xC8)
        res["got"] = [v & 0xFFFF for v in await host.dma_read(len(want))]
        res["aborted"] = host.aborted_bursts

    async def mcu_tb(ctx, top, res):
        mcu = Mcu(ctx, top)
        await mcu.delay(800)
        await mcu.write(MCU_STATUS, 0x50)
        await mcu_wait_command(mcu)
        await mcu.push(want)
        await mcu.write(MCU_DMA_MODE, 1)
        await mcu.write(MCU_HW_INFO, 0)

    def check(res):
        problem = classify(res["got"], want)
        assert problem is None, problem
        assert res["aborted"] == 0, f'{res["aborted"]} burst(s) saw DMARQ negate mid-burst'

    return host_tb, mcu_tb, check


# -- transfers that are not a whole number of 32-byte bursts (OI-04) -----------
#
# The FPGA grants DMARQ only with 16 words in the FIFO, so the firmware pads a
# read up to the next 32-byte block. What Holly does with the pad is NOT known.
# Stock, asked for exactly 2,352 or 16,464 B, completes with memory written 16 B
# further, those bytes zero (R183; probe record stock_20261123_27_dma-exact).
# That is memory, not bus cycles, and fits two hosts, so every case runs both:
#   "full"   strobes whole 16-word bursts, so it also takes the pad;
#   "exact"  strobes only the programmed length (last burst 8 words for 2,352)
#            and leaves the pad in the FIFO with DMARQ up.
# Only hardware decides; the firmware's "drain done (lvl=...)" line shows it.
#
# The MCU model follows stream_from_sd: prime, BSY + DMA mode, push the rest a
# chunk at a time, pad, then the drain loop with its 100-poll floor,
# host_took_everything, the fast-complete path, the aborted checks and
# DmaPadDrain. One firmware 1 ms poll is FW_POLL_NS here, to keep it cheap to
# simulate; it is still several times one burst (about 2 us).

DMA_BLOCK_BYTES = 32
FW_POLL_NS = 5_000
FW_MIN_WAIT_POLLS = 100
FW_CAP_POLLS = 200
DMA_PAD_SETTLE_READS = 3
FIRST_DMARQ_NS = 2_000_000
HOST_GIVES_UP_NS = 3_000_000
MCU_CMD_FEAT = 4
XFER_MASK = 0x0F00


def dma_pad_bytes(total):
    """discimg::atapi::dma_pad_bytes."""
    return -total % DMA_BLOCK_BYTES


class PadDrain:
    """discimg::atapi::DmaPadDrain."""

    def __init__(self):
        self.last, self.same = 0, 0

    def poll(self, level, pad_bytes):
        if level == 0 or level * 2 != pad_bytes:
            self.same = 0
            return False
        self.same = self.same + 1 if self.same and level == self.last else 1
        self.last = level
        return self.same >= DMA_PAD_SETTLE_READS


def exact_words(nbytes):
    return [(i * 0x9E37 + 0x4321) & 0xFFFF or 1 for i in range(nbytes // 2)]


def dma_read_exact(chunks, model, pad=True, host_bytes=None, fast=True,
                   sd_latency_ns=20_000, late_pad_ns=0, host_start_ns=0,
                   pause_last_ns=0, pause_in_last=None, abort=False, next_cmd=False,
                   cap=FW_CAP_POLLS, floor=FW_MIN_WAIT_POLLS, expect="done"):
    """A DMA read of sum(chunks) bytes against one host model.

    `chunks` are the firmware's staging rounds, `host_bytes` the length the
    host programmed (default: the transfer), `fast` the `fast-complete` feature.
    `late_pad_ns` delays the pad, `host_start_ns` the host's first burst,
    `pause_last_ns` its last. `pause_in_last` = (words, ns) pauses a full host
    that many words into its last burst. `abort` writes a new command while the
    pad is in the FIFO; `next_cmd` runs a second read after the completion.
    `cap` and `floor` are the drain loop's limits in polls. `expect`:
      "done"  all data delivered, clean completion;
      "stall" no pad: the tail is never granted, the poll cap ends the command;
      "short" the host stops early and the device still completes;
      "cut"   the known limit of DmaPadDrain: a full host paused at exactly the
              pad level has DMA turned off under it, with all its data in.
    """
    assert model in ("full", "exact")
    total = sum(chunks)
    want = exact_words(total)
    host_len = total if host_bytes is None else host_bytes
    pad_bytes = dma_pad_bytes(total) if pad else 0
    if model == "full":
        host_words = (host_len + DMA_BLOCK_BYTES - 1) // DMA_BLOCK_BYTES * HOLLY_BURST_WORDS
    else:
        host_words = host_len // 2
    if abort:
        # Stop inside the pad: all data taken, and for "full" two pad words.
        host_words = total // 2 + (2 if model == "full" else 0)
    second = pattern(7)

    async def host_tb(ctx, top, res):
        host = Host(ctx, top)
        host.dev = top.core.dev0
        core = top.core
        await host.reset()
        await host.delay(2000)
        await host.pio_write(CSN_CMD, ADDR_DEVICE, 0x00)
        await host.pio_write(CSN_CMD, ADDR_COMMAND, 0xC8)
        got, stalled, paused = [], False, False
        waited_first = False
        while len(got) < host_words:
            waited = 0
            while not ctx.get(core.io_IDE_DMARQ):
                if ctx.get(core.io_IDE_INTRQ) or waited > HOST_GIVES_UP_NS:
                    stalled = True
                    break
                await host.delay(20)
                waited += 20
            if stalled:
                break
            if not waited_first:
                waited_first = True
                await host.delay(host_start_ns)
            if pause_last_ns and not paused and host_words - len(got) <= HOLLY_BURST_WORDS:
                paused = True
                await host.delay(pause_last_ns)
            await host.delay(host.t["tLR"])
            if pause_in_last and host_words - len(got) == HOLLY_BURST_WORDS:
                k, ns = pause_in_last
                got += [v & 0xFFFF for v in await host.dma_read_burst(k)]
                await host.delay(ns)
                if not ctx.get(core.io_IDE_DMARQ):
                    res["cut_off"] = True
                    break
                burst = await host.dma_read_burst(HOLLY_BURST_WORDS - k)
            else:
                burst = await host.dma_read_burst(min(HOLLY_BURST_WORDS, host_words - len(got)))
            if burst is None:
                stalled = True
                break
            got += [v & 0xFFFF for v in burst]
            await host.delay(100)
        res["got"], res["stalled"] = got, stalled
        res["aborted_bursts"] = host.aborted_bursts
        if abort:
            await host.delay(300)
            await host.pio_write(CSN_CMD, ADDR_COMMAND, 0xC8)
            await host.delay(FW_POLL_NS * 6)
            res["intrq"] = bool(ctx.get(core.io_IDE_INTRQ))
            res["dmarq_after"] = ctx.get(core.io_IDE_DMARQ)
            res["level_after"] = ctx.get(core.dev0.dbg_rfifo_level)
            res["status"] = (await host.pio_read(CSN_CTRL_ALT, ADDR_ALT_STATUS)) & 0xFF
            return
        latency = 0
        while not ctx.get(core.io_IDE_INTRQ) and latency < HOST_GIVES_UP_NS:
            await host.delay(200)
            latency += 200
        res["latency_ns"] = latency
        res["intrq"] = bool(ctx.get(core.io_IDE_INTRQ))
        res["status"] = (await host.pio_read(CSN_CMD, ADDR_STATUS)) & 0xFF
        await host.delay(500)
        res["dmarq_after"] = ctx.get(core.io_IDE_DMARQ)
        res["level_after"] = ctx.get(core.dev0.dbg_rfifo_level)
        if next_cmd:
            await host.delay(FW_POLL_NS)
            res["dmarq_idle"] = ctx.get(core.io_IDE_DMARQ)
            await host.pio_write(CSN_CMD, ADDR_COMMAND, 0xC8)
            res["got2"] = [v & 0xFFFF for v in await host.dma_read(len(second))]

    async def mcu_tb(ctx, top, res):
        mcu = Mcu(ctx, top, dev=0)
        depth = top.core.dev0.read_fifo_depth

        async def take():
            await mcu_wait_command(mcu, timeout_ns=10_000_000)
            return (await mcu.read(MCU_HW_INFO)) & 0xFF00

        def posted(val, token):
            return val | (token & 0x8000) | ((token << 1) & 0x7E00)

        async def is_aborted():
            hw = await mcu.read(MCU_HW_INFO)
            return bool(hw & 0x2) and bool((await mcu.read(MCU_CMD_FEAT)) & 0xFF)

        async def push_paced(words):
            i = idle = 0
            while i < len(words):
                if await is_aborted():
                    return False
                room = max(0, depth - (await mcu.read(MCU_RFIFO_LEVEL) + 4))
                if room == 0:
                    await mcu.delay(1000)
                    idle += 1000
                    assert idle < HOST_GIVES_UP_NS, "host stopped draining during the push"
                    continue
                idle = 0
                n = min(room, len(words) - i)
                await mcu.push(words[i:i + n])
                i += n
            return True

        await mcu.delay(800)
        await mcu.write(MCU_STATUS, 0x50)
        token = await take()
        first = want[:chunks[0] // 2]
        primed = min(depth - 8, len(first))
        await mcu.push(first[:primed])
        level_after_prime = await mcu.read(MCU_RFIFO_LEVEL)
        await mcu.write(MCU_STATUS, posted(0xD0, token))         # BSY|DRDY|DSC
        await mcu.write(MCU_DMA_MODE, XFER_MASK | 1)
        await mcu.write(MCU_HW_INFO, token)
        pushed_all = await push_paced(first[primed:])
        off = len(first)
        for c in chunks[1:]:
            await mcu.delay(sd_latency_ns)
            pushed_all = pushed_all and await push_paced(want[off:off + c // 2])
            off += c // 2
        if late_pad_ns:
            await mcu.delay(late_pad_ns)
        pad_pushed = 0
        if pushed_all and pad_bytes:
            pushed_all = await push_paced([0] * (pad_bytes // 2))
            pad_pushed = pad_bytes if pushed_all else 0

        level = await mcu.read(MCU_RFIFO_LEVEL)
        took_all = pushed_all and primed > 0 and level == 0
        min_wait = 0 if took_all else floor
        peak, pulled, zeros = level_after_prime, False, 0
        drain = PadDrain()
        polls = cap
        first_gone = None
        for i in range(cap):
            if await is_aborted():
                polls = i
                break
            level = await mcu.read(MCU_RFIFO_LEVEL)
            if first_gone is None and level * 2 <= pad_pushed:
                first_gone = i
            if level > peak:
                peak = level
            elif level < peak:
                pulled = True
            zeros = zeros + 1 if level == 0 else 0
            waived = fast and pulled and zeros >= 2
            # The higher of two reads, as the firmware.
            pad_level = max(level, await mcu.read(MCU_RFIFO_LEVEL)) if pad_pushed else level
            if (i >= min_wait and level == 0) or waived or drain.poll(pad_level, pad_pushed):
                polls = i
                break
            await mcu.delay(FW_POLL_NS)
        res["polls"] = polls
        res["polls_after_data"] = polls - first_gone if first_gone is not None else None
        if await is_aborted():
            res["mcu_aborted"] = True
            await mcu.write(MCU_DMA_MODE, XFER_MASK)
            return
        res["mcu_aborted"] = False
        leftover = await mcu.read(MCU_RFIFO_LEVEL)
        res["left_words"] = leftover
        res["delivered"] = min(total, max(0, total + pad_pushed - leftover * 2))
        await mcu.write(MCU_DMA_MODE, XFER_MASK)
        await mcu.write(MCU_STATUS, posted(0x50 | 0x0100, token))
        if next_cmd:
            token = await take()
            res["level_at_next"] = await mcu.read(MCU_RFIFO_LEVEL)
            await mcu.push(second)
            await mcu.write(MCU_STATUS, posted(0xD0, token))
            await mcu.write(MCU_DMA_MODE, XFER_MASK | 1)
            await mcu.write(MCU_HW_INFO, token)

    def check(res):
        got = res["got"]
        n = min(len(got), len(want))
        problem = classify(got[:n], want[:n])
        assert problem is None, problem
        assert res["aborted_bursts"] == 0, "DMARQ negated inside a burst"
        assert not res["dmarq_after"], "DMARQ still up after the command ended"
        assert res["level_after"] == 0 or not abort, "abort left words in the FIFO"
        if abort:
            print(f"        {model}: {total} B, abort in the pad: host got {len(got) * 2} B, "
                  f"polls={res['polls']} level={res['level_after']} status={res['status']:#04x}")
            assert res["mcu_aborted"], "the MCU did not see the abort"
            assert not res["intrq"], "completion posted over the new command"
            assert res["status"] & 0x80, "the new command must keep BSY"
            return
        print(f"        {model}{'' if fast else ' (no fast-complete)'}: {total} B (host {host_len}, "
              f"pad {pad_bytes}): host got {len(got) * 2} B, FIFO left {res['left_words']} words, "
              f"delivered {res['delivered']}, polls {res['polls']} "
              f"({res['polls_after_data']} after the data), status {res['status']:#04x}")
        assert res["intrq"] and res["status"] == 0x50, "no clean completion"
        assert bool(res.get("cut_off")) == (expect == "cut"), "cut-off not as expected"
        if expect == "cut":
            assert len(got) == total // 2, "the host must have all its data"
            assert res["delivered"] == total
            assert res["left_words"] == pad_bytes // 2
            assert res["polls"] < cap, "ran to the drain cap"
        elif expect == "done":
            assert not res["stalled"], "host never got its data"
            assert len(got) == host_words
            assert res["delivered"] == total
            assert res["left_words"] == (0 if model == "full" else pad_bytes // 2)
            assert all(w == 0 for w in got[len(want):]), "pad is not zero"
            assert res["polls"] < cap, "ran to the drain cap"
            if model == "exact" and pad_bytes:
                assert res["polls_after_data"] == DMA_PAD_SETTLE_READS - 1, "pad settle count"
        elif expect == "stall":
            assert res["stalled"], "expected the tail to starve"
            assert len(got) == total // DMA_BLOCK_BYTES * HOLLY_BURST_WORDS
            assert res["left_words"] == (total % DMA_BLOCK_BYTES) // 2
            assert res["polls"] == cap
        else:
            assert not res["stalled"] and len(got) == host_words
            assert res["left_words"] == (total + pad_bytes - host_len) // 2
            assert res["delivered"] == host_len
            assert res["polls"] == cap, "must not end early with data queued"
        if next_cmd:
            assert not res["dmarq_idle"], "DMARQ up between commands"
            assert res["level_at_next"] == 0, "pad survived the next Command write"
            problem = classify(res["got2"], second)
            assert problem is None, f"next command: {problem}"

    return host_tb, mcu_tb, check


def oi04_cases():
    # 304 B: 9.5 blocks, pad 16, as the 2,352 B sector ends, at an eighth of
    # the simulation time. Where the host model cannot matter a case runs once.
    small = dict(chunks=[304], cap=100, floor=50)
    cases = [
        # The tail is never granted, so neither host gets past the last whole block.
        ("dma_exact_full_unpadded_stalls",
         dma_read_exact(model="full", pad=False, expect="stall", **small)),
        # The 100 ms floor; DmaPadDrain does not depend on fast-complete.
        ("dma_exact_full_no_fast", dma_read_exact(model="full", fast=False, **small)),
        # Both hosts stop at the programmed length here.
        ("dma_exact_exact_floor",
         dma_read_exact(model="exact", host_bytes=288, expect="short", **small)),
        ("dma_exact_exact_16464_two_chunks", dma_read_exact([5 * 2352, 2 * 2352], "exact")),
        ("dma_exact_exact_mixed_2324_2048", dma_read_exact([2324 + 2048], "exact")),
        ("dma_exact_exact_4704_no_pad", dma_read_exact([4704], "exact")),
        # Paused 12 words into the last burst (level 4): must not be cut off.
        ("dma_exact_full_pause_inside_last_block",
         dma_read_exact(model="full", pause_in_last=(12, 60_000), **small)),
        # Paused 8 words in (level 8 = the pad): the known limit, recorded.
        ("dma_exact_full_pause_at_pad_level_is_cut",
         dma_read_exact(model="full", pause_in_last=(8, 60_000), expect="cut", **small)),
    ]
    for m in ("full", "exact"):
        cases += [
            (f"dma_exact_{m}_2352", dma_read_exact([2352], m)),
            (f"dma_exact_{m}_2324", dma_read_exact([2324], m)),
            (f"dma_exact_{m}_late_pad", dma_read_exact(model=m, late_pad_ns=40_000, **small)),
            (f"dma_exact_{m}_slow_host", dma_read_exact(model=m, host_start_ns=60_000, **small)),
            (f"dma_exact_{m}_pause_before_last",
             dma_read_exact(model=m, pause_last_ns=60_000, **small)),
            (f"dma_exact_{m}_abort_in_pad", dma_read_exact(model=m, abort=True, **small)),
            (f"dma_exact_{m}_next_command", dma_read_exact(model=m, next_cmd=True, **small)),
        ]
    return cases


TESTBENCHES = [
    ("dma_read_prestaged", dma_read_prestaged()),
    ("dma_read_prestaged_noabort", dma_read_prestaged(abort=False)),
    ("dma_read_refill_race_t0_120", dma_read_refill_race(t0=120)),
    ("dma_read_refill_race_t0_100", dma_read_refill_race(t0=100)),
    ("dma_read_refill_race_t0_80", dma_read_refill_race(t0=80)),
] + oi04_cases()


def run_one(name, test):
    host_tb, mcu_tb, check = test
    top = SimTop()
    sim = Simulator(top)
    sim.add_clock(CORE_PERIOD)
    res = {}

    async def host_wrapper(ctx):
        await host_tb(ctx, top, res)

    async def mcu_wrapper(ctx):
        await mcu_tb(ctx, top, res)

    sim.add_testbench(host_wrapper)
    sim.add_testbench(mcu_wrapper)
    os.makedirs("vcd", exist_ok=True)
    if name.startswith("dma_exact_"):      # many long, idle-heavy runs
        sim.run()
    else:
        with sim.write_vcd(f"vcd/timing_{name}.vcd"):
            sim.run()
    check(res)


def main() -> None:
    # Which FIFO memory style is under test: set NETGD_FPGA_TARGET=ag16k
    # to run the same suite against the block-RAM build (hw/fifo.py).
    print(f"FPGA target: {resolve_target()}")
    failures = 0
    for name, tb in TESTBENCHES:
        try:
            run_one(name, tb)
            print(f"  ok    {name}")
        except AssertionError as e:
            print(f"  FAIL  {name}: {e}")
            failures += 1
    print()
    print(f"all {len(TESTBENCHES)} test(s) passed" if not failures
          else f"{failures} test(s) failed")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
