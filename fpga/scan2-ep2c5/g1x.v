// g1x: G1-side discrimination block of the EP2C5F256 scan2 core (Run A,
// docs/research/ag16k-ps-plan-run4.md section 2; register map
// docs/research/ep2c5-scan2.md). Hand-written.
//
// INPUT ONLY. The block has no pin outputs: it only adds words to the read
// mux of scan2_core (d_out while `hit`). Commands are reads of reserved
// addresses, latched on the rising edge of NRD while NCS3 is low (as in
// scan_core); there is no write path.
//
// Addresses (addr = A8..A1). Only 0x41..0x43 and 0x45..0x4E are decoded:
// 0x40 stays one-hot and 0x44 / 0x4F stay scan echo words, because the
// sweep-11 T0 list reads them (psload::sweep11::T0_ADDRS).
//   0x41 ID 0x5C21, 0x42 status, 0x43 ptr, 0x45 item[ptr][15:0],
//   0x46 item[ptr][31:16], 0x47 WIN_LOG2,
//   commands (return status): 0x48 clear, 0x49 arm, 0x4A stop,
//   0x4B ptr = 0, 0x4C ptr += 1, 0x4D ptr += 32, 0x4E no-op.
//
// Clock domains:
//   H16 (48 MHz PCK0): everything except below. Inputs pass a 2-flop
//     synchroniser (c1/c2, the same structure as scan_core so the fitter
//     can share the flops); c3 is one sample older for edges.
//   H16 falling edge: a second 2-flop synchroniser for RDn only (timing,
//     half-tick resolution), re-registered on the rising edge.
//   NRD (MCU read strobe): the command latch and ptr.
//   B4 rising / RDn falling: the two async counters (g1x_acnt). Their
//     enable is synchronised into their own domain (2 flops), they count
//     in binary and publish a registered Gray copy; the H16 side samples
//     the Gray word through 2 flops (one bit changes per count, so a
//     sample is either the old or the new value) and converts it back.
//     The H16 snapshot stops updating when the block is frozen, so the MCU
//     reads a value that no longer changes. Residual risk: a B4 glitch
//     shorter than the flops' minimum clock pulse width can upset the
//     async counter itself (that is what it is there to detect; the
//     sampled counters are the reference).
//   Every MCU read of an H16 register is asynchronous to H16: read only
//   when status.frozen = 1 (then nothing changes), or read twice.
//
// Storage: the 22 scalar event counters and the 6 histograms live in M4K
// RAM (32 x 32 bit each), so that the logic fits beside scan_core:
//   * scalar counters (region 0): one 6-bit pre-counter per counter; a
//     scanner visits one slot per tick (32 slots, period 32 ticks), adds
//     the pre-counter to the RAM word (read-modify-write over 2 ticks) and
//     restarts it. A slot gets at most one event per tick, so 6 bits hold
//     the 32 ticks between visits.
//   * histograms (regions 1..8): one RAM per event source; each source
//     fires at most once every 2 ticks (an edge needs the opposite level
//     in between), so a 2-tick read-modify-write never collides.
//   After the window closes the scanner runs 256 more ticks (8 rounds) to
//   flush the pre-counters, then status.frozen = 1 and the RAM read
//   address is ptr. Clear (0x48) zeroes all RAM words in 32 ticks
//   (status.zeroing) and every register; the window cannot start before.
module g1x #(
    parameter [7:0] WIN_LOG2 = 8'd28   // window 2^28 H16 ticks = 5.59 s at 48 MHz
) (
    input  wire        h16,
    input  wire [32:0] dc,     // v5.5 DC_* balls, mk_scan.DC_ORDER, raw pins
    input  wire        c4,     // control ball C4 (unassigned neighbour of B4)
    input  wire [7:0]  addr,
    input  wire        ncs,
    input  wire        nrd,
    output reg         hit,
    output reg  [15:0] d_out
);
    // ------------------------------------------------ command latch (NRD) --
    reg clr_g = 1'b0, arm_q = 1'b0, stop_q = 1'b0;
    reg [8:0] ptr = 9'd0;
    always @(posedge nrd) begin
        if (!ncs) begin
            clr_g <= (addr == 8'h48);
            case (addr)
                8'h48: begin arm_q <= 1'b0; stop_q <= 1'b0; ptr <= 9'd0; end
                8'h49: arm_q <= 1'b1;
                8'h4A: stop_q <= 1'b1;
                8'h4B: ptr <= 9'd0;
                8'h4C: ptr <= ptr + 9'd1;
                8'h4D: ptr <= ptr + 9'd32;
                default: ;
            endcase
        end
    end

    // ------------------------------------------- H16 reset, arm, window --
    // rst: asserted with clr_g, released synchronously to H16.
    reg [1:0] rs = 2'b00;
    always @(posedge h16 or posedge clr_g) begin
        if (clr_g) rs <= 2'b11;
        else       rs <= {rs[0], 1'b0};
    end
    wire rst = rs[1];

    reg [1:0] arm_s, stop_s;
    reg z_run, run, done, frozen, started;
    reg [4:0] zc;
    reg [31:0] ticks;
    reg [7:0] post;
    always @(posedge h16 or posedge rst) begin
        if (rst) begin
            arm_s <= 2'b00; stop_s <= 2'b00;
            z_run <= 1'b1; zc <= 5'd0;
            run <= 1'b0; done <= 1'b0; frozen <= 1'b0; started <= 1'b0;
            ticks <= 32'd0; post <= 8'd0;
        end else begin
            arm_s <= {arm_s[0], arm_q};
            stop_s <= {stop_s[0], stop_q};
            if (z_run) begin
                zc <= zc + 5'd1;
                if (&zc) z_run <= 1'b0;
            end
            if (run) begin
                ticks <= ticks + 32'd1;
                if (ticks == ((32'd1 << WIN_LOG2) - 32'd1) || stop_s[1]) begin
                    run <= 1'b0;
                    done <= 1'b1;
                end
            end else if (arm_s[1] && !done && !z_run && !stop_s[1]) begin
                run <= 1'b1;
            end
            if (done && !frozen) begin
                post <= post + 8'd1;
                if (&post) frozen <= 1'b1;
            end
            if (run) started <= 1'b1;
        end
    end
    wire g_on = run;

    // ------------------------------------------------ input synchronisers --
    reg [32:0] c1, c2, c3;
    reg k1, k2, k3;
    always @(posedge h16) begin
        c1 <= dc;  c2 <= c1;  c3 <= c2;
        k1 <= c4;  k2 <= k1;  k3 <= k2;
    end
    // Signal index k: 0 B4/INTRQ, 1 RDn, 2 CSn1, 3 CSn0, 4 WRn, 5 DMACKn,
    // 6 RSTn, 7 C4.
    wire [7:0] cur = {k2, c2[24], c2[23], c2[22], c2[19], c2[20], c2[21], c2[25]};
    wire [7:0] prv = {k3, c3[24], c3[23], c3[22], c3[19], c3[20], c3[21], c3[25]};
    wire [7:0] rise = cur & ~prv;
    wire [7:0] fall = ~cur & prv;
    wire b4 = cur[0], rd = cur[1];

    reg [7:0] lvl_start, lvl_end;
    always @(posedge h16 or posedge rst) begin
        if (rst) begin
            lvl_start <= 8'd0; lvl_end <= 8'd0;
        end else if (g_on) begin
            if (!started) lvl_start <= prv;
            lvl_end <= cur;
        end
    end

    // ----------------------------------------------------------- helpers --
    function [2:0] wbin;  // pulse width in ticks: 1,2,3-4,5-8,9-16,17-64,65-1024,>1024
        input [10:0] w;
        begin
            if (w <= 11'd1) wbin = 3'd0;
            else if (w == 11'd2) wbin = 3'd1;
            else if (w <= 11'd4) wbin = 3'd2;
            else if (w <= 11'd8) wbin = 3'd3;
            else if (w <= 11'd16) wbin = 3'd4;
            else if (w <= 11'd64) wbin = 3'd5;
            else if (w <= 11'd1024) wbin = 3'd6;
            else wbin = 3'd7;
        end
    endfunction
    function [2:0] abin;  // age in ticks: 0,1,2,3,4-7,8-15,16-63,>=64
        input [6:0] a;
        begin
            if (a <= 7'd3) abin = a[2:0];
            else if (a <= 7'd7) abin = 3'd4;
            else if (a <= 7'd15) abin = 3'd5;
            else if (a <= 7'd63) abin = 3'd6;
            else abin = 3'd7;
        end
    endfunction

    // ----------------------------------------- (d) pulse width counters --
    // bw: B4 high run, rw: RDn low run, in ticks, saturating at 2047. A
    // pulse is binned at its end edge if its start edge was in the window.
    reg [10:0] bw, rw;
    reg bv, rv;
    always @(posedge h16 or posedge rst) begin
        if (rst) begin
            bw <= 11'd0; rw <= 11'd0; bv <= 1'b0; rv <= 1'b0;
        end else begin
            if (rise[0]) begin bw <= 11'd1; bv <= g_on; end
            else if (b4 && !(&bw)) bw <= bw + 11'd1;
            if (fall[0]) bv <= 1'b0;
            if (fall[1]) begin rw <= 11'd1; rv <= g_on; end
            else if (!rd && !(&rw)) rw <= rw + 11'd1;
            if (rise[1]) rv <= 1'b0;
        end
    end
    wire ev_db = g_on & fall[0] & bv;
    wire ev_dr = g_on & rise[1] & rv;
    wire [4:0] bin_db = {2'b00, wbin(bw)};
    wire [4:0] bin_dr = {2'b00, wbin(rw)};

    // ----------------------------------------------- (e) coincidence ages --
    // Ticks since the last RDn fall / RDn rise / CSn1 fall / CSn1 rise,
    // saturating at 127 (127 also = "none yet"); 0 = the same tick.
    wire [3:0] aedge = {rise[2], fall[2], rise[1], fall[1]};
    reg [6:0] ag0, ag1, ag2, ag3;
    always @(posedge h16 or posedge rst) begin
        if (rst) begin
            ag0 <= 7'd127; ag1 <= 7'd127; ag2 <= 7'd127; ag3 <= 7'd127;
        end else begin
            ag0 <= aedge[0] ? 7'd1 : (&ag0 ? ag0 : ag0 + 7'd1);
            ag1 <= aedge[1] ? 7'd1 : (&ag1 ? ag1 : ag1 + 7'd1);
            ag2 <= aedge[2] ? 7'd1 : (&ag2 ? ag2 : ag2 + 7'd1);
            ag3 <= aedge[3] ? 7'd1 : (&ag3 ? ag3 : ag3 + 7'd1);
        end
    end
    wire ev_e = g_on & rise[0];
    wire [4:0] bin_e0 = {2'b00, abin(aedge[0] ? 7'd0 : ag0)};
    wire [4:0] bin_e1 = {2'b00, abin(aedge[1] ? 7'd0 : ag1)};
    wire [4:0] bin_e2 = {2'b00, abin(aedge[2] ? 7'd0 : ag2)};
    wire [4:0] bin_e3 = {2'b00, abin(aedge[3] ? 7'd0 : ag3)};

    // RDn falls followed by a B4 rise within 0..4 ticks (counted once per fall).
    reg [2:0] pend;
    reg ev_w4;
    always @* begin
        ev_w4 = 1'b0;
        if (g_on && rise[0] && (fall[1] || pend != 3'd0)) ev_w4 = 1'b1;
    end
    always @(posedge h16 or posedge rst) begin
        if (rst) pend <= 3'd0;
        else if (fall[1] && g_on) pend <= rise[0] ? 3'd0 : 3'd4;
        else if (pend != 3'd0) pend <= rise[0] ? 3'd0 : pend - 3'd1;
    end

    // ------------------------------------------- (g) protocol histograms --
    // At RDn / WRn fall + 2 ticks: bin {CSn0, CSn1, A2, A1, A0}.
    // DATA AND / OR over the reads that still have RDn low at +2.
    reg rf1, rf2, wf1, wf2;
    always @(posedge h16 or posedge rst) begin
        if (rst) begin rf1 <= 1'b0; rf2 <= 1'b0; wf1 <= 1'b0; wf2 <= 1'b0; end
        else begin
            rf1 <= g_on & fall[1]; rf2 <= rf1;
            wf1 <= g_on & fall[4]; wf2 <= wf1;
        end
    end
    wire [4:0] pbin = {c2[19], c2[20], c2[18], c2[17], c2[16]};
    wire ev_qr = rf2 & ~rd;
    reg [15:0] d_and, d_or;
    always @(posedge h16 or posedge rst) begin
        if (rst) begin d_and <= 16'hFFFF; d_or <= 16'h0000; end
        else if (ev_qr) begin d_and <= d_and & c2[15:0]; d_or <= d_or | c2[15:0]; end
    end

    // -------------------------------- (h) RDn timing, both H16 edges --
    // Half-tick sample stream: per rising edge the pair [c2[21], rn2] =
    // samples at t-2 and t-1.5 ticks; hp = the previous pair's last sample.
    reg rn1, rn2;
    always @(negedge h16) begin rn1 <= dc[21]; rn2 <= rn1; end
    reg hp;
    reg hlv, hpv;
    reg [15:0] hlw, hpc, minw, maxw, minp, maxp;
    reg        t_lv, t_pv;
    reg [15:0] t_lw, t_pc, t_minw, t_maxw, t_minp, t_maxp;
    task hstep;
        input p, x, en;
        begin
            if (p && !x) begin                         // fall
                if (t_pv && en) begin
                    if (t_pc < t_minp) t_minp = t_pc;
                    if (t_pc > t_maxp) t_maxp = t_pc;
                end
                t_pc = 16'd1; t_pv = en;
                t_lw = 16'd1; t_lv = en;
            end else begin
                if (!(&t_pc)) t_pc = t_pc + 16'd1;
                if (!p && x) begin                     // rise
                    if (t_lv && en) begin
                        if (t_lw < t_minw) t_minw = t_lw;
                        if (t_lw > t_maxw) t_maxw = t_lw;
                    end
                    t_lv = 1'b0;
                end else if (!x && !(&t_lw)) begin
                    t_lw = t_lw + 16'd1;
                end
            end
        end
    endtask
    always @* begin
        t_lv = hlv; t_pv = hpv; t_lw = hlw; t_pc = hpc;
        t_minw = minw; t_maxw = maxw; t_minp = minp; t_maxp = maxp;
        hstep(hp, c2[21], g_on);
        hstep(c2[21], rn2, g_on);
    end
    always @(posedge h16 or posedge rst) begin
        if (rst) begin
            hp <= 1'b1; hlv <= 1'b0; hpv <= 1'b0; hlw <= 16'd0; hpc <= 16'd0;
            minw <= 16'hFFFF; maxw <= 16'd0; minp <= 16'hFFFF; maxp <= 16'd0;
        end else begin
            hp <= rn2;
            hlv <= t_lv; hpv <= t_pv; hlw <= t_lw; hpc <= t_pc;
            minw <= t_minw; maxw <= t_maxw; minp <= t_minp; maxp <= t_maxp;
        end
    end

    // ----------------------------------------------- (b) async counters --
    wire [23:0] ab4, ard;
    g1x_acnt cb4 (.aclk(dc[25]),  .arst(clr_g), .en_h(run), .h16(h16), .hrst(rst),
                  .freeze(frozen), .val(ab4));
    g1x_acnt crd (.aclk(~dc[21]), .arst(clr_g), .en_h(run), .h16(h16), .hrst(rst),
                  .freeze(frozen), .val(ard));

    // ------------------------------- region 0: scalar counters (scanner) --
    // slot 0..7 rises of k, 8..15 falls of k, 16..19 ticks with
    // {B4, RDn} = 00, 01, 10, 11, 20 RDn falls followed by a B4 rise
    // within 4 ticks, 21 reads still low at +2 (the DATA AND/OR reads).
    localparam NS = 22;
    wire [NS-1:0] ev;
    assign ev[7:0]   = {8{g_on}} & rise;
    assign ev[15:8]  = {8{g_on}} & fall;
    assign ev[16]    = g_on & ~b4 & ~rd;
    assign ev[17]    = g_on & ~b4 &  rd;
    assign ev[18]    = g_on &  b4 & ~rd;
    assign ev[19]    = g_on &  b4 &  rd;
    assign ev[20]    = ev_w4;
    assign ev[21]    = ev_qr;

    reg [4:0] slot;
    wire scan_on = ~frozen & ~z_run;
    reg [5:0] pre [0:NS-1];
    reg [5:0] dlt;
    reg       hv;
    reg [4:0] hwa;
    integer si;
    always @(posedge h16 or posedge rst) begin
        if (rst) begin
            slot <= 5'd0; dlt <= 6'd0; hv <= 1'b0; hwa <= 5'd0;
            for (si = 0; si < NS; si = si + 1) pre[si] <= 6'd0;
        end else begin
            if (scan_on) slot <= slot + 5'd1;
            hv <= scan_on;
            hwa <= slot;
            dlt <= (slot < NS) ? pre[slot] : 6'd0;
            for (si = 0; si < NS; si = si + 1)
                if (scan_on && slot == si)
                    pre[si] <= {5'd0, ev[si]};
                else
                    pre[si] <= pre[si] + {5'd0, ev[si]};
        end
    end

    // ------------------------------------------------------------ RAMs --
    // Event registers (one tick of read latency, then write q + 1).
    reg [7:0] hv_ev;          // regions 1..8 event pending write
    reg [4:0] hb [1:8];       // their bins
    wire [8:1] evr = {wf2, rf2, ev_dr, ev_db, ev_e, ev_e, ev_e, ev_e};
    wire [4:0] binr [1:8];
    assign binr[1] = bin_e0;  assign binr[2] = bin_e1;
    assign binr[3] = bin_e2;  assign binr[4] = bin_e3;
    assign binr[5] = bin_db;  assign binr[6] = bin_dr;
    assign binr[7] = pbin;    assign binr[8] = pbin;
    integer ri;
    always @(posedge h16 or posedge rst) begin
        if (rst) begin
            hv_ev <= 8'd0;
            for (ri = 1; ri <= 8; ri = ri + 1) hb[ri] <= 5'd0;
        end else begin
            for (ri = 1; ri <= 8; ri = ri + 1) begin
                hv_ev[ri-1] <= evr[ri];
                hb[ri] <= binr[ri];
            end
        end
    end

    wire [31:0] q [0:8];
    // region 0
    g1x_ram r0 (.clk(h16),
                .we(z_run | hv), .wa(z_run ? zc : hwa),
                .wd(z_run ? 32'd0 : q[0] + {26'd0, dlt}),
                .ra(scan_on ? slot : ptr[4:0]), .q(q[0]));
    genvar gi;
    generate
        for (gi = 1; gi <= 8; gi = gi + 1) begin : hr
            g1x_ram r (.clk(h16),
                       .we(z_run | hv_ev[gi-1]), .wa(z_run ? zc : hb[gi]),
                       .wd(z_run ? 32'd0 : q[gi] + 32'd1),
                       .ra(evr[gi] ? binr[gi] : ptr[4:0]), .q(q[gi]));
        end
    endgenerate

    // ------------------------------------------------------- read path --
    reg [31:0] item;
    always @* begin
        item = 32'd0;
        case (ptr[8:5])
            4'd0: item = q[0];
            4'd1: item = q[1];
            4'd2: item = q[2];
            4'd3: item = q[3];
            4'd4: item = q[4];
            4'd5: item = q[5];
            4'd6: item = q[6];
            4'd7: item = q[7];
            4'd8: item = q[8];
            4'd9: case (ptr[4:0])
                5'd0: item = ticks;
                5'd1: item = {8'd0, ab4};
                5'd2: item = {8'd0, ard};
                5'd3: item = {maxw, minw};
                5'd4: item = {maxp, minp};
                5'd5: item = {d_or, d_and};
                5'd6: item = {16'd0, lvl_end, lvl_start};
                5'd7: item = {24'd0, WIN_LOG2};
                default: ;
            endcase
            default: ;
        endcase
    end

    wire [15:0] status = {4'hA, 4'h0, arm_s[1], frozen, done, run, z_run, stop_q, arm_q, clr_g};
    always @* begin
        hit = 1'b1;
        case (addr)
            8'h41: d_out = 16'h5C21;
            8'h42: d_out = status;
            8'h43: d_out = {7'd0, ptr};
            8'h45: d_out = item[15:0];
            8'h46: d_out = item[31:16];
            8'h47: d_out = {8'd0, WIN_LOG2};
            8'h48, 8'h49, 8'h4A, 8'h4B, 8'h4C, 8'h4D, 8'h4E: d_out = status;
            default: begin hit = 1'b0; d_out = 16'h0000; end
        endcase
    end
endmodule

// 32 x 32 simple dual-port RAM, registered read address (M4K).
// No read-during-write check: the g1x schedules never read the word that
// is written in the same tick.
module g1x_ram (
    input  wire        clk,
    input  wire        we,
    input  wire [4:0]  wa,
    input  wire [31:0] wd,
    input  wire [4:0]  ra,
    output wire [31:0] q
);
    (* ramstyle = "M4K, no_rw_check" *) reg [31:0] mem [0:31];
    reg [4:0] ra_r = 5'd0;
    integer i;
    initial for (i = 0; i < 32; i = i + 1) mem[i] = 32'd0;
    always @(posedge clk) begin
        if (we) mem[wa] <= wd;
        ra_r <= ra;
    end
    assign q = mem[ra_r];
endmodule

// Async edge counter (24 bit) clocked by a G1 ball, with a Gray-coded
// crossing into H16 (see the g1x header).
module g1x_acnt (
    input  wire        aclk,
    input  wire        arst,     // clr_g: asserted while the window is off
    input  wire        en_h,     // H16-domain register (window running)
    input  wire        h16,
    input  wire        hrst,
    input  wire        freeze,
    output reg  [23:0] val
);
    reg [1:0] es;
    reg [23:0] bin, gray;
    wire [23:0] nb = bin + 24'd1;
    always @(posedge aclk or posedge arst) begin
        if (arst) begin
            es <= 2'b00; bin <= 24'd0; gray <= 24'd0;
        end else begin
            es <= {es[0], en_h};
            if (es[1]) begin
                bin <= nb;
                gray <= nb ^ (nb >> 1);
            end
        end
    end
    reg [23:0] g1, g2;
    reg [23:0] b;
    integer i;
    always @* begin
        b[23] = g2[23];
        for (i = 22; i >= 0; i = i - 1) b[i] = b[i+1] ^ g2[i];
    end
    always @(posedge h16 or posedge hrst) begin
        if (hrst) begin
            g1 <= 24'd0; g2 <= 24'd0; val <= 24'd0;
        end else begin
            g1 <= gray; g2 <= g1;
            if (!freeze) val <= b;
        end
    end
endmodule
