#!/usr/bin/env python3
"""mksdcard: build the test SD card used to probe a stock GDEMU with tools/gdprobe.

    python mksdcard.py OUTDIR [--menu DIR_01] [--dcload disc.cdi] [--small] [--force]

Creates OUTDIR/01 .. 07 (see docs/research/stock-gdemu-flow.md, section 3) and
OUTDIR/NETGD-PROBE-MANIFEST.json. Python 3 standard library only. Output is
deterministic: the same arguments give the same bytes.

Images generated here (all synthetic, no copyrighted content, they do not boot):
  03  GD-ROM GDI : track01 data (LBA 0), track02 audio (756), track03 data (45000, IP.BIN header)
  04  CDI        : session 1 = one audio track, session 2 = one data track (MIL-CD "audio/data")
  05  CDI        : session 1 = two audio tracks, session 2 = one data track
  06  GDI        : like 03 with three audio tracks in the low-density area, data track05 at 45000
  10  CDI        : like 04 (one audio track, then a data session) but the data track is Mode 1 stored as
                   2352-byte raw sectors (DiscJuggler track mode 1, read mode 2), not mkdcdisc's Mode 2 Form 1
                   (--set variants only; LOG R137 g)

Data sectors
  Each 2048-byte user-data block starts with the ASCII tag
  "NETGD-PROBE img=03 trk=3 lba=00045000 " followed by bytes from SHAKE-128 seeded by
  (img, trk, lba). In the first sector of a track that carries an IP.BIN header (the
  data track of 03/06, the session-2 data track of 04/05) the 256-byte IP.BIN header sits at
  offset 0 and the tag moves to offset 0x100.
  GDI data tracks are raw 2352-byte Mode 1 sectors (sync, header, data, EDC, P/Q ECC; ECMA-130).
  CDI data tracks are 2336-byte Mode 2 Form 1 sectors (subheader, data, EDC, ECC), as mkdcdisc writes them.
  the variant image 10 stores 2352-byte raw Mode 1 sectors instead (identical to the GDI data sectors).
  Audio sectors: 2352 bytes of 16-bit stereo PCM, a quiet sawtooth; the first stereo frame
  of each sector is (left = lba & 0xFFFF, right = lba >> 16).

CDI layout: this follows mkdcdisc (https://gitlab.com/simulant/mkdcdisc, MIT:
src/disc_image/formats/cdi.c, src/disc_image/disc_image.c, src/disc_layout.cpp,
src/disc_image/raw_sector.c): track data back to back (pregaps stored), then the descriptor,
then a u32 descriptor length. Pregap 150 on every track, 2-sector postgap on data tracks,
audio padded to >= 302 sectors, session length = 4500 lead-in + (6750 | 2250) lead-out +
tracks, descriptor start_lba = sum of earlier session lengths (11702 for one 302-sector
audio track), "CDI v3.5" version word 0x80000006.

Manifest: per track type, LBA range, sector size, file, generation parameters and the CRC-32
of the track data; "sector_crc" tells how gdprobe's HASH lines are reproduced
(tools/gdprobe/compare.py --manifest).
"""
import argparse
import hashlib
import json
import os
import shutil
import struct
import sys
import zlib
from operator import itemgetter

VERSION = 1
MANIFEST_NAME = 'NETGD-PROBE-MANIFEST.json'

# ----------------------------------------------------------------------------
# EDC / ECC (ECMA-130 section 14 / annex A)
# ----------------------------------------------------------------------------
_EDC_TABLE = []
for _i in range(256):
    _c = _i
    for _ in range(8):
        _c = (_c >> 1) ^ (0xD8018001 if _c & 1 else 0)
    _EDC_TABLE.append(_c)

_F_LUT = bytes(((i << 1) ^ (0x11D if i & 0x80 else 0)) & 0xFF for i in range(256))
_B_LUT_L = [0] * 256
for _i in range(256):
    _B_LUT_L[_i ^ _F_LUT[_i]] = _i
_B_LUT = bytes(_B_LUT_L)


def edc(data):
    """EDC: CRC with reflected polynomial 0xD8018001 (x^32+x^31+x^16+x^15+x^4+x^3+x+1), init 0, no final xor."""
    crc = 0
    t = _EDC_TABLE
    for b in data:
        crc = (crc >> 8) ^ t[(crc ^ b) & 0xFF]
    return crc


