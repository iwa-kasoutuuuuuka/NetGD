"""Tests of netgd-send's `push` / `remote` and the NGTP client (netgd_send/ngtp.py) against a small fake menu in
Python.  The real menu (menu_host) is exercised by tools/netgd-menu/test/run_tests.py.
Run: python -m unittest discover -s tests -v"""

import io
import os
import socket
import struct
import sys
import threading
import unittest
import zlib
from contextlib import redirect_stdout

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
sys.path.insert(0, HERE)

from netgd_send import manifest as mf  # noqa: E402
from netgd_send import ngtp  # noqa: E402
from netgd_send import push as pushmod  # noqa: E402
from netgd_send.__main__ import main  # noqa: E402
from test_netgd_send import Tmp  # noqa: E402


class FakeMenu(object):
    """Speaks the Dreamcast side of NGTP well enough to take one PUT.  `script` options:
    code (default 4821), listing (lines for LIST), fail_done (DONE failure name for PUT), cut_after_wants (close
    the socket after this many WANTs)."""

    def __init__(self, code=4821, listing=("NGD:LIST complete 02",), fail_done=None, cut_after_wants=None, put_errors=None):
        self.code, self.listing, self.fail_done, self.cut_after_wants = code, list(listing), fail_done, cut_after_wants
        self.put_errors = dict(put_errors or {})     # folder -> the DONE error for a PUT into it (e.g. "EEXIST")
        self.put_folders = []                        # the folder of each PUT, in order
        self.srv = socket.socket()
        self.srv.bind(("127.0.0.1", 0))
        self.srv.listen(1)
        self.port = self.srv.getsockname()[1]
        self.received = {}          # file index -> bytes
        self.manifest = None
        self.requests = []
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def read(self, c, n):
        buf = b""
        while len(buf) < n:
            chunk = c.recv(n - len(buf))
            if not chunk:
                raise EOFError
            buf += chunk
        return buf

    def frame(self, c):
        hdr = self.read(c, 16)
        t, fl, sq, n = ngtp.parse_header(hdr)
        body = self.read(c, n + 4)
        assert zlib.crc32(hdr + body[:n]) & 0xFFFFFFFF == struct.unpack("<I", body[n:])[0]
        return t, fl, sq, body[:n]

    def run(self):
        c = None
        try:
            c, _ = self.srv.accept()
            c.settimeout(20)
            while True:
                t, fl, sq, p = self.frame(c)
                self.requests.append(t)
                if t == ngtp.HELLO:
                    ver, code = struct.unpack_from("<HH", p)
                    if code != self.code:
                        c.sendall(ngtp.build_frame(ngtp.ERROR, b"EAUTH wrong code", 0, sq))
                        return
                    c.sendall(ngtp.build_frame(ngtp.HELLO_OK, struct.pack("<HHIBB", 1, 0, 65536, 3, 7) +
                                               b"fake-fw".ljust(24, b"\0") + b"0.0", 0, sq))
                elif t == ngtp.LIST:
                    for ln in self.listing:
                        c.sendall(ngtp.build_frame(ngtp.LINE, ln.encode(), 0, sq))
                    c.sendall(ngtp.build_frame(ngtp.DONE, b"\1", 0, sq))
                elif t == ngtp.PUT:
                    self.mode, self.flags = p[0], p[1]
                    self.manifest = mf.parse_manifest(p[8:].decode("utf-8"))
                    self.put_folders.append(self.manifest.image)
                    if self.manifest.image in self.put_errors:
                        c.sendall(ngtp.build_frame(ngtp.DONE, b"\0" + self.put_errors[self.manifest.image].encode(), 0, sq))
                        continue
                    wants = 0
                    for idx, f in enumerate(self.manifest.files):
                        off = 0
                        buf = b""
                        while off < f.size:
                            n = min(65536, f.size - off)
                            c.sendall(ngtp.build_frame(ngtp.WANT, struct.pack("<HHQI", idx, 0, off, n), 0, sq))
                            wants += 1
                            if self.cut_after_wants is not None and wants > self.cut_after_wants:
                                c.close()
                                return
                            t2, fl2, sq2, p2 = self.frame(c)
                            assert t2 == ngtp.DATA and sq2 == sq
                            i2, _z, o2, = struct.unpack_from("<HHQ", p2)
                            assert (i2, o2) == (idx, off)
                            buf += p2[12:]
                            off += len(p2) - 12
                        self.received[idx] = buf
                        c.sendall(ngtp.build_frame(ngtp.LINE, ("NGD:FILE 0001 file=%d size=%d crc=%08X" %
                                                               (idx + 1, f.size, f.crc)).encode(), 0, sq))
                    if self.fail_done:
                        c.sendall(ngtp.build_frame(ngtp.DONE, b"\0" + self.fail_done.encode(), 0, sq))
                    else:
                        c.sendall(ngtp.build_frame(ngtp.LINE, b"NGD:DONE 0001 ok", 0, sq))
                        c.sendall(ngtp.build_frame(ngtp.DONE, b"\1", 0, sq))
                elif t == ngtp.FINISH:
                    self.finish = (p[0], p[1:].decode())
                    c.sendall(ngtp.build_frame(ngtp.LINE, b"NGD:FINISH ok", 0, sq))
                    c.sendall(ngtp.build_frame(ngtp.DONE, b"\1", 0, sq))
                    return
                else:
                    c.sendall(ngtp.build_frame(ngtp.ERROR, b"EPROTO unknown", 0, sq))
                    return
        except (EOFError, OSError):
            pass
        finally:
            if c is not None:
                c.close()
            self.srv.close()


