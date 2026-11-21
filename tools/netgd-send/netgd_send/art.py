"""NETGD.ART: the box art file of a game folder, as the NetGD menu reads it (docs/design/menu-app.md step 11,
tools/netgd-menu/README_ja.md "実装メモ").

    offset 0   4  "NGAR"
           4   2  version, 1                (little endian)
           6   2  width, 128
           8   2  height, 128
          10   6  zero (reserved)
          16  32768  128 x 128 pixels, row by row from the top left, RGB565, 2 bytes each, little endian
                      (bits 15..11 red, 10..5 green, 4..0 blue)
    32784 bytes in all.

`make_art` turns a picture into that file: the picture is scaled to fit inside 128 x 128 (aspect ratio kept, black bars
around it).  Pillow is used when it is installed (any picture format it reads); without it a small PNG reader is used
(8 or 16 bit grey, grey + alpha, RGB, RGB + alpha, or palette; not interlaced), so a PNG always works.  An already
finished NETGD.ART is passed through.  Standard library only.  Not run on a Dreamcast.
"""

import struct
import zlib

MAGIC = b"NGAR"
VERSION = 1
SIZE = 128
HEADER_LEN = 16
FILE_LEN = HEADER_LEN + SIZE * SIZE * 2
FILE_NAME = "netgd.art"


class ArtError(Exception):
    pass


# ---- the file ----

def encode(rgb):
    """128 x 128 RGB bytes (3 per pixel) -> the file's bytes"""
    if len(rgb) != SIZE * SIZE * 3:
        raise ArtError("encode needs exactly %d x %d RGB pixels" % (SIZE, SIZE))
    out = bytearray(struct.pack("<4sHHH6x", MAGIC, VERSION, SIZE, SIZE))
    px = bytearray(SIZE * SIZE * 2)
    j = 0
    for i in range(0, len(rgb), 3):
        v = ((rgb[i] >> 3) << 11) | ((rgb[i + 1] >> 2) << 5) | (rgb[i + 2] >> 3)
        px[j] = v & 0xFF
        px[j + 1] = v >> 8
        j += 2
    return bytes(out + px)


def parse(data):
    """the file's bytes -> list of RGB565 values (128 * 128); ArtError when it is not a file the menu would show"""
    if len(data) < FILE_LEN:
        raise ArtError("a NETGD.ART is %d bytes (this one has %d)" % (FILE_LEN, len(data)))
    magic, ver, w, h = struct.unpack_from("<4sHHH", data, 0)
    if magic != MAGIC:
        raise ArtError("not a NETGD.ART (no NGAR magic)")
    if ver != VERSION:
        raise ArtError("NETGD.ART version %d (this tool and the menu know version %d)" % (ver, VERSION))
    if (w, h) != (SIZE, SIZE):
        raise ArtError("NETGD.ART is %d x %d, the menu shows %d x %d only" % (w, h, SIZE, SIZE))
    return list(struct.unpack_from("<%dH" % (SIZE * SIZE), data, HEADER_LEN))


# ---- scaling ----

def contain_size(sw, sh, size=SIZE):
    """the size a sw x sh picture gets when it is fitted into size x size, aspect ratio kept"""
    scale = min(size / float(sw), size / float(sh))
    return max(1, min(size, int(round(sw * scale)))), max(1, min(size, int(round(sh * scale))))


