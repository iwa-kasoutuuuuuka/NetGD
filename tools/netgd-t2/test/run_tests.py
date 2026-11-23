"""Hardware-free tests of netgd-t2, the standalone T2 receiver (NGTP over TCP -> the NetGD SD card).

    py -3 tools/netgd-t2/test/run_tests.py
    py -3 tools/netgd-t2/test/run_tests.py -k window

Builds build/t2_host from the T2 core, netgd-menu's NGTP server and host doubles (plat_host.c, net_host.c), the netgd-recv
core, FatFs and netgd-recv's disk-image / firmware models; never the KOS files.  The PC side is netgd-send's own client
(netgd_send.ngtp.Client and `netgd-send.py push`), over a real 127.0.0.1 socket.  Scratch files go under the system temp
folder (NETGD_TEST_WORK=<dir> to choose); every child process has a time limit (NETGD_CHILD_TIMEOUT, default 120 s).
Nothing here has run on a Dreamcast.
"""

import atexit
import hashlib
import os
import re
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import zlib

if sys.platform in ("cygwin", "msys"):
    sys.exit("run_tests.py: run with a native Python (py -3), not the MSYS one (see tools/ci-local/README_ja.md)")

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)                      # tools/netgd-t2
TOOLS = os.path.dirname(ROOT)
MENU = os.path.join(TOOLS, "netgd-menu")
RECV = os.path.join(TOOLS, "netgd-recv")
SEND_PY = os.path.join(TOOLS, "netgd-send", "netgd-send.py")
sys.path.insert(0, os.path.join(TOOLS, "netgd-send"))
sys.path.insert(0, os.path.join(RECV, "test"))

from netgd_send import manifest as mf  # noqa: E402
from netgd_send import ngtp  # noqa: E402
import fatcheck  # noqa: E402

CHILD_TIMEOUT = float(os.environ.get("NETGD_CHILD_TIMEOUT", "120"))
CODE = 4821
SECTORS = 100000
EXE = os.path.join(ROOT, "build", "t2_host.exe" if os.name == "nt" else "t2_host")
RECV_EXE = os.path.join(ROOT, "build", "recv_tool.exe" if os.name == "nt" else "recv_tool")

# T2_S_* / T2_R_* of src/t2_app.h
S_DONE, S_FAILED = 4, 5
R_FINISH, R_USER, R_IDLE, R_NONET = 1, 2, 3, 4

T2_SRCS = ["src/t2_app.c", "test/t2_host.c"]
MENU_SRCS = ["src/nm_screen.c", "src/ngtp_frame.c", "src/ngtp_srv.c", "test/plat_host.c", "test/net_host.c"]
RECV_CORE = ["src/crc32.c", "src/manifest.c", "src/ngblk.c", "src/fwctl.c", "src/ngfs.c", "src/recv_core.c", "src/diskio.c",
             "third_party/fatfs/ff.c", "third_party/fatfs/ffunicode.c", "third_party/fatfs/ffsystem.c",
             "test/dev_file.c", "test/fwctl_sim.c"]
RECV_TOOL = RECV_CORE[:7] + ["src/xport_pc.c"] + RECV_CORE[7:] + ["test/plat_host.c", "test/test_host.c"]


def run_child(cmd, **kw):
    kw.setdefault("capture_output", True)
    kw.setdefault("text", True)
    kw.setdefault("stdin", subprocess.DEVNULL)
    kw.setdefault("timeout", CHILD_TIMEOUT)
    try:
        return subprocess.run(cmd, **kw)
    except subprocess.TimeoutExpired as e:
        raise AssertionError("no exit after %g s: %s\n%s" % (kw["timeout"], " ".join(map(str, cmd)), e.stdout or ""))


def _work_root():
    env = os.environ.get("NETGD_TEST_WORK")
    if env:
        os.makedirs(env, exist_ok=True)
        return os.path.abspath(env)
    d = tempfile.mkdtemp(prefix="ngt2-", dir=os.path.realpath(tempfile.gettempdir()))
    atexit.register(shutil.rmtree, d, True)
    return d


WORK_ROOT = _work_root()


def _newest(paths):
    t = max(os.path.getmtime(p) for p in paths)
    for d in (os.path.join(ROOT, "src"), os.path.join(ROOT, "test"), os.path.join(MENU, "src"), os.path.join(MENU, "test"),
              os.path.join(RECV, "src"), os.path.join(RECV, "test")):
        for f in os.listdir(d):
            if f.endswith(".h"):
                t = max(t, os.path.getmtime(os.path.join(d, f)))
    return t


def build():
    os.makedirs(os.path.join(ROOT, "build"), exist_ok=True)
    srcs = ([os.path.join(ROOT, s) for s in T2_SRCS] + [os.path.join(MENU, s) for s in MENU_SRCS]
            + [os.path.join(RECV, s) for s in RECV_CORE])
    lib = ["-lws2_32"] if os.name == "nt" else []
    flags = ["gcc", "-O1", "-g", "-Wall", "-Werror", "-Wno-unused-function", "-Wno-format-truncation", "-DNG_HOST"]
    if not os.path.exists(EXE) or os.path.getmtime(EXE) < _newest(srcs):
        cmd = flags + ["-Isrc", "-I../netgd-menu/src", "-I../netgd-menu/test", "-I../netgd-recv/src",
                       "-I../netgd-recv/third_party/fatfs", "-I../netgd-recv/test", "-o", EXE] + srcs + lib
        r = run_child(cmd, cwd=ROOT, timeout=max(CHILD_TIMEOUT, 600))
        if r.returncode:
            raise RuntimeError("build failed:\n" + r.stdout + r.stderr)
    tool = [os.path.join(RECV, s) for s in RECV_TOOL]
    if not os.path.exists(RECV_EXE) or os.path.getmtime(RECV_EXE) < _newest(tool):
        # netgd-recv's test_host (mkimage / addfile), built here so that the netgd-recv tree is not written to
        cmd = flags[:-1] + ["-DNG_HOST", "-Isrc", "-Ithird_party/fatfs", "-Itest", "-o", RECV_EXE] + RECV_TOOL
        r = run_child(cmd, cwd=RECV, timeout=max(CHILD_TIMEOUT, 600))
        if r.returncode:
            raise RuntimeError("build of test_host failed:\n" + r.stdout + r.stderr)


