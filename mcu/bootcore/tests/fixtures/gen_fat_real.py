#!/usr/bin/env python3
"""Generate the `fat_real` fixtures: FAT32 images made by real formatters
(dosfstools `mkfs.fat`, util-linux `sfdisk` for the MBR, mtools
`mcopy`/`mdel`/`mmd` to populate them), stored sparsely (only the non-zero
512-byte sectors).

    python3 gen_fat_real.py OUTDIR [--sdcard DIR_MADE_BY_mksdcard]

Needs (Debian/Ubuntu): dosfstools mtools util-linux python3. Run it on a
Linux filesystem that supports sparse files (SEEK_DATA), not on /mnt/c.
The --sdcard folder is `tools/mksdcard/mksdcard.py DIR --small`.

Output per case: NAME.sparse  = b"SPRS", u32 total_sectors, then records (LE u32):
                                0, lba, count, count*512 bytes of raw data; or
                                1, lba, count, seed, size, first_block: `count`
                                sectors that are blocks first_block.. of the
                                zero-padded lcg(seed, size) stream (the image
                                bytes were checked to be exactly that)
                  NAME.txt    = key=value lines read by tests/fat_real.rs
File contents are the `lcg(seed, size)` stream of tests/common/mod.rs.
mtools stamps directory entries with the current time, so a regeneration
changes some bytes (not behaviour); the committed files are the fixtures.
"""
import os
import shutil
import struct
import subprocess
import sys
import tempfile

SEC = 512


def lcg(seed, n):
    s = (seed * 2654435761 + 12345) & 0xFFFFFFFF
    out = bytearray()
    for _ in range(n):
        s = (s * 1664525 + 1013904223) & 0xFFFFFFFF
        out.append(s >> 24)
    return bytes(out)


def run(*a, **kw):
    env = dict(os.environ, MTOOLS_SKIP_CHECK="1", TZ="UTC")
    r = subprocess.run(a, env=env, capture_output=True, **kw)
    if r.returncode != 0:
        raise RuntimeError("%s failed: %s %s" % (a, r.stdout.decode(errors="replace"), r.stderr.decode(errors="replace")))
    return r.stdout.decode(errors="replace")


