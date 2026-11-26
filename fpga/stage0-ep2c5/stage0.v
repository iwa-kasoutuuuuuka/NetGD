// stage-0 for the GDEMU 6.0 FPGA, built as a Cyclone II EP2C5F256C8.
//
// No ports and no logic on purpose: once the FPGA is configured, every
// user pin must be an input with its output buffer off (stage0.qsf). The
// only point of this image is to see CONF_DONE rise without the FPGA
// driving anything on the board.
module stage0();
endmodule