class TestFrames(unittest.TestCase):
    def test_round_trip_and_header_checks(self):
        f = ngtp.build_frame(ngtp.LINE, b"NGD:HELLO x", 1, 0x01020304)
        self.assertEqual(f[:4], b"NGT1")
        self.assertEqual(ngtp.parse_header(f[:16]), (ngtp.LINE, 1, 0x01020304, 11))
        self.assertEqual(struct.unpack("<I", f[-4:])[0], zlib.crc32(f[:-4]) & 0xFFFFFFFF)
        self.assertEqual(len(f), 16 + 11 + 4)
        for bad in (b"XXXX" + f[4:16], f[:6] + b"\1\0" + f[8:16], f[:12] + struct.pack("<I", ngtp.MAX_PAYLOAD + 1)):
            self.assertRaises(ValueError, ngtp.parse_header, bad)
        self.assertEqual(ngtp.parse_header(f[:12] + struct.pack("<I", ngtp.MAX_PAYLOAD))[3], ngtp.MAX_PAYLOAD)

    def test_slot_choice(self):
        lines = ["NGD:LIST complete 02", "NGD:LIST partial 003", "NGD:LIST complete Crazy Taxi", "NGD:LIST other 05/x",
                 "NGD:LIST complete 1", "NGD:LIST complete 00", "NGD:LIST complete 07 Title"]
        used = pushmod.used_slot_numbers(lines)
        self.assertEqual(used, {2, 3, 5})
        self.assertEqual(pushmod.first_free_slot(used), 4)
        self.assertEqual(pushmod.first_free_slot(set()), 2)
        self.assertEqual(pushmod.first_free_slot({2, 3, 4, 5, 6}), 7)

    def test_partial_slot_numbers(self):
        lines = ["NGD:LIST partial 03", "NGD:LIST complete 02", "NGD:LIST partial 01", "NGD:LIST partial 12",
                 "NGD:LIST partial Crazy Taxi", "NGD:LIST partial 05/sub", "NGD:LIST other 06", "NGD:LIST partial 1000"]
        self.assertEqual(pushmod.partial_slot_numbers(lines), [3, 12, 1000])

    def test_ascii_title(self):
        self.assertEqual(pushmod.ascii_title("  Crazy \t Taxi\n"), "Crazy Taxi")
        self.assertEqual(pushmod.ascii_title("Café"), "Caf?")
        self.assertEqual(len(pushmod.ascii_title("x" * 100)), 64)


