#!/usr/bin/env bash
# NetGD: build the AG16K (GDEMU 6.0) FPGA bitstream with AGM Supra.
#
#   SUPRA_HOME=/path/to/Supra-2026.03.b0 fpga/agm/build.sh [options]
#
#   --pins FILE   pin assignment (.asf). Without it the PROVISIONAL placeholder
#                 pins/PROVISIONAL.asf is used and the result is named
#                 OpenGDEMU_ag16k-provisional.rbf, which the firmware build
#                 does not pick up. Only a pin file without the provisional
#                 marker produces output/OpenGDEMU_ag16k.rbf.
#   --seed N      place-and-route seed (Supra's default otherwise).
#   --core FILE   core Verilog (default ../amaranth/out/OpenGDEMUCore_ag16k.v).
#   --work DIR    scratch directory (default fpga/agm/build; wiped each run).
#   --timeout S   give up on place-and-route after S seconds (default 1200).
#                 Supra's router can circle on a last conflict without ending;
#                 run again (the result differs from run to run) or try a seed.
#   --supra-retiming
#                 leave af_map's RETIMING step (Yosys `abc -dff` before
#                 dfflegalize) on. For testing only: the result is broken (below).
#
# af_map.tcl runs `abc -markgroups -dff -D 100` ("RETIMING") by default. In
# Supra 2026.03.b0 (Yosys 0.61) that pass hands each flop to ABC as a plain
# latch (Q <= D every clock) and puts its enable / sync reset back afterwards,
# so ABC's scorr / retime work on a wrong model. In this core:
#   - cdda.pair_valid (set to constant 1 under its enable, cleared by its sync
#     reset) is folded away: the audio FIFO is never read, CDDA is silent and
#     no underflow is counted (gate-level simulation of the mapped .vqm);
#   - $sdffce (reset only while enabled) comes back as $sdffe (reset always
#     wins): cdda.shift[0] (the right sample's LSB would always be 0) and
#     cdda.sdat (no visible effect there);
#   - power-up values move (cdda.phase starts elsewhere; harmless there).
# build.sh switches it off through OpenGDEMU.pre_map.asf (af_map.tcl sources
# it after setting its default) and stops if map.log still shows an ABC pass
# before DFFLEGALIZE. Same fix as fpga/pinfind/build.sh.
#
# Linux / WSL only. Every Supra program runs inside `unshare -rn` (a user
# namespace with no network), and only af_cmd, af_setup, af_map and af_run are
# ever started. Nothing is written into, or copied out of, the Supra tree
# except the bitstream it generates for this design. See README_ja.md.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
MARKER=NETGD-PROVISIONAL-PINS
DESIGN=OpenGDEMU
DEVICE=AG16KDDF256
RBF_BYTES=552352            # uncompressed AG16K stream: fixed, design-independent
XZ_FILTER=preset=9e,dict=4KiB,lc=0,lp=0,pb=0   # = mcu/netgd/build.rs XZ_FILTER

PINS=$HERE/pins/PROVISIONAL.asf
CORE=$HERE/../amaranth/out/OpenGDEMUCore_ag16k.v
WORK=$HERE/build
SEED=
AF_TIMEOUT=1200
RETIME=0

die() { echo "build.sh: $*" >&2; exit 1; }

while [ $# -gt 0 ]; do
    case $1 in
        --pins) PINS=$2; shift 2 ;;
        --seed) SEED=$2; shift 2 ;;
        --core) CORE=$2; shift 2 ;;
        --work) WORK=$2; shift 2 ;;
        --timeout) AF_TIMEOUT=$2; shift 2 ;;
        --supra-retiming) RETIME=1; shift ;;
        -h|--help) sed -n '2,33p' "$0"; exit 0 ;;
        *) die "unknown argument: $1" ;;
    esac
done

[ -n "${SUPRA_HOME:-}" ] || die "set SUPRA_HOME to the Supra installation (the directory that holds bin/af)"
SUPRA_HOME=$(cd "$SUPRA_HOME" && pwd)
for t in af_cmd af_setup af_map af_run; do
    [ -x "$SUPRA_HOME/bin/$t" ] || die "$SUPRA_HOME/bin/$t not found"
done
for t in unshare timeout python3 xz sha256sum; do
    command -v "$t" >/dev/null || die "\`$t\` is needed"
