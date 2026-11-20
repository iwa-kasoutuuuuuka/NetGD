#!/usr/bin/env python3
"""Tests for compare.py with small synthetic records and a tiny synthetic GDI."""
import contextlib
import io
import os
import sys
import tempfile
import unittest
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, '..'))
import compare  # noqa: E402


def run(argv):
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        try:
            rc = compare.main(argv)
        except SystemExit as e:
            rc = e.code
    return rc, out.getvalue(), err.getvalue()


REC_A = """# gdprobe version=1 label=stock
img=0 probe=TUR cdb=000000000000000000000000 rc=ok status=0x50 err=0x00 irr=0x03 bytes=0 us=1500
img=3 probe=REQ_SES_2 cdb=150002000600000000000000 rc=ok status=0x50 err=0x00 irr=0x03 bytes=6 data=010003 00b05e us=1000
img=3 probe=REQ_SES_1 cdb=150001000600000000000000 rc=ok status=0x50 err=0x00 irr=0x03 bytes=6 data=010001000096 us=1000
img=3 probe=IP_READ fad=45150 count=7 cdb=302400b05e00000000070000 rc=ok status=0x50 err=0x00 irr=0x03 bytes=14336 us=20000
img=3 probe=HASH fad=45150 ssz=2048 crc=11111111
img=3 probe=HASH fad=45151 ssz=2048 crc=22222222
img=3 probe=SW_RESULT n=3 rc=ok t_ms=800 polls=40 changes=3
img=7 probe=SW_RESULT n=7 rc=timeout t_ms=15000 polls=700 changes=1
img=3 probe=TP_SUMMARY ok=4 bytes=131072 us=200000 min_us=40000 max_us=60000 kib_s=640
""".replace('010003 00b05e', '01000300b05e')

REC_B = """# gdprobe version=1 label=netgd
img=0 probe=TUR cdb=000000000000000000000000 rc=ok status=0x50 err=0x00 irr=0x03 bytes=0 us=1600
img=3 probe=REQ_SES_2 cdb=150002000600000000000000 rc=check status=0x51 err=0x50 irr=0x03 bytes=0 us=1200
img=3 probe=REQ_SES_1 cdb=150001000600000000000000 rc=ok status=0x50 err=0x00 irr=0x03 bytes=6 data=010001000097 us=9000
img=3 probe=IP_READ fad=45150 count=7 cdb=302400b05e00000000070000 rc=ok status=0x50 err=0x00 irr=0x03 bytes=14336 us=21000
img=3 probe=HASH fad=45150 ssz=2048 crc=11111111
img=3 probe=HASH fad=45151 ssz=2048 crc=33333333
img=3 probe=SW_RESULT n=3 rc=ok t_ms=1700 polls=80 changes=3
img=7 probe=SW_RESULT n=7 rc=ok t_ms=900 polls=40 changes=2
img=3 probe=TP_SUMMARY ok=4 bytes=131072 us=100000 min_us=20000 max_us=30000 kib_s=1280
"""


class CompareTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = self.tmp.name

    def tearDown(self):
        self.tmp.cleanup()

    def write(self, name, data, mode='w'):
        p = os.path.join(self.dir, name)
        with open(p, mode) as f:
            f.write(data)
        return p

    def test_parse(self):
        hdr, ents = compare.parse_record(REC_A)
        self.assertEqual(hdr, ['gdprobe version=1 label=stock'])
        self.assertEqual(len(ents), 9)
        self.assertEqual(ents[1]['probe'], 'REQ_SES_2')
        self.assertEqual(ents[1]['img'], 3)
        self.assertEqual(ents[1]['data'], '01000300b05e')

    def test_identical_records_have_no_differences(self):
        a = self.write('a.txt', REC_A)
        rc, out, _ = run([a, a])
        self.assertEqual(rc, 0, out)
        self.assertIn('0 difference(s), 0 timing note(s)', out)

    def test_differences_reported(self):
        a, b = self.write('a.txt', REC_A), self.write('b.txt', REC_B)
        rc, out, _ = run([a, b])
        self.assertEqual(rc, 1)
        self.assertIn('img=3 REQ_SES_2: rc ok vs check', out)
        self.assertIn('status 0x50 vs 0x51', out)
        self.assertIn('bytes 6 vs 0', out)
        self.assertIn('data differs', out)
        self.assertIn('img=3 REQ_SES_1: data differs (+5: 96!=97)', out)
        self.assertIn('img=3 HASH fad=45151: crc 22222222(ssz 2048) vs 33333333(ssz 2048)', out)
        self.assertNotIn('HASH fad=45150', out)
        self.assertIn('img=7 SW_RESULT: rc timeout vs ok', out)
        # timings: only notes, never differences
        self.assertIn('TIMING', out)
        self.assertIn('img=3 REQ_SES_1: us 1000 -> 9000 (x9.00)', out)
        self.assertIn('img=3 SW_RESULT: t_ms 800 -> 1700', out)
        self.assertIn('kib_s 640 -> 1280', out)
        self.assertNotIn('img=0 TUR: us', out)     # 1500 vs 1600: noise
        self.assertNotIn('IP_READ: us', out)       # 20000 vs 21000: within the ratio

    def test_poll_counts_are_not_differences(self):
        a, b = self.write('a.txt', REC_A), self.write('b.txt', REC_B)
        _, out, _ = run([a, b])
        self.assertNotIn('polls ', out)
        self.assertNotIn('changes ', out)

    def test_only_in_one_side_and_ignore_timing(self):
        a = self.write('a.txt', REC_A)
        b = self.write('b.txt', REC_A.replace('img=0 probe=TUR', 'img=0 probe=TUR2').replace('us=1000', 'us=99999'))
        rc, out, _ = run([a, b, '--ignore-timing'])
        self.assertEqual(rc, 1)
        self.assertIn('img=0 TUR: only in A', out)
        self.assertIn('img=0 TUR2: only in B', out)
        self.assertNotIn('TIMING', out)

    def test_repeated_probe_names_are_matched_in_order(self):
        text = ('img=1 probe=SW_POLL t_ms=1 poll=1 rc=check status=0x51 err=0x20 sense=2/04/01\n'
                'img=1 probe=SW_POLL t_ms=300 poll=14 rc=ok status=0x50 err=0x00 sense=none\n')
        a = self.write('a.txt', text)
        b = self.write('b.txt', text.replace('300 poll=14', '350 poll=17'))
        rc, out, _ = run([a, b])
        self.assertEqual(rc, 0, out)
        c = self.write('c.txt', text.replace('sense=2/04/01', 'sense=2/3a/00'))
        rc, out, _ = run([a, c])
        self.assertEqual(rc, 1)
        self.assertIn('sense 2/04/01 vs 2/3a/00', out)

    def test_missing_file(self):
        rc, _, err = run([os.path.join(self.dir, 'nope.txt'), os.path.join(self.dir, 'nope2.txt')])
        self.assertEqual(rc, 2)
        self.assertIn('error', err)


