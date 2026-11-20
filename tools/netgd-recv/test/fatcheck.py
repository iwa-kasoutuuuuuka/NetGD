"""Independent FAT32 reader/checker for the test images (does not use FatFs).

Reads MBR partition 1, the BPB, FAT 1 and walks the directory tree.  Reports structural
errors (bad / looping / cross-linked chains, wrong chain length, broken LFN checksums,
..), lost clusters (allocated but unreferenced), files and their extents.
"""

import struct


def lfn_checksum(sfn11):
    s = 0
    for b in sfn11:
        s = (((s & 1) << 7) + (s >> 1) + b) & 0xFF
    return s


class Fat32(object):
    def __init__(self, path):
        with open(path, "rb") as f:
            self.data = f.read()
        d = self.data
        if len(d) < 512 or d[510:512] != b"\x55\xaa":
            raise ValueError("no MBR")
        self.ptype = d[446 + 4]
        self.pstart = struct.unpack_from("<I", d, 446 + 8)[0]
        b = self.pstart * 512
        (self.bps, self.spc, self.rsv, self.nfats) = struct.unpack_from("<HBHB", d, b + 11)
        self.fatsz = struct.unpack_from("<I", d, b + 36)[0]
        self.root = struct.unpack_from("<I", d, b + 44)[0]
        self.tot = struct.unpack_from("<I", d, b + 32)[0]
        assert self.bps == 512
        self.fat_lba = self.pstart + self.rsv
        self.data_lba = self.fat_lba + self.nfats * self.fatsz
        self.ncl = (self.tot - self.rsv - self.nfats * self.fatsz) // self.spc
        self.csz = self.spc * 512
        n = self.ncl + 2
        raw = self._sectors(self.fat_lba, (n * 4 + 511) // 512)
        self.fat = list(struct.unpack_from("<%dI" % n, raw, 0))
        self.fat = [v & 0x0FFFFFFF for v in self.fat]
        raw2 = self._sectors(self.fat_lba + self.fatsz, (n * 4 + 511) // 512)
        self.fat2 = [v & 0x0FFFFFFF for v in struct.unpack_from("<%dI" % n, raw2, 0)]
        self.errors = []
        self.files = {}     # path -> dict(size, first, chain)
        self.dirs = {}
        self.used = {}
        self._walk()

    def _sectors(self, lba, n):
        off = lba * 512
        raw = self.data[off:off + n * 512]
        return raw + b"\0" * (n * 512 - len(raw))

    def chain(self, first, what):
        out, seen, c = [], set(), first
        while True:
            if c < 2 or c >= self.ncl + 2:
                self.errors.append("%s: chain leaves the volume (cluster %d)" % (what, c))
                return out
            if c in seen:
                self.errors.append("%s: chain loop at %d" % (what, c))
                return out
            seen.add(c)
            out.append(c)
            nxt = self.fat[c]
            if nxt >= 0x0FFFFFF8:
                return out
            if nxt == 0 or nxt == 1 or nxt == 0x0FFFFFF7:
                self.errors.append("%s: chain hits free/bad cluster after %d" % (what, c))
                return out
            c = nxt

    def read_clusters(self, chain):
        return b"".join(self._sectors(self.data_lba + (c - 2) * self.spc, self.spc) for c in chain)

    def _claim(self, chain, what):
        for c in chain:
            if c in self.used:
                self.errors.append("%s: cluster %d also belongs to %s" % (what, c, self.used[c]))
            self.used[c] = what

    def _entries(self, dirchain):
        raw = self.read_clusters(dirchain)
        lfn = []
        for i in range(0, len(raw), 32):
            e = raw[i:i + 32]
            if e[0] == 0:
                break
            if e[0] == 0xE5:
                lfn = []
                continue
            attr = e[11]
            if attr == 0x0F:
                lfn.append(e)
                continue
            if attr & 0x08:     # volume label
                lfn = []
                continue
            sfn = bytes(e[:11])
            name = None
            if lfn:
                chk = lfn_checksum(sfn)
                parts = []
                ok = True
                for l in sorted(lfn, key=lambda x: x[0] & 0x1F):
                    if l[13] != chk:
                        ok = False
                    parts.append(bytes(l[1:11]) + bytes(l[14:26]) + bytes(l[28:32]))
                if not ok:
                    self.errors.append("LFN checksum mismatch for %r" % sfn)
                u = b"".join(parts).decode("utf-16-le", "replace")
                name = u.split("\u0000")[0].split("￿")[0]
            if name is None:
                base = sfn[:8].decode("latin-1").rstrip()
                ext = sfn[8:].decode("latin-1").rstrip()
                nt = e[12]
                if nt & 0x08:
                    base = base.lower()
                if nt & 0x10:
                    ext = ext.lower()
                name = base + ("." + ext if ext else "")
            lfn = []
            first = (struct.unpack_from("<H", e, 20)[0] << 16) | struct.unpack_from("<H", e, 26)[0]
            size = struct.unpack_from("<I", e, 28)[0]
            yield name, attr, first, size, sfn

    def _walk(self):
        rc = self.chain(self.root, "root")
        self._claim(rc, "/")
        self._dir("", rc)

    def _dir(self, path, chain):
        for name, attr, first, size, sfn in self._entries(chain):
            if name in (".", ".."):
                continue
            p = path + "/" + name
            if attr & 0x10:
                ch = self.chain(first, p) if first else []
                if not ch:
                    self.errors.append("%s: directory without clusters" % p)
                    continue
                self._claim(ch, p)
                self.dirs[p] = {"first": first, "chain": ch}
                self._dir(p, ch)
            else:
                ch = self.chain(first, p) if first else []
                need = (size + self.csz - 1) // self.csz
                if len(ch) != need:
                    self.errors.append("%s: %d clusters for %d bytes (need %d)" % (p, len(ch), size, need))
                self._claim(ch, p)
                self.files[p] = {"size": size, "first": first, "chain": ch, "sfn": sfn}

    # ------------------------------------------------------------ queries
    def lost_clusters(self):
        n = 0
        for c in range(2, self.ncl + 2):
            if self.fat[c] != 0 and c not in self.used:
                n += 1
        return n

    def fat_mismatch(self):
        return sum(1 for a, b in zip(self.fat, self.fat2) if a != b)

    def read_file(self, path):
        f = self.files[path]
        return self.read_clusters(f["chain"])[:f["size"]]

    def runs(self, path):
        ch = self.files[path]["chain"]
        n = 0
        prev = None
        for c in ch:
            if prev is None or c != prev + 1:
                n += 1
            prev = c
        return n

    def folder_files(self, folder):
        pre = folder.rstrip("/") + "/"
        return {p[len(pre):]: v for p, v in self.files.items()
                if p.startswith(pre) and "/" not in p[len(pre):]}

    def visible_images(self):
        """Folders that hold a .GDI/.CDI file (what the firmware would mount)."""
        out = {}
        for p in self.files:
            low = p.lower()
            if low.endswith(".gdi") or low.endswith(".cdi"):
                out.setdefault(p.rsplit("/", 1)[0], []).append(p)
        return out