done
unshare -rn true 2>/dev/null || die "\`unshare -rn\` does not work here; Supra is only ever run without network"
[ -f "$PINS" ] || die "pin file not found: $PINS"
[ -f "$CORE" ] || die "core not found: $CORE (cd fpga/amaranth && python main.py --target ag16k)"
case ${SEED:-0} in *[!0-9]*) die "--seed takes a number" ;; esac
case $AF_TIMEOUT in ''|*[!0-9]*) die "--timeout takes seconds" ;; esac

# --- provisional or real? ---------------------------------------------------
if grep -q "$MARKER" "$PINS"; then
    PROVISIONAL=1; OUTNAME=${DESIGN}_ag16k-provisional
else
    PROVISIONAL=0; OUTNAME=${DESIGN}_ag16k
    # A real pin file has to place every port, exactly once. The port list is
    # taken from the placeholder (same top level).
    sed -n 's/^set_location_assignment -to \([^ ]*\) .*/\1/p' "$HERE/pins/PROVISIONAL.asf" |
    while read -r port; do
        n=$(tr -d '\r' <"$PINS" | grep -c -F -- "set_location_assignment -to $port " || true)
        [ "$n" = 1 ] || die "$PINS: port $port is assigned $n times (need exactly 1)"
    done
fi

# Supra runs here and nowhere else: no network, PATH cut down to Supra + coreutils.
ns() { unshare -rn env SUPRA="$SUPRA_HOME" PATH="$SUPRA_HOME/bin:/usr/bin:/bin" "$@"; }

# --- scratch directory ------------------------------------------------------
if [ -e "$WORK" ]; then
    [ -f "$WORK/.netgd-agm-work" ] || die "$WORK exists and is not a build.sh work directory; not touching it"
    rm -rf "$WORK"
fi
mkdir -p "$WORK/pll"
WORK=$(cd "$WORK" && pwd)
: >"$WORK/.netgd-agm-work"
cd "$WORK"

# Inputs, with CRs dropped (a Windows checkout has CRLF in the generated core).
tr -d '\r' <"$CORE"                    >OpenGDEMUCore_ag16k.v
tr -d '\r' <"$HERE/src/OpenGDEMU.v"   >$DESIGN.v
tr -d '\r' <"$HERE/src/OpenGDEMU.sdc" >$DESIGN.sdc
{
    tr -d '\r' <"$PINS" | grep -v '^[[:space:]]*#' | grep -v '^[[:space:]]*$'
    # The firmware LZMA-packs the stream itself; AGM's own (LZW) compression
    # makes the packed result twice as big (ag16k-supra-build.md section 6).
    echo 'set_global_assignment -name ON_CHIP_BITSTREAM_DECOMPRESSION OFF'
} >$DESIGN.asf

# --- PLL: Supra computes the settings, mkpll.py writes NetGD's wrapper --------
echo "== generate_pll"
(cd pll && tr -d '\r' <"$HERE/pll.tcl" >pll.tcl && ns af_cmd -f pll.tcl >pll.out 2>&1) ||
    { tail -20 pll/pll.out >&2; die "generate_pll failed (pll/pll.out)"; }
python3 -I "$HERE/mkpll.py" pll/Audio_PLL_PLLVE.v Audio_PLL.v

# --- setup -> map (Yosys) -> merge inout pads -> place, route, bitgen ---------
echo "== af_setup"
ns af_setup --design $DESIGN --device $DEVICE --verilog $DESIGN.v Audio_PLL.v OpenGDEMUCore_ag16k.v >setup.out 2>&1 ||
    { tail -20 setup.out >&2; die "af_setup failed (setup.out)"; }
# RETIMING off (see the top of this file): af_map.tcl sets its default
# ("100") and then sources $DESIGN.pre_map.asf, which af_setup leaves empty.
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
# Did the RETIMING `abc -dff` run? It is the only ABC pass before DFFLEGALIZE.
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
python3 -I "$HERE/fixio.py" $DESIGN.vqm $DESIGN.fixed.vqm --expect 32
mv $DESIGN.vqm $DESIGN.orig.vqm
cp $DESIGN.fixed.vqm $DESIGN.vqm
echo "== af_run (about two minutes)"
# Supra prints one "Pin ...'s locatiion is not assigned" error per port before
# it reads the .asf, and still exits 0; those are expected and ignored here.
# `timeout` signals its whole process group, so `af` dies with the wrapper script.
rc=0
ns timeout -k 10 "$AF_TIMEOUT" af_run --mode NATIVE ${SEED:+--seed "$SEED"} >run.out 2>&1 || rc=$?
if [ $rc = 124 ] || [ $rc = 137 ]; then
    tail -2 run.out >&2
    die "af_run did not finish in $AF_TIMEOUT s (run.out); run again or pass another --seed"
