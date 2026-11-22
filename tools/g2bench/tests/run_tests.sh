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
# g2x (custom UDP transfer): codecs, receiver window, the protocol over a simulated lossy link (C), and the
# Python sender over localhost UDP through a lossy shim; xport_vec.bin cross-checks the two codecs
XSRC="../g2x_core.c ../../netgd-recv/src/crc32.c"
gcc $FLAGS -O1 -fsanitize=address,undefined -o test_xport test_xport.c $XSRC 2>/dev/null \
  || gcc $FLAGS -O1 -o test_xport test_xport.c $XSRC
./test_xport xport_vec.bin
python test_g2x.py xport_vec.bin
rm -f xport_vec.bin
