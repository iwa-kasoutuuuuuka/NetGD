#!/usr/bin/env python3
"""Tests for mksdcard (python tests/test_mksdcard.py, or python -m unittest discover tests)."""
import contextlib
import io
import json
import os
import struct
import sys
import tempfile
import unittest
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, '..'))
import mksdcard as m  # noqa: E402


def run(argv):
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        try:
            rc = m.main(argv)
        except SystemExit as e:
            rc = e.code
    return rc, out.getvalue()


# --- straightforward (slow, unvectorised) reference of the ECC, written from ECMA-130 annex A ---
def ref_ecc_block(src, major_count, minor_count, major_mult, minor_inc):
    f = [((i << 1) ^ (0x11D if i & 0x80 else 0)) & 0xFF for i in range(256)]
    b = [0] * 256
    for i in range(256):
        b[i ^ f[i]] = i
    size = major_count * minor_count
    dest = [0] * (2 * major_count)
    for major in range(major_count):
        index = (major >> 1) * major_mult + (major & 1)
        a = bb = 0
        for _ in range(minor_count):
            t = src[index]
            index += minor_inc
            if index >= size:
                index -= size
            a ^= t
            bb ^= t
            bb = f[bb]
        a = b[f[a] ^ bb]
        dest[major] = a
        dest[major + major_count] = a ^ bb
    return bytes(dest)


def ref_edc_bitwise(data):
    crc = 0
    for byte in data:
        crc ^= byte
        for _ in range(8):
            crc = (crc >> 1) ^ (0xD8018001 if crc & 1 else 0)
    return crc


class EdcEccTests(unittest.TestCase):
    def test_edc_known_vector_mkdcdisc_form2_gap(self):
        # mkdcdisc write_mode2_pregap: subheader 00 00 20 00 00 00 20 00, 2324 zero bytes, EDC 3F 13 B0 BE
        data = bytes((0, 0, 0x20, 0, 0, 0, 0x20, 0)) + bytes(2324)
        self.assertEqual(m.edc(data), 0xBEB0133F)
        self.assertEqual(struct.pack('<I', m.edc(data)), bytes((0x3F, 0x13, 0xB0, 0xBE)))
        self.assertEqual(m.FORM2_GAP[-4:], bytes((0x3F, 0x13, 0xB0, 0xBE)))

    def test_edc_matches_bitwise_reference(self):
        d = m.user_data(3, 3, 45001)
        self.assertEqual(m.edc(d), ref_edc_bitwise(d))

    def test_ecc_matches_reference(self):
        import random
        rnd = random.Random(1)
        for _ in range(3):
            sec = bytearray(rnd.randrange(256) for _ in range(2352))
            sec[12:16] = bytes(4)
            p = ref_ecc_block(bytes(sec[12:]), 86, 24, 2, 86)
            self.assertEqual(m.ecc_p(bytes(sec[12:])), p)
            sec[0x81C:0x81C + 172] = p
            q = ref_ecc_block(bytes(sec[12:]), 52, 43, 86, 88)
            self.assertEqual(m.ecc_q(bytes(sec[12:])), q)

    def test_zero_sector_values(self):
        z = bytes(2048)
        s = m.raw_mode1(-150, z)          # address 00:00:00
        self.assertEqual(s[:12], m.SYNC)
        self.assertEqual(s[12:16], bytes((0, 0, 0, 1)))
        # data zero, header 00:00:00 mode 1: only the sync pattern feeds the EDC
        self.assertEqual(struct.unpack('<I', s[2064:2068])[0], m.edc(s[:2064]))
        self.assertEqual(s[2068:2076], bytes(8))
        # the zero stored Form 1 sector has non-zero EDC (subheader) but its P/Q follow from it
        f = m.stored_form1(z)
        self.assertEqual(f[:8], m.FORM1_SUBHEADER)
        # an all-zero input gives zero parity
        self.assertEqual(m.ecc_p(bytes(2340)), bytes(172))
        self.assertEqual(m.ecc_q(bytes(2340)), bytes(104))

    def test_ecc_is_linear(self):
        a = bytes((i * 7) & 0xFF for i in range(2340))
        b = bytes((i * 13 + 5) & 0xFF for i in range(2340))
        x = bytes(p ^ q for p, q in zip(a, b))
        self.assertEqual(m.ecc_p(x), bytes(p ^ q for p, q in zip(m.ecc_p(a), m.ecc_p(b))))
        self.assertEqual(m.ecc_q(x), bytes(p ^ q for p, q in zip(m.ecc_q(a), m.ecc_q(b))))


