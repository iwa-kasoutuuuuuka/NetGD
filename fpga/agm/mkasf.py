#!/usr/bin/env python3
"""Make the PROVISIONAL pin file (pins/PROVISIONAL.asf). Not the GDEMU 6.0 pinout.

usage: mkasf.py $SUPRA_HOME/etc/AG16KDDF256.pinmap CLK11_PIN CLK48_PIN > body.asf

The GDEMU 6.0 pinout is unknown. To run the flow at all (bitstream size,
resources, rough timing) every port still needs a ball, so this puts the two
clocks on the dedicated clock inputs named on the command line and spreads the
other 64 ports over plain user I/O balls in the order Supra's pinmap lists
them: the MCU bus from the start of the list, the G1 bus from the middle.
The pinmap is read from the Supra installation; it is not copied into the repo.

pins/PROVISIONAL.asf is this script's output for PIN_J4 / PIN_H16 under a
marker header. Only rerun it to move the clock pins.
"""
import sys

pm, clk11, clk48 = sys.argv[1:4]
plain = []
for line in open(pm):
    w = line.split()
    if len(w) == 2 and w[0].startswith("PIN_"):   # bonded user I/O with no special function
        plain.append(w[0])
ports = (["MCU_RSTn", "MCU_NCS", "MCU_NRE", "MCU_NWE", "MCU_NBS[0]", "MCU_NBS[1]"]
         + [f"MCU_ADDR[{i}]" for i in range(1, 9)] + [f"MCU_DATA[{i}]" for i in range(16)] + ["MCU_IRQn"]
         + ["DC_CDCLK", "DC_SCK", "DC_SDAT", "DC_LRCK", "DC_EMPH", "DC_RSTn", "DC_CSn[0]", "DC_CSn[1]",
            "DC_WRn", "DC_RDn"] + [f"DC_ADDR[{i}]" for i in range(3)] + [f"DC_DATA[{i}]" for i in range(16)]
         + ["DC_DMARQ", "DC_DMACKn", "DC_IORDY", "DC_INTRQ"])
assert len(ports) == 64 and len(plain) >= 64
print(f"set_location_assignment -to CLK_11_2896_MHz {clk11}")
print(f"set_location_assignment -to CLK_48_MHz {clk48}")
half = len(plain) // 2
for p, loc in zip(ports[:31], plain[:31]):
    print(f"set_location_assignment -to {{{p}}} {loc}")
for p, loc in zip(ports[31:], plain[half:half + 33]):
    print(f"set_location_assignment -to {{{p}}} {loc}")
