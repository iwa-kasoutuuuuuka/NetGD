"""NGTP version 1: the PC side of the transfer to the NetGD menu (docs/design/menu-app.md 8).

Frames (little endian): "NGT1", type, flags, 2 zero bytes, sequence u32, payload length u32, payload, CRC-32 (zlib)
of everything before it.  The Dreamcast is the TCP server (port 20039); the PC sends requests and answers the
Dreamcast's WANT frames with DATA.  Standard library only.  Not run against a real Dreamcast.
"""

import socket
import struct
import zlib

VERSION = 1
DEFAULT_PORT = 20039
MAX_DATA = 65536
MAX_PAYLOAD = 65536 + 16

# PC -> Dreamcast
HELLO, STATFS, LIST, PUT, DATA, DELETE, VERIFY, FINISH, PING = range(1, 10)
# Dreamcast -> PC
HELLO_OK, PONG, LINE, DONE, WANT, ERROR = 0x81, 0x89, 0x90, 0x91, 0x92, 0x7F

PUT_NEW, PUT_RESUME, PUT_OVERWRITE = 0, 1, 2
FINISH_NONE, FINISH_DEFAULT, FINISH_MOUNT = 0, 1, 2


class NgtpError(Exception):
    """The Dreamcast answered with an ERROR frame (the connection is closed after it)."""

    def __init__(self, name, text=""):
        Exception.__init__(self, "%s %s" % (name, text) if text else name)
        self.name, self.text = name, text


class ConnectionLost(Exception):
    """The connection broke or timed out."""


def build_frame(type_, payload=b"", flags=0, seq=1):
    hdr = b"NGT1" + bytes([type_, flags, 0, 0]) + struct.pack("<II", seq, len(payload))
    return hdr + payload + struct.pack("<I", zlib.crc32(hdr + payload) & 0xFFFFFFFF)


def parse_header(hdr):
    """-> (type, flags, seq, length); ValueError on a bad header"""
    if len(hdr) != 16 or hdr[:4] != b"NGT1" or hdr[6] or hdr[7]:
        raise ValueError("bad NGTP header")
    n = struct.unpack_from("<I", hdr, 12)[0]
    if n > MAX_PAYLOAD:
        raise ValueError("NGTP payload of %d bytes is over the limit" % n)
    return hdr[4], hdr[5], struct.unpack_from("<I", hdr, 8)[0], n


class OpResult(object):
    """The outcome of one request: `lines` are the NGD: lines the Dreamcast sent, `ok` the DONE status."""

    def __init__(self, ok, error, lines):
        self.ok, self.error, self.lines = ok, error, lines


class HelloInfo(object):
    def __init__(self, payload):
        (self.version, flags, self.max_data) = struct.unpack_from("<HHI", payload, 0)
        self.session_open = bool(flags & 1)
        self.proto, self.caps = payload[8], payload[9]
        self.firmware = payload[10:34].split(b"\0")[0].decode("ascii", "replace")
        self.app = payload[34:].decode("ascii", "replace")


