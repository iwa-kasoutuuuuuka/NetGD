#!/usr/bin/env python3
"""fwbackup - read-only check/backup of a stock GDEMU 6.0 MCU firmware over SWD.

Target MCU: Atmel ATSAM3U2E (128 KB flash, single plane at 0x00080000-0x0009FFFF).
Probe: Raspberry Pi Pico running debugprobe (CMSIS-DAP) driven by probe-rs.

SAFETY MODEL (the stock firmware cannot be restored if harmed)
  * This tool only ever runs the probe-rs subcommands `list`, `info`, `read`
    (and the bare `--version`).  Every probe-rs invocation goes through
    ProbeRs.run(), which validates the argument vector against a strict
    allowlist *before* spawning anything (see validate_args()).
  * The only memory addresses `read` may touch are the flash window and the
    8 bytes of CHIPID_CIDR/CHIPID_EXID.
  * PROBE_RS_* environment variables are scrubbed from the child environment,
    because clap reads PROBE_RS_CONNECT_UNDER_RESET / PROBE_RS_ALLOW_ERASE_ALL /
    PROBE_RS_CYCLE_POWER from the environment (common_options.rs, `env = ...`).
  * No ERASE pin, no reset pin, no --connect-under-reset: only SWDIO/SWCLK/GND.
  * probe-rs also reads `.probe-rs.{toml,json,yaml,yml}` (cwd, executable dir,
    home dir); a `[presets.default]` there could silently add options such as
    connect-under-reset.  ProbeRs.run() refuses to run if any such file exists
    and runs probe-rs from a fresh empty temp directory.
  * Only the reviewed probe-rs version (REVIEWED) is accepted.

probe-rs facts this code is built on (source: probe-rs master, v0.32.0 at the
time of writing; see README_ja.md for URLs):
  * cmd/read.rs   : `probe-rs read [probe opts] <b8|b16|b32|b64> <ADDR> <WORDS>`;
                    default --format is hex-table; stdout lines are
                    "{address:08x}: " + words separated by one space, each word
                    printed most-significant byte first (bytes of the LE value
                    reversed); b32 = 32 bytes (8 words) per line.
  * session attach: NOT free of side effects on the running core.  Every attach
                    halts the core briefly, twice (Session::new ->
                    clear_all_hw_breakpoints -> halted_access, and again on
                    drop), single-steps on resume, then clears DHCSR
                    debug-enable and writes DEMCR=0.  No reset, no flash / EEFC
                    / RAM writes.  A full dump attaches ~260 times, so run it
                    with the Dreamcast sitting at the BIOS menu (no disc
                    access); if the target stays stopped, power-cycle it.
  * cmd/list.rs   : "The following debug probes were found:" then "[N]: <probe>"
                    lines, or "No debug probes were found."
  * cmd/info.rs   : with --verbose prints "Probing target via SWD" and a tree
                    whose root is "Debug Port: DPv1, Designer: ..." when the DP
                    answers; without --verbose prints "Detected chip: <name>".
                    Errors go to stderr and the process exits non-zero (anyhow).
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import zlib
from datetime import datetime, timezone
from pathlib import Path

# --------------------------------------------------------------------------
# Constants (SAM3U datasheet 6430G: 8.2 memory map, 28 CHIPID)
# --------------------------------------------------------------------------
CHIPID_CIDR_ADDR = 0x400E0740
CHIPID_EXID_ADDR = 0x400E0744
CIDR_SAM3U2E = 0x281A0761
CIDR_SAM3U4E = 0x28100961
KNOWN_SAM3U = {
    0x28000961: "AT91SAM3U4C (Rev A)",
    0x280A0761: "AT91SAM3U2C (Rev A)",
    0x28090561: "AT91SAM3U1C (Rev A)",
    0x28100961: "AT91SAM3U4E (Rev A)",
    0x281A0761: "AT91SAM3U2E (Rev A)",
    0x28190561: "AT91SAM3U1E (Rev A)",
}
FLASH_BASE = 0x00080000
FLASH_SIZE = 0x00020000  # 128 KB
FLASH_END = FLASH_BASE + FLASH_SIZE
DEFAULT_CHIP = "ATSAM3U2E"
DEFAULT_CHUNK = 4096
# Upper bounds inclusive for the initial SP (stack top == end of SRAM is legal).
SRAM_RANGES = [(0x20000000, 0x20008000), (0x2007C000, 0x20084000), (0x20080000, 0x20084000)]

ST_UNLOCKED_2E = "UNLOCKED_2E"
ST_UNLOCKED_OTHER = "UNLOCKED_OTHER"
ST_LOCKED = "LOCKED_OR_NO_ACCESS"
ST_NO_TARGET = "NO_TARGET"
ST_NO_PROBE = "NO_PROBE"
ST_NO_PROBE_RS = "NO_PROBE_RS"
ST_MULTI_PROBE = "MULTIPLE_PROBES"
ST_BAD_CHIP = "BAD_CHIP_NAME"
ST_OLD_PROBE_RS = "OLD_PROBE_RS"
ST_UNREVIEWED_PROBE_RS = "UNREVIEWED_PROBE_RS"
# probe-rs versions whose source was safety-reviewed for this tool.
REVIEWED = ("0.32.0",)
CIDR_VERSION_MASK = 0x1F  # CHIPID_CIDR low 5 bits = VERSION (revision) field
EXIT_CODES = {
    ST_UNLOCKED_2E: 0,
    ST_UNLOCKED_OTHER: 10,
    ST_LOCKED: 11,
    ST_NO_TARGET: 12,
    ST_NO_PROBE: 2,
    ST_NO_PROBE_RS: 2,
    ST_MULTI_PROBE: 2,
    ST_BAD_CHIP: 2,
    ST_OLD_PROBE_RS: 2,
    ST_UNREVIEWED_PROBE_RS: 2,
}


# --------------------------------------------------------------------------
# Allowlist wrapper around probe-rs
# --------------------------------------------------------------------------
class SafetyRefusal(Exception):
    """Raised when an argument vector is not on the read-only allowlist."""


FORBIDDEN_WORDS = {
    "download", "erase", "write", "run", "reset", "attach", "gdb", "flash",
    "serve", "debug", "trace", "itm", "benchmark", "profile", "verify",
    "chip-erase", "test", "dap-server", "cargo-flash", "cargo-embed", "write-ap",
}
FORBIDDEN_FLAGS = {
    "--connect-under-reset", "--allow-erase-all", "--cycle-power", "--output", "-o",
    "--chip-erase", "--erase-all", "--preverify", "--verify", "--restore-unwritten",
    "--format", "-f", "--chip-description-path", "--preset", "--host", "--token",
}
ALLOWED_SUBCOMMANDS = {"list", "info", "read"}
_PROBE_SEL = re.compile(r"[0-9A-Fa-f]{4}:[0-9A-Fa-f]{4}(:[0-9A-Za-z_.\-]+)?")
_CHIP = re.compile(r"ATSAM3U[0-9A-Z]{1,3}")
_INT = re.compile(r"[0-9]{1,5}")
CONFIG_NAMES = tuple(f".probe-rs.{e}" for e in ("toml", "json", "yaml", "yml"))
_WIDTHS = {"b8": 1, "b16": 2, "b32": 4, "b64": 8}
# (start, end-exclusive) windows a `read` may touch.
READ_WINDOWS = [
    (FLASH_BASE, FLASH_END),
    (CHIPID_CIDR_ADDR, CHIPID_EXID_ADDR + 4),
]
# option -> kind of value ("flag" = no value)
_COMMON_OPTS = {"--probe": "probe", "--protocol": "swd", "--speed": "int", "--non-interactive": "flag"}
_OPTS = {
    "list": {},
    "info": {**_COMMON_OPTS, "--verbose": "flag", "-v": "flag"},
    "read": {**_COMMON_OPTS, "--chip": "chip"},
}


def _split_eq(arg: str):
    if arg.startswith("--") and "=" in arg:
        k, v = arg.split("=", 1)
        return k, v
    return arg, None


def validate_args(args: list[str]) -> None:
    """Raise SafetyRefusal unless `args` is a read-only probe-rs invocation."""
    if not args:
        raise SafetyRefusal("empty probe-rs argument list")
    if args == ["--version"]:
        return
    for a in args:
        k, _ = _split_eq(a)
        if k in FORBIDDEN_FLAGS:
            raise SafetyRefusal(f"forbidden probe-rs option: {a}")
        if a in FORBIDDEN_WORDS:
            raise SafetyRefusal(f"forbidden probe-rs subcommand/word: {a}")
    sub = args[0]
    if sub not in ALLOWED_SUBCOMMANDS:
        raise SafetyRefusal(f"probe-rs subcommand not allowed: {sub!r} (allowed: list, info, read)")
    opts = _OPTS[sub]
    i = 1
    positionals: list[str] = []
    while i < len(args):
        a = args[i]
        k, v = _split_eq(a)
        if k.startswith("-"):
            if k not in opts:
                raise SafetyRefusal(f"option not allowed for `{sub}`: {a}")
            kind = opts[k]
            if kind == "flag":
                if v is not None:
                    raise SafetyRefusal(f"flag takes no value: {a}")
            else:
                if v is None:
                    i += 1
                    if i >= len(args):
                        raise SafetyRefusal(f"option {k} needs a value")
                    v = args[i]
                _check_opt_value(k, kind, v)
        else:
            positionals.append(a)
        i += 1
    if sub in ("list", "info"):
        if positionals:
            raise SafetyRefusal(f"unexpected arguments for `{sub}`: {positionals}")
        return
    # read: exactly  <width> <address> <words>
    if len(positionals) != 3:
        raise SafetyRefusal("read needs exactly: <b8|b16|b32|b64> <address> <words>")
    width, addr_s, words_s = positionals
    if width not in _WIDTHS:
        raise SafetyRefusal(f"bad read width: {width!r}")
    if not re.fullmatch(r"0x[0-9A-Fa-f]{1,8}", addr_s) or not re.fullmatch(r"[0-9]{1,6}", words_s):
        raise SafetyRefusal(f"bad read address/words: {addr_s!r} {words_s!r}")
    addr, words = int(addr_s, 16), int(words_s)
    nbytes = words * _WIDTHS[width]
    if words < 1 or addr % _WIDTHS[width]:
        raise SafetyRefusal("read size/alignment invalid")
    if not any(lo <= addr and addr + nbytes <= hi for lo, hi in READ_WINDOWS):
        raise SafetyRefusal(f"read outside the allowed windows (flash / CHIPID): {addr_s} x{words_s}")


def _check_opt_value(k: str, kind: str, v: str) -> None:
    ok = True
    if kind == "probe":
        ok = bool(_PROBE_SEL.fullmatch(v))
    elif kind == "swd":
        ok = v == "swd"
    elif kind == "int":
        ok = bool(_INT.fullmatch(v))
    elif kind == "chip":
        ok = bool(_CHIP.fullmatch(v))
    if not ok:
        raise SafetyRefusal(f"bad value for {k}: {v!r}")


class RunResult:
    def __init__(self, rc: int, out: str, err: str):
        self.rc, self.out, self.err = rc, out, err


class ProbeRs:
    """The ONLY place that spawns probe-rs."""

    def __init__(self, cmd: list[str] | None = None, timeout: float = 90.0, log=None):
        self.cmd = cmd if cmd is not None else self._discover()
        self.timeout = timeout
        self.log = log  # optional callable(list[str])

    @staticmethod
    def _discover() -> list[str] | None:
        env = os.environ.get("FWBACKUP_PROBE_RS")
        if env:
            env = env.strip()
            if env.startswith("["):
                # a JSON list could smuggle unvalidated leading arguments
                if os.environ.get("FWBACKUP_TEST_WRAPPER") != "1":
                    print("[fwbackup] FWBACKUP_PROBE_RS には probe-rs 実行ファイルのパスだけを指定してください "
                          "(JSON リスト形式はテスト専用です)。", file=sys.stderr)
                    sys.exit(2)
                return [str(x) for x in json.loads(env)]
            return [os.path.abspath(env)]
        exe = shutil.which("probe-rs")
        return [exe] if exe else None

    @property
    def available(self) -> bool:
        return bool(self.cmd)

    def _exe_dirs(self) -> list[Path]:
        dirs = []
        for part in (self.cmd or []):
            exe = shutil.which(part) or part
            try:
                ep = Path(exe).resolve()
                if ep.is_file():
                    dirs.append(ep.parent)
            except OSError:
                continue
        return dirs

    def find_config_files(self) -> list[Path]:
        """`.probe-rs.*` files probe-rs would load (cwd, home, executable dir)."""
        cands: list[Path] = []
        try:
            cands.append(Path.cwd())
        except OSError:
            pass
        try:
            cands.append(Path.home())
        except (RuntimeError, KeyError):
            pass
        for var in ("USERPROFILE", "HOME"):
            if os.environ.get(var):
                cands.append(Path(os.environ[var]))
        cands += self._exe_dirs()
        found: list[Path] = []
        seen = set()
        for d in cands:
            for n in CONFIG_NAMES:
                f = d / n
                try:
                    key = os.path.normcase(str(f.resolve()))
                    if f.exists() and key not in seen:
                        seen.add(key)
                        found.append(f)
                except OSError:
                    continue
        return found

    def run(self, args: list[str]) -> RunResult:
        validate_args(args)  # raises SafetyRefusal BEFORE anything is spawned
        cfg = self.find_config_files()
        if cfg:
            raise SafetyRefusal(
                "probe-rs の設定ファイルが見つかりました。[presets.default] などで意図しないオプション "
                "(例: connect-under-reset) が暗黙に追加されるおそれがあるため実行しません。"
                "移動または削除してください: " + ", ".join(str(f) for f in cfg))
        if not self.cmd:
            raise FileNotFoundError("probe-rs not found")
        if self.log:
            self.log(list(args))
        env = {k: v for k, v in os.environ.items() if not k.upper().startswith("PROBE_RS_")}
        try:
            # fresh empty cwd so a cwd-level .probe-rs.* can never apply
            with tempfile.TemporaryDirectory(prefix="fwbackup-cwd-") as empty:
                p = subprocess.run(
                    self.cmd + list(args), capture_output=True, timeout=self.timeout, env=env,
                    stdin=subprocess.DEVNULL, cwd=empty,
                )
        except subprocess.TimeoutExpired:
            return RunResult(124, "", "fwbackup: probe-rs timed out")
        return RunResult(p.returncode, p.stdout.decode("utf-8", "replace"), p.stderr.decode("utf-8", "replace"))


# --------------------------------------------------------------------------
# probe-rs output parsing
# --------------------------------------------------------------------------
class ParseError(Exception):
    pass


_ADDR_LINE = re.compile(r"^([0-9A-Fa-f]{8}):\s*(.*)$")


def parse_read_output(text: str, base: int, nbytes: int, width: int = 4) -> bytes:
    """Parse `probe-rs read` stdout (hex-table, also bare simple-hex).

    Format (read.rs write_hex_table/write_simple_hex): each line is
    "{addr:08x}: " + words joined by " "; a word is the little-endian value
    printed as big-endian hex, so bytes are reversed back here.  Lines that
    are not made purely of hex words of the expected width are treated as log
    noise and ignored; the final length check catches anything lost.
    """
    data = bytearray()
    hexw = width * 2
    for raw in text.splitlines():
        line = raw.strip()
        if not line:
            continue
        m = _ADDR_LINE.match(line)
        addr = None
        rest = line
        if m:
            addr, rest = int(m.group(1), 16), m.group(2)
        toks = rest.split()
        if not toks or not all(len(t) == hexw and re.fullmatch(r"[0-9A-Fa-f]+", t) for t in toks):
            continue  # noise
        if addr is not None and addr != base + len(data):
            raise ParseError(f"address discontinuity: line says {addr:#010x}, expected {base + len(data):#010x}")
        for t in toks:
            data += int(t, 16).to_bytes(width, "little")
    if len(data) != nbytes:
        raise ParseError(f"expected {nbytes} bytes, parsed {len(data)}")
    return bytes(data)


def parse_list_output(text: str) -> list[dict]:
    """`probe-rs list` (cmd/list.rs): "[N]: <name> -- VID:PID:SERIAL (type)"."""
    probes = []
    for line in text.splitlines():
        m = re.match(r"^\s*\[(\d+)\]:\s*(.*)$", line)
        if m:
            desc = m.group(2)
            vp = re.search(r"\b([0-9A-Fa-f]{4}):([0-9A-Fa-f]{4})(?::([^\s)]+))?", desc)
            probes.append({
                "index": int(m.group(1)), "desc": desc.strip(),
                "vidpid": f"{vp.group(1)}:{vp.group(2)}" if vp else None,
                "inaccessible": "(inaccessible)" in desc,
            })
    return probes


_DP_VISIBLE = re.compile(r"Debug Port:\s*DPv\d|ARM Chip with debug port|Detected chip:", re.I)


def dp_visible(info_text: str) -> bool:
    return bool(_DP_VISIBLE.search(info_text))


# --------------------------------------------------------------------------
# Probe-level operations
# --------------------------------------------------------------------------
class ReadFailed(Exception):
    def __init__(self, rc: int, msg: str):
        super().__init__(f"probe-rs rc={rc}: {msg[-500:]}")
        self.rc, self.msg = rc, msg


def _opts(chip: str | None, probe: str | None) -> list[str]:
    a = ["--protocol", "swd", "--non-interactive"]
    if probe:
        a += ["--probe", probe]
    if chip:
        a += ["--chip", chip]
    return a


def read_block(pr: ProbeRs, addr: int, nbytes: int, chip: str, probe: str | None) -> bytes:
    if nbytes % 4 or addr % 4:
        raise ValueError("reads are 32-bit aligned")
    words = nbytes // 4
    r = pr.run(["read"] + _opts(chip, probe) + ["b32", f"0x{addr:08x}", str(words)])
    if r.rc != 0:
        raise ReadFailed(r.rc, (r.err or r.out).strip())
    try:
        return parse_read_output(r.out, addr, nbytes, 4)
    except ParseError as e:
        raise ReadFailed(r.rc, f"unparseable probe-rs output: {e}")


def probe_rs_version(pr: ProbeRs) -> str:
    try:
        r = pr.run(["--version"])
        txt = (r.out or r.err).strip()
        return txt.splitlines()[0] if txt else "unknown"
    except SafetyRefusal:
        raise
    except Exception:
        return "unknown"


def version_number(line: str) -> str | None:
    m = re.match(r"probe-rs\s+v?(\S+)", line.strip())
    return m.group(1) if m else None


_OLD_RE = re.compile(r"unexpected argument|unrecognized", re.I)


def select_probe(probes: list[dict], sel: str | None) -> dict | None:
    if sel:
        for p in probes:
            if sel.lower() in p["desc"].lower():
                return p
        return None
    ok = [p for p in probes if not p["inaccessible"]]
    return ok[0] if ok else None


def classify(pr: ProbeRs, chip: str, probe: str | None) -> dict:
    """Run the read-only diagnosis.  Returns {status, cidr, exid, detail...}."""
    res: dict = {"chip": chip, "probe_sel": probe}
    if not pr.available:
        res["status"] = ST_NO_PROBE_RS
        return res
    res["probe_rs_version"] = probe_rs_version(pr)
    if version_number(res["probe_rs_version"]) not in REVIEWED:
        res["status"] = ST_UNREVIEWED_PROBE_RS
        return res
    r = pr.run(["list"])
    probes = parse_list_output(r.out)
    res["probes"] = probes
    if r.rc != 0 and _OLD_RE.search(r.out + "\n" + r.err):
        res["status"] = ST_OLD_PROBE_RS
        return res
    if r.rc != 0 or not probes:
        res["status"] = ST_NO_PROBE
        res["detail"] = (r.err or r.out).strip()[-300:]
        return res
    if all(p["inaccessible"] for p in probes):
        res["status"] = ST_NO_PROBE
        res["detail"] = "見つかったプローブはすべて (inaccessible) です (WinUSB ドライバ未設定の可能性)。"
        return res
    res["selected_probe"] = (select_probe(probes, probe) or {}).get("desc")
    if not probe and len([p for p in probes if not p["inaccessible"]]) > 1:
        res["status"] = ST_MULTI_PROBE
        return res
    try:
        blk = read_block(pr, CHIPID_CIDR_ADDR, 8, chip, probe)
        cidr = int.from_bytes(blk[0:4], "little")
        exid = int.from_bytes(blk[4:8], "little")
        res.update(cidr=cidr, exid=exid)
        if cidr not in (0, 0xFFFFFFFF):
            if cidr == CIDR_SAM3U2E:
                res["status"] = ST_UNLOCKED_2E
            else:
                res["status"] = ST_UNLOCKED_OTHER
                res["known_name"] = KNOWN_SAM3U.get(cidr)
                if (cidr & ~CIDR_VERSION_MASK) == (CIDR_SAM3U2E & ~CIDR_VERSION_MASK):
                    res["known_name"] = (f"ATSAM3U2E だがリビジョンが違う (VERSION フィールド 0x{cidr & CIDR_VERSION_MASK:02X})。"
                                         "安全側に倒して dump は拒否します")
            return res
        res["detail"] = f"CIDR read as {cidr:#010x} (not a valid chip id)"
    except ReadFailed as e:
        res["detail"] = e.msg[-400:]
        if _OLD_RE.search(e.msg):
            res["status"] = ST_OLD_PROBE_RS
            return res
        if re.search(r"(chip|target).{0,40}(not found|unknown|not (a )?(valid|supported))|no (such )?(chip|target)|"
                     r"Specified chip|could not find chip", e.msg, re.I) and not dp_visible(e.msg):
            res["status"] = ST_BAD_CHIP
            return res
    # read failed or returned junk: is the debug port alive?
    sel = ["--probe", probe] if probe else []
    iv = pr.run(["info", "--verbose", "--protocol", "swd", "--non-interactive"] + sel)
    text = iv.out + "\n" + iv.err
    if _OLD_RE.search(text):  # no fallback to plain `info` (it auto-detects targets)
        res["status"] = ST_OLD_PROBE_RS
        return res
    res["info_excerpt"] = text.strip()[-1200:]
    res["status"] = ST_LOCKED if dp_visible(text) else ST_NO_TARGET
    return res


MSG = {
    ST_UNLOCKED_2E: (
        "判定: 読み出し可能 (UNLOCKED_2E)\n"
        "  CHIPID_CIDR = 0x281A0761 (ATSAM3U2E)。セキュリティビット (GPNVM0) は立っておらず、SWD でフラッシュを読めます。\n"
        "  次の手順: 何かを書き込む前に `python fwbackup.py dump --out <保存先>` でバックアップを取ってください。"
    ),
    ST_UNLOCKED_OTHER: (
        "判定: 読み出しは可能だが ATSAM3U2E ではない (UNLOCKED_OTHER)\n"
        "  別の SAM3U (または別の MCU) です。この出力を Claude に送ってください。\n"
        "  このツールは 128 KB (ATSAM3U2E) 用の dump しか許可しません (4E は 256 KB で 2 バンクのため別対応が必要)。"
    ),
    ST_LOCKED: (
        "判定: デバッグポートは見えるが読めない (LOCKED_OR_NO_ACCESS)\n"
        "  SWD の DP は応答していますが、CHIPID/フラッシュを読めません。セキュリティビット (GPNVM0) が立っている可能性が高いです\n"
        "  (立っていると ICE 経由のフラッシュ/SRAM/コアレジスタ/周辺レジスタのアクセスがすべて禁止されます)。\n"
        "  次の手順: ここで止めてください。ERASE ピンは触らないこと (純正ファームが消え、二度と戻りません)。\n"
        "  純正を捨てる判断をした場合だけ ERASE を検討します。この check の出力を Claude に送ってください。"
    ),
    ST_NO_TARGET: (
        "判定: ターゲットが応答しない (NO_TARGET)\n"
        "  プローブは見えていますが、SWD で何も返ってきません。確認:\n"
        "   - Dreamcast の電源が入っているか (基板は Dreamcast から給電。プローブからは給電しない)\n"
        "   - GND / SWDIO / SWCLK の配線 (入れ替わっていないか、はんだ不良がないか)\n"
        "   - SAM3U の JTAGSEL ピンがローか (High だと SWD でなく JTAG になる)\n"
        "   - 他のプローブ・ソフトが同じプローブを掴んでいないか\n"
        "   - 配線も電源も確実に正しいのに NO_TARGET のままなら、セキュリティビットで SWD 自体が無効になっている可能性もあります\n"
        "     (配線不良と区別がつきません)。その場合も ERASE ピンには触らないでください。\n"
        "  次の手順: 配線を直して再度 check。それでも駄目ならこの出力を Claude に送ってください。"
    ),
    ST_NO_PROBE: (
        "判定: デバッグプローブが見つからない (NO_PROBE)\n"
        "  `probe-rs list` に何も出ません。Pico を USB に挿し、debugprobe (>= 2.2.3, CMSIS-DAP v2) のファームが入っているか、\n"
        "  Windows のドライバ (WinUSB; Zadig 等) が必要か確認してください。`(inaccessible)` と出る場合は WinUSB ドライバが未設定です。"
    ),
    ST_NO_PROBE_RS: (
        "判定: probe-rs が見つからない (NO_PROBE_RS)\n"
        "  PATH に probe-rs がありません。https://probe.rs/docs/getting-started/installation/ から入れてください。\n"
        "  (別の場所にある場合は環境変数 FWBACKUP_PROBE_RS にフルパスを設定)"
    ),
    ST_MULTI_PROBE: (
        "判定: プローブが複数見つかった (MULTIPLE_PROBES)\n"
        "  `--probe VID:PID[:SERIAL]` で 1 つ指定してください。上の一覧の VID:PID を使います。"
    ),
    ST_OLD_PROBE_RS: (
        "判定: probe-rs が古い (OLD_PROBE_RS)\n"
        "  probe-rs が必要なオプションを受け付けませんでした。probe-rs 0.32.0 を入れてください (このバージョンだけが安全確認済み)。"
    ),
    ST_UNREVIEWED_PROBE_RS: (
        "判定: 未確認の probe-rs バージョン (UNREVIEWED_PROBE_RS)\n"
        "  安全確認 (ソースレビュー) を行ったのは probe-rs 0.32.0 だけです。ちょうど 0.32.0 を入れてください。\n"
        "  (上書きオプションはありません。)"
    ),
    ST_BAD_CHIP: (
        "判定: チップ名を probe-rs が受け付けない (BAD_CHIP_NAME)\n"
        "  probe-rs のバージョンが古い可能性があります (`probe-rs chip list` に ATSAM3U2E があるか確認)。"
    ),
}


def cmd_check(args, pr: ProbeRs) -> int:
    res = classify(pr, args.chip, args.probe)
    st = res["status"]
    print(f"[fwbackup] probe-rs: {res.get('probe_rs_version', '(not found)')}")
    for p in res.get("probes", []):
        print(f"[fwbackup] probe [{p['index']}]: {p['desc']}")
    if "cidr" in res:
        print(f"[fwbackup] CHIPID_CIDR = 0x{res['cidr']:08X}   CHIPID_EXID = 0x{res['exid']:08X}")
    if st == ST_UNLOCKED_OTHER:
        print(f"[fwbackup] 検出した ID: 0x{res['cidr']:08X} = {res.get('known_name') or '既知の SAM3U 表に無い ID'}")
    if res.get("detail"):
        print(f"[fwbackup] 詳細: {res['detail']}")
    if st in (ST_LOCKED, ST_NO_TARGET) and res.get("info_excerpt"):
        print("[fwbackup] probe-rs info の出力 (末尾):\n" + res["info_excerpt"])
    print()
    print(MSG[st])
    print(f"\nRESULT: {st}")
    return EXIT_CODES[st]


# --------------------------------------------------------------------------
# dump
# --------------------------------------------------------------------------
def chunk_plan(base: int, size: int, chunk: int) -> list[tuple[int, int]]:
    if chunk <= 0 or chunk % 4:
        raise ValueError("chunk must be a positive multiple of 4")
    out, off = [], 0
    while off < size:
        n = min(chunk, size - off)
        out.append((base + off, n))
        off += n
    return out


def _vote(reads: list[bytes]) -> bytes | None:
    counts: dict[bytes, int] = {}
    for r in reads:
        counts[r] = counts.get(r, 0) + 1
    ranked = sorted(counts.items(), key=lambda kv: -kv[1])
    top = ranked[0]
    second = ranked[1][1] if len(ranked) > 1 else 0
    if top[1] >= 3 and second <= 1:
        return top[0]
    return None


class DumpUnstable(Exception):
    pass


def read_pass(pr, plan, chip, probe, retries, say, label) -> list[bytes]:
    out = []
    for i, (addr, n) in enumerate(plan):
        last = None
        for attempt in range(retries + 1):
            try:
                out.append(read_block(pr, addr, n, chip, probe))
                break
            except ReadFailed as e:
                last = e
                say(f"  {label} chunk {i} @0x{addr:08x}: read failed (try {attempt + 1}/{retries + 1}): {e.msg[-120:]}")
                time.sleep(0.2)
        else:
            raise last  # type: ignore[misc]
        if (i + 1) % 8 == 0 or i + 1 == len(plan):
            say(f"  {label}: {i + 1}/{len(plan)} chunks")
    return out


def dump_image(pr, plan, chip, probe, retries=3, max_rereads=5, say=print):
    """Two full passes + targeted re-reads.  Returns (data, info). Raises DumpUnstable."""
    p1 = read_pass(pr, plan, chip, probe, retries, say, "pass 1")
    p2 = read_pass(pr, plan, chip, probe, retries, say, "pass 2")
    final: list[bytes] = []
    rereads: dict[int, int] = {}
    mismatches = [i for i in range(len(plan)) if p1[i] != p2[i]]
    for i in range(len(plan)):
        if p1[i] == p2[i]:
            final.append(p1[i])
            continue
        addr, n = plan[i]
        say(f"  MISMATCH chunk {i} @0x{addr:08x}: re-reading up to {max_rereads} times")
        reads = [p1[i], p2[i]]
        chosen = None
        for k in range(max_rereads):
            reads.append(read_block(pr, addr, n, chip, probe))
            rereads[i] = k + 1
            chosen = _vote(reads)
            if chosen is not None:
                break
        if chosen is None:
            raise DumpUnstable(f"chunk {i} @0x{addr:08x} did not stabilise after {max_rereads} re-reads "
                               f"({len(set(reads))} distinct values). Check wiring/power; nothing was written.")
        final.append(chosen)
    return b"".join(final), {"plan": plan, "passes": 2, "mismatched_chunks": mismatches,
                              "rereads": {str(k): v for k, v in rereads.items()}, "chunks": final}


def _rename_noreplace(src: Path, dst: Path) -> None:
    """Rename that fails if `dst` exists (os.rename on Windows; link+unlink elsewhere)."""
    if os.name == "nt":
        os.rename(src, dst)
    else:
        os.link(src, dst)
        os.unlink(src)


def cmd_dump(args, pr: ProbeRs) -> int:
    if args.chunk % 4 or not 256 <= args.chunk <= 65536:
        print("[fwbackup] --chunk は 4 の倍数で 256〜65536 にしてください。", file=sys.stderr)
        return 2
    out_dir = Path(args.out)
    res = classify(pr, args.chip, args.probe)
    st = res["status"]
    if st != ST_UNLOCKED_2E:
        print(f"[fwbackup] dump を中止: check の結果が {st} です (UNLOCKED_2E のときだけ dump します)。", file=sys.stderr)
        print(MSG[st], file=sys.stderr)
        return EXIT_CODES[st] or 1
    cidr, exid = res["cidr"], res["exid"]
    plan = chunk_plan(FLASH_BASE, FLASH_SIZE, args.chunk)
    t0 = datetime.now(timezone.utc)
    print(f"[fwbackup] CIDR=0x{cidr:08X}. フラッシュ 0x{FLASH_BASE:08X}-0x{FLASH_END - 1:08X} を "
          f"{len(plan)} チャンク x2 回読みます (読み取り専用)。")
    try:
        data, info = dump_image(pr, plan, args.chip, args.probe, retries=args.retries,
                                max_rereads=args.max_rereads, say=lambda s: print(s, file=sys.stderr))
    except (DumpUnstable, ReadFailed) as e:
        print(f"[fwbackup] FAILED (何も書き出していません): {e}", file=sys.stderr)
        return 3
    # the chip must still be the same chip after the whole run
    try:
        again = read_block(pr, CHIPID_CIDR_ADDR, 8, args.chip, args.probe)
        if int.from_bytes(again[:4], "little") != cidr:
            print("[fwbackup] FAILED: dump 後に CIDR が変わった。書き出しません。", file=sys.stderr)
            return 3
    except ReadFailed as e:
        print(f"[fwbackup] FAILED: dump 後の CIDR 再読み出しに失敗: {e}", file=sys.stderr)
        return 3
    assert len(data) == FLASH_SIZE
    out_dir.mkdir(parents=True, exist_ok=True)
    stamp = t0.strftime("%Y%m%dT%H%M%SZ")
    stem = f"gdemu60_flash_{cidr:08X}_{stamp}"
    bin_p, sha_p, js_p = (out_dir / f"{stem}{e}" for e in (".bin", ".sha256", ".json"))
    part_p = out_dir / f"{stem}.bin.part"
    for p in (bin_p, sha_p, js_p, part_p):
        if p.exists():
            print(f"[fwbackup] 既存ファイルがあるので上書きしません: {p}", file=sys.stderr)
            return 4
    digest = hashlib.sha256(data).hexdigest()
    meta = {
        "tool": "fwbackup", "cidr": f"0x{cidr:08X}", "exid": f"0x{exid:08X}", "chip": args.chip,
        "flash_base": f"0x{FLASH_BASE:08X}", "flash_size": FLASH_SIZE, "chunk_size": args.chunk,
        "probe_rs_version": res.get("probe_rs_version"), "probe": res.get("selected_probe"), "probe_selector": args.probe,
        "started_utc": t0.isoformat(), "finished_utc": datetime.now(timezone.utc).isoformat(),
        "read_passes": info["passes"], "mismatched_chunks": info["mismatched_chunks"], "rereads": info["rereads"],
        "sha256": digest, "file": bin_p.name,
        "chunk_crc32": [{"addr": f"0x{a:08X}", "len": n, "crc32": f"{zlib.crc32(c) & 0xFFFFFFFF:08x}"}
                        for (a, n), c in zip(info["plan"], info["chunks"])],
    }
    try:
        with open(part_p, "xb") as f:
            f.write(data)
            f.flush()
            os.fsync(f.fileno())
        if hashlib.sha256(part_p.read_bytes()).hexdigest() != digest:
            raise OSError("書き込み後の再読み込みで SHA-256 が一致しません")
        _rename_noreplace(part_p, bin_p)
    except OSError as e:
        print(f"[fwbackup] FAILED: .bin の書き出しに失敗 ({e})。", file=sys.stderr)
        try:
            part_p.unlink()
        except OSError:
            pass
        return 3
    with open(sha_p, "x", encoding="utf-8") as f:
        f.write(f"{digest} *{bin_p.name}\n")
    with open(js_p, "x", encoding="utf-8") as f:
        json.dump(meta, f, indent=2)
    print(f"[fwbackup] OK: {bin_p}\n           sha256 {digest}")
    print("[fwbackup] このファイルは非公開に保管。コミット/アップロード禁止 (README_ja.md 参照)。")
    return 0


# --------------------------------------------------------------------------
# inspect
# --------------------------------------------------------------------------
def entropy(b: bytes) -> float:
    if not b:
        return 0.0
    c = [0] * 256
    for x in b:
        c[x] += 1
    n = len(b)
    return -sum(v / n * math.log2(v / n) for v in c if v)


def find_lzma_headers(data: bytes) -> list[int]:
    """LZMA-alone header: props 0x5D, dict size = power of two (64K..64M), 8-byte size (FF.. or < 64M)."""
    hits, i = [], data.find(b"\x5d")
    while i != -1 and i + 13 <= len(data):
        d = int.from_bytes(data[i + 1:i + 5], "little")
        us = int.from_bytes(data[i + 5:i + 13], "little")
        if d and d & (d - 1) == 0 and (1 << 16) <= d <= (1 << 26) and (us == 0xFFFFFFFFFFFFFFFF or 0 < us < (1 << 26)):
            hits.append(i)
        i = data.find(b"\x5d", i + 1)
    return hits


def inspect_image(data: bytes, base: int = FLASH_BASE) -> dict:
    r: dict = {"size": len(data), "problems": [], "notes": []}
    if not data:
        r["problems"].append("ファイルが空")
        return r
    if data.count(0xFF) == len(data):
        r["problems"].append("全バイトが 0xFF (消去済み/読めていない)")
    if data.count(0x00) == len(data):
        r["problems"].append("全バイトが 0x00 (読めていない)")
    if len(data) != FLASH_SIZE:
        r["notes"].append(f"サイズが 128 KB ({FLASH_SIZE}) ではない: {len(data)}")
    end = base + len(data)
    if len(data) >= 8:
        sp = int.from_bytes(data[0:4], "little")
        rv = int.from_bytes(data[4:8], "little")
        r["initial_sp"], r["reset_vector"] = sp, rv
        if not any(lo <= sp <= hi for lo, hi in SRAM_RANGES):
            r["problems"].append(f"初期 SP 0x{sp:08X} が SRAM 範囲外")
        if not rv & 1:
            r["problems"].append(f"リセットベクタ 0x{rv:08X} が偶数 (Thumb ではない)")
        if not base <= (rv & ~1) < end:
            r["problems"].append(f"リセットベクタ 0x{rv:08X} がフラッシュ範囲 0x{base:08X}-0x{end - 1:08X} の外")
    else:
        r["problems"].append("8 バイト未満でベクタテーブルが無い")
    page = 256
    pages = [data[i:i + page] for i in range(0, len(data), page)]
    r["blank_pages_ff"] = sum(1 for p in pages if p.count(0xFF) == len(p))
    r["total_pages"] = len(pages)
    strs = re.findall(rb"[\x20-\x7e]{6,}", data)
    r["strings"] = [s.decode("ascii") for s in strs]
    r["lzma_candidates"] = find_lzma_headers(data)
    magics = {"xz": b"\xfd7zXZ\x00", "gzip": b"\x1f\x8b\x08"}
    r["magic_hits"] = {k: data.count(v) for k, v in magics.items() if data.count(v)}
    blocks = [data[i:i + 1024] for i in range(0, len(data) - 1023, 1024)]
    best = cur = 0
    for b in blocks:
        cur = cur + 1 if entropy(b) > 7.5 else 0
        best = max(best, cur)
    r["max_high_entropy_run_kb"] = best
    r["bitstream_like"] = best >= 16 or bool(r["lzma_candidates"] and best >= 4)
    return r


def cmd_inspect(args) -> int:
    data = Path(args.file).read_bytes()
    r = inspect_image(data)
    print(f"ファイル: {args.file}  ({r['size']} bytes)")
    if "initial_sp" in r:
        print(f"初期 SP        : 0x{r['initial_sp']:08X}")
        print(f"リセットベクタ : 0x{r['reset_vector']:08X}")
    if "total_pages" in r:
        print(f"空 (0xFF) ページ: {r['blank_pages_ff']} / {r['total_pages']} (256 バイト単位)")
        print(f"表示可能文字列 : {len(r['strings'])} 個 (6 文字以上)")
        print(f"LZMA ヘッダ風  : {len(r['lzma_candidates'])} 箇所 {['0x%X' % o for o in r['lzma_candidates'][:5]]} (目安)")
        print(f"高エントロピー領域の最長連続: {r['max_high_entropy_run_kb']} KB -> "
              f"{'圧縮/ビットストリーム様の領域あり (目安)' if r['bitstream_like'] else '目立った領域なし'}")
    for n in r["notes"]:
        print("注意:", n)
    for p in r["problems"]:
        print("問題:", p)
    if args.strings:
        print("---- strings (自分の解析用。共有しないこと) ----")
        for s in r["strings"]:
            print(s)
    print("RESULT:", "SUSPECT" if r["problems"] else "OK")
    return 1 if r["problems"] else 0


# --------------------------------------------------------------------------
def build_parser() -> argparse.ArgumentParser:
    ap = argparse.ArgumentParser(description="GDEMU 6.0 MCU firmware: read-only check / backup over SWD")
    sub = ap.add_subparsers(dest="cmd", required=True)

    def common(p):
        p.add_argument("--probe", help="probe selector VID:PID[:SERIAL]")
        p.add_argument("--chip", default=DEFAULT_CHIP)

    c = sub.add_parser("check", help="CHIPID を読んで読み出し可否を判定")
    common(c)
    d = sub.add_parser("dump", help="フラッシュ全体を 2 回読んで保存 (UNLOCKED_2E のときだけ)")
    common(d)
    d.add_argument("--out", required=True)
    d.add_argument("--chunk", type=int, default=DEFAULT_CHUNK, help="bytes per probe-rs read (default 4096)")
    d.add_argument("--retries", type=int, default=3, help="retries per chunk on probe errors")
    d.add_argument("--max-rereads", type=int, default=5, help="re-reads for a mismatching chunk")
    i = sub.add_parser("inspect", help="dump の妥当性チェック")
    i.add_argument("file")
    i.add_argument("--strings", action="store_true")
    return ap


def main(argv=None, pr: ProbeRs | None = None) -> int:
    for s in (sys.stdout, sys.stderr):
        try:
            s.reconfigure(errors="replace")
        except Exception:
            pass
    args = build_parser().parse_args(argv)
    if args.cmd == "inspect":
        return cmd_inspect(args)
    pr = pr or ProbeRs()
    try:
        return cmd_check(args, pr) if args.cmd == "check" else cmd_dump(args, pr)
    except SafetyRefusal as e:  # indicates a bug in this tool
        print(f"[fwbackup] SAFETY REFUSAL: {e}", file=sys.stderr)
        return 99


if __name__ == "__main__":
    sys.exit(main())
