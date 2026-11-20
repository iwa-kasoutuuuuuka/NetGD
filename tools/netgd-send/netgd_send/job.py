"""T1 job folder (design 5.3): creation (`prepare`) and result interpretation (`collect`, `status`).

Layout under JOBDIR (dc-tool-ip serves it as /pc/...; ASCII, no spaces):

    job.ngj               operations, top to bottom
    NNNN/manifest.ngm     manifest of one image (design 5.1)
    NNNN/manifest.ngc     chunk checkpoints: cumulative CRC32 at every chunk boundary (PC-side aid)
    NNNN/src.txt          one /pc/ path per manifest `file` line, same order
    NNNN/<8.3 names>      hard links (or copies) of the track files + the rewritten disc.gdi
    cancel.req            created by the PC to stop the app (never created by this tool)
    result.ngr            written by the Dreamcast app
"""

import os
import re
import shutil

from . import image as imgmod
from .manifest import (DEFAULT_CHUNK_BYTES, FileEntry, Manifest, ManifestError, hash_file, parse_chunks,
                       parse_manifest, render_chunks, crc_hex)

JOB_MAGIC = "NETGD-JOB 1"
JOB_FILE = "job.ngj"
RESULT_FILE = "result.ngr"
MODES = ("new", "resume", "overwrite")
_PC_BAD = set(' \t"*?<>|:')

EXIT_OK = 0
EXIT_FAILED = 1
EXIT_INCOMPLETE = 2


class JobError(ValueError):
    pass


# ---------------------------------------------------------------- paths

def to_pc_path(path):
    """Windows path -> the path the Dreamcast uses: '/pc' + path without the drive letter.

    dc-tool-ip (Windows build) serves its current drive as /pc/, so the job folder and
    the files must be on that drive (the drive letter is dropped here).  ASCII, no spaces.
    """
    full = os.path.abspath(path)
    drive, rest = os.path.splitdrive(full)
    if drive.startswith("\\\\") or drive.startswith("//"):
        raise JobError("UNC path %s cannot be used under /pc/" % path)
    p = rest.replace("\\", "/")
    if not p.startswith("/"):
        p = "/" + p
    try:
        p.encode("ascii")
    except UnicodeEncodeError:
        raise JobError("path %s contains non-ASCII characters; /pc/ paths must be ASCII" % path)
    if any(ch in _PC_BAD or ord(ch) < 0x20 for ch in p):
        raise JobError("path %s contains a space or a character that is unsafe for /pc/" % path)
    return "/pc" + p


def _drive_of(path):
    """Drive letter (or UNC share) of an absolute path, case folded; '' on POSIX."""
    return os.path.splitdrive(os.path.abspath(path))[0].lower()


def _require_same_drive(src, dst):
    """/pc/ paths carry no drive letter: a source on another drive would name a different file (R89 C4)."""
    if _drive_of(src) != _drive_of(dst):
        raise JobError("%s is on another drive than the job folder (%s); /pc/ paths have no drive letter, so the "
                       "Dreamcast would read a different file.  Use --placement copy, or put the job folder on the "
                       "image's drive" % (src, _drive_of(dst) or "?"))


def _place(src, dst, placement):
    """Put `src` at `dst` in the job folder.  Returns the path the Dreamcast should read."""
    if placement == "direct":
        _require_same_drive(src, dst)
        return src
    if os.path.lexists(dst):
        try:
            if os.path.samefile(src, dst):
                return dst
        except OSError:
            pass
        os.remove(dst)
    if placement in ("auto", "link"):
        try:
            os.link(src, dst)
            return dst
        except OSError as e:
            if placement == "link":
                raise JobError("cannot hard-link %s (different drive?): %s" % (src, e))
            # auto: fall back to the original path when it is /pc/-safe and on the job drive
            _require_same_drive(src, dst)
            try:
                to_pc_path(src)
            except JobError:
                raise JobError("cannot hard-link %s into the job folder (%s) and its own path is "
                               "not /pc/-safe; put the image on the job drive or use --placement copy" % (src, e))
            return src
    if placement == "copy":
        shutil.copyfile(src, dst)
        return dst
    raise JobError("unknown placement %r" % placement)


