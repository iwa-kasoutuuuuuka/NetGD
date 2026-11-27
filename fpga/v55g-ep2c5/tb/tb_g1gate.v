// Behavioural check of g1gate.v (Icarus Verilog, not part of the build):
//   iverilog -g2005 -o tbg.out fpga/v55g-ep2c5/tb/tb_g1gate.v \
//       fpga/v55g-ep2c5/g1gate.v && vvp -n tbg.out
// Prints "TB PASS" at the end, or "TB FAIL n".
//
// Time constants are scaled down (PERIOD 480 ticks, engine expiry 1024 /
// 8192 ticks, group expiry 1024 ticks) so the run is short; the widths
// (W = 12 / 24 / 48 ticks) are the real ones.
//
// Checked continuously (every 1 ns, whole run):
//   * no pin_oe bit while R14 (master_n) is high, while DC_RSTn is low,
//     or while H16 is stopped for more than ~1 us;
//   * a DATA bit only inside a raw AltStatus read strobe (CSn1 = 0,
//     CSn0 = 1, A = 6, RDn = 0, WRn = 1, DMACKn = 1);
//   * IORDY only while the bus is idle;
//   * a faulted ball never again.
// Tests: power-up off; ID / status before H16 runs; commands ignored
// outside wrapper mode; ARM refusals; INTRQ pulses (width, period, count,
// readback, duty); master release mid-pulse; expiry (short, long) and
// reads not extending it; fault latch (at the enable edge: <= 3 ticks;
// mid-drive: <= 60 ns) and stickiness; DATA only inside AltStatus reads;
// IORDY only while idle, cut by a bus cycle; Phase C pattern 0x50; group
// enable + core path + group expiry + core_run; DC_RSTn low; H16 stop;
// wrapper exit; bus counters and write capture.
`timescale 1ns/1ps
module tb;
    localparam real TCK = 20.833;
    localparam real TX  = 88.577;
    localparam integer PERIOD = 480;

    reg clk = 0, clk_x = 0, run_clk = 0;
    always #(TX/2) clk_x = ~clk_x;
    always #(TCK/2) if (run_clk) clk = ~clk; else clk = 0;

    reg mcu_ncs = 1, mcu_nre = 1;
    reg [7:0] mcu_addr = 8'h00;
    reg wrap_n = 1, master_n = 1;
    reg dc_rstn = 1, dc_rdn = 1, dc_wrn = 1, dc_dmackn = 1;
    reg [1:0] dc_csn = 2'b11;
    reg [2:0] dc_addr = 3'd0;
    reg [23:0] core_oe = 0, core_val = 0;
    reg [23:0] rest = 24'h07FFF8 | 24'h000004 | 24'hF80000; // DATA, IORDY, audio high; INTRQ/DMARQ low
    reg [23:0] ext_on = 0, ext_val = 0;     // strong external driver (contention)
    reg [15:0] holly_wd = 16'h0000;
    reg holly_w = 0;                        // Holly drives DATA (write)
    wire [15:0] w_rdata;
    wire core_run;
    wire [23:0] pin_oe, pin_val;
    reg  [23:0] pad_in;
    integer b;
    always @* begin
        for (b = 0; b < 24; b = b + 1) begin
            if (ext_on[b]) pad_in[b] = ext_val[b];
            else if (holly_w && b >= 3 && b <= 18) pad_in[b] = holly_wd[b - 3];
            else if (pin_oe[b]) pad_in[b] = pin_val[b];
            else pad_in[b] = rest[b];
        end
    end
    wire [23:0] pad_d;
    assign #4 pad_d = pad_in;   // pad round trip

    g1gate #(.PERIOD(PERIOD), .PRE_LOG2(4), .E_SHORT(64), .E_LONG(512),
             .GPRE_LOG2(8), .G_LOAD(4)) dut (
        .clk(clk), .clk_x(clk_x), .mcu_ncs(mcu_ncs), .mcu_nre(mcu_nre),
        .mcu_addr(mcu_addr), .wrap_n(wrap_n), .master_n(master_n),
        .w_rdata(w_rdata), .core_run(core_run), .dc_rstn(dc_rstn),
        .dc_rdn(dc_rdn), .dc_wrn(dc_wrn), .dc_dmackn(dc_dmackn),
        .dc_csn(dc_csn), .dc_addr(dc_addr), .pad_in(pad_d),
        .core_oe(core_oe), .core_val(core_val), .pin_oe(pin_oe), .pin_val(pin_val));

    integer errs = 0;
    task fail(input [8*80-1:0] msg);
        begin errs = errs + 1; $display("FAIL @%0t: %0s", $time, msg); end
    endtask

    // ---------------------------------------------- continuous checks --
    localparam [23:0] M_DATA = 24'h07FFF8;
    wire q_alt_raw = ~dc_rdn & ~dc_csn[1] & dc_csn[0] & (dc_addr == 3'd6) & dc_wrn & dc_dmackn;
    wire q_idle_raw = dc_rdn & dc_wrn & dc_csn[0] & dc_csn[1] & dc_dmackn;
    reg core_data_ok = 0;     // set only in the core-path test
    reg [23:0] fault_seen = 0;
    real last_clk = 0;
    always @(posedge clk) last_clk = $realtime;
    initial #0.25 forever #1 begin
        if (pin_oe !== 24'd0 && pin_oe !== {24{1'b0}}) begin
            if (master_n) fail("oe while master released");
            if (!dc_rstn) fail("oe while DC_RSTn low");
            if ($realtime - last_clk > 1200.0) fail("oe while H16 stopped");
            if ((pin_oe & M_DATA) != 0 && !q_alt_raw && !core_data_ok)
                fail("DATA driven outside an AltStatus read");
            if (pin_oe[2] && !q_idle_raw && !core_data_ok)
                fail("IORDY driven during a bus cycle");
            if ((pin_oe & fault_seen) != 0) fail("faulted ball driven");
        end
        if (^pin_oe === 1'bx) fail("pin_oe unknown");
    end
    always @(posedge clk) fault_seen <= fault_seen | dut.fault;

    // ------------------------------------------------------ helpers --
    reg [15:0] rv;
    task rd(input [7:0] a);
        begin
            mcu_addr = a;
            #12 mcu_ncs = 0;
            #5 mcu_nre = 0;
            #85 rv = w_rdata;
            mcu_nre = 1;
            #5 mcu_ncs = 1;
            #40;
        end
    endtask
    task cmd(input [7:0] a); begin rd(a); if (run_clk) ticks(3); end endtask
    task ticks(input integer n); begin repeat (n) @(posedge clk); end endtask
    task expect16(input [7:0] a, input [15:0] want, input [15:0] mask,
                  input [8*40-1:0] what);
        begin
            rd(a);
            if ((rv & mask) !== (want & mask)) begin
                errs = errs + 1;
                $display("FAIL @%0t: %0s: read 0x%02h = 0x%04h, want 0x%04h (mask %04h)",
                         $time, what, a, rv, want, mask);
            end
        end
    endtask
    // staging: sel, mode, w, rest, long, n
    task stage(input [4:0] sel, input [1:0] mode, input [1:0] w, input rst_lvl,
               input lng, input [5:0] n);
        begin
            cmd(8'h80 | sel); cmd(8'hA0 | mode); cmd(8'hA4 | w);
            cmd(8'hA8 | rst_lvl); cmd(8'hAA | lng); cmd(8'hC0 | n);
        end
    endtask
    task holly_alt;   // one AltStatus read: CS1, A = 6, RDn ~100 ns
        begin
            dc_addr = 3'd6; dc_csn = 2'b01;
            #40 dc_rdn = 0;
            #100 dc_rdn = 1;
            #15 dc_csn = 2'b11;
            #30;
        end
    endtask
    task holly_rd0(input [2:0] a);   // a CS0 read (not AltStatus)
        begin
            dc_addr = a; dc_csn = 2'b10;
            #40 dc_rdn = 0;
            #100 dc_rdn = 1;
            #15 dc_csn = 2'b11;
            #30;
        end
    endtask
    task holly_wr(input [2:0] a, input [15:0] d);
        begin
            dc_addr = a; dc_csn = 2'b10;
            #30 holly_wd = d; holly_w = 1;
            #10 dc_wrn = 0;
            #100 dc_wrn = 1;
            #10 holly_w = 0;
            #15 dc_csn = 2'b11;
            #30;
        end
    endtask
    function [15:0] st; input dummy; st = dut.status; endfunction

    // measure oe on a ball
    real t_rise, t_fall, t_first, hi_time;
    integer n_rise;
    reg mon_on = 0;
    integer mon_b = 0;
    reg prev_oe = 0;
    real w_min, w_max, p_min, p_max, last_rise;
    always #0.5 begin
        if (mon_on) begin
            if (pin_oe[mon_b] && !prev_oe) begin
                if (n_rise > 0) begin
                    if ($realtime - last_rise < p_min) p_min = $realtime - last_rise;
                    if ($realtime - last_rise > p_max) p_max = $realtime - last_rise;
                end else t_first = $realtime;
                last_rise = $realtime; n_rise = n_rise + 1; t_rise = $realtime;
            end
            if (!pin_oe[mon_b] && prev_oe) begin
                t_fall = $realtime;
                if (t_fall - t_rise < w_min) w_min = t_fall - t_rise;
                if (t_fall - t_rise > w_max) w_max = t_fall - t_rise;
                hi_time = hi_time + (t_fall - t_rise);
            end
            prev_oe = pin_oe[mon_b];
        end
    end
    task mon_start(input integer bb);
        begin
            mon_b = bb; n_rise = 0; hi_time = 0; w_min = 1e9; w_max = 0;
            p_min = 1e9; p_max = 0; prev_oe = 0; mon_on = 1;
        end
    endtask

    integer i, k, t0;
    real tr, tf;
    initial begin
        // ---------------- 1. before H16 runs: everything off, ID readable
        #500;
        wrap_n = 0;
        rd(8'h00); if (rv !== 16'h5C55) fail("ID before H16");
        rd(8'h01); if (rv !== 16'h0001) fail("version before H16");
        rd(8'h02); if (rv !== 16'h0000) fail("status before H16 (all 0)");
        rd(8'h35); if (rv !== 16'h0020) fail("walking 1 word 0x35");
        rd(8'h2A); if (rv !== 16'hA52A) fail("unmapped word");
        if (pin_oe !== 0) fail("oe before H16");
        if (core_run !== 0) fail("core_run at power-up");
        // ---------------- 2. H16 on, master low, no command: still off
        run_clk = 1;
        master_n = 0;
        for (i = 0; i < 20; i = i + 1) holly_alt;
        holly_rd0(3'd7);
        ticks(100);
        if (pin_oe !== 0) fail("oe without any command");
        if (!dut.alive) fail("H16 alive not seen");
        expect16(8'h02, 16'h01E0, 16'hFFFF, "status idle (alive, rstn, wrap, master)");
        // ---------------- 3. commands outside wrapper mode are ignored
        wrap_n = 1;
        ticks(4);
        stage(5'd0, 2'd2, 2'd2, 1'b0, 1'b0, 6'd3);
        cmd(8'hB0); cmd(8'hB4); cmd(8'hB9);
        ticks(10);
        if (dut.e_arm || core_run || dut.grp_en != 0) fail("command accepted outside wrapper mode");
        if (dut.s_mode != 0 || dut.s_sel != 31) fail("staging changed outside wrapper mode");
        wrap_n = 0;
        ticks(4);
        // ---------------- 4. refusals
        master_n = 1; ticks(4);
        stage(5'd0, 2'd2, 2'd2, 1'b0, 1'b0, 6'd3);
        cmd(8'hB0); ticks(3);
        if (dut.e_arm || dut.e_ref != 1) fail("ARM without master not refused (1)");
        master_n = 0; ticks(4);
        stage(5'd25, 2'd2, 2'd2, 1'b0, 1'b0, 6'd3);
        cmd(8'hB0); ticks(3);
        if (dut.e_arm || dut.e_ref != 3) fail("bad ball not refused (3)");
        stage(5'd0, 2'd2, 2'd2, 1'b1, 1'b0, 6'd3);   // INTRQ rests 0, table says 1
        cmd(8'hB0); ticks(3);
        if (dut.e_arm || dut.e_ref != 5) fail("level mismatch not refused (5)");
        if (pin_oe !== 0) fail("oe after refusals");
        // ---------------- 5. INTRQ: 4 high pulses of 1 us, every PERIOD
        stage(5'd0, 2'd2, 2'd2, 1'b0, 1'b1, 6'd3);
        mon_start(0);
        cmd(8'hB0); ticks(2);
        if (!dut.e_arm || dut.e_ref != 0) fail("INTRQ pulse arm");
        t0 = 0;
        while (dut.e_arm && t0 < 4000) begin
            @(posedge clk); t0 = t0 + 1;
            if (pin_oe[0] && pin_val[0] !== 1'b1) fail("INTRQ pulse not high");
            if ((pin_oe & 24'hFFFFFE) != 0) fail("other ball driven during INTRQ walk");
        end
        mon_on = 0;
        $display("INTRQ pulses: n=%0d width %0.1f..%0.1f ns period %0.1f..%0.1f ns hi %0.1f ns",
                 n_rise, w_min, w_max, p_min, p_max, hi_time);
        if (n_rise != 4) fail("INTRQ pulse count != 4");
        if (w_min < 48*TCK - 2 || w_max > 48*TCK + 2) fail("INTRQ pulse width != 48 ticks");
        if (p_min < PERIOD*TCK - 2 || p_max > PERIOD*TCK + 2) fail("INTRQ pulse period != PERIOD");
        if (!dut.e_done) fail("INTRQ walk not done");
        expect16(8'h0D, 16'd4, 16'hFFFF, "pulses");
        expect16(8'h0E, 16'd4, 16'hFFFF, "readback followed");
        expect16(8'h0F, 16'd0, 16'hFFFF, "no mismatch samples");
        expect16(8'h03, 16'd0, 16'hFFFF, "no fault");
        // duty (scaled PERIOD 480: 10 %; real image 48000: 0.1 %)
        if (hi_time > 4 * 48 * TCK + 10) fail("INTRQ duty too high");
        // ---------------- 6. master released mid-pulse
        stage(5'd0, 2'd2, 2'd2, 1'b0, 1'b1, 6'd5);
        cmd(8'hB0);
        wait (pin_oe[0] === 1'b1);
        #200 master_n = 1;
        #0.1 if (pin_oe[0] !== 1'b0) fail("master release not immediate");
        ticks(4);
        if (dut.e_arm) fail("engine still armed after master release");
        if (!dut.seen_mdrop) fail("master drop not flagged");
        master_n = 0; ticks(4);
        ticks(PERIOD * 2);
        if (pin_oe !== 0) fail("drive resumed after master returned");
        // ---------------- 7. expiry: level mode on DMARQ, 64 periods > short
        stage(5'd1, 2'd1, 2'd0, 1'b0, 1'b0, 6'd63);
        cmd(8'hB0); ticks(1);
        if (!dut.e_arm) fail("DMARQ level arm");
        tr = $realtime;
        wait (pin_oe[1] === 1'b1);
        if (pin_val[1] !== 1'b0) fail("level-match value not the rest level");
        // reads must not extend the arm
        while (dut.e_arm) rd(8'h02);
        tf = $realtime;
        $display("short expiry after %0.0f ticks", (tf - tr) / TCK);
        if ((tf - tr) / TCK < 1024 - 40 || (tf - tr) / TCK > 1024 + 40)
            fail("short expiry not ~1024 ticks");
        if (!dut.e_exp) fail("expiry flag");
        if (pin_oe !== 0) fail("oe after expiry");
        if (dut.c_mis != 0) fail("level-match mismatch samples");
        // long
        stage(5'd1, 2'd1, 2'd0, 1'b0, 1'b1, 6'd63);
        cmd(8'hB0); tr = $realtime;
        wait (!dut.e_arm); tf = $realtime;
        $display("long expiry after %0.0f ticks", (tf - tr) / TCK);
        if ((tf - tr) / TCK < 8192 - 40 || (tf - tr) / TCK > 8192 + 40)
            fail("long expiry not ~8192 ticks");
        // ---------------- 8. fault at the enable edge (INTRQ held low)
        ext_on[0] = 1; ext_val[0] = 0;
        stage(5'd0, 2'd2, 2'd2, 1'b0, 1'b1, 6'd3);
        cmd(8'hB0);
        wait (pin_oe[0] === 1'b1); tr = $realtime;
        wait (pin_oe[0] === 1'b0); tf = $realtime;
        $display("fault (enable edge): oe on for %0.1f ns", tf - tr);
        if (tf - tr > 3 * TCK + 1) fail("fault not latched within 3 ticks");
        ticks(2);
        if (!dut.fault[0] || dut.e_arm) fail("fault[0] / engine kill");
        expect16(8'h05, 16'hC000, 16'hFF00, "fault info (valid, mode 2, ball 0)");
        ext_on[0] = 0;
        stage(5'd0, 2'd2, 2'd2, 1'b0, 1'b1, 6'd3);
        cmd(8'hB0); ticks(2);
        if (dut.e_arm || dut.e_ref != 4) fail("faulted ball not refused (4)");
        cmd(8'hB2); cmd(8'hB3); ticks(4);
        if (!dut.fault[0]) fail("fault not sticky");
        // ---------------- 9. fault mid-drive (DMARQ level, then contention)
        stage(5'd1, 2'd1, 2'd0, 1'b0, 1'b1, 6'd10);
        cmd(8'hB0);
        wait (pin_oe[1] === 1'b1);
        ticks(200);
        #3.3 ext_on[1] = 1; ext_val[1] = 1; tr = $realtime;
        wait (pin_oe[1] === 1'b0); tf = $realtime;
        $display("fault (mid-drive): %0.1f ns from contention start", tf - tr);
        if (tf - tr > 60.0) fail("mid-drive fault slower than 60 ns");
        ext_on[1] = 0;
        ticks(4);
        if (dut.fault != 24'h000003) fail("fault vector != INTRQ | DMARQ");
        // ---------------- 10. DATA0: drive 0 inside AltStatus reads only
        stage(5'd3, 2'd2, 2'd0, 1'b1, 1'b1, 6'd3);
        cmd(8'hB3);
        cmd(8'hB0); ticks(2);
        if (!dut.e_arm) fail("DATA0 arm");
        for (i = 0; i < 3; i = i + 1) begin
            holly_rd0(3'd7);         // status read on CS0: must not be driven
            #300;
        end
        if (dut.e_pulses != 0) fail("CS0 read counted as AltStatus");
        k = 0;
        for (i = 0; i < 6 && dut.e_arm; i = i + 1) begin
            fork
                holly_alt;
                begin
                    wait (dc_rdn === 1'b0);
                    #95 if (pin_oe[3] && pin_val[3] === 1'b0 && pad_in[3] === 1'b0) k = k + 1;
                end
            join
            #300;
        end
        ticks(4);
        $display("DATA0: reads %0d, followed %0d, bus 0 at strobe end %0d",
                 dut.e_pulses, dut.e_follow, k);
        if (dut.e_pulses != 4 || dut.e_follow != 4 || k != 4) fail("DATA0 4 driven reads");
        if (dut.e_arm || !dut.e_done) fail("DATA0 done");
        if (dut.c_mis != 0) fail("DATA0 mismatch samples");
        // ---------------- 11. IORDY low pulses of 0.5 us, only while idle
        stage(5'd2, 2'd2, 2'd1, 1'b1, 1'b1, 6'd3);
        mon_start(2);
        cmd(8'hB0);
        wait (pin_oe[2] === 1'b1);
        if (pin_val[2] !== 1'b0) fail("IORDY pulse not low");
        #100 dc_csn = 2'b01; tr = $realtime;      // a bus cycle starts mid-pulse
        #0.1 if (pin_oe[2] !== 1'b0) fail("IORDY not released at CS");
        #50 dc_csn = 2'b11;
        wait (!dut.e_arm);
        mon_on = 0;
        $display("IORDY: n=%0d width %0.1f..%0.1f ns", n_rise, w_min, w_max);
        if (w_max > 24 * TCK + 2) fail("IORDY pulse wider than 24 ticks");
        if (dut.e_pulses != 4) fail("IORDY pulse count");
        // ---------------- 12. Phase C: 0x50 on DATA[7:0] in AltStatus reads
        cmd(8'h40); cmd(8'h55); cmd(8'h6F); cmd(8'h7F);   // pat 0x50, mask 0xFF
        stage(5'd31, 2'd3, 2'd0, 1'b0, 1'b1, 6'd1);
        cmd(8'hB0); ticks(2);
        if (!dut.e_arm) fail("pattern arm");
        k = 0;
        for (i = 0; i < 2; i = i + 1) begin
            fork
                holly_alt;
                begin
                    wait (dc_rdn === 1'b0);
                    #95 if (pad_in[10:3] === 8'h50 && pin_oe[18:11] === 8'h00) k = k + 1;
                end
            join
            #300;
        end
        if (k != 2) fail("pattern 0x50 not on the bus");
        if (dut.e_arm) fail("pattern done");
        // ---------------- 13. core path: group 3 (IORDY), expiry, core_run
        core_oe = 24'hFFFFFF; core_val = 24'h000004;    // IORDY = 1 (rest level)
        cmd(8'hBB); ticks(4);                              // group 3 without core_run
        if (pin_oe !== 0) fail("core path without core_run");
        cmd(8'hB4); ticks(4);
        if (!core_run) fail("core_run");
        if (pin_oe !== 24'h000004) fail("group 3 core path: exactly IORDY");
        tr = $realtime;
        while (pin_oe[2] === 1'b1) rd(8'h02);
        tf = $realtime;
        $display("group expiry after %0.0f ticks", (tf - tr) / TCK);
        if ((tf - tr) / TCK < 3 * 256 || (tf - tr) / TCK > 4 * 256 + 20) fail("group expiry");
        cmd(8'hBB); ticks(4);
        if (pin_oe !== 24'h000004) fail("group 3 re-enable");
        cmd(8'hB8); ticks(2);
        if (pin_oe !== 0) fail("group off command");
        // ---------------- 14. DC_RSTn low: immediate off, groups cleared
        cmd(8'hBB); ticks(4);
        dc_rstn = 0;
        #0.1 if (pin_oe !== 0) fail("DC_RSTn low not immediate");
        ticks(4);
        if (dut.grp_en != 0) fail("groups not cleared by DC_RSTn");
        dc_rstn = 1; ticks(4);
        if (pin_oe !== 0) fail("drive back after DC_RSTn");
        if (dut.c_rst != 1) fail("RSTn fall count");
        cmd(8'hB5); ticks(2);
        if (core_run) fail("core_run off");
        core_oe = 0; core_val = 0;
        // ---------------- 15. H16 stops during a level drive
        stage(5'd23, 2'd1, 2'd0, 1'b1, 1'b1, 6'd30);     // EMPH level 1
        cmd(8'hB0);
        wait (pin_oe[23] === 1'b1);
        ticks(10);
        run_clk = 0; tr = $realtime;
        wait (pin_oe[23] === 1'b0); tf = $realtime;
        $display("H16 stop: oe off after %0.0f ns", tf - tr);
        if (tf - tr > 1100) fail("H16 stop not detected within 1.1 us");
        run_clk = 1; ticks(20);
        if (pin_oe[23] === 1'b1 && !dut.e_arm) fail("oe without arm");
        cmd(8'hB1); ticks(4);
        if (pin_oe !== 0) fail("STOP");
        // ---------------- 16. wrapper exit kills the engine
        stage(5'd23, 2'd1, 2'd0, 1'b1, 1'b1, 6'd30);
        cmd(8'hB0);
        wait (pin_oe[23] === 1'b1);
        wrap_n = 1; ticks(4);
        if (dut.e_arm || pin_oe !== 0) fail("wrapper exit");
        wrap_n = 0; ticks(4);
        // ---------------- 17. bus counters and capture
        cmd(8'hB3); ticks(2);
        for (i = 0; i < 5; i = i + 1) holly_alt;
        holly_rd0(3'd7);
        holly_wr(3'd7, 16'h00A0);
        holly_wr(3'd1, 16'h1234);
        ticks(4);
        expect16(8'h12, 16'd5, 16'hFFFF, "AltStatus count");
        expect16(8'h13, 16'd1, 16'hFFFF, "other read count");
        expect16(8'h14, 16'd2, 16'hFFFF, "write count");
        expect16(8'h20, 16'h002F, 16'h007F, "capture 0: CS0 read reg 7");
        expect16(8'h22, 16'h006F, 16'h007F, "capture 1: CS0 write reg 7");
        expect16(8'h23, 16'h00A0, 16'hFFFF, "capture 1 data");
        expect16(8'h24, 16'h0063, 16'h007F, "capture 2: CS0 write reg 1");
        expect16(8'h25, 16'h1234, 16'hFFFF, "capture 2 data");
        expect16(8'h1A, 16'h8000 | (3 << 10), 16'hFC00, "levels: RSTn, cap_n");
        // ---------------- 18. W code 0: one EMPH low pulse of 12 ticks
        stage(5'd23, 2'd2, 2'd0, 1'b1, 1'b1, 6'd0);
        mon_start(23);
        cmd(8'hB0);
        wait (!dut.e_arm);
        #50 mon_on = 0;
        $display("EMPH W0: n=%0d width %0.1f..%0.1f ns", n_rise, w_min, w_max);
        if (n_rise != 1 || w_min < 12*TCK - 2 || w_max > 12*TCK + 2 || w_max < 1.0) fail("W code 0 != 12 ticks");
        // ---------------- 19. single-sample glitches must not latch a fault
        stage(5'd23, 2'd1, 2'd0, 1'b1, 1'b1, 6'd20);
        cmd(8'hB0);
        wait (pin_oe[23] === 1'b1);
        ticks(10);
        for (i = 0; i < 16; i = i + 1) begin
            @(posedge clk); #(1.3 * i);
            ext_val[23] = 0; ext_on[23] = 1;
            #6 ext_on[23] = 0;
            ticks(5);
        end
        if (dut.fault[23]) fail("6 ns glitch latched a fault");
        if (!dut.e_arm) fail("engine stopped by a glitch");
        cmd(8'hB1); ticks(4);
        // ---------------- 20. mid-drive fault latency over the clock phase
        w_max = 0;
        for (i = 0; i < 3; i = i + 1) begin
            stage(5'd20 + i, 2'd1, 2'd0, 1'b1, 1'b1, 6'd20);
            cmd(8'hB0);
            wait (pin_oe[20 + i] === 1'b1);
            ticks(20);
            @(posedge clk); #(1.0 + 6.9 * i);
            ext_val[20 + i] = 0; ext_on[20 + i] = 1; tr = $realtime;
            wait (pin_oe[20 + i] === 1'b0); tf = $realtime;
            if (tf - tr > w_max) w_max = tf - tr;
            ext_on[20 + i] = 0;
            ticks(4);
        end
        $display("mid-drive fault latency, worst of 3 phases: %0.1f ns", w_max);
        if (w_max > 50.0) fail("mid-drive fault latency > 50 ns");
        // ---------------- 21. a fault removes a core-path ball although its group stays on
        core_oe = 24'hFFFFFF; core_val = 24'h000004;
        cmd(8'hB4); cmd(8'hBB); ticks(4);
        if (pin_oe !== 24'h000004) fail("core path IORDY");
        ext_val[2] = 0; ext_on[2] = 1;
        ticks(10);
        if (!dut.fault[2]) fail("core-path fault not latched");
        if (!dut.grp_en[3]) fail("group 3 should still be on");
        for (i = 0; i < 100; i = i + 1) begin
            @(posedge clk);
            if (pin_oe[2]) fail("faulted core-path ball driven");
        end
        ext_on[2] = 0;
        cmd(8'hB2); ticks(4);
        core_oe = 0; core_val = 0;
        if (dut.fault != 24'h700007) fail("unexpected faults at the end");
        if (fault_seen != 24'h700007) fail("fault history");
        #100;
        if (errs == 0) $display("TB PASS");
        else $display("TB FAIL %0d", errs);
        $finish;
    end
    initial begin
        #20_000_000;
        $display("TB FAIL timeout");
        $finish;
    end
endmodule
