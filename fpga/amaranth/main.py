#!/usr/bin/env python3
"""Generate Verilog for the OpenGDEMU FPGA core.

Run from the fpga/amaranth directory:

    amaranth_env/bin/python3 main.py                  # Cyclone II (v5.5)
    amaranth_env/bin/python3 main.py --target ag16k   # AGM AG16K (6.0)

Output goes to ./out/. The Quartus project picks up the default file
(out/OpenGDEMUCore.v) via fpga/Quartus/OpenGDEMU.qsf. `--target ag16k`
writes out/OpenGDEMUCore_ag16k.v instead: the same core, same module name
and ports, with the data FIFOs on registered-read memories so they map to
M9K block RAM (see hw/fifo.py).
"""

import argparse
import os
import sys

# Generate with the yosys amaranth ships rather than whatever the host has
# installed. Both emit Verilog that Quartus fits to the same bitstream, but
# they format it differently, so a system yosys makes this file churn on every
# distro upgrade and disagree with the container. Set before importing the
# backend, which resolves yosys at import time.
os.environ.setdefault("AMARANTH_USE_YOSYS", "builtin")

from amaranth.back import verilog  # noqa: E402

from hw.fifo import TARGETS  # noqa: E402
from hw.OpenGDEMU import OpenGDEMUCore  # noqa: E402


OUT_DIR = "out"
# The default target keeps the file name the Quartus project and CI expect.
OUT_NAMES = {
    "cyclone2": "OpenGDEMUCore.v",
    "ag16k": "OpenGDEMUCore_ag16k.v",
}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    # Deliberately not taken from $NETGD_FPGA_TARGET: what this writes must
    # depend on the command line alone.
    parser.add_argument("--target", choices=TARGETS, default="cyclone2",
                        help="FPGA family to generate for (default: cyclone2)")
    args = parser.parse_args()

    os.makedirs(OUT_DIR, exist_ok=True)

    core = OpenGDEMUCore(target=args.target)
    rtl = verilog.convert(core, name="OpenGDEMUCore", ports=core.ports(), emit_src=False)
    out_path = os.path.join(OUT_DIR, OUT_NAMES[args.target])
    with open(out_path, "w") as f:
        f.write(rtl)
    print(f"wrote {out_path} ({len(rtl)} bytes)", file=sys.stderr)


if __name__ == "__main__":
    main()
