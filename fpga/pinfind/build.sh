#!/usr/bin/env bash
# NetGD: build the AG16K pin-finder bitstream with AGM Supra (WSL / Linux).
#
#   SUPRA_HOME=/path/to/Supra-2026.03.b0 fpga/pinfind/build.sh [options]
#
# Input: the files gen.py wrote into fpga/pinfind/build/ (PinFind.v,
# PinFindCore.v unless stage 0, PinFind.asf, PinFind.sdc, params.txt,
# balls.txt). Run gen.py first (Windows side, Amaranth venv), see README_ja.md.
#
#   --gen DIR              where gen.py wrote its files (default fpga/pinfind/build)
#   --work DIR             scratch directory (default fpga/pinfind/build/supra; wiped)
#   --seed N               place-and-route seed
#   --timeout S            give up on place-and-route after S seconds (default 1200)
#   --placement FILE       Supra report that lists the ball each I/O was PLACED on,
#   --placement-regex RE   and a regex with (?P<port>..) and (?P<ball>..) for its lines
#                          (only when checkpnr.py cannot find the placement by itself;
#                          with Supra 2026.03.b0 it reads alta_db/io.asf; README_ja.md 3-3)
#   --sites-ref FILE       an earlier build's pinfind_ag16k.sites: every ball must be on
#                          the same die site (coord_x/y/z of the routed netlist) as there
#   --unused-ok PIN_xx,..  user-capable balls a human accepts as unused (recorded)
#   --gate-sim             also simulate the routed netlist with Supra's models
#                          (Icarus Verilog): at most one pad low at any time
#   --gate-sim-ms N        simulated time for --gate-sim (default 300)
#   --gate-sim-done        with --gate-sim: also require that the scan ran to the end
#                          (every pad driven, then no pad change for the last 10 %;
#                          only for a fast design, gen.py --prescale 1 --start 8)
#   --supra-retiming       leave af_map's RETIMING step (Yosys `abc -dff`) on; for
#                          testing only (it mis-maps enable-gated sync resets, README_ja.md)
#
# Output: fpga/pinfind/output/pinfind_ag16k.rbf (the name the `pinfind`
# firmware build embeds), pinfind_ag16k.params (params.txt + the bitstream's
# size and POSIX cksum; the firmware's build.rs reads it) and
# pinfind_ag16k.txt (report), plus pinfind_ag16k.sites (port, ball, die site;
# for --sites-ref). All are deleted first, so a failed build leaves no
# bitstream behind.
#
# What this script checks, and stops on (R143 item 1):
#   - that af_map's RETIMING step (`abc -dff` before dfflegalize) did not run:
#     Yosys 0.61 there turns a sync reset that only acts while the enable is on
#     into one that always acts (gate-level simulation: the last ball was never
#     driven, DONE never reached). It is switched off through
#     PinFind.pre_map.asf, which af_map.tcl sources after its own default;
#   - the .rbf size and header; Supra errors other than the known pre-.asf
#     "location is not assigned" ones;
#   - any automatic placement ("placed at pin") in Supra's log;
#   - the pin count, and (checkpnr.py) that the ACTUAL placement read from
#     Supra's report puts every port on the ball balls.txt says, hold balls
#     included; if the placement cannot be read, it stops;
#   - (checkpnr.py) that every user-capable ball of $SUPRA_HOME's pinmap is in
#     the design (Supra's unused-pin state is unknown, so none is left unused),
#     unless named with --unused-ok;
#   - with --gate-sim: a gate-level simulation of the routed netlist.
# What it does NOT check (a human must): that Supra's report is what the
# silicon does, the electrical behaviour, the board. See README_ja.md 0 and 6.
#
# Same rules as fpga/agm/build.sh: every Supra program runs inside
# `unshare -rn` (no network); only af_setup, af_map, af_run and af_cmd
# --version are started; nothing is written into or copied out of the Supra
# tree except the bitstream for this design. The pinmap and the simulation
# models are read in place.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
AGM=$HERE/../agm
DESIGN=PinFind
DEVICE=AG16KDDF256
RBF_BYTES=552352
XZ_FILTER=preset=9e,dict=4KiB,lc=0,lp=0,pb=0   # = mcu/netgd/build.rs XZ_FILTER

GEN=$HERE/build
WORK=
SEED=
AF_TIMEOUT=1200
PLACEMENT=
PLACEMENT_RE=
SITES_REF=
UNUSED_OK=()
GATE_SIM=0
GATE_MS=300
GATE_DONE=
RETIME=0

die() { echo "build.sh: $*" >&2; exit 1; }

