#!/usr/bin/env python3
"""PC side of g2netbench (phase D of docs/research/g2-perf-plan.md).

Connects to g2netbench.elf on the Dreamcast (TCP port 20039, the NGTP framing of tools/netgd-send/netgd_send/ngtp.py),
answers its WANT frames with DATA from the g2bench test pattern (same bytes as g2b_gen_block / mkdata.py), and at
the Dreamcast's request sends UDP bursts to port 20040 and runs g2x passes (the custom UDP transfer of
g2x_send.py / g2x_core.h: paced 1,400-byte datagrams, selective ACKs). The Dreamcast's key=value lines (LINE frames) are saved to
<out-dir>/g2net_NNN.txt together with the PC's own checks (CRC of the bytes it served against the CRC the
Dreamcast computed while receiving).

  python g2n_send.py --host 192.168.10.2 --out-dir H:\\OpenGDEmu-g2bench\\tools\\g2bench\\host [--wait 600]

--wait: keep trying to connect for that many seconds (the preset starts this before the ELF is uploaded).
Exit code 0 = all passes finished and every CRC matched. Standard library only (Python 3.8+)."""
import argparse
import os
import socket
import struct
import sys
import time
import zlib

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "netgd-send"))
from netgd_send import ngtp  # noqa: E402
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import g2x_send  # noqa: E402

BLOCK = 65536
WORDS = BLOCK // 4
TCP_PORT = ngtp.DEFAULT_PORT
UDP_PORT = 20040


def _base():
    s, out = 0x2545F491, bytearray()
    for _ in range(WORDS):
        s ^= (s << 13) & 0xFFFFFFFF
        s ^= s >> 17
        s ^= (s << 5) & 0xFFFFFFFF
        out += struct.pack("<I", s)
    return int.from_bytes(bytes(out), "little")


_BASE = _base()
_REP = int.from_bytes(b"\x01\x00\x00\x00" * WORDS, "little")


def gen_block(k):
    """block k of the test pattern (g2b_gen_block): word j = xorshift32 value j XOR k * 0x9E3779B1"""
    m = (k * 0x9E3779B1) & 0xFFFFFFFF
    return (_BASE ^ (m * _REP)).to_bytes(BLOCK, "little")


def gen_range(off, n):
    out, k, skip = [], off // BLOCK, off % BLOCK
    while n > 0:
        b = gen_block(k)[skip:]
        out.append(b[:n])
        n -= len(out[-1])
        k, skip = k + 1, 0
    return b"".join(out)


class Source(object):
    """answers WANT: `read(idx, off, n)`; keeps the CRC-32 of what it served per file (sequential reads)"""

    def __init__(self):
        self.crc = {}       # idx -> (crc, bytes)

    def read(self, idx, off, n):
        crc, done = self.crc.get(idx, (0, 0))
        data = gen_range(off, n)
        if off == done:
            self.crc[idx] = (zlib.crc32(data, crc) & 0xFFFFFFFF, done + len(data))
        else:
            self.crc[idx] = (None, done)    # not a straight run: no CRC to compare
        return data


def udp_burst(host, port, pass_no, count, size, pps, log=print):
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        s.setsockopt(socket.SOL_SOCKET, socket.SO_SNDBUF, 1 << 20)
        pkt = bytearray(size)
        pkt[0:4] = b"G2U1"
        pkt[4], pkt[5] = 1, 0
        struct.pack_into("<H", pkt, 6, pass_no)
        pkt[12:] = bytes((0xA5 ^ i) & 0xFF for i in range(12, size))
        t0 = time.perf_counter()
        for seq in range(count):
            struct.pack_into("<I", pkt, 8, seq)
            if pps:
                due = t0 + seq / float(pps)
                while time.perf_counter() < due:
                    pass
            s.sendto(pkt, (host, port))
        el = time.perf_counter() - t0
        log("[g2n_send] UDP pass %d: %d packets of %d bytes in %.2f s (%.0f pps)" % (pass_no, count, size, el, count / el if el else 0))
        end = bytearray(pkt[:12])
        end[4] = 2
        for _ in range(5):
            s.sendto(bytes(end), (host, port))
            time.sleep(0.02)
        return el
    finally:
        s.close()


_STREAM = {}


