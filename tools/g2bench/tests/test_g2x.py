"""Tests of the g2x sender (g2x_send.py) over real UDP on localhost against the Python receiver (g2x_send.Rx),
through a lossy shim (datagrams and ACKs dropped / duplicated / corrupted on purpose), plus the codec cross-check
with the C core (xport_vec.bin written by tests/test_xport). Usage: python test_g2x.py [xport_vec.bin]"""
import os
import random
import socket
import sys
import threading
import time
import unittest
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, ".."))
import g2x_send as gx  # noqa: E402

VEC = sys.argv.pop(1) if len(sys.argv) > 1 and not sys.argv[1].startswith("-") else None


class FakeRx(threading.Thread):
    """the Dreamcast's g2x receiver loop (main_net.c run_xport) with impairments on its input and its ACKs"""

    def __init__(self, pass_no, total, need_crc=True, loss=0.0, ack_loss=0.0, dup=0.0, flip=0.0, burst=0, seed=1,
                 stop_after_s=None):
        threading.Thread.__init__(self)
        self.daemon = True
        self.s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.s.bind(("127.0.0.1", 0))
        self.s.settimeout(0.001)
        self.port = self.s.getsockname()[1]
        self.out = bytearray()
        self.order_ok = True
        self.rx = gx.Rx(pass_no, total, self._deliver, need_crc)
        self.loss, self.ack_loss, self.dup, self.flip, self.burst = loss, ack_loss, dup, flip, burst
        self.rnd = random.Random(seed)
        self.stop_after_s = stop_after_s
        self.status = None
        self.error = None
        self.dropped = 0

    def _deliver(self, seq, data):
        if seq * gx.PAYLOAD != len(self.out):
            self.order_ok = False
        self.out += data

    def _ack(self, peer, flags=0):
        a = self.rx.make_ack(flags, self._now())
        if self.rnd.random() >= self.ack_loss:
            self.s.sendto(a.pack(), peer)

    def _now(self):
        return int((time.perf_counter() - self.t0) * 1e6) + 1

    def run(self):
        self.t0 = time.perf_counter()
        peer, last = None, time.perf_counter()
        burst_left = 0
        try:
            while not self.rx.complete():
                try:
                    d, a = self.s.recvfrom(2048)
                except socket.timeout:
                    d = None
                except ConnectionResetError:
                    continue
                now = self._now()
                if d is not None:
                    peer = peer or a
                    last = time.perf_counter()
                    if burst_left or self.rnd.random() < self.loss:     # lost (a burst stands for a full BBA ring)
                        self.dropped += 1
                        burst_left = burst_left - 1 if burst_left else self.burst
                        continue
                    if self.flip and self.rnd.random() < self.flip and len(d) > gx.HDR:
                        b = bytearray(d)
                        b[gx.HDR + self.rnd.randrange(len(b) - gx.HDR)] ^= 0x10
                        d = bytes(b)
                    copies = 2 if self.rnd.random() < self.dup else 1
                    for _ in range(copies):
                        if self.rx.input(d, now):
                            self._ack(peer)
                    continue
                if peer and self.rx.poll(now):
                    self._ack(peer)
                if self.stop_after_s and peer and time.perf_counter() - self.t0 > self.stop_after_s:
                    self.status = "time_limit"
                    break
                if time.perf_counter() - last > 5:
                    self.status = "idle"
                    break
            if self.status is None:
                self.status = "done"
            for _ in range(5):
                if peer:
                    self.s.sendto(self.rx.make_ack(gx.A_STOP if self.status != "done" else 0, self._now()).pack(), peer)
                time.sleep(0.003)
        except Exception as e:  # noqa: BLE001
            self.error = repr(e)
        finally:
            self.s.close()


