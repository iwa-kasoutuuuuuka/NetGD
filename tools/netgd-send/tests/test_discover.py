"""Tests of the UDP discovery on the PC side (netgd_send/ngtp.py discover, `discover`, `push`/`remote` without --host)
against a small fake menu.  The real menu (menu_host) is exercised by tools/netgd-menu/test/run_tests.py.
Run: python -m unittest discover -s tests -v"""

import io
import os
import socket
import sys
import threading
import unittest
from contextlib import redirect_stdout

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
sys.path.insert(0, HERE)

from netgd_send import ngtp  # noqa: E402
from netgd_send import push as pushmod  # noqa: E402
from netgd_send.__main__ import main  # noqa: E402
from test_netgd_send import Tmp  # noqa: E402
from test_ngtp import FakeMenu  # noqa: E402


class FakeBeacon(object):
    """answers UDP datagrams on 127.0.0.1:port (0 = any free port) with `replies` (a list of byte strings, all sent for each
    request); remembers what it was asked"""

    def __init__(self, replies=(b"NGTP-HERE 1 name=NetGD app=0.1.0 state=idle",), port=0):
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.sock.bind(("127.0.0.1", port))
        self.sock.settimeout(0.2)
        self.port = self.sock.getsockname()[1]
        self.replies = list(replies)
        self.asked = []
        self.stop = False
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def run(self):
        while not self.stop:
            try:
                data, addr = self.sock.recvfrom(512)
            except socket.timeout:
                continue
            except OSError:
                return
            self.asked.append(data)
            for r in self.replies:
                self.sock.sendto(r, addr)

    def close(self):
        self.stop = True
        self.thread.join(2)
        self.sock.close()


def free_udp_port():
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


class TestParse(unittest.TestCase):
    def test_parse_here(self):
        self.assertEqual(ngtp.parse_here(b"NGTP-HERE 1 name=NetGD app=0.1.0 state=idle"),
                         {"name": "NetGD", "app": "0.1.0", "state": "idle"})
        self.assertEqual(ngtp.parse_here(b"NGTP-HERE 1"), {})
        self.assertEqual(ngtp.parse_here(b"NGTP-HERE 1 name=A future=thing junk"), {"name": "A", "future": "thing"})
        self.assertIsNone(ngtp.parse_here(b"NGTP-HERE 2 name=NetGD"))
        self.assertIsNone(ngtp.parse_here(b"NGTP-DISCOVER 1"))
        self.assertIsNone(ngtp.parse_here(b"\xff\xfe"))
        self.assertIsNone(ngtp.parse_here(b""))


class TestDiscover(unittest.TestCase):
    def test_finds_a_menu_and_sends_the_request(self):
        b = FakeBeacon()
        self.addCleanup(b.close)
        found = ngtp.discover(1.0, targets=("127.0.0.1",), port=b.port)
        self.assertEqual([(f.host, f.name, f.app, f.state) for f in found], [("127.0.0.1", "NetGD", "0.1.0", "idle")])
        self.assertEqual(b.asked, [b"NGTP-DISCOVER 1"])

    def test_junk_and_duplicate_answers_are_dropped(self):
        b = FakeBeacon(replies=(b"garbage", b"NGTP-HERE 9 name=X", b"NGTP-HERE 1 name=NetGD app=1 state=busy",
                                b"NGTP-HERE 1 name=NetGD app=1 state=busy"))
        self.addCleanup(b.close)
        found = ngtp.discover(0.8, targets=("127.0.0.1",), port=b.port)
        self.assertEqual(len(found), 1)
        self.assertEqual(found[0].state, "busy")

    def test_no_answer_is_an_empty_list_and_unreachable_targets_do_not_raise(self):
        self.assertEqual(ngtp.discover(0.3, targets=("127.0.0.1",), port=free_udp_port()), [])
        self.assertEqual(ngtp.discover(0.3, targets=("not an address", "127.0.0.1"), port=free_udp_port()), [])