# ---------------------------------------------------------------- job file

class Job(object):
    def __init__(self, puts=None, finish=None, others=None):
        self.puts = puts or []      # [[num_str, mode]]
        self.finish = finish        # string after 'finish ' or None
        self.others = others or []  # statfs / list / verify / delete lines, kept in order


def read_job(jobdir):
    path = os.path.join(jobdir, JOB_FILE)
    if not os.path.isfile(path):
        raise JobError("%s not found (run `prepare` first)" % path)
    with open(path, "rb") as f:
        lines = f.read().decode("utf-8").split("\n")
    if not lines or lines[0] != JOB_MAGIC:
        raise JobError("%s: missing %r header" % (path, JOB_MAGIC))
    job = Job()
    for ln in lines[1:]:
        if not ln:
            continue
        key, _, rest = ln.partition(" ")
        if key == "put":
            p = rest.split(" ")
            if not re.fullmatch(r"\d{4}", p[0]) or (len(p) > 1 and p[1] not in MODES):
                raise JobError("bad put line: %r" % ln)
            job.puts.append([p[0], p[1] if len(p) > 1 else "new"])
        elif key == "finish":
            job.finish = rest
        else:
            job.others.append(ln)
    return job


def write_job(jobdir, job):
    lines = [JOB_MAGIC] + job.others
    for num, mode in job.puts:
        lines.append("put %s %s" % (num, mode))
    lines.append("finish " + (job.finish or "default"))
    with open(os.path.join(jobdir, JOB_FILE), "wb") as f:
        f.write(("\n".join(lines) + "\n").encode("utf-8"))


# ---------------------------------------------------------------- prepare

def _read_result(jobdir):
    """The previous result.ngr parsed, or None (absent / unreadable)."""
    try:
        with open(os.path.join(jobdir, RESULT_FILE), "rb") as f:
            return parse_result(f.read().decode("utf-8", "replace"))
    except OSError:
        return None


class PrepareResult(object):
    def __init__(self, number, folder, manifest, files, mode, warnings):
        self.number, self.folder, self.manifest = number, folder, manifest
        self.files, self.mode, self.warnings = files, mode, warnings  # files: [(name, pc_path)]


def _find_entry(jobdir, folder):
    """Existing entry number for the same target folder, and the highest number in use."""
    best = 0
    found = None
    for d in sorted(os.listdir(jobdir)) if os.path.isdir(jobdir) else []:
        if re.fullmatch(r"\d{4}", d) and os.path.isfile(os.path.join(jobdir, d, "manifest.ngm")):
            best = max(best, int(d))
            try:
                with open(os.path.join(jobdir, d, "manifest.ngm"), "rb") as f:
                    if parse_manifest(f.read().decode("utf-8")).image == folder:
                        found = d
            except (ManifestError, OSError, UnicodeDecodeError):
                pass
    return found, best


