"""Manifest (design 5.1), CRC32 helpers and the chunk-checkpoint sidecar."""

import zlib

MANIFEST_MAGIC = "NETGD-MANIFEST"
MANIFEST_VERSION = 1
MAX_FILES = 128
MAX_FILE_SIZE = 4 * 1024 * 1024 * 1024 - 1  # FAT32: < 4 GiB
MAX_NAME_BYTES = 127
MAX_PATH_PARTS = 3  # firmware gdi.rs MAX_PATH_PARTS
MAX_PATH_BYTES = 96  # gamecfg::MAX_NEXT
CHUNK_MAGIC = "NETGD-CHUNKS"
DEFAULT_CHUNK_BYTES = 8 * 1024 * 1024  # checkpoint interval of NETGD.CKP (5.2)

_BAD_CHARS = set('\\/:*?"<>|')


class ManifestError(ValueError):
    pass


def crc32(data, value=0):
    """zlib-compatible CRC32 (poly 0xEDB88320, init/xorout 0xFFFFFFFF); chainable."""
    return zlib.crc32(data, value) & 0xFFFFFFFF


def crc_hex(value):
    return "%08X" % value


def validate_name(name, what="name"):
    """One path component (folder or file name): printable ASCII only, FAT-safe."""
    if not name:
        raise ManifestError("%s is empty" % what)
    try:
        raw = name.encode("ascii")
    except UnicodeEncodeError:
        raise ManifestError("%s %r contains non-ASCII characters (the firmware cannot mount them)" % (what, name))
    if len(raw) > MAX_NAME_BYTES:
        raise ManifestError("%s %r is longer than %d bytes" % (what, name, MAX_NAME_BYTES))
    for ch in name:
        if ord(ch) < 0x20 or ord(ch) > 0x7E:
            raise ManifestError("%s %r contains a non-printable character" % (what, name))
        if ch in _BAD_CHARS:
            raise ManifestError("%s %r contains the forbidden character %r" % (what, name, ch))
    if name != name.strip(" ") or name in (".", "..") or name.endswith("."):
        raise ManifestError("%s %r has a leading/trailing space or dot" % (what, name))


def validate_image_path(path):
    """`image` line: '/'-separated, at most 3 levels, total <= MAX_PATH_BYTES."""
    parts = path.split("/")
    if len(parts) > MAX_PATH_PARTS:
        raise ManifestError("image path %r has more than %d levels" % (path, MAX_PATH_PARTS))
    for p in parts:
        validate_name(p, "image folder")
    if len(path.encode("ascii")) > MAX_PATH_BYTES:
        raise ManifestError("image path %r is longer than %d bytes" % (path, MAX_PATH_BYTES))


class FileEntry(object):
    __slots__ = ("size", "crc", "name")

    def __init__(self, size, crc, name):
        self.size = size
        self.crc = crc
        self.name = name

    def __eq__(self, other):
        return isinstance(other, FileEntry) and (self.size, self.crc, self.name) == (other.size, other.crc, other.name)

    def __repr__(self):
        return "FileEntry(%d, %s, %r)" % (self.size, crc_hex(self.crc), self.name)


class Manifest(object):
    def __init__(self, image, type_, files, descriptor):
        self.image = image
        self.type = type_
        self.files = list(files)
        self.descriptor = descriptor

    def validate(self):
        validate_image_path(self.image)
        if self.type not in ("gdi", "cdi"):
            raise ManifestError("type must be gdi or cdi, not %r" % self.type)
        if not self.files:
            raise ManifestError("no files")
        if len(self.files) > MAX_FILES:
            raise ManifestError("more than %d files" % MAX_FILES)
        seen = set()
        for f in self.files:
            validate_name(f.name, "file name")
            if f.size < 0 or f.size > MAX_FILE_SIZE:
                raise ManifestError("file %s: size %d is outside 0..4GiB-1" % (f.name, f.size))
            if not 0 <= f.crc <= 0xFFFFFFFF:
                raise ManifestError("file %s: bad CRC" % f.name)
            key = f.name.lower()
            if key in seen:
                raise ManifestError("duplicate file name %s" % f.name)
            seen.add(key)
        if self.descriptor.lower() not in seen:
            raise ManifestError("descriptor %s is not one of the file lines" % self.descriptor)
        if not self.descriptor.lower().endswith("." + self.type):
            raise ManifestError("descriptor %s does not end in .%s" % (self.descriptor, self.type))

    def total_bytes(self):
        return sum(f.size for f in self.files)

    def render(self):
        """Canonical text (UTF-8, LF). Deterministic for a given content."""
        self.validate()
        lines = [
            "%s %d" % (MANIFEST_MAGIC, MANIFEST_VERSION),
            "image %s" % self.image,
            "type %s" % self.type,
        ]
        for f in self.files:
            lines.append("file %d %s %s" % (f.size, crc_hex(f.crc), f.name))
        lines.append("descriptor %s" % self.descriptor)
        lines.append("end")
        return "\n".join(lines) + "\n"

    def crc(self):
        # DESIGN-GAP: 5.2 says "CRC32 of the whole manifest" without fixing the bytes; we use the exact
        # rendered UTF-8/LF text of manifest.ngm.
        """CRC32 of the rendered manifest bytes (what NETGD.CKP stores at offset 8)."""
        return crc32(self.render().encode("utf-8"))


