#!/usr/bin/env python3
"""Join the EP2C5T144 (v5.5) and EP2C5F256 (stage-0) pinouts by die pad.

usage: ep2c5_padmap.py --t144-fit OpenGDEMU.fit.rpt --f256-fit stage0.fit.rpt
                       [--qsf fpga/Quartus/OpenGDEMU.qsf]
                       [--csv tools/pinfind/ep2c5_padmap.csv]
                       [--md docs/research/ep2c5-padmap.md]

Both reports have an "All Package Pins" table with a "Pad Number" column.
The pad number is the die's own I/O pad index, so it is the same on every
package of the EP2C5 die; joining on it gives T144 pin <-> pad <-> F256 ball.
The v5.5 signal names come from the qsf (set_location_assignment PIN_n).

Checks (printed, and listed in the Markdown):
  * every qsf signal sits on a T144 pin that has a pad, and that pad has an
    F256 ball (a v5.5 signal with no F256 ball is a FINDING)
  * the qsf name matches the fit.rpt "Pin Name/Usage" at that pin
  * the I/O bank of the pad is the same in both packages
  * fixed (dedicated) functions such as ^nCONFIG / #TDO agree on both sides

Standard library only. Exit 1 if a check fails.
"""
import argparse
import csv
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ep2c5gate import rpt_tables, find_table, col  # noqa: E402


def read(path):
    with open(path, encoding="utf-8", errors="replace") as f:
        return f.read()


def package_pins(text):
    """Return list of dicts loc/pad/bank/name/dir/iotype/bushold/pullup."""
    t, rows = find_table(rpt_tables(text), "All Package Pins")
    if not t:
        raise SystemExit("no 'All Package Pins' table")
    h = rows[0]
    idx = {k: col(h, k) for k in ("Location", "Pad Number", "I/O Bank",
                                  "Pin Name/Usage", "Dir.", "I/O Type",
                                  "Bus Hold", "Weak Pull Up")}
    if idx["Location"] is None or idx["Pad Number"] is None:
        raise SystemExit("All Package Pins: no Location / Pad Number column")
    out = []
    for r in rows[1:]:
        def g(k):
            i = idx[k]
            return r[i] if i is not None and i < len(r) else ""
        out.append({"loc": g("Location"), "pad": g("Pad Number"),
                    "bank": g("I/O Bank"), "name": g("Pin Name/Usage"),
                    "dir": g("Dir."), "iotype": g("I/O Type"),
                    "bushold": g("Bus Hold"), "pullup": g("Weak Pull Up")})
    return out


def qsf_locations(text):
    """Return {pin: signal} from set_location_assignment PIN_x -to NAME."""
    m = {}
    for line in text.splitlines():
        line = line.split("#", 1)[0]
        r = re.match(r"\s*set_location_assignment\s+PIN_(\S+)\s+-to\s+\"?([^\"\s]+)\"?", line)
        if r:
            m[r.group(1)] = r.group(2)
    return m


FIXED = re.compile(r"^[\^#]")


def special(t, f):
    """Special-function text for a pad, from both packages' names."""
    s = []
    for p in (t, f):
        if not p:
            continue
        n = p["name"]
        if FIXED.match(n):
            s.append(n[1:] + " (dedicated)")
        m = re.match(r"^~([^~]+)~", n)
        if m:
            s.append(m.group(1) + " (dual-purpose)")
        if n == "GND+":
            # Quartus lists an unused dedicated clock input pad as GND+.
            s.append("dedicated clock input (CLK, input only)")
    out = []
    for x in s:
        if x not in out:
            out.append(x)
    return "; ".join(out)


def natkey(s):
    return [int(x) if x.isdigit() else x for x in re.split(r"(\d+)", s)]


