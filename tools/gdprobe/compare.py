#!/usr/bin/env python3
"""gdprobe record tools (Python 3, standard library only).

  compare.py A.txt B.txt            per-probe differences between two records
                                    (e.g. stock GDEMU vs NetGD, or two runs of the same device)
  compare.py --image DISC.gdi REC   recompute per-sector hashes from the image and check the record
  compare.py --raw FILE --raw-start-fad N [--raw-sector-size 2352] REC
                                    same, for one raw track file (e.g. a CDI data track you extracted)

Exit status: 0 = no significant difference / all hashes match, 1 = differences / mismatches, 2 = usage or input error.

Record format (see README_ja.md): one line per probe,
    img=3 probe=REQ_SES_2 [extra k=v ...] cdb=... rc=ok status=0x50 err=0x00 irr=0x03 bytes=6 data=... us=1234
    img=3 probe=HASH fad=45150 ssz=2048 crc=0badf00d
Lines starting with '#' are header/trailer comments and are ignored.

Image assumptions (the `--image` check):
  * CD_READ addresses are FADs; image LBA = FAD - 150. The GDI track table gives each track's start LBA,
    sector size, file and byte offset; sector N of a track is at offset + (LBA - start_lba) * sector_size.
  * The record's ssz tells what the drive returned. ssz 2048 = the user data area: taken from a 2048-byte
    image sector as is, or from a 2352-byte raw sector at offset 16 (Mode 1) / 24 (Mode 2 Form 1; the mode
    byte at offset 15 decides). ssz 2352 = the whole raw sector (needs a 2352-byte image).
    Mode 2 Form 2 sectors (2324 bytes of data) are not modelled and will show as mismatches.
  * The hash is CRC-32 (zlib.crc32) of those bytes, which is what the tool wrote.
"""
import argparse
import os
import re
import shlex
import sys
import zlib

TIMING_RATIO_DEFAULT = 1.5


# ------------------------------------------------------------------ parsing
def read_text(path):
    with open(path, encoding='utf-8', errors='replace') as f:
        return f.read()


def parse_record(text):
    """Return (header_lines, entries). entries: list of dicts with 'img', 'probe', all key=value fields."""
    header, entries = [], []
    for raw in text.splitlines():
        line = raw.strip()
        if not line:
            continue
        if line.startswith('#'):
            header.append(line.lstrip('# ').strip())
            continue
        fields = {}
        for tok in line.split(' '):
            if '=' in tok:
                k, v = tok.split('=', 1)
                fields[k] = v
        if 'probe' not in fields:
            continue
        try:
            fields['img'] = int(fields.get('img', '0'))
        except ValueError:
            fields['img'] = 0
        fields['_line'] = line
        entries.append(fields)
    return header, entries


def index_entries(entries):
    """Key every entry: (img, probe, n) with n the occurrence number; HASH is keyed by (img, 'HASH', fad)."""
    idx, count = {}, {}
    for e in entries:
        if e['probe'] == 'HASH':
            key = (e['img'], 'HASH', e.get('fad', '?'))
        else:
            k0 = (e['img'], e['probe'])
            n = count.get(k0, 0)
            count[k0] = n + 1
            key = (e['img'], e['probe'], n)
        idx[key] = e
    return idx


def key_name(key):
    img, probe, extra = key
    if probe == 'HASH':
        return 'img=%d HASH fad=%s' % (img, extra)
    return 'img=%d %s%s' % (img, probe, '' if extra == 0 else '#%d' % extra)


def sort_key(key):
    img, probe, extra = key
    return (img, probe == 'HASH', probe, int(extra) if str(extra).isdigit() else 0)


def to_int(s, default=None):
    try:
        return int(s, 0)
    except (TypeError, ValueError):
        return default


# ------------------------------------------------------------------ compare
FIELDS_EXACT = ('rc', 'status', 'err', 'irr', 'bytes')
# fields of event lines that are timings (compared by ratio, never as exact differences)
TIMING_FIELDS = ('us', 't_ms', 'poll', 'polls', 'changes', 'min_us', 'max_us', 'kib_s')
NO_COMPARE = ('img', 'probe', '_line', 'cdb', 'data')