def parse_manifest(text):
    lines = text.split("\n")
    if lines and lines[-1] == "":
        lines.pop()
    if not lines or not lines[0].startswith(MANIFEST_MAGIC + " "):
        raise ManifestError("missing %s header" % MANIFEST_MAGIC)
    try:
        ver = int(lines[0].split(" ", 1)[1])
    except ValueError:
        raise ManifestError("bad manifest version")
    if ver != MANIFEST_VERSION:
        raise ManifestError("unsupported manifest version %d" % ver)
    image = type_ = descriptor = None
    files = []
    ended = False
    for ln in lines[1:]:
        if ended:
            raise ManifestError("data after 'end'")
        if ln == "end":
            ended = True
            continue
        key, _, rest = ln.partition(" ")
        if key == "image":
            image = rest
        elif key == "type":
            type_ = rest
        elif key == "descriptor":
            descriptor = rest
        elif key == "file":
            parts = rest.split(" ", 2)
            if len(parts) != 3 or len(parts[1]) != 8:
                raise ManifestError("bad file line: %r" % ln)
            try:
                files.append(FileEntry(int(parts[0]), int(parts[1], 16), parts[2]))
            except ValueError:
                raise ManifestError("bad file line: %r" % ln)
        else:
            raise ManifestError("unknown line: %r" % ln)
    if not ended:
        raise ManifestError("manifest is incomplete (no 'end')")
    if image is None or type_ is None or descriptor is None:
        raise ManifestError("manifest lacks image/type/descriptor")
    m = Manifest(image, type_, files, descriptor)
    m.validate()
    return m


def hash_file(path, chunk_bytes=DEFAULT_CHUNK_BYTES, block=1 << 20):
    """Return (size, crc32, checkpoints).

    checkpoints = [(offset, cumulative_crc32)] at every multiple of chunk_bytes that is
    strictly below the file size (the DC resumes only from such offsets).
    """
    if chunk_bytes <= 0 or chunk_bytes % 512:
        raise ValueError("chunk size must be a positive multiple of 512")
    crc = 0
    pos = 0
    cps = []
    next_cp = chunk_bytes
    with open(path, "rb") as f:
        while True:
            data = f.read(min(block, next_cp - pos))
            if not data:
                break
            crc = zlib.crc32(data, crc) & 0xFFFFFFFF
            pos += len(data)
            if pos == next_cp:
                cps.append((pos, crc))
                next_cp += chunk_bytes
    if cps and cps[-1][0] == pos:
        cps.pop()  # a checkpoint at EOF is not a resume point
    return pos, crc, cps


# DESIGN-GAP: 5.1 only has whole-file CRC32.  Per-chunk (cumulative) CRC32 at every checkpoint
# boundary is kept PC-side in manifest.ngc so `collect` can check the resume CRC the app reports.
def render_chunks(chunk_bytes, per_file):
    """per_file: list of checkpoint lists, in manifest order. 1-based file index."""
    lines = ["%s 1" % CHUNK_MAGIC, "chunk %d" % chunk_bytes]
    for i, cps in enumerate(per_file, 1):
        for off, c in cps:
            lines.append("c %d %d %s" % (i, off, crc_hex(c)))
    lines.append("end")
    return "\n".join(lines) + "\n"


def parse_chunks(text):
    """Return (chunk_bytes, {(file_index, offset): crc})."""
    lines = [l for l in text.split("\n") if l]
    if not lines or lines[0] != "%s 1" % CHUNK_MAGIC or lines[-1] != "end":
        raise ManifestError("bad chunk sidecar")
    chunk = None
    table = {}
    for ln in lines[1:-1]:
        p = ln.split(" ")
        if p[0] == "chunk" and len(p) == 2:
            chunk = int(p[1])
        elif p[0] == "c" and len(p) == 4:
            table[(int(p[1]), int(p[2]))] = int(p[3], 16)
        else:
            raise ManifestError("bad chunk line: %r" % ln)
    if chunk is None:
        raise ManifestError("chunk sidecar lacks chunk size")
    return chunk, table