class Img:
    def __init__(self, path, total_sectors):
        self.path = path
        self.total = total_sectors
        self.start = 0  # sector of the FAT volume
        with open(path, "wb") as f:
            f.truncate(total_sectors * SEC)
        self.mt = path
        self.tmp = tempfile.mkdtemp(dir=os.path.dirname(path))
        self.n = 0
        self.known = []  # (seed, size) of every put_seed file

    def set_start(self, start):
        self.start = start
        self.mt = self.path + "@@" + str(start * SEC) if start else self.path

    def mbr(self, parts):
        """parts: list of (start, size, type_hex, bootable)."""
        lines = ["label: dos"]
        for st, sz, ty, boot in parts:
            lines.append("start=%d, size=%d, type=%s%s" % (st, sz, ty, ", bootable" if boot else ""))
        subprocess.run(["sfdisk", "--quiet", self.path], input="\n".join(lines).encode() + b"\n", check=True, capture_output=True)

    def mkfs(self, spc, nfats=2, reserved=32, sector=512, label="NETGD", sectors=None):
        sectors = sectors or (self.total - self.start)
        a = ["mkfs.fat", "-F", "32", "-s", str(spc), "-S", str(sector), "-f", str(nfats), "-R", str(reserved),
             "-n", label, "-i", "1234ABCD", "-h", str(self.start), "--offset", str(self.start), self.path,
             str(sectors * SEC // 1024)]
        run(*a)

    def put(self, dest, data, mtime=1_700_000_000):
        self.n += 1
        p = os.path.join(self.tmp, "f%d" % self.n)
        with open(p, "wb") as f:
            f.write(data)
        os.utime(p, (mtime, mtime))
        run("mcopy", "-m", "-i", self.mt, p, "::" + dest)
        os.unlink(p)

    def put_seed(self, dest, seed, size):
        self.known.append((seed, size))
        self.put(dest, lcg(seed, size))

    def mkdir(self, d):
        run("mmd", "-i", self.mt, "::" + d)

    def rm(self, dest):
        run("mdel", "-i", self.mt, "::" + dest)

    # --- raw BPB / FAT access (for the post-formatter patches and checks)
    def rd(self, off, n):
        with open(self.path, "rb") as f:
            f.seek(self.start * SEC + off)
            return f.read(n)

    def wr(self, off, data):
        with open(self.path, "r+b") as f:
            f.seek(self.start * SEC + off)
            f.write(data)

    def bpb(self):
        b = self.rd(0, 512)
        return dict(spc=b[0x0D], res=struct.unpack_from("<H", b, 0x0E)[0], nfats=b[0x10],
                    spf=struct.unpack_from("<I", b, 0x24)[0], root=struct.unpack_from("<I", b, 0x2C)[0])

    def fat(self, i):
        p = self.bpb()
        return self.rd((p["res"] + i * p["spf"]) * SEC, p["spf"] * SEC)

    def set_fat(self, i, data):
        p = self.bpb()
        self.wr((p["res"] + i * p["spf"]) * SEC, data)

    def chain(self, first, fat=None):
        fat = fat or self.fat(0)
        out, c = [], first
        while 2 <= c < 0x0FFFFFF0 and len(out) < 10_000_000:
            out.append(c)
            c = struct.unpack_from("<I", fat, c * 4)[0] & 0x0FFFFFFF
        return out

    def _root_entries(self):
        p = self.bpb()
        fat = self.fat(0)
        data = p["res"] + p["nfats"] * p["spf"]
        for c in self.chain(p["root"], fat):
            raw = self.rd((data + (c - 2) * p["spc"]) * SEC, p["spc"] * SEC)
            for o in range(0, len(raw), 32):
                yield raw[o:o + 32]

    def find_first_cluster(self, name11):
        """First cluster of the live short entry `name11` in the root, by
        walking the root chain here (independent of the code under test)."""
        for e in self._root_entries():
            if e[0] == 0:
                return None
            if e[0] != 0xE5 and e[11] != 0x0F and not e[11] & 0x18 and e[:11] == name11:
                return (struct.unpack_from("<H", e, 20)[0] << 16) | struct.unpack_from("<H", e, 26)[0]
        return None

    def deleted_entries(self, name11):
        n = 0
        for e in self._root_entries():
            if e[0] == 0:
                break
            if e[0] == 0xE5 and e[1:11] == name11[1:] and e[11] != 0x0F:
                n += 1
        return n

    def root_capacity(self):
        p = self.bpb()
        return len(self.chain(p["root"])) * p["spc"] * SEC // 32, len(self.chain(p["root"]))

    def frag_jumps(self, first):
        ch = self.chain(first)
        return sum(1 for a, b in zip(ch, ch[1:]) if b != a + 1), len(ch)

    # --- extraction of the non-zero sectors
    def nonzero_sectors(self):
        with open(self.path, "rb") as f:
            fd = f.fileno()
            end = self.total * SEC
            pos = 0
            while pos < end:
                try:
                    d = os.lseek(fd, pos, os.SEEK_DATA)
                except OSError:
                    return
                h = os.lseek(fd, d, os.SEEK_HOLE)
                f.seek(d)
                left = h - d
                lba = d // SEC
                while left > 0:
                    chunk = f.read(min(left, 1 << 20))
                    for i in range(0, len(chunk), SEC):
                        s = chunk[i:i + SEC]
                        if any(s):
                            yield lba + i // SEC, s
                    lba += len(chunk) // SEC
                    left -= len(chunk)
                pos = h

    def extract(self, out):
        """Raw runs for everything the formatter / mtools wrote, except the
        sectors that are exactly a block of one of the lcg(seed, size) files
        (zero padded to 512): those become 'gen' records (seed, size, first
        block) that the test regenerates, so the file data is not stored."""
        blocks = {}
        for seed, size in self.known:
            d = lcg(seed, size)
            d += bytes(-len(d) % SEC)
            for k in range(len(d) // SEC):
                blocks.setdefault(d[k * SEC:(k + 1) * SEC], (seed, size, k))
        recs = []  # ["raw", lba, bytearray] / ["gen", lba, count, seed, size, k0]
        for lba, s in self.nonzero_sectors():
            g = blocks.get(s)
            last = recs[-1] if recs else None
            if g:
                seed, size, k = g
                if last and last[0] == "gen" and last[3] == seed and last[4] == size and last[1] + last[2] == lba and last[5] + last[2] == k:
                    last[2] += 1
                else:
                    recs.append(["gen", lba, 1, seed, size, k])
            else:
                if last and last[0] == "raw" and last[1] + len(last[2]) // SEC == lba:
                    last[2] += s
                else:
                    recs.append(["raw", lba, bytearray(s)])
        with open(out, "wb") as o:
            o.write(b"SPRS" + struct.pack("<I", self.total))
            for r in recs:
                if r[0] == "raw":
                    o.write(struct.pack("<III", 0, r[1], len(r[2]) // SEC) + bytes(r[2]))
                else:
                    o.write(struct.pack("<IIIIII", 1, r[1], r[2], r[3], r[4], r[5]))
        return len(recs)

    def close(self):
        shutil.rmtree(self.tmp, ignore_errors=True)


def write_meta(path, desc, files, mount="ok", ideal=None, note=None):
    """files: list of (name11, seed, size, expect)."""
    with open(path, "w", newline="\n") as f:
        f.write("desc=%s\n" % desc)
        f.write("mount=%s\n" % mount)
        for n, seed, size, exp in files:
            f.write("file=%s:%d:%d:%s\n" % (n, seed, size, exp))
        if ideal:
            f.write("ideal=%s\n" % ideal)
        if note:
            f.write("note=%s\n" % note)


OGF = "NETGD   OGF"
BIT = "NETGD   BIT"
GIB = 2 * 1024 * 1024  # sectors per GiB


def min_total(spc, extra=4096):
    """Volume size (sectors) that mkfs.fat accepts as FAT32 at this cluster
    size (>= 65525 clusters), with room for the FAT and reserved sectors."""
    return 65600 * spc + extra + 2 * (65600 // 128 + 1) + 64


class Gen:
    def __init__(self, work, out):
        self.work, self.out = work, out
        self.report = []

    def case(self, name, total, spc, *, desc, start=0, ptype=None, nfats=2, reserved=32, boot=False,
             pre_parts=None, populate=None, files=None, mount="ok", ideal=None, note=None,
             sector=512, total_vol=None):
        print(name)
        p = os.path.join(self.work, name + ".img")
        im = Img(p, total)
        try:
            if ptype:
                parts = list(pre_parts or [])
                parts.append((start, total_vol or total - start, ptype, boot))
                im.mbr(parts)
            im.set_start(start)
            im.mkfs(spc, nfats, reserved, sector, sectors=total_vol)
            if populate:
                populate(im)
            n = im.extract(os.path.join(self.out, name + ".sparse"))
            write_meta(os.path.join(self.out, name + ".txt"), desc, files if files is not None else [(OGF, 7, 6000, "ok")], mount, ideal, note)
            self.report.append((name, n, os.path.getsize(os.path.join(self.out, name + ".sparse"))))
        finally:
            im.close()
            os.unlink(p)


def basic(im):
    im.put_seed("/NETGD.BIT", 9, 3000)
    im.put_seed("/NETGD.OGF", 7, 6000)


def top_nibble(nib):
    def f(im):
        p = im.bpb()
        for i in range(p["nfats"]):
            fat = bytearray(im.fat(i))
            for o in range(0, len(fat), 4):
                v = struct.unpack_from("<I", fat, o)[0]
                if v & 0x0FFFFFFF:
                    struct.pack_into("<I", fat, o, (v & 0x0FFFFFFF) | (nib << 28))
            im.set_fat(i, bytes(fat))
    return f


def fragmenting(size, nfill=12):
    """Fillers of one cluster each, delete every other one, then write the
    target. mtools allocates next-fit from the FSInfo hint, so the hint is
    set back to cluster 2 first (what a card has after the hint was lost or
    reset); the target then lands in the holes and continues after the last
    filler."""
    def f(im):
        cl = im.bpb()["spc"] * SEC
        for i in range(nfill):
            im.put_seed("/FILL%02d.DAT" % i, 100 + i, cl)
        for i in range(0, nfill, 2):
            im.rm("/FILL%02d.DAT" % i)
        im.wr(1 * SEC + 0x1EC, struct.pack("<I", 2))
        im.put_seed("/NETGD.OGF", 7, size)
        im.put_seed("/NETGD.BIT", 9, 3000)
        j, n = im.frag_jumps(im.find_first_cluster(b"NETGD   OGF"))
        print("   fragmented: %d jumps in %d clusters" % (j, n))
        assert j >= 3, "not fragmented enough"
    return f


def main():
    out = sys.argv[1]
    sdcard = None
    if "--sdcard" in sys.argv:
        sdcard = sys.argv[sys.argv.index("--sdcard") + 1]
    os.makedirs(out, exist_ok=True)
    work = tempfile.mkdtemp(prefix="fatreal-")
    g = Gen(work, out)
    both = lambda n: [(OGF, 7, n, "ok"), (BIT, 9, 3000, "ok")]

    # 1. superfloppy, every cluster size
    for spc in (1, 2, 4, 8, 16, 32, 64):
        size = 20000 if spc <= 8 else 6000
        def pop(im, size=size):
            im.put_seed("/NETGD.BIT", 9, 3000)
            im.put_seed("/NETGD.OGF", 7, size)
        g.case("sf_spc%d" % spc, min_total(spc), spc, populate=pop, files=both(size),
               desc="mkfs.fat -F 32 -s %d, no partition table (LBA 0 is the BPB), %d-byte clusters" % (spc, spc * 512))

    # 2. MBR variants: type 0x0C / 0x0B, start 2048 / 63 / odd, bootable flag
    for ty, st in (("c", 2048), ("b", 2048), ("c", 63), ("b", 12345), ("c", 8192)):
        g.case("mbr_%s_%d" % (ty, st), st + min_total(4), 4, start=st, ptype=ty, boot=(st == 2048), populate=basic, files=both(6000),
               desc="MBR entry 0 type 0x0%s starting at LBA %d, mkfs.fat -s 4" % (ty.upper(), st))

    # 3. partition not in entry 0
    small = 40000
    vol = min_total(4)
    st1 = 2048 + small + 2048
    g.case("mbr_entry1", st1 + vol, 4, start=st1, ptype="c", pre_parts=[(2048, small, "83", False)], total_vol=vol, populate=basic,
           files=both(6000), mount="NotFat32", ideal="ok",
           desc="the FAT32 volume (0x0C) is MBR entry 1; entry 0 is a small type-0x83 partition",
           note="mount only looks at MBR entry 0 (src/fat.rs Volume::mount)")

    # 4. 32 GB-class cards. SD Association layout of a 32 GB SDHC card:
    # 62333952 sectors, partition at 8192, 32 KiB clusters.
    sd32 = 62333952

    def big_pop(dummy_clusters):
        def f(im):
            cl = im.bpb()["spc"] * SEC
            if dummy_clusters:
                # push the files far into the volume (FAT sector index > 0), then free the space again
                dummy = os.path.join(im.tmp, "dummy")
                with open(dummy, "wb") as d:
                    d.truncate(dummy_clusters * cl)
                run("mcopy", "-i", im.mt, dummy, "::/DUMMY.BIN")
                os.unlink(dummy)
            im.put_seed("/NETGD.BIT", 9, 3000)
            im.put_seed("/NETGD.OGF", 7, 6000)
            if dummy_clusters:
                im.rm("/DUMMY.BIN")
            print("   target first cluster:", im.find_first_cluster(b"NETGD   OGF"))
        return f
    g.case("sd32g_sda_32k", sd32, 64, start=8192, ptype="c", populate=big_pop(0), files=both(6000),
           desc="32 GB SDHC, SD-Association layout: partition at 8192, 62333952 sectors, 32 KiB clusters, type 0x0C")
    g.case("sd32g_16k_far", sd32, 32, start=8192, ptype="c", populate=big_pop(20000), files=both(6000),
           desc="32 GB card, 16 KiB clusters (what Windows picks), target file at cluster ~20000 (FAT sector ~156)")
    g.case("sd32gib_2048_far", 32 * GIB, 64, start=2048, ptype="c", populate=big_pop(12000), files=both(6000),
           desc="exactly 32 GiB, partition at 2048, 32 KiB clusters, target file at cluster ~12000")

    # 5. FAT flags: active FAT / mirroring (BPB 0x28), single FAT
    def flags(val, stale0):
        def f(im):
            snap = im.fat(0) if stale0 else None
            basic(im)
            if stale0:
                im.set_fat(0, snap)
            im.wr(0x28, struct.pack("<H", val))
            im.wr(6 * SEC + 0x28, struct.pack("<H", val))  # backup BPB
        return f
    g.case("fat_nomirror_active0", min_total(1), 1, populate=flags(0x0080, False), files=both(6000),
           desc="BPB 0x28 = 0x0080: mirroring off, active FAT 0")
    g.case("fat_active1_identical", min_total(1), 1, populate=flags(0x0081, False), files=both(6000),
           desc="BPB 0x28 = 0x0081: mirroring off, active FAT 1; both FATs identical")
    g.case("fat_active1_stale0", min_total(1), 1, populate=flags(0x0081, True),
           files=[(OGF, 7, 6000, "BadChain"), (BIT, 9, 3000, "BadChain")], ideal="ok",
           desc="BPB 0x28 = 0x0081: active FAT 1 is current, FAT 0 is a stale copy from before the files were written (the spec lets the inactive FAT lag)",
           note="the reader reads FAT 0 only (documented in src/fat.rs), so a card whose active FAT is 1 and whose FAT 0 lags is misread")
    g.case("fat_single", min_total(2), 2, nfats=1, populate=basic, files=both(6000), desc="mkfs.fat -f 1: one FAT only")
    g.case("fat_res2", min_total(2), 2, reserved=2, populate=basic, files=both(6000), desc="mkfs.fat -R 2: the minimal reserved area FAT32 allows (no room for the backup BPB at sector 6)")

    # 6. reserved top 4 bits of FAT entries (on a fragmented file, so chains are really followed)
    for nib in (0xF, 0xA, 0x5):
        def pop(im, nib=nib):
            fragmenting(20000)(im)
            top_nibble(nib)(im)
        g.case("fat_top_%x" % nib, min_total(1), 1, populate=pop, files=both(20000),
               desc="every non-free FAT entry (EOC marks too) has its top 4 bits set to 0x%X, in both FATs; fragmented target" % nib)

    # 7. fragmentation made by create / delete / create
    for spc in (1, 2, 8):
        size = 20000 if spc < 8 else 30000
        g.case("frag_spc%d" % spc, min_total(spc), spc, populate=fragmenting(size), files=both(size),
               desc="target written into the holes left by deleted fillers (mcopy/mdel), %d-byte clusters" % (spc * 512))

    # 8. names: 8.3 / long names, lower case, decoys, deleted entries, subdirectory
    def names(im):
        im.put_seed("/netgd.ogfx", 31, 700)
        im.put_seed("/NETGD-OLD.OGF", 32, 700)
        im.put_seed("/netgd.ogf.bak", 33, 700)
        im.put_seed("/NETGD.OG", 34, 700)
        im.put_seed("/A very long file name for a game (USA) (Disc 1).cdi", 35, 700)
        im.put_seed("/NETGD.OGF", 36, 1000)   # old content ...
        im.put_seed("/Z.TXT", 37, 100)
        im.put_seed("/NETGD.BIT", 9, 3000)
        im.rm("/NETGD.OGF")                    # ... deleted: its E5 entry stays (the next file needs 2 slots, the hole has 1)
        im.put_seed("/NetGD.Ogf", 7, 6000)     # mixed case: LFN entry + 8.3 NETGD.OGF
        n = im.deleted_entries(b"NETGD   OGF")
        print("   deleted NETGD.OGF entries left:", n)
        assert n == 1
    g.case("names_lfn_decoys", min_total(1), 1, populate=names, files=both(6000),
           desc="mixed-case long name NetGD.Ogf (LFN + 8.3 entry); decoys NETGD.OGFX, NETGD-OLD.OGF, netgd.ogf.bak, NETGD.OG; a deleted NETGD.OGF before")

    def lower(im):
        im.put_seed("/netgd.ogf", 7, 6000)
        im.put_seed("/netgd.bit", 9, 3000)
    g.case("names_lowercase", min_total(1), 1, populate=lower, files=both(6000),
           desc="lower-case netgd.ogf / netgd.bit: 8.3 entries carrying the NT lower-case flag (no LFN)")

    def subdir_only(im):
        im.mkdir("/SUB")
        im.put_seed("/SUB/NETGD.OGF", 7, 6000)
        im.put_seed("/NETGD.BIT", 9, 3000)
    g.case("subdir_only", min_total(1), 1, populate=subdir_only,
           files=[(OGF, 7, 6000, "NotFound"), (BIT, 9, 3000, "ok")],
           desc="NETGD.OGF only inside a subdirectory (not found by the root lookup); NETGD.BIT in the root")

    def subdir_both(im):
        im.mkdir("/SUB")
        im.put_seed("/SUB/NETGD.OGF", 77, 6000)
        im.put_seed("/NETGD.OGF", 7, 6000)
    g.case("subdir_and_root", min_total(1), 1, populate=subdir_both, files=[(OGF, 7, 6000, "ok")],
           desc="NETGD.OGF in a subdirectory (different content) and in the root; the root one is the answer")

    # 9. a real NetGD card: mksdcard folders + NETGD files, many long names, target last
    if sdcard:
        def card(nlong):
            def f(im):
                for d in sorted(os.listdir(sdcard)):
                    pth = os.path.join(sdcard, d)
                    if os.path.isdir(pth):
                        im.mkdir("/" + d)
                        for fn in sorted(os.listdir(pth)):
                            data = open(os.path.join(pth, fn), "rb").read()
                            if len(data) > 3000 and not fn.endswith((".gdi", ".txt")):
                                data = data[:512]   # keep names and structure, not the megabytes
                            im.put("/%s/%s" % (d, fn), data)
                    else:
                        im.put("/" + d, open(pth, "rb").read()[:1024])
                im.mkdir("/System Volume Information")
                for i in range(nlong):
                    im.put("/Game Title Number %03d (Europe) (En,Fr,De) [Disc 1 of 2] v1.%d.txt" % (i, i % 7), lcg(500 + i, 40) if i % 10 == 0 else b"")
                im.put_seed("/NETGD.BIT", 9, 5000)
                im.put_seed("/NETGD.OGF", 7, 70000)
                cap, nclu = im.root_capacity()
                print("   root directory: %d entry slots over %d clusters" % (cap, nclu))
            return f
        g.case("card_mksdcard_4k", 8192 + min_total(8), 8, start=8192, ptype="c", populate=card(60),
               files=[(OGF, 7, 70000, "ok"), (BIT, 9, 5000, "ok")],
               desc="mksdcard --small folders 01..07 + manifest, 60 long-named files, System Volume Information, then NETGD.BIT and NETGD.OGF (70000 B) last; 4 KiB clusters, MBR 0x0C at 8192")
        g.case("card_mksdcard_512", min_total(1), 1, populate=card(40),
               files=[(OGF, 7, 70000, "ok"), (BIT, 9, 5000, "ok")],
               desc="same card contents, 512-byte clusters, superfloppy: the root directory spans many clusters")

    # 10. not a 512-byte-sector volume
    g.case("sector4096", min_total(1) // 8 + 4096, 1, sector=4096, populate=None, files=[], mount="REFUSE",
           desc="mkfs.fat -S 4096 (4Kn logical sectors, never on an SD card): must be refused, not misread")

    tot = 0
    for n, ns, sz in g.report:
        print("%-24s %6d records %9d bytes" % (n, ns, sz))
        tot += sz
    print("total", tot)
    shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    main()