class TestPush(Tmp):
    def run_push(self, menu, image, *extra):
        out = io.StringIO()
        with redirect_stdout(out):
            rc = main(["push", image, "--host", "127.0.0.1", "--port", str(menu.port), "--code", "4821",
                       "--timeout", "20"] + list(extra))
        menu.thread.join(10)
        return rc, out.getvalue()

    def test_push_gdi_picks_the_free_slot_and_adds_name_txt(self):
        gdi = self.make_gdi()
        menu = FakeMenu(listing=["NGD:LIST complete 02", "NGD:LIST complete 03"])
        rc, text = self.run_push(menu, gdi, "--title", "My Game")
        self.assertEqual(rc, 0, text)
        self.assertIn("using the free folder 04", text)
        m = menu.manifest
        self.assertEqual((m.image, m.type, m.descriptor), ("04", "gdi", "disc.gdi"))
        self.assertEqual([f.name for f in m.files], ["disc.gdi", "track01.bin", "track02.raw", "track03.bin", "name.txt"])
        self.assertEqual(menu.mode, ngtp.PUT_RESUME)
        for i, f in enumerate(m.files):
            self.assertEqual(len(menu.received[i]), f.size)
            self.assertEqual(zlib.crc32(menu.received[i]) & 0xFFFFFFFF, f.crc)
        self.assertEqual(menu.received[4], b"My Game\n")
        with open(self.path("src", "track01.bin"), "rb") as h:
            self.assertEqual(menu.received[1], h.read())
        self.assertIn(b"track03.bin", menu.received[0])                      # the rewritten descriptor, from memory
        self.assertEqual(menu.finish, (1, ""))
        self.assertIn("RESULT: success", text)

    def test_push_with_name_mount_and_flags(self):
        gdi = self.make_gdi()
        menu = FakeMenu()
        rc, text = self.run_push(menu, gdi, "--name", "GAMES/X", "--mount", "--mode", "overwrite", "--force",
                                 "--allow-fragmented")
        self.assertEqual(rc, 0, text)
        self.assertEqual(menu.manifest.image, "GAMES/X")
        self.assertEqual((menu.mode, menu.flags), (ngtp.PUT_OVERWRITE, 3))
        self.assertEqual(menu.finish, (2, "GAMES/X"))
        self.assertNotIn(ngtp.LIST, menu.requests)                           # no slot search for a given name

    def test_no_finish(self):
        menu = FakeMenu()
        rc, text = self.run_push(menu, self.make_gdi(), "--slot", "9", "--no-finish")
        self.assertEqual(rc, 0, text)
        self.assertFalse(hasattr(menu, "finish"))
        self.assertEqual(menu.manifest.image, "09")

    def test_wrong_code_exit_1(self):
        menu = FakeMenu(code=1234)
        rc, text = self.run_push(menu, self.make_gdi())
        self.assertEqual(rc, 1, text)
        self.assertIn("EAUTH", text)

    # ---- R104 finding 2: --slot auto goes on in the unfinished numbered folder ----
    def test_auto_resumes_the_partial_folder_before_taking_a_new_one(self):
        menu = FakeMenu(listing=["NGD:LIST complete 02", "NGD:LIST partial 03"])
        rc, text = self.run_push(menu, self.make_gdi())
        self.assertEqual(rc, 0, text)
        self.assertEqual(menu.put_folders, ["03"])
        self.assertEqual(menu.mode, ngtp.PUT_RESUME)
        self.assertIn("trying to resume it", text)
        self.assertNotIn("using the free folder", text)

    def test_auto_skips_a_partial_folder_of_another_image(self):
        menu = FakeMenu(listing=["NGD:LIST partial 03", "NGD:LIST partial 05", "NGD:LIST complete 02"],
                        put_errors={"03": "EEXIST"})
        rc, text = self.run_push(menu, self.make_gdi())
        self.assertEqual(rc, 0, text)
        self.assertEqual(menu.put_folders, ["03", "05"])
        self.assertIn("holds a different image", text)

    def test_auto_takes_a_free_folder_when_every_partial_one_differs(self):
        menu = FakeMenu(listing=["NGD:LIST complete 02", "NGD:LIST partial 03"], put_errors={"03": "EEXIST"})
        rc, text = self.run_push(menu, self.make_gdi())
        self.assertEqual(rc, 0, text)
        self.assertEqual(menu.put_folders, ["03", "04"])
        self.assertIn("using the free folder 04", text)

    def test_other_errors_on_a_partial_folder_are_not_skipped(self):
        menu = FakeMenu(listing=["NGD:LIST partial 03", "NGD:LIST partial 05"], put_errors={"03": "ENOSPC"})
        rc, text = self.run_push(menu, self.make_gdi())
        self.assertEqual(rc, 1, text)
        self.assertEqual(menu.put_folders, ["03"])

    def test_new_and_overwrite_modes_never_probe_a_partial_folder(self):
        for mode in ("new", "overwrite"):
            menu = FakeMenu(listing=["NGD:LIST complete 02", "NGD:LIST partial 03"])
            rc, text = self.run_push(menu, self.make_gdi(), "--mode", mode)
            self.assertEqual(rc, 0, text)
            self.assertEqual(menu.put_folders, ["04"], mode)

    def test_an_explicit_slot_or_name_does_not_look_for_partial_folders(self):
        menu = FakeMenu(listing=["NGD:LIST partial 03"])
        rc, text = self.run_push(menu, self.make_gdi(), "--slot", "7")
        self.assertEqual(rc, 0, text)
        self.assertEqual(menu.put_folders, ["07"])
        self.assertNotIn(ngtp.LIST, menu.requests)

    def test_an_interrupted_auto_push_prints_the_exact_command_with_the_slot(self):
        menu = FakeMenu(listing=["NGD:LIST complete 02"], cut_after_wants=1)
        rc, text = self.run_push(menu, self.make_gdi(), "--title", "My Game")
        self.assertEqual(rc, 2, text)
        line = [l for l in text.splitlines() if l.startswith("to go on with exactly this folder:")]
        self.assertEqual(len(line), 1, text)
        self.assertIn("--slot 03", line[0])
        self.assertIn("--host 127.0.0.1", line[0])
        self.assertIn("--code 4821", line[0])
        self.assertIn('--title "My Game"', line[0])

    def test_a_stop_status_also_prints_the_command(self):
        menu = FakeMenu(listing=["NGD:LIST complete 02"], fail_done="ESTOP")
        rc, text = self.run_push(menu, self.make_gdi())
        self.assertEqual(rc, 2, text)
        self.assertIn("--slot 03", text)

    def test_no_rerun_line_for_an_explicit_name(self):
        menu = FakeMenu(cut_after_wants=1)
        rc, text = self.run_push(menu, self.make_gdi(), "--name", "MYGAME")
        self.assertEqual(rc, 2, text)
        self.assertNotIn("exactly this folder", text)

    def test_connection_lost_is_resumable_exit_2(self):
        menu = FakeMenu(cut_after_wants=1)
        rc, text = self.run_push(menu, self.make_gdi(), "--slot", "3")
        self.assertEqual(rc, 2, text)
        self.assertIn("run the same command again", text)

    def test_done_failure_codes(self):
        for name, rc in (("ESTOP", 2), ("ESRC", 2), ("ENOSPC", 1), ("EPROTECTED", 1)):
            menu = FakeMenu(fail_done=name)
            got, text = self.run_push(menu, self.make_gdi(), "--slot", "3")
            self.assertEqual(got, rc, (name, text))

    def test_nothing_is_created_next_to_the_image(self):
        gdi = self.make_gdi()
        before = sorted(os.listdir(os.path.dirname(gdi)))
        menu = FakeMenu()
        self.assertEqual(self.run_push(menu, gdi, "--slot", "3")[0], 0)
        self.assertEqual(sorted(os.listdir(os.path.dirname(gdi))), before)

    def test_image_errors_are_reported_before_connecting(self):
        out = io.StringIO()
        with redirect_stdout(out):
            self.assertEqual(main(["push", self.path("nope.gdi"), "--host", "127.0.0.1", "--port", "9", "--code", "1"]), 1)

    def test_a_name_txt_in_the_image_is_refused(self):
        gdi = self.make_gdi(names=("name.txt", "track02.raw", "track03.bin"))
        out = io.StringIO()
        with redirect_stdout(out):
            rc = main(["push", gdi, "--host", "127.0.0.1", "--port", "9", "--code", "1"])
        self.assertEqual(rc, 1)

    def test_prepare_and_push_plan_the_same_files(self):
        from netgd_send import image as imgmod
        gdi = self.make_gdi()
        kind, desc, sources = imgmod.plan_image(gdi)
        self.assertEqual((kind, desc), ("gdi", "disc.gdi"))
        kind2, desc2, files = pushmod.plan_push(gdi, title="T")
        self.assertEqual([f.name for f in files][:-1], [s[0] for s in sources])


class TestRemote(Tmp):
    def test_list_prints_the_lines(self):
        menu = FakeMenu(listing=["NGD:LIST complete 02"])
        out = io.StringIO()
        with redirect_stdout(out):
            rc = main(["remote", "list", "--host", "127.0.0.1", "--port", str(menu.port), "--code", "4821"])
        self.assertEqual(rc, 0)
        self.assertIn("NGD:LIST complete 02", out.getvalue())

    def test_delete_needs_a_folder(self):
        with self.assertRaises(SystemExit):
            main(["remote", "delete", "--host", "127.0.0.1", "--code", "1"])


if __name__ == "__main__":
    unittest.main()