elif [ $rc != 0 ]; then
    grep -n '^Error\|^Fatal' run.out | grep -v locatiion | head -20 >&2
    die "af_run failed (run.out)"
fi

# --- checks -------------------------------------------------------------------
RBF=${DESIGN}_slave.rbf
[ -f "$RBF" ] || die "af_run produced no $RBF"
size=$(wc -c <"$RBF")
[ "$size" = "$RBF_BYTES" ] || die "$RBF is $size bytes, expected $RBF_BYTES (compression left on?)"
head4=$(head -c 4 "$RBF" | od -An -tx1 | tr -d ' \n')
[ "$head4" = 697e3c5a ] || die "$RBF starts with $head4, expected 697e3c5a (the .rbf, bit-reversed .bin)"
grep -q 'Total  *Pins *: *66/' run.out || die "pin count is not 66 (run.out): inout pads not merged?"
grep -q 'multiply_by 3 ' run.out || die "Supra did not report the audio PLL as x3 (run.out)"
if grep '^Error\|^Fatal' run.out | grep -v locatiion | grep -q .; then
    grep -n '^Error\|^Fatal' run.out | grep -v locatiion | head -20 >&2
    die "af_run reported errors (run.out)"
fi
lzma=$(xz --format=raw --lzma1=$XZ_FILTER -c "$RBF" | wc -c)

# --- deliver ------------------------------------------------------------------
OUT=$HERE/output
mkdir -p "$OUT"
cp "$RBF" "$OUT/$OUTNAME.rbf"
sha() { sha256sum "$1" | cut -d' ' -f1; }
{
    if [ $PROVISIONAL = 1 ]; then
        echo "*** PROVISIONAL: placeholder pinout, NOT the GDEMU 6.0 pinout. DO NOT LOAD INTO A BOARD. ***"
        echo "*** 仮のピン配置で作ったビットストリーム。基板に流してはいけない。 ***"
    else
        echo "Built with a user-supplied pin file. Not verified on hardware by this script."
    fi
    echo "file        $OUTNAME.rbf"
    echo "bytes       $size"
    echo "sha256      $(sha "$RBF")"
    echo "lzma        $lzma bytes (xz --format=raw --lzma1=$XZ_FILTER)"
    echo "supra       $(ns af_cmd --version 2>/dev/null | head -1)"
    echo "device      $DEVICE"
    echo "seed        ${SEED:-default}"
    if [ "$RETIMED" = on ]; then
        echo "retiming    ON (--supra-retiming: af_map abc -dff before dfflegalize; CDDA broken, testing only, DO NOT USE)"
    else
        echo "retiming    off (af_map abc -dff before dfflegalize switched off; build.sh header)"
    fi
    echo "pins        $(basename "$PINS") sha256 $(tr -d '\r' <"$PINS" | sha256sum | cut -d' ' -f1)"
    echo "core        $(basename "$CORE") sha256(LF) $(sha OpenGDEMUCore_ag16k.v)"
    echo "wrapper     sha256 $(sha $DESIGN.v)  sdc $(sha $DESIGN.sdc)"
    echo "Audio_PLL   sha256 $(sha Audio_PLL.v) (generated)"
    echo
    grep -A10 'Packing Statistics' run.out | tail -n +1 || true
    echo
    grep 'Fmax:' alta_db/fmax.rpt || true
    grep -m1 -B1 'Slack:' alta_db/fmax.rpt || true
} >"$OUT/$OUTNAME.txt"

echo
cat "$OUT/$OUTNAME.txt"
echo
echo "-> $OUT/$OUTNAME.rbf"
if [ $PROVISIONAL = 1 ]; then
    echo "PROVISIONAL build: the firmware build does not look at this file name. Do not rename it."
fi