def build(t144, f256, qsf):
    tpad = {p["pad"]: p for p in t144 if p["pad"]}
    fpad = {p["pad"]: p for p in f256 if p["pad"]}
    tloc = {p["loc"]: p for p in t144}
    errors, findings = [], []
    for d, nm in ((t144, "T144"), (f256, "F256")):
        pads = [p["pad"] for p in d if p["pad"]]
        if len(pads) != len(set(pads)):
            errors.append(f"{nm}: duplicate pad numbers")
    rows = []
    for pad in sorted(set(tpad) | set(fpad), key=int):
        t, f = tpad.get(pad), fpad.get(pad)
        sig = qsf.get(t["loc"], "") if t else ""
        bank_t = t["bank"] if t else ""
        bank_f = f["bank"] if f else ""
        note = []
        if t and f and bank_t and bank_f and bank_t != bank_f:
            note.append(f"bank mismatch T144 {bank_t} / F256 {bank_f}")
            errors.append(f"pad {pad}: bank mismatch T144 {bank_t} F256 {bank_f}")
        if t and f:
            ft, ff = FIXED.match(t["name"]), FIXED.match(f["name"])
            if (ft or ff) and t["name"] != f["name"]:
                note.append(f"fixed-function mismatch {t['name']} / {f['name']}")
                errors.append(f"pad {pad}: {t['name']} vs {f['name']}")
        if sig and not f:
            note.append("v5.5 signal has NO F256 ball")
            findings.append(f"pad {pad} (T144 {t['loc']}) {sig}: no F256 ball")
        if sig and t["name"] != sig:
            note.append(f"fit.rpt name {t['name']!r} != qsf {sig!r}")
            errors.append(f"T144 {t['loc']}: fit.rpt {t['name']!r} != qsf {sig!r}")
        rows.append({
            "pad": pad,
            "t144_pin": t["loc"] if t else "",
            "f256_ball": f["loc"] if f else "",
            "v55_signal": sig,
            "v55_dir": t["dir"] if sig else "",
            "bank": bank_f or bank_t,
            "io_type": (f or t)["iotype"],
            "special": special(t, f),
            "t144_usage": t["name"] if t else "",
            "f256_usage": f["name"] if f else "",
            "note": "; ".join(note),
        })
    for pin, sig in sorted(qsf.items(), key=lambda kv: natkey(kv[0])):
        p = tloc.get(pin)
        if p is None:
            errors.append(f"qsf {sig} on PIN_{pin}: no such T144 pin in fit.rpt")
        elif not p["pad"]:
            errors.append(f"qsf {sig} on PIN_{pin}: T144 pin has no pad")
    return rows, errors, findings


COLS = ["pad", "t144_pin", "f256_ball", "v55_signal", "v55_dir", "bank",
        "io_type", "special", "t144_usage", "f256_usage", "note"]


