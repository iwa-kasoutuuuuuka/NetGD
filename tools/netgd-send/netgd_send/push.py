"""`push` and `remote`: the PC side of the transfer to the NetGD menu over a BBA (docs/design/menu-app.md 8.4).

`push` checks the image (the same code as `prepare`), builds the manifest, adds `name.txt` (the title the menu
shows), connects to the menu's transfer screen with its pass code, sends PUT and answers the Dreamcast's requests
for file data straight from the original files.  Nothing is copied or linked into a job folder.
Exit codes as `collect`: 0 success, 2 incomplete (run the same command again: it resumes), 1 failed.
Not run against a real Dreamcast.
"""

import os
import re
import sys

from . import art as artmod
from . import image as imgmod
from . import job as jobmod
from . import ngtp
from .manifest import FileEntry, Manifest, ManifestError, crc32, hash_file

TAG = "0001"
NAME_TXT = "name.txt"
# a stop the PC can recover from by running the same command again
_RESUMABLE = ("ESTOP", "ESRC")


def ascii_title(text):
    """ASCII only (anything else becomes '?'), one line, at most 64 characters"""
    out = "".join(c if 0x20 <= ord(c) < 0x7F else "?" for c in " ".join(text.split()))
    return out[:64]


class PushFile(object):
    __slots__ = ("name", "path", "data", "size", "crc")

    def __init__(self, name, path, data):
        self.name, self.path, self.data = name, path, data
        if data is not None:
            self.size, self.crc = len(data), crc32(data)
        else:
            self.size, self.crc, _cps = hash_file(path)


def sibling_name_txt(image_path):
    d = os.path.dirname(os.path.abspath(image_path))
    for f in sorted(os.listdir(d)):
        if f.lower() == NAME_TXT and os.path.isfile(os.path.join(d, f)):
            return os.path.join(d, f)
    return None


def plan_push(image_path, title=None, art=None):
    """-> (kind, descriptor, [PushFile]).  Files are read from where they are; a rewritten .gdi is held in memory.
    art: the bytes of a NETGD.ART (netgd_send.art.make_art) to put next to the image, or None."""
    image_path = os.path.abspath(image_path)
    if not os.path.isfile(image_path):
        raise imgmod.ImageError("%s not found" % image_path)
    kind, desc, sources = imgmod.plan_image(image_path)
    files = []
    for target, src, text in sources:
        files.append(PushFile(target, None, text.encode("latin-1")) if text is not None else PushFile(target, src, None))
    if any(f.name.lower() == NAME_TXT for f in files):
        raise imgmod.ImageError("the image already has a file called %s: rename it" % NAME_TXT)
    if art is not None:
        if any(f.name.lower() == artmod.FILE_NAME for f in files):
            raise imgmod.ImageError("the image already has a file called %s: rename it" % artmod.FILE_NAME)
        files.append(PushFile(artmod.FILE_NAME, None, art))
    if title is not None:
        files.append(PushFile(NAME_TXT, None, (ascii_title(title) + "\n").encode("ascii")))
    else:
        own = sibling_name_txt(image_path)
        if own:
            files.append(PushFile(NAME_TXT, own, None))
        else:
            stem = os.path.splitext(os.path.basename(image_path))[0]
            files.append(PushFile(NAME_TXT, None, (ascii_title(stem) + "\n").encode("ascii")))
    return kind, desc, files


class FileSource(object):
    """answers the Dreamcast's WANT frames from the original files"""

    def __init__(self, files):
        self.files = files
        self._open = {}

    def read(self, idx, off, n):
        f = self.files[idx]
        if f.data is not None:
            return f.data[off:off + n]
        h = self._open.get(idx)
        if h is None:
            h = self._open[idx] = open(f.path, "rb")
        h.seek(off)
        return h.read(n)

    def close(self):
        for h in self._open.values():
            h.close()
        self._open = {}


_LIST_LINE = re.compile(r"^NGD:LIST \w+ (.*)$")


def used_slot_numbers(lines):
    """the numbers of the numbered folders (2 to 4 digits, value >= 1) a `list /` reply shows"""
    used = set()
    for ln in lines:
        m = _LIST_LINE.match(ln)
        if not m:
            continue
        top = m.group(1).split("/")[0]
        if re.fullmatch(r"\d{2,4}", top) and int(top) >= 1:
            used.add(int(top))
    return used


_LIST_STATE_LINE = re.compile(r"^NGD:LIST (\w+) (.*)$")


def partial_slot_numbers(lines):
    """the numbers (ascending, 02 and up) of the top-level numbered folders a `list /` reply calls "partial": an
    interrupted transfer left NETGD.CKP or a *.NGP there (R104 finding 2)"""
    found = set()
    for ln in lines:
        m = _LIST_STATE_LINE.match(ln)
        if m and m.group(1) == "partial" and re.fullmatch(r"\d{2,4}", m.group(2)) and int(m.group(2)) >= 2:
            found.add(int(m.group(2)))
    return sorted(found)


