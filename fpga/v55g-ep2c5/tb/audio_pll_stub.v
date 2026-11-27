// Simulation stand-in for the Quartus ALTPLL Audio_PLL (x3 of the 11.2896 MHz
// crystal = 33.8688 MHz). Testbench only; the build uses Audio_PLL.v.
`timescale 1ns/1ps
module Audio_PLL (input wire inclk0, output reg c0);
    initial c0 = 1'b0;
    always #14.7628 c0 = ~c0;
endmodule
