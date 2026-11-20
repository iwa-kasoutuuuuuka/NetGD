#!/bin/sh
# Host tests without make:  sh tests/run_tests.sh   (needs gcc; python 3 is optional)
#   FWIMAGE_ELF_2E=<elf> FWIMAGE_ELF_4E=<elf>  additionally build and send images made from real firmware ELFs
#   (each cargo build overwrites the ELF: copy it aside first, see tools/fwimage/README_ja.md)
set -e
cd "$(dirname "$0")"
SRC="../fwcore.c ../fwmsg.c ../resout.c"
FLAGS="-std=gnu11 -Wall -Wextra -g"
PY=""
for c in python3 python; do
    if command -v "$c" >/dev/null 2>&1 && "$c" -c 'import sys; sys.exit(sys.version_info < (3, 8))' 2>/dev/null; then PY="$c"; break; fi
done

# the message table is generated: it must match fwmsg.def
if [ -n "$PY" ]; then
    "$PY" ../gen_msgs.py --check
else
    echo "(no python: skipping the fwmsg_gen.h up-to-date check)"
fi

gcc $FLAGS -fsanitize=address,undefined -o test_all test_all.c $SRC 2>/dev/null \
  || gcc $FLAGS -o test_all test_all.c $SRC

GOLD="${TMPDIR:-/tmp}/netgd-fwupd-golden.$$"
if [ -n "$PY" ]; then
    mkdir -p "$GOLD"
    "$PY" gen_golden.py "$GOLD"
    FWUPD_GOLDEN_DIR="$GOLD" ./test_all
    rm -rf "$GOLD"
else
    ./test_all
fi
