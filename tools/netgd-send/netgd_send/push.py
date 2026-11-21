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


def plan_push(image_path, title=None):
    """-> (kind, descriptor, [PushFile]).  Files are read from where they are; a rewritten .gdi is held in memory."""
    image_path = os.path.abspath(image_path)
    if not os.path.isfile(image_path):
        raise imgmod.ImageError("%s not found" % image_path)
    kind, desc, sources = imgmod.plan_image(image_path)
    files = []
    for target, src, text in sources:
        files.append(PushFile(target, None, text.encode("latin-1")) if text is not None else PushFile(target, src, None))
    if any(f.name.lower() == NAME_TXT for f in files):
        raise imgmod.ImageError("the image already has a file called %s: rename it" % NAME_TXT)
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


def first_free_slot(used):
    n = 2                                   # 01 is the menu itself
    while n in used:
        n += 1
    if n > 9999:
        raise imgmod.ImageError("no free numbered folder")
    return n


def _printer(out):
    def on_line(text, throttled):
        if not throttled:
            out(text)
    return on_line


def cmd_push(a, out=print):
    if a.slot not in (None, "auto") and a.name is not None:
        raise imgmod.ImageError("use either --slot or --name, not both")
    if a.mode not in jobmod.MODES:
        raise jobmod.JobError("mode must be one of %s" % (jobmod.MODES,))
    out("checking and hashing %s ..." % a.image)
    kind, desc, files = plan_push(a.image, a.title)
    total = sum(f.size for f in files)
    source = FileSource(files)
    try:
        try:
            cl = ngtp.Client(a.host, a.port, timeout=a.timeout)
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
            try:
                # the target folder
                if a.name is not None:
                    imgmod.validate_image_path(a.name)
                    folder = a.name
                elif a.slot in (None, "auto"):
                    r = cl.list("/", on_line=lambda t, th: None)
                    if not r.ok:
                        out("error: cannot list the card: %s" % r.error)
                        return jobmod.EXIT_FAILED
                    folder = imgmod.slot_folder(first_free_slot(used_slot_numbers(r.lines)))
                    out("using the free folder %s" % folder)
                else:
                    folder = imgmod.slot_folder(int(a.slot))
                manifest = Manifest(folder, kind, [FileEntry(f.size, f.crc, f.name) for f in files], desc)
                mtext = manifest.render()                # validates (duplicate names, sizes, ...)
                out("sending %d files, %d bytes to %s (mode %s)" % (len(files), total, folder, a.mode))
                res = cl.put(mtext, source, mode=jobmod.MODES.index(a.mode), force=a.force,
                             allow_fragmented=a.allow_fragmented, tag=TAG, on_line=on_line)
                code = _evaluate_put(res, manifest, out)
                if code != jobmod.EXIT_OK:
                    return code
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
                return jobmod.EXIT_INCOMPLETE
    finally:
        source.close()


def _evaluate_put(res, manifest, out):
    """The DONE status of the PUT, checked against the NGD: lines (same reading as `collect`)."""
    result = jobmod.parse_result("\n".join(res.lines) + "\n")
    st = result.puts.get(TAG)
    if not res.ok:
        if res.error in _RESUMABLE:
            out("RESULT: incomplete (%s; nothing is lost, run the same command again to resume)" % res.error)
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
    try:
        cl = ngtp.Client(a.host, a.port, timeout=a.timeout)
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
