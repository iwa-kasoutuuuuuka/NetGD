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

    def test_cdbst_and_secnr_follow_stock(self):
        # gdprobe's cdbst= is the CDB-phase ALTSTATUS, which keeps the previous command's DRDY (R147): 0x18 after
        # a CHECK, else 0x58; a CHECK in CDREAD_FLAGS and in the switch polls is followed by an unlogged REQ_ERROR.
        # secnr= is the SECNR byte at completion (R144 b, R148): format | pause at rest, 0x07 with no medium,
        # 0x06 / 0x00 / format|1 through the window. records 13 and 14 match all of it.
        _, entries = compare.parse_record(self.text)
        one = lambda img, probe, i=0: [e for e in entries if e['img'] == img and e['probe'] == probe][i]
        self.assertEqual(one(3, 'REQ_SES_2')['cdbst'], '0x58')
        self.assertEqual(one(3, 'REQ_SES_3')['cdbst'], '0x18')          # after REQ_SES_2's CHECK
        self.assertEqual(one(3, 'GET_SCD_0')['cdbst'], '0x18')          # after REQ_SES_3's CHECK
        flags = [e for e in entries if e['img'] == 3 and e['probe'] == 'CDREAD_FLAGS']
        self.assertEqual([e['cdbst'] for e in flags], ['0x58'] * 4)    # hidden REQ_ERROR after the 0x28 CHECK
        self.assertEqual([one(3, 'SW_POLL', i)['secnr'] for i in range(4)], ['0x06', '0x00', '0x81', '0x81'])
        self.assertEqual(one(3, 'TUR')['secnr'], '0x81')
        self.assertEqual(one(4, 'SW_CMD')['secnr'], '0x81')             # still the previous disc's
        self.assertEqual(one(4, 'TUR')['secnr'], '0x21')                # CDI with a Mode 2 track: XA
        self.assertEqual([one(7, 'SW_POLL', i)['secnr'] for i in range(3)], ['0x06', '0x00', '0x07'])
        self.assertEqual((one(7, 'TUR_W0')['cdbst'], one(7, 'TUR_W0')['secnr']), ('0x58', '0x07'))
        self.assertEqual(one(7, 'REQ_ERROR_W0')['cdbst'], '0x18')
        self.assertNotIn('secnr', one(3, 'SW_CMD'))                     # the boot disc's SECNR is not known

    def test_cdbst_carries_a_check_from_the_last_battery_command_into_the_next_section(self):
        # Records 17 / 18 (cdread_flags_test=0, no throughput / seek): the battery ends with IP_READ, which is a
        # CHECK for the CDI with a Mode 2 track at 0x24, and no hidden REQ_ERROR follows it, so the next section's
        # first command (RS_SW_CMD) sees 0x18. With the flag test on, the last command is CDREAD_FLAGS 0x10 (ok): 0x58.
        base = ['--cdread-flags', '0x24', '--no-throughput', '--no-seek']
        for extra, want in (([ '--no-flags-test'], '0x18'), ([], '0x58')):
            text = expect_sd.predict(self.card, images=[4], return_to=0, regpoll=True, reswitch=True,
                                     passthrough=base + extra)
            _, entries = compare.parse_record(text)
            get = lambda probe: [e for e in entries if e['img'] == 4 and e['probe'] == probe]
            self.assertEqual(get('IP_READ')[0]['rc'], 'check')
            self.assertEqual(get('RS_SW_CMD')[0]['cdbst'], want)
            self.assertEqual(get('RS_REQ_STAT')[0]['cdbst'], '0x58')

    def test_regpoll_and_device_reset_as_netgd_answers(self):
        # gdprobe swap_regpoll / swap_devreset (R154), as NetGD answers since R159: the R<ms>_ switch is a
        # same-folder re-switch (a no-op: SECNR flat, ALTSTATUS 0x50), then DEVICE RESET re-detects: status 0x00,
        # error 0x01, IRR 1, 14/EB, SECNR 0x00 at once and the byte at rest detect_time later, then UNIT
        # ATTENTION (records 15 / 16, seq 17).
        text = expect_sd.predict(self.card, images=[3, 4, 7], return_to=0, regpoll=True, devreset=(50, 200))
        _, entries = compare.parse_record(text)
        get = lambda img, probe: [e for e in entries if e['img'] == img and e['probe'] == probe]
        self.assertEqual([e['secnr'] for e in get(4, 'SW_REGPOLL')], ['0x81', '0x06', '0x00', '0x21'])
        self.assertEqual([e['altst'] for e in get(4, 'SW_REGPOLL')], ['0x50'] * 4)
        for pfx in ('R50_', 'R200_'):
            for img, final in ((3, '0x81'), (4, '0x21'), (7, '0x07')):
                rst = get(img, pfx + 'SW_DEVRESET')[0]
                self.assertEqual((rst['altst'], rst['err'], rst['irr'], rst['secnr'], rst['bcl'], rst['bch']),
                                 ('0x00', '0x01', '0x01', '0x00', '0x14', '0xeb'))
                polls = get(img, pfx + 'SW_REGPOLL')
                self.assertEqual([e['secnr'] for e in polls], [final, '0x00', final])   # no 0x06: no window
                self.assertEqual([e['altst'] for e in polls], ['0x50', '0x00', '0x00'])
                # ERROR is sticky: the last CHECK's (the battery's 0x50, or 2/3A/00's 0x20 with no medium, and in
                # R200_ the UNIT ATTENTION 0x60 of R50_), then the reset's 0x01.
                first = {('R50_', 7): '0x20', ('R200_', 7): '0x20', ('R50_', 3): '0x50', ('R50_', 4): '0x50'}
                self.assertEqual([e['err'] for e in polls], [first.get((pfx, img), '0x60'), '0x01', '0x01'])
                self.assertEqual([e['irr'] for e in polls], ['0x03', '0x01', '0x01'])
                sw = get(img, pfx + 'SW_POLL')
                self.assertEqual(sw[0]['sense'], '2/3a/00' if img == 7 else '6/28/00')
                self.assertNotIn('ua_28', get(img, pfx + 'SW_RESULT')[0])
                self.assertEqual(len(get(img, pfx + 'REQ_STAT')), 1)
                self.assertEqual(len(get(img, pfx + 'REQ_ERROR')), 1)
        self.assertTrue(get(4, 'R50_REQ_STAT')[0]['data'].startswith('012f01'))
        # A bitstream without ERROR bit 0 shows 0x00 there.
        old = expect_sd.predict(self.card, images=[4], return_to=0, devreset=(50,), passthrough=['--old-bitstream'])
        _, oe = compare.parse_record(old)
        self.assertEqual([e['err'] for e in oe if e['probe'] == 'R50_SW_DEVRESET'], ['0x00'])
        # ... and clears ERROR on every PACKET write: 0x00 after the successful re-switch.
        self.assertEqual([e['err'] for e in oe if e['probe'] == 'R50_SW_REGPOLL'], ['0x00', '0x00', '0x00'])

    def test_head_follows_cd_read_and_sticky_error(self):
        # Stock seq 17/18: after the battery's reads and a no-op re-switch REQ_STAT reports where the last CD_READ
        # ended (GDI 03: IP_READ of 7 at 45150 -> 0xB065); a read refused for its type leaves the start FAD
        # (CDI 04 at 0x2E4C); DEVICE RESET parks the head again. ERROR stays the last CHECK's.
        text = expect_sd.predict(self.card, images=[3, 4], return_to=0, regpoll=True, reswitch=True,
                                 devreset_only=True, passthrough=['--no-throughput', '--no-seek', '--no-flags-test'])
        _, entries = compare.parse_record(text)
        get = lambda img, probe: [e for e in entries if e['img'] == img and e['probe'] == probe]
        self.assertEqual(get(3, 'RS_REQ_STAT')[0]['data'], '018f41030100b0650000')
        self.assertEqual(get(3, 'DR_REQ_STAT')[0]['data'], '018f41030100b05e0000')
        self.assertEqual(get(4, 'RS_REQ_STAT')[0]['data'], '012f410201002e4c0000')
        self.assertEqual(get(4, 'DR_REQ_STAT')[0]['data'], '012f0101010000960000')
        self.assertEqual(get(3, 'REQ_STAT')[0]['data'], '018f41030100b05e0000')     # before the reads
        # The first switch: the img=0 battery's REQ_SES_3 CHECK (0x50); img=4's: DR_'s UNIT ATTENTION (0x60);
        # RS_ after img=4's refused IP_READ: 0x50.
        self.assertEqual({e['err'] for e in get(3, 'SW_REGPOLL')}, {'0x50'})
        self.assertEqual({e['err'] for e in get(4, 'SW_REGPOLL')}, {'0x60'})
        self.assertEqual([e['err'] for e in get(4, 'RS_SW_REGPOLL')], ['0x50'])

    def test_reswitch_and_devreset_only_as_netgd_answers(self):
        # gdprobe seq 17: RS_ (52 82 n for the mounted folder) is a no-op, no UNIT ATTENTION; DR_ (DEVICE RESET
        # with no switch) re-detects and raises the UNIT ATTENTION with a medium; the empty folder stays 2/3A/00.
        text = expect_sd.predict(self.card, images=[3, 7], return_to=0, regpoll=True, reswitch=True,
                                 devreset_only=True)
        _, entries = compare.parse_record(text)
        get = lambda img, probe: [e for e in entries if e['img'] == img and e['probe'] == probe]
        for img, final, medium in ((3, '0x81', True), (7, '0x07', False)):
            self.assertEqual([e['secnr'] for e in get(img, 'RS_SW_REGPOLL')], [final])
            self.assertEqual(get(img, 'RS_SW_POLL')[0]['sense'], 'none' if medium else '2/3a/00')
            self.assertEqual(get(img, 'RS_SW_RESULT')[0]['ua_28'], '0')
            self.assertEqual(get(img, 'DR_IDLE')[0]['idle_ms'], '500')
            rst = get(img, 'DR_SW_DEVRESET')[0]
            self.assertEqual((rst['delay_ms'], rst['altst'], rst['err'], rst['secnr']), ('0', '0x00', '0x01', '0x00'))
            self.assertEqual([e['secnr'] for e in get(img, 'DR_SW_REGPOLL')], ['0x00', final])
            self.assertEqual(get(img, 'DR_SW_POLL')[0]['sense'], '6/28/00' if medium else '2/3a/00')
            self.assertEqual(get(img, 'DR_SW_RESULT')[0]['ua_28'], '1' if medium else '0')
        self.assertEqual(get(3, 'DR_REQ_STAT')[0]['data'], '018f41030100b05e0000')

    def test_regpoll_first_sample_is_the_old_byte(self):
        # NetGD completes 52 82 before it writes the window's 0x06 (gdemu_mount_staged: finish_ok, then
        # show_window_open), into an image and into the empty folder alike: sample 1 is the old SECNR byte. Stock
        # agrees for images (records 15-18 img=3/4/11); into the empty folder 07 stock shows 0x06 already at sample 1.
        text = expect_sd.predict(self.card, images=[4, 3, 7], return_to=0, regpoll=True)
        _, entries = compare.parse_record(text)
        get = lambda img: [e['secnr'] for e in entries if e['img'] == img and e['probe'] == 'SW_REGPOLL']
        self.assertEqual(get(3), ['0x21', '0x06', '0x00', '0x81'])
        self.assertEqual(get(7), ['0x81', '0x06', '0x00', '0x07'])

    def test_devreset_cmd_as_netgd_answers(self):
        # gdprobe seq 20 devreset_cmd (record 20): DEVICE RESET, then TUR / REQ_STAT / REQ_ERROR 10 / 50 / 100 ms
        # later inside the re-detect (detect_time 150): NOT READY 2/04/01, SECNR 0x00; then UNIT ATTENTION and
        # READY (the empty folder: 2/3A/00, no READY). The next switch's SW_CMD sees the last TUR's good end (0x58).
        text = expect_sd.predict(self.card, images=[4, 3, 7], return_to=2, devreset_cmd=(10, 50, 100),
                                 passthrough=['--cdread-flags', '0x24', '--no-throughput', '--no-seek',
                                              '--no-flags-test'])
        _, entries = compare.parse_record(text)
        get = lambda img, probe: [e for e in entries if e['img'] == img and e['probe'] == probe]
        self.assertEqual(get(4, 'IP_READ')[0]['rc'], 'check')
        self.assertEqual(get(3, 'SW_CMD')[0]['cdbst'], '0x58')
        for d in (10, 50, 100):
            p = 'DC%d_' % d
            for img, final, medium in ((4, '0x21', True), (3, '0x81', True), (7, '0x07', False)):
                self.assertEqual(get(img, p + 'IDLE')[0]['idle_ms'], '500')
                rst = get(img, p + 'SW_DEVRESET')[0]
                self.assertEqual((rst['delay_ms'], rst['altst'], rst['err'], rst['irr'], rst['secnr']),
                                 ('0', '0x00', '0x01', '0x01', '0x00'))
                tur = get(img, p + 'CMD_TUR')[0]
                self.assertEqual((tur['delay_ms'], tur['rc'], tur['err'], tur['sense'], tur['secnr'], tur['altst']),
                                 (str(d), 'check', '0x20', '2/04/01', '0x00', '0x50'))
                rs = get(img, p + 'CMD_REQ_STAT')[0]
                self.assertEqual((rs['rc'], rs['err'], rs['cdbst'], rs['secnr']), ('check', '0x20', '0x58', '0x00'))
                re_ = get(img, p + 'CMD_REQ_ERROR')[0]
                self.assertEqual((re_['data'], re_['cdbst']), ('f0000200000000000401', '0x18'))
                polls = get(img, p + 'CMD_POLL')
                self.assertEqual([(e['sense'], e['secnr']) for e in polls],
                                 [('6/28/00', final), ('none', final)] if medium else [('2/3a/00', final)])
                res = get(img, p + 'CMD_RESULT')[0]
                self.assertEqual((res['rc'], res['ready']), ('ok', '1') if medium else ('timeout', '0'))
                self.assertEqual(res['ready_ms'], '-')
        # Past detect_time the TUR itself gets the UNIT ATTENTION, REQ_STAT is READY at the park position.
        late = expect_sd.predict(self.card, images=[4], return_to=0, devreset_cmd=(200,),
                                 passthrough=['--no-throughput', '--no-seek'])
        _, le = compare.parse_record(late)
        lget = lambda probe: [e for e in le if e['probe'] == probe]
        self.assertEqual(lget('DC200_CMD_TUR')[0]['sense'], '6/28/00')
        self.assertTrue(lget('DC200_CMD_REQ_STAT')[0]['data'].startswith('012f01'))
        self.assertEqual([e['sense'] for e in lget('DC200_CMD_POLL')], ['none'])

    def test_compare_other_record_with_only_common(self):
        # a stock-like record: one probe, real timings, a different REQ_SES(2) answer
        want = ('img=3 probe=REQ_SES_2 cdb=150002000600000000000000 rc=check status=0x11 err=0x50 irr=0x03 bytes=0 '
                'cdbst=0x58 secnr=0x81 us=-')
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
