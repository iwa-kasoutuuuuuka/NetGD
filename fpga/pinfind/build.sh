#!/usr/bin/env bash
# NetGD: build the AG16K pin-finder bitstream with AGM Supra (WSL / Linux).
#
#   SUPRA_HOME=/path/to/Supra-2026.03.b0 fpga/pinfind/build.sh [options]
#
# Input: the files gen.py wrote into fpga/pinfind/build/ (PinFind.v,
# PinFindCore.v, PinFind.asf, PinFind.sdc, params.txt). Run gen.py first
# (Windows side, Amaranth venv), see README_ja.md.
#
#   --gen DIR     where gen.py wrote its files (default fpga/pinfind/build)
#   --work DIR    scratch directory (default fpga/pinfind/build/supra; wiped)
#   --seed N      place-and-route seed
#   --timeout S   give up on place-and-route after S seconds (default 1200)
#
# Output: fpga/pinfind/output/pinfind_ag16k.rbf (the name the `pinfind`
# firmware build embeds) and pinfind_ag16k.txt (hashes, parameters, Supra's
# resource report).
#
# Same rules as fpga/agm/build.sh: every Supra program runs inside
# `unshare -rn` (no network); only af_setup, af_map, af_run and af_cmd
# --version are started; nothing is written into or copied out of the Supra
# tree except the bitstream for this design.
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

die() { echo "build.sh: $*" >&2; exit 1; }

while [ $# -gt 0 ]; do
    case $1 in
        --gen) GEN=$2; shift 2 ;;
        --work) WORK=$2; shift 2 ;;
        --seed) SEED=$2; shift 2 ;;
        --timeout) AF_TIMEOUT=$2; shift 2 ;;
        -h|--help) sed -n '2,22p' "$0"; exit 0 ;;
        *) die "unknown argument: $1" ;;
    esac
done
WORK=${WORK:-$GEN/supra}

[ -n "${SUPRA_HOME:-}" ] || die "set SUPRA_HOME to the Supra installation (the directory that holds bin/af)"
SUPRA_HOME=$(cd "$SUPRA_HOME" && pwd)
for t in af_cmd af_setup af_map af_run; do
    [ -x "$SUPRA_HOME/bin/$t" ] || die "$SUPRA_HOME/bin/$t not found"
done
for t in unshare timeout python3 xz sha256sum; do
    command -v "$t" >/dev/null || die "\`$t\` is needed"
done
unshare -rn true 2>/dev/null || die "\`unshare -rn\` does not work here; Supra is only ever run without network"
for f in PinFind.v PinFindCore.v PinFind.asf PinFind.sdc params.txt; do
    [ -f "$GEN/$f" ] || die "$GEN/$f not found: run gen.py first (README_ja.md)"
done
case ${SEED:-0} in *[!0-9]*) die "--seed takes a number" ;; esac
case $AF_TIMEOUT in ''|*[!0-9]*) die "--timeout takes seconds" ;; esac

param() { tr -d '\r' <"$GEN/params.txt" | sed -n "s/^$1 //p"; }
N_DRIVE=$(param n_drive)
TOTAL_PINS=$(param total_pins)
CLOCK=$(param clock)
[ -n "$N_DRIVE" ] && [ -n "$TOTAL_PINS" ] || die "params.txt is incomplete"

ns() { unshare -rn env SUPRA="$SUPRA_HOME" PATH="$SUPRA_HOME/bin:/usr/bin:/bin" "$@"; }

if [ -e "$WORK" ]; then
    [ -f "$WORK/.netgd-pinfind-work" ] || die "$WORK exists and is not a build.sh work directory; not touching it"
    rm -rf "$WORK"
fi
mkdir -p "$WORK"
WORK=$(cd "$WORK" && pwd)
: >"$WORK/.netgd-pinfind-work"
cd "$WORK"

for f in PinFind.v PinFindCore.v PinFind.sdc; do tr -d '\r' <"$GEN/$f" >"$f"; done
tr -d '\r' <"$GEN/PinFind.asf" | grep -v '^[[:space:]]*#' | grep -v '^[[:space:]]*$' >$DESIGN.asf

echo "== af_setup"
ns af_setup --design $DESIGN --device $DEVICE --verilog PinFind.v PinFindCore.v >setup.out 2>&1 ||
    { tail -20 setup.out >&2; die "af_setup failed (setup.out)"; }
echo "== af_map"
ns af_map -TQd -v 1 -L map.log -c af_map.tcl >map.out 2>&1 ||
    { tail -20 map.out >&2; die "af_map failed (map.out, map.log)"; }
# Each scanned ball is an inout (the core listens before it drives): merge
# the ibuf + obuf pair into one alta_io, as for the real core.
python3 -I "$AGM/fixio.py" $DESIGN.vqm $DESIGN.fixed.vqm --expect "$N_DRIVE"
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
grep -q "Total  *Pins *: *$TOTAL_PINS/" run.out ||
    die "pin count is not $TOTAL_PINS (run.out): a port was dropped or split. Every ball in the list must be in the design"
if grep '^Error\|^Fatal' run.out | grep -v locatiion | grep -q .; then
    grep -n '^Error\|^Fatal' run.out | grep -v locatiion | head -20 >&2
    die "af_run reported errors (run.out)"
fi
lzma=$(xz --format=raw --lzma1=$XZ_FILTER -c "$RBF" | wc -c)

OUT=$HERE/output
mkdir -p "$OUT"
cp "$RBF" "$OUT/pinfind_ag16k.rbf"
sha() { sha256sum "$1" | cut -d' ' -f1; }
{
    echo "*** PIN-FINDER bitstream: drives one ball at a time. Read fpga/pinfind/README_ja.md (safety) before loading it. ***"
    echo "file        pinfind_ag16k.rbf"
    echo "bytes       $size"
    echo "sha256      $(sha "$RBF")"
    echo "lzma        $lzma bytes"
    echo "supra       $(ns af_cmd --version 2>/dev/null | head -1)"
    echo "seed        ${SEED:-default}"
    echo "top         sha256 $(sha PinFind.v)  core $(sha PinFindCore.v)  asf $(sha $DESIGN.asf)"
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
echo "-> $OUT/pinfind_ag16k.rbf"
