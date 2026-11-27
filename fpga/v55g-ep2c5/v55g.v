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
module v55g (
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

    // ------------------------------------------------------ MCU bus --
    wire MCU_READ = ~MCU_NCS & ~MCU_NRE;
    wire [15:0] MCU_DATA_OUT;   // core
    wire [15:0] w_rdata;        // g1gate
    wire [15:0] mdo = wrap ? w_rdata : MCU_DATA_OUT;
    assign MCU_DATA[15:8] = (MCU_READ & (wrap | ~MCU_NBS[1])) ? mdo[15:8] : 8'hzz;
    assign MCU_DATA[7:0]  = (MCU_READ & (wrap | ~MCU_NBS[0])) ? mdo[7:0]  : 8'hzz;

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
        .rst(~MCU_RSTn | ~core_run),

        .io_MCU_CS(~MCU_NCS & ~wrap),
        .io_MCU_BS(~MCU_NBS),
        .io_MCU_RD(~MCU_NRE),
        .io_MCU_WR(~MCU_NWE),
        .io_MCU_ADDR(MCU_ADDR),
        .io_MCU_DATA_IN(MCU_DATA),
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
