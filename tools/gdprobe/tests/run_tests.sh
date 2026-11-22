#!/bin/sh
# Host tests without make:  sh tests/run_tests.sh   (needs gcc and python 3)
set -e
cd "$(dirname "$0")"
SRC="../hash.c ../record.c ../whitelist.c ../cfg.c ../pcout.c ../probes.c"
FLAGS="-std=gnu11 -Wall -Wextra -Wno-unused-parameter -g"
gcc $FLAGS -fsanitize=address,undefined -o test_all test_all.c $SRC 2>/dev/null \
  || gcc $FLAGS -o test_all test_all.c $SRC
GDPROBE_DUMP=gdprobe_sample.txt GDPROBE_DUMP_R148=gdprobe_sample_r148.txt ./test_all
GDPROBE_SAMPLE=gdprobe_sample.txt GDPROBE_SAMPLE_R148=gdprobe_sample_r148.txt python test_compare.py
