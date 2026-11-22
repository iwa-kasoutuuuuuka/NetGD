#!/usr/bin/env python3
"""Predict NetGD's gdprobe record for a whole tools/mksdcard card (Python 3, standard library only).

    python expect_sd.py OUTDIR [--out FILE] [--label netgd-expected] [--images 3,4,5,6,7] [--return-to 2]
                               [--bin PATH-TO-expect-binary] [--no-throughput] [--no-seek] [...]

Runs `cargo run --example expect` (common/discimg) once per image folder and joins the sections into one
record, in the order a gdprobe switch run (switch=on, images=3,4,5,6,7, return_to=2) writes them:
SW_CMD / SW_POLL / SW_RESULT, then the battery, all tagged img=<folder number>.

  * A folder with a disc.gdi / *.gdi / *.cdi is mounted: the polls see the swap window (2/3A/00 no medium,
    2/04/01 becoming ready), then UNIT ATTENTION 6/28/00, then READY (stock GDEMU 6.0, D13).
  * A folder with no image (07) is accepted like stock does (D12): the same window, then no medium (2/3A/00)
    until gdprobe's switch timeout, and the battery under img=7 is the no-medium one (CHECK 0x11 / key 2, REQ_SES
    past 0 key 5, no reads).
  * A folder number not on the card (1..999) is accepted and mounts folder 01, like stock GDEMU 6.0
    (stock_20261122_09_missing-folder.txt, whatever GDEMU.ini reset_goto says): the same window and battery as a
    switch to 01 (no medium if 01 has no image or does not exist). Past 999 it is refused with ILLEGAL REQUEST
    (`52 82 n` -> status 0x11 err 0x50) and the previously mounted image (or no medium) stays; stock was not
    probed there.
  * img=0 (the image mounted at power-up, the dcload-ip disc) is not predicted.
  * --regpoll / --devreset 50,200 (gdprobe swap_regpoll=1 / swap_devreset=1 devreset_ms=50,200, R154): every
    switch to an image folder gets the command-free register poll (SW_REGPOLL), and after its battery the same
    folder is switched to again once per delay with a DEVICE RESET that many ms in (lines prefixed R<ms>_, then
    REQ_STAT / REQ_ERROR). The final return_to switch has neither. --window O,D (default 150,150) goes to expect.
    NetGD (R159) treats that re-switch as a no-op and re-detects on the reset (SECNR 0x00, the byte at rest
    detect_time later, ALTSTATUS 0x00, ERROR 0x01 -- --old-bitstream for a bitstream without ERROR bit 0 --, then
    UNIT ATTENTION).
  * --reswitch / --devreset-only (gdprobe seq 17 swap_reswitch=1 / devreset_only=1): after the battery (and the
    R<ms>_ runs) the same folder again with no reset (RS_ lines, a no-op), then a DEVICE RESET with no switch
    command (DR_ lines).
  * --devreset-cmd 10,50,100 (gdprobe seq 20 devreset_cmd=1 devreset_cmd_ms=...): after all that, once per delay:
    500 ms idle, a DEVICE RESET, a TUR / REQ_STAT / REQ_ERROR that many ms later, then TUR polls until READY (DC<ms>_
    lines). Inside the re-detect (detect_time, --window's second value) NetGD answers NOT READY 2/04/01 with SECNR
    0x00, then UNIT ATTENTION and READY (no medium: 2/3A/00 until gdprobe's 2 s give up).
  * ERROR is sticky (fix/head-pos-sticky-err): the register polls show the last CHECK's value, 0x01 after DEVICE
    RESET (--old-bitstream: 0x00 after every packet command that succeeds). REQ_STAT after the RS_ / R<ms>_ / DR_
    lines reports where the battery's last CD_READ left the head (RS_), or the park position after a reset.
Without --out the record goes to stdout. Everything not predictable (us, t_ms, polls, ...) is '-'.
"""
import argparse
import glob
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
CRATE = os.path.normpath(os.path.join(HERE, '..', '..', 'common', 'discimg'))


def find_image(folder):
    """disc.gdi, else any .gdi, else disc.cdi, else any .cdi; None for a folder with no image."""
    for pat in ('disc.gdi', '*.gdi', 'disc.cdi', '*.cdi'):
        hits = sorted(glob.glob(os.path.join(folder, pat)))
        if not hits:
            hits = sorted(glob.glob(os.path.join(folder, pat.upper())))
        if hits:
            return hits[0]
    return None


def numbered_folders(outdir):
    out = {}
    for name in os.listdir(outdir):
        if re.fullmatch(r'\d{1,3}', name) and os.path.isdir(os.path.join(outdir, name)):
            out[int(name)] = os.path.join(outdir, name)
    return out


def parse_images(spec):
    nums = []
    for part in spec.split(','):
        part = part.strip()
        if '-' in part:
            a, b = part.split('-', 1)
            nums.extend(range(int(a), int(b) + 1))
        elif part:
            nums.append(int(part))
    return nums


class Expect:
    def __init__(self, binary, passthrough):
        self.binary, self.passthrough = binary, passthrough

    def run(self, args):
        if self.binary:
            cmd = [self.binary] + args + self.passthrough
        else:
            cmd = ['cargo', 'run', '-q', '--example', 'expect', '--'] + args + self.passthrough
        r = subprocess.run(cmd, cwd=CRATE, capture_output=True, text=True)
        if r.returncode != 0:
            raise RuntimeError('expect failed (%d): %s\n%s' % (r.returncode, ' '.join(cmd), r.stderr.strip()))
        return r.stdout


