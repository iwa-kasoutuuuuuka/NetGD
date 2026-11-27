// g1gate: output gate, pulse engine and collision monitor between the v5.5
// core (OpenGDEMUCore) and the 24 Dreamcast G1 output / bidir balls of the
// GDEMU 6.0 FPGA (EP2C5F256). Run B / C of docs/research/ag16k-ps-plan-run4.md;
// register map and safety design: docs/research/ep2c5-v55g.md.
//
// Hand-written. Not hardware-verified.
//
// SAFETY RULES (each one is checked by fpga/v55g-ep2c5/tb/tb_g1gate.v):
//  * Every register powers up 0 / idle: no G1 output is enabled after
//    configuration, and none can be until the MCU issues commands.
//  * pin_oe[b] = (engine or core path) & ~fault[b]
//                & ~master_n (raw R14) & master synchronised
//                & dc_rstn (raw)     & dc_rstn synchronised
//                & H16 alive (watchdog clocked by the J2 crystal).
//    Releasing R14, DC_RSTn going low or H16 stopping removes every enable
//    combinationally (no clock needed for the turn-off).
//  * The core path needs core_run (command, 0 at configuration), the
//    group's enable grp_en[g] (command, expires by itself after ~2.8 s)
//    and the engine idle.
//  * The engine drives one ball (modes 1 / 2) or a masked set of DATA[7:0]
//    (mode 3, Phase C) and expires by itself (43.7 ms / 2.8 s); no ordinary
//    read extends it.
//  * DATA balls are driven by the engine ONLY inside a Dreamcast AltStatus
//    read strobe: the start is a sampled qualification, the stop is
//    combinational from the raw pads (RDn / CSn1 rising, CSn0 / WRn /
//    DMACKn falling, ADDR leaving 6). IORDY is driven by the engine only
//    while the bus is idle (CSn0 = CSn1 = RDn = WRn = DMACKn = 1, raw).
//  * Pulses are at most 48 ticks (1 us) every PERIOD = 48000 ticks (1 ms):
//    the duty is <= 0.1 % by construction.
//  * Collision monitor: every driven ball's pad input is sampled on both
//    H16 edges together with the pad's own oe / value; a sample counts only
//    if oe / value were the same for the two samples before it (>= 1 tick
//    settle). Two consecutive mismatching samples latch fault[b], which
//    removes that ball's enable combinationally (<= ~50 ns from the start
//    of the mismatch). Faults are sticky until reconfiguration; any new
//    fault also stops the engine.
//
// MCU side: read only. MCU_DATA is driven by the top level only while
// NCS3 and NRD are both low; commands are reads of 0x40..0xFF, executed at
// the end of the read (sampled in the H16 domain, the address must be
// stable for two ticks). Nothing here needs the MCU to drive D0..D15.
// The read data is combinational from the address pins, so the ID and
// status words answer before H16 (PA21 / PCK0) runs.
//
// Ball index b (vectors pad_in / core_* / pin_*):
//   0 INTRQ B4   1 DMARQ D11   2 IORDY A6   3..18 DATA0..DATA15
//   19 CDCLK E1  20 SCK A3     21 SDAT B3   22 LRCK F3   23 EMPH D8
// Groups (grp_en bit): 1 INTRQ, 2 DMARQ, 3 IORDY, 4 DATA[7:0],
//   5 DATA[15:8], 6 CDCLK SCK SDAT LRCK EMPH.
module g1gate #(
    parameter integer PERIOD    = 48000, // pulse / level period in H16 ticks (1 ms)
    parameter integer PRE_LOG2  = 13,    // engine timer unit 2^13 ticks (170.7 us)
    parameter integer E_SHORT   = 256,   // engine expiry, short: 2^21 ticks = 43.7 ms
    parameter integer E_LONG    = 16384, // engine expiry, long:  2^27 ticks = 2.80 s
    parameter integer GPRE_LOG2 = 19,    // group timer unit 2^19 ticks (10.9 ms)
    parameter integer G_LOAD    = 256,   // group expiry: 2^27 ticks = 2.80 s
    parameter integer XDEAD     = 8      // J2 cycles without an H16 change -> dead (709 ns)
) (
    input  wire        clk,       // H16 = CLK_48_MHz (MCU PA21 / PCK0)
    input  wire        clk_x,     // J2  = CLK_11_2896_MHz (board crystal)
    // MCU (raw pads)
    input  wire        mcu_ncs,   // NCS3 R8, active low
    input  wire        mcu_nre,   // NRD  M12, active low
    input  wire [7:0]  mcu_addr,  // A8..A1
    input  wire        wrap_n,    // N8 (PA26): low = wrapper mode
    input  wire        master_n,  // R14 (PA28): low = G1 outputs allowed
    output reg  [15:0] w_rdata,   // wrapper read word (combinational)
    output wire        core_run,  // 1 = the core may leave reset
    // G1 inputs (raw pads)
    input  wire        dc_rstn,
    input  wire        dc_rdn,
    input  wire        dc_wrn,
    input  wire        dc_dmackn,
    input  wire [1:0]  dc_csn,
    input  wire [2:0]  dc_addr,
    // the 24 driven balls
    input  wire [23:0] pad_in,    // pad input buffers (readback)
    input  wire [23:0] core_oe,   // v5.5 output enables
    input  wire [23:0] core_val,  // v5.5 output values
    output wire [23:0] pin_oe,
    output wire [23:0] pin_val
);
    localparam [15:0] ID_WORD  = 16'h5C55;
    localparam [15:0] VERSION  = 16'h0001;

    localparam [23:0] M_G1   = 24'h000001;
    localparam [23:0] M_G2   = 24'h000002;
    localparam [23:0] M_G3   = 24'h000004;
    localparam [23:0] M_G4   = 24'h0007F8;
    localparam [23:0] M_G5   = 24'h07F800;
    localparam [23:0] M_G6   = 24'hF80000;
    localparam [23:0] M_DATA = M_G4 | M_G5;
    localparam [23:0] M_IORDY = M_G3;

    // ------------------------------------------------------------ inputs --
    // Raw qualifications (combinational from the pads): used only to STOP
    // a drive immediately.
    wire q_alt_raw  = ~dc_rdn & ~dc_csn[1] & dc_csn[0] & (dc_addr == 3'd6)
                      & dc_wrn & dc_dmackn;
    wire q_idle_raw = dc_rdn & dc_wrn & dc_csn[0] & dc_csn[1] & dc_dmackn;

    // 2-flop synchronisers. Power-up values are the idle levels, except
    // the reset / master / wrapper inputs, which power up "not allowed".
    reg [8:0] g1, g2;   // {addr[2:0], csn[1:0], dmackn, wrn, rdn, rstn}
    initial begin g1 = 9'b000_11_1110; g2 = 9'b000_11_1110; end
    always @(posedge clk) begin
        g1 <= {dc_addr, dc_csn, dc_dmackn, dc_wrn, dc_rdn, dc_rstn};
        g2 <= g1;
    end
    wire       rstn_s  = g2[0];
    wire       rd_s    = g2[1];
    wire       wr_s    = g2[2];
    wire       dmack_s = g2[3];
    wire [1:0] csn_s   = g2[5:4];
    wire [2:0] a_s     = g2[8:6];
    wire q_alt_s  = ~rd_s & ~csn_s[1] & csn_s[0] & (a_s == 3'd6) & wr_s & dmack_s;
    wire q_idle_s = rd_s & wr_s & csn_s[0] & csn_s[1] & dmack_s;
    // first-stage version, only to START a DATA window early (a wrong start
    // is harmless: the pin enable also needs q_alt_raw)
    wire q_alt_1  = ~g1[1] & ~g1[5] & g1[4] & (g1[8:6] == 3'd6) & g1[2] & g1[3];

    reg [11:0] m1, m2;  // {addr[7:0], master_n, wrap_n, nre, ncs}
    initial begin m1 = 12'h00F; m2 = 12'h00F; end
    always @(posedge clk) begin
        m1 <= {mcu_addr, master_n, wrap_n, mcu_nre, mcu_ncs};
        m2 <= m1;
    end
    wire       ncs_s  = m2[0];
    wire       nre_s  = m2[1];
    wire       wrap_s = ~m2[2];       // 1 = wrapper mode
    wire       mst_s  = ~m2[3];       // 1 = master enable held
    wire [7:0] ma_s   = m2[11:4];

    reg [23:0] pad1, pad_s;
    always @(posedge clk) begin
        pad1  <= pad_in;
        pad_s <= pad1;
    end

    // ---------------------------------------- H16 liveness (J2 domain) --
    reg [3:0] hdiv = 4'd0;
    always @(posedge clk) hdiv <= hdiv + 1'b1;   // bit 3: 3 MHz square
    reg [2:0] xs = 3'b000;
    reg [3:0] xcnt = 4'd15;                      // powers up "dead"
    reg       alive = 1'b0;
    always @(posedge clk_x) begin
        xs <= {xs[1:0], hdiv[3]};
        if (xs[2] != xs[1])
            xcnt <= 4'd0;
        else if (xcnt != 4'd15)
            xcnt <= xcnt + 1'b1;
        alive <= (xcnt < XDEAD);
    end
    reg [1:0] alive_s = 2'b00;
    always @(posedge clk) alive_s <= {alive_s[0], alive};

    // ------------------------------------------------- MCU commands --
    wire m_rd = ~ncs_s & ~nre_s & wrap_s;
    reg       m_rd_d = 1'b0;
    reg [7:0] ma_d = 8'h00;
    reg [7:0] cmd_a = 8'h00;
    reg       cmd_ok = 1'b0;
    always @(posedge clk) begin
        m_rd_d <= m_rd;
        ma_d   <= ma_s;
        if (m_rd && !m_rd_d)
            cmd_ok <= 1'b0;
        else if (m_rd && m_rd_d && ma_s == ma_d) begin
            cmd_a  <= ma_s;
            cmd_ok <= 1'b1;
        end
    end
    wire cmd_fire = m_rd_d & ~m_rd & cmd_ok & cmd_a[7:6] != 2'b00;
    wire c_arm     = cmd_fire & (cmd_a == 8'hB0);
    wire c_stop    = cmd_fire & (cmd_a == 8'hB1);
    wire c_release = cmd_fire & (cmd_a == 8'hB2);
    wire c_clear   = cmd_fire & (cmd_a == 8'hB3);
    wire c_run_on  = cmd_fire & (cmd_a == 8'hB4);
    wire c_run_off = cmd_fire & (cmd_a == 8'hB5);
    wire c_grp_off = cmd_fire & (cmd_a == 8'hB8);
    wire c_grp_on  = cmd_fire & (cmd_a[7:3] == 5'b10111) & (cmd_a[2:0] != 3'd0)
                     & (cmd_a[2:0] != 3'd7);

    // staging registers (what the next ARM will use)
    reg [1:0] s_mode = 2'd0;
    reg [4:0] s_sel  = 5'd31;
    reg       s_rest = 1'b0;
    reg       s_long = 1'b0;
    reg [1:0] s_w    = 2'd0;
    reg [5:0] s_n    = 6'd0;
    reg [7:0] s_pat  = 8'h00;
    reg [7:0] s_msk  = 8'h00;
    always @(posedge clk) begin
        if (cmd_fire) begin
            casez (cmd_a)
                8'b0100_????: s_pat[3:0] <= cmd_a[3:0];
                8'b0101_????: s_pat[7:4] <= cmd_a[3:0];
                8'b0110_????: s_msk[3:0] <= cmd_a[3:0];
                8'b0111_????: s_msk[7:4] <= cmd_a[3:0];
                8'b100?_????: s_sel <= cmd_a[4:0];
                8'b1010_00??: s_mode <= cmd_a[1:0];
                8'b1010_01??: s_w <= cmd_a[1:0];
                8'b1010_100?: s_rest <= cmd_a[0];
                8'b1010_101?: s_long <= cmd_a[0];
                8'b11??_????: s_n <= cmd_a[5:0];
                default: ;
            endcase
        end
    end

    // ------------------------------------------------------- timers --
    reg [GPRE_LOG2-1:0] pre = {GPRE_LOG2{1'b0}};
    always @(posedge clk) pre <= pre + 1'b1;
    wire e_tick = &pre[PRE_LOG2-1:0];
    wire g_tick = &pre;

    // ------------------------------------------------- engine state --
    reg        e_arm = 1'b0, e_drive = 1'b0, e_done = 1'b0, e_exp = 1'b0;
    reg        e_pend = 1'b0, e_seen = 1'b0;
    reg [3:0]  e_ref = 4'd0;        // last ARM result: 0 ok, else refusal code
    reg [1:0]  a_mode = 2'd0;
    reg [4:0]  a_sel = 5'd31;
    reg        a_rest = 1'b0;
    reg [1:0]  a_w = 2'd0;
    reg [5:0]  a_n = 6'd0;
    reg [7:0]  a_pat = 8'h00, a_msk = 8'h00;
    reg [14:0] e_tmr = 15'd0;
    reg [15:0] e_per = 16'd0;
    reg [5:0]  e_wcnt = 6'd0;
    reg [15:0] e_pulses = 16'd0, e_follow = 16'd0;
    reg [7:0]  arm_seq = 8'd0;
    reg        core_run_r = 1'b0;
    reg [6:1]  grp_en = 6'd0;
    reg [7:0]  g_tmr1 = 8'd0, g_tmr2 = 8'd0, g_tmr3 = 8'd0,
               g_tmr4 = 8'd0, g_tmr5 = 8'd0, g_tmr6 = 8'd0;
    reg        seen_rst = 1'b0, seen_mdrop = 1'b0;
    reg [23:0] fault = 24'd0;
    assign core_run = core_run_r;

    function [23:0] onehot24(input [4:0] i);
        onehot24 = (i < 5'd24) ? (24'd1 << i) : 24'd0;
    endfunction
    function is_data(input [4:0] i);
        is_data = (i >= 5'd3) && (i <= 5'd18);
    endfunction
    function [5:0] wticks(input [1:0] c);   // pulse width - 1
        case (c)
            2'd0: wticks = 6'd11;   // 0.25 us
            2'd1: wticks = 6'd23;   // 0.5 us
            default: wticks = 6'd47; // 1 us
        endcase
    endfunction

    wire a_data   = (a_mode == 2'd3) || (a_mode != 2'd0 && is_data(a_sel));
    wire a_iordy  = (a_mode == 2'd1 || a_mode == 2'd2) && a_sel == 5'd2;
    wire [23:0] e_mask = (a_mode == 2'd3) ? {13'd0, a_msk, 3'd0}
                       : (a_mode == 2'd0) ? 24'd0 : onehot24(a_sel);
    wire [23:0] e_valv = (a_mode == 2'd3) ? {13'd0, a_pat, 3'd0}
                       : (a_mode == 2'd1) ? {24{a_rest}} : {24{~a_rest}};
    wire [23:0] qual_vec = (M_DATA & {24{q_alt_raw}}) | (M_IORDY & {24{q_idle_raw}})
                         | ~(M_DATA | M_IORDY);
    wire [23:0] eng_oe = (e_arm & e_drive) ? (e_mask & qual_vec) : 24'd0;

    wire [23:0] grp_ball = ({24{grp_en[1]}} & M_G1) | ({24{grp_en[2]}} & M_G2)
                         | ({24{grp_en[3]}} & M_G3) | ({24{grp_en[4]}} & M_G4)
                         | ({24{grp_en[5]}} & M_G5) | ({24{grp_en[6]}} & M_G6);
    wire [23:0] core_path = (core_run_r & ~e_arm) ? (grp_ball & core_oe) : 24'd0;

    // 'alive' is the J2-domain register itself, so a stopped H16 still
    // turns every output off.
    wire allow = ~master_n & mst_s & dc_rstn & rstn_s & alive;
    assign pin_oe  = (eng_oe | core_path) & ~fault & {24{allow}};
    assign pin_val = e_arm ? e_valv : core_val;

    // ------------------------------------------------ monitor --
    reg [23:0] pp = 0, op = 0, vp = 0;     // posedge samples: pad, oe, value
    reg [23:0] pn = 0, onx = 0, vn = 0;     // negedge samples
    always @(posedge clk) begin pp <= pad_in; op <= pin_oe; vp <= pin_val; end
    always @(negedge clk) begin pn <= pad_in; onx <= pin_oe; vn <= pin_val; end
    reg [23:0] op1 = 0, vp1 = 0, on1 = 0, vn1 = 0, mn1 = 0;
    // sample stream ... p[k-1] n[k-1] p[k] n[k]; evaluated at edge k+1
    wire [23:0] val_p = op & on1 & op1 & ~(vp ^ vn1) & ~(vp ^ vp1);
    wire [23:0] val_n = onx & op & on1 & ~(vn ^ vp) & ~(vn ^ vn1);
    wire [23:0] mis_p = val_p & (pp ^ vp);
    wire [23:0] mis_n = val_n & (pn ^ vn);
    wire [23:0] f_set = mis_p & (mis_n | mn1);
    wire [23:0] match = (val_p & ~mis_p) | (val_n & ~mis_n);
    always @(posedge clk) begin
        op1 <= op; vp1 <= vp; on1 <= onx; vn1 <= vn; mn1 <= mis_n;
    end
    wire new_fault = |(f_set & ~fault);
    // Counters and the readback-follows flag use a registered copy (one
    // tick later): the negedge samples leave only half a tick for logic,
    // enough for the per-ball fault path but not for the wide reductions
    // and adders (CI run 46035870326: -1.7 ns). The fault path itself is
    // not delayed.
    reg vp_any = 1'b0, vn_any = 1'b0, mp_any = 1'b0, mn_any = 1'b0;
    reg [23:0] match_r = 24'd0;
    always @(posedge clk) begin
        vp_any <= |val_p; vn_any <= |val_n;
        mp_any <= |mis_p; mn_any <= |mis_n;
        match_r <= match;
    end

    reg        f_valid = 1'b0;
    reg [4:0]  f_ball = 5'd0;
    reg [7:0]  f_seq = 8'd0;
    reg [1:0]  f_mode = 2'd0;
    reg [7:0]  f_cnt = 8'd0;
    reg [32:0] f_lvl = 33'd0;
    reg [31:0] c_valid = 32'd0;
    reg [15:0] c_mis = 16'd0;

    function [4:0] lowest(input [23:0] v);
        integer i;
        begin
            lowest = 5'd31;
            for (i = 23; i >= 0; i = i - 1)
                if (v[i]) lowest = i;
        end
    endfunction
    function [1:0] cnt2(input a, input b);
        cnt2 = {1'b0, a} + {1'b0, b};
    endfunction

    // the 33 synchronised G1 levels (snapshot / live word order)
    wire [32:0] lvl = {rstn_s, dmack_s, wr_s, rd_s, csn_s[1], csn_s[0], a_s,
                       pad_s[23:19], pad_s[2:0], pad_s[18:3]};

    always @(posedge clk) begin
        fault <= fault | f_set;
        if (new_fault && !f_valid) begin
            f_valid <= 1'b1;
            f_ball  <= lowest(f_set);
            f_seq   <= arm_seq;
            f_mode  <= a_mode;
            f_cnt   <= e_pulses[7:0];
            f_lvl   <= lvl;
        end
        if (c_clear) begin
            c_valid <= 32'd0;
            c_mis   <= 16'd0;
        end else begin
            c_valid <= c_valid + cnt2(vp_any, vn_any);
            if (c_mis < 16'hFFFE)
                c_mis <= c_mis + cnt2(mp_any, mn_any);
        end
    end

    // -------------------------------------------------- engine --
    // A new fault stops the engine one tick later (registered: the half-tick
    // path from the negedge samples into the engine missed timing, CI run
    // 46037003725 -0.7 ns); the faulted ball itself is already off.
    reg new_fault_r = 1'b0;
    always @(posedge clk) new_fault_r <= new_fault;
    wire kill = ~mst_s | ~rstn_s | ~wrap_s | c_stop | c_release | new_fault_r;
    wire [23:0] tgt = (s_mode == 2'd3) ? {13'd0, s_msk, 3'd0} : onehot24(s_sel);
    wire tgt_lvl_ok = (s_mode == 2'd3) ? &(pad_s[10:3] | ~s_msk)
                                       : (pad_s[s_sel] == s_rest);
    reg [3:0] ref_code;
    always @* begin
        if (e_arm)                                   ref_code = 4'd6;
        else if (s_mode == 2'd0)                     ref_code = 4'd0;
        else if (!mst_s || master_n)                 ref_code = 4'd1;
        else if (!rstn_s)                            ref_code = 4'd2;
        else if (tgt == 24'd0)                       ref_code = 4'd3;
        else if (|(tgt & fault))                     ref_code = 4'd4;
        else if (!tgt_lvl_ok)                        ref_code = 4'd5;
        else if (!alive_s[1])                        ref_code = 4'd7;
        else                                         ref_code = 4'd0;
    end
    wire per_wrap = (e_per == PERIOD - 1);
    wire last = (e_pulses == {10'd0, a_n});      // a_n + 1 pulses in total

    always @(posedge clk) begin
        if (c_arm) begin
            e_ref <= ref_code;
            if (ref_code == 4'd0) begin
                e_arm   <= 1'b1;
                e_drive <= (s_mode == 2'd1) && !is_data(s_sel);
                e_done  <= 1'b0;
                e_exp   <= 1'b0;
                e_pend  <= 1'b0;
                e_seen  <= 1'b0;
                a_mode  <= s_mode;
                a_sel   <= s_sel;
                a_rest  <= s_rest;
                a_w     <= s_w;
                a_n     <= s_n;
                a_pat   <= s_pat;
                a_msk   <= s_msk;
                e_tmr   <= s_long ? E_LONG : E_SHORT;
                e_per   <= 16'd0;
                e_wcnt  <= 6'd0;
                e_pulses <= 16'd0;
                e_follow <= 16'd0;
                arm_seq <= arm_seq + 1'b1;
            end
        end else if (e_arm) begin
            if (kill) begin
                e_arm   <= 1'b0;
                e_drive <= 1'b0;
            end else if (e_tick && e_tmr <= 15'd1) begin
                e_arm   <= 1'b0;
                e_drive <= 1'b0;
                e_exp   <= 1'b1;
            end else begin
                if (e_tick)
                    e_tmr <= e_tmr - 1'b1;
                if (e_drive && |(match_r & e_mask))
                    e_seen <= 1'b1;
                if (a_data) begin
                    // one window per qualified AltStatus read
                    if (!e_drive) begin
                        if (q_alt_1)
                            e_drive <= 1'b1;
                    end else if (!q_alt_1 && !q_alt_s) begin
                        e_drive  <= 1'b0;
                        e_seen   <= 1'b0;
                        e_pulses <= e_pulses + 1'b1;
                        if (e_seen || |(match_r & e_mask))
                            e_follow <= e_follow + 1'b1;
                        if (last) begin
                            e_arm  <= 1'b0;
                            e_done <= 1'b1;
                        end
                    end
                end else begin
                    e_per <= per_wrap ? 16'd0 : e_per + 1'b1;
                    if (a_mode == 2'd2) begin
                        // N opposite pulses of W ticks, one per period
                        if (per_wrap)
                            e_pend <= 1'b1;
                        if (!e_drive) begin
                            if (e_pend && (!a_iordy || q_idle_s)) begin
                                e_drive <= 1'b1;
                                e_pend  <= 1'b0;
                                e_wcnt  <= wticks(a_w);
                            end
                        end else if (e_wcnt == 6'd0) begin
                            e_drive  <= 1'b0;
                            e_seen   <= 1'b0;
                            e_pulses <= e_pulses + 1'b1;
                            if (e_seen || |(match_r & e_mask))
                                e_follow <= e_follow + 1'b1;
                            if (last) begin
                                e_arm  <= 1'b0;
                                e_done <= 1'b1;
                            end
                        end else
                            e_wcnt <= e_wcnt - 1'b1;
                    end else if (per_wrap) begin
                        // mode 0 (passive) / 1 (level): N + 1 periods
                        e_seen   <= 1'b0;
                        e_pulses <= e_pulses + 1'b1;
                        if (a_mode == 2'd1 && (e_seen || |(match_r & e_mask)))
                            e_follow <= e_follow + 1'b1;
                        if (last) begin
                            e_arm   <= 1'b0;
                            e_drive <= 1'b0;
                            e_done  <= 1'b1;
                        end
                    end
                end
            end
        end
    end

    // ---------------------------------------- core run, group enables --
    wire g_kill = ~mst_s | ~rstn_s | c_release | c_grp_off;
    always @(posedge clk) begin
        if (c_run_on)
            core_run_r <= 1'b1;
        else if (c_run_off || c_release)
            core_run_r <= 1'b0;
        if (g_kill) begin
            grp_en <= 6'd0;
        end else begin
            if (c_grp_on && mst_s && !master_n && rstn_s) begin
                grp_en[cmd_a[2:0]] <= 1'b1;
                case (cmd_a[2:0])
                    3'd1: g_tmr1 <= G_LOAD - 1;
                    3'd2: g_tmr2 <= G_LOAD - 1;
                    3'd3: g_tmr3 <= G_LOAD - 1;
                    3'd4: g_tmr4 <= G_LOAD - 1;
                    3'd5: g_tmr5 <= G_LOAD - 1;
                    default: g_tmr6 <= G_LOAD - 1;
                endcase
            end else if (g_tick) begin
                if (grp_en[1]) begin if (g_tmr1 == 8'd0) grp_en[1] <= 1'b0; else g_tmr1 <= g_tmr1 - 1'b1; end
                if (grp_en[2]) begin if (g_tmr2 == 8'd0) grp_en[2] <= 1'b0; else g_tmr2 <= g_tmr2 - 1'b1; end
                if (grp_en[3]) begin if (g_tmr3 == 8'd0) grp_en[3] <= 1'b0; else g_tmr3 <= g_tmr3 - 1'b1; end
                if (grp_en[4]) begin if (g_tmr4 == 8'd0) grp_en[4] <= 1'b0; else g_tmr4 <= g_tmr4 - 1'b1; end
                if (grp_en[5]) begin if (g_tmr5 == 8'd0) grp_en[5] <= 1'b0; else g_tmr5 <= g_tmr5 - 1'b1; end
                if (grp_en[6]) begin if (g_tmr6 == 8'd0) grp_en[6] <= 1'b0; else g_tmr6 <= g_tmr6 - 1'b1; end
            end
        end
        if (c_clear) begin
            seen_rst   <= 1'b0;
            seen_mdrop <= 1'b0;
        end else begin
            if (rstn_d && !rstn_s) seen_rst <= 1'b1;   // a real fall
            if ((e_arm || grp_en != 6'd0) && !mst_s) seen_mdrop <= 1'b1;
        end
    end

    // ------------------------------------------- G1 bus observation --
    reg rd_d = 1'b1, wr_d = 1'b1, dmack_d = 1'b1, rstn_d = 1'b0, intrq_d = 1'b0;
    reg [15:0] c_alt = 0, c_rdo = 0, c_wr = 0, c_dmack = 0, c_rst = 0, c_intrq = 0;
    reg [2:0]  cap_n = 3'd0;
    reg [6:0]  cap_m0 = 0, cap_m1 = 0, cap_m2 = 0, cap_m3 = 0; // {wr, csn1, csn0, a[2:0], 1}
    reg [15:0] cap_d0 = 0, cap_d1 = 0, cap_d2 = 0, cap_d3 = 0;
    reg [15:0] wdat = 16'd0;
    reg [5:0]  wmeta = 6'd0;
    wire any_cs = ~csn_s[0] | ~csn_s[1];
    wire rd_fall = rd_d & ~rd_s;
    wire wr_fall = wr_d & ~wr_s;
    wire wr_rise = ~wr_d & wr_s;
    wire rd_rise = ~rd_d & rd_s;
    reg  rdo_open = 1'b0;   // a non-AltStatus CS read is in progress
    reg  wr_open  = 1'b0;
    // capture: first four non-AltStatus cycles (CS reads at their end,
    // CS writes with the data sampled on the last tick before WRn rose)
    wire cap_do = (rd_rise & rdo_open) | (wr_rise & wr_open);
    wire [6:0]  cap_meta = {wr_rise & wr_open, wmeta};
    always @(posedge clk) begin
        rd_d <= rd_s; wr_d <= wr_s; dmack_d <= dmack_s; rstn_d <= rstn_s;
        intrq_d <= pad_s[0];
        if (!rd_s || !wr_s) begin
            wdat <= pad_s[18:3];
        end
        if (rd_fall || wr_fall)
            wmeta <= {csn_s[1], csn_s[0], a_s, 1'b1};
        if (rd_fall) rdo_open <= any_cs & ~q_alt_s;
        else if (rd_rise) rdo_open <= 1'b0;
        if (wr_fall) wr_open <= any_cs;
        else if (wr_rise) wr_open <= 1'b0;
        if (c_clear) begin
            c_alt <= 0; c_rdo <= 0; c_wr <= 0; c_dmack <= 0; c_rst <= 0; c_intrq <= 0;
            cap_n <= 3'd0;
            cap_m0 <= 0; cap_m1 <= 0; cap_m2 <= 0; cap_m3 <= 0;
        end else begin
            if (rd_fall && q_alt_s) c_alt <= c_alt + 1'b1;
            if (rd_fall && any_cs && !q_alt_s) c_rdo <= c_rdo + 1'b1;
            if (wr_fall && any_cs) c_wr <= c_wr + 1'b1;
            if (dmack_d && !dmack_s) c_dmack <= c_dmack + 1'b1;
            if (rstn_d && !rstn_s) c_rst <= c_rst + 1'b1;
            if (!intrq_d && pad_s[0]) c_intrq <= c_intrq + 1'b1;
            if (cap_do && cap_n != 3'd4) begin
                cap_n <= cap_n + 1'b1;
                case (cap_n[1:0])
                    2'd0: begin cap_m0 <= cap_meta; cap_d0 <= wdat; end
                    2'd1: begin cap_m1 <= cap_meta; cap_d1 <= wdat; end
                    2'd2: begin cap_m2 <= cap_meta; cap_d2 <= wdat; end
                    default: begin cap_m3 <= cap_meta; cap_d3 <= wdat; end
                endcase
            end
        end
    end

    // ------------------------------------------------- read map --
    wire [15:0] status = {e_ref, seen_mdrop, seen_rst, core_run_r, alive_s[1],
                          rstn_s, wrap_s, mst_s, f_valid, e_exp, e_done, e_drive, e_arm};
    always @* begin
        case (mcu_addr)
            8'h00: w_rdata = ID_WORD;
            8'h01: w_rdata = VERSION;
            8'h02: w_rdata = status;
            8'h03: w_rdata = fault[15:0];
            8'h04: w_rdata = {8'h00, fault[23:16]};
            8'h05: w_rdata = {f_valid, f_mode, f_ball, f_seq};
            8'h06: w_rdata = f_lvl[15:0];
            8'h07: w_rdata = f_lvl[31:16];
            8'h08: w_rdata = {7'd0, f_lvl[32], f_cnt};
            8'h09: w_rdata = {3'd0, s_long, s_rest, s_w, s_mode, 2'd0, s_sel};
            8'h0A: w_rdata = {s_msk, s_pat};
            8'h0B: w_rdata = {arm_seq, 2'd0, s_n};
            8'h0C: w_rdata = {1'b0, e_tmr};
            8'h0D: w_rdata = e_pulses;
            8'h0E: w_rdata = e_follow;
            8'h0F: w_rdata = c_mis;
            8'h10: w_rdata = c_valid[15:0];
            8'h11: w_rdata = c_valid[31:16];
            8'h12: w_rdata = c_alt;
            8'h13: w_rdata = c_rdo;
            8'h14: w_rdata = c_wr;
            8'h15: w_rdata = c_dmack;
            8'h16: w_rdata = c_rst;
            8'h17: w_rdata = c_intrq;
            8'h18: w_rdata = lvl[15:0];
            8'h19: w_rdata = lvl[31:16];
            8'h1A: w_rdata = {lvl[32], 2'd0, cap_n, 3'd0, grp_en, 1'b0};
            8'h1B: w_rdata = {a_msk, a_mode, a_w, 2'd0, a_rest, 1'b0};
            8'h1C: w_rdata = {a_sel, 5'd0, a_n};
            8'h20: w_rdata = {9'd0, cap_m0};
            8'h21: w_rdata = cap_d0;
            8'h22: w_rdata = {9'd0, cap_m1};
            8'h23: w_rdata = cap_d1;
            8'h24: w_rdata = {9'd0, cap_m2};
            8'h25: w_rdata = cap_d2;
            8'h26: w_rdata = {9'd0, cap_m3};
            8'h27: w_rdata = cap_d3;
            default:
                if (mcu_addr[7:4] == 4'h3)
                    w_rdata = 16'd1 << mcu_addr[3:0];   // walking 1
                else if (mcu_addr[7:6] != 2'b00)
                    w_rdata = status;                    // commands
                else
                    w_rdata = {8'hA5, mcu_addr};
        endcase
    end
endmodule
