"""CLI: python -m netgd_send {prepare,collect,status,push,remote,discover} ...

`discover` (UDP) finds the menu without its IP address.  `prepare`/`collect`/`status` never talk to the Dreamcast: sending, running, logging and noexec detection
belong to the owner's dc_auto / dc_gui (design 7.1).  `push` and `remote` talk to the NetGD menu over a BBA
(docs/design/menu-app.md 8): the menu's transfer screen shows the IP address and the 4-digit code.
"""

import argparse
import sys

from . import __version__
from . import image as imgmod
from . import job as jobmod
from . import ngtp
from . import push as pushmod
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


def cmd_push(a):
    return pushmod.cmd_push(a)


def cmd_remote(a):
    return pushmod.cmd_remote(a)


def _slot_arg(v):
    if v == "auto":
        return v
    try:
        return int(v)
    except ValueError:
        raise argparse.ArgumentTypeError("--slot takes a number or auto")


def cmd_discover(a):
    return pushmod.cmd_discover(a)


def _add_discover_args(p):
    p.add_argument("--broadcast", default=ngtp.DISCOVER_BROADCAST,
                   help="where the discovery request goes (default %s; if it finds nothing give your network's broadcast "
                        "address, e.g. 192.168.1.255, or the console's IP)" % ngtp.DISCOVER_BROADCAST)
    p.add_argument("--discover-timeout", type=float, default=2.0, help="seconds to wait for answers (default 2)")


def _add_net_args(p):
    p.add_argument("--host", help="IP address shown on the menu's transfer screen (default: look for the menu with the UDP discovery)")
    p.add_argument("--code", required=True, type=int, help="the 4-digit code shown there")
    p.add_argument("--port", type=int, default=ngtp.DEFAULT_PORT)
    p.add_argument("--timeout", type=float, default=60.0, help="network timeout in seconds (default 60)")
    _add_discover_args(p)


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

    p = sub.add_parser("push", help="send an image to the NetGD menu's transfer screen over the BBA (no job folder)")
    p.add_argument("image", help=".gdi or .cdi file")
    _add_net_args(p)
    p.add_argument("--slot", type=_slot_arg, default=None,
                   help="numbered folder 01..9999, or `auto` for the lowest free one from 02 (default when no --name)")
    p.add_argument("--name", help="ASCII folder name on the SD card (up to 3 levels with '/')")
    p.add_argument("--mode", choices=jobmod.MODES, default="resume", help="put mode (default resume: a repeat goes on)")
    p.add_argument("--title", help="the title the menu shows (written as name.txt; default: the image file name)")
    p.add_argument("--mount", action="store_true", help="mount this image when the transfer is finished")
    p.add_argument("--force", action="store_true", help="allow overwriting a protected folder (the console asks too)")
    p.add_argument("--allow-fragmented", action="store_true", help="accept a fragmented allocation")
    p.add_argument("--no-finish", action="store_true", help="do not close the session at the end")
    p.set_defaults(func=cmd_push)

    p = sub.add_parser("discover", help="look for NetGD menus on the network (UDP); the menu's transfer screen must be open")
    p.add_argument("--port", type=int, default=ngtp.DEFAULT_PORT)
    _add_discover_args(p)
    p.set_defaults(func=cmd_discover)

    p = sub.add_parser("remote", help="one operation on the menu's transfer screen: statfs, list, delete, verify, finish")
    p.add_argument("op", choices=("statfs", "list", "delete", "verify", "finish"))
    p.add_argument("folder", nargs="?", help="folder for list (default /), delete and verify")
    _add_net_args(p)
    p.add_argument("--force", action="store_true", help="delete: allow a protected folder (the console asks too)")
    p.add_argument("--mount", metavar="FOLDER", help="finish: mount this folder")
    p.add_argument("--none", action="store_true", help="finish: mount nothing")
    p.set_defaults(func=cmd_remote)
    return ap


def main(argv=None):
    a = build_parser().parse_args(argv)
    if a.cmd == "push" and a.slot is None and a.name is None:
        a.slot = "auto"
    if a.cmd == "remote" and a.op in ("delete", "verify") and not a.folder:
        build_parser().error("%s needs a folder" % a.op)
    try:
        return a.func(a)
    except (imgmod.ImageError, ManifestError, jobmod.JobError, ngtp.NgtpError) as e:
        print("error: %s" % e, file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