def pattern(n, seed=7):
    r = random.Random(seed)
    return bytes(r.getrandbits(8) for _ in range(min(n, 4096))) * (n // 4096 + 1)


class G2xTests(unittest.TestCase):
    def go(self, total, pps=3000, pmin=0, pmax=0, crc=True, **imp):
        data = pattern(total)[:total]
        rx = FakeRx(1, total, crc, **imp)
        rx.start()
        r = gx.send_pass("127.0.0.1", rx.port, 1, total, data, pps, pmin, pmax, crc, log=lambda s: None, idle_s=3.0, max_s=60)
        rx.join(10)
        self.assertIsNone(rx.error)
        return rx, r, data

    def check_ok(self, rx, r, data):
        self.assertEqual(rx.status, "done")
        self.assertEqual(r["reason"], "done")
        self.assertTrue(rx.order_ok)
        self.assertEqual(bytes(rx.out), data)

    def test_clean(self):
        rx, r, data = self.go(300 * 1024)
        self.check_ok(rx, r, data)
        self.assertEqual(r["resent"], 0)
        self.assertEqual(rx.rx.bad, 0)

    def test_random_loss_and_ack_loss(self):
        rx, r, data = self.go(400 * 1024, loss=0.05, ack_loss=0.1, dup=0.02, seed=3)
        self.check_ok(rx, r, data)
        self.assertGreater(r["resent"], 0)
        self.assertGreaterEqual(r["resent"], rx.dropped - 1)

    def test_bursts_like_a_full_ring(self):
        rx, r, data = self.go(400 * 1024, loss=0.01, burst=12, seed=5)          # 13 in a row lost now and then
        self.check_ok(rx, r, data)
        self.assertGreater(rx.dropped, 12)

    def test_corruption_caught_by_crc(self):
        rx, r, data = self.go(300 * 1024, flip=0.03, seed=9)
        self.check_ok(rx, r, data)
        self.assertGreater(rx.rx.bad, 0)

    def test_adaptive_rate_backs_off(self):
        rx, r, data = self.go(400 * 1024, pps=2000, pmin=800, pmax=4000, loss=0.05, seed=11)
        self.check_ok(rx, r, data)
        self.assertGreater(r["loss_events"], 0)
        self.assertLess(r["pps_end"], 4000)

    def test_tiny_and_odd_sizes(self):
        for total in (1, gx.PAYLOAD, gx.PAYLOAD + 1, 5 * gx.PAYLOAD - 3):
            rx, r, data = self.go(total)
            self.check_ok(rx, r, data)

    def test_time_limit_stop(self):
        rx, r, data = self.go(50 << 20, pps=500, stop_after_s=0.5)              # far too much for 0.5 s at 500 pps
        self.assertEqual(rx.status, "time_limit")
        self.assertEqual(r["reason"], "stop")
        self.assertTrue(rx.order_ok)
        self.assertEqual(bytes(rx.out), data[:len(rx.out)])
        self.assertGreater(len(rx.out), 0)

    def test_tx_rto_resends_tail(self):
        tx = gx.Tx(10, 1000)
        for s in range(10):
            q, rs = tx.next(100 + s)
            self.assertEqual((q, rs), (s, False))
            tx.sent_one(q, rs, 100 + s)
        self.assertEqual(tx.next(200), (None, False))           # window open but everything sent
        q, rs = tx.next(100 + gx.RTO_US + 50)                   # no ACK at all: resend from the oldest
        self.assertEqual((q, rs), (0, True))
        self.assertEqual(tx.rto_fires, 1)

    def test_tx_sack_marks_hole_lost(self):
        tx = gx.Tx(20, 1000)
        for s in range(8):
            q, rs = tx.next(10)
            tx.sent_one(q, rs, 10)
        # receiver has 0, 1, 3, 4 (2 lost): cum = 2, bits for 3 and 4 = bits 0 and 1
        tx.on_ack(gx.Ack(0, 1, 2, 66, 0b11, 0, 1), 20)
        self.assertEqual(tx.next(21), (2, True))
        self.assertEqual(tx.loss_events, 1)

    @unittest.skipUnless(VEC, "no xport_vec.bin")
    def test_c_vectors(self):
        with open(VEC, "rb") as fh:
            v = fh.read()
        d, a = v[:gx.DGRAM], v[gx.DGRAM:]
        p = gx.parse_data(d)
        self.assertNotIn(p, (None, "crc"))
        flags, pass_no, seq, data = p
        self.assertEqual((pass_no, seq, len(data)), (5, 3, gx.PAYLOAD))
        self.assertTrue(flags & gx.F_CRC)
        self.assertEqual(gx.build_data(5, 3, 0, data, True), d)
        ack = gx.Ack.parse(a)
        self.assertEqual((ack.flags, ack.pass_no, ack.cum, ack.limit, ack.sack_lo, ack.sack_hi, ack.ack_no, ack.dgrams, ack.dups, ack.bad),
                         (gx.A_DONE, 5, 10, 10, 5, 0x80000000, 42, 11, 1, 0))
        self.assertEqual(ack.pack(), a)
        # the C core's pattern for datagram 3: pat(off) = ((off * 2654435761) >> 13 ^ off >> 7) & 0xFF (64-bit product)
        exp = bytes((((o * 2654435761) >> 13) ^ (o >> 7)) & 0xFF for o in range(3 * gx.PAYLOAD, 4 * gx.PAYLOAD))
        self.assertEqual(data, exp)
        self.assertEqual(zlib.crc32(data, zlib.crc32(d[:16])) & 0xFFFFFFFF, int.from_bytes(d[16:20], "little"))


if __name__ == "__main__":
    unittest.main()
