#!/usr/bin/env python3
"""Predict NetGD's gdprobe record for a whole tools/mksdcard card (Python 3, standard library only).

    python expect_sd.py OUTDIR [--out FILE] [--label netgd-expected] [--images 3,4,5,6,7] [--return-to 2]
                               [--bin PATH-TO-expect-binary] [--no-throughput] [--no-seek] [...]

Runs `cargo run --example expect` (common/discimg) once per image folder and joins the sections into one
record, in the order a gdprobe switch run (switch=on, images=3,4,5,6,7, return_to=2) writes them:
SW_CMD / SW_POLL / SW_RESULT, then the battery, all tagged img=<folder number>.

  * A folder with a disc.gdi / *.gdi / *.cdi is mounted: the first poll sees UNIT ATTENTION, the second READY.
  * A folder with no image (07) is refused with ILLEGAL REQUEST (`52 82 07` -> status 0x51 err 0x54) and the
    previously mounted image stays: the battery under img=7 is that image's answers again (UNIT ATTENTION
    already consumed). That is what NetGD does; a stock GDEMU may differ, which is what the diff is for.
  * img=0 (the image mounted at power-up, the dcload-ip disc) is not predicted.
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


def predict(outdir, images=None, return_to=None, label='netgd-expected', binary=None, passthrough=()):
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
    mounted = None      # the image file currently mounted, None = unknown (the boot disc)
    last_img = 0
    for n in images:
        img = find_image(folders[n]) if n in folders else None
        args = ['--img', str(n), '--switch']
        if img:
            parts.append(ex.run(args + ['--image', img]))
            mounted = img
        else:
            args.append('--empty-folder')
            if mounted:
                args += ['--image', mounted, '--no-ua']
            parts.append(ex.run(args))
        last_img = n
    if return_to > 0:
        img = find_image(folders[return_to]) if return_to in folders else None
        args = ['--img', str(return_to), '--switch', '--sw-only']
        args += ['--image', img] if img else ['--empty-folder']
        parts.append(ex.run(args))
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
    ap.add_argument('--bin', help='a built expect binary (default: cargo run --example expect; env NETGD_EXPECT_BIN)')
    args, extra = ap.parse_known_args(argv)
    if not os.path.isdir(args.outdir):
        print('error: %s is not a folder' % args.outdir, file=sys.stderr)
        return 2
    try:
        text = predict(args.outdir, parse_images(args.images) if args.images else None, args.return_to,
                       args.label, args.bin, extra)   # unknown options (--no-seek, --tp-batches 8 ...) go to expect
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
