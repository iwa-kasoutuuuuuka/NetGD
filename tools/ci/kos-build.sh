#!/bin/sh
# Runs INSIDE the pcercuei/dreamcast-toolchain container (see .github/workflows/kos.yaml).
#   /kos   = KallistiOS checkout (writable)      /work = this repo (writable)
# Builds KOS once (skipped when the cache already holds lib/libkallisti.a), then every
# tools/*/Makefile that includes Makefile.rules; ELFs are copied to /work/out.
set -eu

export HOME=/tmp
cd /kos
if [ ! -f environ.sh ]; then
    cp doc/environ.sh.sample environ.sh
    sed -i 's|^export KOS_BASE=.*$|export KOS_BASE="/kos"|' environ.sh
fi
# environ.sh is not written for `set -u`
set +u
. ./environ.sh
set -u
echo "KOS_BASE=$KOS_BASE  CC=$(command -v sh-elf-gcc)"
sh-elf-gcc --version | head -1

if [ ! -f lib/dreamcast/libkallisti.a ]; then
    echo "::group::build KallistiOS"
    make -j"$(nproc)"
    echo "::endgroup::"
else
    echo "KallistiOS restored from cache"
fi

mkdir -p /work/out
fail=0
for mk in /work/tools/*/Makefile; do
    [ -f "$mk" ] || continue
    grep -q 'Makefile.rules' "$mk" || continue
    d=$(dirname "$mk")
    echo "::group::build $(basename "$d")"
    if (cd "$d" && make clean >/dev/null 2>&1; cd "$d" && make HOST_DIR_REL=/ci); then
        cp "$d"/*.elf /work/out/
    else
        echo "::error::build of $d failed"
        fail=1
    fi
    echo "::endgroup::"
done
exit $fail
