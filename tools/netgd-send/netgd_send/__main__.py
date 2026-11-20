"""CLI: python -m netgd_send {prepare,collect,status} ...

This tool never talks to the Dreamcast.  Sending, running, logging and noexec detection
belong to the owner's dc_auto / dc_gui (design 7.1).
"""

import argparse
import sys

from . import __version__
from . import image as imgmod
from . import job as jobmod
from .manifest import ManifestError


def _human(n):
    for unit in ("B", "KiB", "MiB", "GiB"):
        if n < 1024 or unit == "GiB":
            return ("%d %s" % (n, unit)) if unit == "B" else "%.1f %s" % (n, unit)
        n /= 1024.0


def cmd_prepare(a):
    r = jobmod.prepare(a.image, a.job, slot=a.slot, name=a.name, chunk_mib=a.chunk_mib, mode=a.mode,
                       mount=a.mount, placement=a.placement)
    m = r.manifest
    print("job folder : %s" % a.job)
    print("entry      : %s  (put mode: %s)" % (r.number, r.mode))
    print("SD folder  : %s   type: %s   %d files, %s" % (m.image, m.type, len(m.files), _human(m.total_bytes())))
    for f in m.files:
        print("  %-12s %12d  CRC32 %08X" % (f.name, f.size, f.crc))
    print("manifest CRC32: %08X" % m.crc())
    for w in r.warnings:
        print("warning: " + w)
    print("next: send netgd-recv.elf with dc_auto (preset netgd.ps1), then `collect --job %s`" % a.job)
    return 0


def cmd_collect(a):
    rep = jobmod.evaluate(a.job)
    for ln in rep.lines():
        print(ln)
    return rep.exit_code


def cmd_status(a):
    job = jobmod.read_job(a.job)
    rep = jobmod.evaluate(a.job)
    print("job %s: %d put(s), finish %s" % (a.job, len(job.puts), job.finish or "default"))
    for (num, mode), p in zip(job.puts, rep.puts):
        m, chunk, _t = jobmod._load_entry(a.job, num)
        print("  %s  %-10s mode=%-9s %s (%s, %d files, %s)" % (num, p.state.upper() if rep.has_result else "PENDING",
                                                              mode, m.image, m.type, len(m.files), _human(m.total_bytes())))
        for n in p.notes:
            print("      " + n)
        if p.resume_point and p.state in ("incomplete", "failed"):
            print("      resume point: %s @ %d" % p.resume_point)
    if not rep.has_result:
        print("no result.ngr yet")
    for w in rep.warnings:
        if rep.has_result:
            print("warning: " + w)
    return 0


def build_parser():
    ap = argparse.ArgumentParser(prog="netgd_send", description=__doc__.strip().splitlines()[0])
    ap.add_argument("--version", action="version", version="netgd-send " + __version__)
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("prepare", help="validate an image and create/extend a job folder")
    p.add_argument("image", help=".gdi or .cdi file")
    p.add_argument("--job", required=True, help="job folder, on the drive dc-tool-ip runs from (ASCII, no spaces)")
    g = p.add_mutually_exclusive_group()
    g.add_argument("--slot", type=int, help="numbered folder 01..9999 (GDmenu/openMenu compatible)")
    g.add_argument("--name", help="ASCII folder name on the SD card (up to 3 levels with '/')")
    p.add_argument("--chunk-mib", type=int, default=8, help="checkpoint interval in MiB (design: 8)")
    p.add_argument("--mode", choices=jobmod.MODES, default="new", help="put mode (default new)")
    p.add_argument("--mount", action="store_true", help="mount this image when the job finishes")
    p.add_argument("--placement", choices=("auto", "link", "copy", "direct"), default="auto",
                   help="how files get into the job folder (default auto: hard link)")
    p.set_defaults(func=cmd_prepare)

    p = sub.add_parser("collect", help="read result.ngr and report; exit 0 only on full success")
    p.add_argument("--job", required=True)
    p.set_defaults(func=cmd_collect)

    p = sub.add_parser("status", help="show what is pending / done / resumable")
    p.add_argument("--job", required=True)
    p.set_defaults(func=cmd_status)
    return ap


def main(argv=None):
    a = build_parser().parse_args(argv)
    try:
        return a.func(a)
    except (imgmod.ImageError, ManifestError, jobmod.JobError) as e:
        print("error: %s" % e, file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
