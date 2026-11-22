"""Pin-finder core for the AG16K on the GDEMU 6.0 board.

`docs/design/g1-ag16k-port.md` 2-4, method 2. The board's FPGA ball <-> MCU
pin / GD-ROM connector wiring is unknown; this core makes each candidate ball
say its own number, one ball at a time, so a receiver on the other end of the
trace (the MCU firmware's `pinfind` feature, or a logic analyser) can tell
which ball it is wired to.

What the core does, per candidate ball `k` (in the order of the ball list):

  1. LISTEN  -- `listen` ticks with every output off, watching ball k. With
                `skip_if_low` (the default), a single low sample skips the
                ball: something other than a pull-up is holding or toggling
                it (a clock buffer, a driven bus line, a ball tied to GND),
                and driving it would be a fight.
  2. FRAME   -- `frames` frames carrying index k (line code below), with
                ball k's output enable on and every other ball off.
  3. NEXT    -- output off, k + 1. After the last ball, one loop is done;
                after `max_loops` loops (0 = never) the core stops for good
                with every output off (DONE).

The line code is the one `common/pinfind` decodes (format 1): ticks of
`2^prescale` clocks; SYNC = active 8T + idle 1T; each bit active 1T (0) or
3T (1) + idle 1T; MSB first; idle >= 4T after the last bit. Payload: `idx`,
`~idx`, and with clock probes `hdr = div_bits << 4 | win_bits`, a 16-bit
count per probe, and an 8-bit sum. "Active" is `pad_o = 0`; the wrapper
decides whether idle is driven high (push-pull) or released (open-drain).

At most one bit of `pad_oe` is ever set (checked in test_pinfind.py), and it
is clear outside FRAME.

Clock probes: each `probe[c]` (a dedicated clock-input ball, which cannot be
driven and so cannot be found by the scan) clocks a `div_bits`-bit counter in
a domain of its own. Its top bit is brought into `sync` and its toggles are
counted over windows of `2^win_bits` ticks; each frame carries the last
window's counts (16 bits, saturating). The receiver turns them into Hz with
its own measure of T (`pinfind::clock_hz`), which is how the 48 MHz and
11.2896 MHz balls are found.

Every register resets / powers up to 0 (the first state is START), so the
design does not depend on how the AGM flow handles non-zero init values.
"""

from amaranth import *
from amaranth.lib.cdc import FFSynchronizer


# Line code, in ticks. Must agree with common/pinfind/src/lib.rs.
SYNC_ACTIVE = 8
IDLE = 1
BIT0_ACTIVE = 1
BIT1_ACTIVE = 3
GAP = 4
MAX_PROBES = 12

# FSM states. START must be 0 (power-up value).
START, LISTEN, FRAME, NEXT, DONE = range(5)
# Symbol phases inside FRAME.
PH_SYNC_A, PH_SYNC_I, PH_BIT_A, PH_BIT_I = range(4)


def frame_bytes(idx, counts=None, div_bits=10, win_bits=6):
    """Reference payload (Python), for the tests."""
    b = [idx & 0xFF, (~idx) & 0xFF]
    if counts is not None:
        b.append(((div_bits & 0xF) << 4) | (win_bits & 0xF))
        for c in counts:
            b += [(c >> 8) & 0xFF, c & 0xFF]
        b.append(sum(b) & 0xFF)
    return b


