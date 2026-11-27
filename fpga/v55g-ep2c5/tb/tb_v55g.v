// Top-level check of v55g.v with the real v5.5 core (Icarus Verilog):
//   iverilog -g2005 -o tbv.out fpga/v55g-ep2c5/tb/tb_v55g.v \
//       fpga/v55g-ep2c5/tb/audio_pll_stub.v fpga/v55g-ep2c5/v55g.v \
//       fpga/v55g-ep2c5/g1gate.v fpga/amaranth/out/OpenGDEMUCore.v && vvp -n tbv.out
// Prints "TB PASS".
//
//  * after configuration, with H16 / J2 running, MCU reads in normal and
//    wrapper mode and Holly AltStatus polling: no G1 ball is ever driven
//    (every DC_* bidir stays 'z'), MCU_IRQn stays 'z', the core is in reset;
//  * MCU_DATA is driven only while NCS and NRD are both low;
//  * wrapper mode: the ID word comes from g1gate and the core's CS input
//    stays 0 for every wrapper read (no collision with the core map);
//  * core_run (0xB4) releases the core's reset only with MCU_RSTn high;
//    G1 stays undriven without a group enable.
`timescale 1ns/1ps
module tb;
    reg CLK_48 = 0, CLK_11 = 0;
    always #10.4165 CLK_48 = ~CLK_48;
    always #44.2885 CLK_11 = ~CLK_11;

    reg MCU_RSTn = 0, MCU_NCS = 1, MCU_NRE = 1, MCU_NWE = 1;
    reg [1:0] MCU_NBS = 2'b00;
    reg [8:1] MCU_ADDR = 0;
    wire [15:0] MCU_DATA;
    wire MCU_IRQn;
    wire DC_CDCLK, DC_SCK, DC_SDAT, DC_LRCK, DC_EMPH;
    reg DC_RSTn = 1, DC_WRn = 1, DC_RDn = 1, DC_DMACKn = 1;
    reg [1:0] DC_CSn = 2'b11;
    reg [2:0] DC_ADDR = 0;
    wire [15:0] DC_DATA;
    wire DC_DMARQ, DC_IORDY, DC_INTRQ;
    reg G_MASTERn = 1, G_WRAPn = 1;

    v55g dut (
        .CLK_11_2896_MHz(CLK_11), .CLK_48_MHz(CLK_48),
        .MCU_RSTn(MCU_RSTn), .MCU_NCS(MCU_NCS), .MCU_NRE(MCU_NRE), .MCU_NWE(MCU_NWE),
        .MCU_NBS(MCU_NBS), .MCU_ADDR(MCU_ADDR), .MCU_DATA(MCU_DATA), .MCU_IRQn(MCU_IRQn),
        .DC_CDCLK(DC_CDCLK), .DC_SCK(DC_SCK), .DC_SDAT(DC_SDAT), .DC_LRCK(DC_LRCK),
        .DC_EMPH(DC_EMPH), .DC_RSTn(DC_RSTn), .DC_CSn(DC_CSn), .DC_WRn(DC_WRn),
        .DC_RDn(DC_RDn), .DC_ADDR(DC_ADDR), .DC_DATA(DC_DATA), .DC_DMARQ(DC_DMARQ),
        .DC_DMACKn(DC_DMACKn), .DC_IORDY(DC_IORDY), .DC_INTRQ(DC_INTRQ),
        .G_MASTERn(G_MASTERn), .G_WRAPn(G_WRAPn));

    integer errs = 0;
    task fail(input [8*80-1:0] msg);
        begin errs = errs + 1; $display("FAIL @%0t: %0s", $time, msg); end
    endtask

    wire [23:0] g1 = {DC_EMPH, DC_LRCK, DC_SDAT, DC_SCK, DC_CDCLK, DC_DATA,
                      DC_IORDY, DC_DMARQ, DC_INTRQ};
    reg in_wrap_read = 0;
    initial #0.5 forever #1 begin
        if (g1 !== {24{1'bz}}) fail("a G1 ball is driven");
        if (MCU_IRQn !== 1'bz) fail("MCU_IRQn driven");
        if (!(~MCU_NCS & ~MCU_NRE) && MCU_DATA !== 16'hzzzz) fail("MCU_DATA driven outside a read");
        if (in_wrap_read && dut.core.io_MCU_CS !== 1'b0) fail("core saw CS in wrapper mode");
    end

    reg [15:0] rv;
    task rd(input [7:0] a);
        begin
            MCU_ADDR = a;
            in_wrap_read = ~G_WRAPn;
            #12 MCU_NCS = 0;
            #5 MCU_NRE = 0;
            #85 rv = MCU_DATA;
            MCU_NRE = 1;
            #5 MCU_NCS = 1;
            #40 in_wrap_read = 0;
        end
    endtask
    task holly_alt;
        begin
            DC_ADDR = 3'd6; DC_CSn = 2'b01;
            #40 DC_RDn = 0;
            #100 DC_RDn = 1;
            #15 DC_CSn = 2'b11;
            #30;
        end
    endtask

    integer i;
    initial begin
        #2000;
        // normal mode, core in reset (MCU_RSTn low and core_run 0)
        for (i = 0; i < 16; i = i + 1) rd(i);
        for (i = 0; i < 10; i = i + 1) holly_alt;
        if (dut.core.rst !== 1'b1) fail("core not in reset");
        // MCU_RSTn high alone does not release the core
        MCU_RSTn = 1;
        #500;
        if (dut.core.rst !== 1'b1) fail("core left reset without core_run");
        MCU_RSTn = 0;
        // wrapper mode
        G_WRAPn = 0;
        #200;
        rd(8'h00); if (rv !== 16'h5C55) fail("wrapper ID through MCU_DATA");
        rd(8'h33); if (rv !== 16'h0008) fail("walking 1 through MCU_DATA");
        for (i = 0; i < 256; i = i + 1) if (i < 8'h40 || i == 8'hB3) rd(i);
        G_MASTERn = 0;
        #200;
        rd(8'h02);
        if (rv[5] !== 1'b1 || rv[6] !== 1'b1) fail("status: master / wrap");
        for (i = 0; i < 10; i = i + 1) holly_alt;
        // core_run with MCU_RSTn high: core leaves reset, G1 still off
        rd(8'hB4); #100;
        MCU_RSTn = 1;
        #500;
        if (dut.core.rst !== 1'b0) fail("core_run + MCU_RSTn did not release reset");
        for (i = 0; i < 10; i = i + 1) holly_alt;
        rd(8'hB5); #100;
        if (dut.core.rst !== 1'b1) fail("core_run off did not reset the core");
        MCU_RSTn = 0;
        G_MASTERn = 1;
        G_WRAPn = 1;
        #1000;
        if (errs == 0) $display("TB PASS");
        else $display("TB FAIL %0d", errs);
        $finish;
    end
endmodule
