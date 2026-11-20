#!/bin/sh
# Predict NetGD's gdprobe answers for one image:  tools/netgd-expect/expect.sh --image disc.gdi [--img 3] [...]
# (a thin wrapper over `cargo run --example expect` in common/discimg; all options are expect's)
cd "$(dirname "$0")/../../common/discimg" || exit 2
exec cargo run -q --example expect -- "$@"
