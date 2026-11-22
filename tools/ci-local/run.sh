#!/bin/sh
# Local mirror of .github/workflows/build.yaml (firmware builds, host tests,
# gdprobe C tests, tools tests, FPGA simulation).  See README_ja.md.
#
#   sh tools/ci-local/run.sh [--only builds,tests,tools,fpga] [--worktree PATH] [--keep-going|--fail-fast]
#
# One line per step:  name OK|FAIL|SKIP extra     Logs: $TMPDIR/ci-local-<time>/<step>.log
# Exit status 1 when any step FAILs.  Steps run one after another (never two
# cargo runs at once in one worktree: the heapless build script breaks).

ONLY="builds,tests,tools,fpga"
WT=""
FAILFAST=0
while [ $# -gt 0 ]; do
    case "$1" in
        --only) ONLY="$2"; shift 2 ;;
        --worktree) WT="$2"; shift 2 ;;
        --fail-fast) FAILFAST=1; shift ;;
        --keep-going) FAILFAST=0; shift ;;
        -h|--help) sed -n '2,10p' "$0"; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
done
if [ -z "$WT" ]; then
    WT=$(cd "$(dirname "$0")/../.." && pwd)
fi
WT=$(cd "$WT" && pwd) || { echo "bad --worktree" >&2; exit 2; }
cd "$WT" || exit 2

want() { case ",$ONLY," in *",$1,"*) return 0 ;; esac; return 1; }

TS=$(date +%Y%m%d-%H%M%S)
LOGDIR="${TMPDIR:-${TEMP:-/tmp}}/ci-local-$TS"
mkdir -p "$LOGDIR" || exit 2
echo "worktree: $WT"
echo "logs:     $LOGDIR"

# ---- tool discovery -------------------------------------------------------
HOST=$(rustc -vV 2>/dev/null | sed -n 's/^host: //p')
# Plain python for most tools; netgd-recv / netgd-menu refuse DreamSDK's MSYS
# python on purpose, so they use the Windows launcher (py -3) when present.
PYPLAIN=""
for c in python python3; do
    if command -v "$c" >/dev/null 2>&1; then PYPLAIN="$c"; break; fi
done
if command -v py >/dev/null 2>&1 && py -3 -c "import sys" >/dev/null 2>&1; then
    PYNATIVE="py -3"
else
    PYNATIVE="$PYPLAIN"
fi
ARMSIZE="${ARM_SIZE:-}"
if [ -z "$ARMSIZE" ]; then
    for c in arm-eabi-size arm-none-eabi-size /c/DreamSDK/opt/toolchains/dc/arm-eabi/bin/arm-eabi-size; do
        if command -v "$c" >/dev/null 2>&1; then ARMSIZE="$c"; break; fi
    done
fi
LIMIT_2E=122368   # text+data of the 2E resident image (docs/research/flash-budget-2e.md)

NFAIL=0; NOK=0; NSKIP=0
SUMMARY="$LOGDIR/summary.txt"
: > "$SUMMARY"
T0=$(date +%s)

report() { # name status extra
    printf '%-34s %-4s %s\n' "$1" "$2" "$3" | tee -a "$SUMMARY"
}

# size_check ELF LIMIT  -> prints extra text, returns 1 if over limit
size_check() {
    elf="$1"; lim="$2"
    [ -f "$elf" ] || { echo "no ELF"; return 1; }
    [ -n "$ARMSIZE" ] || { echo "(no arm-eabi-size)"; return 0; }
    line=$("$ARMSIZE" "$elf" | sed -n 2p)
    t=$(echo "$line" | awk '{print $1}'); d=$(echo "$line" | awk '{print $2}')
    tot=$((t + d))
    if [ -n "$lim" ]; then
        margin=$((lim - tot))
        if [ "$margin" -lt 0 ]; then echo "text=$t data=$d total=$tot OVER limit $lim by $((-margin))"; return 1; fi
        echo "text=$t data=$d total=$tot margin=$margin (limit $lim)"
    else
        echo "text=$t data=$d total=$tot"
    fi
    return 0
}

# step NAME DIR "command" [ELF LIMIT]
step() {
    name="$1"; dir="$2"; cmd="$3"; elf="$4"; lim="$5"
    if [ "$FAILFAST" = 1 ] && [ "$NFAIL" -gt 0 ]; then return; fi
    log="$LOGDIR/$name.log"
    s=$(date +%s)
    ( cd "$dir" && eval "$cmd" ) >"$log" 2>&1
    rc=$?
    e=$(( $(date +%s) - s ))
    extra="${e}s"
    if [ $rc -eq 0 ] && [ -n "$elf" ]; then
        sz=$(size_check "$elf" "$lim"); rc=$?
        extra="$extra $sz"
        echo "$sz" >>"$log"
    fi
    if [ $rc -eq 0 ]; then NOK=$((NOK+1)); report "$name" OK "$extra"
    else NFAIL=$((NFAIL+1)); report "$name" FAIL "$extra (log: $log)"; fi
}

skip() { NSKIP=$((NSKIP+1)); report "$1" SKIP "$2"; }

ELFDIR="mcu/netgd/target/thumbv7m-none-eabi"