while [ $# -gt 0 ]; do
    case $1 in
        --gen) GEN=$2; shift 2 ;;
        --work) WORK=$2; shift 2 ;;
        --seed) SEED=$2; shift 2 ;;
        --timeout) AF_TIMEOUT=$2; shift 2 ;;
        --placement) PLACEMENT=$(cd "$(dirname "$2")" && pwd)/$(basename "$2"); shift 2 ;;
        --placement-regex) PLACEMENT_RE=$2; shift 2 ;;
        --sites-ref) SITES_REF=$(cd "$(dirname "$2")" && pwd)/$(basename "$2"); shift 2 ;;
        --unused-ok) UNUSED_OK+=(--unused-ok "$2"); shift 2 ;;
        --gate-sim) GATE_SIM=1; shift ;;
        --gate-sim-ms) GATE_MS=$2; shift 2 ;;
        --gate-sim-done) GATE_DONE=--expect-done; shift ;;
        --supra-retiming) RETIME=1; shift ;;
        -h|--help) sed -n '2,60p' "$0"; exit 0 ;;
        *) die "unknown argument: $1" ;;
    esac
done
WORK=${WORK:-$GEN/supra}

[ -n "${SUPRA_HOME:-}" ] || die "set SUPRA_HOME to the Supra installation (the directory that holds bin/af)"
SUPRA_HOME=$(cd "$SUPRA_HOME" && pwd)
for t in af_cmd af_setup af_map af_run; do
    [ -x "$SUPRA_HOME/bin/$t" ] || die "$SUPRA_HOME/bin/$t not found"
done
PINMAP=$SUPRA_HOME/etc/$DEVICE.pinmap
[ -f "$PINMAP" ] || die "$PINMAP not found (needed to check that no user-capable ball is left unused)"
for t in unshare timeout python3 xz sha256sum cksum; do
    command -v "$t" >/dev/null || die "\`$t\` is needed"
done
unshare -rn true 2>/dev/null || die "\`unshare -rn\` does not work here; Supra is only ever run without network"
for f in PinFind.v PinFind.asf PinFind.sdc params.txt balls.txt; do
    [ -f "$GEN/$f" ] || die "$GEN/$f not found: run gen.py first (README_ja.md)"
done
case ${SEED:-0} in *[!0-9]*) die "--seed takes a number" ;; esac
case $AF_TIMEOUT in ''|*[!0-9]*) die "--timeout takes seconds" ;; esac
case $GATE_MS in ''|*[!0-9]*) die "--gate-sim-ms takes a number" ;; esac
[ -z "$PLACEMENT" ] || [ -n "$PLACEMENT_RE" ] || die "--placement needs --placement-regex"
[ -z "$SITES_REF" ] || [ -f "$SITES_REF" ] || die "--sites-ref $SITES_REF not found"

param() { tr -d '\r' <"$GEN/params.txt" | sed -n "s/^$1 //p"; }
N_DRIVE=$(param n_drive)
TOTAL_PINS=$(param total_pins)
CLOCK=$(param clock)
STAGE0=$(param stage0)
MAX_LOOPS=$(param max_loops)
[ -n "$N_DRIVE" ] && [ -n "$TOTAL_PINS" ] && [ "$(param format)" = 2 ] ||
    die "params.txt is incomplete or from an older gen.py: run gen.py again"
case $MAX_LOOPS in [1-8]) ;; *) die "params.txt: max_loops $MAX_LOOPS is not 1..8" ;; esac
if [ "$STAGE0" = 1 ]; then
    VERILOG="PinFind.v"
    N_INOUT=0
else
    [ -f "$GEN/PinFindCore.v" ] || die "$GEN/PinFindCore.v not found: run gen.py again"
    VERILOG="PinFind.v PinFindCore.v"
    N_INOUT=$N_DRIVE
fi

ns() { unshare -rn env SUPRA="$SUPRA_HOME" PATH="$SUPRA_HOME/bin:/usr/bin:/bin" "$@"; }

# No stale bitstream: whatever happens below, the old one is gone.
OUT=$HERE/output
rm -f "$OUT/pinfind_ag16k.rbf" "$OUT/pinfind_ag16k.params" "$OUT/pinfind_ag16k.txt" "$OUT/pinfind_ag16k.sites"

if [ -e "$WORK" ]; then
    [ -f "$WORK/.netgd-pinfind-work" ] || die "$WORK exists and is not a build.sh work directory; not touching it"
    rm -rf "$WORK"
