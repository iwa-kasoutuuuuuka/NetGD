// v55s MCU-port check with the real v5.5 core (Icarus Verilog):
//   iverilog -g2005 -o tbs.out fpga/v55s-ep2c5/tb/tb_v55s.v \
//       fpga/v55g-ep2c5/tb/audio_pll_stub.v fpga/v55s-ep2c5/v55s.v \
//       fpga/v55g-ep2c5/g1gate.v fpga/amaranth/out/OpenGDEMUCore.v && vvp -n tbs.out
// Prints "TB PASS". With -DDUT_V55G (and v55g.v instead of v55s.v) the
// same stimulus runs against v55g, for comparison (expected to fail).
//
// Bit-banged MCU cycles like the firmware's (ADDR / D set first, NCS, NWE
// or NRE low for ~0.5 us), but "aggressive": right after NWE rises the
// MCU releases D (pulled up to 0xFFFF) and moves ADDR. Checks:
//  * slot 4 (command / features) write + readback, also with that release;
//  * slot 9 FIFO pushes: exactly one dev0 read_fifo push per write, with
//    the written value (watched at the FIFO's write port);
//  * a 12 ns high glitch on NWE inside a write, and a 12 ns low glitch on
//    G_WRAPn inside a write: the write is taken exactly once;
//  * 12 ns NWE / G_WRAPn / NRE / MCU_RSTn glitches while idle: no push, no
//    write, no reset, the FPGA never drives MCU_DATA;
//  * no G1 ball is driven; MCU_DATA only inside a raw read (~NCS & ~NRE).
`timescale 1ns/1ps
module tb;
    reg CLK_48 = 0, CLK_11 = 0;
    always #10.4165 CLK_48 = ~CLK_48;
    always #44.2885 CLK_11 = ~CLK_11;

    reg MCU_RSTn = 0, MCU_NCS = 1, MCU_NRE = 1, MCU_NWE = 1;
    reg [1:0] MCU_NBS = 2'b00;
    reg [8:1] MCU_ADDR = 0;
    tri1 [15:0] MCU_DATA;           // MCU pull-ups: released D reads 1
    reg [15:0] d_drv = 16'h0000;
    reg d_en = 1'b0;
    assign MCU_DATA = d_en ? d_drv : 16'hzzzz;
    wire MCU_IRQn;
    wire DC_CDCLK, DC_SCK, DC_SDAT, DC_LRCK, DC_EMPH;
    reg DC_RSTn = 1, DC_WRn = 1, DC_RDn = 1, DC_DMACKn = 1;
    reg [1:0] DC_CSn = 2'b11;
    reg [2:0] DC_ADDR = 0;
    wire [15:0] DC_DATA;
    wire DC_DMARQ, DC_IORDY, DC_INTRQ;
    reg G_MASTERn = 1, G_WRAPn = 1;

`ifdef DUT_V55G
    v55g dut (
`else
    v55s dut (
`endif
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

    // FPGA output enable on MCU_DATA, as the top level computes it.
`ifdef DUT_V55G
    wire fpga_oe = dut.MCU_READ;
`else
    wire fpga_oe = dut.MCU_READ & dut.rd_ok;
`endif
    wire [23:0] g1 = {DC_EMPH, DC_LRCK, DC_SDAT, DC_SCK, DC_CDCLK, DC_DATA,
                      DC_IORDY, DC_DMARQ, DC_INTRQ};
    reg in_write = 0;
    initial #0.5 forever #1 begin
        if (g1 !== {24{1'bz}}) fail("a G1 ball is driven");
        if (!(~MCU_NCS & ~MCU_NRE) && fpga_oe) fail("MCU_DATA driven outside a read");
        if (in_write && fpga_oe) fail("FPGA drives MCU_DATA during a write");
    end

    // Every push into dev0's read FIFO (MCU slot 9 write).
    integer npush = 0;
    reg [15:0] last_push = 0;
    always @(posedge CLK_48)
        if (dut.core.dev0.read_fifo.w_en === 1'b1) begin
            npush = npush + 1;
            last_push = dut.core.dev0.read_fifo.w_port__data;
        end
    // Every clock with the core's (dev0) write strobe on.
    integer nwr = 0;
    always @(posedge CLK_48)
        if (dut.core.io_MCU_CS === 1'b1 && dut.core.io_MCU_WR === 1'b1) nwr = nwr + 1;

    reg [15:0] rv;
    task rd(input [7:0] a);
        begin
            MCU_ADDR = a;
            #200 MCU_NCS = 0;
            #50 MCU_NRE = 0;
            #300 rv = MCU_DATA;
            MCU_NRE = 1;
            #20 MCU_NCS = 1;
            #200;
        end
    endtask
    // gl: 0 none, 1 NWE high glitch, 2 G_WRAPn low glitch, 3 NRE low glitch
    // (all 12 ns, in the middle of NWE low). agg: release D / move ADDR
    // 5 ns after NWE rises.
    task wr(input [7:0] a, input [15:0] v, input [1:0] gl, input agg);
        begin
            MCU_ADDR = a; d_drv = v; d_en = 1; in_write = 1;
            #200 MCU_NCS = 0;
            #50 MCU_NWE = 0;
            #250;
            case (gl)
                2'd1: begin MCU_NWE = 1; #12 MCU_NWE = 0; end
                2'd2: begin G_WRAPn = 0; #12 G_WRAPn = 1; end
                2'd3: begin MCU_NRE = 0; #12 MCU_NRE = 1; end
                default: #12;
            endcase
            #250 MCU_NWE = 1;
            if (agg) begin
                #5 d_en = 0; MCU_ADDR = 8'h0A;
                #15 MCU_NCS = 1;
            end else begin
                #50 MCU_NCS = 1;
                #50 d_en = 0;
            end
            #200 in_write = 0;
        end
    endtask
    task expect_push(input integer n0, input [15:0] v, input [8*40-1:0] what);
        begin
            if (npush != n0 + 1) begin
                errs = errs + 1;
                $display("FAIL @%0t: %0s: %0d pushes, expected 1", $time, what, npush - n0);
            end else if (last_push !== v) begin
                errs = errs + 1;
                $display("FAIL @%0t: %0s: pushed %h, expected %h", $time, what, last_push, v);
            end
        end
    endtask

    integer i, n0, w0;
    reg [15:0] v;
    initial begin
        #2000;
        // core_run (wrapper command 0xB4), MCU_RSTn high, back to core mode
        G_WRAPn = 0;
        #300;
        rd(8'hB4);
        MCU_RSTn = 1;
        G_WRAPn = 1;
        #500;
        if (dut.core.rst !== 1'b0) fail("core did not leave reset");
        rd(8'd12); if (rv !== 16'hC0DE) fail("slot 12 ID");

        // slot 4 write / readback, normal and aggressive release
        wr(8'd4, 16'h1234, 0, 0); rd(8'd4);
        if (rv !== 16'h1234) begin fail("slot 4 normal"); $display("  read %h", rv); end
        wr(8'd4, 16'hA5C3, 0, 1); rd(8'd4);
        if (rv !== 16'hA5C3) begin fail("slot 4 aggressive"); $display("  read %h", rv); end
        w0 = nwr;
        wr(8'd4, 16'h0F0F, 1, 1); rd(8'd4);
        if (rv !== 16'h0F0F) begin fail("slot 4 NWE glitch"); $display("  read %h", rv); end
`ifndef DUT_V55G
        if (nwr - w0 != 1) begin fail("slot 4 NWE glitch: not one write clock"); $display("  %0d", nwr - w0); end
`endif

        // five FIFO pushes, aggressive release
        n0 = npush;
        for (i = 0; i < 5; i = i + 1) begin
            v = 16'h1111 * (i + 1) ^ 16'h8000;
            wr(8'd9, v, 0, 1);
            expect_push(n0 + i, v, "fifo push");
        end
        rd(8'd10);
        if (rv !== 16'd5) begin fail("slot 10 level after 5 pushes"); $display("  read %h", rv); end

        // a write with a 12 ns NWE high glitch: one push
        n0 = npush; wr(8'd9, 16'h00F0, 1, 1); expect_push(n0, 16'h00F0, "NWE glitch");
        // a write with a 12 ns G_WRAPn low glitch: one push
        n0 = npush; wr(8'd9, 16'h0F00, 2, 1); expect_push(n0, 16'h0F00, "G_WRAPn glitch");
        // a write with a 12 ns NRE low glitch: one push, FPGA never drives D
        n0 = npush; wr(8'd9, 16'hF000, 3, 1); expect_push(n0, 16'hF000, "NRE glitch");

        // idle glitches: nothing happens
        n0 = npush; w0 = nwr;
        MCU_ADDR = 8'd9;
        #200 MCU_NWE = 0; #12 MCU_NWE = 1;            // NCS high
        #200 MCU_NCS = 0;
        #200 MCU_NWE = 0; #12 MCU_NWE = 1;            // NCS low
        #200 G_WRAPn = 0; #12 G_WRAPn = 1;
        #200 MCU_NRE = 0; #12 MCU_NRE = 1;            // (a raw read window)
        #200 MCU_NCS = 1;
        #200 MCU_RSTn = 0; #12 MCU_RSTn = 1;
        #500;
        if (npush != n0) fail("idle glitch pushed");
`ifndef DUT_V55G
        if (nwr != w0) fail("idle glitch wrote");
`endif
        rd(8'd4);
        if (rv !== 16'h0F0F) begin fail("slot 4 after idle / reset glitches"); $display("  read %h", rv); end
        rd(8'd10);
        if (rv !== 16'd8) begin fail("slot 10 level at the end"); $display("  read %h", rv); end

        #1000;
        $display("pushes %0d, write clocks %0d", npush, nwr);
        if (errs == 0) $display("TB PASS");
        else $display("TB FAIL %0d", errs);
        $finish;
    end
endmodule