class TestResolve(Tmp):
    def args(self, **kw):
        class A(object):
            host = None
            port = 20039
            discover_timeout = 0.5
            broadcast = "127.0.0.1"
        a = A()
        for k, v in kw.items():
            setattr(a, k, v)
        return a

    def test_a_given_host_is_used_without_any_discovery(self):
        lines = []
        self.assertEqual(pushmod.resolve_host(self.args(host="10.0.0.5", port=free_udp_port()), lines.append), "10.0.0.5")
        self.assertEqual(lines, [])

    def test_one_answer_is_used_and_busy_is_warned_about(self):
        b = FakeBeacon(replies=(b"NGTP-HERE 1 name=NetGD app=0.1.0 state=busy",))
        self.addCleanup(b.close)
        lines = []
        self.assertEqual(pushmod.resolve_host(self.args(port=b.port), lines.append), "127.0.0.1")
        self.assertTrue(any("busy" in ln for ln in lines), lines)

    def test_several_answers_are_refused(self):
        real = ngtp.discover
        ngtp.discover = lambda *a, **k: [ngtp.Found("10.0.0.2", "NetGD", "1", "idle"), ngtp.Found("10.0.0.3", "NetGD", "1", "idle")]
        self.addCleanup(setattr, ngtp, "discover", real)
        lines = []
        self.assertIsNone(pushmod.resolve_host(self.args(), lines.append))
        self.assertIn("10.0.0.2", "\n".join(lines))
        self.assertIn("10.0.0.3", "\n".join(lines))

    def test_no_answer_is_refused(self):
        lines = []
        self.assertIsNone(pushmod.resolve_host(self.args(port=free_udp_port()), lines.append))
        self.assertIn("--host", "\n".join(lines))


class TestCommands(Tmp):
    def run_main(self, *argv):
        out = io.StringIO()
        with redirect_stdout(out):
            rc = main(list(argv))
        return rc, out.getvalue()

    def test_discover_command(self):
        b = FakeBeacon()
        self.addCleanup(b.close)
        rc, text = self.run_main("discover", "--broadcast", "127.0.0.1", "--port", str(b.port), "--discover-timeout", "1")
        self.assertEqual(rc, 0)
        self.assertIn("127.0.0.1  NetGD  app 0.1.0  idle", text)
        rc, text = self.run_main("discover", "--broadcast", "127.0.0.1", "--port", str(free_udp_port()), "--discover-timeout", "0.3")
        self.assertEqual(rc, 1)
        self.assertIn("no menu answered", text)

    def test_push_without_host_discovers_then_pushes(self):
        gdi = self.make_gdi()
        menu = FakeMenu(listing=["NGD:LIST complete 02"])
        beacon = FakeBeacon(port=menu.port)                # the TCP and the UDP port numbers are separate spaces
        self.addCleanup(beacon.close)
        rc, text = self.run_main("push", gdi, "--code", "4821", "--port", str(menu.port), "--broadcast", "127.0.0.1",
                                 "--discover-timeout", "1", "--timeout", "20", "--title", "T")
        menu.thread.join(10)
        self.assertEqual(rc, 0, text)
        self.assertIn("found NetGD at 127.0.0.1", text)
        self.assertEqual(beacon.asked, [b"NGTP-DISCOVER 1"])
        self.assertEqual(menu.requests[0], ngtp.HELLO)

    def test_push_without_host_checks_the_image_before_looking(self):
        b = FakeBeacon()
        self.addCleanup(b.close)
        rc = None
        out = io.StringIO()
        with redirect_stdout(out):
            rc = main(["push", self.path("nope.gdi"), "--code", "1", "--port", str(b.port), "--broadcast", "127.0.0.1"])
        self.assertEqual(rc, 1)
        self.assertEqual(b.asked, [])

    def test_remote_without_host_and_without_a_menu_exits_1(self):
        rc, text = self.run_main("remote", "statfs", "--code", "1", "--port", str(free_udp_port()), "--broadcast", "127.0.0.1",
                                 "--discover-timeout", "0.3")
        self.assertEqual(rc, 1)
        self.assertIn("no menu answered", text)


if __name__ == "__main__":
    unittest.main()