class Client(object):
    def __init__(self, host, port=DEFAULT_PORT, timeout=30.0):
        try:
            self.sock = socket.create_connection((host, port), timeout=timeout)
        except OSError as e:
            raise ConnectionLost("cannot connect to %s:%d: %s" % (host, port, e))
        self.sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        self.seq = 0

    def close(self):
        try:
            self.sock.close()
        except OSError:
            pass

    def __enter__(self):
        return self

    def __exit__(self, *a):
        self.close()

    # ---- frames
    def _read(self, n):
        buf = bytearray()
        while len(buf) < n:
            try:
                chunk = self.sock.recv(n - len(buf))
            except (OSError, socket.timeout) as e:
                raise ConnectionLost("receive failed: %s" % e)
            if not chunk:
                raise ConnectionLost("the Dreamcast closed the connection")
            buf += chunk
        return bytes(buf)

    def _send(self, data):
        try:
            self.sock.sendall(data)
        except OSError as e:
            raise ConnectionLost("send failed: %s" % e)

    def send(self, type_, payload=b"", flags=0, seq=None):
        self.seq = self.seq + 1 if seq is None else seq
        self._send(build_frame(type_, payload, flags, self.seq))
        return self.seq

    def recv(self):
        """-> (type, flags, seq, payload)"""
        hdr = self._read(16)
        try:
            type_, flags, seq, n = parse_header(hdr)
        except ValueError as e:
            raise ConnectionLost(str(e))
        body = self._read(n + 4)
        payload, crc = body[:n], struct.unpack("<I", body[n:])[0]
        if zlib.crc32(hdr + payload) & 0xFFFFFFFF != crc:
            raise ConnectionLost("a frame from the Dreamcast has a bad CRC")
        return type_, flags, seq, payload

    # ---- requests
    def hello(self, code, name="netgd-send"):
        self.send(HELLO, struct.pack("<HH", VERSION, int(code)) + name.encode("ascii", "replace")[:30])
        type_, flags, seq, p = self.recv()
        if type_ == ERROR:
            text = p.decode("ascii", "replace")
            raise NgtpError(*(text.split(" ", 1) if " " in text else (text, "")))
        if type_ != HELLO_OK:
            raise ConnectionLost("unexpected frame %#x after HELLO" % type_)
        return HelloInfo(p)

    def request(self, type_, payload=b"", source=None, on_line=None, abort=None):
        """Send one request and follow it to its DONE.

        source: object with read(index, offset, n) -> bytes, answers the WANT frames of a PUT.
        on_line(text, throttled): each NGD: line as it arrives.  abort(): true = set the abort flag on the next DATA.
        Returns an OpResult; raises NgtpError (an ERROR frame) or ConnectionLost."""
        seq = self.send(type_, payload)
        lines = []
        while True:
            t, flags, sq, p = self.recv()
            if t == LINE:
                text = p.decode("utf-8", "replace")
                lines.append(text)
                if on_line:
                    on_line(text, bool(flags & 1))
            elif t == WANT:
                idx, _z, off, n = struct.unpack("<HHQI", p)
                if source is None:
                    raise ConnectionLost("the Dreamcast wants file data for a request that has none")
                data = source.read(idx, off, n)
                f = 1 if (abort and abort()) else 0
                self._send(build_frame(DATA, struct.pack("<HHQ", idx, 0, off) + data, f, sq))
            elif t == DONE:
                return OpResult(p[0] == 1, p[1:].decode("ascii", "replace"), lines)
            elif t == ERROR:
                text = p.decode("ascii", "replace")
                raise NgtpError(*(text.split(" ", 1) if " " in text else (text, "")))
            else:
                raise ConnectionLost("unexpected frame %#x" % t)

    def put(self, manifest_text, source, mode=PUT_RESUME, force=False, allow_fragmented=False, tag="0001",
            on_line=None, abort=None):
        flags = (1 if force else 0) | (2 if allow_fragmented else 0)
        payload = struct.pack("<BBH", mode, flags, 0) + tag.encode("ascii") + manifest_text.encode("utf-8")
        return self.request(PUT, payload, source=source, on_line=on_line, abort=abort)

    def statfs(self, on_line=None):
        return self.request(STATFS, on_line=on_line)

    def list(self, folder="/", on_line=None):
        return self.request(LIST, folder.encode("ascii"), on_line=on_line)

    def delete(self, folder, force=False, on_line=None):
        return self.request(DELETE, bytes([1 if force else 0]) + folder.encode("ascii"), on_line=on_line)

    def verify(self, folder, on_line=None):
        return self.request(VERIFY, folder.encode("ascii"), on_line=on_line)

    def finish(self, mode=FINISH_DEFAULT, folder="", on_line=None):
        return self.request(FINISH, bytes([mode]) + folder.encode("ascii"), on_line=on_line)


# ---- UDP discovery (docs/design/menu-app.md 8.1, step 9) ----
DISCOVER_REQUEST = b"NGTP-DISCOVER 1"
DISCOVER_BROADCAST = "255.255.255.255"


class Found(object):
    """one menu that answered: `host` is the address the reply came from"""

    def __init__(self, host, name, app, state):
        self.host, self.name, self.app, self.state = host, name, app, state

    def __repr__(self):
        return "Found(%s %s app=%s state=%s)" % (self.host, self.name, self.app, self.state)


def parse_here(data):
    """'NGTP-HERE 1 name=NetGD app=0.1.0 state=idle' -> {'name': ..., 'app': ..., 'state': ...}; None if it is not a version 1 reply"""
    try:
        words = data.decode("ascii").split()
    except UnicodeDecodeError:
        return None
    if len(words) < 2 or words[0] != "NGTP-HERE" or words[1] != "1":
        return None
    kv = {}
    for w in words[2:]:
        if "=" in w:
            k, v = w.split("=", 1)
            kv[k] = v
    return kv


def discover(timeout=2.0, targets=(DISCOVER_BROADCAST,), port=DEFAULT_PORT, bind=""):
    """Send the discovery request to each of `targets` (UDP, `port`) and collect the answers for `timeout` seconds.
    Returns a list of Found, one per answering host (the first answer of a host counts).  Never raises for network
    trouble: no answer is an empty list."""
    import select
    import time
    found, seen = [], set()
    try:
        s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    except OSError:
        return found
    try:
        s.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
        s.bind((bind, 0))
        for t in targets:
            try:
                s.sendto(DISCOVER_REQUEST, (t, port))
            except OSError:
                pass                    # an unreachable network or address: the other targets may still work
        end = time.time() + timeout
        while True:
            left = end - time.time()
            if left <= 0:
                break
            r, _w, _x = select.select([s], [], [], left)
            if not r:
                break
            try:
                data, addr = s.recvfrom(512)
            except OSError:
                continue                # Windows reports an ICMP error of an earlier datagram here
            kv = parse_here(data)
            if kv is None or addr[0] in seen:
                continue
            seen.add(addr[0])
            found.append(Found(addr[0], kv.get("name", ""), kv.get("app", ""), kv.get("state", "")))
    finally:
        s.close()
    return found
