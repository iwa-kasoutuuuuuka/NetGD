// NetGD pin-finder: where the core clock comes from. Exactly one of
// PF_CLK_OSC / PF_CLK_RING / PF_CLK_PIN is defined by gen.py.
//
// PF_CLK_OSC   AGM's on-chip oscillator primitive `alta_osc` (i_osc_enb,
//              o_osc). Named in Supra's primitive library; the simulation
//              model there toggles only while i_osc_enb is 0, so the enable
//              is taken as active low. UNVERIFIED: that the primitive can be
//              placed on AG16KDDF256, the polarity, and the frequency (the
//              UFM model's comment suggests a few MHz; nothing is published).
// PF_CLK_RING  A ring of STAGES inverters built from explicit LUT cells
//              (cycloneive_lcell_comb, the cell af_map emits), so Yosys
//              cannot optimise the loop away. UNVERIFIED: whether af places
//              and routes a combinational loop at all; frequency set by
//              routing, guessed 5..200 MHz.
// PF_CLK_PIN   A ball used as a clock input (`pin`), e.g. DCLK (G1) toggled
//              by the MCU after configuration, or a dedicated clock input
//              once its source is known.
//
// PF_CLKCTRL   Put the clock on a global network through an explicit
//              cycloneive_clkctrl with ena = 1 (a second-hand report says
//              Yosys otherwise inserts one with enable tied low, which af
//              rejects).
module pf_clock #(parameter STAGES = 31) (
    input  pin,
    output clk
);
    wire raw;
`ifdef PF_CLK_OSC
    alta_osc u_osc (.i_osc_enb(1'b0), .o_osc(raw));
`elsif PF_CLK_RING
    wire [STAGES:0] n;
    genvar i;
    generate
        for (i = 0; i < STAGES; i = i + 1) begin : stage
            cycloneive_lcell_comb #(
                .lut_mask(16'h5555),          // combout = ~dataa
                .sum_lutc_input("datac")
            ) inv (
                .dataa(n[i]), .datab(1'b1), .datac(1'b1), .datad(1'b1),
                .cin(1'b0), .combout(n[i + 1])
            );
        end
    endgenerate
    assign n[0] = n[STAGES];
    // Divide by two: a clean 50 % duty cycle whatever the ring does.
    reg half = 1'b0;
    always @(posedge n[0]) half <= ~half;
    assign raw = half;
`elsif PF_CLK_PIN
    assign raw = pin;
`else
    initial $fatal(1, "pf_clock: define PF_CLK_OSC, PF_CLK_RING or PF_CLK_PIN");
`endif

`ifdef PF_CLKCTRL
    cycloneive_clkctrl u_gclk (
        .inclk({3'b000, raw}), .clkselect(2'b00), .ena(1'b1), .outclk(clk)
    );
`else
    assign clk = raw;
`endif
endmodule