def prepare(image_path, jobdir, slot=None, name=None, chunk_mib=8, mode="new", mount=False,
            placement="auto", statfs=True):
    if mode not in MODES:
        raise JobError("mode must be one of %s" % (MODES,))
    if not isinstance(chunk_mib, int) or chunk_mib < 1:
        raise JobError("--chunk-mib must be a positive integer")
    chunk_bytes = chunk_mib * 1024 * 1024
    warnings = []
    # DESIGN-GAP: the DC writes NETGD.CKP every 8 MiB and resumes only from multiples of 8 MiB
    # (5.2); another chunk size is accepted for the sidecar but the app would not use it.
    if chunk_bytes != DEFAULT_CHUNK_BYTES:
        warnings.append("chunk size %d MiB differs from the 8 MiB checkpoint interval of the design" % chunk_mib)

    image_path = os.path.abspath(image_path)
    if not os.path.isfile(image_path):
        raise imgmod.ImageError("%s not found" % image_path)
    kind = imgmod.detect_type(image_path)
    folder = imgmod.folder_name(slot, name, image_path)

    # 1. validate and plan names (nothing is written yet)
    sources = []  # (target_name, source_path or None, text or None)
    if kind == "gdi":
        gi = imgmod.check_gdi(image_path)
        by_track, gdi_text = imgmod.plan_gdi_names(gi)
        desc = imgmod.DESCRIPTOR_BASE + ".gdi"
        sources.append((desc, None, gdi_text))
        done = set()
        for t in gi.tracks:
            nm = by_track[t.number]
            if nm.lower() not in done:
                done.add(nm.lower())
                sources.append((nm, t.path, None))
    else:
        imgmod.check_cdi(image_path)
        desc = imgmod.DESCRIPTOR_BASE + ".cdi"
        sources.append((desc, image_path, None))

    # 2. job folder and entry number
    jobdir = os.path.abspath(jobdir)
    to_pc_path(jobdir)  # the job folder itself must be /pc/-safe
    os.makedirs(jobdir, exist_ok=True)
    existing, highest = _find_entry(jobdir, folder)
    num = existing or "%04d" % (highest + 1)
    if int(num) > 9999:
        raise JobError("too many entries in the job folder")
    edir = os.path.join(jobdir, num)
    os.makedirs(edir, exist_ok=True)

    # 3. place files, hash, build the manifest
    entries, srcs, cps_all = [], [], []
    for target, src, text in sources:
        dst = os.path.join(edir, target)
        if text is not None:
            with open(dst, "wb") as f:
                f.write(text.encode("latin-1"))
            read_from = dst
        else:
            read_from = _place(src, dst, placement)
        size, crc, cps = hash_file(read_from, chunk_bytes)
        entries.append(FileEntry(size, crc, target))
        # DESIGN-GAP: 5.3 says "/pc/ path" without saying whether the /pc prefix is included; we write it.
        srcs.append(to_pc_path(read_from))
        cps_all.append(cps)
    manifest = Manifest(folder, kind, entries, desc)
    mtext = manifest.render()

    with open(os.path.join(edir, "manifest.ngm"), "wb") as f:
        f.write(mtext.encode("utf-8"))
    with open(os.path.join(edir, "manifest.ngc"), "wb") as f:
        f.write(render_chunks(chunk_bytes, cps_all).encode("ascii"))
    with open(os.path.join(edir, "src.txt"), "wb") as f:
        f.write(("\n".join(srcs) + "\n").encode("ascii"))

    # 4. job.ngj: keep other entries, set this one.  Entries the previous run finished with DONE ok are dropped
    #    (R89 C1): run again they would be sent again (overwrite: the old image is erased and rewritten, new: EEXIST,
    #    resume: a full read-back).  Failed / unfinished / unreached entries stay, so a resume still finds them.
    job = read_job(jobdir) if os.path.isfile(os.path.join(jobdir, JOB_FILE)) else Job(others=["statfs"] if statfs else [])
    prev_res = _read_result(jobdir)
    if prev_res is not None:
        kept = []
        for p in job.puts:
            st = prev_res.puts.get(p[0])
            if p[0] != num and st is not None and st.done and not st.error:
                warnings.append("entry %s was sent successfully by the previous run: dropped from job.ngj" % p[0])
            else:
                kept.append(p)
        job.puts = kept
    for p in job.puts:
        if p[0] == num:
            p[1] = mode
            break
    else:
        job.puts.append([num, mode])
    if mount:
        job.finish = "mount " + folder
    write_job(jobdir, job)

    # 5. a new run: keep the previous result for reference, drop a stale cancel request
    res = os.path.join(jobdir, RESULT_FILE)
    if os.path.isfile(res):
        prev = os.path.join(jobdir, "result.prev.ngr")
        if os.path.exists(prev):
            os.remove(prev)
        os.replace(res, prev)
    # stale request files from an earlier run would change this one (R89 C2): cancel.req / reload.req stop it at
    # once, force.req / fragok.req silently lift protections.  Create force.req / fragok.req AFTER `prepare`.
    for req in ("cancel.req", "reload.req", "force.req", "fragok.req"):
        path = os.path.join(jobdir, req)
        if os.path.isfile(path):
            os.remove(path)
            if req != "cancel.req":
                warnings.append("removed stale %s from the job folder (create it again after `prepare` if you want it)" % req)

    return PrepareResult(num, folder, manifest, list(zip([e.name for e in entries], srcs)), mode, warnings)


