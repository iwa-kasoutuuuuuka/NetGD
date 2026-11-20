"""GDI / CDI inspection, 8.3 renaming of track files and rewriting of the .gdi (design 5.1, 5.2)."""

import locale
import os
import re

from .manifest import ManifestError, MAX_FILE_SIZE, validate_name, validate_image_path

SECTOR_SIZES = (2048, 2336, 2352)
MAX_TRACKS = 99
DESCRIPTOR_BASE = "disc"  # NetGD folder rule: main image is disc.gdi / disc.cdi

_SFN_CHARS = set("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789!#$%&'()-@^_`{}~")

CDI_VERSIONS = {0x80000005: "3", 0x80000006: "3.5"}   # what the firmware's cdi.rs mounts (R89 C3)
CDI_V2 = 0x80000004                                    # recognised only to say why it is refused


class ImageError(ValueError):
    pass


# ---------------------------------------------------------------- names

def is_dos83(name):
    """Valid 8.3 short name: ASCII, base 1..8, optional single extension 1..3, no spaces."""
    if not name or any(ch not in _SFN_CHARS and ch != "." for ch in name):
        return False
    if name.count(".") > 1:
        return False
    base, _, ext = name.partition(".")
    if not 1 <= len(base) <= 8:
        return False
    if "." in name and not 1 <= len(ext) <= 3:
        return False
    return True


def assign_83_names(items, reserved=()):
    """Choose a unique 8.3 name for each source file.

    items: list of (key, original_name, preferred_name); preferred_name is used when the
    original is not a valid 8.3 name or is already taken (it keeps its own extension).
    Returns {key: name}.  Names are unique case-insensitively and never collide with
    `reserved`.  Original names that are already valid 8.3 are kept (first pass) so a
    generated name can never steal a later file's own name.
    """
    taken = set(r.lower() for r in reserved)
    out = {}
    for key, orig, _pref in items:
        if is_dos83(orig) and orig.lower() not in taken:
            out[key] = orig
            taken.add(orig.lower())
    counter = 0
    for key, _orig, pref in items:
        if key in out:
            continue
        cand = pref
        while not is_dos83(cand) or cand.lower() in taken:
            counter += 1
            ext = pref.rpartition(".")[2] if "." in pref else ""
            cand = "tr%06d" % counter + ("." + ext if ext else "")
            if counter > 999999:
                raise ImageError("cannot find a free 8.3 name")
        out[key] = cand
        taken.add(cand.lower())
    return out


def slot_folder(n):
    """Numbered folder of the original GDEMU: 01..99 (2 digits), 100..999, 1000..9999."""
    if not isinstance(n, int) or not 1 <= n <= 9999:
        raise ImageError("slot must be 1..9999")
    return "%02d" % n if n < 100 else str(n)


def folder_from_stem(stem):
    """Default folder name from an image file stem: ASCII only, else ask for --name/--slot."""
    # DESIGN-GAP: 11.1 leaves the default SD folder open; default = sanitized ASCII image file name.
    cleaned = re.sub(r"[^A-Za-z0-9 _()\-.,!&'#+=@\[\]~]", "_", stem) if _is_ascii(stem) else None
    if cleaned is None:
        raise ImageError("image name %r is not ASCII; pass --slot N or --name ASCII_NAME" % stem)
    cleaned = cleaned.strip(" .")
    if not cleaned:
        raise ImageError("cannot derive a folder name from %r; pass --slot or --name" % stem)
    validate_name(cleaned, "folder name")
    return cleaned


def _is_ascii(s):
    try:
        s.encode("ascii")
        return True
    except UnicodeEncodeError:
        return False


# ---------------------------------------------------------------- GDI

_GDI_LINE = re.compile(r'^\s*(\d+)\s+(\d+)\s+(\d+)\s+(\d+)\s+("[^"]*"|\S+)\s+(\d+)\s*$')


class Track(object):
    def __init__(self, number, lba, ctrl, sector, filename, offset, path, size):
        self.number, self.lba, self.ctrl, self.sector = number, lba, ctrl, sector
        self.filename, self.offset, self.path, self.size = filename, offset, path, size