class SectorTests(unittest.TestCase):
    def test_mode1_layout(self):
        u = m.user_data(3, 3, 45000, 'NETGD PROBE 03')
        s = m.raw_mode1(45000, u)
        self.assertEqual(len(s), 2352)
        self.assertEqual(s[0:12], b'\x00' + b'\xff' * 10 + b'\x00')
        self.assertEqual(s[12:16], bytes((0x10, 0x02, 0x00, 0x01)))      # FAD 45150 = 10:02:00
        self.assertEqual(s[16:2064], u)
        self.assertEqual(struct.unpack('<I', s[2064:2068])[0], m.edc(s[:2064]))
        self.assertEqual(s[2068:2076], bytes(8))
        sec = bytearray(s)
        self.assertEqual(m.ecc_p(bytes(sec[12:])), s[0x81C:0x81C + 172])
        self.assertEqual(m.ecc_q(bytes(sec[12:])), s[0x8C8:0x8C8 + 104])

    def test_lba_zero_header(self):
        self.assertEqual(m.msf_bytes(0), bytes((0, 2, 0)))
        self.assertEqual(m.msf_bytes(11702), bytes((0x02, 0x38, 0x02)))     # FAD 11852

    def test_form1_stored(self):
        u = m.user_data(4, 2, 11702, 'NETGD PROBE 04')
        s = m.stored_form1(u)
        self.assertEqual(len(s), 2336)
        self.assertEqual(s[:8], m.FORM1_SUBHEADER)
        self.assertEqual(s[8:2056], u)
        self.assertEqual(struct.unpack('<I', s[2056:2060])[0], m.edc(s[:2056]))
        raw = m.raw_xa_form1(11702, u)
        self.assertEqual(len(raw), 2352)
        self.assertEqual(raw[15], 2)

    def test_user_data_tag_and_ip_bin(self):
        u = m.user_data(3, 3, 45001)
        self.assertTrue(u.startswith(b'NETGD-PROBE img=03 trk=3 lba=00045001 '))
        self.assertEqual(len(u), 2048)
        ip = m.user_data(3, 3, 45000, 'NETGD PROBE 03')
        self.assertEqual(ip[:16], b'SEGA SEGAKATANA ')
        self.assertEqual(ip[16:32], b'SEGA ENTERPRISES')
        self.assertIn(b'NETGD PROBE 03', ip[:256])
        self.assertEqual(ip[256:256 + 12], b'NETGD-PROBE ')
        self.assertEqual(len(ip), 2048)
        self.assertNotEqual(m.user_data(3, 3, 45001), m.user_data(3, 3, 45002))

    def test_audio_sector(self):
        a = m.audio_sector(0x12345)
        self.assertEqual(len(a), 2352)
        self.assertEqual(struct.unpack('<HH', a[:4]), (0x2345, 0x1))
        self.assertEqual(a, m.audio_sector(0x12345))


class CardTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        cls.d1 = os.path.join(cls.tmp.name, 'a')
        cls.d2 = os.path.join(cls.tmp.name, 'b')
        rc, _ = run([cls.d1, '--small'])
        assert rc == 0
        rc, _ = run([cls.d2, '--small'])
        assert rc == 0
        with open(os.path.join(cls.d1, m.MANIFEST_NAME)) as f:
            cls.man = json.load(f)

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def files(self, d):
        out = {}
        for root, _, names in os.walk(d):
            for n in names:
                p = os.path.join(root, n)
                with open(p, 'rb') as f:
                    out[os.path.relpath(p, d)] = f.read()
        return out

    def test_layout_and_empty_07(self):
        for n in ('01', '02', '03', '04', '05', '06', '07'):
            self.assertTrue(os.path.isdir(os.path.join(self.d1, n)), n)
        self.assertEqual(os.listdir(os.path.join(self.d1, '07')), [])
        self.assertTrue(os.path.exists(os.path.join(self.d1, '01', 'README.txt')))
        self.assertTrue(os.path.exists(os.path.join(self.d1, '02', 'README.txt')))
        self.assertTrue(os.path.exists(os.path.join(self.d1, '03', 'disc.gdi')))
        self.assertTrue(os.path.exists(os.path.join(self.d1, '04', 'disc.cdi')))

    def test_deterministic(self):
        self.assertEqual(self.files(self.d1), self.files(self.d2))

    def test_manifest_consistency(self):
        for n, e in self.man['images'].items():
            for t in e['tracks']:
                path = os.path.join(self.d1, n, t['file'])
                with open(path, 'rb') as f:
                    f.seek(t['file_offset'])
                    data = f.read(t['sector_count'] * t['sector_size'])
                self.assertEqual(len(data), t['bytes'])
                self.assertEqual('%08x' % (zlib.crc32(data) & 0xFFFFFFFF), t['crc32'], (n, t['number']))
                # every sector equals what the shared generator says
                for i in (0, 1, t['sector_count'] - 1):
                    lba = t['start_lba'] + i
                    want = data[i * t['sector_size']:(i + 1) * t['sector_size']]
                    self.assertEqual(m.stored_sector(t, lba), want, (n, t['number'], i))
                    if t['type'] == 'data':
                        u, why = m.sector_for(t, lba, 2048)
                        self.assertIsNone(why)
                        self.assertEqual(len(u), 2048)
            for fn, crc in e['file_crc32'].items():
                self.assertEqual(crc, m.crc_file(os.path.join(self.d1, n, fn)))

    def test_gdi_files(self):
        txt = open(os.path.join(self.d1, '03', 'disc.gdi')).read().split('\n')
        self.assertEqual(txt[0], '3')
        self.assertEqual(txt[1], '1 0 4 2352 track01.bin 0')
        self.assertEqual(txt[2], '2 756 0 2352 track02.raw 0')
        self.assertEqual(txt[3], '3 45000 4 2352 track03.bin 0')
        t3 = self.man['images']['03']['tracks'][2]
        self.assertEqual(t3['start_lba'], 45000)
        self.assertEqual(os.path.getsize(os.path.join(self.d1, '03', 'track03.bin')), t3['sector_count'] * 2352)

    def test_cdi_structure(self):
        for n, tracks, sessions in (('04', 2, 2), ('05', 3, 2)):
            e = self.man['images'][n]
            data = open(os.path.join(self.d1, n, 'disc.cdi'), 'rb').read()
            dlen = struct.unpack('<I', data[-4:])[0]
            self.assertEqual(data[len(data) - dlen], sessions)
            self.assertEqual(len(e['tracks']), tracks)
            self.assertEqual(e['data_bytes'] + dlen, len(data))
            # CDI v3.5 version word sits just before the length word
            self.assertEqual(struct.unpack('<I', data[-8:-4])[0], 0x80000006)
        # classic MIL-CD: data track at LBA 11702 after one 302-sector audio track
        self.assertEqual(self.man['images']['04']['tracks'][1]['start_lba'], 11702)
        self.assertEqual(self.man['images']['04']['tracks'][1]['pregap'], 150)

    def test_refuses_non_empty_outdir(self):
        rc, _ = run([self.d1, '--small'])
        self.assertNotEqual(rc, 0)
        rc, _ = run([self.d1, '--small', '--force'])
        self.assertEqual(rc, 0)
        self.assertEqual(self.files(self.d1), self.files(self.d2))

    def test_menu_and_dcload(self):
        with tempfile.TemporaryDirectory() as t:
            menu = os.path.join(t, 'menu')
            os.makedirs(os.path.join(menu, 'sub'))
            open(os.path.join(menu, 'disc.gdi'), 'w').write('x')
            open(os.path.join(menu, 'sub', 'f'), 'w').write('y')
            dl = os.path.join(t, 'dc.cdi')
            open(dl, 'wb').write(b'dcload')
            out = os.path.join(t, 'out')
            rc, _ = run([out, '--small', '--menu', menu, '--dcload', dl])
            self.assertEqual(rc, 0)
            self.assertTrue(os.path.exists(os.path.join(out, '01', 'sub', 'f')))
            self.assertEqual(open(os.path.join(out, '02', 'disc.cdi'), 'rb').read(), b'dcload')
            self.assertFalse(os.path.exists(os.path.join(out, '01', 'README.txt')))