# ---------------------------------------------------------------- result file

_KV = re.compile(r'(\w+)=("[^"]*"|\S+)')


def _kv(s):
    return {k: v.strip('"') for k, v in _KV.findall(s)}


class PutState(object):
    def __init__(self):
        self.progress = None   # (file_i, file_n, bytes, total)
        self.resume = None     # {'file','offset','crc'}
        self.done = False
        self.error = None      # {'code':..., ...}
        self.files = {}        # file index -> (size, crc)


class Result(object):
    def __init__(self):
        self.hello = None
        self.statfs = None
        self.puts = {}
        self.finish = None     # {'ok': bool, ...}
        self.end = None
        self.global_errors = []
        self.malformed = []
        self.truncated = False
        self.listing = []

    def put(self, num):
        return self.puts.setdefault(num, PutState())


def parse_result(text):
    """Parse result.ngr.  Tolerates a torn last line (the app writes 4 KiB at a time)."""
    r = Result()
    text = text.replace("\r\n", "\n")
    if text and not text.endswith("\n"):
        r.truncated = True
        text = text[: text.rfind("\n") + 1]
    for ln in text.split("\n"):
        if not ln.startswith("NGD:"):
            continue
        head, _, rest = ln[4:].partition(" ")
        try:
            if head == "HELLO":
                r.hello = _kv(rest)
            elif head == "STATFS":  # DESIGN-GAP: `statfs` output format
                r.statfs = _kv(rest)
            elif head == "LIST":
                r.listing.append(rest)
            elif head == "PUT":
                num, _, tail = rest.partition(" ")
                kv = _kv(tail)
                fi, fn = kv["file"].split("/")
                b, tot = kv["bytes"].split("/")
                r.put(num).progress = (int(fi), int(fn), int(b), int(tot))
            elif head == "RESUME":  # DESIGN-GAP: where a resumed put starts
                num, _, tail = rest.partition(" ")
                kv = _kv(tail)
                r.put(num).resume = {"file": int(kv["file"]), "offset": int(kv["offset"]),
                                     "crc": int(kv["crc"], 16) if "crc" in kv else None}
            elif head == "FILE":  # DESIGN-GAP: per-file result with CRC
                num, _, tail = rest.partition(" ")
                kv = _kv(tail)
                r.put(num).files[int(kv["file"])] = (int(kv["size"]), int(kv["crc"], 16))
            elif head == "DONE":
                num, _, tail = rest.partition(" ")
                if tail.strip() == "ok":
                    r.put(num).done = True
                else:
                    r.put(num).error = {"code": "EUNKNOWN", "raw": tail}
            elif head == "ERROR":
                tgt, _, tail = rest.partition(" ")
                kv = _kv(tail)
                if re.fullmatch(r"\d{4}", tgt):
                    r.put(tgt).error = kv or {"code": "EUNKNOWN"}
                else:
                    r.global_errors.append(kv or {"code": "EUNKNOWN"})
            elif head == "FINISH":
                st, _, tail = rest.partition(" ")
                r.finish = dict(_kv(tail), ok=(st == "ok"))
            elif head == "END":  # DESIGN-GAP: end marker
                r.end = _kv(rest)
            # other NGD: lines (progress chatter) are ignored
        except (KeyError, ValueError):
            r.malformed.append(ln)
    return r


# ---------------------------------------------------------------- evaluation

