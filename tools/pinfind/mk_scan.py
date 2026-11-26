#!/usr/bin/env python3
"""Generate the EP2C5F256C8 scan core wrapper (fpga/scan-ep2c5).

usage: mk_scan.py [--padmap tools/pinfind/ep2c5_padmap.csv]
                  [--table tools/pinfind/ep2c5_scan.csv]
                  [--out fpga/scan-ep2c5]

Every user I/O ball and every dedicated clock-input ball of the F256
package (ep2c5_padmap.csv rows whose I/O Type is Row I/O or Column I/O,
158 balls) becomes a port p_<BALL>:

  * the 16 MCU_DATA balls of v5.5 (MCU_DATA[0..15], pad-identical on 6.0,
    ag16k-ps-runs-20261124.md section 17) are tri-state OUTPUTS, driven only
    while NCS3 (R8) and NRD (M12) are both low. Weak pull-up OFF (the MCU
    keeps its own pull-ups on D0..D15).
  * every other user I/O ball is an INPUT with weak pull-up ON;
  * the 8 dedicated clock-input balls (H1 H2 J1 J2 H15 H16 J15 J16) are
    INPUTS with weak pull-up OFF (they have none, ep2c5-pullcode.md).

Bus hold is OFF on every port. No PLL. All other settings are the stage-0
ones (fpga/stage0-ep2c5/stage0.qsf).

The inputs (142) are numbered in pad order: raw index i is bit i % 16 of
raw word i // 16 (read at 0x60 + i // 16). The 33 DC_* balls of v5.5 get a
G1 index (DC_ORDER below); the 8 clock balls a clock index (CLOCKS).

Writes:
  <table>                ep2c5_scan.csv (ball,pad,bank,io_type,kind,dir,
                         pullup,signal,raw,d_bit,clk,dc)
  <out>/scan.qpf
  <out>/scan.v           port wrapper around scan_core (scan_core.v is
                         hand-written and not generated)
  <out>/scan.qsf

Standard library only. The output is deterministic.
"""
import argparse
import csv
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))

# Dedicated clock inputs CLK0..CLK7 (F256 balls).
CLOCKS = ["H1", "H2", "J1", "J2", "H15", "H16", "J15", "J16"]
# Window / observation reference clock (v5.5 CLK_48_MHz, MCU PA21 PCK0).
REF_CLOCK = "H16"
# MCU bus balls (v5.5 signal names): address A1..A8, chip select, read.
MCU_ADDR = [f"MCU_ADDR[{i}]" for i in range(1, 9)]
MCU_NCS = "MCU_NCS"
MCU_NRD = "MCU_NRE"
# G1 (Dreamcast) side, v5.5 names, in G1 index order.
DC_ORDER = ([f"DC_DATA[{i}]" for i in range(16)]
            + ["DC_ADDR[0]", "DC_ADDR[1]", "DC_ADDR[2]", "DC_CSn[0]", "DC_CSn[1]",
               "DC_RDn", "DC_WRn", "DC_DMACKn", "DC_RSTn",
               "DC_INTRQ", "DC_DMARQ", "DC_IORDY",
               "DC_CDCLK", "DC_SCK", "DC_SDAT", "DC_LRCK", "DC_EMPH"])
NRAW_WORDS = 9


def load_balls(padmap):
    """[(ball, pad, bank, io_type, kind, v55_signal)] in pad order."""
    out = []
    with open(padmap, encoding="utf-8") as f:
        for r in csv.DictReader(f):
            ball = r["f256_ball"]
            if not ball or r["io_type"] not in ("Row I/O", "Column I/O"):
                continue
            kind = "clock" if "clock input" in r["special"] else "io"
            out.append((ball, int(r["pad"]), r["bank"], r["io_type"], kind,
                        r["v55_signal"]))
    out.sort(key=lambda x: x[1])
    return out