def diff_hex(a, b, limit=8):
    """Offsets (and values) where two hex strings differ."""
    n = min(len(a), len(b)) // 2
    out = []
    for i in range(n):
        x, y = a[2 * i:2 * i + 2], b[2 * i:2 * i + 2]
        if x != y:
            out.append('+%d: %s!=%s' % (i, x, y))
    total = len(out)
    shown = out[:limit]
    if total > limit:
        shown.append('... %d more' % (total - limit))
    if len(a) != len(b):
        shown.append('length %d vs %d bytes shown' % (len(a) // 2, len(b) // 2))
    return shown


def ratio(a, b):
    if a is None or b is None:
        return None
    if a == 0 and b == 0:
        return 1.0
    if a == 0 or b == 0:
        return float('inf')
    return b / a


def compare_records(ea, eb, ratio_limit=TIMING_RATIO_DEFAULT, ignore_timing=False):
    """Return (diffs, timing_notes). Each is a list of strings."""
    ia, ib = index_entries(ea), index_entries(eb)
    diffs, notes = [], []
    for key in sorted(set(ia) | set(ib), key=sort_key):
        a, b = ia.get(key), ib.get(key)
        name = key_name(key)
        if a is None:
            diffs.append('%s: only in B' % name)
            continue
        if b is None:
            diffs.append('%s: only in A' % name)
            continue
        if key[1] == 'HASH':
            if a.get('crc') != b.get('crc') or a.get('ssz') != b.get('ssz'):
                diffs.append('%s: crc %s(ssz %s) vs %s(ssz %s)' % (name, a.get('crc'), a.get('ssz'), b.get('crc'), b.get('ssz')))
            continue
        parts = []
        if a.get('cdb') != b.get('cdb'):
            parts.append('cdb %s vs %s' % (a.get('cdb'), b.get('cdb')))
        for f in FIELDS_EXACT:
            if a.get(f) != b.get(f):
                parts.append('%s %s vs %s' % (f, a.get(f), b.get(f)))
        if a.get('data') != b.get('data'):
            da, db = a.get('data', ''), b.get('data', '')
            parts.append('data differs (%s)' % '; '.join(diff_hex(da, db)))
        # free-form event fields (n=, ready=, sense=, ok=, abort= ...) except timings
        for f in sorted((set(a) | set(b)) - set(NO_COMPARE) - set(FIELDS_EXACT) - set(TIMING_FIELDS) - {'us'}):
            if a.get(f) != b.get(f):
                parts.append('%s %s vs %s' % (f, a.get(f), b.get(f)))
        if parts:
            diffs.append('%s: %s' % (name, ' | '.join(parts)))
        if not ignore_timing:
            for f in ('us', 't_ms', 'kib_s'):
                va, vb = to_int(a.get(f)), to_int(b.get(f))
                r = ratio(va, vb)
                if r is None:
                    continue
                # tiny absolute values are noise (< 2 ms or 2 ms)
                if f == 'us' and max(va, vb) < 2000:
                    continue
                if f == 't_ms' and max(va, vb) < 2:
                    continue
                if r > ratio_limit or r < 1.0 / ratio_limit:
                    notes.append('%s: %s %s -> %s (x%.2f)' % (name, f, va, vb, r))
    return diffs, notes


def cmd_compare(args):
    ha, ea = parse_record(read_text(args.a))
    hb, eb = parse_record(read_text(args.b))
    diffs, notes = compare_records(ea, eb, args.ratio, args.ignore_timing)
    print('A: %s  (%d probes) %s' % (args.a, len(ea), '; '.join(h for h in ha[:1])))
    print('B: %s  (%d probes) %s' % (args.b, len(eb), '; '.join(h for h in hb[:1])))
    print()
    if diffs:
        print('DIFFERENCES (%d):' % len(diffs))
        for d in diffs:
            print('  ' + d)
    else:
        print('No differences in rc / status / error / size / data / hashes.')
    if notes:
        print()
        print('TIMING (ratio B/A beyond x%.2f or below x%.2f; not counted as differences):' % (args.ratio, 1 / args.ratio))
        for n in notes:
            print('  ' + n)
    print()
    print('summary: %d difference(s), %d timing note(s)' % (len(diffs), len(notes)))
    return 1 if diffs else 0


# ------------------------------------------------------------------ images
class Track:
    def __init__(self, num, lba, ctrl, ssz, path, off):
        self.num, self.lba, self.ctrl, self.ssz, self.path, self.off = num, lba, ctrl, ssz, path, off
        self.count = None  # filled in from the file size


def parse_gdi(path):
    base = os.path.dirname(os.path.abspath(path))
    lines = [l.strip() for l in read_text(path).splitlines() if l.strip()]
    n = int(lines[0])
    tracks = []
    for l in lines[1:1 + n]:
        t = shlex.split(l)  # handles "file name.bin"
        num, lba, ctrl, ssz, fname, off = int(t[0]), int(t[1]), int(t[2]), int(t[3]), t[4], int(t[5])
        tracks.append(Track(num, lba, ctrl, ssz, os.path.join(base, fname), off))
    for t in tracks:
        try:
            t.count = (os.path.getsize(t.path) - t.off) // t.ssz
        except OSError:
            t.count = None
    return tracks


def image_sector(tracks, lba, want_ssz):
    """Bytes the drive should return for `lba` with sector size `want_ssz`, or (None, reason)."""
    best = None
    for t in tracks:
        if t.lba <= lba and (best is None or t.lba > best.lba):
            best = t
    if best is None:
        return None, 'before the first track'
    if best.count is None:
        return None, 'track file missing: %s' % best.path
    rel = lba - best.lba
    if rel >= best.count:
        return None, 'beyond the end of track %d' % best.num
    return read_sector(best.path, best.off + rel * best.ssz, best.ssz, want_ssz)


def read_sector(path, offset, file_ssz, want_ssz):
    with open(path, 'rb') as f:
        f.seek(offset)
        raw = f.read(file_ssz)
    if len(raw) != file_ssz:
        return None, 'short read'
    if want_ssz == file_ssz:
        return raw, None
    if want_ssz == 2048 and file_ssz == 2352:
        mode = raw[15]
        if mode == 1:
            return raw[16:16 + 2048], None
        if mode == 2:
            return raw[24:24 + 2048], None
        return None, 'unknown sector mode %d (audio track?)' % mode
    return None, 'cannot make %d-byte sectors from a %d-byte image' % (want_ssz, file_ssz)


def check_hashes(entries, getter, img=None):
    ok = bad = skipped = 0
    problems = []
    for e in entries:
        if e['probe'] != 'HASH':
            continue
        if img is not None and e['img'] != img:
            continue
        fad, ssz, crc = to_int(e.get('fad')), to_int(e.get('ssz')), e.get('crc')
        if fad is None or ssz is None or crc is None:
            continue
        data, why = getter(fad - 150, ssz)
        if data is None:
            skipped += 1
            problems.append('img=%d fad=%d: not checked (%s)' % (e['img'], fad, why))
            continue
        want = '%08x' % (zlib.crc32(data) & 0xFFFFFFFF)
        if want == crc.lower():
            ok += 1
        else:
            bad += 1
            problems.append('img=%d fad=%d: MISMATCH record crc=%s image crc=%s (ssz %d)' % (e['img'], fad, crc, want, ssz))
    return ok, bad, skipped, problems


def cmd_image(args):
    _, entries = parse_record(read_text(args.record))
    if args.raw:
        size = args.raw_sector_size
        start_lba = args.raw_start_fad - 150
        total = os.path.getsize(args.raw) // size

        def getter(lba, want):
            rel = lba - start_lba
            if rel < 0 or rel >= total:
                return None, 'outside the raw file'
            return read_sector(args.raw, rel * size, size, want)
    else:
        tracks = parse_gdi(args.image)

        def getter(lba, want):
            return image_sector(tracks, lba, want)
    imgs = sorted({e['img'] for e in entries if e['probe'] == 'HASH'})
    if args.img is None and len(imgs) > 1:
        print('note: the record has hashes for images %s; checking all of them against this one image.'
              ' Use --img N to check only one.' % imgs)
    ok, bad, skipped, problems = check_hashes(entries, getter, args.img)
    for p in problems[:args.max_report]:
        print('  ' + p)
    if len(problems) > args.max_report:
        print('  ... %d more' % (len(problems) - args.max_report))
    print('hashes: %d match, %d mismatch, %d not checked' % (ok, bad, skipped))
    if ok + bad == 0:
        print('nothing was checked (no HASH lines for this image, or the image does not cover them)')
        return 2
    return 1 if bad else 0


# --------------------------------------------------------------------- main
def main(argv=None):
    ap = argparse.ArgumentParser(description='gdprobe record compare / image check')
    ap.add_argument('files', nargs='*', help='A.txt B.txt  |  REC.txt with --image/--raw')
    ap.add_argument('--image', help='GDI file to check the sector hashes against')
    ap.add_argument('--raw', help='one raw track file to check against (with --raw-start-fad)')
    ap.add_argument('--raw-start-fad', type=int, default=0, help='FAD of the first sector of the raw file')
    ap.add_argument('--raw-sector-size', type=int, default=2352, choices=(2048, 2352))
    ap.add_argument('--img', type=int, help='check only HASH lines tagged with this image number')
    ap.add_argument('--ratio', type=float, default=TIMING_RATIO_DEFAULT, help='timing ratio worth a note (default 1.5)')
    ap.add_argument('--ignore-timing', action='store_true')
    ap.add_argument('--max-report', type=int, default=20)
    args = ap.parse_args(argv)
    try:
        if args.image or args.raw:
            if len(args.files) != 1:
                ap.error('--image/--raw take exactly one record file')
            args.record = args.files[0]
            return cmd_image(args)
        if len(args.files) != 2:
            ap.error('give two record files, or --image DISC.gdi REC.txt')
        args.a, args.b = args.files
        return cmd_compare(args)
    except (OSError, ValueError, IndexError) as e:
        print('error: %s' % e, file=sys.stderr)
        return 2


if __name__ == '__main__':
    sys.exit(main())
