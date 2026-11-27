// Behavioural check of scan2_core.v + g1x.v (Icarus Verilog, not part of
// the Quartus build):
//   iverilog -g2005 -o tb2.out fpga/scan2-ep2c5/tb/tb_scan2.v \
//       fpga/scan2-ep2c5/scan2_core.v fpga/scan2-ep2c5/g1x.v \
//       fpga/scan-ep2c5/scan_core.v && vvp -n tb2.out
// Prints "TB PASS".
//
// Part 1 (regression): a reference scan_core runs beside scan2_core on the
// same pins; every read of an address outside the g1x set must return the
// same word from both (the sweep-11 T0 list, a sweep of all 256 addresses,
// and the tb_scan.v sequence with its own expected words).
// Part 2 (g1x): the Holly AltStatus poll model (CS1 low, A2..A0 = 110,
// RDn strobes, DATA floating high) with four B4 models: none (B4 rests
// low), H1 (short crosstalk pulses 0..2 ticks after RDn edges, plus
// sub-tick glitches), H2 (B4 = ~RDn), H3 (long random pulses). Every
// region is read back through ptr; the histograms, counters and a
// plan-style verdict are asserted. WIN_LOG2 is 10 for scan_core and 14
// (16384 ticks, 341 us) for g1x.
`timescale 1ns/1ps
module tb;
    reg [141:0] raw = {142{1'b1}};
    reg [7:0] clk = 8'h00;
    reg [32:0] dc = {33{1'b1}};
    reg c4 = 1'b1;
    reg [7:0] addr = 0;
    reg ncs = 1, nrd = 1;
    wire [15:0] d, dref;
    wire oe, oeref;
    integer errs = 0, n, m;
    reg [15:0] v;
    localparam GW = 14;

    scan2_core #(.WIN_LOG2(10), .G_WIN_LOG2(GW)) dut (
        .raw(raw), .clk(clk), .dc(dc), .c4(c4), .addr(addr), .ncs(ncs), .nrd(nrd),
        .d_out(d), .d_oe(oe));
    scan_core #(.WIN_LOG2(10)) ref0 (
        .raw(raw), .clk(clk), .dc(dc), .addr(addr), .ncs(ncs), .nrd(nrd),
        .d_out(dref), .d_oe(oeref));

    function is_g1x;
        input [7:0] a;
        is_g1x = (a >= 8'h41 && a <= 8'h4E && a != 8'h44);
    endfunction

    task rd(input [7:0] a, output [15:0] q);
        begin
            addr = a; #50; ncs = 0; #20; nrd = 0; #100;
            if (!oe) begin $display("ERR oe low during read"); errs = errs + 1; end
            if (oe !== oeref) begin $display("ERR oe differs from scan_core"); errs = errs + 1; end
            q = d;
            if (!is_g1x(a) && d !== dref) begin
                $display("ERR @%h scan2 %h != scan_core %h", a, d, dref); errs = errs + 1;
            end
            if (is_g1x(a) && dref !== (16'hA500 | a)) begin
                $display("ERR @%h g1x address is not a scan echo (%h)", a, dref); errs = errs + 1;
            end
            nrd = 1; #20; ncs = 1; #50;
            if (oe) begin $display("ERR oe high after read"); errs = errs + 1; end
        end
    endtask
    task expect(input [7:0] a, input [15:0] want);
        begin
            rd(a, v);
            if (v !== want) begin $display("ERR @%h = %h want %h", a, v, want); errs = errs + 1; end
        end
    endtask
    task check(input ok, input [8*48-1:0] what);
        begin
            if (!ok) begin $display("ERR %0s", what); errs = errs + 1; end
        end
    endtask

    reg h16_on = 0, j2_on = 0;
    always #10.4167 if (h16_on) clk[5] = ~clk[5];
    always #44.29 if (j2_on) clk[3] = ~clk[3];

    // ------------------------------------------------ sweep-11 T0 list --
    reg [7:0] t0 [0:62];
    initial begin
        t0[0]=8'h00; t0[1]=8'h03; t0[2]=8'h05; t0[3]=8'h07;
        for (n = 0; n < 16; n = n + 1) begin t0[4+n] = 8'h30 + n; t0[20+n] = 8'h50 + n; end
        for (n = 0; n < 8; n = n + 1) t0[36+n] = 8'h1 << n;
        t0[44]=8'h0B; t0[45]=8'h0D; t0[46]=8'h0E; t0[47]=8'h11; t0[48]=8'h22; t0[49]=8'h44;
        t0[50]=8'h88; t0[51]=8'h1F; t0[52]=8'h2F; t0[53]=8'h4F; t0[54]=8'h8F; t0[55]=8'h7F;
        t0[56]=8'hBF; t0[57]=8'hFE; t0[58]=8'hFD; t0[59]=8'hFB; t0[60]=8'hFF; t0[61]=8'h6A;
        t0[62]=8'h00;
    end
    function [15:0] model;  // psload::sweep11::model for the fixed words
        input [7:0] a;
        begin
            model = 16'hA500 | a;
            case (a[7:4])
                4'h0: case (a[3:0])
                    4'h0: model = 16'hC0DE; 4'h3: model = 16'h3F21;
                    4'h5: model = 16'h5CA1; 4'h7: model = 16'd10;
                    default: ;
                endcase
                4'h3: model = 16'h1 << a[3:0];
                4'h5: model = ~(16'h1 << a[3:0]);
                default: ;
            endcase
            case (a)
                8'h01: model = 16'hB001; 8'h02: model = 16'hB102; 8'h04: model = 16'hB204;
                8'h08: model = 16'hB308; 8'h10: model = 16'hB410; 8'h20: model = 16'hB520;
                8'h40: model = 16'hB640; 8'h80: model = 16'hB780;
                default: ;
            endcase
        end
    endfunction

    // ------------------------------------------------ G1 side models --
    // Holly AltStatus poll: CS1 low, CS0 high, A2..A0 = 110, DATA floating
    // high (all 1), RDn low RD_LO ns every RD_LO + RD_HI ns. B4 per model.
    localparam RD_LO = 125, RD_HI = 475;
    reg holly = 0;
    integer bmodel = 0;   // 0 none, 1 H1, 2 H2, 3 H3
    reg rdn = 1;
    always @(rdn) dc[21] = rdn;
    initial begin
        forever begin
            if (holly) begin
                #(RD_HI); rdn = 0; #(RD_LO); rdn = 1;
            end else #100;
        end
    end
    // H1: crosstalk. Per RDn edge a B4 pulse 0..2 ticks later; the pulse is
    // 22 ns (sampled once or twice) or, for half of them, a 4 ns glitch
    // (seen by the async counter, rarely by the sampler); fixed pattern.
    integer xk = 0;
    always @(rdn) if (holly && bmodel == 1) begin
        xk = xk + 1;
        #((xk % 3) * 15 + 3);
        if (xk % 4 >= 2) begin dc[25] = 1; #4; dc[25] = 0; end
        else begin dc[25] = 1; #22; dc[25] = 0; end
    end
    // H2: B4 = ~RDn.
    always @(rdn) if (holly && bmodel == 2) dc[25] = ~rdn;
    // H3: long random pulses, 3..20 us high, 5..30 us low, independent of RDn.
    integer seed = 32'h5EED;
    initial begin
        forever begin
            if (holly && bmodel == 3) begin
                #(5000 + ({$random(seed)} % 25000)); dc[25] = 1;
                #(3000 + ({$random(seed)} % 17000)); dc[25] = 0;
            end else #100;
        end
    end

    // ------------------------------------------------ g1x readout --
    reg [31:0] g [0:319];
    task g_read_all;
        integer i;
        reg [15:0] lo, hi;
        begin
            rd(8'h4B, v);
            for (i = 0; i < 320; i = i + 1) begin
                rd(8'h45, lo); rd(8'h46, hi);
                g[i] = {hi, lo};
                rd(8'h4C, v);
            end
            rd(8'h43, v);
            check(v == 9'd320, "ptr after the readout");
        end
    endtask
    task g_window;   // clear, arm, wait frozen, read back
        integer t;
        begin
            rd(8'h48, v);
            expect(8'h42, 16'hA009);         // clr_g, zeroing
            #(40 * 20.8333);
            expect(8'h42, 16'hA000);         // ready
            expect(8'h49, 16'hA000);
            #(10 * 20.8333);
            rd(8'h42, v); check(v[4] && v[7], "window running after arm");
            t = 0;
            rd(8'h42, v);
            while (!v[6] && t < 10000) begin #1000; t = t + 1; rd(8'h42, v); end
            check(v == 16'hA0E2, "status frozen");
            g_read_all;
        end
    endtask

    function [31:0] sum8;
        input integer base;
        integer i;
        begin
            sum8 = 0;
            for (i = 0; i < 8; i = i + 1) sum8 = sum8 + g[base + i];
        end
    endfunction

    // Region / slot indices (docs/research/ep2c5-scan2.md).
    localparam R_H = 0, R_E0 = 32, R_E1 = 64, R_E2 = 96, R_E3 = 128, R_DB = 160,
               R_DR = 192, R_GR = 224, R_GW = 256, R_X = 288;
    localparam S_RISE = 0, S_FALL = 8, S_C00 = 16, S_W4 = 20, S_QR = 21;

    integer verdict, nb, nshort, nlong, ncoin, i;
    integer rd_min = 565, rd_max = 571;
    reg paused = 0;
    task g_common;   // checks that hold for every Holly scenario
        begin
            check(g[R_X + 0] == (1 << GW), "ticks = 2^GW");
            check(g[R_H + S_C00] + g[R_H + S_C00 + 1] + g[R_H + S_C00 + 2] + g[R_H + S_C00 + 3]
                  == (1 << GW), "contingency sums to ticks");
            for (i = 0; i < 8; i = i + 1)
                check(g[R_H + S_RISE + i] - g[R_H + S_FALL + i]
                      == g[R_X + 6][8 + i] - g[R_X + 6][i], "rises - falls = end - start");
            // the poll: RDn falls ~ 341 us / 600 ns, all in bin CSn0=1 CSn1=0 A=6
            check(g[R_H + S_FALL + 1] >= rd_min && g[R_H + S_FALL + 1] <= rd_max, "RDn falls ~569");
            check(g[R_GR + 22] == g[R_H + S_FALL + 1], "every read in the AltStatus bin");
            check(sum8(R_GR) + sum8(R_GR + 8) + sum8(R_GR + 16) + sum8(R_GR + 24)
                  == g[R_GR + 22], "no other read bin");
            check(g[R_H + S_QR] == g[R_GR + 22], "every read still low at +2");
            check(g[R_X + 5] == 32'hFFFF_FFFF, "DATA AND / OR = FFFF / FFFF");
            check(g[R_DR + 3] >= g[R_H + S_FALL + 1] - 1 && sum8(R_DR) == g[R_DR + 3],
                  "RDn low widths all 5-8 ticks");
            check(g[R_X + 3][15:0] >= 11 && g[R_X + 3][31:16] <= 13 &&
                  g[R_X + 3][15:0] <= g[R_X + 3][31:16], "RDn low 11..13 half-ticks");
            check(g[R_X + 4][15:0] >= 56 && (paused || g[R_X + 4][31:16] <= 59) &&
                  g[R_X + 4][15:0] <= g[R_X + 4][31:16], "RDn period 56..59 half-ticks");
            check(g[R_X + 2] + 2 >= g[R_H + S_FALL + 1] && g[R_X + 2] <= g[R_H + S_FALL + 1] + 2,
                  "async RDn falls = sampled +-2");
            check(paused || g[R_H + S_FALL + 2] == 0 && g[R_H + S_FALL + 3] == 0 &&
                  g[R_H + S_FALL + 5] == 0 && g[R_H + S_FALL + 6] == 0, "CS / DMACK / RST quiet");
            check(g[R_X + 7] == GW, "region 9 WIN_LOG2");
            // every B4 rise lands in each coincidence histogram once
            nb = g[R_H + S_RISE + 0];
            check(sum8(R_E0) == nb && sum8(R_E1) == nb && sum8(R_E2) == nb && sum8(R_E3) == nb,
                  "coincidence histograms sum to B4 rises");
            // plan-style verdict
            nshort = g[R_DB + 0] + g[R_DB + 1];
            nlong = g[R_DB + 6] + g[R_DB + 7];
            ncoin = g[R_E0 + 0] + g[R_E0 + 1] + g[R_E0 + 2] + g[R_E0 + 3]
                  + g[R_E1 + 0] + g[R_E1 + 1] + g[R_E1 + 2] + g[R_E1 + 3];
            if (nb == 0) verdict = 0;
            else if (g[R_H + S_C00] == 0 && g[R_H + S_C00 + 3] == 0) verdict = 2;
            else if (nshort * 10 >= nb * 9 && ncoin * 10 >= nb * 9) verdict = 1;
            else verdict = 3;
            $display("  B4 rises %0d (async %0d), short %0d long %0d coincident %0d, RDn falls %0d, low %0d..%0d, period %0d..%0d half-ticks -> verdict %0d",
                     nb, g[R_X + 1], nshort, nlong, ncoin, g[R_H + S_FALL + 1],
                     g[R_X + 3][15:0], g[R_X + 3][31:16], g[R_X + 4][15:0], g[R_X + 4][31:16],
                     verdict);
        end
    endtask

    initial begin
        // ================================ part 1: scan_core regression ==
        for (n = 0; n < 63; n = n + 1) expect(t0[n], model(t0[n]));
        expect(8'h41, 16'h5C21);
        for (n = 0; n < 256; n = n + 1) if (n < 8'hF0 || n > 8'hF4) rd(n, v);
        // the tb_scan.v sequence
        expect(8'h00, 16'hC0DE);
        expect(8'h03, 16'h3F21);
        expect(8'h05, 16'h5CA1);
        expect(8'h07, 16'd10);
        for (n = 0; n < 16; n = n + 1) expect(8'h30 + n, 16'h1 << n);
        for (n = 0; n < 16; n = n + 1) expect(8'h50 + n, ~(16'h1 << n));
        for (n = 0; n < 8; n = n + 1) expect(8'h1 << n, 16'hB000 | (n << 8) | (1 << n));
        expect(8'h09, 16'hA509);
        raw = 0; raw[17] = 1; raw[141] = 1;
        expect(8'h61, 16'h0002);
        expect(8'h68, 16'h2000);
        raw = {142{1'b1}};
        rd(8'hF0, v);
        expect(8'h06, 16'h5001);
        expect(8'h06, 16'h5000);
        for (n = 0; n < 4096; n = n + 1) begin clk[5] = 1; #30; clk[5] = 0; #30; end
        expect(8'hCA, 16'h1000);
        expect(8'hCB, 16'h0000);
        expect(8'hC6, 16'h0000);
        rd(8'hF0, v); expect(8'hF4, 16'h5001);
        h16_on = 1; j2_on = 1;
        expect(8'hF1, 16'h5000);
        #(1100 * 20.8333 + 2000);
        rd(8'h06, v); if (v[4] !== 1) begin $display("ERR window not done %h", v); errs = errs + 1; end
        expect(8'hDA, 16'd1024);
        rd(8'hD6, v);
        if (v < 238 || v > 244) begin errs = errs + 1; $display("ERR J2 count %0d", v); end
        rd(8'hF0, v);
        expect(8'hF2, 16'h5001);
        #200;
        dc[19] = 0;
        for (n = 0; n < 10; n = n + 1) begin
            dc[15:0] = 16'h1100 + n; dc[18:16] = n[2:0]; dc[20] = n[0];
            #100; dc[22] = 0; #200; dc[22] = 1; #200;
        end
        dc[19] = 1;
        raw[5] = 0; #200; raw[5] = 1; #200;
        expect(8'hF3, 16'h5824);
        expect(8'hF5, 16'd10);
        expect(8'hE0, 16'h1100);
        expect(8'hE1, 16'h8000);
        expect(8'hE2, 16'h1101);
        expect(8'hE3, 16'h8009);
        expect(8'hEE, 16'h1107);
        expect(8'hEF, 16'h800F);
        rd(8'h70, v); if (v !== 16'h0020) begin $display("ERR seen0 w0 %h", v); errs = errs + 1; end
        rd(8'h90, v); if (v !== 16'hFFFF) begin $display("ERR seen1 w0 %h", v); errs = errs + 1; end
        rd(8'hAB, v);
        if (v !== 16'h0014) begin errs = errs + 1; $display("ERR WRn toggles %h", v); end
        for (n = 0; n < 63; n = n + 1) expect(t0[n], model(t0[n]));
        $display("part 1 (scan_core regression) done, errors %0d", errs);

        // ================================ part 2: g1x ==
        dc = {33{1'b1}};
        dc[25] = 0;                  // INTRQ rests low (host pull-down)
        dc[26] = 0;                  // DMARQ too
        expect(8'h47, GW);
        dc[20] = 0; dc[18:16] = 3'b110;   // CS1 low, A = 6
        holly = 1;

        // ---- no B4 activity
        bmodel = 0;
        $display("scenario quiet:");
        g_window;
        g_common;
        check(verdict == 0 && nb == 0 && g[R_X + 1] == 0, "quiet: no B4 edges");
        check(g[R_H + S_C00 + 1] + g[R_H + S_C00] == (1 << GW), "quiet: B4 always 0");
        for (i = 0; i < 32; i = i + 1) check(g[R_GW + i] == 0, "no WR bins");

        // ---- H1 crosstalk
        bmodel = 1; xk = 0;
        $display("scenario H1:");
        g_window;
        g_common;
        check(verdict == 1, "H1 verdict");
        check(g[R_X + 1] > nb + nb / 4, "H1: async B4 >> sampled (glitches)");
        check(nlong == 0 && g[R_DB + 2] + g[R_DB + 3] == 0, "H1: no B4 pulse over 2 ticks");
        check(g[R_H + S_W4] > 0 && g[R_H + S_W4] <= g[R_H + S_FALL + 1], "H1: RDn fall -> B4 rise in 4");

        // ---- H2: B4 = ~RDn
        bmodel = 2;
        #1000;
        $display("scenario H2:");
        g_window;
        g_common;
        check(verdict == 2, "H2 verdict");
        check(nb == g[R_H + S_FALL + 1] && g[R_H + S_FALL + 0] == g[R_H + S_RISE + 1],
              "H2: B4 edges 1:1 with RDn");
        for (i = 0; i < 8; i = i + 1) check(g[R_DB + i] == g[R_DR + i], "H2: B4 widths = RDn widths");
        check(g[R_E0 + 0] == nb, "H2: every B4 rise at an RDn fall (age 0)");
        check(g[R_H + S_W4] == nb, "H2: every RDn fall followed by a B4 rise");
        check(g[R_X + 1] + 2 >= nb && g[R_X + 1] <= nb + 2, "H2: async = sampled +-2");

        // ---- H3: long random pulses, plus C4 toggles and a few CS0 writes
        bmodel = 3;
        dc[25] = 0;
        rd_min = 540; paused = 1;
        $display("scenario H3:");
        fork
            g_window;
            begin
                #20000;
                for (n = 0; n < 5; n = n + 1) begin c4 = 0; #3000; c4 = 1; #3000; end
                // 3 writes, CS0 low CS1 high A = 7, between polls (poll stopped)
                holly = 0; #1000;
                dc[20] = 1; dc[19] = 0; dc[18:16] = 3'b111;
                for (n = 0; n < 3; n = n + 1) begin #100; dc[22] = 0; #200; dc[22] = 1; #200; end
                dc[19] = 1; dc[20] = 0; dc[18:16] = 3'b110;
                holly = 1;
            end
        join
        g_common;
        check(verdict == 3, "H3 verdict");
        check(nlong == sum8(R_DB) && nlong > 0 && nshort == 0, "H3: B4 pulses long");
        check(g[R_H + S_RISE + 7] == 5 && g[R_H + S_FALL + 7] == 5, "H3: C4 5 rises / 5 falls");
        check(g[R_GW + 15] == 3 && g[R_H + S_FALL + 4] == 3, "H3: 3 writes in bin CSn0=0 CSn1=1 A=7");
        check(g[R_H + S_FALL + 3] == 1 && g[R_H + S_RISE + 2] == 1, "H3: one CS0 fall, one CS1 rise");
        check(g[R_H + S_FALL + 5] == 0 && g[R_H + S_FALL + 6] == 0, "H3: DMACK / RST quiet");
        check(g[R_X + 1] + 2 >= nb && g[R_X + 1] <= nb + 2, "H3: async = sampled +-2");

        // ---- stop command ends the window early
        bmodel = 0;
        rd(8'h48, v); #(40 * 20.8333);
        rd(8'h49, v); #(500 * 20.8333);
        rd(8'h4A, v); #(300 * 20.8333);
        rd(8'h42, v); check(v[6] && v[5], "stop: frozen");
        rd(8'h4B, v); rd(8'h4D, v); rd(8'h4D, v); rd(8'h4D, v); rd(8'h4D, v); rd(8'h4D, v);
        rd(8'h4D, v); rd(8'h4D, v); rd(8'h4D, v); rd(8'h4D, v);
        rd(8'h43, v); check(v == 9'd288, "ptr += 32 x 9");
        rd(8'h45, v); check(v > 400 && v < 600, "stop: ticks ~500");

        // part 1 again after the g1x traffic
        holly = 0;
        for (n = 0; n < 63; n = n + 1) expect(t0[n], model(t0[n]));
        if (errs == 0) $display("TB PASS"); else $display("TB FAIL %0d", errs);
        $finish;
    end
endmodule
