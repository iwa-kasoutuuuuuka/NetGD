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
# phase D (g2netbench): NGTP frames, WANT/DATA bookkeeping, UDP packets, and the Python sender against a fake Dreamcast
NSRC="../g2n_core.c ../g2b_core.c ../../netgd-recv/src/crc32.c"
gcc $FLAGS -fsanitize=address,undefined -o test_net test_net.c $NSRC 2>/dev/null \
  || gcc $FLAGS -o test_net test_net.c $NSRC
./test_net xcheck.bin
python test_sender.py xcheck.bin
rm -f xcheck.bin
