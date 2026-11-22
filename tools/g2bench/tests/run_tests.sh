#!/bin/sh
# Host tests without make:  sh tests/run_tests.sh   (needs gcc and python 3)
set -e
cd "$(dirname "$0")"
SRC="../g2b_core.c ../../netgd-recv/src/crc32.c"
FLAGS="-std=gnu11 -Wall -Wextra -g"
gcc $FLAGS -fsanitize=address,undefined -o test_all test_all.c $SRC 2>/dev/null \
  || gcc $FLAGS -o test_all test_all.c $SRC
python ../mkdata.py g2bench_test.bin 2
G2B_TESTFILE=g2bench_test.bin ./test_all
rm -f g2bench_test.bin
