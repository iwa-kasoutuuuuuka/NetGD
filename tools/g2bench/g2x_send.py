"""g2x: the PC side of the custom UDP transfer (g2x_core.h, docs/research/g2-transport.md).

The PC streams 1,400-byte DATA datagrams (20-byte header, 1,380 data bytes, CRC-32 per datagram) to the
Dreamcast's UDP port, paced at a fixed or adaptive rate, and recovers losses from the Dreamcast's ACKs
(cumulative + 63-bit selective bitmap + credit limit). Same algorithm as g2x_tx_* in g2x_core.c:

* a hole below a datagram that was sent after it is lost (the BBA RX ring is a FIFO: nothing is reordered),
  so it is resent at once (no timer);
* nothing acknowledged for RTO (30 ms): resend everything in flight (tail loss);
* adaptive rate: +3 pps per ACK with progress, x0.85 per loss event (at most one per 20 ms).

Rx is a Python port of the receiver (g2x_rx_*), used by the tests as the fake Dreamcast.
Standard library only."""
import socket
import struct
import time
import zlib
from collections import deque

HDR = 20
PAYLOAD = 1380
DGRAM = HDR + PAYLOAD
ACK_LEN = 40
WIN = 64
T_DATA, T_ACK = 1, 2
F_RESEND, F_CRC = 0x01, 0x02
A_DONE, A_STOP = 0x01, 0x02
RTO_US = 30000
CUT_GAP_US = 20000
ACK_EVERY = 16
ACK_GAP_US = 1000
ACK_IDLE_US = 4000

_H = struct.Struct("<4sBBHIHH")      # magic, type, flags, pass, seq, len, 0  (16 bytes; then u32 crc)
_A = struct.Struct("<4sBBHIIIIIIII")  # 40 bytes


def count_for(total):
    return (total + PAYLOAD - 1) // PAYLOAD


def len_for(total, seq):
    off = seq * PAYLOAD
    return 0 if off >= total else min(PAYLOAD, total - off)


def build_data(pass_no, seq, flags, data, with_crc=True):
    flags = (flags & ~F_CRC) | (F_CRC if with_crc else 0)
    h = _H.pack(b"G2X1", T_DATA, flags, pass_no, seq, len(data), 0)
    crc = zlib.crc32(data, zlib.crc32(h)) & 0xFFFFFFFF if with_crc else 0
    return h + struct.pack("<I", crc) + bytes(data)


def parse_data(d):
    """-> (flags, pass, seq, data) or None (not G2X / bad length) or 'crc' (CRC wrong)"""
    if len(d) < HDR or d[:4] != b"G2X1" or d[4] != T_DATA:
        return None
    _m, _t, flags, pass_no, seq, n, z = _H.unpack_from(d)
    crc, = struct.unpack_from("<I", d, 16)
    if z or n > PAYLOAD or n + HDR != len(d):
        return None
    data = d[HDR:]
    if flags & F_CRC and zlib.crc32(data, zlib.crc32(d[:16])) & 0xFFFFFFFF != crc:
        return "crc"
    return flags, pass_no, seq, data


class Ack(object):
    __slots__ = ("flags", "pass_no", "cum", "limit", "sack_lo", "sack_hi", "ack_no", "dgrams", "dups", "bad")

    def __init__(self, flags=0, pass_no=0, cum=0, limit=0, sack_lo=0, sack_hi=0, ack_no=0, dgrams=0, dups=0, bad=0):
        self.flags, self.pass_no, self.cum, self.limit = flags, pass_no, cum, limit
        self.sack_lo, self.sack_hi, self.ack_no = sack_lo, sack_hi, ack_no
        self.dgrams, self.dups, self.bad = dgrams, dups, bad

    def pack(self):
        return _A.pack(b"G2XA", T_ACK, self.flags, self.pass_no, self.cum, self.limit, self.sack_lo, self.sack_hi,
                       self.ack_no, self.dgrams, self.dups, self.bad)

    @classmethod
    def parse(cls, d):
        if len(d) < ACK_LEN or d[:4] != b"G2XA" or d[4] != T_ACK:
            return None
        v = _A.unpack_from(d)
        return cls(*v[2:])


