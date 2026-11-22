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

    def test_optional_cdbst_secnr_fields(self):
        base = 'img=3 probe=REQ_SES_2 cdb=150002000600000000000000 rc=ok status=0x50 err=0x00 irr=0x03 bytes=6 %sus=1000\n'
        old = self.write('old.txt', base % '')
        new = self.write('new.txt', base % 'cdbst=0x58 secnr=0x82 ')
        new2 = self.write('new2.txt', base % 'cdbst=0x50 secnr=0x82 ')
        _, ents = compare.parse_record(base % 'cdbst=0x58 secnr=0x82 ')
        self.assertEqual((ents[0]['cdbst'], ents[0]['secnr']), ('0x58', '0x82'))
        rc, out, _ = run([old, new])           # old record without the fields: no difference
        self.assertEqual(rc, 0, out)
        rc, out, _ = run([new, new])
        self.assertEqual(rc, 0, out)
        rc, out, _ = run([new, new2])          # both have it: a real difference
        self.assertEqual(rc, 1)
        self.assertIn('cdbst 0x58 vs 0x50', out)
        poll = 'img=3 probe=SW_POLL t_ms=1 poll=1 rc=check status=0x51 err=0x20 sense=2/04/01%s\n'
        rc, out, _ = run([self.write('p1.txt', poll % ''), self.write('p2.txt', poll % ' secnr=0x81')])
        self.assertEqual(rc, 0, out)
        rc, out, _ = run([self.write('p3.txt', poll % ' secnr=0x84'), self.write('p4.txt', poll % ' secnr=0x81')])
        self.assertEqual(rc, 1)
        self.assertIn('secnr 0x84 vs 0x81', out)

    REGPOLL = (
        'img=3 probe=SW_CMD n=3 cdb=528203000000000000000000 rc=ok status=0x50 err=0x00 irr=0x03 bytes=0 us=1500\n'
        'img=3 probe=SW_REGPOLL t_ms=1 sample=1 altst=0x50 err=0x00 irr=0x03 secnr=0x06\n'
        'img=3 probe=SW_REGPOLL t_ms=151 sample=76 altst=0x50 err=0x00 irr=0x03 secnr=0x00\n'
        'img=3 probe=SW_REGPOLL t_ms=301 sample=151 altst=0x50 err=0x00 irr=0x03 secnr=0x21\n'
        'img=3 probe=SW_REGPOLL_END t_ms=1001 samples=500 changes=3 stop=time\n'
        'img=3 probe=R50_SW_DEVRESET delay_ms=50 t_ms=51 rc=ok reset_us=2000 bsy_seen=0 altst=0x50 err=0x01 irr=0x01 secnr=0x06 bcl=0x14 bch=0xeb\n'
        'img=3 probe=R50_SW_REGPOLL t_ms=53 sample=26 altst=0x50 err=0x01 irr=0x01 secnr=0x06\n'
        'img=3 probe=R50_SW_REGPOLL_END t_ms=1001 samples=499 changes=4 stop=time\n')

    def test_regpoll_lines_timing_ignored(self):
        _, ents = compare.parse_record(self.REGPOLL)
        self.assertEqual([e['secnr'] for e in ents if e['probe'] == 'SW_REGPOLL'], ['0x06', '0x00', '0x21'])
        a = self.write('a.txt', self.REGPOLL)
        # other times, sample numbers and reset duration: no difference (timing at most a note)
        t = (self.REGPOLL.replace('t_ms=151 sample=76', 't_ms=163 sample=80').replace('t_ms=301 sample=151', 't_ms=480 sample=230')
             .replace('samples=500', 'samples=333').replace('reset_us=2000', 'reset_us=9000').replace('t_ms=53 sample=26', 't_ms=60 sample=29'))
        rc, out, _ = run([a, self.write('b.txt', t)])
        self.assertEqual(rc, 0, out)
        self.assertIn('img=3 SW_REGPOLL#2: t_ms 301 -> 480', out)      # a timing note, not a difference
        rc, out, _ = run([a, self.write('b2.txt', t), '--ignore-timing'])
        self.assertEqual(rc, 0, out)
        self.assertNotIn('TIMING', out)
        # a different register value is a difference
        rc, out, _ = run([a, self.write('c.txt', self.REGPOLL.replace('secnr=0x21', 'secnr=0x81'))])
        self.assertEqual(rc, 1)
        self.assertIn('img=3 SW_REGPOLL#2: secnr 0x21 vs 0x81', out)
        rc, out, _ = run([a, self.write('d.txt', self.REGPOLL.replace('bch=0xeb', 'bch=0x00'))])
        self.assertEqual(rc, 1)
        self.assertIn('img=3 R50_SW_DEVRESET: bch 0xeb vs 0x00', out)
        rc, out, _ = run([a, self.write('e.txt', self.REGPOLL.replace('rc=ok reset_us', 'rc=timeout reset_us'))])
        self.assertEqual(rc, 1)
        self.assertIn('rc ok vs timeout', out)
        # a record without the R148 lines (an older run): only-in-one-side, or nothing with --only-common
        old = self.write('old.txt', self.REGPOLL.splitlines(True)[0])
        rc, out, _ = run([old, a, '--only-common'])
        self.assertEqual(rc, 0, out)

    CDR_A = (
        'img=3 probe=CDREAD_FLAGS flags=0x20 fad=45150 count=1 ssz=2048 crc=11111111 sense=none cdb=302000b05e00000000000100 rc=ok status=0x50 err=0x00 irr=0x03 bytes=2048 us=21000\n'
        'img=3 probe=CDREAD_FLAGS flags=0x24 fad=45150 count=1 ssz=2048 crc=11111111 sense=none cdb=302400b05e00000000000100 rc=ok status=0x50 err=0x00 irr=0x03 bytes=2048 us=21000\n'
        'img=3 probe=CDREAD_FLAGS flags=0x28 fad=45150 count=1 ssz=- crc=- sense=5/64/00 cdb=302800b05e00000000000100 rc=check status=0x51 err=0x50 irr=0x03 bytes=0 us=1500\n'
        'img=3 probe=CDREAD_FLAGS flags=0x10 fad=45150 count=1 ssz=2352 crc=22222222 sense=none cdb=301000b05e00000000000100 rc=ok status=0x50 err=0x00 irr=0x03 bytes=2352 us=21000\n')

    def test_cdread_flags_lines_parse(self):
        _, ents = compare.parse_record(self.CDR_A)
        self.assertEqual([e['flags'] for e in ents], ['0x20', '0x24', '0x28', '0x10'])
        self.assertEqual(ents[3]['ssz'], '2352')
        self.assertEqual(ents[2]['sense'], '5/64/00')
        a = self.write('a.txt', self.CDR_A)
        rc, out, _ = run([a, a])
        self.assertEqual(rc, 0, out)

    def test_cdread_flags_diff(self):
        a = self.write('a.txt', self.CDR_A)
        # the other drive: 0x28 works, 0x10 returns 2048 instead of 2352
        b_text = self.CDR_A.replace(
            'flags=0x28 fad=45150 count=1 ssz=- crc=- sense=5/64/00 cdb=302800b05e00000000000100 rc=check status=0x51 err=0x50 irr=0x03 bytes=0',
            'flags=0x28 fad=45150 count=1 ssz=2048 crc=11111111 sense=none cdb=302800b05e00000000000100 rc=ok status=0x50 err=0x00 irr=0x03 bytes=2048'
        ).replace('ssz=2352 crc=22222222', 'ssz=2048 crc=11111111').replace('bytes=2352', 'bytes=2048')
        b = self.write('b.txt', b_text)
        rc, out, _ = run([a, b])
        self.assertEqual(rc, 1)
        self.assertIn('img=3 CDREAD_FLAGS flags=0x28: rc check vs ok', out)
        self.assertIn('ssz - vs 2048', out)
        self.assertIn('sense 5/64/00 vs none', out)
        self.assertIn('img=3 CDREAD_FLAGS flags=0x10: ', out)
        self.assertIn('ssz 2352 vs 2048', out)
        self.assertNotIn('flags=0x20:', out)
        self.assertNotIn('flags=0x24:', out)

    def test_cdread_flags_matched_by_flags_not_position(self):
        a = self.write('a.txt', self.CDR_A)
        # a run without the 0x24 line: only that variant is "only in A", the others still pair up
        lines = [l for l in self.CDR_A.splitlines(True) if 'flags=0x24' not in l]
        b = self.write('b.txt', ''.join(lines))
        rc, out, _ = run([a, b])
        self.assertEqual(rc, 1)
        self.assertIn('img=3 CDREAD_FLAGS flags=0x24: only in A', out)
        self.assertEqual(out.count('only in'), 1, out)

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

    @staticmethod
    def cl(img, fad, flags, ssz, data, rc='ok'):
        if rc != 'ok':
            return 'img=%d probe=CDREAD_FLAGS flags=%s fad=%d count=1 ssz=- crc=- sense=5/64/00 cdb=00 rc=%s bytes=0 us=1' % (img, flags, fad, rc)
        return 'img=%d probe=CDREAD_FLAGS flags=%s fad=%d count=1 ssz=%d crc=%08x sense=none cdb=00 rc=ok bytes=%d us=1' % (
            img, flags, fad, ssz, zlib.crc32(data) & 0xFFFFFFFF, ssz)

    def test_cdread_flags_checked_against_image(self):
        fad = 45150 + 3
        lines = [self.cl(3, fad, '0x20', 2048, self.user[3]),
                 self.cl(3, fad, '0x24', 2048, self.user[3]),
                 self.cl(3, fad, '0x28', 2048, b'', rc='check'),                  # CHECK: nothing to verify
                 self.cl(3, fad, '0x10', 2352, self.t3[3 * 2352:4 * 2352])]      # whole raw sector
        rc, out, _ = run(['--image', self.gdi, self.rec(lines)])
        self.assertEqual(rc, 0, out)
        self.assertIn('hashes: 3 match, 0 mismatch, 0 not checked', out)
        bad = bytearray(self.t3[3 * 2352:4 * 2352])
        bad[2000] ^= 1
        rc, out, _ = run(['--image', self.gdi, self.rec([self.cl(3, fad, '0x10', 2352, bytes(bad))])])
        self.assertEqual(rc, 1)
        self.assertIn('flags=0x10: MISMATCH', out)
        # a 2048-byte image cannot give the 2352 form: skipped with a reason, not a mismatch
        rc, out, _ = run(['--image', self.gdi, self.rec([self.cl(3, 150 + 1, '0x10', 2352, b'x' * 2352)])])
        self.assertEqual(rc, 2)
        self.assertIn('0 match, 0 mismatch, 1 not checked', out)
        self.assertIn('cannot make 2352-byte sectors from a 2048-byte image', out)

    def test_raw_file(self):
        p = self.rec([self.hl(0, 45150 + 2, 2048, self.user[2])])
        raw = os.path.join(self.dir, 'track 03.bin')
        rc, out, _ = run(['--raw', raw, '--raw-start-fad', '45150', p])
        self.assertEqual(rc, 0, out)

    def test_nothing_checked(self):
        rc, out, _ = run(['--image', self.gdi, self.rec(['img=0 probe=TUR rc=ok'])])
        self.assertEqual(rc, 2)