def build(padmap):
    balls = load_balls(padmap)
    sig = {s: b for b, _, _, _, _, s in balls if s}
    for s in MCU_ADDR + [MCU_NCS, MCU_NRD] + DC_ORDER + [f"MCU_DATA[{i}]" for i in range(16)]:
        if s not in sig:
            raise SystemExit(f"mk_scan: {s} not in {padmap}")
    if sorted(b for b, _, _, _, k, _ in balls if k == "clock") != sorted(CLOCKS):
        raise SystemExit("mk_scan: clock balls differ from CLOCKS")
    dbit = {sig[f"MCU_DATA[{i}]"]: i for i in range(16)}
    dcidx = {sig[s]: i for i, s in enumerate(DC_ORDER)}
    rows = []
    raw = 0
    for ball, pad, bank, iot, kind, s in balls:
        if ball in dbit:
            k, d, pu, ri = "data", "output", "off", ""
        elif kind == "clock":
            if ball not in CLOCKS:
                raise SystemExit(f"mk_scan: unexpected clock ball {ball}")
            k, d, pu, ri = "clock", "input", "off", str(raw)
            raw += 1
        else:
            k, d, pu, ri = "io", "input", "on", str(raw)
            raw += 1
        rows.append({"ball": ball, "pad": str(pad), "bank": bank, "io_type": iot,
                     "kind": k, "dir": d, "pullup": pu, "signal": s, "raw": ri,
                     "d_bit": str(dbit[ball]) if ball in dbit else "",
                     "clk": str(CLOCKS.index(ball)) if ball in CLOCKS else "",
                     "dc": str(dcidx[ball]) if ball in dcidx else ""})
    if len(rows) != 158 or raw != 142:
        raise SystemExit(f"mk_scan: {len(rows)} ports / {raw} inputs, expected 158 / 142")
    return rows


def by(rows, key):
    out = {}
    for r in rows:
        if r[key] != "":
            out[int(r[key])] = r
    return [out[i] for i in range(len(out))]


def sig_ball(rows, s):
    for r in rows:
        if r["signal"] == s:
            return r["ball"]
    raise KeyError(s)


def port(ball):
    return "p_" + ball


def verilog_text(rows):
    raws = by(rows, "raw")
    data = by(rows, "d_bit")
    clks = by(rows, "clk")
    dcs = by(rows, "dc")
    addr = [sig_ball(rows, s) for s in MCU_ADDR]
    L = [
        "// GENERATED by tools/pinfind/mk_scan.py -- do not edit.",
        "// EP2C5F256C8 scan core: port wrapper. One port per user I/O and",
        "// clock-input ball. Outputs: only the 16 MCU_DATA balls, tri-state,",
        "// enabled while NCS3 (R8) and NRD (M12) are both low. Every other",
        "// port is an input. The logic is in scan_core.v; the address map is",
        "// docs/research/ep2c5-scan.md.",
        "module scan (",
    ]
    for i, r in enumerate(rows):
        sep = "," if i + 1 < len(rows) else ""
        d = "output" if r["dir"] == "output" else "input "
        L.append(f"    {d} wire {port(r['ball'])}{sep}")
    L += [");", "",
          f"    wire [{len(raws) - 1}:0] raw;",
          "    wire [7:0] clk;",
          f"    wire [{len(dcs) - 1}:0] dc;",
          "    wire [7:0] addr;",
          "    wire [15:0] d_out;",
          "    wire d_oe;", ""]
    for i, r in enumerate(raws):
        L.append(f"    assign raw[{i}] = {port(r['ball'])};")
    L.append("")
    for i, r in enumerate(clks):
        L.append(f"    assign clk[{i}] = {port(r['ball'])};")
    L.append("")
    for i, r in enumerate(dcs):
        L.append(f"    assign dc[{i}] = {port(r['ball'])};  // {r['signal']}")
    L.append("")
    for i, b in enumerate(addr):
        L.append(f"    assign addr[{i}] = {port(b)};  // MCU A{i + 1}")
    L.append("")
    for i, r in enumerate(data):
        L.append(f"    assign {port(r['ball'])} = d_oe ? d_out[{i}] : 1'bz;  // MCU D{i}")
    L += ["",
          "    scan_core core (",
          "        .raw(raw), .clk(clk), .dc(dc), .addr(addr),",
          f"        .ncs({port(sig_ball(rows, MCU_NCS))}), .nrd({port(sig_ball(rows, MCU_NRD))}),",
          "        .d_out(d_out), .d_oe(d_oe)",
          "    );",
          "endmodule", ""]
    return "\n".join(L)


