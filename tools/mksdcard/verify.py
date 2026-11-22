#!/usr/bin/env python3
"""Check a generated card against NetGD's own parsers: runs the discimg `inspect` example on every
image of OUTDIR and compares the track table with NETGD-PROBE-MANIFEST.json.

    python verify.py OUTDIR [--cargo-dir common/discimg]
Exit 0 = all tables match, 1 = mismatch, 2 = could not run."""
import argparse
import json
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_CRATE = os.path.join(HERE, '..', '..', 'common', 'discimg')


def parse_inspect(text):
    rows = []
    for line in text.splitlines():
        m = re.match(r'track (\d+) lba=(\d+) ctrl=(\d+) ssize=(\d+) count=(\d+) offset=(\d+) file=(\S+)', line)
        if m:
            rows.append(tuple(int(x) if i < 6 else x for i, x in enumerate(m.groups())))
    return rows


def expected_rows(entry):
    rows = []
    for t in entry['tracks']:
        rows.append((t['number'], t['start_lba'], t['ctrl'], t['sector_size'], t['parsed_sector_count'],
                     t['file_offset'], t['file'] if entry['format'] == 'gdi' else '-'))
    return rows


def check_mode1_cdi(path, entry):
    """Image 10 (R137 g): the CDI's data track must hold raw 2352-byte Mode 1 sectors (sync, mode byte 1 at
    offset 15, the generator's user data at 16). The parser table alone cannot tell Mode 1 from Mode 2."""
    sys.path.insert(0, HERE)
    import mksdcard
    problems = []
    with open(path, 'rb') as f:
        for t in entry['tracks']:
            if t.get('form') != 'cdi_mode1_2352':
                continue
            for i in (0, 1, t['sector_count'] - 1):
                f.seek(t['file_offset'] + i * 2352)
                s = f.read(2352)
                lba = t['start_lba'] + i
                if s[0:12] != mksdcard.SYNC or s[15] != 1:
                    problems.append('track %d sector %d: no Mode 1 sync/header (mode byte %d)' % (t['number'], i, s[15]))
                elif s != mksdcard.sector_for(t, lba, 2352)[0]:
                    problems.append('track %d sector %d: differs from the generator' % (t['number'], i))
    return problems


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument('outdir')
    ap.add_argument('--crate', default=DEFAULT_CRATE)
    a = ap.parse_args(argv)
    with open(os.path.join(a.outdir, 'NETGD-PROBE-MANIFEST.json')) as f:
        man = json.load(f)
    bad = 0
    for n, e in sorted(man['images'].items()):
        path = os.path.abspath(os.path.join(a.outdir, n, e['file']))
        r = subprocess.run(['cargo', 'run', '-q', '--example', 'inspect', '--', path], cwd=a.crate,
                           capture_output=True, text=True)
        print('== %s %s' % (n, e['file']))
        print(r.stdout.rstrip())
        if r.returncode != 0:
            print(r.stderr.rstrip())
            bad += 1
            continue
        got, want = parse_inspect(r.stdout), expected_rows(e)
        if got != want:
            bad += 1
            print('MISMATCH\n  parser:   %s\n  manifest: %s' % (got, want))
        else:
            extra = check_mode1_cdi(path, e) if e['format'] == 'cdi' else []
            if extra:
                bad += 1
                print('MISMATCH (Mode 1 content)\n  ' + '\n  '.join(extra))
            else:
                print('OK (matches manifest)' + (' + Mode 1 sectors' if any(t.get('form') == 'cdi_mode1_2352' for t in e['tracks']) else ''))
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