def stream(n):
    """the first n bytes of the test pattern (cached: a g2x pass reads it at random offsets)"""
    if _STREAM.get("n", 0) < n:
        _STREAM["b"] = gen_range(0, n)
        _STREAM["n"] = n
    return _STREAM["b"]


def g2x_pass(host, port, kv, log=print, drop=None):
    """runs the g2x pass the Dreamcast asked for ("G2X go pass=.. bytes=.. pps=.. min=.. max=.. crc=.."), returns
    the PC's lines"""
    n, total = int(kv["pass"]), int(kv["bytes"])
    r = g2x_send.send_pass(host, port, n, total, stream(total), int(kv["pps"]), int(kv.get("min", 0)), int(kv.get("max", 0)),
                           kv.get("crc", "1") == "1", log=log, max_s=float(kv.get("limit_s", 20)) + 30.0, drop=drop)
    return ["x%d_pc_%s=%s" % (n, k, v) for k, v in sorted(r.items())]


def connect(host, port, wait_s, log=print):
    """ngtp.Client, retrying until `wait_s` have passed"""
    deadline, shown = time.time() + wait_s, 0
    while True:
        try:
            return ngtp.Client(host, port, timeout=90.0)
        except ngtp.ConnectionLost as e:
            if time.time() >= deadline:
                raise
            if time.time() - shown >= 15:
                log("[g2n_send] waiting for g2netbench on %s:%d ..." % (host, port))
                shown = time.time()
            time.sleep(1.0)


def pick_path(out_dir):
    os.makedirs(out_dir, exist_ok=True)
    for n in range(1, 1000):
        p = os.path.join(out_dir, "g2net_%03d.txt" % n)
        if not os.path.exists(p):
            return p
    raise RuntimeError("no free g2net_NNN.txt")


def parse_kv(lines):
    kv = {}
    for l in lines:
        if "=" in l and " " not in l.split("=", 1)[0]:
            k, v = l.split("=", 1)
            kv[k.strip()] = v.strip()
    return kv


def run(host, out_dir, wait_s, port=TCP_PORT, udp_port=UDP_PORT, log=print, g2x_drop=None):
    """-> (ok, result path, kv dict)"""
    src = Source()
    lines = []
    cli = connect(host, port, wait_s, log)
    log("[g2n_send] connected to %s:%d" % (host, port))
    path = pick_path(out_dir)
    fout = open(path, "w")
    fout.write("pc_time=%s\npc_partial=1\n" % time.strftime("%Y-%m-%d %H:%M:%S"))
    fout.flush()
    log("[g2n_send] result file (written as lines arrive): %s" % path)
    with cli:
        info = cli.hello(0)
        log("[g2n_send] HELLO_OK app=%s max_data=%d" % (info.app, info.max_data))

        def on_line(text, throttled):
            if text.startswith("G2X go"):
                kv = dict(w.split("=", 1) for w in text.split()[2:] if "=" in w)
                lines.extend(g2x_pass(host, udp_port, kv, log, g2x_drop))
                return
            if text.startswith("G2N-UDP go"):
                kv = dict(w.split("=") for w in text.split()[2:] if "=" in w)
                udp_burst(host, udp_port, int(kv["pass"]), int(kv["count"]), int(kv["size"]), int(kv["pps"]), log)
                lines.append("u%s_pc_sent=%s" % (kv["pass"], kv["count"]))
                return
            lines.append(text)
            log("[g2n_send] DC: " + text)
            fout.write(text + "\n")        # incrementally: a later hang keeps the earlier numbers
            fout.flush()

        payload = struct.pack("<BBH", ngtp.PUT_NEW, 0, 0) + b"9999" + b"G2NBENCH 1\n"
        try:
            res = cli.request(ngtp.PUT, payload, source=src, on_line=on_line)
        except (ngtp.ConnectionLost, ngtp.NgtpError) as e:
            fout.write("pc_error=%s\n" % e)
            fout.close()
            raise
    kv = parse_kv(lines)
    all_ok = res.ok and kv.get("status") == "done"
    n = 1
    while ("p%d_bytes" % n) in kv:
        p = "p%d" % n
        got_crc = int(kv.get(p + "_crc", "0"), 16)
        if kv.get(p + "_checksums") == "1":
            crc, done = src.crc.get(n, (None, 0))
            match = crc is not None and crc == got_crc and done == int(kv[p + "_bytes"])
            lines.append("%s_pc_crc=0x%08x" % (p, crc or 0))
            lines.append("%s_pc_crc_ok=%d" % (p, 1 if match else 0))
            all_ok = all_ok and match
        else:
            _c, done = src.crc.get(n, (None, 0))
            lines.append("%s_pc_bytes_ok=%d" % (p, 1 if done == int(kv[p + "_bytes"]) else 0))
        all_ok = all_ok and kv.get(p + "_status_ok") == "1"
        n += 1
    n = 1
    while ("x%d_bytes" % n) in kv:
        p = "x%d" % n
        got = int(kv[p + "_bytes"])
        if kv.get(p + "_ck") == "2":
            crc = zlib.crc32(memoryview(stream(got))[:got]) & 0xFFFFFFFF
            match = crc == int(kv.get(p + "_crc", "0"), 16)
            lines.append("%s_pc_crc=0x%08x" % (p, crc))
            lines.append("%s_pc_crc_ok=%d" % (p, 1 if match else 0))
            all_ok = all_ok and match
        all_ok = all_ok and kv.get(p + "_status_ok") == "1"
        n += 1
    # the PC's own checks go after the Dreamcast's lines; pc_partial=1 above is superseded by pc_partial=0
    fout.write("\n".join(l for l in lines if l[:1] in ("p", "u", "x") and "_pc_" in l) + "\n")
    fout.write("pc_partial=0\npc_all_ok=%d\n" % (1 if all_ok else 0))
    fout.close()
    return all_ok, path, kv