fi
mkdir -p "$WORK"
WORK=$(cd "$WORK" && pwd)
: >"$WORK/.netgd-pinfind-work"
cd "$WORK"

for f in $VERILOG PinFind.sdc; do tr -d '\r' <"$GEN/$f" >"$f"; done
tr -d '\r' <"$GEN/PinFind.asf" | grep -v '^[[:space:]]*#' | grep -v '^[[:space:]]*$' >$DESIGN.asf

echo "== af_setup"
# shellcheck disable=SC2086
ns af_setup --design $DESIGN --device $DEVICE --verilog $VERILOG >setup.out 2>&1 ||
    { tail -20 setup.out >&2; die "af_setup failed (setup.out)"; }
# af_map's RETIMING step (`abc -markgroups -dff` before dfflegalize) mis-maps
# $sdffce (sync reset gated by the enable) as $sdffe (reset always active):
# Yosys 0.61 abc -dff keeps clk/en/srst but drops which one wins. af_map.tcl
# sources $DESIGN.pre_map.asf after setting its default RETIMING ("100").
[ -f $DESIGN.pre_map.asf ] || die "af_setup wrote no $DESIGN.pre_map.asf (Supra changed?)"
[ -s $DESIGN.pre_map.asf ] && die "$DESIGN.pre_map.asf from af_setup is not empty (Supra changed?)"
grep -q 'RETIMING != "None"' af_map.tcl && grep -q 'source .*pre_map.asf' af_map.tcl ||
    die "af_map.tcl has no RETIMING switch / pre_map.asf hook (Supra changed?): cannot turn the step off"
if [ "$RETIME" = 0 ]; then
    echo 'set RETIMING "None"' >$DESIGN.pre_map.asf
fi
echo "== af_map"
ns af_map -TQd -v 1 -L map.log -c af_map.tcl >map.out 2>&1 ||
    { tail -20 map.out >&2; die "af_map failed (map.out, map.log)"; }
# Was the RETIMING `abc -dff` run? It is the only ABC pass before DFFLEGALIZE.
first_abc=$(grep -n 'Executing ABC pass' map.log | head -1 | cut -d: -f1)
legalize=$(grep -n 'Executing DFFLEGALIZE pass' map.log | head -1 | cut -d: -f1)
[ -n "$first_abc" ] && [ -n "$legalize" ] || die "map.log: ABC / DFFLEGALIZE pass not found (Supra changed?)"
if [ "$first_abc" -lt "$legalize" ]; then
    RETIMED=on
    [ "$RETIME" = 1 ] || die "af_map ran its RETIMING abc -dff although it was switched off (map.log)"
else
    RETIMED=off
    [ "$RETIME" = 0 ] || die "--supra-retiming given but af_map did not run the RETIMING step (map.log)"
fi
# Each scanned ball is an inout (the core listens before it drives): merge
# the ibuf + obuf pair into one alta_io, as for the real core. Stage 0 has none.
python3 -I "$AGM/fixio.py" $DESIGN.vqm $DESIGN.fixed.vqm --expect "$N_INOUT"
mv $DESIGN.vqm $DESIGN.orig.vqm
cp $DESIGN.fixed.vqm $DESIGN.vqm
case $CLOCK in
    osc)  grep -q 'alta_osc' $DESIGN.vqm || die "alta_osc is not in the mapped netlist (optimised away or unknown to af_map); try --clock ring or ball:" ;;
    ring) n=$(grep -c 'cycloneive_lcell_comb' $DESIGN.vqm || true)
          [ "$n" -ge "$(param ring_stages)" ] || die "ring oscillator cells missing from the mapped netlist ($n)" ;;
esac

echo "== af_run"
rc=0
ns timeout -k 10 "$AF_TIMEOUT" af_run --mode NATIVE ${SEED:+--seed "$SEED"} >run.out 2>&1 || rc=$?
if [ $rc = 124 ] || [ $rc = 137 ]; then
    tail -2 run.out >&2
    die "af_run did not finish in $AF_TIMEOUT s (run.out); run again or pass another --seed"
elif [ $rc != 0 ]; then
    grep -n '^Error\|^Fatal' run.out | grep -v locatiion | head -20 >&2
    die "af_run failed (run.out)"
fi

RBF=${DESIGN}_slave.rbf
[ -f "$RBF" ] || die "af_run produced no $RBF"
size=$(wc -c <"$RBF")
[ "$size" = "$RBF_BYTES" ] || die "$RBF is $size bytes, expected $RBF_BYTES (compression left on?)"
head4=$(head -c 4 "$RBF" | od -An -tx1 | tr -d ' \n')
[ "$head4" = 697e3c5a ] || die "$RBF starts with $head4, expected 697e3c5a"
if grep -in 'placed at pin' run.out >&2; then
    die "Supra placed I/O by itself (\"placed at pin\" above): a location assignment did not take"
