"""Tests of NETGD.ART (netgd_send/art.py) and `push --art` / `art`.  The tests never need Pillow (the ones that use it are
skipped when it is not installed).  Run: python -m unittest discover -s tests -v"""

import io
import os
import struct
import sys
import unittest
import zlib
from contextlib import redirect_stdout

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
sys.path.insert(0, HERE)

from netgd_send import art  # noqa: E402
from netgd_send import push as pushmod  # noqa: E402
from netgd_send.__main__ import main  # noqa: E402
from test_netgd_send import Tmp  # noqa: E402
from test_ngtp import FakeMenu  # noqa: E402

try:
    import PIL.Image  # noqa: F401
    HAVE_PILLOW = True
except Exception:
    HAVE_PILLOW = False


def chunk(typ, body):
    return struct.pack(">I", len(body)) + typ + body + struct.pack(">I", zlib.crc32(typ + body) & 0xFFFFFFFF)


def paeth(a, b, c):
    p = a + b - c
    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
    return a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)


def make_png(w, h, pixel, ctype=2, depth=8, filters=True, palette=None, trns=None, interlace=0):
    """a PNG of w x h: pixel(x, y) -> tuple of samples (0..255 for depth 8; 16 bit samples are the value * 257).
    With filters=True the row filter cycles through 0..4 so that the reader's unfiltering is exercised."""
    chans = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}[ctype]
    bpp = max(1, chans * depth // 8)
    rows = []
    for y in range(h):
        raw = bytearray()
        for x in range(w):
            for s in pixel(x, y):
                raw += struct.pack(">H", s * 257) if depth == 16 else bytes([s])
        rows.append(bytes(raw))
    out, prev = bytearray(), bytes(len(rows[0]))
    for y, cur in enumerate(rows):
        f = y % 5 if filters else 0
        enc = bytearray(len(cur))
        for i in range(len(cur)):
            a = cur[i - bpp] if i >= bpp else 0
            b = prev[i]
            c = prev[i - bpp] if i >= bpp else 0
            pred = (0, a, b, (a + b) >> 1, paeth(a, b, c))[f]
            enc[i] = (cur[i] - pred) & 255
        out += bytes([f]) + bytes(enc)
        prev = cur
    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, depth, ctype, 0, 0, interlace))
    if palette:
        png += chunk(b"PLTE", b"".join(bytes(p) for p in palette))
    if trns:
        png += chunk(b"tRNS", bytes(trns))
    return png + chunk(b"IDAT", zlib.compress(bytes(out))) + chunk(b"IEND", b"")


def px565(r, g, b):
    return ((r >> 3) << 11) | ((g >> 2) << 5) | (b >> 3)