class ImageTests(unittest.TestCase):
    """A tiny GDI: track 1 (2048-byte data, LBA 0), track 3 (2352-byte raw Mode 1, LBA 45000)."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        d = self.dir = self.tmp.name
        self.t1 = bytes((i * 3 + 1) & 0xFF for i in range(2048 * 4))
        raw = bytearray()
        self.user = []
        for s in range(10):
            user = bytes((s * 17 + i) & 0xFF for i in range(2048))
            self.user.append(user)
            sector = b'\x00' + b'\xff' * 10 + b'\x00' + bytes([0, 2, s]) + b'\x01' + user + b'\x00' * 288
            assert len(sector) == 2352
            raw += sector
        self.t3 = bytes(raw)
        with open(os.path.join(d, 'track01.bin'), 'wb') as f:
            f.write(self.t1)
        with open(os.path.join(d, 'track 03.bin'), 'wb') as f:
            f.write(self.t3)
        with open(os.path.join(d, 'disc.gdi'), 'w') as f:
            f.write('2\n1 0 4 2048 track01.bin 0\n3 45000 4 2352 "track 03.bin" 0\n')
        self.gdi = os.path.join(d, 'disc.gdi')

    def tearDown(self):
        self.tmp.cleanup()

    def rec(self, lines):
        p = os.path.join(self.dir, 'rec.txt')
        with open(p, 'w') as f:
            f.write('\n'.join(lines) + '\n')
        return p

    @staticmethod
    def hl(img, fad, ssz, data):
        return 'img=%d probe=HASH fad=%d ssz=%d crc=%08x' % (img, fad, ssz, zlib.crc32(data) & 0xFFFFFFFF)

    def test_gdi_parse(self):
        tr = compare.parse_gdi(self.gdi)
        self.assertEqual([t.num for t in tr], [1, 3])
        self.assertEqual(tr[1].path, os.path.join(self.dir, 'track 03.bin'))
        self.assertEqual(tr[0].count, 4)
        self.assertEqual(tr[1].count, 10)

    def test_all_match(self):
        lines = [self.hl(3, 45150 + s, 2048, self.user[s]) for s in range(10)]
        lines.append(self.hl(3, 150 + 2, 2048, self.t1[2 * 2048:3 * 2048]))
        lines.append(self.hl(3, 45150 + 4, 2352, self.t3[4 * 2352:5 * 2352]))   # whole raw sector
        rc, out, _ = run(['--image', self.gdi, self.rec(lines)])
        self.assertEqual(rc, 0, out)
        self.assertIn('hashes: 12 match, 0 mismatch, 0 not checked', out)

    def test_mismatch_and_unchecked(self):
        bad = bytearray(self.user[3])
        bad[100] ^= 1
        lines = [self.hl(3, 45150 + 3, 2048, bytes(bad)),
                 self.hl(3, 45150 + 5, 2048, self.user[5]),
                 self.hl(3, 45150 + 50, 2048, b'x'),      # past the end of the track
                 self.hl(3, 100000, 2048, b'x')]          # no such track data
        rc, out, _ = run(['--image', self.gdi, self.rec(lines)])
        self.assertEqual(rc, 1)
        self.assertIn('fad=45153: MISMATCH', out)
        self.assertIn('hashes: 1 match, 1 mismatch, 2 not checked', out)
        self.assertIn('beyond the end of track 3', out)

    def test_img_filter(self):
        lines = [self.hl(3, 45150, 2048, self.user[0]), self.hl(4, 45150, 2048, b'other disc')]
        p = self.rec(lines)
        rc, out, _ = run(['--image', self.gdi, p])
        self.assertEqual(rc, 1)
        self.assertIn('note: the record has hashes for images [3, 4]', out)
        rc, out, _ = run(['--image', self.gdi, '--img', '3', p])
        self.assertEqual(rc, 0, out)

    def test_raw_file(self):
        p = self.rec([self.hl(0, 45150 + 2, 2048, self.user[2])])
        raw = os.path.join(self.dir, 'track 03.bin')
        rc, out, _ = run(['--raw', raw, '--raw-start-fad', '45150', p])
        self.assertEqual(rc, 0, out)

    def test_nothing_checked(self):
        rc, out, _ = run(['--image', self.gdi, self.rec(['img=0 probe=TUR rc=ok'])])
        self.assertEqual(rc, 2)


class SampleFromCTests(unittest.TestCase):
    def test_runner_output_parses_and_self_compares(self):
        p = os.environ.get('GDPROBE_SAMPLE')
        if not p or not os.path.exists(p):
            self.skipTest('GDPROBE_SAMPLE (record dumped by test_all) not set')
        rc, out, _ = run([p, p])
        self.assertEqual(rc, 0, out)
        _, ents = compare.parse_record(compare.read_text(p))
        self.assertTrue(any(e['probe'] == 'SW_RESULT' for e in ents))
        self.assertTrue(any(e['probe'] == 'HASH' for e in ents))


if __name__ == '__main__':
    unittest.main(verbosity=2)
