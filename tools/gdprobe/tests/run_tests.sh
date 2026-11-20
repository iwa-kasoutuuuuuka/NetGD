#!/bin/sh
# Host tests without make:  sh tests/run_tests.sh   (needs gcc and python 3)
set -e
cd "$(dirname "$0")"
SRC="../hash.c ../record.c ../whitelist.c ../cfg.c ../pcout.c ../probes.c"
FLAGS="-std=gnu11 -Wall -Wextra -Wno-unused-parameter -g"
gcc $FLAGS -fsanitize=address,undefined -o test_all test_all.c $SRC 2>/dev/null \
  || gcc $FLAGS -o test_all test_all.c $SRC
GDPROBE_DUMP=/tmp/gdprobe_sample.txt ./test_all
GDPROBE_SAMPLE=/tmp/gdprobe_sample.txt python test_compare.py
