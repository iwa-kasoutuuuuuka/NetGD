"""Data FIFOs, in two flavours with the same cycle-exact behaviour.

`target` picks the memory style:

  "cyclone2"  (default) amaranth's `SyncFIFO` behind a `ResetInserter`:
              asynchronous-read storage. Quartus re-times that into M4K
              block RAM on the v5.5 board's Cyclone II; this is the path
              the committed out/OpenGDEMUCore.v and bitstream come from and
              it must not change.

  "ag16k"     `SyncReadFIFO` below: the same first-word-fall-through FIFO
              on a memory with a *registered* read, which is the only kind
              an M9K-style block RAM (AGM AG16K, Cyclone IV) can implement
              and the only kind open Yosys maps to block RAM. See
              docs/design/g1-ag16k-port.md section 2-3.

The default comes from the NETGD_FPGA_TARGET environment variable so the
simulation suites can run against either flavour unchanged; main.py always
passes the target explicitly.
"""

import os

from amaranth import *
from amaranth.hdl.xfrm import ResetInserter
from amaranth.lib.fifo import SyncFIFO
from amaranth.lib.memory import Memory


TARGETS = ("cyclone2", "ag16k")
TARGET_ENV = "NETGD_FPGA_TARGET"


def resolve_target(target: str | None = None) -> str:
    """`target` if given, else $NETGD_FPGA_TARGET, else "cyclone2"."""
    if target is None:
        target = os.environ.get(TARGET_ENV) or "cyclone2"
    if target not in TARGETS:
        raise ValueError(f"unknown FPGA target {target!r}; expected one of {TARGETS}")
    return target


class SyncReadFIFO(Elaboratable):
    """Drop-in for `ResetInserter(clear)(SyncFIFO(width, depth))` whose
    storage is read synchronously.

    Same ports (w_data/w_en/w_rdy, r_data/r_en/r_rdy, level) and the same
    value on every one of them in every cycle, including `r_data` while the
    FIFO is empty (the stale word at the read pointer) and across `clear`.

    How: `SyncFIFO` reads `storage[consume]` combinationally. Here the read
    port is addressed one cycle early with the value `consume` is about to
    take -- `consume + 1` on a pop, 0 on a clear, `consume` otherwise -- so
    its output register holds `storage[consume]` exactly when the
    asynchronous version would show it. The one case a registered read
    cannot see is a word written in that same early cycle to that same
    address (a push into an empty FIFO, or a push alongside the pop of the
    last word); a one-word bypass register covers it. The bypass is written
    out here rather than left to the memory's transparency option so the
    RAM itself is the plain "read old data" kind every block RAM offers
    and no tool has to synthesise it.

    Nothing between the pointers and the consumers gains or loses a cycle,
    so latency, level counts and the DMARQ/IORDY/DRQ handshakes built on
    `level`, `r_rdy` and `w_rdy` are untouched.
    """

    def __init__(self, *, width: int, depth: int):
        self.width = width
        self.depth = depth

        self.w_data = Signal(width)
        self.w_rdy = Signal()
        self.w_en = Signal()
        self.r_data = Signal(width)
        self.r_rdy = Signal()
        self.r_en = Signal()
        self.level = Signal(range(depth + 1))
        # Synchronous clear: empties the FIFO at the next edge, exactly as
        # ResetInserter does for SyncFIFO. Storage contents are kept.
        self.clear = Signal()

    def elaborate(self, platform):
        m = Module()
        depth = self.depth

        def incr(value):
            if depth & (depth - 1) == 0:
                return (value + 1)[:len(value)]
            return Mux(value == depth - 1, 0, value + 1)

        m.d.comb += [
            self.w_rdy.eq(self.level != depth),
            self.r_rdy.eq(self.level != 0),
        ]
        do_read = self.r_rdy & self.r_en
        do_write = self.w_rdy & self.w_en

        m.submodules.storage = storage = Memory(
            shape=self.width, depth=depth, init=[])
        w_port = storage.write_port()
        # Not transparent: on a same-address collision the port returns the
        # old word and the bypass below supplies the new one.
        r_port = storage.read_port(domain="sync", transparent_for=())

        produce = Signal(range(depth))
        consume = Signal(range(depth))
        consume_next = Signal(range(depth))

        # The domain reset has to steer the read address too, or the output
        # register would come out of reset holding storage[old consume].
        clear = Signal()
        m.d.comb += clear.eq(self.clear | ResetSignal("sync"))

        m.d.comb += [
            w_port.addr.eq(produce),
            w_port.data.eq(self.w_data),
            w_port.en.eq(do_write),
        ]
        with m.If(do_read):
            m.d.comb += consume_next.eq(incr(consume))
        with m.Else():
            m.d.comb += consume_next.eq(consume)
        with m.If(clear):
            m.d.comb += consume_next.eq(0)
        m.d.comb += [
            r_port.addr.eq(consume_next),
            r_port.en.eq(1),
        ]

        bypass = Signal()
        bypass_data = Signal(self.width)
        m.d.sync += [
            bypass.eq(do_write & (produce == consume_next)),
            bypass_data.eq(self.w_data),
        ]
        m.d.comb += self.r_data.eq(Mux(bypass, bypass_data, r_port.data))

        m.d.sync += consume.eq(consume_next)
        with m.If(do_write):
            m.d.sync += produce.eq(incr(produce))
        with m.If(do_write & ~do_read):
            m.d.sync += self.level.eq(self.level + 1)
        with m.If(do_read & ~do_write):
            m.d.sync += self.level.eq(self.level - 1)
        with m.If(self.clear):
            m.d.sync += [produce.eq(0), self.level.eq(0)]

        return m


def make_fifo(m: Module, name: str, *, width: int, depth: int,
              clear: Signal, target: str):
    """Add a data FIFO to `m` as submodule `name` and return it.

    `clear` empties it synchronously. For "cyclone2" this is exactly the
    construction the design has always used.
    """
    if target == "cyclone2":
        fifo = ResetInserter(clear)(SyncFIFO(width=width, depth=depth))
        m.submodules[name] = fifo
        return fifo
    fifo = SyncReadFIFO(width=width, depth=depth)
    m.submodules[name] = fifo
    m.d.comb += fifo.clear.eq(clear)
    return fifo