def summary(kv, log=print):
    n = 1
    while ("p%d_bytes" % n) in kv:
        p = "p%d_" % n
        log("[g2n_send] pass %d: want %s KiB x depth %s, rcvbuf %s, checksums %s: %s KB/s (%s bytes in %.2f s) %s" % (
            n, int(kv.get(p + "want", 0)) // 1024, kv.get(p + "depth"), kv.get(p + "rcvbuf"), kv.get(p + "checksums"),
            kv.get(p + "kbps"), kv.get(p + "bytes"), int(kv.get(p + "us", 0)) / 1e6, kv.get(p + "status")))
        n += 1
    n = 1
    while ("u%d_got" % n) in kv:
        p = "u%d_" % n
        log("[g2n_send] UDP %d: pps %s: got %s of %s, missing %s, %s KB/s" % (
            n, kv.get(p + "pps"), kv.get(p + "got"), kv.get(p + "count"), kv.get(p + "missing"), kv.get(p + "kbps")))
        n += 1
    n = 1
    while ("x%d_bytes" % n) in kv:
        p = "x%d_" % n
        log("[g2n_send] g2x %d: pps %s (%s..%s), ck %s: %s KB/s (%s bytes in %.2f s) %s, dgrams %s bad %s dups %s gaps %s acks %s" % (
            n, kv.get(p + "pps"), kv.get(p + "pps_min"), kv.get(p + "pps_max"), kv.get(p + "ck"), kv.get(p + "kbps"),
            kv.get(p + "bytes"), int(kv.get(p + "us", 0)) / 1e6, kv.get(p + "status"), kv.get(p + "dgrams"), kv.get(p + "bad"),
            kv.get(p + "dups"), kv.get(p + "gaps"), kv.get(p + "acks")))
        n += 1


def main(argv=None):
    ap = argparse.ArgumentParser(description="PC side of g2netbench")
    ap.add_argument("--host", required=True, help="the Dreamcast's IP")
    ap.add_argument("--out-dir", required=True, help="folder for g2net_NNN.txt (g2bench's host\\)")
    ap.add_argument("--wait", type=float, default=0.0, help="seconds to keep trying to connect (default: once)")
    ap.add_argument("--port", type=int, default=TCP_PORT)
    ap.add_argument("--udp-port", type=int, default=UDP_PORT)
    a = ap.parse_args(argv)
    try:
        ok, path, kv = run(a.host, a.out_dir, a.wait, a.port, a.udp_port)
    except (ngtp.ConnectionLost, ngtp.NgtpError, OSError) as e:
        print("[g2n_send] failed: %s" % e)
        return 2
    summary(kv)
    print("[g2n_send] result: %s (%s)" % (path, "all CRCs match" if ok else "CHECK FAILED"))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