class TestFile(unittest.TestCase):
    def test_layout_and_round_trip(self):
        rgb = bytes(v for i in range(128 * 128) for v in (i % 256, (i // 3) % 256, 255 - i % 256))
        data = art.encode(rgb)
        self.assertEqual(len(data), 16 + 128 * 128 * 2)
        self.assertEqual(data[:16], b"NGAR" + struct.pack("<HHH", 1, 128, 128) + bytes(6))
        px = art.parse(data)
        self.assertEqual(len(px), 128 * 128)
        for i in (0, 1, 777, 128 * 128 - 1):
            self.assertEqual(px[i], px565(rgb[i * 3], rgb[i * 3 + 1], rgb[i * 3 + 2]))
        # little endian: the first pixel's low byte comes first
        self.assertEqual(data[16:18], struct.pack("<H", px[0]))
        self.assertEqual(art.parse(data + b"trailing"), px)             # extra bytes at the end do not matter

    def test_encode_known_colours(self):
        data = art.encode(bytes([255, 0, 0] + [0, 255, 0] + [0, 0, 255] + [255, 255, 255]) + bytes(3 * (128 * 128 - 4)))
        self.assertEqual(struct.unpack_from("<4H", data, 16), (0xF800, 0x07E0, 0x001F, 0xFFFF))

    def test_parse_refuses_what_the_menu_would_ignore(self):
        good = art.encode(bytes(128 * 128 * 3))
        art.parse(good)
        for bad, why in ((good[:-1], "short"), (b"XGAR" + good[4:], "magic"), (good[:4] + struct.pack("<H", 2) + good[6:], "version"),
                         (good[:6] + struct.pack("<HH", 64, 64) + good[10:], "size"), (b"", "empty")):
            with self.assertRaises(art.ArtError, msg=why):
                art.parse(bad)
        with self.assertRaises(art.ArtError):
            art.encode(bytes(10))


class TestScaling(unittest.TestCase):
    def test_contain_size(self):
        self.assertEqual(art.contain_size(128, 128), (128, 128))
        self.assertEqual(art.contain_size(256, 128), (128, 64))
        self.assertEqual(art.contain_size(100, 400), (32, 128))
        self.assertEqual(art.contain_size(64, 64), (128, 128))
        self.assertEqual(art.contain_size(1, 1000), (1, 128))

    def test_box_resize_averages(self):
        # 4 x 2 -> 2 x 1: each output pixel is the mean of a 2 x 2 block
        rgb = bytes([0, 0, 0, 100, 100, 100, 200, 0, 0, 0, 40, 0,
                     100, 0, 0, 0, 0, 0, 0, 0, 50, 0, 40, 100])
        self.assertEqual(art.box_resize(rgb, 4, 2, 2, 1), bytes([50, 25, 25, 50, 20, 37]))   # (0+100+100+0)/4 = 50 ...
        # no change in size
        self.assertEqual(art.box_resize(rgb, 4, 2, 4, 2), rgb)

    def test_upscale_repeats_pixels(self):
        out = art.fit_rgb(bytes([255, 0, 0, 0, 255, 0, 0, 0, 255, 9, 9, 9]), 2, 2)
        self.assertEqual(len(out), 128 * 128 * 3)
        self.assertEqual(out[0:3], bytes([255, 0, 0]))
        self.assertEqual(out[63 * 3:64 * 3], bytes([255, 0, 0]))
        self.assertEqual(out[64 * 3:65 * 3], bytes([0, 255, 0]))
        self.assertEqual(out[(127 * 128 + 127) * 3:][:3], bytes([9, 9, 9]))

    def test_wide_picture_is_letterboxed(self):
        rgb = bytes([200, 100, 50]) * (256 * 128)
        out = art.fit_rgb(rgb, 256, 128)                                # 128 x 64 in the middle
        row = lambda y: out[y * 128 * 3:(y + 1) * 128 * 3]              # noqa: E731
        self.assertEqual(row(0), bytes(128 * 3))
        self.assertEqual(row(31), bytes(128 * 3))
        self.assertEqual(row(32), bytes([200, 100, 50]) * 128)
        self.assertEqual(row(95), bytes([200, 100, 50]) * 128)
        self.assertEqual(row(96), bytes(128 * 3))


class TestPng(unittest.TestCase):
    def test_rgb_all_filter_types(self):
        w, h = 40, 30
        pix = lambda x, y: ((x * 6) % 256, (y * 8) % 256, (x * y) % 256)         # noqa: E731
        got_w, got_h, rgb = art.decode_png(make_png(w, h, pix))
        self.assertEqual((got_w, got_h), (w, h))
        want = bytes(v for y in range(h) for x in range(w) for v in pix(x, y))
        self.assertEqual(rgb, want)

    def test_other_colour_types(self):
        pix_grey = lambda x, y: ((x * 9 + y) % 256,)                              # noqa: E731
        _w, _h, rgb = art.decode_png(make_png(20, 7, pix_grey, ctype=0))
        self.assertEqual(rgb[:6], bytes([0, 0, 0, 9, 9, 9]))
        pix_ga = lambda x, y: (200, 255 if x else 0)                              # noqa: E731
        _w, _h, rgb = art.decode_png(make_png(2, 1, pix_ga, ctype=4))
        self.assertEqual(rgb, bytes([0, 0, 0, 200, 200, 200]))                    # alpha 0 -> black
        pix_rgba = lambda x, y: (200, 100, 50, 128)                               # noqa: E731
        _w, _h, rgb = art.decode_png(make_png(1, 1, pix_rgba, ctype=6))
        self.assertEqual(rgb, bytes([100, 50, 25]))                               # 50 % over black
        pix_idx = lambda x, y: ((x + y) % 3,)                                     # noqa: E731
        _w, _h, rgb = art.decode_png(make_png(3, 1, pix_idx, ctype=3, palette=[(255, 0, 0), (0, 255, 0), (0, 0, 255)], trns=[255, 0]))
        self.assertEqual(rgb, bytes([255, 0, 0, 0, 0, 0, 0, 0, 255]))             # index 1 is transparent
        pix16 = lambda x, y: (10, 20, 30)                                         # noqa: E731
        _w, _h, rgb = art.decode_png(make_png(2, 2, pix16, depth=16))
        self.assertEqual(rgb[:3], bytes([10, 20, 30]))

    def test_bad_pngs_are_errors(self):
        good = make_png(4, 4, lambda x, y: (1, 2, 3))
        for bad in (b"", b"not a png at all", good[:30], good[:-30], make_png(4, 4, lambda x, y: (1, 2, 3), interlace=1)):
            with self.assertRaises(art.ArtError):
                art.decode_png(bad)


class Pics(Tmp):
    def picture(self, name, data):
        p = self.path(name)
        with open(p, "wb") as f:
            f.write(data)
        return p


class TestMakeArt(Pics):
    def test_png_without_pillow(self):
        p = self.picture("cover.png", make_png(256, 128, lambda x, y: (255, 0, 0)))
        data = art.make_art(p, use_pillow=False)
        px = art.parse(data)
        self.assertEqual(px[0], 0)                                                # the black bar
        self.assertEqual(px[64 * 128 + 5], 0xF800)                                # red in the middle
        self.assertEqual(px[127 * 128 + 127], 0)

    def test_a_finished_art_file_passes_through(self):
        good = art.encode(bytes([7] * (128 * 128 * 3)))
        p = self.picture("NETGD.ART", good + b"junk")
        self.assertEqual(art.make_art(p), good)
        with self.assertRaises(art.ArtError):
            art.make_art(self.picture("bad.art", good[:6] + struct.pack("<HH", 64, 64) + good[10:]))

    def test_not_a_picture(self):
        with self.assertRaises(art.ArtError):
            art.make_art(self.picture("x.png", b"this is text"), use_pillow=False)

    @unittest.skipUnless(HAVE_PILLOW, "Pillow is not installed")
    def test_pillow_reads_other_formats_and_agrees_on_the_layout(self):
        from PIL import Image
        im = Image.new("RGB", (300, 150), (0, 128, 255))
        p = self.path("cover.jpg")
        im.save(p, quality=95)
        px = art.parse(art.make_art(p, use_pillow=True))
        self.assertEqual(px[0], 0)                                                # letterbox
        mid = px[64 * 128 + 64]
        self.assertTrue(abs(((mid >> 11) & 31) - 0) <= 2 and abs(((mid >> 5) & 63) - 32) <= 3 and abs((mid & 31) - 31) <= 2, hex(mid))
        # the same PNG through both readers lands on (almost) the same pixels
        pp = self.picture("c.png", make_png(200, 200, lambda x, y: ((x * 5) % 256, (y * 5) % 256, 90)))
        a, b = art.parse(art.make_art(pp, use_pillow=True)), art.parse(art.make_art(pp, use_pillow=False))
        diff = sum(1 for u, v in zip(a, b) if abs(((u >> 11) & 31) - ((v >> 11) & 31)) > 3 or abs((u & 31) - (v & 31)) > 3)
        self.assertLess(diff, 128 * 128 // 20)


class TestPushArt(Pics):
    def run_main(self, *argv):
        out = io.StringIO()
        with redirect_stdout(out):
            rc = main(list(argv))
        return rc, out.getvalue()

    def test_push_sends_netgd_art_next_to_the_image(self):
        gdi = self.make_gdi()
        picture = self.picture("cover.png", make_png(64, 64, lambda x, y: (x * 4, y * 4, 0)))
        want = art.make_art(picture, use_pillow=False)
        menu = FakeMenu(listing=["NGD:LIST complete 02"])
        rc, text = self.run_main("push", gdi, "--host", "127.0.0.1", "--port", str(menu.port), "--code", "4821", "--timeout", "20",
                                 "--slot", "3", "--art", picture)
        menu.thread.join(10)
        self.assertEqual(rc, 0, text)
        names = [f.name.lower() for f in menu.manifest.files]
        self.assertIn("netgd.art", names)
        self.assertEqual(names.count("name.txt"), 1)
        # an art file that was made with Pillow or without is the same size; compare against what the PC side makes by itself
        got = menu.received[names.index("netgd.art")]
        self.assertEqual(len(got), art.FILE_LEN)
        self.assertEqual(art.parse(got), art.parse(art.make_art(picture)))
        if not HAVE_PILLOW:
            self.assertEqual(got, want)

    def test_push_without_art_sends_none(self):
        gdi = self.make_gdi()
        menu = FakeMenu(listing=["NGD:LIST complete 02"])
        rc, _t = self.run_main("push", gdi, "--host", "127.0.0.1", "--port", str(menu.port), "--code", "4821", "--timeout", "20", "--slot", "3")
        menu.thread.join(10)
        self.assertEqual(rc, 0)
        self.assertNotIn("netgd.art", [f.name.lower() for f in menu.manifest.files])

    def test_a_bad_picture_is_reported_before_connecting(self):
        gdi = self.make_gdi()
        rc, _t = self.run_main("push", gdi, "--host", "127.0.0.1", "--port", "9", "--code", "1", "--art", self.picture("x.png", b"nope"))
        self.assertEqual(rc, 1)
        rc, _t = self.run_main("push", gdi, "--host", "127.0.0.1", "--port", "9", "--code", "1", "--art", self.path("missing.png"))
        self.assertEqual(rc, 1)

    def test_plan_push_adds_one_art_file(self):
        gdi = self.make_gdi()
        art_bytes = art.encode(bytes(128 * 128 * 3))
        _kind, _desc, files = pushmod.plan_push(gdi, art=art_bytes)
        self.assertEqual([f.name for f in files if f.name == "netgd.art"], ["netgd.art"])
        self.assertEqual([f.size for f in files if f.name == "netgd.art"], [art.FILE_LEN])
        _kind, _desc, files = pushmod.plan_push(gdi)
        self.assertNotIn("netgd.art", [f.name for f in files])

    def test_art_command_writes_the_file(self):
        picture = self.picture("cover.png", make_png(32, 32, lambda x, y: (0, 255, 0)))
        outp = self.path("NETGD.ART")
        rc, text = self.run_main("art", picture, "-o", outp, "--no-pillow")
        self.assertEqual(rc, 0, text)
        with open(outp, "rb") as f:
            data = f.read()
        self.assertEqual(len(data), art.FILE_LEN)
        self.assertEqual(art.parse(data)[64 * 128 + 64], 0x07E0)
        rc, _t = self.run_main("art", self.picture("bad.png", b"zzz"), "-o", self.path("o2"), "--no-pillow")
        self.assertEqual(rc, 1)


if __name__ == "__main__":
    unittest.main()