class PinFindCore(Elaboratable):
    def __init__(self, n_drive, n_probe=0, *, prescale=8, frames=3, listen=16,
                 start=64, max_loops=8, skip_if_low=True, div_bits=10, win_bits=6):
        if not 1 <= n_drive <= 256:
            raise ValueError("n_drive must be 1..256 (the index is one byte)")
        if not 0 <= n_probe <= MAX_PROBES:
            raise ValueError(f"n_probe must be 0..{MAX_PROBES}")
        if not (1 <= div_bits <= 15 and 0 <= win_bits <= 15):
            raise ValueError("div_bits 1..15, win_bits 0..15 (they share one header byte)")
        if frames < 1 or listen < 1 or start < 1 or prescale < 0:
            raise ValueError("frames, listen, start >= 1; prescale >= 0")
        self.n_drive = n_drive
        self.n_probe = n_probe
        self.prescale = prescale
        self.frames = frames
        self.listen = listen
        self.start = start
        self.max_loops = max_loops
        self.skip_if_low = skip_if_low
        self.div_bits = div_bits
        self.win_bits = win_bits

        # Pads (asynchronous inputs; synchronised inside).
        self.pad_i = Signal(n_drive)
        self.pad_oe = Signal(n_drive)
        self.pad_o = Signal()
        self.probe = Signal(max(n_probe, 1))
        # Status, for simulation (not wired to a pad).
        self.done = Signal()
        self.ball = Signal(range(n_drive))
        self.state = Signal(3)
        self.skipped = Signal()     # pulses in NEXT when the ball was skipped

    def ports(self):
        p = [self.pad_i, self.pad_oe, self.pad_o]
        if self.n_probe:
            p.append(self.probe)
        return p

    def elaborate(self, platform):
        m = Module()
        n = self.n_drive

        # ---- tick -------------------------------------------------------
        tick = Signal()
        if self.prescale == 0:
            m.d.comb += tick.eq(1)
        else:
            pre = Signal(self.prescale)
            m.d.sync += pre.eq(pre + 1)
            m.d.comb += tick.eq(pre == (1 << self.prescale) - 1)

        # ---- inputs -----------------------------------------------------
        pad_s = Signal(n)
        m.submodules.pad_sync = FFSynchronizer(self.pad_i, pad_s)

        # ---- clock probes -----------------------------------------------
        counts = []
        if self.n_probe:
            wcnt = Signal(self.win_bits) if self.win_bits else None
            win_end = Signal()
            if wcnt is None:
                m.d.comb += win_end.eq(tick)
            else:
                with m.If(tick):
                    m.d.sync += wcnt.eq(wcnt + 1)
                m.d.comb += win_end.eq(tick & (wcnt == (1 << self.win_bits) - 1))
            for c in range(self.n_probe):
                cd = ClockDomain(f"probe{c}", reset_less=True)
                m.domains += cd
                m.d.comb += cd.clk.eq(self.probe[c])
                div = Signal(self.div_bits, name=f"probe{c}_div")
                m.d[f"probe{c}"] += div.eq(div + 1)
                msb_s = Signal(name=f"probe{c}_msb_s")
                m.submodules[f"probe{c}_sync"] = FFSynchronizer(div[-1], msb_s)
                msb_d = Signal(name=f"probe{c}_msb_d")
                acc = Signal(16, name=f"probe{c}_acc")
                cnt = Signal(16, name=f"probe{c}_cnt")
                toggle = Signal(name=f"probe{c}_toggle")
                m.d.comb += toggle.eq(msb_s ^ msb_d)
                m.d.sync += msb_d.eq(msb_s)
                with m.If(win_end):
                    m.d.sync += [cnt.eq(acc), acc.eq(toggle)]
                with m.Elif(toggle & (acc != 0xFFFF)):
                    m.d.sync += acc.eq(acc + 1)
                counts.append(cnt)

        # ---- payload ----------------------------------------------------
        idx = Signal(8)
        m.d.comb += idx.eq(self.ball)
        payload = [idx, ~idx]
        if self.n_probe:
            payload.append(Const((self.div_bits << 4) | self.win_bits, 8))
            for cnt in counts:
                payload += [cnt[8:16], cnt[0:8]]
            csum = Signal(8)
            m.d.comb += csum.eq(sum(payload))      # truncates to 8 bits
            payload.append(csum)
        nbits = 8 * len(payload)
        data = Signal(nbits)
        # Cat is LSB first: the first byte goes on top, MSB is sent first.
        m.d.comb += data.eq(Cat(*reversed(payload)))

        # ---- sequencer --------------------------------------------------
        state = self.state
        ball = self.ball
        left = Signal(range(max(self.start, self.listen, SYNC_ACTIVE, GAP, BIT1_ACTIVE) + 1))
        phase = Signal(2)
        sr = Signal(nbits)
        bits_left = Signal(range(nbits + 1))
        frame_no = Signal(range(self.frames + 1))
        loops = Signal(range(max(self.max_loops, 1) + 1))
        skip = Signal()
        level = Signal()        # 1 = idle, 0 = active

        m.d.comb += [
            self.pad_o.eq(level),
            self.pad_oe.eq(Mux(state == FRAME, Const(1, n) << ball, 0)),
            self.done.eq(state == DONE),
        ]

        def start_frame():
            return [sr.eq(data), bits_left.eq(nbits), phase.eq(PH_SYNC_A),
                    left.eq(SYNC_ACTIVE)]

        with m.Switch(state):
            with m.Case(START):
                m.d.sync += level.eq(1)
                with m.If(tick):
                    with m.If(left == self.start - 1):
                        m.d.sync += [state.eq(LISTEN), left.eq(0), skip.eq(0)]
                    with m.Else():
                        m.d.sync += left.eq(left + 1)

            with m.Case(LISTEN):
                if self.skip_if_low:
                    with m.If(~pad_s.bit_select(ball, 1)):
                        m.d.sync += skip.eq(1)
                with m.If(tick):
                    with m.If(left == self.listen - 1):
                        with m.If(skip):
                            m.d.sync += state.eq(NEXT)
                        with m.Else():
                            m.d.sync += [state.eq(FRAME), frame_no.eq(0), level.eq(0)]
                            m.d.sync += start_frame()
                    with m.Else():
                        m.d.sync += left.eq(left + 1)

            with m.Case(FRAME):
                with m.If(tick):
                    with m.If(left > 1):
                        m.d.sync += left.eq(left - 1)
                    with m.Else():
                        with m.Switch(phase):
                            with m.Case(PH_SYNC_A):
                                m.d.sync += [phase.eq(PH_SYNC_I), left.eq(IDLE), level.eq(1)]
                            with m.Case(PH_SYNC_I, PH_BIT_I):
                                with m.If(bits_left == 0):
                                    # End of the closing gap.
                                    with m.If(frame_no == self.frames - 1):
                                        m.d.sync += [state.eq(NEXT), level.eq(1)]
                                    with m.Else():
                                        m.d.sync += [frame_no.eq(frame_no + 1), level.eq(0)]
                                        m.d.sync += start_frame()
                                with m.Else():
                                    m.d.sync += [
                                        phase.eq(PH_BIT_A), level.eq(0),
                                        left.eq(Mux(sr[-1], BIT1_ACTIVE, BIT0_ACTIVE)),
                                        sr.eq(sr << 1),
                                        bits_left.eq(bits_left - 1),
                                    ]
                            with m.Case(PH_BIT_A):
                                m.d.sync += [
                                    phase.eq(PH_BIT_I), level.eq(1),
                                    left.eq(Mux(bits_left == 0, GAP, IDLE)),
                                ]

            with m.Case(NEXT):
                m.d.comb += self.skipped.eq(skip)
                m.d.sync += [left.eq(0), skip.eq(0), level.eq(1), state.eq(LISTEN)]
                with m.If(ball == n - 1):
                    m.d.sync += ball.eq(0)
                    if self.max_loops:
                        with m.If(loops == self.max_loops - 1):
                            m.d.sync += state.eq(DONE)
                        with m.Else():
                            m.d.sync += loops.eq(loops + 1)
                with m.Else():
                    m.d.sync += ball.eq(ball + 1)

            with m.Case(DONE):
                m.d.sync += level.eq(1)

        return m