class GdiImage(object):
    def __init__(self, path, tracks, newline):
        self.path = path
        self.tracks = tracks
        self.newline = newline


def parse_gdi(text):
    """Return (declared_count, [(number, lba, ctrl, sector, filename, offset)], newline)."""
    if text.startswith("ï»¿"):  # UTF-8 BOM, read as latin-1
        text = text[3:]
    newline = "\r\n" if "\r\n" in text else "\n"
    # split on \n only: str.splitlines() also breaks at U+0085 / U+2028 etc. (R89 C5), and a name may contain them
    text = text.lstrip("﻿")
    lines = [l.rstrip("\r") for l in text.split("\n") if l.strip()]
    if not lines:
        raise ImageError("empty .gdi")
    try:
        count = int(lines[0].strip())
    except ValueError:
        raise ImageError("first line of .gdi must be the track count, got %r" % lines[0])
    rows = []
    for ln in lines[1:]:
        m = _GDI_LINE.match(ln)
        if not m:
            raise ImageError("cannot parse .gdi line: %r" % ln)
        num, lba, ctrl, sector, fname, off = m.groups()
        rows.append((int(num), int(lba), int(ctrl), int(sector), fname.strip('"'), int(off)))
    return count, rows, newline


def _ansi_encoding():
    if os.name == "nt":
        return "mbcs"
    return locale.getpreferredencoding(False) or "latin-1"


def _decode_candidates():
    seen = []
    for enc in ("utf-8", _ansi_encoding(), "latin-1"):
        if enc.lower() not in seen:
            seen.append(enc.lower())
            yield enc


def _read_gdi(gdi_path):
    """Decode the .gdi with UTF-8, then the ANSI code page, then latin-1: the first reading whose track
    files all exist on disk wins (R89 C5); if none does, the first one that decodes (the error then names
    the missing file).  Returns (count, rows, newline)."""
    try:
        with open(gdi_path, "rb") as f:
            raw = f.read()
    except OSError as e:
        raise ImageError("cannot read %s: %s" % (gdi_path, e))
    base = os.path.dirname(os.path.abspath(gdi_path))
    first, err = None, None
    for enc in _decode_candidates():
        try:
            parsed = parse_gdi(raw.decode(enc))
        except (UnicodeDecodeError, LookupError):
            continue
        except ImageError as e:
            err = err or e
            continue
        first = first or parsed
        if all(os.path.isfile(os.path.normpath(os.path.join(base, r[4].replace("\\", os.sep)))) for r in parsed[1]):
            return parsed
    if first:
        return first
    raise err or ImageError("cannot decode %s" % gdi_path)


def check_gdi(gdi_path):
    """Parse and validate a .gdi against the files next to it. Returns GdiImage."""
    count, rows, newline = _read_gdi(gdi_path)
    if not 1 <= len(rows) <= MAX_TRACKS:
        raise ImageError(".gdi has %d track lines (1..%d allowed)" % (len(rows), MAX_TRACKS))
    if count != len(rows):
        raise ImageError(".gdi says %d tracks but lists %d" % (count, len(rows)))
    base = os.path.dirname(os.path.abspath(gdi_path))
    tracks = []
    prev_lba = -1
    for i, (num, lba, ctrl, sector, fname, off) in enumerate(rows, 1):
        if num != i:
            raise ImageError("track numbers must be 1..N in order (line %d has %d)" % (i, num))
        if ctrl not in (0, 4):
            raise ImageError("track %d: type %d is not 0 (audio) or 4 (data)" % (num, ctrl))
        if sector not in SECTOR_SIZES:
            raise ImageError("track %d: sector size %d is not one of %s" % (num, sector, SECTOR_SIZES))
        if ctrl == 0 and sector != 2352:
            raise ImageError("track %d: audio track must use 2352-byte sectors" % num)
        if off != 0:
            # the firmware parser ignores the offset column (file_offset = 0)
            raise ImageError("track %d: nonzero file offset %d is not supported" % (num, off))
        if lba <= prev_lba:
            raise ImageError("track %d: start LBA %d is not after the previous track" % (num, lba))
        prev_lba = lba
        path = os.path.normpath(os.path.join(base, fname.replace("\\", os.sep)))
        if not os.path.isfile(path):
            raise ImageError("track %d: file %s not found" % (num, path))
        size = os.path.getsize(path)
        if size == 0:
            raise ImageError("track %d: %s is empty" % (num, path))
        if size % sector:
            raise ImageError("track %d: %s is %d bytes, not a multiple of the sector size %d" % (num, fname, size, sector))
        if size > MAX_FILE_SIZE:
            raise ImageError("track %d: %s is larger than %d bytes (FAT32 limit)" % (num, fname, MAX_FILE_SIZE))
        tracks.append(Track(num, lba, ctrl, sector, fname, off, path, size))
    return GdiImage(gdi_path, tracks, newline)