class Tx(object):
    """the sender's bookkeeping (g2x_tx_*): which seq to send next; times in microseconds"""

    def __init__(self, count, pps, pps_min=0.0, pps_max=0.0):
        self.count = count
        self.next_new = 0
        self.cum = 0
        self.limit = min(count, WIN)
        self.acked = bytearray(count)
        self.queued = bytearray(count)
        self.stamp = [0] * count
        self.tx_no = 0
        self.rq = deque()
        self.last_progress = 0
        self.pps, self.pps_min, self.pps_max = float(pps), float(pps_min), float(pps_max)
        self.adaptive = pps_max > pps_min
        self.last_cut = 0
        self.sent = self.resent = self.loss_events = self.rto_fires = 0

    def _enqueue(self, s):
        if not self.acked[s] and not self.queued[s] and len(self.rq) < WIN:
            self.rq.append(s)
            self.queued[s] = 1

    def _cut(self, now):
        if self.last_cut and now - self.last_cut < CUT_GAP_US:
            return
        self.last_cut = now
        self.loss_events += 1
        if self.adaptive:
            self.pps = max(self.pps_min, self.pps * 0.85)

    def on_ack(self, a, now):
        progress = False
        cum = min(a.cum, self.count)
        acked = self.acked
        while self.cum < cum:
            if not acked[self.cum]:
                acked[self.cum] = 1
                progress = True
            self.cum += 1
        high = -1
        bits = a.sack_lo | (a.sack_hi << 32)
        i = 0
        while bits and i < WIN - 1:
            if bits & 1:
                q = cum + 1 + i
                if q < self.count:
                    if not acked[q]:
                        acked[q] = 1
                        progress = True
                    high = q
            bits >>= 1
            i += 1
        while self.cum < self.count and acked[self.cum]:
            self.cum += 1
        if a.limit > self.limit:
            self.limit = min(a.limit, self.count)
        if progress:
            self.last_progress = now
        lost = False
        if high >= 0:
            hs = self.stamp[high]
            for q in range(self.cum, high):
                if not acked[q] and not self.queued[q] and self.stamp[q] and self.stamp[q] < hs:
                    self._enqueue(q)
                    lost = True
        if lost:
            self._cut(now)
        elif progress and self.adaptive:
            self.pps = min(self.pps_max, self.pps + 3.0)

    def next(self, now):
        """-> (seq, resend) or (None, False)"""
        while self.rq:
            s = self.rq.popleft()
            self.queued[s] = 0
            if not self.acked[s]:
                return s, True
        if self.cum < self.next_new and self.last_progress and now - self.last_progress > RTO_US:
            self.last_progress = now
            self.rto_fires += 1
            self._cut(now)
            for q in range(self.cum, self.next_new):
                self._enqueue(q)
            if self.rq:
                return self.next(now)
        if self.next_new < self.count and self.next_new < self.limit:
            return self.next_new, False
        return None, False

    def sent_one(self, seq, resend, now):
        self.tx_no += 1
        self.stamp[seq] = self.tx_no
        if resend:
            self.resent += 1
        else:
            self.next_new += 1
            self.sent += 1
        if not self.last_progress:
            self.last_progress = now or 1

    def done(self):
        return self.cum >= self.count


class Rx(object):
    """the receiver (g2x_rx_*), for tests: deliver(seq, data) is called in order"""

    def __init__(self, pass_no, total, deliver, need_crc=True):
        self.pass_no, self.total, self.count = pass_no, total, count_for(total)
        self.deliver, self.need_crc = deliver, need_crc
        self.cum = self.high = 0
        self.slots = {}
        self.since_ack = 0
        self.ack_no = 0
        self.gap_pending = False
        self.last_ack = 0
        self.dgrams = self.dups = self.bad = self.beyond = self.gaps = self.acks = 0

    def complete(self):
        return self.cum >= self.count

    def _due(self, now):
        return self.complete() or self.since_ack >= ACK_EVERY or (self.gap_pending and now - self.last_ack >= ACK_GAP_US)

    def input(self, d, now):
        """-> True when an ACK is due, False, or None (not ours)"""
        p = parse_data(d)
        if p is None:
            if len(d) >= 8 and d[:4] == b"G2X1" and struct.unpack_from("<H", d, 6)[0] == self.pass_no:
                self.bad += 1
                return False
            return None
        if p == "crc":
            if struct.unpack_from("<H", d, 6)[0] != self.pass_no:
                return None
            self.dgrams += 1
            self.bad += 1
            return self._due(now)
        flags, pass_no, seq, data = p
        if pass_no != self.pass_no:
            return None
        self.dgrams += 1
        if (self.need_crc and not flags & F_CRC) or seq >= self.count or len(data) != len_for(self.total, seq):
            self.bad += 1
            return self._due(now)
        if seq < self.cum:
            self.dups += 1
            self.gap_pending = True
        elif seq >= self.cum + WIN:
            self.beyond += 1
        elif seq == self.cum:
            self.deliver(seq, data)
            self.cum += 1
            self.since_ack += 1
            while self.cum < self.count and self.cum in self.slots:
                self.deliver(self.cum, self.slots.pop(self.cum))
                self.cum += 1
                self.since_ack += 1
            self.high = max(self.high, self.cum)
        elif seq in self.slots:
            self.dups += 1
            self.gap_pending = True
        else:
            self.slots[seq] = bytes(data)
            self.gap_pending = True
            if seq > self.high:
                self.gaps += 1
            if seq >= self.high:
                self.high = seq + 1
        return self._due(now)

    def poll(self, now):
        if not (self.since_ack or self.gap_pending or self.high > self.cum):
            return False
        if self.gap_pending and now - self.last_ack >= ACK_GAP_US:
            return True
        return now - self.last_ack >= ACK_IDLE_US

    def make_ack(self, flags, now):
        lo = hi = 0
        for i in range(WIN - 1):
            q = self.cum + 1 + i
            if q >= self.high:
                break
            if q in self.slots:
                if i < 32:
                    lo |= 1 << i
                else:
                    hi |= 1 << (i - 32)
        self.ack_no += 1
        self.acks += 1
        self.since_ack = 0
        self.gap_pending = False
        self.last_ack = now
        return Ack(flags | (A_DONE if self.complete() else 0), self.pass_no, self.cum, min(self.cum + WIN, self.count),
                   lo, hi, self.ack_no, self.dgrams, self.dups, self.bad)