class PutReport(object):
    def __init__(self, num, folder):
        self.num, self.folder = num, folder
        self.state = "pending"   # pending | ok | failed | incomplete
        self.notes = []          # human-readable lines
        self.resumed_from = None  # (file name, offset)
        self.resume_point = None  # (file name, offset) to continue from next time


class Report(object):
    def __init__(self):
        self.puts = []
        self.finish_state = None  # None (not requested) | ok | failed | pending
        self.finish_note = ""
        self.warnings = []
        self.has_result = False
        self.exit_code = EXIT_OK

    def lines(self):
        out = []
        for p in self.puts:
            out.append("put %s %-10s %s" % (p.num, p.state.upper(), p.folder))
            if p.resumed_from:
                out.append("    resumed from %s @ %d" % p.resumed_from)
            for n in p.notes:
                out.append("    " + n)
            if p.resume_point and p.state in ("incomplete", "failed"):
                out.append("    next resume point: %s @ %d (run `prepare --mode resume`)" % p.resume_point)
        if self.finish_state:
            out.append("finish %s %s" % (self.finish_state.upper(), self.finish_note))
        for w in self.warnings:
            out.append("warning: " + w)
        out.append({EXIT_OK: "RESULT: success", EXIT_FAILED: "RESULT: FAILED",
                    EXIT_INCOMPLETE: "RESULT: incomplete (nothing is lost; resume with `prepare --mode resume`)"}[self.exit_code])
        return out


