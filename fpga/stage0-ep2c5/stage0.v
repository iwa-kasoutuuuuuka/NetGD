// Stage-0 "do nothing" design for an EP2C5-compatible die (GDEMU 6.0 probe).
//
// One input port and NO outputs or bidirs. The input drives nothing, so the
// fitter keeps it as a plain input buffer and no logic is placed. Quartus II
// 13.0 refuses a design with zero pins, which is the only reason the port is
// here. Everything else on the device is reserved as input tri-stated by
// stage0.qsf, and tools/pinfind/ep2c5gate.py checks the fitter's .pin file
// and fit.rpt to prove it.
module stage0 (
    input wire stage0_in
);
endmodule
