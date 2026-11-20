#!/bin/sh
# host test build without make (same as Makefile.host)
cd "$(dirname "$0")" && mkdir -p build && gcc -O1 -g -Wall -Wno-unused-function -Wno-format-truncation -DNG_HOST -Isrc -Ithird_party/fatfs -Itest -o build/test_host src/crc32.c src/manifest.c src/ngblk.c src/ngfs.c src/recv_core.c src/xport_pc.c src/diskio.c third_party/fatfs/ff.c third_party/fatfs/ffunicode.c third_party/fatfs/ffsystem.c test/dev_file.c test/fwctl_sim.c test/plat_host.c test/test_host.c