_IDX0 = [(m >> 1) * 86 + (m & 1) for m in range(52)]
_QG = [itemgetter(*[(i + 88 * k) % 2236 for i in _IDX0]) for k in range(43)]


def _xtime(x, n):
    return int.from_bytes(x.to_bytes(n, 'little').translate(_F_LUT), 'little')


def _finish(a, b, n):
    fa = int.from_bytes(a.to_bytes(n, 'little').translate(_F_LUT), 'little')
    res = int.from_bytes((fa ^ b).to_bytes(n, 'little').translate(_B_LUT), 'little')
    return res.to_bytes(n, 'little'), (res ^ b).to_bytes(n, 'little')


def ecc_p(src):
    """172 P-parity bytes; src = sector[12:] (the first 2064 bytes are read)."""
    a = b = 0
    for k in range(24):
        v = int.from_bytes(src[86 * k:86 * k + 86], 'little')
        a ^= v
        b = _xtime(b ^ v, 86)
    p0, p1 = _finish(a, b, 86)
    return p0 + p1


def ecc_q(src):
    """104 Q-parity bytes; src = sector[12:] (the first 2236 bytes are read, P parity included)."""
    a = b = 0
    for g in _QG:
        v = int.from_bytes(bytes(g(src)), 'little')
        a ^= v
        b = _xtime(b ^ v, 52)
    q0, q1 = _finish(a, b, 52)
    return q0 + q1


def add_ecc(sector):
    """Fill P (0x81C) and Q (0x8C8) of a 2352-byte sector bytearray. The header (12..15) must be
    zero when this is called for Mode 2 (the caller restores it)."""
    sector[0x81C:0x81C + 172] = ecc_p(bytes(sector[12:12 + 2064]))
    sector[0x8C8:0x8C8 + 104] = ecc_q(bytes(sector[12:12 + 2236]))


SYNC = b'\x00' + b'\xff' * 10 + b'\x00'