def _locate(manifest, total_bytes, chunk):
    """Total bytes written -> (file index 1-based, file name, checkpoint offset in that file)."""
    left = total_bytes
    for i, f in enumerate(manifest.files, 1):
        if left < f.size or i == len(manifest.files):
            off = min(left, f.size)
            return i, f.name, (off // chunk) * chunk if off < f.size else 0
        left -= f.size
    return 1, manifest.files[0].name, 0


def _load_entry(jobdir, num):
    edir = os.path.join(jobdir, num)
    try:
        with open(os.path.join(edir, "manifest.ngm"), "rb") as f:
            manifest = parse_manifest(f.read().decode("utf-8"))
    except (OSError, ManifestError, UnicodeDecodeError) as e:
        raise JobError("entry %s: manifest unreadable: %s" % (num, e))
    chunk, table = DEFAULT_CHUNK_BYTES, None
    try:
        with open(os.path.join(edir, "manifest.ngc"), "rb") as f:
            chunk, table = parse_chunks(f.read().decode("ascii"))
    except (OSError, ManifestError, ValueError):
        pass
    return manifest, chunk, table


def evaluate(jobdir):
    job = read_job(jobdir)
    rep = Report()
    rpath = os.path.join(jobdir, RESULT_FILE)
    result = None
    if os.path.isfile(rpath):
        with open(rpath, "rb") as f:
            result = parse_result(f.read().decode("utf-8", "replace"))
        rep.has_result = True
    else:
        rep.warnings.append("no %s yet: the Dreamcast app has not run, or died before writing it" % RESULT_FILE)

    for num, _mode in job.puts:
        manifest, chunk, table = _load_entry(jobdir, num)
        pr = PutReport(num, manifest.image)
        rep.puts.append(pr)
        st = result.puts.get(num) if result else None
        if st is None:
            if result and result.end is not None:
                pr.notes.append("not reached by the app")
            continue
        problems = []
        n = len(manifest.files)
        if st.progress:
            fi, fn, b, tot = st.progress
            if fn != n or tot != manifest.total_bytes():
                problems.append("progress says %d files / %d bytes, manifest has %d / %d" %
                                (fn, tot, n, manifest.total_bytes()))
        if st.resume:
            r = st.resume
            fidx, off = r["file"], r["offset"]
            if not 1 <= fidx <= n or off < 0 or off >= max(manifest.files[fidx - 1].size, 1):
                problems.append("resume point file=%s offset=%s is outside the manifest" % (fidx, off))
            elif off % chunk:
                problems.append("resume offset %d is not a multiple of the chunk size %d" % (off, chunk))
            else:
                if off and table is not None and r["crc"] is not None:
                    want = table.get((fidx, off))
                    if want is None:
                        problems.append("resume offset %d of file %d is not a checkpoint boundary" % (off, fidx))
                    elif want != r["crc"]:
                        problems.append("resume CRC %s at file %d offset %d != expected %s" %
                                        (crc_hex(r["crc"]), fidx, off, crc_hex(want)))
                pr.resumed_from = (manifest.files[fidx - 1].name, off)
        for fidx, (size, crc) in sorted(st.files.items()):
            if not 1 <= fidx <= n:
                problems.append("file result for index %d, manifest has %d files" % (fidx, n))
                continue
            f = manifest.files[fidx - 1]
            if size != f.size:
                problems.append("%s: app wrote %d bytes, manifest says %d" % (f.name, size, f.size))
            if crc != f.crc:
                problems.append("%s: CRC32 %s, manifest says %s" % (f.name, crc_hex(crc), crc_hex(f.crc)))
        if st.error:
            e = st.error
            pr.state = "failed"
            extra = " ".join("%s=%s" % (k, v) for k, v in sorted(e.items()) if k != "code")
            pr.notes.append("error %s %s" % (e.get("code", "?"), extra))
            if "file" in e:
                try:
                    fidx = int(e["file"])
                    off = int(e.get("offset", 0))
                    pr.resume_point = (manifest.files[fidx - 1].name, (off // chunk) * chunk)
                except (ValueError, IndexError):
                    pass
            elif st.progress:
                _i, nm, off = _locate(manifest, st.progress[2], chunk)
                pr.resume_point = (nm, off)
        # DESIGN-GAP: result.ngr "full content" is only sketched in 5.3; the collect rules below are ours.
        elif st.done:
            pr.state = "failed" if problems else "ok"
            if not problems:
                pr.notes.append("%d files, %d bytes" % (n, manifest.total_bytes()))
                if st.files:
                    pr.notes.append("app verified CRC32 of %d/%d files" % (len(st.files), n))
        else:
            pr.state = "incomplete"
            if st.progress:
                i, nm, off = _locate(manifest, st.progress[2], chunk)
                pr.notes.append("stopped at %d / %d bytes (file %s)" % (st.progress[2], st.progress[3], nm))
                pr.resume_point = (nm, off)
            else:
                pr.notes.append("started but no progress was recorded")
                pr.resume_point = (manifest.files[0].name, 0)
        for p in problems:
            pr.notes.append("PROBLEM: " + p)
        if problems and pr.state == "incomplete":
            pr.state = "failed"

    if job.finish and job.finish != "none":
        fin = result.finish if result else None
        if fin is None:
            rep.finish_state = "pending"
        elif fin["ok"]:
            rep.finish_state, rep.finish_note = "ok", ("mounted=%s" % fin["mounted"]) if "mounted" in fin else ""
        else:
            rep.finish_state, rep.finish_note = "failed", " ".join("%s=%s" % kv for kv in sorted(fin.items()) if kv[0] != "ok")

    if result:
        for e in result.global_errors:
            rep.warnings.append("app error: " + " ".join("%s=%s" % kv for kv in sorted(e.items())))
        if result.malformed:
            rep.warnings.append("%d malformed result line(s), first: %r" % (len(result.malformed), result.malformed[0]))
        if result.truncated:
            rep.warnings.append("result file ends with a torn line (ignored)")
        if result.end is None:
            rep.warnings.append("no NGD:END line: the app may have stopped before finishing")

    failed = any(p.state == "failed" for p in rep.puts) or rep.finish_state == "failed" \
        or bool(result and (result.global_errors or result.malformed))
    pending = any(p.state in ("pending", "incomplete") for p in rep.puts) or rep.finish_state == "pending"
    rep.exit_code = EXIT_FAILED if failed else (EXIT_INCOMPLETE if pending else EXIT_OK)
    if not rep.has_result:
        rep.exit_code = EXIT_INCOMPLETE
    return rep