def send_pass(host, port, pass_no, total, data, pps, pps_min=0, pps_max=0, crc=True, log=print, idle_s=5.0,
              max_s=120.0, sock=None, drop=None):
    """Streams data[:total] as G2X pass `pass_no` to host:port until the Dreamcast's ACK says DONE or STOP.
    `data` is a bytes-like object of at least `total` bytes. `drop(seq) -> bool` (tests) loses datagrams on purpose.
    Returns a dict of the sender's counters."""
    own = sock is None
    s = sock or socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        if own:
            try:
                s.setsockopt(socket.SOL_SOCKET, socket.SO_SNDBUF, 1 << 20)
                s.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 1 << 20)
            except OSError:
                pass
        s.setblocking(False)
        mv = memoryview(data)
        count = count_for(total)
        tx = Tx(count, pps if pps else 1500, pps_min, pps_max)
        clock = time.perf_counter
        t0 = clock()
        next_send = t0
        last_ack = t0
        end_flags, acks, bad_acks, stop_reason, pps_sum, pps_n = 0, 0, 0, "", 0.0, 0
        dest = (host, port)
        while True:
            now = clock()
            now_us = int((now - t0) * 1e6) + 1
            while True:
                try:
                    d, _a = s.recvfrom(256)
                except (BlockingIOError, InterruptedError):
                    break
                except ConnectionResetError:        # Windows: an ICMP port unreachable from an earlier send
                    continue
                a = Ack.parse(d)
                if a is None or a.pass_no != pass_no:
                    bad_acks += 1
                    continue
                acks += 1
                last_ack = now
                tx.on_ack(a, now_us)
                pps_sum += tx.pps
                pps_n += 1
                if a.flags & (A_DONE | A_STOP):
                    end_flags = a.flags
            if end_flags:
                stop_reason = "done" if end_flags & A_DONE else "stop"
                break
            if now - last_ack > idle_s:
                stop_reason = "no_ack"
                break
            if now - t0 > max_s:
                stop_reason = "max_time"
                break
            if now >= next_send:
                seq, resend = tx.next(now_us)
                if seq is not None:
                    off = seq * PAYLOAD
                    pkt = build_data(pass_no, seq, F_RESEND if resend else 0, mv[off:off + len_for(total, seq)], crc)
                    tx.sent_one(seq, resend, now_us)
                    if drop is None or not drop(seq):
                        try:
                            s.sendto(pkt, dest)
                        except (BlockingIOError, InterruptedError):
                            pass
                    gap = 1.0 / tx.pps
                    next_send = max(next_send + gap, now - 3 * gap)   # catch up by 3 at most (the ring holds ~11)
        el = clock() - t0
        log("[g2x] pass %d: %s after %.2f s, sent %d + %d resent, rto %d, loss events %d, pps end %.0f, acks %d" % (
            pass_no, stop_reason, el, tx.sent, tx.resent, tx.rto_fires, tx.loss_events, tx.pps, acks))
        return {"reason": stop_reason, "sent": tx.sent, "resent": tx.resent, "rto": tx.rto_fires, "loss_events": tx.loss_events,
                "pps_end": int(tx.pps), "pps_avg": int(pps_sum / pps_n) if pps_n else int(tx.pps), "acks": acks,
                "bad_acks": bad_acks, "acked": tx.cum, "s": round(el, 3)}
    finally:
        if own:
            s.close()