def bcd(n):
    return ((n // 10) << 4) | (n % 10)


def msf_bytes(lba):
    """MSF of the absolute address lba + 150, BCD."""
    f = lba + 150
    return bytes((bcd(f // 4500), bcd((f % 4500) // 75), bcd(f % 75)))


def raw_mode1(lba, user):
    """2352-byte Mode 1 sector: sync, header, 2048 user bytes, EDC, 8 zero bytes, P, Q."""
    assert len(user) == 2048
    s = bytearray(2352)
    s[0:12] = SYNC
    s[12:15] = msf_bytes(lba)
    s[15] = 1
    s[16:2064] = user
    s[2064:2068] = struct.pack('<I', edc(bytes(s[0:2064])))
    add_ecc(s)
    return bytes(s)


FORM1_SUBHEADER = bytes((0, 0, 0x09, 0, 0, 0, 0x09, 0))   # mkdcdisc raw_sector.c


def stored_form1(user):
    """2336-byte Mode 2 Form 1 sector without sync/header, as a CDI data track stores it."""
    assert len(user) == 2048
    s = bytearray(2352)
    s[16:24] = FORM1_SUBHEADER
    s[24:2072] = user
    s[2072:2076] = struct.pack('<I', edc(bytes(s[16:2072])))
    add_ecc(s)           # header bytes are zero here, as ECMA-130 requires for Mode 2
    return bytes(s[16:])


def raw_xa_form1(lba, user):
    """The 2352-byte raw form of a CDI data sector (sync + header with mode 2 + stored 2336 bytes)."""
    return SYNC + msf_bytes(lba) + b'\x02' + stored_form1(user)


_gap = bytearray(2336)           # mkdcdisc write_mode2_pregap
_gap[0:8] = bytes((0, 0, 0x20, 0, 0, 0, 0x20, 0))
_gap[2332:2336] = bytes((0x3F, 0x13, 0xB0, 0xBE))
FORM2_GAP = bytes(_gap)

# ----------------------------------------------------------------------------
# Deterministic payloads
# ----------------------------------------------------------------------------


def tag(img, trk, lba):
    return ('NETGD-PROBE img=%02d trk=%d lba=%08d ' % (img, trk, lba)).encode('ascii')


def ip_bin_header(product):
    """A minimal 256-byte IP.BIN header: hardware id, maker, device info, areas, peripherals,
    product number, version, date, boot file, company, software name."""
    def f(s, n):
        return s.encode('ascii').ljust(n, b' ')[:n]
    h = (f('SEGA SEGAKATANA ', 16) + f('SEGA ENTERPRISES', 16) + f('0000 GD-ROM1/1', 16) +
         f('JUE', 8) + f('E000F10', 8) + f('T-NETGD', 10) + f('V1.000', 6) + f('20260101', 16) +
         f('1ST_READ.BIN', 16) + f('NETGD', 16) + f(product, 128))
    assert len(h) == 256
    return h


def user_data(img, trk, lba, ip_product=None):
    """The 2048 user bytes of the data sector at `lba` of track `trk` of image `img`."""
    t = tag(img, trk, lba)
    body = hashlib.shake_128(b'netgd-probe %d %d %d' % (img, trk, lba)).digest(2048 - len(t))
    blk = t + body
    if ip_product is not None:
        return ip_bin_header(ip_product) + blk[:2048 - 256]
    return blk


def audio_sector(lba):
    """2352 bytes: 588 stereo 16-bit frames; frame 0 = (lba & 0xFFFF, lba >> 16), then a quiet sawtooth."""
    out = bytearray(2352)
    struct.pack_into('<HH', out, 0, lba & 0xFFFF, (lba >> 16) & 0xFFFF)
    base = lba * 588
    for f in range(1, 588):
        v = (((base + f) * 40) % 4000) - 2000
        struct.pack_into('<hh', out, 4 * f, v, -v)
    return bytes(out)


# ----------------------------------------------------------------------------
# Track description (shared with compare.py --manifest)
# ----------------------------------------------------------------------------
# A manifest track has: number, type ('data'|'audio'), form ('gd_mode1_2352' | 'cdi_xa_form1_2336' |
# 'cdi_mode1_2352' | 'audio_2352'), start_lba, sector_count, sector_size, file, file_offset, img, ip_product (or null).


def _user(track, lba):
    n = lba - track['start_lba']
    return user_data(track['img'], track['number'], lba, track['ip_product'] if n == 0 else None)


def sector_for(track, lba, want):
    """Bytes a drive should return for `lba` of `track` with sector size `want`: (data, None) or (None, reason)."""
    n = lba - track['start_lba']
    if n < 0 or n >= track['sector_count']:
        return None, 'outside track %d' % track['number']
    form = track['form']
    if form == 'audio_2352':
        return (audio_sector(lba), None) if want == 2352 else (None, 'audio track needs ssz 2352')
    user = _user(track, lba)
    if want == 2048:
        return user, None
    if form == 'gd_mode1_2352' and want == 2352:
        return raw_mode1(lba, user), None
    if form == 'cdi_mode1_2352' and want == 2352:
        return raw_mode1(lba, user), None
    if form == 'cdi_xa_form1_2336':
        if want == 2336:
            return stored_form1(user), None
        if want == 2352:
            return raw_xa_form1(lba, user), None
    return None, 'unsupported ssz %d for %s' % (want, form)


def stored_sector(track, lba):
    """The sector as stored in the track's file."""
    form = track['form']
    if form == 'audio_2352':
        return audio_sector(lba)
    user = _user(track, lba)
    return raw_mode1(lba, user) if form in ('gd_mode1_2352', 'cdi_mode1_2352') else stored_form1(user)


def sector_crc(data):
    """CRC-32 as zlib.crc32 / gdprobe hash.c (poly 0xEDB88320, init and final xor 0xFFFFFFFF)."""
    return zlib.crc32(data) & 0xFFFFFFFF


# ----------------------------------------------------------------------------
# GDI
# ----------------------------------------------------------------------------

def build_gdi(folder, img, spec):
    """spec: list of (type, start_lba, count, ip_product|None). Returns the manifest entry."""
    os.makedirs(folder, exist_ok=True)
    tracks, lines, files = [], [str(len(spec))], []
    for i, (typ, start, count, ip) in enumerate(spec, 1):
        fname = 'track%02d.%s' % (i, 'bin' if typ == 'data' else 'raw')
        t = {'number': i, 'type': typ, 'form': 'gd_mode1_2352' if typ == 'data' else 'audio_2352',
             'start_lba': start, 'sector_count': count, 'sector_size': 2352, 'file': fname, 'file_offset': 0,
             'img': img, 'ip_product': ip, 'ctrl': 4 if typ == 'data' else 0, 'parsed_sector_count': count}
        crc = 0
        with open(os.path.join(folder, fname), 'wb') as f:
            for n in range(count):
                s = stored_sector(t, start + n)
                crc = zlib.crc32(s, crc)
                f.write(s)
        t['crc32'] = '%08x' % (crc & 0xFFFFFFFF)
        t['bytes'] = count * 2352
        tracks.append(t)
        lines.append('%d %d %d 2352 %s 0' % (i, start, t['ctrl'], fname))
        files.append(fname)
    text = '\n'.join(lines) + '\n'
    with open(os.path.join(folder, 'disc.gdi'), 'wb') as f:
        f.write(text.encode('ascii'))
    return {'folder': '%02d' % img, 'format': 'gdi', 'file': 'disc.gdi', 'files': ['disc.gdi'] + files,
            'tracks': tracks, 'gdi_text': text}


def gdi_spec_03(small):
    t1, a, d = (150, 150, 1100) if small else (600, 600, 20000)
    return [('data', 0, t1, None), ('audio', 756, a, None), ('data', 45000, d, 'NETGD PROBE 03')]


def gdi_spec_06(small):
    t1, a, d = (150, 150, 1100) if small else (600, 600, 20000)
    s = [('data', 0, t1, None)]
    lba = 756
    for _ in range(3):
        s.append(('audio', lba, a, None))
        lba += a + 150
    s.append(('data', 45000, d, 'NETGD PROBE 06'))
    return s


# Variant set (--set variants, LOG R121): why does the stock firmware refuse 06?
# 08 = normal low-density area (data + 1 audio) and three tracks in the high-density
#      area (data, audio, data), 5 tracks in all: does the total track count matter?
# 09 = two audio tracks in the low-density area, so the high-density area starts at
#      track 4 (like 06, but 4 tracks in all): does a high-density start other than
#      track 3 matter?
def gdi_spec_08(small):
    t1, a, d = (150, 150, 550) if small else (600, 600, 10000)
    d2 = 45000 + d + 150 + a + 150
    return [('data', 0, t1, None), ('audio', 756, a, None),
            ('data', 45000, d, 'NETGD PROBE 08'), ('audio', 45000 + d + 150, a, None),
            ('data', d2, d, None)]


def gdi_spec_09(small):
    t1, a, d = (150, 150, 1100) if small else (600, 600, 20000)
    return [('data', 0, t1, None), ('audio', 756, a, None), ('audio', 756 + a + 150, a, None),
            ('data', 45000, d, 'NETGD PROBE 09')]


# ----------------------------------------------------------------------------
# CDI (DiscJuggler), after mkdcdisc src/disc_image/formats/cdi.c
# ----------------------------------------------------------------------------
LEAD_IN, LEAD_OUT_FIRST, LEAD_OUT_NEXT = 4500, 6750, 2250
PREGAP, DATA_POSTGAP, CDDA_MIN = 150, 2, 302


def build_cdi(folder, img, sessions, volname='disc.cdi'):
    """sessions: list of sessions, each a list of (type, count, ip_product|None).
    type 'data' = Mode 2 Form 1 (2336 bytes stored, mkdcdisc), 'data1' = Mode 1 (2352 raw bytes stored,
    DiscJuggler read mode 2). Audio counts are raised to 302 (mkdcdisc's floor). Returns the manifest entry."""
    os.makedirs(folder, exist_ok=True)
    path = os.path.join(folder, 'disc.cdi')
    tracks, sess_lens, layout = [], [], []
    number = 0
    base = 0            # file offset of the next track's stored data (its pregap first)
    session_start = 0
    for si, sess in enumerate(sessions):
        pos = session_start
        sl = LEAD_IN + (LEAD_OUT_FIRST if si == 0 else LEAD_OUT_NEXT)
        row = []
        for (typ, count, ip) in sess:
            number += 1
            if typ == 'audio':
                count, post = max(count, CDDA_MIN), 0
            else:
                post = DATA_POSTGAP
            form = {'audio': 'audio_2352', 'data': 'cdi_xa_form1_2336', 'data1': 'cdi_mode1_2352'}[typ]
            ssz = 2336 if form == 'cdi_xa_form1_2336' else 2352
            t = {'number': number, 'type': 'audio' if typ == 'audio' else 'data', 'session': si + 1,
                 'form': form,
                 'start_lba': pos, 'sector_count': count, 'pregap': PREGAP, 'postgap': post,
                 'sector_size': ssz, 'file': 'disc.cdi',
                 'img': img, 'ip_product': ip, 'ctrl': 0 if typ == 'audio' else 4,
                 'parsed_sector_count': count + post}
            t['file_offset'] = base + PREGAP * t['sector_size']
            base += (PREGAP + count + post) * t['sector_size']
            pos += PREGAP + count + post
            sl += PREGAP + count + post
            row.append(t)
            tracks.append(t)
        layout.append(row)
        sess_lens.append(sl)
        session_start += sl
    with open(path, 'wb') as f:
        for t in tracks:
            gap = FORM2_GAP if t['form'] == 'cdi_xa_form1_2336' else bytes(2352)   # audio / Mode 1 gaps: zero sectors
            f.write(gap * PREGAP)
            crc = 0
            for n in range(t['sector_count']):
                s = stored_sector(t, t['start_lba'] + n)
                crc = zlib.crc32(s, crc)
                f.write(s)
            t['crc32'] = '%08x' % (crc & 0xFFFFFFFF)
            t['bytes'] = t['sector_count'] * t['sector_size']
            if t['postgap']:
                f.write(gap * t['postgap'])
        data_end = f.tell()
        name = volname.encode('ascii')[:32].ljust(32, b' ')
        marker = bytes((0, 0, 1, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF))

        def track_header():
            return (marker + marker + bytes((0xAB, 0, 0x10, len(tracks), 32)) + name + bytes(11) +
                    struct.pack('<IIIII', 2, 0, 0x80000000, 360000, 0x980000))
        d = bytearray(struct.pack('<B', len(sessions)))
        for si, row in enumerate(layout):
            d += struct.pack('<BHI', 0, len(row), 0)
            for ti, t in enumerate(row):
                d += track_header()
                last = ti == len(row) - 1
                # DiscJuggler: track mode 0 audio / 1 Mode 1 / 2 Mode 2; read mode 0 = 2048, 1 = 2336, 2 = 2352 bytes stored
                track_mode = {'audio_2352': 0, 'cdi_mode1_2352': 1, 'cdi_xa_form1_2336': 2}[t['form']]
                read_mode = {'audio_2352': 2, 'cdi_mode1_2352': 2, 'cdi_xa_form1_2336': 1}[t['form']]
                tl = t['sector_count'] + t['postgap']
                td = bytearray()
                td += struct.pack('<H', 2)
                td += struct.pack('<II', t['pregap'], tl)
                td += bytes(6)
                td += struct.pack('<IIIII', track_mode, 0, si, ti, t['start_lba'])
                td += struct.pack('<I', tl + t['pregap'])
                td += bytes(16)
                td += struct.pack('<II', read_mode, t['ctrl'])
                td += bytes(1)
                td += struct.pack('<II', tl + t['pregap'], 0)
                td += bytes(12)
                td += struct.pack('<I', 0)
                td += bytes(1)
                td += b'\xff' * 8
                td += struct.pack('<IIII', 1, 0x80, 2, 0x10)
                td += struct.pack('<I', 44100)
                td += bytes(42)
                td += struct.pack('<I', 0xFFFFFFFF)
                td += bytes(12)
                td += struct.pack('<B', track_mode if last else 0)
                td += bytes(5)
                td += struct.pack('<BB', 0 if last else 1, 0)
                td += struct.pack('<I', t['start_lba'])
                assert len(td) == 188, len(td)
                d += td if last else td[:-8]
        d += struct.pack('<BHI', 0, 0, 0)
        d += track_header()
        d += struct.pack('<I', sum(sess_lens))
        d += bytes((32,)) + name
        d += bytes(1) + struct.pack('<II', 1, 1) + bytes(13) + struct.pack('<I', 0)
        d += bytes(1) + struct.pack('<II', 0, 0) + bytes(3) + struct.pack('<I', 0x80000006)
        d += struct.pack('<I', len(d) + 4)
        f.write(d)
    return {'folder': '%02d' % img, 'format': 'cdi', 'file': 'disc.cdi', 'files': ['disc.cdi'],
            'sessions': len(sessions), 'session_lengths': sess_lens, 'data_bytes': data_end,
            'descriptor_bytes': len(d), 'tracks': tracks}


def cdi_spec_04(small):
    return [[('audio', CDDA_MIN, None)], [('data', 1100 if small else 4000, 'NETGD PROBE 04')]]


def cdi_spec_10(small):
    """Same layout as 04 with a Mode 1 data track (R137 g): does stock take CD_READ byte 1 = 0x24 (Mode 1) here,
    and refuse 0x28 (Mode 2 Form 1)?"""
    return [[('audio', CDDA_MIN, None)], [('data1', 1100 if small else 4000, 'NETGD PROBE 10')]]


def cdi_spec_05(small):
    return [[('audio', CDDA_MIN, None), ('audio', CDDA_MIN + 100, None)],
            [('data', 1100 if small else 4000, 'NETGD PROBE 05')]]


# ----------------------------------------------------------------------------
# SD card
# ----------------------------------------------------------------------------
README_01 = """This folder (01) holds the menu (GDmenu or openMenu) of your normal SD card.
Copy the whole 01 folder from your normal card here (or run mksdcard.py with --menu PATH).
"""
README_02 = """This folder (02) must hold the dcload-ip disc as disc.cdi (fixed-IP build).
Build it on the DreamSDK PC with the Dreamcast-Dev-Tools-for-BBA script
scripts\\make_dcload_disc.ps1 and copy the result here as disc.cdi
(or run mksdcard.py with --dcload PATH_TO_CDI).
"""


def crc_file(path):
    c = 0
    with open(path, 'rb') as f:
        while True:
            b = f.read(1 << 20)
            if not b:
                break
            c = zlib.crc32(b, c)
    return '%08x' % (c & 0xFFFFFFFF)


def make_variants(outdir, small=False, force=False):
    """--set variants: only the extra images 08, 09 (GDI) and 10 (CDI, Mode 1) (copied next to an existing card's 01-07)."""
    if os.path.exists(outdir) and os.listdir(outdir) and not force:
        raise SystemExit('error: %s is not empty (use --force to write into it)' % outdir)
    os.makedirs(outdir, exist_ok=True)
    manifest = {'tool': 'mksdcard', 'version': VERSION, 'small': bool(small), 'set': 'variants',
                'folders': {}, 'images': {}}
    for n, kind, spec in ((8, 'gdi', gdi_spec_08), (9, 'gdi', gdi_spec_09), (10, 'cdi', cdi_spec_10)):
        folder = os.path.join(outdir, '%02d' % n)
        e = build_gdi(folder, n, spec(small)) if kind == 'gdi' else build_cdi(folder, n, spec(small))
        e['file_crc32'] = {fn: crc_file(os.path.join(folder, fn)) for fn in e['files']}
        manifest['images']['%02d' % n] = e
    with open(os.path.join(outdir, MANIFEST_NAME), 'w') as f:
        json.dump(manifest, f, indent=1, sort_keys=True)
        f.write('\n')
    return manifest


def make_card(outdir, menu=None, dcload=None, small=False, force=False):
    if os.path.exists(outdir) and os.listdir(outdir) and not force:
        raise SystemExit('error: %s is not empty (use --force to write into it)' % outdir)
    os.makedirs(outdir, exist_ok=True)
    manifest = {
        'tool': 'mksdcard', 'version': VERSION, 'small': bool(small),
        'sector_crc': {
            'algorithm': 'CRC-32, zlib.crc32 (poly 0xEDB88320, init/xorout 0xFFFFFFFF), 8 lowercase hex digits',
            'fad_to_lba': 'lba = fad - 150',
            'ssz_2048': 'the 2048 user-data bytes of a data sector',
            'ssz_2352': 'the whole raw sector (GDI data: Mode 1 raw; CDI data: sync + header(mode 2) + 2336 stored bytes; audio: PCM)',
            'audio_note': 'audio sectors are 2352 bytes of PCM; the first stereo frame is (lba & 0xFFFF, lba >> 16)',
        },
        'folders': {},
        'images': {},
    }
    d01 = os.path.join(outdir, '01')
    if menu:
        if os.path.exists(d01):
            shutil.rmtree(d01)
        shutil.copytree(menu, d01)
        manifest['folders']['01'] = {'content': 'menu copied from ' + os.path.basename(os.path.abspath(menu))}
    else:
        os.makedirs(d01, exist_ok=True)
        with open(os.path.join(d01, 'README.txt'), 'w') as f:
            f.write(README_01)
        manifest['folders']['01'] = {'content': 'placeholder README.txt (copy the menu folder here)'}
    d02 = os.path.join(outdir, '02')
    os.makedirs(d02, exist_ok=True)
    if dcload:
        shutil.copyfile(dcload, os.path.join(d02, 'disc.cdi'))
        manifest['folders']['02'] = {'content': 'dcload-ip disc.cdi', 'crc32': crc_file(os.path.join(d02, 'disc.cdi'))}
    else:
        with open(os.path.join(d02, 'README.txt'), 'w') as f:
            f.write(README_02)
        manifest['folders']['02'] = {'content': 'placeholder README.txt (put dcload-ip as disc.cdi)'}
    plan = ((3, 'gdi', gdi_spec_03), (4, 'cdi', cdi_spec_04), (5, 'cdi', cdi_spec_05), (6, 'gdi', gdi_spec_06))
    for n, kind, spec in plan:
        folder = os.path.join(outdir, '%02d' % n)
        e = build_gdi(folder, n, spec(small)) if kind == 'gdi' else build_cdi(folder, n, spec(small))
        e['file_crc32'] = {fn: crc_file(os.path.join(folder, fn)) for fn in e['files']}
        manifest['images']['%02d' % n] = e
    os.makedirs(os.path.join(outdir, '07'), exist_ok=True)   # empty on purpose; FAT copy tools may drop it
    manifest['folders']['07'] = {'content': 'empty folder (create it by hand on the card if your copy tool dropped it)'}
    with open(os.path.join(outdir, MANIFEST_NAME), 'w') as f:
        json.dump(manifest, f, indent=1, sort_keys=True)
        f.write('\n')
    return manifest


def main(argv=None):
    ap = argparse.ArgumentParser(description='Build the NetGD/GDEMU probe SD card contents')
    ap.add_argument('outdir')
    ap.add_argument('--menu', help='existing 01 folder (GDmenu/openMenu) to copy as OUTDIR/01')
    ap.add_argument('--dcload', help='dcload-ip CDI to copy as OUTDIR/02/disc.cdi')
    ap.add_argument('--small', action='store_true', help='a few MB instead of ~100 MB (for tests)')
    ap.add_argument('--force', action='store_true', help='write into a non-empty OUTDIR')
    ap.add_argument('--set', choices=('card', 'variants'), default='card',
                    help='card = the full 01-07 card (default); variants = only the extra images 08, 09 (GDI) and 10 (CDI, Mode 1)')
    a = ap.parse_args(argv)
    if a.set == 'variants':
        m = make_variants(a.outdir, a.small, a.force)
    else:
        m = make_card(a.outdir, a.menu, a.dcload, a.small, a.force)
    total = 0
    for n, e in sorted(m['images'].items()):
        sz = sum(os.path.getsize(os.path.join(a.outdir, n, fn)) for fn in e['files'])
        total += sz
        print('%s  %-3s %8.1f MB  %d tracks' % (n, e['format'], sz / 1e6, len(e['tracks'])))
    print('total generated: %.1f MB; manifest: %s' % (total / 1e6, os.path.join(a.outdir, MANIFEST_NAME)))
    return 0


if __name__ == '__main__':
    sys.exit(main())
