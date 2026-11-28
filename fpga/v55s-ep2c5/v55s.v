// v55g: the v5.5 OpenGDEMU core on the GDEMU 6.0 FPGA (EP2C5F256), with
// every Dreamcast G1 output behind g1gate.v. Run B / C image of
// docs/research/ag16k-ps-plan-run4.md; map and safety design in
// docs/research/ep2c5-v55g.md. Pins: v55g.qsf, generated from
// tools/pinfind/ep2c5_padmap.csv by tools/pinfind/mk_v55g.py.
//
// Differences from fpga/src/OpenGDEMU.v (the v5.5 top), nothing else:
//  * every G1 output / bidir goes through g1gate (off at configuration);
//    the outputs are bidirectional so the pad input buffer gives readback;
//  * two new inputs (weak pull-up on): G_MASTERn (R14, MCU PA28; low =
//    outputs allowed) and G_WRAPn (N8, PA26; low = wrapper mode);
//  * in wrapper mode the MCU reads g1gate's read-only map and the core
//    never sees CS (no collision with the core map);
//  * the core is held in reset unless MCU_RSTn is high AND g1gate's
//    core_run bit (0 at configuration, set only by a command) is set;
//  * MCU_IRQn is driven only while core_run and not in wrapper mode;
//  * core_pll is gone (its output was unused in v5.5, Quartus removed it).
// The Amaranth core (fpga/amaranth/out/OpenGDEMUCore.v) is unchanged.
//
// v55s = v55g (fpga/v55g-ep2c5/v55g.v) with a hardened MCU -> core port;
// g1gate.v, every G1 path, pins and drive strengths are v55g's. v55g gave
// the core the raw, unsynchronised MCU pads: register slots latched D on
// every clock of cs & wr, and a slot-9 FIFO push fired on the clock after
// the (raw) strobe ended, using whatever ADDR / D the pads had then.
// Bit-banged writes were seen as 0x0000 or not at all. Here:
//  * NCS / NWE / NRE / G_WRAPn / MCU_RSTn: 2-FF synchroniser + one more
//    flop; the filtered level changes only when two successive
//    synchronised samples agree (a pulse < 1 clock, 20.8 ns, is never
//    seen; latency 3-4 clocks, 63-83 ns);
//  * ADDR / D are registered from the pads on every idle clock and frozen
//    from the clock the filtered strobe starts until two clocks after it
//    ends: the core only ever sees the values that were on the pads
//    ~3 clocks after NWE / NRE fell;
//  * a write is one clock of core cs & wr, on the clock after the filtered
//    write strobe ends (NWE high again), with the frozen ADDR / D; the
//    core's own FIFO push (wr falling edge) follows on the next clock,
//    still with the frozen values. A read is the filtered NRE level; the
//    FIFO pop at its end also uses the frozen ADDR;
//  * in core mode MCU_DATA is driven only while the raw read (~NCS & ~NRE)
//    AND the filtered core read are both on: a glitch on NRE during a
//    write cannot make the FPGA drive D (turn-on +3-4 clocks, turn-off
//    immediate). Wrapper-mode reads (g1gate) are unchanged;
//  * weak pull-ups on MCU_ADDR[8:1] and MCU_RSTn (v55s.qsf).
// MCU timing assumptions: ADDR / D stable >= ~100 ns before NWE / NRE
// fall and NWE / NRE low >= ~100 ns (the firmware's bit-banged cycles use
// microseconds); >= ~150 ns between strobes; core read data is valid
// ~5 clocks (~105 ns) after NRE falls.
module v55s (
    input  wire        CLK_11_2896_MHz,
    input  wire        CLK_48_MHz,

    input  wire        MCU_RSTn,
    input  wire        MCU_NCS,
    input  wire        MCU_NRE,
    input  wire        MCU_NWE,
    input  wire [1:0]  MCU_NBS,
    input  wire [8:1]  MCU_ADDR,
    inout  wire [15:0] MCU_DATA,
    output wire        MCU_IRQn,

    inout  wire        DC_CDCLK,
    inout  wire        DC_SCK,
    inout  wire        DC_SDAT,
    inout  wire        DC_LRCK,
    inout  wire        DC_EMPH,

    input  wire        DC_RSTn,
    input  wire [1:0]  DC_CSn,
    input  wire        DC_WRn,
    input  wire        DC_RDn,
    input  wire [2:0]  DC_ADDR,
    inout  wire [15:0] DC_DATA,

    inout  wire        DC_DMARQ,
    input  wire        DC_DMACKn,
    inout  wire        DC_IORDY,
    inout  wire        DC_INTRQ,

    input  wire        G_MASTERn,
    input  wire        G_WRAPn
);
    wire wrap = ~G_WRAPn;
    wire core_run;

    // ------------------------------------------- MCU port (v55s) --
    // Filtered, synchronised control levels (pad polarity, active low).
    wire f_ncs, f_nwe, f_nre, f_wrapn, f_rstn;
    v55s_sync #(.INIT(1'b1)) s_ncs   (.clk(CLK_48_MHz), .d(MCU_NCS),  .q(f_ncs));
    v55s_sync #(.INIT(1'b1)) s_nwe   (.clk(CLK_48_MHz), .d(MCU_NWE),  .q(f_nwe));
    v55s_sync #(.INIT(1'b1)) s_nre   (.clk(CLK_48_MHz), .d(MCU_NRE),  .q(f_nre));
    v55s_sync #(.INIT(1'b1)) s_wrapn (.clk(CLK_48_MHz), .d(G_WRAPn),  .q(f_wrapn));
    v55s_sync #(.INIT(1'b0)) s_rstn  (.clk(CLK_48_MHz), .d(MCU_RSTn), .q(f_rstn));

    wire c_cs   = ~f_ncs & f_wrapn;         // core selected (not wrapper)
    wire wr_win = c_cs & ~f_nwe;
    wire c_rd   = c_cs & ~f_nre;
    wire strobe = wr_win | c_rd;
    reg  wr_d = 1'b0, h1 = 1'b0, h2 = 1'b0;
    always @(posedge CLK_48_MHz) begin
        wr_d <= wr_win;
        h1   <= strobe;
        h2   <= h1;
    end
    // One clock after the filtered write strobe ends.
    wire wr_pulse = wr_d & ~wr_win;
    // ADDR / D: follow the pads while idle, frozen from the strobe's first
    // filtered clock through the core's FIFO edge (two clocks after).
    reg [7:0]  a_q = 8'd0;
    reg [15:0] d_q = 16'd0;
    always @(posedge CLK_48_MHz)
        if (~(strobe | h1 | h2)) begin
            a_q <= MCU_ADDR;
            d_q <= MCU_DATA;
        end

    wire MCU_READ = ~MCU_NCS & ~MCU_NRE;
    wire [15:0] MCU_DATA_OUT;   // core
    wire [15:0] w_rdata;        // g1gate
    wire [15:0] mdo = wrap ? w_rdata : MCU_DATA_OUT;
    // Core mode: drive only once the filtered read agrees (raw turn-off).
    wire rd_ok = wrap | c_rd;
    assign MCU_DATA[15:8] = (MCU_READ & rd_ok & (wrap | ~MCU_NBS[1])) ? mdo[15:8] : 8'hzz;
    assign MCU_DATA[7:0]  = (MCU_READ & rd_ok & (wrap | ~MCU_NBS[0])) ? mdo[7:0]  : 8'hzz;

    wire mcuIRQ;
    assign MCU_IRQn = (core_run & ~wrap) ? ~mcuIRQ : 1'bz;

    // ---------------------------------------------------- v5.5 glue --
    reg [15:0] audio_div = 16'd0;
    always @(posedge CLK_11_2896_MHz)
        audio_div <= audio_div + 1'b1;
    wire audio_tick = audio_div[15];

    wire DC_CS = ~DC_CSn[0] | ~DC_CSn[1];
    wire DC_RD = DC_CS & ~DC_RDn;
    wire DC_DMA_RD = ~DC_DMACKn & ~DC_RDn;

    reg dc_rd_half = 1'b0;
    always @(negedge CLK_48_MHz)
        dc_rd_half <= ~DC_RDn;
    reg dc_wr_half = 1'b0;
    always @(negedge CLK_48_MHz)
        dc_wr_half <= ~DC_WRn;

    wire cdclk;
    Audio_PLL audio_pll_inst (
        .inclk0 ( CLK_11_2896_MHz ),
        .c0 ( cdclk )
    );

    wire core_intrq, core_dmarq, core_iordy;
    wire core_sck, core_sdat, core_lrck, core_emph;
    wire [15:0] DC_DATA_OUT;

    OpenGDEMUCore core (
        .clk(CLK_48_MHz),
        .rst(~f_rstn | ~core_run),

        .io_MCU_CS(c_cs | wr_pulse),
        .io_MCU_BS(~MCU_NBS),           // unused by the core
        .io_MCU_RD(~f_nre),
        .io_MCU_WR(wr_pulse),
        .io_MCU_ADDR(a_q),
        .io_MCU_DATA_IN(d_q),
        .io_MCU_DATA_OUT(MCU_DATA_OUT),
        .io_MCU_IRQ(mcuIRQ),

        .io_AUDIO_CLK(CLK_11_2896_MHz),
        .io_DC_CDCLK(),
        .io_DC_SCK(core_sck),
        .io_DC_SDAT(core_sdat),
        .io_DC_LRCK(core_lrck),
        .io_DC_EMPH(core_emph),
        .io_audio_tick(audio_tick),

        .io_IDE_RSTn(DC_RSTn),
        .io_IDE_CSn(DC_CSn),
        .io_IDE_WR(~DC_WRn),
        .io_IDE_RD(~DC_RDn),
        .io_IDE_RD_H(dc_rd_half),
        .io_IDE_WR_H(dc_wr_half),
        .io_IDE_ADDR(DC_ADDR),
        .io_IDE_DATA_IN(DC_DATA),
        .io_IDE_DATA_OUT(DC_DATA_OUT),

        .io_IDE_DMARQ(core_dmarq),
        .io_IDE_DMACKn(DC_DMACKn),
        .io_IDE_IORDY(core_iordy),
        .io_IDE_INTRQ(core_intrq)
    );

    // ------------------------------------------------------- g1gate --
    // ball index: 0 INTRQ, 1 DMARQ, 2 IORDY, 3..18 DATA0..15,
    //             19 CDCLK, 20 SCK, 21 SDAT, 22 LRCK, 23 EMPH
    // v5.5 enables: INTRQ / DMARQ / IORDY and the audio lines always (the
    // v5.5 DC_RSTn gating is part of g1gate's 'allow'), DATA on a read.
    wire data_oe = DC_RD | DC_DMA_RD;
    wire [23:0] core_oe  = {5'b11111, {16{data_oe}}, 3'b111};
    wire [23:0] core_val = {core_emph, core_lrck, core_sdat, core_sck, cdclk,
                            DC_DATA_OUT, core_iordy, core_dmarq, core_intrq};
    wire [23:0] pad_in   = {DC_EMPH, DC_LRCK, DC_SDAT, DC_SCK, DC_CDCLK,
                            DC_DATA, DC_IORDY, DC_DMARQ, DC_INTRQ};
    wire [23:0] pin_oe, pin_val;

    g1gate gate (
        .clk(CLK_48_MHz),
        .clk_x(CLK_11_2896_MHz),
        .mcu_ncs(MCU_NCS),
        .mcu_nre(MCU_NRE),
        .mcu_addr(MCU_ADDR),
        .wrap_n(G_WRAPn),
        .master_n(G_MASTERn),
        .w_rdata(w_rdata),
        .core_run(core_run),
        .dc_rstn(DC_RSTn),
        .dc_rdn(DC_RDn),
        .dc_wrn(DC_WRn),
        .dc_dmackn(DC_DMACKn),
        .dc_csn(DC_CSn),
        .dc_addr(DC_ADDR),
        .pad_in(pad_in),
        .core_oe(core_oe),
        .core_val(core_val),
        .pin_oe(pin_oe),
        .pin_val(pin_val)
    );

    assign DC_INTRQ = pin_oe[0] ? pin_val[0] : 1'bz;
    assign DC_DMARQ = pin_oe[1] ? pin_val[1] : 1'bz;
    assign DC_IORDY = pin_oe[2] ? pin_val[2] : 1'bz;
    genvar i;
    generate
        for (i = 0; i < 16; i = i + 1) begin : g_data
            assign DC_DATA[i] = pin_oe[3 + i] ? pin_val[3 + i] : 1'bz;
        end
    endgenerate
    assign DC_CDCLK = pin_oe[19] ? pin_val[19] : 1'bz;
    assign DC_SCK   = pin_oe[20] ? pin_val[20] : 1'bz;
    assign DC_SDAT  = pin_oe[21] ? pin_val[21] : 1'bz;
    assign DC_LRCK  = pin_oe[22] ? pin_val[22] : 1'bz;
    assign DC_EMPH  = pin_oe[23] ? pin_val[23] : 1'bz;
endmodule

// 2-FF synchroniser + glitch filter: q follows d only after two successive
// synchronised samples agree (s[1] == s[2]). s[0] may go metastable; s[1]
// is the synchronised copy. A pulse shorter than one clock is never taken.
module v55s_sync #(parameter INIT = 1'b1) (
    input  wire clk,
    input  wire d,
    output reg  q
);
    reg [2:0] s = {3{INIT}};
    initial q = INIT;
    always @(posedge clk) begin
        s <= {s[1:0], d};
        if (s[1] == s[2])
            q <= s[1];
    end
endmodule
