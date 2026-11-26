// Behavioural check of scan_core.v (Icarus Verilog, not part of the Quartus build):
//   iverilog -g2005 -o tb.out fpga/scan-ep2c5/tb/tb_scan.v fpga/scan-ep2c5/scan_core.v && vvp -n tb.out
// Prints "TB PASS". WIN_LOG2 is 10 here (2^10-cycle window) to keep it short.
`timescale 1ns/1ps
module tb;
    reg [141:0] raw = {142{1'b1}};
    reg [7:0] clk = 8'h00;
    reg [32:0] dc = {33{1'b1}};
    reg [7:0] addr = 0;
    reg ncs = 1, nrd = 1;
    wire [15:0] d;
    wire oe;
    integer errs = 0, n;
    reg [15:0] v;
    scan_core #(.WIN_LOG2(10)) dut (.raw(raw), .clk(clk), .dc(dc), .addr(addr),
                                    .ncs(ncs), .nrd(nrd), .d_out(d), .d_oe(oe));

    task rd(input [7:0] a, output [15:0] q);
        begin
            addr = a; #50; ncs = 0; #20; nrd = 0; #100;
            if (!oe) begin $display("ERR oe low during read"); errs = errs + 1; end
            q = d; nrd = 1; #20; ncs = 1; #50;
            if (oe) begin $display("ERR oe high after read"); errs = errs + 1; end
        end
    endtask
    task expect(input [7:0] a, input [15:0] want);
        begin
            rd(a, v);
            if (v !== want) begin $display("ERR @%h = %h want %h", a, v, want); errs = errs + 1; end
        end
    endtask

    // H16 (clk[5]) and J2 (clk[3]) free-running when enabled
    reg h16_on = 0, j2_on = 0;
    always #10.4167 if (h16_on) clk[5] = ~clk[5];
    always #44.29 if (j2_on) clk[3] = ~clk[3];

    initial begin
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
        // clear, then GPIO-style toggles on H16
        rd(8'hF0, v);    // status before the first clear is power-up state
        expect(8'h06, 16'h5001);    // clr still high until this read ends
        expect(8'h06, 16'h5000);
        for (n = 0; n < 4096; n = n + 1) begin clk[5] = 1; #30; clk[5] = 0; #30; end
        expect(8'hCA, 16'h1000);
        expect(8'hCB, 16'h0000);
        expect(8'hC6, 16'h0000);
        // window
        rd(8'hF0, v); expect(8'hF4, 16'h5001);
        h16_on = 1; j2_on = 1;
        expect(8'hF1, 16'h5000);
        #(1100 * 20.8333 + 2000);
        rd(8'h06, v); if (v[4] !== 1) begin $display("ERR window not done %h", v); errs = errs + 1; end
        expect(8'hDA, 16'd1024);
        rd(8'hD6, v); $display("J2 window count %0d (expect ~%0d)", v, 1024 * 11.2896 / 48.0);
        if (v < 238 || v > 244) begin errs = errs + 1; $display("ERR J2 count"); end
        // G1 observation
        rd(8'hF0, v);
        expect(8'hF2, 16'h5001);
        #200;
        dc[19] = 0;                                 // CS0 low
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
        rd(8'hAB, v); $display("tog 22/23 (WRn, DMACKn) = %h", v);
        if (v !== 16'h0014) begin errs = errs + 1; $display("ERR WRn toggles"); end
        rd(8'hF6, v); $display("obs ticks lo %0d", v);
        if (errs == 0) $display("TB PASS"); else $display("TB FAIL %0d", errs);
        $finish;
    end
endmodule