def box_resize(rgb, sw, sh, dw, dh):
    """RGB bytes sw x sh -> RGB bytes dw x dh.  Every destination pixel is the average of the source pixels it covers
    (at least one, so a picture that gets bigger is repeated pixels, not blurred)."""
    xr = []
    for ox in range(dw):
        x0 = ox * sw // dw
        x1 = max(x0 + 1, -((-(ox + 1) * sw) // dw))
        xr.append((x0, min(x1, sw)))
    out = bytearray(dw * dh * 3)
    stride = sw * 3
    for oy in range(dh):
        y0 = oy * sh // dh
        y1 = min(sh, max(y0 + 1, -((-(oy + 1) * sh) // dh)))
        # column sums over the source rows of this destination row
        col = [0] * stride
        for y in range(y0, y1):
            row = rgb[y * stride:(y + 1) * stride]
            for i in range(stride):
                col[i] += row[i]
        n_rows = y1 - y0
        o = oy * dw * 3
        for (x0, x1) in xr:
            cnt = (x1 - x0) * n_rows
            for c in range(3):
                out[o + c] = sum(col[x * 3 + c] for x in range(x0, x1)) // cnt
            o += 3
    return bytes(out)


def fit_rgb(rgb, sw, sh):
    """RGB bytes sw x sh -> 128 x 128 RGB bytes: scaled to fit, centred on black"""
    tw, th = contain_size(sw, sh)
    small = rgb if (tw, th) == (sw, sh) else box_resize(rgb, sw, sh, tw, th)
    return _centre(small, tw, th)


def _centre(small, tw, th):
    canvas = bytearray(SIZE * SIZE * 3)
    ox, oy = (SIZE - tw) // 2, (SIZE - th) // 2
    for y in range(th):
        d = ((oy + y) * SIZE + ox) * 3
        canvas[d:d + tw * 3] = small[y * tw * 3:(y + 1) * tw * 3]
    return bytes(canvas)


# ---- a small PNG reader (the fallback when Pillow is not installed) ----

def decode_png(data):
    """-> (width, height, RGB bytes); transparency is blended over black"""
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ArtError("not a PNG file")
    pos, idat, plte, trns = 8, [], b"", b""
    ihdr = None
    while pos + 8 <= len(data):
        n, typ = struct.unpack_from(">I4s", data, pos)
        body = data[pos + 8:pos + 8 + n]
        if len(body) != n:
            raise ArtError("the PNG is cut short")
        pos += 12 + n
        if typ == b"IHDR":
            ihdr = struct.unpack(">IIBBBBB", body)
        elif typ == b"PLTE":
            plte = body
        elif typ == b"tRNS":
            trns = body
        elif typ == b"IDAT":
            idat.append(body)
        elif typ == b"IEND":
            break
    if ihdr is None or not idat:
        raise ArtError("the PNG has no image data")
    w, h, depth, ctype, _comp, _filt, interlace = ihdr
    if interlace:
        raise ArtError("interlaced PNGs are not supported without Pillow (save it without interlacing, or install Pillow)")
    if w < 1 or h < 1 or w > 8192 or h > 8192:
        raise ArtError("the PNG is %d x %d: too large or empty" % (w, h))
    chans = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}.get(ctype)
    if chans is None or depth not in {0: (1, 2, 4, 8, 16), 2: (8, 16), 3: (1, 2, 4, 8), 4: (8, 16), 6: (8, 16)}[ctype]:
        raise ArtError("this PNG type (colour type %d, %d bit) is not supported without Pillow" % (ctype, depth))
    try:
        raw = zlib.decompress(b"".join(idat))
    except zlib.error as e:
        raise ArtError("the PNG data is damaged: %s" % e)
    bits = chans * depth
    bpp = max(1, bits // 8)
    rowlen = (w * bits + 7) // 8
    if len(raw) < (rowlen + 1) * h:
        raise ArtError("the PNG data is too short")
    rows, prev = [], bytearray(rowlen)
    for y in range(h):
        f = raw[y * (rowlen + 1)]
        cur = bytearray(raw[y * (rowlen + 1) + 1:(y + 1) * (rowlen + 1)])
        if f == 1:
            for i in range(bpp, rowlen):
                cur[i] = (cur[i] + cur[i - bpp]) & 255
        elif f == 2:
            for i in range(rowlen):
                cur[i] = (cur[i] + prev[i]) & 255
        elif f == 3:
            for i in range(rowlen):
                left = cur[i - bpp] if i >= bpp else 0
                cur[i] = (cur[i] + ((left + prev[i]) >> 1)) & 255
        elif f == 4:
            for i in range(rowlen):
                a = cur[i - bpp] if i >= bpp else 0
                b = prev[i]
                c = prev[i - bpp] if i >= bpp else 0
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pr = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                cur[i] = (cur[i] + pr) & 255
        elif f != 0:
            raise ArtError("the PNG has an unknown filter type")
        rows.append(cur)
        prev = cur
    out = bytearray(w * h * 3)
    o = 0
    pal = [(plte[i], plte[i + 1], plte[i + 2]) for i in range(0, len(plte) - 2, 3)]
    for y in range(h):
        row = rows[y]
        for x in range(w):
            if depth == 8:
                px = row[x * chans:(x + 1) * chans]
            elif depth == 16:
                px = bytes(row[(x * chans + k) * 2] for k in range(chans))          # the high byte of each sample
            else:
                per = 8 // depth
                v = (row[x // per] >> (8 - depth * (x % per + 1))) & ((1 << depth) - 1)
                px = bytes([v if ctype == 3 else v * 255 // ((1 << depth) - 1)])
            alpha = 255
            if ctype == 0:
                r = g = b = px[0]
            elif ctype == 2:
                r, g, b = px[0], px[1], px[2]
            elif ctype == 3:
                if px[0] >= len(pal):
                    raise ArtError("the PNG uses a colour that is not in its palette")
                r, g, b = pal[px[0]]
                if px[0] < len(trns):
                    alpha = trns[px[0]]
            elif ctype == 4:
                r = g = b = px[0]
                alpha = px[1]
            else:
                r, g, b, alpha = px[0], px[1], px[2], px[3]
            if alpha != 255:
                r, g, b = r * alpha // 255, g * alpha // 255, b * alpha // 255
            out[o] = r
            out[o + 1] = g
            out[o + 2] = b
            o += 3
    return w, h, bytes(out)


# ---- the entry point ----

def _pillow():
    try:
        from PIL import Image  # noqa: F401
        return True
    except Exception:
        return False


def make_art(path, use_pillow=None):
    """the bytes of a NETGD.ART made from the picture at `path` (or the file itself, when it already is one).
    use_pillow: None = if installed, True = required, False = never (the built-in PNG reader)."""
    with open(path, "rb") as f:
        data = f.read()
    if data[:4] == MAGIC:
        parse(data)                                 # validates
        return data[:FILE_LEN]
    if use_pillow is None:
        use_pillow = _pillow()
    if use_pillow:
        try:
            from PIL import Image
        except Exception:
            raise ArtError("Pillow is not installed (pip install Pillow), or give a PNG file")
        try:
            im = Image.open(path)
            im.load()
        except Exception as e:
            raise ArtError("cannot read %s: %s" % (path, e))
        im = im.convert("RGBA")
        black = Image.new("RGBA", im.size, (0, 0, 0, 255))
        black.alpha_composite(im)
        rgb_im = black.convert("RGB")
        tw, th = contain_size(*rgb_im.size)
        if (tw, th) != rgb_im.size:
            rgb_im = rgb_im.resize((tw, th), Image.LANCZOS)
        return encode(_centre(rgb_im.tobytes(), tw, th))
    w, h, rgb = decode_png(data)
    return encode(fit_rgb(rgb, w, h))