class ManifestTests(unittest.TestCase):
    """compare.py --manifest: expected sectors are regenerated from the mksdcard manifest."""

    @classmethod
    def setUpClass(cls):
        sys.path.insert(0, os.path.join(HERE, '..', '..', 'mksdcard'))
        import mksdcard
        cls.mk = mksdcard
        cls.tmp = tempfile.TemporaryDirectory()
        cls.card = os.path.join(cls.tmp.name, 'card')
        mksdcard.make_card(cls.card, small=True)
        cls.manifest = os.path.join(cls.card, mksdcard.MANIFEST_NAME)
        import json
        with open(cls.manifest) as f:
            cls.man = json.load(f)

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def hash_line(self, img, fad, ssz, corrupt=False):
        e = self.man['images']['%02d' % img]
        lba = fad - 150
        tr = [t for t in e['tracks'] if t['start_lba'] <= lba < t['start_lba'] + t['sector_count']][0]
        data, why = self.mk.sector_for(tr, lba, ssz)
        crc = zlib.crc32(data) & 0xFFFFFFFF
        if corrupt:
            crc ^= 1
        return 'img=%d probe=HASH fad=%d ssz=%d crc=%08x\n' % (img, fad, ssz, crc)

    def write(self, text):
        p = os.path.join(self.tmp.name, 'rec.txt')
        with open(p, 'w') as f:
            f.write(text)
        return p

    def test_all_match(self):
        text = ''.join([
            self.hash_line(3, 45150, 2048), self.hash_line(3, 45151, 2048), self.hash_line(3, 45150, 2352),
            self.hash_line(4, 11852, 2048), self.hash_line(4, 11853, 2352),
            self.hash_line(5, 12404, 2048), self.hash_line(6, 45150 + 700, 2048),
            self.hash_line(6, 756 + 150 + 3, 2352),
        ])
        rc, out, _ = run(['--manifest', self.manifest, self.write(text)])
        self.assertEqual(rc, 0, out)
        self.assertIn('8 match, 0 mismatch, 0 not checked', out)

    def test_audio_only_image_11(self):
        # R148: the variant image 11 (audio-only CDI). gdprobe runs only the CDREAD_FLAGS battery on its first
        # audio track (FAD 150); 0x20 / 0x10 return 2352-byte PCM, 0x24 / 0x28 are CHECK (not checkable).
        import json
        d = os.path.join(self.tmp.name, 'v11')
        self.mk.make_variants(d, only={11})
        man_path = os.path.join(d, self.mk.MANIFEST_NAME)
        with open(man_path) as f:
            man = json.load(f)
        t1 = man['images']['11']['tracks'][0]
        crc = zlib.crc32(self.mk.audio_sector(0)) & 0xFFFFFFFF
        text = ('img=11 probe=NO_DATA_TRACK audio_fad=150 reads=skipped\n'
                'img=11 probe=CDREAD_FLAGS flags=0x20 fad=150 count=1 ssz=2352 crc=%08x sense=none cdb=302000000096000000000100 rc=ok status=0x50 err=0x00 irr=0x03 bytes=2352 us=1\n'
                'img=11 probe=CDREAD_FLAGS flags=0x24 fad=150 count=1 ssz=- crc=- sense=5/64/00 cdb=302400000096000000000100 rc=check status=0x51 err=0x50 irr=0x03 bytes=0 us=1\n'
                'img=11 probe=CDREAD_FLAGS flags=0x10 fad=150 count=1 ssz=2352 crc=%08x sense=none cdb=301000000096000000000100 rc=ok status=0x50 err=0x00 irr=0x03 bytes=2352 us=1\n'
                % (crc, crc))
        self.assertEqual(t1['start_lba'], 0)
        rc, out, _ = run(['--manifest', man_path, self.write(text)])
        self.assertEqual(rc, 0, out)
        self.assertIn('2 match, 0 mismatch, 0 not checked', out)
        # a drive that answered 2048 bytes for an audio sector cannot be checked (and says why)
        rc, out, _ = run(['--manifest', man_path, self.write(text.replace('flags=0x20 fad=150 count=1 ssz=2352', 'flags=0x20 fad=150 count=1 ssz=2048'))])
        self.assertIn('audio track needs ssz 2352', out)

    def test_mismatch_reported_per_image_and_lba(self):
        text = self.hash_line(3, 45150, 2048) + self.hash_line(4, 11853, 2048, corrupt=True)
        rc, out, _ = run(['--manifest', self.manifest, self.write(text)])
        self.assertEqual(rc, 1)
        self.assertIn('img=4 fad=11853 lba=11703 track 2: MISMATCH', out)
        self.assertIn('1 match, 1 mismatch', out)

    def cdr_line(self, img, fad, flags, ssz, corrupt=False):
        e = self.man['images']['%02d' % img]
        lba = fad - 150
        tr = [t for t in e['tracks'] if t['start_lba'] <= lba < t['start_lba'] + t['sector_count']][0]
        data, why = self.mk.sector_for(tr, lba, ssz)
        crc = zlib.crc32(data) & 0xFFFFFFFF
        if corrupt:
            crc ^= 1
        return ('img=%d probe=CDREAD_FLAGS flags=%s fad=%d count=1 ssz=%d crc=%08x sense=none cdb=00 rc=ok '
                'bytes=%d us=1\n' % (img, flags, fad, ssz, crc, ssz))

    def test_cdread_flags_incl_raw_2352(self):
        # 2048 for 0x20/0x24/0x28 and the rebuilt raw sector (sync+header+user+EDC/ECC) for 0x10
        text = ''.join([self.cdr_line(3, 45150, '0x20', 2048), self.cdr_line(3, 45150, '0x24', 2048),
                        self.cdr_line(3, 45150, '0x28', 2048), self.cdr_line(3, 45150, '0x10', 2352),
                        self.cdr_line(4, 11852, '0x10', 2352),
                        'img=3 probe=CDREAD_FLAGS flags=0x28 fad=45150 count=1 ssz=- crc=- sense=5/64/00 cdb=00 rc=check bytes=0 us=1\n'])
        rc, out, _ = run(['--manifest', self.manifest, self.write(text)])
        self.assertEqual(rc, 0, out)
        self.assertIn('5 match, 0 mismatch, 0 not checked', out)

    def test_cdread_flags_mismatch_and_unknown_size(self):
        text = self.cdr_line(3, 45150, '0x10', 2352, corrupt=True)
        rc, out, _ = run(['--manifest', self.manifest, self.write(text)])
        self.assertEqual(rc, 1)
        self.assertIn('img=3 fad=45150 flags=0x10 lba=45000 track', out)
        self.assertIn('MISMATCH', out)
        # a drive that returned 2340 bytes (not a form we can rebuild): not checked, with the reason
        text = 'img=3 probe=CDREAD_FLAGS flags=0x10 fad=45150 count=1 ssz=2340 crc=12345678 sense=none cdb=00 rc=ok bytes=2340 us=1\n'
        rc, out, _ = run(['--manifest', self.manifest, self.write(text)])
        self.assertEqual(rc, 2)
        self.assertIn('flags=0x10: not checked (unsupported ssz 2340', out)

    def test_not_checked_and_filter(self):
        text = ('img=2 probe=HASH fad=45150 ssz=2048 crc=00000000\n'      # dcload folder: no generated image
                'img=3 probe=HASH fad=9999999 ssz=2048 crc=00000000\n'    # outside every track
                'img=3 probe=HASH fad=906 ssz=2048 crc=00000000\n')       # audio track needs ssz 2352
        rc, out, _ = run(['--manifest', self.manifest, self.write(text)])
        self.assertEqual(rc, 2, out)
        self.assertIn('0 match, 0 mismatch, 3 not checked', out)
        text = self.hash_line(3, 45150, 2048) + 'img=4 probe=HASH fad=11852 ssz=2048 crc=00000000\n'
        rc, out, _ = run(['--manifest', self.manifest, '--img', '3', self.write(text)])
        self.assertEqual(rc, 0, out)


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
        cdr = [e for e in ents if e['probe'] == 'CDREAD_FLAGS' and e['img'] == 3]
        self.assertEqual([e['flags'] for e in cdr], ['0x20', '0x24', '0x28', '0x10'])
        self.assertEqual([e['ssz'] for e in cdr], ['2048', '2048', '2048', '2352'])

    def test_r148_sample_record_from_test_all(self):
        p = os.environ.get('GDPROBE_SAMPLE_R148')
        if not p or not os.path.exists(p):
            self.skipTest('GDPROBE_SAMPLE_R148 (record dumped by test_all) not set')
        rc, out, _ = run([p, p])
        self.assertEqual(rc, 0, out)
        _, ents = compare.parse_record(compare.read_text(p))
        self.assertTrue(any(e['probe'] == 'NO_DATA_TRACK' for e in ents))


if __name__ == '__main__':
    unittest.main(verbosity=2)
