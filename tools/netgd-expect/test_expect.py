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
        # tests/atapi.rs: REQ_STAT of a GDI after a switch (paused, head at track 3's start, D2/D4; stock 01 8f 41 03 01 00 b0 5e),
        # and GET_SCD format 1 there (audio status still 0x15)
        self.assertEqual(self.one(3, 'REQ_STAT')['data'], '018f41030100b05e0000')   # repeat nibble F when idle (D3)
        self.assertEqual(self.one(3, 'GET_SCD_1')['data'], '0015000e4103010000000000b05e')
        # TOC area 0 of the GDI: track 1 = 0x41 at FAD 150, track 2 audio at FAD 906 (LBA 756 + 150)
        self.assertTrue(self.one(3, 'GET_TOC_0')['data'].startswith('410000960100038a'))
        # REQ_SES(0): session count 1, lead-out = end of track 3 (D1, stock): 45000 + 1100 + 150 = 0x00b4aa
        # (it was 549150 + 150 = 0x0861b4); REQ_SES(1): track 1 at FAD 150
        self.assertEqual(self.one(3, 'REQ_SES_0')['data'], '01000100b4aa')
        self.assertEqual(self.one(3, 'GET_TOC_1')['data'][-8:], '4100b4aa')
        self.assertEqual(self.one(3, 'REQ_SES_1')['data'], '010001000096')
        # a GDI has no session 2 / 3: CHECK, ILLEGAL REQUEST in stock's form (status 0x11, err 0x50; D5)
        for s in (2, 3):
            e = self.one(3, 'REQ_SES_%d' % s)
            self.assertEqual((e['rc'], e['status'], e['err'], e['bytes']), ('check', '0x11', '0x50', '0'))
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

    def test_cd_read_checks_the_sector_type_and_is_always_2048(self):
        # D8/D9 (stock, every record): Mode 1 (0x24) is refused on a CDI, Mode 2 Form 1 (0x28) on a GDI,
        # with CHECK 5/64/00; whatever reads is 2048 bytes (no raw 2352 sectors, D10).
        for img, bad in ((3, '0x28'), (4, '0x24'), (5, '0x24'), (6, '0x28')):
            rows = [e for e in self.entries if e['img'] == img and e['probe'] == 'CDREAD_FLAGS']
            self.assertEqual([r['flags'] for r in rows], ['0x20', '0x24', '0x28', '0x10'])
            good = [r for r in rows if r['flags'] != bad]
            self.assertEqual(len({(r['ssz'], r['crc'], r['bytes'], r['rc']) for r in good}), 1)
            self.assertEqual(good[0]['ssz'], '2048')
            refused = [r for r in rows if r['flags'] == bad][0]
            self.assertEqual((refused['rc'], refused['status'], refused['err'], refused['sense']),
                             ('check', '0x11', '0x50', '5/64/00'))

    def test_hashes_match_the_generator(self):
        man = os.path.join(self.card, 'NETGD-PROBE-MANIFEST.json')
        rc, out = run_compare(['--manifest', man, self.pred])
        self.assertIn(' 0 mismatch', out)
        self.assertEqual(rc, 0, out)

    def test_empty_folder_07_is_accepted_and_reports_no_medium(self):
        # stock_20261121_04_img3-7.txt img=7 (D12): accepted, the swap window, then no medium for good
        cmd = self.one(7, 'SW_CMD')
        self.assertEqual((cmd['rc'], cmd['status'], cmd['err']), ('ok', '0x50', '0x00'))
        polls = [e for e in self.entries if e['img'] == 7 and e['probe'] == 'SW_POLL']
        self.assertEqual([(p['rc'], p['status'], p['err'], p['sense']) for p in polls],
                         [('check', '0x11', '0x20', '2/3a/00'), ('check', '0x11', '0x20', '2/04/01'),
                          ('check', '0x11', '0x20', '2/3a/00')])
        self.assertEqual(self.one(7, 'SW_RESULT')['rc'], 'timeout')
        # the no-medium battery: warmup gives up, the disc commands are CHECK, no reads
        self.assertEqual([self.one(7, 'TUR_W%d' % i)['err'] for i in range(8)], ['0x20'] * 8)
        self.assertEqual(self.one(7, 'REQ_ERROR_W0')['data'], 'f0000200000000003a00')
        self.assertEqual(self.one(7, 'WARMUP')['ready'], '0')
        for probe, err in (('TUR', '0x20'), ('REQ_STAT', '0x20'), ('GET_TOC_0', '0x20'), ('GET_TOC_1', '0x20'),
                           ('REQ_SES_0', '0x20'), ('REQ_SES_1', '0x50'), ('REQ_SES_2', '0x50'), ('REQ_SES_3', '0x50'),
                           ('GET_SCD_0', '0x20'), ('GET_SCD_1', '0x20')):
            e = self.one(7, probe)
            self.assertEqual((e['rc'], e['status'], e['err'], e['bytes']), ('check', '0x11', err, '0'), probe)
        for probe in ('REQ_MODE_0_10', 'REQ_MODE_18_8', 'REQ_MODE_0_32', 'REQ_ERROR', 'GDEMU_VERSION'):
            self.assertEqual(self.one(7, probe)['rc'], 'ok', probe)
        self.assertEqual(self.one(7, 'REQ_ERROR')['data'], 'f0' + '00' * 9)
        self.assertFalse([e for e in self.entries if e['img'] == 7 and e['probe'] in ('IP_READ', 'HASH', 'TP_000')])
        # a later switch to a folder with an image is a normal one (02 of the test card has no image either)
        text = expect_sd.predict(self.card, images=[7, 3], return_to=0)
        _, entries = compare.parse_record(text)
        polls = [e for e in entries if e['img'] == 3 and e['probe'] == 'SW_POLL']
        self.assertEqual([p['sense'] for p in polls], ['2/3a/00', '2/04/01', '6/28/00', 'none'])
        self.assertEqual([e['ready'] for e in entries if e['probe'] == 'WARMUP'], ['0', '1'])

    def test_mount_gets_the_swap_window_then_unit_attention_then_ready(self):
        # stock (D13): 2/3A/00 from t=0, 2/04/01 from ~150 ms, 6/28/00 once at ~300 ms, then READY
        polls = [e for e in self.entries if e['img'] == 3 and e['probe'] == 'SW_POLL']
        self.assertEqual([(p['rc'], p['status'], p['err'], p['sense']) for p in polls],
                         [('check', '0x11', '0x20', '2/3a/00'), ('check', '0x11', '0x20', '2/04/01'),
                          ('check', '0x11', '0x60', '6/28/00'), ('ok', '0x50', '0x00', 'none')])
        self.assertEqual(self.one(3, 'SW_RESULT')['rc'], 'ok')
        self.assertTrue(all(p['t_ms'] == '-' for p in polls))      # time is not simulated

    def test_missing_folder_mounts_folder_01(self):
        # stock_20261122_09_missing-folder.txt img=50 (folders 01-09 only): 52 82 50 is accepted (0x50), the normal
        # swap window runs and folder 01 (the GDmenu CDI) is mounted; the same with reset_goto = 1 and 2 (record 10).
        card = os.path.join(self.tmp, 'card01')
        shutil.copytree(self.card, card)
        shutil.copy(os.path.join(card, '04', 'disc.cdi'), os.path.join(card, '01', 'disc.cdi'))
        text = expect_sd.predict(card, images=[4, 42, 3], return_to=2)
        _, entries = compare.parse_record(text)
        one = lambda img, probe: [e for e in entries if e['img'] == img and e['probe'] == probe][0]
        cmd = one(42, 'SW_CMD')
        self.assertEqual((cmd['rc'], cmd['status'], cmd['err']), ('ok', '0x50', '0x00'))
        polls = [e for e in entries if e['img'] == 42 and e['probe'] == 'SW_POLL']
        self.assertEqual([p['sense'] for p in polls], ['2/3a/00', '2/04/01', '6/28/00', 'none'])
        self.assertEqual(one(42, 'SW_RESULT')['rc'], 'ok')
        # what is mounted is 01 (here a copy of 04's CDI), not the image before the switch
        for probe in ('GET_TOC_0', 'REQ_SES_0', 'REQ_STAT', 'IP_READ'):
            self.assertEqual(one(42, probe).get('data'), one(4, probe).get('data'), probe)
        # a later switch is a normal one
        polls = [e for e in entries if e['img'] == 3 and e['probe'] == 'SW_POLL']
        self.assertEqual([p['sense'] for p in polls], ['2/3a/00', '2/04/01', '6/28/00', 'none'])
        self.assertEqual(one(3, 'GET_TOC_0')['data'], self.one(3, 'GET_TOC_0')['data'])

    def test_missing_folder_with_unusable_01_reports_no_medium(self):
        # folder 01 of the test card has no image: the switch to 01 is the D12 empty-folder one
        text = expect_sd.predict(self.card, images=[3, 42], return_to=0)
        _, entries = compare.parse_record(text)
        one = lambda img, probe: [e for e in entries if e['img'] == img and e['probe'] == probe][0]
        cmd = one(42, 'SW_CMD')
        self.assertEqual((cmd['rc'], cmd['status'], cmd['err']), ('ok', '0x50', '0x00'))
        polls = [e for e in entries if e['img'] == 42 and e['probe'] == 'SW_POLL']
        self.assertEqual([p['sense'] for p in polls], ['2/3a/00', '2/04/01', '2/3a/00'])
        self.assertEqual(one(42, 'SW_RESULT')['rc'], 'timeout')
        self.assertEqual(one(42, 'TUR_W0')['err'], '0x20')
        self.assertFalse([e for e in entries if e['img'] == 42 and e['probe'] == 'IP_READ'])

    def test_number_past_999_with_no_folder_is_still_refused(self):
        # not probed on stock; NetGD keeps refusing it (ILLEGAL REQUEST) and the old image stays
        text = expect_sd.predict(self.card, images=[3, 1000], return_to=0)
        _, entries = compare.parse_record(text)
        one = lambda img, probe: [e for e in entries if e['img'] == img and e['probe'] == probe][0]
        cmd = one(1000, 'SW_CMD')
        self.assertEqual((cmd['rc'], cmd['status'], cmd['err']), ('check', '0x11', '0x50'))
        self.assertEqual(one(1000, 'GET_TOC_0')['data'], one(3, 'GET_TOC_0')['data'])

    def test_compare_other_record_with_only_common(self):
        # a stock-like record: one probe, real timings, a different REQ_SES(2) answer
        want = 'img=3 probe=REQ_SES_2 cdb=150002000600000000000000 rc=check status=0x11 err=0x50 irr=0x03 bytes=0 us=-'
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