fi
grep -q "Total  *Pins *: *$TOTAL_PINS/" run.out ||
    die "pin count is not $TOTAL_PINS (run.out): a port was dropped or split. Every ball in the list must be in the design"
if grep '^Error\|^Fatal' run.out | grep -v locatiion | grep -q .; then
    grep -n '^Error\|^Fatal' run.out | grep -v locatiion | head -20 >&2
    die "af_run reported errors (run.out)"
fi

echo "== checkpnr (actual placement vs balls.txt, unused balls)"
python3 -I "$HERE/checkpnr.py" --gen "$GEN" --work "$WORK" --pinmap "$PINMAP" \
    ${PLACEMENT:+--placement "$PLACEMENT" --placement-regex "$PLACEMENT_RE"} \
    ${SITES_REF:+--sites-ref "$SITES_REF"} \
    ${UNUSED_OK[@]+"${UNUSED_OK[@]}"} --report "$WORK/checkpnr.txt" ||
    die "post-route check failed (checkpnr.txt): do not use this bitstream"

GATE="not run (build.sh --gate-sim)"
if [ "$GATE_SIM" = 1 ]; then
    echo "== gatesim (routed netlist, Supra's models)"
    python3 -I "$HERE/gatesim.py" --gen "$GEN" --work "$WORK" --supra "$SUPRA_HOME" --ms "$GATE_MS" $GATE_DONE ||
        die "gate-level check failed or could not run (gatesim.out)"
    GATE=$(grep '^GATESIM' gatesim.out | paste -sd ' ')
fi

lzma=$(xz --format=raw --lzma1=$XZ_FILTER -c "$RBF" | wc -c)
read -r CK CKN _ < <(cksum "$RBF")

mkdir -p "$OUT"
cp "$RBF" "$OUT/pinfind_ag16k.rbf"
[ ! -f "$WORK/checkpnr.sites" ] || cp "$WORK/checkpnr.sites" "$OUT/pinfind_ag16k.sites"
{
    tr -d '\r' <"$GEN/params.txt"
    echo "rbf_bytes $CKN"
    echo "rbf_cksum $CK"
} >"$OUT/pinfind_ag16k.params"
sha() { sha256sum "$1" | cut -d' ' -f1; }
{
    if [ "$STAGE0" = 1 ]; then
        echo "*** PIN-FINDER STAGE 0 bitstream: no outputs (every ball an input). Read fpga/pinfind/README_ja.md before loading it. ***"
    else
        echo "*** PIN-FINDER bitstream: drives one ball at a time. Read fpga/pinfind/README_ja.md (safety) before loading it. ***"
    fi
    echo "file        pinfind_ag16k.rbf"
    echo "bytes       $size"
    echo "sha256      $(sha "$RBF")"
    echo "cksum       $CK $CKN"
    echo "lzma        $lzma bytes"
    echo "supra       $(ns af_cmd --version 2>/dev/null | head -1)"
    echo "seed        ${SEED:-default}"
    echo "retiming    $RETIMED (af_map abc -dff before dfflegalize; must be off, README_ja.md)"
    echo "top         sha256 $(sha PinFind.v)  asf $(sha $DESIGN.asf)"
    echo
    echo "--- checks passed by build.sh"
    echo "rbf size/header, no Supra errors, no automatic placement, pin count $TOTAL_PINS"
    cat "$WORK/checkpnr.txt"
    echo "gate-level  $GATE"
    echo
    echo "--- NOT checked (a human must; README_ja.md 0 and 6)"
    echo "that Supra's placement report matches the silicon; unused-pin state of any --unused-ok ball;"
    echo "electrical behaviour; the board (PS pins, power, pull-ups); the bitstream on hardware."
    echo
    echo "--- what Supra's log says about unused pins (for the record)"
    grep -i 'unused\|reserve' run.out | head -10 || echo "(nothing)"
    echo
    echo "--- params.txt (gen.py)"
    tr -d '\r' <"$GEN/params.txt"
    echo
    grep -A10 'Packing Statistics' run.out || true
    echo
    grep -i 'osc\|loop' run.out | head -20 || true
} >"$OUT/pinfind_ag16k.txt"

echo
cat "$OUT/pinfind_ag16k.txt"
echo
echo "-> $OUT/pinfind_ag16k.rbf (+ .params, .txt)"
