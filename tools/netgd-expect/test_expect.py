#!/usr/bin/env python3
"""Tests for tools/netgd-expect: expect_sd.py + common/discimg/examples/expect.rs on a --small mksdcard set.

    python tools/netgd-expect/test_expect.py          (needs cargo; set CARGO_TARGET_DIR to avoid a build-lock wait)
Optional: GDPROBE_SAMPLE=<a record from the C test harness> also checks that such a record can be compared.
"""
import contextlib
import io
import os
import shutil
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, '..', '..'))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(ROOT, 'tools', 'mksdcard'))
sys.path.insert(0, os.path.join(ROOT, 'tools', 'gdprobe'))
import compare  # noqa: E402
import expect_sd  # noqa: E402
import mksdcard  # noqa: E402


def run_compare(argv):
    out = io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(out):
        rc = compare.main(argv)
    return rc, out.getvalue()


class ExpectTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.mkdtemp(prefix='netgd_expect_')
        cls.card = os.path.join(cls.tmp, 'card')
        with contextlib.redirect_stdout(io.StringIO()):
            mksdcard.make_card(cls.card, small=True)
        cls.text = expect_sd.predict(cls.card)
        cls.pred = os.path.join(cls.tmp, 'pred.txt')
        with open(cls.pred, 'w', newline='\n') as f:
            f.write(cls.text)
        _, cls.entries = compare.parse_record(cls.text)

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.tmp, ignore_errors=True)

    def one(self, img, probe, n=0):
        hits = [e for e in self.entries if e['img'] == img and e['probe'] == probe]
        return hits[n]

    def test_self_compare_has_no_differences(self):
        rc, out = run_compare([self.pred, self.pred])
        self.assertEqual(rc, 0, out)
        self.assertIn('No differences', out)

    def test_every_probe_line_is_complete(self):
        names = {e['probe'] for e in self.entries}
        for need in ('SW_CMD', 'SW_POLL', 'SW_RESULT', 'TUR_W0', 'WARMUP', 'TUR', 'REQ_STAT', 'REQ_MODE_0_10',
                     'REQ_MODE_18_8', 'REQ_MODE_0_32', 'REQ_ERROR', 'GET_TOC_0', 'GET_TOC_1', 'REQ_SES_0',
                     'REQ_SES_1', 'REQ_SES_2', 'REQ_SES_3', 'GET_SCD_0', 'GET_SCD_1', 'GDEMU_VERSION', 'IP_READ',
                     'HASH', 'CDREAD_FLAGS', 'TP_000', 'TP_SUMMARY', 'SEEK_00', 'END'):
            self.assertIn(need, names)
        for e in self.entries:
            if 'cdb' in e:
                self.assertEqual(e.get('us'), '-', e['_line'][:80])   # timings are never invented

    def test_bytes_match_discimg_test_expectations(self):
        # tests/atapi.rs: standby REQ_STAT of a GDI, and the standby GET_SCD format 1
        self.assertEqual(self.one(3, 'REQ_STAT')['data'], '028f4101010000960000')   # repeat nibble F when idle (D3)
        self.assertEqual(self.one(3, 'GET_SCD_1')['data'], '0015000e41010100000000000096')
        # TOC area 0 of the GDI: track 1 = 0x41 at FAD 150, track 2 audio at FAD 906 (LBA 756 + 150)
        self.assertTrue(self.one(3, 'GET_TOC_0')['data'].startswith('410000960100038a'))
        # REQ_SES(0): session count 1, lead-out = end of track 3 (D1, stock): 45000 + 1100 + 150 = 0x00b4aa
        # (it was 549150 + 150 = 0x0861b4); REQ_SES(1): track 1 at FAD 150
        self.assertEqual(self.one(3, 'REQ_SES_0')['data'], '02000100b4aa')
        self.assertEqual(self.one(3, 'GET_TOC_1')['data'][-8:], '4100b4aa')
        self.assertEqual(self.one(3, 'REQ_SES_1')['data'], '020001000096')
        # a GDI has no session 2 / 3: ABRT
        for s in (2, 3):
            e = self.one(3, 'REQ_SES_%d' % s)
            self.assertEqual((e['rc'], e['status'], e['err'], e['bytes']), ('check', '0x51', '0x04', '0'))
        # REQ_MODE as the firmware answers it (the stock 32-byte block, R117), REQ_ERROR F0 + no sense, version 5.20.0
        self.assertEqual(self.one(3, 'REQ_MODE_18_8')['data'], '52657620362e3432')
        self.assertEqual(self.one(3, 'REQ_MODE_0_10')['data'], '0000000000b419000008')
        self.assertEqual(self.one(3, 'REQ_MODE_0_32')['data'],
                         '0000000000b419000008534520202020202052657620362e3432393930333136')
        self.assertEqual(self.one(3, 'REQ_ERROR')['data'], 'f0' + '00' * 9)
        self.assertEqual(self.one(3, 'GDEMU_VERSION')['data'], '0000000000002005')

    def test_cdi_with_two_tracks_has_req_ses_2(self):
        # D-1 (fixed, R65): REQ_SES(2) is the first track of the last session; it used to abort (boot area guessed as 45000)
        self.assertEqual(self.one(4, 'REQ_SES_2')['rc'], 'ok')
        self.assertEqual(self.one(4, 'REQ_SES_0')['data'][4:6], '02')       # CDI: session count 2 in the track byte
        self.assertEqual(self.one(4, 'IP_READ')['fad'], '11852')

    def test_cd_read_ignores_flags_and_is_always_2048(self):
        for img in (3, 4, 5, 6):
            rows = [e for e in self.entries if e['img'] == img and e['probe'] == 'CDREAD_FLAGS']
            self.assertEqual([r['flags'] for r in rows], ['0x20', '0x24', '0x28', '0x10'])
            self.assertEqual(len({(r['ssz'], r['crc'], r['bytes'], r['rc']) for r in rows}), 1)
            self.assertEqual(rows[0]['ssz'], '2048')

    def test_hashes_match_the_generator(self):
        man = os.path.join(self.card, 'NETGD-PROBE-MANIFEST.json')
        rc, out = run_compare(['--manifest', man, self.pred])
        self.assertIn(' 0 mismatch', out)
        self.assertEqual(rc, 0, out)

    def test_empty_folder_07_is_refused_and_media_untouched(self):
        cmd = self.one(7, 'SW_CMD')
        self.assertEqual((cmd['rc'], cmd['status'], cmd['err']), ('check', '0x51', '0x54'))   # ILLEGAL REQUEST
        polls = [e for e in self.entries if e['img'] == 7 and e['probe'] == 'SW_POLL']
        self.assertEqual([p['rc'] for p in polls], ['ok'])            # no UNIT ATTENTION: nothing was mounted
        # the battery under img=7 is image 06's, UNIT ATTENTION already gone
        self.assertEqual(self.one(7, 'TUR_W0')['rc'], 'ok')
        self.assertEqual(self.one(7, 'GET_TOC_0')['data'], self.one(6, 'GET_TOC_0')['data'])

    def test_mount_gets_unit_attention_then_ready(self):
        polls = [e for e in self.entries if e['img'] == 3 and e['probe'] == 'SW_POLL']
        self.assertEqual([(p['rc'], p['err']) for p in polls], [('check', '0x60'), ('ok', '0x00')])
        self.assertEqual(polls[0]['sense'], '6/28/00')   # medium changed (D7)

    def test_compare_other_record_with_only_common(self):
        # a stock-like record: one probe, real timings, a different REQ_SES(2) answer
        want = 'img=3 probe=REQ_SES_2 cdb=150002000600000000000000 rc=check status=0x51 err=0x04 irr=0x03 bytes=0 us=-'
        self.assertIn(want, self.text)
        stock = ('img=3 probe=REQ_SES_2 cdb=150002000600000000000000 rc=ok status=0x50 err=0x00 irr=0x03 '
                 'bytes=6 data=01000300b05e us=1234\n')
        other = os.path.join(self.tmp, 'stock.txt')
        with open(other, 'w', newline='\n') as f:
            f.write(stock)
        rc, out = run_compare([other, self.pred])
        self.assertEqual(rc, 1)
        self.assertIn('only in B', out)
        rc2, out2 = run_compare(['--only-common', other, self.pred])
        self.assertEqual(rc2, 1)
        self.assertEqual(out2.count('\n  img='), 1, out2)        # only REQ_SES_2 differs
        self.assertIn('REQ_SES_2', out2)
        self.assertNotIn('only in', out2)

    def test_c_harness_record_can_be_compared(self):
        sample = os.environ.get('GDPROBE_SAMPLE') or '/tmp/gdprobe_sample.txt'
        if not os.path.exists(sample):
            self.skipTest('no sample record (run tools/gdprobe/tests/run_tests.sh first)')
        rc, out = run_compare(['--only-common', sample, self.pred])
        self.assertIn('summary:', out)
        self.assertIn(rc, (0, 1), out)


if __name__ == '__main__':
    unittest.main()