def first_free_slot(used):
    n = 2                                   # 01 is the menu itself
    while n in used:
        n += 1
    if n > 9999:
        raise imgmod.ImageError("no free numbered folder")
    return n


def rerun_command(a, host, folder_number):
    """the command line that goes on with the same transfer: the options of this run, with --slot pinned to the folder"""
    parts = ["netgd-send", "push", '"%s"' % a.image]
    if host:
        parts += ["--host", host]
    if a.port != ngtp.DEFAULT_PORT:
        parts += ["--port", str(a.port)]
    parts += ["--code", "%04d" % a.code, "--slot", "%02d" % folder_number if folder_number < 100 else str(folder_number)]
    if a.title is not None:
        parts += ["--title", '"%s"' % a.title]
    if a.art:
        parts += ["--art", '"%s"' % a.art]
    for flag, on in (("--mount", a.mount), ("--force", a.force), ("--allow-fragmented", a.allow_fragmented),
                     ("--no-finish", a.no_finish)):
        if on:
            parts.append(flag)
    return " ".join(parts)


def _printer(out):
    def on_line(text, throttled):
        if not throttled:
            out(text)
    return on_line


def resolve_host(a, out):
    """a.host, or the one menu a discovery finds.  Returns the address, or None after printing why not."""
    if a.host:
        return a.host
    out("looking for a NetGD menu on the network (UDP %d, %.0f s) ..." % (a.port, a.discover_timeout))
    found = ngtp.discover(a.discover_timeout, targets=(a.broadcast,), port=a.port)
    if not found:
        out("error: no menu answered. Open the transfer screen (Y) and give the IP address with --host.")
        return None
    if len(found) > 1:
        out("error: %d menus answered: %s. Choose one with --host." % (len(found), ", ".join(f.host for f in found)))
        return None
    f = found[0]
    out("found %s at %s (app %s, %s)" % (f.name or "a menu", f.host, f.app or "?", f.state or "?"))
    if f.state == "busy":
        out("warning: it says it is busy: another PC may be connected")
    return f.host


def cmd_discover(a, out=print):
    """list the menus that answer (exit 0 when there is at least one)"""
    found = ngtp.discover(a.discover_timeout, targets=(a.broadcast,), port=a.port)
    for f in found:
        out("%s  %s  app %s  %s" % (f.host, f.name or "?", f.app or "?", f.state or "?"))
    if not found:
        out("no menu answered (is the transfer screen open? is the BBA on this network?)")
    return jobmod.EXIT_OK if found else jobmod.EXIT_FAILED


def cmd_push(a, out=print):
    if a.slot not in (None, "auto") and a.name is not None:
        raise imgmod.ImageError("use either --slot or --name, not both")
    if a.mode not in jobmod.MODES:
        raise jobmod.JobError("mode must be one of %s" % (jobmod.MODES,))
    out("checking and hashing %s ..." % a.image)
    art = artmod.make_art(a.art) if a.art else None
    kind, desc, files = plan_push(a.image, a.title, art)
    total = sum(f.size for f in files)
    host = resolve_host(a, out)
    if host is None:
        return jobmod.EXIT_FAILED
    source = FileSource(files)
    try:
        try:
            cl = ngtp.Client(host, a.port, timeout=a.timeout)
        except ngtp.ConnectionLost as e:
            out("error: %s" % e)
            return jobmod.EXIT_FAILED
        with cl:
            try:
                info = cl.hello(a.code)
            except ngtp.NgtpError as e:
                out("error: the menu refused the connection: %s" % e)
                return jobmod.EXIT_FAILED
            out("connected: firmware %s, menu %s%s" % (info.firmware, info.app, " (session open)" if info.session_open else ""))
            on_line = _printer(out)
            state = {"folder": None, "number": None}
            try:
                # the target folder
                auto = a.name is None and a.slot in (None, "auto")

                def attempt(folder, number=None):
                    state["folder"], state["number"] = folder, number
                    manifest = Manifest(folder, kind, [FileEntry(f.size, f.crc, f.name) for f in files], desc)
                    mtext = manifest.render()            # validates (duplicate names, sizes, ...)
                    out("sending %d files, %d bytes to %s (mode %s)" % (len(files), total, folder, a.mode))
                    res = cl.put(mtext, source, mode=jobmod.MODES.index(a.mode), force=a.force,
                                 allow_fragmented=a.allow_fragmented, tag=TAG, on_line=on_line)
                    return res, manifest

                res = manifest = None
                if a.name is not None:
                    imgmod.validate_image_path(a.name)
                    res, manifest = attempt(a.name)
                elif not auto:
                    n = int(a.slot)
                    res, manifest = attempt(imgmod.slot_folder(n), n)
                else:
                    r = cl.list("/", on_line=lambda t, th: None)
                    if not r.ok:
                        out("error: cannot list the card: %s" % r.error)
                        return jobmod.EXIT_FAILED
                    # An earlier run that was interrupted left a partial numbered folder: go on there before taking a new one
                    # (R104 finding 2).  Only in resume mode: new and overwrite must never be pointed at an old folder.
                    if a.mode == "resume":
                        for n in partial_slot_numbers(r.lines):
                            folder = imgmod.slot_folder(n)
                            out("an unfinished folder %s is there: trying to resume it" % folder)
                            res, manifest = attempt(folder, n)
                            if not res.ok and res.error == "EEXIST":
                                out("folder %s holds a different image: going on to the next" % folder)
                                res = None
                                continue
                            break
                    if res is None:
                        n = first_free_slot(used_slot_numbers(r.lines))
                        out("using the free folder %s" % imgmod.slot_folder(n))
                        res, manifest = attempt(imgmod.slot_folder(n), n)
                rerun = rerun_command(a, host, state["number"]) if state["number"] is not None else None
                code = _evaluate_put(res, manifest, out, rerun)
                if code != jobmod.EXIT_OK:
                    return code
                folder = state["folder"]
                if not a.no_finish:
                    fin = cl.finish(ngtp.FINISH_MOUNT if a.mount else ngtp.FINISH_DEFAULT, folder if a.mount else "",
                                    on_line=on_line)
                    if not fin.ok:
                        out("error: FINISH failed: %s" % fin.error)
                        return jobmod.EXIT_FAILED
                out("RESULT: success")
                return jobmod.EXIT_OK
            except ngtp.NgtpError as e:
                out("error: the Dreamcast refused: %s" % e)
                return jobmod.EXIT_FAILED
            except ngtp.ConnectionLost as e:
                out("connection lost: %s" % e)
                out("RESULT: incomplete (nothing is lost; run the same command again to resume)")
                if state["number"] is not None:
                    out("to go on with exactly this folder: " + rerun_command(a, host, state["number"]))
                return jobmod.EXIT_INCOMPLETE
    finally:
        source.close()