def write_csv(path, rows):
    with open(path, "w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=COLS, lineterminator="\n")
        w.writeheader()
        w.writerows(rows)


def md_escape(s):
    return s.replace("|", "\\|")


def write_md(path, rows, errors, findings, qsf, src):
    sig_rows = [r for r in rows if r["v55_signal"]]
    on_f = [r for r in sig_rows if r["f256_ball"]]
    t_only = [r for r in rows if r["t144_pin"] and not r["f256_ball"]]
    f_only = [r for r in rows if r["f256_ball"] and not r["t144_pin"]]
    L = []
    L.append("# EP2C5 パッド対応表 (T144 ピン ↔ ダイパッド ↔ F256 ボール)")
    L.append("")
    L.append("このファイルは `tools/pinfind/ep2c5_padmap.py` が生成する。手で編集しない。")
    L.append("")
    L.append("## 入力")
    L.append("")
    for k, v in src.items():
        L.append(f"- {k}: `{v}`")
    L.append("")
    L.append("両レポートの \"All Package Pins\" 表の Pad Number 列で結合した。"
             "Pad Number はダイ上の I/O パッド番号で、同じ EP2C5 ダイならパッケージに"
             "よらず同じ番号になる (TDO=13, DATA0=17, nCE=19, nCONFIG=22, nSTATUS=99, "
             "CONF_DONE=100, MSEL1=101, MSEL0=102 が両パッケージで一致することで確認)。")
    L.append("")
    L.append("## 結果の要約")
    L.append("")
    L.append(f"- qsf のピン割り当て: {len(qsf)} 本")
    L.append(f"- そのうち F256 ボールがあるもの: {len(on_f)} 本 / {len(sig_rows)} 本")
    L.append(f"- パッド総数 (どちらかのパッケージに出ているもの): {len(rows)}")
    L.append(f"- T144 にだけ出ているパッド: {len(t_only)}"
             + (" (" + ", ".join(r['pad'] for r in t_only) + ")" if t_only else ""))
    L.append(f"- F256 にだけ出ているパッド: {len(f_only)}")
    L.append(f"- 不一致 (バンク / 専用機能名 / qsf と fit.rpt の名前): {len(errors)} 件")
    L.append("")
    if findings:
        L.append("### F256 ボールが無い v5.5 信号")
        L.append("")
        for x in findings:
            L.append(f"- {x}")
        L.append("")
    else:
        L.append("v5.5 の全信号が F256 のボールに出ている (F256 ボールの無い v5.5 信号は 0 本)。")
        L.append("")
    if errors:
        L.append("### 不一致")
        L.append("")
        for x in errors:
            L.append(f"- {x}")
        L.append("")
    L.append("## 注意")
    L.append("")
    L.append("- これは Quartus のパッケージ情報だけから作った対応表で、GDEMU 6.0 基板上で"
             "どのボールがどこに配線されているかは何も言っていない。")
    L.append("- T144 の v5.5 が使うパッドが F256 にもあることは、6.0 基板で同じパッドが"
             "同じ役割で使われていることを意味しない。")
    L.append("- `GND+` は未使用の専用クロック入力 (CLK0..7)。入力専用パッド。")
    L.append("- v5.5 の `~ASDO~` / `~nCSO~` / `~LVDS41p/nCEO~` はデュアルパーパスピン。")
    L.append("")
    L.append("## v5.5 信号 (qsf) の対応")
    L.append("")
    L.append("| v5.5 信号 | 方向 | T144 ピン | パッド | F256 ボール | バンク | 特殊機能 | 備考 |")
    L.append("|---|---|---|---|---|---|---|---|")
    for r in sorted(sig_rows, key=lambda r: natkey(r["v55_signal"])):
        L.append("| " + " | ".join(md_escape(x) for x in (
            r["v55_signal"], r["v55_dir"], r["t144_pin"], r["pad"],
            r["f256_ball"] or "**なし**", r["bank"], r["special"], r["note"])) + " |")
    L.append("")
    L.append("## 全パッド")
    L.append("")
    L.append("| パッド | T144 ピン | F256 ボール | v5.5 信号 | バンク | I/O 種別 | 特殊機能 | T144 での用途 | F256 (stage-0) での用途 |")
    L.append("|---|---|---|---|---|---|---|---|---|")
    for r in rows:
        L.append("| " + " | ".join(md_escape(x) for x in (
            r["pad"], r["t144_pin"] or "-", r["f256_ball"] or "-", r["v55_signal"],
            r["bank"], r["io_type"], r["special"], r["t144_usage"],
            r["f256_usage"])) + " |")
    L.append("")
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(L))


def main(argv=None):
    here = os.path.dirname(os.path.abspath(__file__))
    root = os.path.normpath(os.path.join(here, "..", ".."))
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--t144-fit", required=True, help="v5.5 OpenGDEMU.fit.rpt")
    ap.add_argument("--f256-fit", required=True, help="stage-0 stage0.fit.rpt")
    ap.add_argument("--qsf", default=os.path.join(root, "fpga", "Quartus", "OpenGDEMU.qsf"))
    ap.add_argument("--csv", default=os.path.join(here, "ep2c5_padmap.csv"))
    ap.add_argument("--md", default=os.path.join(root, "docs", "research", "ep2c5-padmap.md"))
    ap.add_argument("--src-t144", default=None, help="label for the T144 source in the .md")
    ap.add_argument("--src-f256", default=None, help="label for the F256 source in the .md")
    a = ap.parse_args(argv)
    t144 = package_pins(read(a.t144_fit))
    f256 = package_pins(read(a.f256_fit))
    qsf = qsf_locations(read(a.qsf))
    rows, errors, findings = build(t144, f256, qsf)
    write_csv(a.csv, rows)
    src = {"T144 (v5.5) fit.rpt": a.src_t144 or a.t144_fit,
           "F256 (stage-0) fit.rpt": a.src_f256 or a.f256_fit,
           "qsf": "fpga/Quartus/OpenGDEMU.qsf"}
    write_md(a.md, rows, errors, findings, qsf, src)
    nsig = sum(1 for r in rows if r["v55_signal"])
    non = sum(1 for r in rows if r["v55_signal"] and r["f256_ball"])
    print(f"pads {len(rows)}; qsf signals {len(qsf)}; on a pad {nsig}; with F256 ball {non}")
    for x in findings:
        print("FINDING:", x)
    for x in errors:
        print("MISMATCH:", x)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