class VariantTests(unittest.TestCase):
    """--set variants: 08 / 09 (GDI) and 10 (CDI with a Mode 1 data track, R137 g)."""

    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        cls.d = os.path.join(cls.tmp.name, 'v')
        rc, _ = run([cls.d, '--set', 'variants', '--small'])
        assert rc == 0
        with open(os.path.join(cls.d, m.MANIFEST_NAME)) as f:
            cls.man = json.load(f)

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def test_folders(self):
        self.assertEqual(sorted(os.listdir(self.d)), ['08', '09', '10', m.MANIFEST_NAME])
        self.assertEqual(sorted(self.man['images']), ['08', '09', '10'])

    def test_image_10_is_mode1_cdi(self):
        e = self.man['images']['10']
        self.assertEqual((e['format'], e['sessions'], len(e['tracks'])), ('cdi', 2, 2))
        a, t = e['tracks']
        self.assertEqual((a['type'], t['type'], t['form'], t['sector_size'], t['ctrl'], t['start_lba']),
                         ('audio', 'data', 'cdi_mode1_2352', 2352, 4, 11702))
        data = open(os.path.join(self.d, '10', 'disc.cdi'), 'rb').read()
        # stored data: raw Mode 1 sectors (sync, header with mode 1, user data, EDC, ECC)
        for i in (0, 1, t['sector_count'] - 1):
            off = t['file_offset'] + i * 2352
            s = data[off:off + 2352]
            self.assertEqual(s[0:12], m.SYNC)
            self.assertEqual(s[15], 1)
            self.assertEqual(s[12:15], m.msf_bytes(t['start_lba'] + i))
            self.assertEqual(s[16:2064], m.user_data(10, 2, t['start_lba'] + i, 'NETGD PROBE 10' if i == 0 else None))
            self.assertEqual(s, m.raw_mode1(t['start_lba'] + i, s[16:2064]))
            self.assertEqual(m.sector_for(t, t['start_lba'] + i, 2352)[0], s)
            self.assertEqual(m.sector_for(t, t['start_lba'] + i, 2048)[0], s[16:2064])
        self.assertIsNone(m.sector_for(t, t['start_lba'], 2336)[0])      # no 2336-byte form for Mode 1
        # descriptor: track mode 1 (Mode 1) and read mode 2 (2352 stored) for the last track, mode 0 / 2 for the audio track
        dlen = struct.unpack('<I', data[-4:])[0]
        desc = data[len(data) - dlen:]
        self.assertEqual(desc[0], 2)
        self.assertEqual(struct.unpack('<I', data[-8:-4])[0], 0x80000006)
        self.assertEqual(e['data_bytes'] + dlen, len(data))
        self.assertEqual(t['parsed_sector_count'], t['sector_count'] + 2)
        # file size: audio (150 + 302) + data (150 + n + 2 postgap) sectors of 2352, then the descriptor
        self.assertEqual(e['data_bytes'], (150 + 302 + 150 + t['sector_count'] + 2) * 2352)

    def test_descriptor_mode_fields_differ_from_image_04(self):
        # image 04 (Mode 2 Form 1, 2336) and image 10 differ in exactly the data track's mode fields
        a = m.build_cdi(os.path.join(self.tmp.name, 'x4'), 4, m.cdi_spec_04(True))
        b = m.build_cdi(os.path.join(self.tmp.name, 'x10'), 10, m.cdi_spec_10(True))
        self.assertEqual(a['tracks'][1]['sector_size'], 2336)
        self.assertEqual(b['tracks'][1]['sector_size'], 2352)
        self.assertEqual(a['tracks'][1]['start_lba'], b['tracks'][1]['start_lba'])
        da = open(os.path.join(self.tmp.name, 'x4', 'disc.cdi'), 'rb').read()
        db = open(os.path.join(self.tmp.name, 'x10', 'disc.cdi'), 'rb').read()
        ta = da[len(da) - struct.unpack('<I', da[-4:])[0]:]
        tb = db[len(db) - struct.unpack('<I', db[-4:])[0]:]
        self.assertEqual(len(ta), len(tb))
        diff = [i for i in range(len(ta)) if ta[i] != tb[i]]
        self.assertTrue(diff)
        # only mode / size / length words differ (a handful of bytes), not the structure
        self.assertLess(len(diff), 40, diff)

    def test_gdi_variants_unchanged(self):
        self.assertEqual([len(self.man['images'][n]['tracks']) for n in ('08', '09')], [5, 4])


if __name__ == '__main__':
    unittest.main()