def _evaluate_put(res, manifest, out, rerun=None):
    """The DONE status of the PUT, checked against the NGD: lines (same reading as `collect`)."""
    result = jobmod.parse_result("\n".join(res.lines) + "\n")
    st = result.puts.get(TAG)
    if not res.ok:
        if res.error in _RESUMABLE:
            out("RESULT: incomplete (%s; nothing is lost, run the same command again to resume)" % res.error)
            if rerun:
                out("to go on with exactly this folder: " + rerun)
            return jobmod.EXIT_INCOMPLETE
        out("RESULT: FAILED (%s)" % res.error)
        return jobmod.EXIT_FAILED
    problems = []
    if st is not None:
        for idx, (size, crc) in sorted(st.files.items()):
            if not 1 <= idx <= len(manifest.files):
                problems.append("file result for index %d" % idx)
                continue
            f = manifest.files[idx - 1]
            if size != f.size or crc != f.crc:
                problems.append("%s: the app says %d bytes CRC32 %08X, the manifest says %d bytes CRC32 %08X"
                                % (f.name, size, crc, f.size, f.crc))
    if problems:
        for p in problems:
            out("PROBLEM: " + p)
        out("RESULT: FAILED")
        return jobmod.EXIT_FAILED
    return jobmod.EXIT_OK


def cmd_remote(a, out=print):
    on_line = _printer(out)
    host = resolve_host(a, out)
    if host is None:
        return jobmod.EXIT_FAILED
    try:
        cl = ngtp.Client(host, a.port, timeout=a.timeout)
    except ngtp.ConnectionLost as e:
        out("error: %s" % e)
        return jobmod.EXIT_FAILED
    with cl:
        try:
            cl.hello(a.code)
            if a.op == "statfs":
                res = cl.statfs(on_line=on_line)
            elif a.op == "list":
                res = cl.list(a.folder or "/", on_line=on_line)
            elif a.op == "delete":
                res = cl.delete(a.folder, force=a.force, on_line=on_line)
            elif a.op == "verify":
                res = cl.verify(a.folder, on_line=on_line)
            else:
                mode = ngtp.FINISH_MOUNT if a.mount else (ngtp.FINISH_NONE if a.none else ngtp.FINISH_DEFAULT)
                res = cl.finish(mode, a.mount or "", on_line=on_line)
        except ngtp.NgtpError as e:
            out("error: %s" % e)
            return jobmod.EXIT_FAILED
        except ngtp.ConnectionLost as e:
            out("connection lost: %s" % e)
            return jobmod.EXIT_INCOMPLETE
    if not res.ok:
        out("failed: %s" % res.error)
        return jobmod.EXIT_FAILED
    return jobmod.EXIT_OK