# ---- firmware builds ------------------------------------------------------
if want builds; then
    if ! command -v cargo >/dev/null 2>&1; then
        skip builds "cargo not found"
    else
        command -v xz >/dev/null 2>&1 || report "xz" FAIL "build.rs needs xz in PATH"
        step build-fwupdater mcu/fwupdater "cargo build --release"
        step build-netgd mcu/netgd "cargo build --release" "$ELFDIR/release/netgd"
        step build-netgd-resident-4e mcu/netgd "cargo build --release --features resident" "$ELFDIR/release/netgd"
        step build-netgd-resident-2e mcu/netgd \
            "DEFMT_LOG=off cargo build --release --no-default-features --features board-v60,resident" "$ELFDIR/release/netgd"
        step build-netgd-2e-resident-z mcu/netgd \
            "DEFMT_LOG=off cargo build --profile release-z --features sam3u2e,resident,sd-fast" "$ELFDIR/release-z/netgd" "$LIMIT_2E"
        step build-netgd-2e mcu/netgd \
            "DEFMT_LOG=off cargo build --release --features sam3u2e" "$ELFDIR/release/netgd"
        step build-netgd-pinfind mcu/netgd \
            "cargo build --release --no-default-features --features pinfind" "$ELFDIR/release/netgd"
    fi
fi

# ---- host tests (Rust crates + gdprobe C tests) ---------------------------
if want tests; then
    if ! command -v cargo >/dev/null 2>&1; then
        skip tests-cargo "cargo not found"
    else
        for c in common/lzma common/gamecfg common/discimg common/psload mcu/bootcore \
                 common/sdfast common/fatmap common/stackmark common/pinfind; do
            step "test-$(echo "$c" | tr / -)" "$c" "cargo test"
        done
        # .cargo/config.toml defaults to thumbv7m-none-eabi: name the host target.
        step test-mcu-fwupdater mcu/fwupdater "cargo test --lib --target $HOST"
    fi
    if command -v gcc >/dev/null 2>&1 && [ -n "$PYPLAIN" ]; then
        # the tests write binaries/dumps into tests/: run on a scratch copy (mksdcard is imported by the tests)
        step test-gdprobe-host "$WT/tools/gdprobe"             "rm -rf '$LOGDIR/ws' && mkdir -p '$LOGDIR/ws/tools' && cp -r '$WT/tools/gdprobe' '$WT/tools/mksdcard' '$LOGDIR/ws/tools/' && cd '$LOGDIR/ws/tools/gdprobe' && sh tests/run_tests.sh"
    else
        skip test-gdprobe-host "gcc or python missing"
    fi
fi

# ---- tools tests ----------------------------------------------------------
if want tools; then
    if [ -z "$PYPLAIN" ]; then
        skip tools "no python"
    else
        FWBACKUP_TEST_WRAPPER=1; export FWBACKUP_TEST_WRAPPER
        step tools-mksdcard tools/mksdcard "$PYPLAIN tests/test_mksdcard.py"
        step tools-netgd-send tools/netgd-send "$PYPLAIN -m unittest discover -s tests -v"
        step tools-fwbackup tools/fwbackup "$PYPLAIN -m unittest discover -s tests -p 'test_*.py'"
        step tools-fwimage . "$PYPLAIN -m unittest discover -s tools/fwimage -v"
        if command -v gcc >/dev/null 2>&1; then
            step tools-netgd-recv . "CI=true $PYNATIVE tools/netgd-recv/test/run_tests.py"
            step tools-netgd-menu . "$PYNATIVE tools/netgd-menu/test/run_tests.py"
        else
            skip tools-netgd-recv "gcc missing"; skip tools-netgd-menu "gcc missing"
        fi
        step tools-pinfind . "$PYPLAIN -m unittest discover -s tools/pinfind/tests -v"
        # not in the CI matrix, but present in the tree (needs cargo)
        if command -v cargo >/dev/null 2>&1; then
            step tools-netgd-expect . "$PYPLAIN tools/netgd-expect/test_expect.py"
        else
            skip tools-netgd-expect "cargo not found"
        fi
        # not in the CI matrix; its gen_msgs.py --check compares bytes, so it
        # fails on a CRLF (autocrlf) checkout.  Opt in with CI_LOCAL_FWUPD=1.
        if [ "${CI_LOCAL_FWUPD:-0}" = 1 ] && command -v gcc >/dev/null 2>&1; then
            step tools-netgd-fwupd "$WT/tools/netgd-fwupd" "sh tests/run_tests.sh"
        else
            skip tools-netgd-fwupd "not in CI; fails on CRLF checkout (CI_LOCAL_FWUPD=1 to run)"
        fi
    fi
fi

# ---- FPGA simulation (needs amaranth==0.5.8; see README_ja.md) -------------
if want fpga; then
    FPY="${CI_LOCAL_FPGA_PYTHON:-$PYNATIVE}"
    if [ -n "$FPY" ] && $FPY -c "import amaranth" >/dev/null 2>&1; then
        export AMARANTH_USE_YOSYS=builtin
        for t in test.py test_cdda.py test_timing.py test_top.py; do
            step "fpga-$t" fpga/amaranth "$FPY $t"
        done
        step fpga-verilog-check fpga/amaranth "$FPY main.py && git diff --exit-code -- out/OpenGDEMUCore.v"
        for t in test.py test_cdda.py test_timing.py test_top.py; do
            step "fpga-ag16k-$t" fpga/amaranth "NETGD_FPGA_TARGET=ag16k $FPY $t"
        done
        step fpga-fifo-equiv fpga/amaranth "$FPY test_fifo.py"
        step fpga-pinfind fpga/pinfind "$FPY test_pinfind.py"
        step fpga-ag16k-verilog-check fpga/amaranth "$FPY main.py --target ag16k && git diff --exit-code -- out/OpenGDEMUCore_ag16k.v"
    else
        skip fpga "amaranth not importable (set CI_LOCAL_FPGA_PYTHON to a venv python)"
    fi
fi

T=$(( $(date +%s) - T0 ))
echo "----"
echo "OK=$NOK FAIL=$NFAIL SKIP=$NSKIP total=${T}s ($((T/60))m$((T%60))s)  logs: $LOGDIR" | tee -a "$SUMMARY"
[ "$NFAIL" -eq 0 ]
