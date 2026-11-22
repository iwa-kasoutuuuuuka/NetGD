"""Tests of g2n_send.py against a small fake Dreamcast (same protocol as main_net.c), and of the pattern / frame
cross-check with the C core (xcheck.bin from test_net). Usage: python test_sender.py [xcheck.bin]"""
import os
import socket
import struct
import sys
import tempfile
import threading
import time
import unittest
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, ".."))
import g2n_send  # noqa: E402
import g2x_send  # noqa: E402
from netgd_send import ngtp  # noqa: E402

XCHECK = sys.argv.pop(1) if len(sys.argv) > 1 and not sys.argv[1].startswith("-") else None


class FakeDc(threading.Thread):
    """accepts one connection, answers HELLO, on PUT runs two WANT/DATA passes and one UDP burst"""

    def __init__(self, corrupt=False):
        threading.Thread.__init__(self)
        self.daemon = True
        self.corrupt = corrupt
        self.ls = socket.socket()
        self.ls.bind(("127.0.0.1", 0))
        self.ls.listen(1)
        self.us = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.us.bind(("127.0.0.1", 0))
        self.us.settimeout(2.0)
        self.tcp_port = self.ls.getsockname()[1]
        self.udp_port = self.us.getsockname()[1]
        self.error = None

    def rd(self, c, n):
        b = b""
        while len(b) < n:
            x = c.recv(n - len(b))
            if not x:
                raise EOFError
            b += x
        return b

    def frame(self, c):
        hdr = self.rd(c, 16)
        t, f, seq, n = ngtp.parse_header(hdr)
        body = self.rd(c, n + 4)
        assert zlib.crc32(hdr + body[:n]) & 0xFFFFFFFF == struct.unpack("<I", body[n:])[0]
        return t, f, seq, body[:n]

    def run(self):
        try:
            c, _ = self.ls.accept()
            t, f, seq, p = self.frame(c)
            assert t == ngtp.HELLO
            c.sendall(ngtp.build_frame(ngtp.HELLO_OK, struct.pack("<HHI", 1, 0, 65536) + b"\0\0" + b"\0" * 24 + b"fake", 0, seq))
            t, f, seq, p = self.frame(c)
            assert t == ngtp.PUT and p[4:8] == b"9999"
            line = lambda s: c.sendall(ngtp.build_frame(ngtp.LINE, s.encode(), 0, seq))
            line("tool=fakedc")
            for n, (want, depth, total) in enumerate([(16384, 1, 70000), (65536, 2, 200000)], 1):
                crc, off_issue, off_recv, out = 0, 0, 0, 0
                while off_recv < total:
                    while out < depth and off_issue < total:
                        ln = min(want, total - off_issue)
                        c.sendall(ngtp.build_frame(ngtp.WANT, struct.pack("<HHQI", n, 0, off_issue, ln), 0, seq))
                        off_issue += ln
                        out += 1
                    t, f, sq, p = self.frame(c)
                    assert t == ngtp.DATA and sq == seq
                    idx, _z, off = struct.unpack_from("<HHQ", p)
                    assert idx == n and off == off_recv
                    data = p[12:]
                    assert data == g2n_send.gen_range(off, len(data))
                    crc = zlib.crc32(data, crc) & 0xFFFFFFFF
                    off_recv += len(data)
                    out -= 1
                if self.corrupt:
                    crc ^= 1
                for s in ("p%d_want=%d" % (n, want), "p%d_depth=%d" % (n, depth), "p%d_checksums=1" % n, "p%d_rcvbuf=65535" % n,
                          "p%d_status=ok" % n, "p%d_status_ok=1" % n, "p%d_bytes=%d" % (n, off_recv), "p%d_us=1000000" % n,
                          "p%d_kbps=%d" % (n, off_recv // 1024), "p%d_crc=0x%08x" % (n, crc)):
                    line(s)
            line("G2N-UDP go pass=1 count=50 size=1400 pps=0")
            got, ended = 0, False
            while not ended:
                d, _a = self.us.recvfrom(2048)
                if d[:4] != b"G2U1":
                    continue
                if d[4] == 2:
                    ended = True
                else:
                    got += 1
            for s in ("u1_pps=0", "u1_count=50", "u1_got=%d" % got, "u1_missing=%d" % (50 - got), "u1_kbps=1"):
                line(s)
            # one g2x pass (custom UDP transfer), every 7th datagram lost once
            total = 150000
            line("G2X go pass=1 bytes=%d pps=4000 min=0 max=0 crc=1 win=64 limit_s=20" % total)
            out = bytearray()
            rx = g2x_send.Rx(1, total, lambda seq, d: out.extend(d), True)
            self.us.settimeout(0.001)
            t0, peer, lost = time.perf_counter(), None, set()
            now = lambda: int((time.perf_counter() - t0) * 1e6) + 1
            while not rx.complete() and time.perf_counter() - t0 < 20:
                try:
                    d, a = self.us.recvfrom(2048)
                except socket.timeout:
                    if peer and rx.poll(now()):
                        self.us.sendto(rx.make_ack(0, now()).pack(), peer)
                    continue
                if d[:4] != b"G2X1":
                    continue                       # the burst's late end packets (another source port)
                peer = peer or a
                seq = int.from_bytes(d[8:12], "little")
                if seq % 7 == 3 and seq not in lost:
                    lost.add(seq)
                    continue
                if rx.input(d, now()):
                    self.us.sendto(rx.make_ack(0, now()).pack(), peer)
            for _ in range(3):
                self.us.sendto(rx.make_ack(0, now()).pack(), peer)
            assert bytes(out) == g2n_send.gen_range(0, total)
            crc = zlib.crc32(bytes(out)) & 0xFFFFFFFF
            if self.corrupt:
                crc ^= 1
            for s in ("x1_pps=4000", "x1_ck=2", "x1_status=done", "x1_status_ok=1", "x1_bytes=%d" % len(out),
                      "x1_us=100000", "x1_kbps=1", "x1_dgrams=%d" % rx.dgrams, "x1_crc=0x%08x" % crc):
                line(s)
            line("status=done")
            c.sendall(ngtp.build_frame(ngtp.DONE, b"\x01", 0, seq))
            c.settimeout(3.0)
            try:
                c.recv(1)
            except OSError:
                pass
            c.close()
        except Exception as e:  # noqa: BLE001
            self.error = repr(e)
        finally:
            self.ls.close()
            self.us.close()


class SenderTests(unittest.TestCase):
    def go(self, corrupt):
        dc = FakeDc(corrupt)
        dc.start()
        with tempfile.TemporaryDirectory() as d:
            ok, path, kv = g2n_send.run("127.0.0.1", d, 5.0, dc.tcp_port, dc.udp_port, log=lambda s: None)
            with open(path) as fh:
                text = fh.read()
            self.assertEqual(os.path.basename(path), "g2net_001.txt")
        dc.join(10)
        self.assertIsNone(dc.error)
        return ok, kv, text

    def test_ok(self):
        ok, kv, text = self.go(False)
        self.assertTrue(ok)
        self.assertIn("p1_pc_crc_ok=1", text)
        self.assertIn("p2_pc_crc_ok=1", text)
        self.assertIn("pc_all_ok=1", text)
        self.assertEqual(kv["u1_got"], "50")
        self.assertIn("x1_pc_crc_ok=1", text)
        self.assertIn("x1_pc_reason=done", text)
        self.assertNotIn("x1_pc_resent=0\n", text)        # every 7th datagram had to be resent

    def test_crc_mismatch_is_reported(self):
        ok, kv, text = self.go(True)
        self.assertFalse(ok)
        self.assertIn("p1_pc_crc_ok=0", text)
        self.assertIn("x1_pc_crc_ok=0", text)
        self.assertIn("pc_all_ok=0", text)

    def test_no_dreamcast(self):
        s = socket.socket()
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
        s.close()
        with self.assertRaises(ngtp.ConnectionLost):
            g2n_send.connect("127.0.0.1", port, 0.5, log=lambda s: None)


class PatternTests(unittest.TestCase):
    def test_range_matches_blocks(self):
        b0, b1 = g2n_send.gen_block(0), g2n_send.gen_block(1)
        self.assertEqual(g2n_send.gen_range(65000, 2000), b0[65000:] + b1[:2000 - (65536 - 65000)])
        self.assertNotEqual(b0, b1)

    @unittest.skipUnless(XCHECK, "no xcheck.bin")
    def test_c_cross_check(self):
        with open(XCHECK, "rb") as fh:
            data = fh.read()
        t, f, seq, n = ngtp.parse_header(data[:16])
        self.assertEqual((t, f, seq, n), (ngtp.HELLO, 0, 12, 3))
        self.assertEqual(data[16:19], b"abc")
        self.assertEqual(zlib.crc32(data[:19]) & 0xFFFFFFFF, struct.unpack("<I", data[19:23])[0])
        self.assertEqual(data[23:], g2n_send.gen_block(5))
        self.assertEqual(ngtp.build_frame(ngtp.HELLO, b"abc", 0, 12), data[:23])


if __name__ == "__main__":
    unittest.main()