def pattern(n, seed):
    base = bytes((i * 7 + seed * 13) & 0xFF for i in range(251))
    return (base * (n // 251 + 1))[:n]


class Serve(object):
    """t2_host serve on a card image"""

    def __init__(self, test, img, *opts, code=CODE):
        s = socket.socket()
        s.bind(("127.0.0.1", 0))
        self.port = s.getsockname()[1]
        s.close()
        self.test = test
        cmd = [EXE, "serve", img, str(SECTORS), "--port", str(self.port), "--code", str(code)] + list(opts)
        self.proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, errors="replace",
                                     stdin=subprocess.DEVNULL)
        test.addCleanup(self._kill)
        self.lines = []
        self.ready = threading.Event()
        self.thread = threading.Thread(target=self._pump, daemon=True)
        self.thread.start()
        if not self.ready.wait(60):
            self._kill()
            test.fail("t2_host did not start:\n" + "\n".join(self.lines))

    def _pump(self):
        for line in self.proc.stdout:
            self.lines.append(line.rstrip("\n"))
            if line.startswith(("LISTENING", "T2-END")):
                self.ready.set()
        self.ready.set()

    def _kill(self):
        if self.proc.poll() is None:
            self.proc.kill()
        self.proc.wait()
        self.thread.join(5)
        self.proc.stdout.close()

    def client(self):
        c = ngtp.Client("127.0.0.1", self.port, timeout=30)
        self.test.addCleanup(c.close)
        return c

    def finish(self, timeout=60, check=True):
        try:
            self.proc.wait(timeout)
        except subprocess.TimeoutExpired:
            self._kill()
            self.test.fail("t2_host did not end:\n" + "\n".join(self.lines))
        self.thread.join(10)
        self.text = "\n".join(self.lines)
        m = re.search(r"^SIM (.*)$", self.text, re.M)
        self.sim = {k: int(v) for k, v in re.findall(r"(\w+)=(-?\d+)", m.group(1))} if m else {}
        m = re.search(r"^T2-END (.*)$", self.text, re.M)
        self.end = {k: int(v) for k, v in re.findall(r"(\w+)=(-?\d+)", m.group(1))} if m else {}
        if check and self.sim and "POWERCUT" not in self.text:
            t = self.test
            # never a write outside a session or one the firmware would refuse, and no session left open at the end
            t.assertEqual((self.sim["unsess"], self.sim["violations"], self.sim["mount_in_session"], self.sim["session"]),
                          (0, 0, 0, 0), self.text)
        return self.sim


class Base(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        build()

    def setUp(self):
        self.work = os.path.join(WORK_ROOT, self.id().split(".")[-1][:40])
        shutil.rmtree(self.work, ignore_errors=True)
        os.makedirs(self.work)

    def card(self, files=None):
        img = os.path.join(self.work, "card.img")
        r = run_child([RECV_EXE, "mkimage", img, str(SECTORS), "512"])
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        for i, (path, data) in enumerate(sorted((files or {}).items())):
            src = os.path.join(self.work, "src%d.bin" % i)
            with open(src, "wb") as f:
                f.write(data)
            r = run_child([RECV_EXE, "addfile", img, str(SECTORS), path, src])
            self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        return img

    def sha(self, p):
        with open(p, "rb") as f:
            return hashlib.sha1(f.read()).hexdigest()

    def gdi_set(self, folder="03", sizes=(3000, 70000, 140001), crc_of=None):
        """(manifest text, [bytes per manifest file]); file 0 is the descriptor"""
        gdi = b"3\r\n1 0 4 2352 track01.bin 0\r\n2 450 0 2352 track02.raw 0\r\n3 45000 4 2048 track03.bin 0\r\n"
        datas = [("disc.gdi", gdi)] + [("track0%d.%s" % (i + 1, "raw" if i == 1 else "bin"), pattern(sz, i + 1))
                                       for i, sz in enumerate(sizes)]
        files = [mf.FileEntry(len(d), zlib.crc32((crc_of or {}).get(i, d)) & 0xFFFFFFFF, n) for i, (n, d) in enumerate(datas)]
        return mf.Manifest(folder, "gdi", files, "disc.gdi").render(), [d for _n, d in datas]

    def check_folder(self, img, folder, datas):
        fc = fatcheck.Fat32(img)
        self.assertEqual(fc.errors, [])
        self.assertEqual(fc.lost_clusters(), 0)
        names = {k.lower(): k for k in fc.folder_files("/" + folder)}
        for n, d in zip(["disc.gdi", "track01.bin", "track02.raw", "track03.bin"], datas):
            self.assertEqual(fc.read_file("/%s/%s" % (folder, names[n])), d, n)
        self.assertFalse([p for p in fc.files if p.lower().startswith("/%s/" % folder.lower()) and p.lower().endswith((".ngp", ".ckp"))])
        return fc


class Src(object):
    """the PC's file data for the WANTs; counts them"""

    def __init__(self, datas, abort_at=None, close_at=None, sock=None):
        self.datas, self.wants, self.sizes = datas, 0, []
        self.abort_at, self.close_at, self.sock = abort_at, close_at, sock

    def read(self, idx, off, n):
        self.wants += 1
        self.sizes.append(n)
        if self.close_at is not None and self.wants >= self.close_at:
            self.sock.close()                        # the PC goes away in the middle of the transfer
        return self.datas[idx][off:off + n]

    def abort(self):
        return self.abort_at is not None and self.wants >= self.abort_at


def put_payload(text, mode=ngtp.PUT_NEW, tag="0001"):
    return struct.pack("<BBH", mode, 0, 0) + tag.encode() + text.encode()


class TestWindow(Base):
    def put_with(self, *opts, sizes=(3000, 70000, 140001)):
        img = self.card()
        text, datas = self.gdi_set(sizes=sizes)
        srv = Serve(self, img, *opts)
        c = srv.client()
        c.hello(CODE)
        src = Src(datas)
        res = c.put(text, src, mode=ngtp.PUT_NEW)
        self.assertTrue(res.ok, res.lines)
        self.assertTrue(c.finish().ok)
        sim = srv.finish()
        self.assertEqual((srv.end["state"], srv.end["result"]), (S_DONE, R_FINISH), srv.text)
        self.assertEqual((sim["begin"], sim["end"], sim["endmode"]), (1, 1, 1), srv.text)
        self.check_folder(img, "03", datas)
        return srv, src, datas

    def test_stock_default_is_16k_x1(self):
        srv, src, datas = self.put_with()
        self.assertIn("want 16384 depth 1", srv.text)
        self.assertEqual(max(src.sizes), 16384)
        expect = sum((len(d) + 16383) // 16384 for d in datas)
        self.assertEqual((src.wants, srv.end["wants"], srv.end["data"]), (expect, expect, expect), srv.text)

    def test_tcpfix_default_is_16k_x1(self):
        # interim: 64 KiB x 2 is the shape of g2netbench p4, which hung the real Dreamcast (cause unknown)
        srv, src, datas = self.put_with("--tcpfix")
        self.assertIn("want 16384 depth 1", srv.text)
        self.assertEqual(max(src.sizes), 16384)
        self.assertEqual(src.wants, sum((len(d) + 16383) // 16384 for d in datas))

    def test_tcpfix_explicit_64k_x2_still_works(self):
        srv, src, datas = self.put_with("--tcpfix", "--want", "65536", "--depth", "2")
        self.assertIn("want 65536 depth 2", srv.text)
        self.assertEqual(src.wants, sum((len(d) + 65535) // 65536 for d in datas))

    def test_odd_size_deep_window(self):
        # a WANT size that does not divide the core's 64 KiB reads: DATA bytes are held between reads
        srv, src, datas = self.put_with("--want", "5000", "--depth", "3")
        self.assertEqual(max(src.sizes), 5000)
        self.assertEqual(src.wants, sum((len(d) + 4999) // 5000 for d in datas))

    def test_two_wants_outstanding_before_any_data(self):
        img = self.card()
        text, datas = self.gdi_set()
        srv = Serve(self, img, "--want", "65536", "--depth", "2")
        s = socket.create_connection(("127.0.0.1", srv.port), timeout=30)
        self.addCleanup(s.close)

        def rd(n):
            b = b""
            while len(b) < n:
                k = s.recv(n - len(b))
                self.assertTrue(k, "closed")
                b += k
            return b

        def frame():
            t, fl, sq, n = ngtp.parse_header(rd(16))
            body = rd(n + 4)
            return t, fl, sq, body[:n]

        s.sendall(ngtp.build_frame(ngtp.HELLO, struct.pack("<HH", 1, CODE) + b"raw", seq=1))
        self.assertEqual(frame()[0], ngtp.HELLO_OK)
        s.sendall(ngtp.build_frame(ngtp.PUT, put_payload(text), seq=2))
        pending, pairs, done = [], [], None
        while done is None:
            t, fl, sq, p = frame()
            if t == ngtp.WANT:
                w = struct.unpack("<HHQI", p)
                pending.append(w)
                # the receiver asks for at most 2 at once: answer when 2 are outstanding or the file has no more chunks
                if len(pending) == 2 or w[2] + w[3] >= len(datas[w[0]]):
                    if len(pending) == 2:
                        pairs.append(tuple(pending))
                    for idx, _z, off, n in pending:
                        s.sendall(ngtp.build_frame(ngtp.DATA, struct.pack("<HHQ", idx, 0, off) + datas[idx][off:off + n], seq=2))
                    pending = []
            elif t == ngtp.DONE:
                done = p
            else:
                self.assertEqual(t, ngtp.LINE, p)
        self.assertTrue(pairs, "never two WANTs outstanding")
        for a, b in pairs:                       # the second WANT of a pair continues the first one
            self.assertEqual((b[0], b[2]), (a[0], a[2] + a[3]), pairs)
        self.assertEqual(done[0], 1, done)
        s.sendall(ngtp.build_frame(ngtp.FINISH, bytes([1]), seq=3))
        srv.finish()
        self.check_folder(img, "03", datas)


class TestStopResume(Base):
    def test_abort_flag_then_resume(self):
        img = self.card()
        text, datas = self.gdi_set()
        srv = Serve(self, img, "--tcpfix")
        c = srv.client()
        c.hello(CODE)
        src = Src(datas, abort_at=3)
        res = c.put(text, src, abort=src.abort)
        self.assertFalse(res.ok)
        self.assertEqual(res.error, "ESTOP", res.lines)
        self.assertTrue(any(l.startswith("NGD:STOP") for l in res.lines), res.lines)
        # the stream is clean after the drained WANTs: the next request works on the same connection
        c.send(ngtp.PING)
        self.assertEqual(c.recv()[0], ngtp.PONG)
        res = c.put(text, Src(datas), mode=ngtp.PUT_RESUME, tag="0002")
        self.assertTrue(res.ok, res.lines)
        self.assertTrue(any(l.startswith("NGD:RESUME") for l in res.lines), res.lines)
        c.finish()
        srv.finish()
        self.check_folder(img, "03", datas)

    def test_b_stops_safely_and_does_not_leave(self):
        img = self.card()
        text, datas = self.gdi_set()
        srv = Serve(self, img, "--stopafter", "1", "--hold-b-ms", "300")
        c = srv.client()
        c.hello(CODE)
        res = c.put(text, Src(datas))
        self.assertEqual(res.error, "ESTOP", res.lines)
        res = c.put(text, Src(datas), mode=ngtp.PUT_RESUME, tag="0002")
        self.assertTrue(res.ok, res.lines)
        c.finish()
        srv.finish()
        self.assertEqual(srv.end["result"], R_FINISH, srv.text)
        self.check_folder(img, "03", datas)

    def test_pc_goes_away_then_resumes_in_the_open_session(self):
        img = self.card()
        text, datas = self.gdi_set()
        srv = Serve(self, img, "--depth", "2")
        c = srv.client()
        c.hello(CODE)
        with self.assertRaises(ngtp.ConnectionLost):
            c.put(text, Src(datas, close_at=4, sock=c.sock))
        c2 = srv.client()
        info = c2.hello(CODE)
        self.assertTrue(info.session_open)
        res = c2.put(text, Src(datas), mode=ngtp.PUT_RESUME, tag="0002")
        self.assertTrue(res.ok, res.lines)
        c2.finish()
        sim = srv.finish()
        self.assertEqual(sim["begin"], 1, srv.text)
        self.check_folder(img, "03", datas)

    def test_power_cut_then_resume(self):
        img = self.card()
        text, datas = self.gdi_set()
        srv = Serve(self, img, "--cut", "300")
        c = srv.client()
        c.hello(CODE)
        try:
            c.put(text, Src(datas))
        except ngtp.ConnectionLost:
            pass
        srv.finish(check=False)
        self.assertIn("POWERCUT", srv.text)
        srv = Serve(self, img)
        c = srv.client()
        c.hello(CODE)
        res = c.put(text, Src(datas), mode=ngtp.PUT_RESUME, tag="0002")
        self.assertTrue(res.ok, res.lines)
        c.finish()
        srv.finish()
        self.check_folder(img, "03", datas)


class TestChecksums(Base):
    def test_file_crc_mismatch_is_reported_and_resumable(self):
        img = self.card()
        text, datas = self.gdi_set(crc_of={2: b"something else"})
        srv = Serve(self, img)
        c = srv.client()
        c.hello(CODE)
        res = c.put(text, Src(datas))
        self.assertFalse(res.ok)
        self.assertEqual(res.error, "ECRC", res.lines)
        self.assertTrue(any("code=ECRC" in l and "file=3" in l for l in res.lines), res.lines)
        good_text, _ = self.gdi_set()
        res = c.put(good_text, Src(datas), mode=ngtp.PUT_OVERWRITE, tag="0002")
        self.assertTrue(res.ok, res.lines)
        c.finish()
        srv.finish()
        self.check_folder(img, "03", datas)

    def test_frame_crc_mismatch_on_data_drops_the_connection(self):
        img = self.card()
        text, datas = self.gdi_set()
        srv = Serve(self, img)
        c = srv.client()
        c.hello(CODE)
        seq = c.send(ngtp.PUT, put_payload(text))
        while True:
            t, fl, sq, p = c.recv()
            if t == ngtp.WANT:
                idx, _z, off, n = struct.unpack("<HHQI", p)
                f = bytearray(ngtp.build_frame(ngtp.DATA, struct.pack("<HHQ", idx, 0, off) + datas[idx][off:off + n], seq=seq))
                f[-1] ^= 0x55
                c._send(bytes(f))
            elif t == ngtp.ERROR:
                self.assertTrue(p.startswith(b"EBADCRC"), p)
                break
        c2 = srv.client()                       # the receiver is still there and the session can be resumed
        self.assertTrue(c2.hello(CODE).session_open)
        res = c2.put(text, Src(datas), mode=ngtp.PUT_RESUME, tag="0002")
        self.assertTrue(res.ok, res.lines)
        c2.finish()
        srv.finish()
        self.check_folder(img, "03", datas)


    def test_lost_last_segment_and_bare_fin_is_not_an_end_of_data(self):
        # The case the net_tcp.c fix (f02c1518) is about, seen from the receiver: the tail of a DATA frame never arrives and
        # the PC's FIN follows.  The receiver must not take the EOF for the end of the file: the frame length and CRC are
        # in the frame, so a cut frame is a broken link (ESRC, checkpoint kept), never a short file.
        img = self.card()
        text, datas = self.gdi_set()
        srv = Serve(self, img)
        c = srv.client()
        c.hello(CODE)
        seq = c.send(ngtp.PUT, put_payload(text))
        lines, got_done = [], None
        while True:
            try:
                t, fl, sq, p = c.recv()
            except ngtp.ConnectionLost:
                break
            if t == ngtp.LINE:
                lines.append(p.decode())
            elif t == ngtp.DONE:
                got_done = p
                break
            elif t == ngtp.WANT:
                idx, _z, off, n = struct.unpack("<HHQI", p)
                f = ngtp.build_frame(ngtp.DATA, struct.pack("<HHQ", idx, 0, off) + datas[idx][off:off + n], seq=seq)
                if idx == 2 and off + n == len(datas[2]):       # the last piece of track02: only part of it, then FIN
                    c._send(f[:len(f) // 2])
                    c.sock.shutdown(socket.SHUT_WR)
                else:
                    c._send(f)
        self.assertTrue(got_done is None or got_done[0] == 0, (got_done, lines))
        self.assertFalse([l for l in lines if l.startswith("NGD:FILE") and "file=3 " in l], lines)
        c2 = srv.client()
        self.assertTrue(c2.hello(CODE).session_open)
        res = c2.put(text, Src(datas), mode=ngtp.PUT_RESUME, tag="0002")
        self.assertTrue(res.ok, res.lines)
        self.assertTrue(any(l.startswith("NGD:RESUME") and "file=3" in l for l in res.lines), res.lines)
        c2.finish()
        srv.finish()
        self.check_folder(img, "03", datas)


def data_frame(datas, w, seq, off=None, n=None, cut=None):
    idx, _z, woff, wn = w
    off = woff if off is None else off
    n = wn if n is None else n
    return ngtp.build_frame(ngtp.DATA, struct.pack("<HHQ", idx, 0, off) + datas[idx][off:off + n], seq=seq)


class TestProtocol(Base):
    """the receiver's checks on the PC's DATA frames (menu-app.md 8.3), with a raw client"""

    def raw_put(self, srv, text, datas, answer, hold=1):
        """PUT; WANTs are collected until `hold` are outstanding (or the file has no more chunks), then
        answer(pending, seq) returns the bytes to send.  -> (kind, payload, lines): kind DONE / ERROR / LOST"""
        c = srv.client()
        c.hello(CODE)
        seq = c.send(ngtp.PUT, put_payload(text))
        pending, lines = [], []
        while True:
            try:
                t, fl, sq, p = c.recv()
            except ngtp.ConnectionLost:
                return "LOST", b"", lines
            if t == ngtp.LINE:
                lines.append(p.decode())
            elif t == ngtp.WANT:
                w = struct.unpack("<HHQI", p)
                pending.append(w)
                if len(pending) >= hold or w[2] + w[3] >= len(datas[w[0]]):
                    out = answer(pending, seq)
                    pending = []
                    try:
                        for i, part in enumerate(out if isinstance(out, list) else [out]):
                            if i:
                                time.sleep(0.3)         # the receiver has taken the earlier part before this one comes
                            c._send(part)
                    except ngtp.ConnectionLost:
                        return "LOST", b"", lines
            elif t == ngtp.DONE:
                return "DONE", p, lines
            elif t == ngtp.ERROR:
                return "ERROR", p, lines

    def resume_and_check(self, srv, img, text, datas, expect_file=None):
        c = srv.client()
        c.hello(CODE)
        res = c.put(text, Src(datas), mode=ngtp.PUT_RESUME, tag="0002")
        self.assertTrue(res.ok, res.lines)
        if expect_file:
            self.assertTrue(any(l.startswith("NGD:RESUME") and "file=%d" % expect_file in l for l in res.lines), res.lines)
        c.finish()
        srv.finish()
        self.check_folder(img, "03", datas)
        return res

    def ok_answer(self, datas):
        return lambda pending, seq: b"".join(data_frame(datas, w, seq) for w in pending)

    def test_data_after_a_short_data_is_refused(self):
        # depth 2: track03 chunk A gets 10 bytes (= its end), chunk B 64 KiB: B must not become bytes 10.. of the file
        img = self.card()
        text, datas = self.gdi_set()
        srv = Serve(self, img, "--want", "65536", "--depth", "2")
        ok = self.ok_answer(datas)

        def answer(pending, seq):
            if pending[0][0] == 3 and len(pending) == 2:
                return [data_frame(datas, pending[0], seq, n=10), data_frame(datas, pending[1], seq)]
            return ok(pending, seq)
        kind, p, lines = self.raw_put(srv, text, datas, answer, hold=2)
        self.assertEqual((kind, p[:6]), ("ERROR", b"ESTATE"), (p, lines))
        self.assertIn(b"after a short DATA", p)
        self.assertFalse([l for l in lines if l.startswith("NGD:FILE") and "file=4 " in l], lines)
        self.resume_and_check(srv, img, text, datas, expect_file=4)

    def test_out_of_order_data_is_refused(self):
        img = self.card()
        text, datas = self.gdi_set()
        srv = Serve(self, img, "--want", "16384", "--depth", "3")

        def answer(pending, seq):
            if len(pending) >= 2:
                return data_frame(datas, pending[1], seq)     # the answer to the second WANT comes first
            return data_frame(datas, pending[0], seq)
        kind, p, lines = self.raw_put(srv, text, datas, answer, hold=3)
        self.assertEqual((kind, p[:6]), ("ERROR", b"ESTATE"), (p, lines))
        self.resume_and_check(srv, img, text, datas)

    def test_offset_mismatch_is_refused(self):
        img = self.card()
        text, datas = self.gdi_set()
        srv = Serve(self, img)
        kind, p, lines = self.raw_put(srv, text, datas, lambda pend, seq: data_frame(datas, pend[0], seq, off=pend[0][2] + 1))
        self.assertEqual((kind, p[:6]), ("ERROR", b"ESTATE"), (p, lines))
        self.resume_and_check(srv, img, text, datas)

    def test_oversized_data_is_refused(self):
        img = self.card()
        text, datas = self.gdi_set()
        srv = Serve(self, img, "--want", "1000")

        def answer(pending, seq):
            w = pending[0]
            if w[0] == 2:                                   # track02 is 70000 B: 1001 bytes for a 1000-byte WANT
                return data_frame(datas, w, seq, n=w[3] + 1)
            return data_frame(datas, w, seq)
        kind, p, lines = self.raw_put(srv, text, datas, answer)
        self.assertEqual((kind, p[:6]), ("ERROR", b"ESTATE"), (p, lines))
        self.resume_and_check(srv, img, text, datas, expect_file=3)

    def test_huge_length_field_is_refused(self):
        img = self.card()
        text, datas = self.gdi_set()
        srv = Serve(self, img)

        def answer(pending, seq):
            return b"NGT1" + bytes([ngtp.DATA, 0, 0, 0]) + struct.pack("<II", seq, 0xFFFFFFF0)    # the header only
        kind, p, lines = self.raw_put(srv, text, datas, answer)
        self.assertEqual((kind, p[:6]), ("ERROR", b"EPROTO"), (p, lines))
        # and as a request header
        c = srv.client()
        c._send(b"NGT1" + bytes([ngtp.PING, 0, 0, 0]) + struct.pack("<II", 1, 0xFFFFFFFF))
        t, fl, sq, p = c.recv()
        self.assertEqual((t, p[:6]), (ngtp.ERROR, b"EPROTO"))
        self.resume_and_check(srv, img, text, datas)

    def test_resume_from_a_checkpoint_inside_a_file(self):
        # checkpoints every 64 KiB: stopping in track03 (140001 B) leaves offset 65536 or 131072, so resume reopens the
        # file at off > 0 and the WANTs start there
        img = self.card()
        text, datas = self.gdi_set()
        srv = Serve(self, img, "--ckp", "65536", "--want", "16384", "--depth", "2")
        c = srv.client()
        c.hello(CODE)
        src = Src(datas)
        src.abort = lambda: sum(src.sizes) > len(datas[0]) + len(datas[1]) + len(datas[2]) + 100000
        res = c.put(text, src, abort=src.abort)
        self.assertEqual(res.error, "ESTOP", res.lines)
        src2 = Src(datas)
        offs = []
        orig = src2.read

        def rd(idx, off, n):
            offs.append((idx, off))
            return orig(idx, off, n)
        src2.read = rd
        res = c.put(text, src2, mode=ngtp.PUT_RESUME, tag="0002")
        self.assertTrue(res.ok, res.lines)
        m = [re.search(r"file=(\d+) offset=(\d+)", l) for l in res.lines if l.startswith("NGD:RESUME")]
        self.assertTrue(m and m[0], res.lines)
        self.assertEqual(m[0].group(1), "4")
        self.assertIn(int(m[0].group(2)), (65536, 131072))
        self.assertEqual(offs[0], (3, int(m[0].group(2))))
        c.finish()
        srv.finish()
        self.check_folder(img, "03", datas)

    def test_verify_after_an_aborted_put_is_not_stopped(self):
        img = self.card()
        text, datas = self.gdi_set()
        srv = Serve(self, img, "--tcpfix")
        c = srv.client()
        c.hello(CODE)
        self.assertTrue(c.put(text, Src(datas)).ok)
        text4, datas4 = self.gdi_set(folder="04")
        src = Src(datas4, abort_at=3)
        self.assertEqual(c.put(text4, src, abort=src.abort, tag="0002").error, "ESTOP")
        res = c.verify("03")
        self.assertTrue(res.ok, res.lines)
        self.assertFalse([l for l in res.lines if l.startswith("NGD:STOP")], res.lines)
        c.finish()
        srv.finish()
        self.check_folder(img, "03", datas)


class TestWriteGate(Base):
    def refused(self, *opts):
        img = self.card()
        before = self.sha(img)
        text, datas = self.gdi_set()
        srv = Serve(self, img, *opts)
        c = srv.client()
        c.hello(CODE)
        res = c.put(text, Src(datas))
        self.assertEqual(res.error, "EFW", res.lines)
        res2 = c.statfs()
        self.assertEqual(res2.error, "EFW", res2.lines)
        c.finish()
        sim = srv.finish()
        self.assertEqual((sim["begin"], sim["end"], sim["sector_writes"], sim["cmd_writes"]), (0, 0, 0, 0), srv.text)
        self.assertEqual(self.sha(img), before)
        return srv, sim, res.lines

    def test_stock_gdemu_gets_only_the_recorded_52_00(self):
        # the stock model answers 52 00 like record 08 (5.20.5 / 2.5.0); it must never see an 0xE0 packet
        srv, sim, lines = self.refused("--stock", "--trace")
        self.assertEqual((sim["packets"], sim["e0"]), (1, 0), srv.text)
        self.assertEqual(re.findall(r"^FW (.*)$", srv.text, re.M), ["52 00 00 00 00 00 00 00 00 00 00 00"])
        self.assertLessEqual(sim["probe_limit"], 3000)
        self.assertTrue(any("GDEMU 5.20.5 is not NetGD" in l for l in lines), lines)
        self.assertEqual(srv.end["fw_ok"], 0)

    def test_stock_after_unit_attention_repeats_52_00_only(self):
        srv, sim, lines = self.refused("--stock", "--ua")
        self.assertEqual((sim["packets"], sim["e0"]), (2, 0), srv.text)

    def test_no_answer_to_52_00_sends_nothing_else(self):
        srv, sim, lines = self.refused("--nofw")
        self.assertEqual((sim["packets"], sim["e0"]), (1, 0), srv.text)
        self.assertTrue(any("did not answer 52 00" in l for l in lines), lines)

    def test_another_gdemu_version_gets_no_e0(self):
        srv, sim, lines = self.refused("--gdemu-version", "0x05140000")
        self.assertEqual((sim["packets"], sim["e0"]), (1, 0), srv.text)

    def test_e0_is_never_repeated_on_unit_attention(self):
        srv, sim, lines = self.refused("--ua-on-e0")
        self.assertEqual((sim["packets"], sim["e0"]), (2, 1), srv.text)
        self.assertLessEqual(sim["probe_limit"], 3000)
        self.assertTrue(any("no OGDM VERSION" in l for l in lines), lines)

    def test_netgd_without_write_session_caps(self):
        srv, sim, lines = self.refused("--caps", "1")
        self.assertEqual((sim["packets"], sim["e0"]), (2, 1), srv.text)
        self.assertTrue(any("checksum" in l for l in lines), lines)

    def test_no_drive_build_never_sends_a_packet(self):
        srv, sim, lines = self.refused("--no-drive")
        self.assertEqual((sim["packets"], srv.end["fw_checked"]), (0, 0), srv.text)

    def test_hello_and_ping_do_not_touch_the_drive(self):
        img = self.card()
        srv = Serve(self, img, "--stock", "--idle-exit-ms", "1500")
        c = srv.client()
        c.hello(CODE)
        c.send(ngtp.PING)
        self.assertEqual(c.recv()[0], ngtp.PONG)
        c.close()
        sim = srv.finish()
        self.assertEqual((sim["packets"], srv.end["result"]), (0, R_IDLE), srv.text)

    def test_wrong_code(self):
        img = self.card()
        srv = Serve(self, img, "--idle-exit-ms", "2000")
        with self.assertRaises(ngtp.NgtpError) as e:
            srv.client().hello(1111)
        self.assertEqual(e.exception.name, "EAUTH")
        sim = srv.finish()
        self.assertEqual(sim["packets"], 0, srv.text)


class TestBoundedWaits(Base):
    def test_no_client_exits_after_the_idle_limit(self):
        srv = Serve(self, self.card(), "--idle-exit-ms", "1500")
        srv.finish(timeout=30)
        self.assertEqual((srv.end["state"], srv.end["result"]), (S_DONE, R_IDLE), srv.text)

    def test_idle_session_is_closed_then_the_program_ends(self):
        img = self.card()
        text, datas = self.gdi_set()
        srv = Serve(self, img, "--idle-close-ms", "1000", "--idle-exit-ms", "4000")
        c = srv.client()
        c.hello(CODE)
        self.assertTrue(c.put(text, Src(datas)).ok)
        sim = srv.finish(timeout=30)
        self.assertEqual((sim["begin"], sim["end"], sim["endmode"], srv.end["result"]), (1, 1, 1, R_IDLE), srv.text)
        self.check_folder(img, "03", datas)

    def test_b_leaves_and_closes_the_session(self):
        img = self.card()
        text, datas = self.gdi_set()
        srv = Serve(self, img, "--leave-after-ms", "1500")
        c = srv.client()
        c.hello(CODE)
        self.assertTrue(c.put(text, Src(datas)).ok)
        sim = srv.finish(timeout=30)
        self.assertEqual((sim["end"], srv.end["result"]), (1, R_USER), srv.text)

    # A pad can report B all the time (real hardware: 400 of 400 samples).  B counts only after a stable release.

    def pad_run(self, script, *opts):
        srv = Serve(self, self.card(), "--pad", script, *opts)
        srv.finish(timeout=30)
        return srv, int(re.search(r"^T2-MS (\d+)$", srv.text, re.M).group(1))

    def test_b_held_from_the_start_neither_leaves_nor_stops(self):
        img = self.card()
        text, datas = self.gdi_set()
        srv = Serve(self, img, "--pad", "B:0")
        c = srv.client()
        c.hello(CODE)
        res = c.put(text, Src(datas))
        self.assertTrue(res.ok, res.lines)
        c.finish()
        srv.finish()
        self.assertEqual(srv.end["result"], R_FINISH, srv.text)
        self.check_folder(img, "03", datas)

    def test_b_held_then_released_then_pressed_leaves(self):
        # the hold (1200 ms) must really be ignored: the old code left on the first step
        srv, ms = self.pad_run("B:1200,-:500,B:0")
        self.assertEqual(srv.end["result"], R_USER, srv.text)
        self.assertGreaterEqual(ms, 1500, srv.text)

    def test_one_up_sample_is_not_a_release(self):
        srv, ms = self.pad_run("B:400,-:5,B:0", "--idle-exit-ms", "1500")
        self.assertEqual(srv.end["result"], R_IDLE, srv.text)

    def test_stuck_pad_replugged_is_still_ignored(self):
        # healthy first (armed), then unplugged, then a pad with B down comes back
        srv, ms = self.pad_run("-:500,x:300,B:0", "--idle-exit-ms", "2000")
        self.assertEqual(srv.end["result"], R_IDLE, srv.text)

    def test_pad_first_seen_with_b_down_is_ignored(self):
        srv, ms = self.pad_run("x:500,B:0", "--idle-exit-ms", "1500")
        self.assertEqual(srv.end["result"], R_IDLE, srv.text)

    def test_no_pad_is_not_a_release(self):
        srv, ms = self.pad_run("B:300,x:600,B:0", "--idle-exit-ms", "1800")
        self.assertEqual(srv.end["result"], R_IDLE, srv.text)

    def test_force_confirm_works_with_b_held(self):
        # with the level of B, hk_confirm refused at once and a stuck pad could never allow anything
        img = self.card({"/01/MENU.CDI": b"menu"})
        text, datas = self.gdi_set("01")
        srv = Serve(self, img, "--pad", "B:0", "--confirm", "allow")
        c = srv.client()
        c.hello(CODE)
        res = c.put(text, Src(datas), mode=ngtp.PUT_OVERWRITE, force=True)
        self.assertTrue(res.ok, res.lines)
        c.finish()
        srv.finish()
        self.check_folder(img, "01", datas)

    def force_refused(self, script, *opts):
        img = self.card({"/01/MENU.CDI": b"menu"})
        text, datas = self.gdi_set("01")
        srv = Serve(self, img, "--pad", script, "--confirm-max-ms", "2500", *opts)
        c = srv.client()
        c.hello(CODE)
        res = c.put(text, Src(datas), mode=ngtp.PUT_OVERWRITE, force=True)
        self.assertEqual(res.error, "EPROTECTED", res.lines)
        c.finish()
        srv.finish()
        self.assertIn("/01/MENU.CDI", fatcheck.Fat32(img).files)        # the protected image is untouched

    def test_stuck_a_and_b_never_approve(self):
        self.force_refused("C:0")

    def test_a_held_before_the_question_never_approves(self):
        self.force_refused("A:0")

    def test_all_zero_raw_status_never_approves(self):
        self.force_refused("Z:0")

    def test_all_zero_after_a_release_never_approves(self):
        # B stuck (so it cannot refuse), A seen released in the question, then every button "down": A must not count
        self.force_refused("B:0", "--confirm", "zero")

    def test_soft_reset_combo_still_leaves_with_b(self):
        srv, ms = self.pad_run("-:500,R:0", "--idle-exit-ms", "4000")
        self.assertEqual(srv.end["result"], R_USER, srv.text)

    def test_no_bba_fails_cleanly(self):
        srv = Serve(self, self.card(), "--nobba", "--dump")
        sim = srv.finish(timeout=30)
        self.assertEqual((srv.end["state"], srv.end["result"], sim["packets"]), (S_FAILED, R_NONET, 0), srv.text)
        self.assertIn("FAILED: no network", srv.text)


class TestPushCli(Base):
    """netgd-send.py push, the PC program a user runs, against the receiver"""

    def make_gdi(self):
        d = os.path.join(self.work, "src")
        os.makedirs(d)
        specs = [(1, 0, 4, 2352, "track01.bin", 4 * 2352), (2, 450, 0, 2352, "track02.raw", 30 * 2352),
                 (3, 45000, 4, 2048, "track03.bin", 70 * 2048)]
        lines = ["3"]
        for num, lba, ctrl, sec, nm, size in specs:
            with open(os.path.join(d, nm), "wb") as f:
                f.write(pattern(size, num))
            lines.append("%d %d %d %d %s 0" % (num, lba, ctrl, sec, nm))
        p = os.path.join(d, "mygame.gdi")
        with open(p, "wb") as f:
            f.write(("\r\n".join(lines) + "\r\n").encode())
        return p, {nm: pattern(size, num) for num, _l, _c, _s, nm, size in specs}

    def push(self, srv, gdi, *extra):
        return run_child([sys.executable, SEND_PY, "push", gdi, "--host", "127.0.0.1", "--port", str(srv.port),
                          "--code", str(CODE)] + list(extra))

    def test_push_slot_and_finish(self):
        for opts in ((), ("--tcpfix",)):
            with self.subTest(opts=opts):
                img = self.card()
                gdi, tracks = self.make_gdi()
                srv = Serve(self, img, *opts)
                r = self.push(srv, gdi, "--slot", "5")
                self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
                srv.finish()
                fc = fatcheck.Fat32(img)
                self.assertEqual(fc.errors, [])
                names = {k.lower(): k for k in fc.folder_files("/05")}
                for n, d in tracks.items():
                    self.assertEqual(fc.read_file("/05/" + names[n]), d, n)
                self.assertEqual(fc.read_file("/05/" + names["name.txt"]), b"mygame\n")
                shutil.rmtree(os.path.join(self.work, "src"))

    def test_push_to_a_stock_gdemu_fails_with_exit_1_and_writes_nothing(self):
        img = self.card()
        before = self.sha(img)
        gdi, _ = self.make_gdi()
        srv = Serve(self, img, "--stock", "--idle-exit-ms", "2000")
        r = self.push(srv, gdi, "--slot", "5")
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        srv.finish()
        self.assertEqual(self.sha(img), before)


if __name__ == "__main__":
    print("scratch: %s; child limit %g s" % (WORK_ROOT, CHILD_TIMEOUT), file=sys.stderr)
    unittest.main(verbosity=2)