def plan_gdi_names(image):
    """Return ({track_number: new_name}, rewritten_gdi_text).

    A source file referenced by several tracks gets one name.  The descriptor name
    disc.gdi is reserved.
    """
    items = []
    seen = {}
    for t in image.tracks:
        key = os.path.normcase(t.path)
        if key in seen:
            continue
        seen[key] = t.number
        # DESIGN-GAP: 5.1 names only track01.bin / track02.raw (audio = .raw, data = .bin); already-8.3 names are kept.
        pref = "track%02d.%s" % (t.number, "raw" if t.ctrl == 0 else "bin")
        items.append((key, os.path.basename(t.path), pref))
    names = assign_83_names(items, reserved=(DESCRIPTOR_BASE + ".gdi",))
    by_track = {t.number: names[os.path.normcase(t.path)] for t in image.tracks}
    nl = image.newline
    out = [str(len(image.tracks))]
    for t in image.tracks:
        out.append("%d %d %d %d %s 0" % (t.number, t.lba, t.ctrl, t.sector, by_track[t.number]))
    return by_track, nl.join(out) + nl


# ---------------------------------------------------------------- CDI

class CdiImage(object):
    def __init__(self, path, size, version, header_offset):
        self.path, self.size, self.version, self.header_offset = path, size, version, header_offset


def check_cdi(path):
    """Basic DiscJuggler trailer sanity: last 8 bytes = version u32, header offset u32 (LE)."""
    try:
        size = os.path.getsize(path)
    except OSError as e:
        raise ImageError("cannot read %s: %s" % (path, e))
    if size < 16:
        raise ImageError("%s is too small to be a CDI" % path)
    if size > MAX_FILE_SIZE:
        raise ImageError("%s is larger than %d bytes (FAT32 limit)" % (path, MAX_FILE_SIZE))
    with open(path, "rb") as f:
        f.seek(-8, os.SEEK_END)
        tail = f.read(8)
    version = int.from_bytes(tail[:4], "little")
    off = int.from_bytes(tail[4:], "little")
    if version == CDI_V2:
        raise ImageError("%s: DiscJuggler v2 CDI images (trailer 0x80000004) are not supported by the "
                         "firmware; convert the image to v3 or v3.5" % path)
    if version not in CDI_VERSIONS:
        raise ImageError("%s: trailer version 0x%08X is not a known CDI version" % (path, version))
    # v3.5 counts the header offset from the end of the file, v3 from the start
    if version == 0x80000006:
        ok = 0 < off <= size
    else:
        ok = 0 < off < size
    if not ok:
        raise ImageError("%s: CDI header offset %d is outside the file (%d bytes)" % (path, off, size))
    return CdiImage(path, size, CDI_VERSIONS[version], off)


# ---------------------------------------------------------------- dispatch

def detect_type(path):
    ext = os.path.splitext(path)[1].lower()
    if ext == ".gdi":
        return "gdi"
    if ext == ".cdi":
        return "cdi"
    raise ImageError("%s: expected a .gdi or .cdi file" % path)


def folder_name(slot, name, image_path):
    """Target folder (the manifest `image` line)."""
    if slot is not None and name is not None:
        raise ImageError("use either --slot or --name, not both")
    if slot is not None:
        return slot_folder(slot)
    if name is not None:
        validate_image_path(name)
        return name
    return folder_from_stem(os.path.splitext(os.path.basename(image_path))[0])