def predict(outdir, images=None, return_to=None, label='netgd-expected', binary=None, passthrough=(),
            regpoll=False, devreset=(), reswitch=False, devreset_only=False, devreset_cmd=()):
    """Return the combined record text."""
    folders = numbered_folders(outdir)
    if images is None:
        images = sorted(n for n in folders if n >= 3)
    if return_to is None:
        return_to = 2 if 2 in folders else 0
    ex = Expect(binary or os.environ.get('NETGD_EXPECT_BIN'), list(passthrough))
    parts = ['# gdprobe version=1 kos=- label=%s stamp=predicted seq=0 dcload=0\n' % label,
             '# config switch=1 identify=0 images=%s return_to=%d\n' % (','.join(str(n) for n in images), return_to),
             '# predicted by tools/netgd-expect (NetGD firmware logic); img=0 (boot disc) is not predicted; timings are "-"\n']
    mounted = None      # the image file currently mounted, None = unknown (the boot disc) or no medium
    last_img = 0
    # --prev-check / --prev-secnr / --prev-err / --prev-head for the next section (cdbst= / secnr= / the sticky
    # ERROR in the register polls / where the last CD_READ left the head). Before the first section: ERROR 0x50,
    # what the img=0 battery leaves (its REQ_SES_3 on the boot disc is ILLEGAL REQUEST in every record).
    state = ['--prev-err', '0x50']
    keys = {'last_check': '--prev-check', 'secnr': '--prev-secnr', 'err': '--prev-err', 'head': '--prev-head'}

    def run(args):
        nonlocal state
        out = ex.run(args + state)
        keep, state = [], []
        for line in out.splitlines(True):
            if line.startswith('# expect-state '):
                for kv in line.split()[2:]:
                    k, v = kv.split('=', 1)
                    state += [keys[k], v]
            else:
                keep.append(line)
        return ''.join(keep)

    def switch_args(n, extra):
        nonlocal mounted
        folder = n if n in folders or not 1 <= n <= 999 else 1     # a missing number mounts 01 (stock)
        img = find_image(folders[folder]) if folder in folders else None
        args = ['--img', str(n), '--switch'] + extra
        if img:
            args += ['--image', img]
            mounted = img
        elif folder in folders or folder == 1:
            args.append('--empty-folder')       # accepted, no medium (D12; also a missing number with no 01)
            mounted = None
        else:
            args.append('--missing-folder')     # past 999: refused, the old image stays
            if mounted:
                args += ['--image', mounted, '--no-ua']
        return args

    def same_args(n, extra):
        # The folder just switched to, still mounted (gdprobe runs these only after an accepted switch).
        return ['--img', str(n)] + extra + (['--image', mounted] if mounted else ['--empty-folder'])

    for n in images:
        parts.append(run(switch_args(n, ['--regpoll'] if regpoll else [])))
        for d in devreset:
            parts.append(run(switch_args(n, ['--sw-only', '--devreset', str(d)])))
        if reswitch:
            parts.append(run(same_args(n, ['--reswitch'])))
        if devreset_only:
            parts.append(run(same_args(n, ['--devreset-only'])))
        for d in devreset_cmd:
            parts.append(run(same_args(n, ['--devreset-cmd', str(d)])))
        last_img = n
    if return_to > 0:
        parts.append(run(switch_args(return_to, ['--sw-only'])))
        last_img = return_to
    parts.append('img=%d probe=END abort=0\n' % last_img)
    return ''.join(parts)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('outdir', help='a tools/mksdcard output folder (01..07)')
    ap.add_argument('--out', help='write the record here instead of stdout')
    ap.add_argument('--label', default='netgd-expected')
    ap.add_argument('--images', help='folders to visit, like gdprobe images= (default: every numbered folder from 03 up)')
    ap.add_argument('--return-to', type=int, help='final switch back to this folder (default 2 if it exists, 0 = none)')
    ap.add_argument('--regpoll', action='store_true', help='gdprobe swap_regpoll=1: the register poll after each image switch')
    ap.add_argument('--devreset', help='gdprobe devreset_ms, e.g. 50,200: DEVICE RESET switches after each image')
    ap.add_argument('--reswitch', action='store_true', help='gdprobe swap_reswitch=1: RS_ re-switch after each image')
    ap.add_argument('--devreset-only', action='store_true', help='gdprobe devreset_only=1: DR_ DEVICE RESET after each image')
    ap.add_argument('--devreset-cmd', help='gdprobe devreset_cmd_ms, e.g. 10,50,100: DC<ms>_ DEVICE RESET + commands after each image')
    ap.add_argument('--bin', help='a built expect binary (default: cargo run --example expect; env NETGD_EXPECT_BIN)')
    args, extra = ap.parse_known_args(argv)
    if not os.path.isdir(args.outdir):
        print('error: %s is not a folder' % args.outdir, file=sys.stderr)
        return 2
    try:
        text = predict(args.outdir, parse_images(args.images) if args.images else None, args.return_to,
                       args.label, args.bin, extra, args.regpoll,
                       [int(x) for x in args.devreset.split(',')] if args.devreset else (),
                       args.reswitch, args.devreset_only,
                       [int(x) for x in args.devreset_cmd.split(',')] if args.devreset_cmd else ())   # unknown options (--no-seek, --tp-batches 8 ...) go to expect
    except (RuntimeError, OSError, ValueError) as e:
        print('error: %s' % e, file=sys.stderr)
        return 1
    if args.out:
        with open(args.out, 'w', encoding='utf-8', newline='\n') as f:
            f.write(text)
        print('wrote %s (%d lines)' % (args.out, text.count('\n')), file=sys.stderr)
    else:
        sys.stdout.write(text)
    return 0


if __name__ == '__main__':
    sys.exit(main())