def stage0_globals():
    p = os.path.join(ROOT, "fpga", "stage0-ep2c5", "stage0.qsf")
    keep = []
    with open(p, encoding="utf-8") as f:
        for line in f:
            s = line.strip()
            if not s.startswith("set_global_assignment"):
                continue
            if re.search(r"-name (TOP_LEVEL_ENTITY|VERILOG_FILE)\b", s):
                continue
            keep.append(s)
    return keep


def qsf_text(rows):
    n_on = sum(r["pullup"] == "on" for r in rows)
    L = [
        "# GENERATED by tools/pinfind/mk_scan.py -- do not edit.",
        "# EP2C5F256C8 scan core (Quartus II 13.0 SP1).",
        "#",
        "# Outputs (tri-state) only on the 16 MCU_DATA balls; every other user",
        f"# I/O is an input with weak pull-up ON ({n_on} balls), the 8 dedicated",
        "# clock inputs are inputs with pull-up OFF, the MCU_DATA outputs have",
        "# pull-up OFF. Bus hold OFF everywhere. No PLL. All other settings are",
        "# the stage-0 ones (fpga/stage0-ep2c5/stage0.qsf).",
        "# tools/pinfind/ep2c5gate.py --scan checks the fitter's result.",
        "# Bitstreams are CI artifacts only.",
        "",
        "set_global_assignment -name TOP_LEVEL_ENTITY scan",
        "set_global_assignment -name VERILOG_FILE scan.v",
        "set_global_assignment -name VERILOG_FILE scan_core.v",
        "set_global_assignment -name SDC_FILE scan.sdc",
    ]
    L += stage0_globals()
    L.append("")
    for r in rows:
        p = port(r["ball"])
        L.append(f"set_location_assignment PIN_{r['ball']} -to {p}")
        L.append(f"set_instance_assignment -name IO_STANDARD \"3.3-V LVTTL\" -to {p}")
        L.append(f"set_instance_assignment -name WEAK_PULL_UP_RESISTOR "
                 f"{'ON' if r['pullup'] == 'on' else 'OFF'} -to {p}")
        L.append(f"set_instance_assignment -name ENABLE_BUS_HOLD_CIRCUITRY OFF -to {p}")
    L.append("")
    return "\n".join(L)


COLS = ["ball", "pad", "bank", "io_type", "kind", "dir", "pullup", "signal",
        "raw", "d_bit", "clk", "dc"]


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--padmap", default=os.path.join(HERE, "ep2c5_padmap.csv"))
    ap.add_argument("--table", default=os.path.join(HERE, "ep2c5_scan.csv"))
    ap.add_argument("--out", default=os.path.join(ROOT, "fpga", "scan-ep2c5"))
    a = ap.parse_args(argv)
    rows = build(a.padmap)
    with open(a.table, "w", newline="", encoding="utf-8") as f:
        w = csv.DictWriter(f, fieldnames=COLS, lineterminator="\n")
        w.writeheader()
        w.writerows(rows)
    write(os.path.join(a.out, "scan.qpf"),
          'QUARTUS_VERSION = "13.0"\nPROJECT_REVISION = "scan"\n')
    write(os.path.join(a.out, "scan.v"), verilog_text(rows))
    write(os.path.join(a.out, "scan.qsf"), qsf_text(rows))
    print(f"ports {len(rows)}; outputs {sum(r['dir'] == 'output' for r in rows)}; "
          f"inputs {sum(r['dir'] == 'input' for r in rows)}; "
          f"pull-up on {sum(r['pullup'] == 'on' for r in rows)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
