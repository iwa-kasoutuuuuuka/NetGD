// NetGD: top-level testbench for fpga/agm/gatesim.py (routed netlist vs RTL).
//
// The same file drives two simulations of the OpenGDEMU top level:
//   - RTL:  src/OpenGDEMU.v + the generated core (fpga/amaranth/out) + the
//           build's Audio_PLL.v (alta_pllve, Supra's model);
//   - gate: Supra's routed netlist (<work>/OpenGDEMU_routed.v) with Supra's
//           own cell models (read from $SUPRA_HOME by gatesim.py).
// It prints a pin-level trace (every output and inout pad, sampled after each
// clock edge, one line per change) plus the value of every bus read, and
// gatesim.py compares the two traces line by line.
//
// The stimulus is fixed: it never depends on a value read back, so the two
// runs get exactly the same pin waveforms however the designs answer, and a
// divergence shows where it starts. Command sequence numbers are tracked here
// (seq0 / seq1), the way the firmware would read them from slot 0.
//
// Coverage, after the fpga/amaranth tests (test.py, test_top.py, test_cdda.py):
//   - MCU register reads of every slot 0..34 (and 63, 127) in both windows,
//     before and after the run, slot 32 (LBA_LOW alone, 0xA5 marker), err_lo
//     (slot 3 bits 0-1), slot 19 masked writes, slot 13, numbered slot 0 / 1
//     writes, Device Control (nIEN, SRST);
//   - ATA PACKET (0xA0): CDB phase opened by the Command write, 6 CDB words,
//     BSY on the last one, then a PIO data-in phase with read flow control
//     (8 words, BSY after the block), INTRQ, completion;
//   - PIO data-out on the secondary (WRITE SECTORS, 8 words);
//   - DMA: two 16-word read bursts (one with a strobe gap seen only by the
//     negative-edge sampler), and a 4-word DMA write on the secondary;
//   - CDDA: enable, 24 PCM words, ~16 frames (pairs, then underflow), clear,
//     flush, disable.
// Lines "EXPECT FAIL" are self-checks taken from those tests (a failure in the
// RTL run means the testbench, not the netlist, needs looking at).
//
// Clocks: 48 MHz with edges on even picoseconds, the 11.2896 MHz xtal on odd
// ones, so no two clock edges ever coincide (a tie would be resolved by the
// simulator's event order, differently in the two runs). Inputs change 5 ns
// after a 48 MHz rising edge; outputs are sampled 4 ns after each edge.
`timescale 1ps/1ps
module tb;
   localparam integer HALF48 = 10416;          // 48.003 MHz
   localparam integer HALF11 = 44290;          // 11.2893 MHz
   reg clk48 = 1'b0, clk11 = 1'b0;
   always #HALF48 clk48 = ~clk48;
   initial begin #1; forever #HALF11 clk11 = ~clk11; end

   reg MCU_RSTn = 1'b0, MCU_NCS = 1'b1, MCU_NRE = 1'b1, MCU_NWE = 1'b1;
   reg [1:0] MCU_NBS = 2'b00;
   reg [8:1] MCU_ADDR = 8'h00;
   reg [15:0] mcu_drv = 16'h0000;
   reg mcu_oe = 1'b0;
   wire [15:0] MCU_DATA = mcu_oe ? mcu_drv : 16'bz;
   wire MCU_IRQn, DC_CDCLK, DC_SCK, DC_SDAT, DC_LRCK, DC_EMPH;
   reg DC_RSTn = 1'b0, DC_WRn = 1'b1, DC_RDn = 1'b1, DC_DMACKn = 1'b1;
   reg [1:0] DC_CSn = 2'b11;
   reg [2:0] DC_ADDR = 3'd0;
   reg [15:0] dc_drv = 16'h0000;
   reg dc_oe = 1'b0;
   wire [15:0] DC_DATA = dc_oe ? dc_drv : 16'bz;
   wire DC_DMARQ, DC_IORDY, DC_INTRQ;
   // No pull-ups: an undriven pad reads z in both runs, so a pad the netlist
   // drives where the RTL does not (an output-enable fault) shows in the trace.

   OpenGDEMU dut (
      .CLK_11_2896_MHz(clk11), .CLK_48_MHz(clk48),
      .MCU_RSTn(MCU_RSTn), .MCU_NCS(MCU_NCS), .MCU_NRE(MCU_NRE), .MCU_NWE(MCU_NWE),
      .MCU_NBS(MCU_NBS), .MCU_ADDR(MCU_ADDR), .MCU_DATA(MCU_DATA), .MCU_IRQn(MCU_IRQn),
      .DC_CDCLK(DC_CDCLK), .DC_SCK(DC_SCK), .DC_SDAT(DC_SDAT), .DC_LRCK(DC_LRCK), .DC_EMPH(DC_EMPH),
      .DC_RSTn(DC_RSTn), .DC_CSn(DC_CSn), .DC_WRn(DC_WRn), .DC_RDn(DC_RDn), .DC_ADDR(DC_ADDR),
      .DC_DATA(DC_DATA), .DC_DMARQ(DC_DMARQ), .DC_DMACKn(DC_DMACKn), .DC_IORDY(DC_IORDY),
      .DC_INTRQ(DC_INTRQ));

   // ---- trace -------------------------------------------------------------
   integer cyc = 0, acyc = 0, fails = 0, cdclk_n = 0;
   always @(posedge clk48) cyc <= cyc + 1;
   always @(posedge DC_CDCLK) cdclk_n = cdclk_n + 1;

   // 48 MHz side: every pad but the CDDA ones, sampled 4 ns after each rising edge.
   reg [37:0] last_p;
   initial last_p = {38{1'bx}};
   wire [37:0] pads = {MCU_DATA, MCU_IRQn, DC_DATA, DC_DMARQ, DC_IORDY, DC_INTRQ, DC_EMPH};
   always @(posedge clk48) begin
      #4000;
      if (pads !== last_p)
         $display("P %0d mcu=%h irqn=%b dc=%h dmarq=%b iordy=%b intrq=%b emph=%b",
                  cyc, MCU_DATA, MCU_IRQn, DC_DATA, DC_DMARQ, DC_IORDY, DC_INTRQ, DC_EMPH);
      last_p = pads;
   end
   // Audio side: SCK / LRCK / SDAT, 4 ns after each xtal edge.
   reg [2:0] last_a = 3'bxxx;
   // ($time > 0: declaring clk11 = 0 is an event at time 0 under Verilog-2005
   // rules (the RTL run) and none under SystemVerilog rules (the gate run).)
   always @(clk11) if ($time > 0) begin
      #4000;
      acyc = acyc + 1;
      if ({DC_SCK, DC_LRCK, DC_SDAT} !== last_a)
         $display("A %0d sck=%b lrck=%b sdat=%b", acyc, DC_SCK, DC_LRCK, DC_SDAT);
      last_a = {DC_SCK, DC_LRCK, DC_SDAT};
   end
   // CDCLK (PLL output, 33.8688 MHz): rising edges per 1024 core cycles.
   always @(posedge clk48) if (cyc % 1024 == 1023) begin
      #4000 $display("C %0d cdclk_edges=%0d", cyc, cdclk_n);
      cdclk_n = 0;
   end

   // ---- bus tasks (inputs change 5 ns after a rising edge) ---------------
   task tick; begin @(posedge clk48); #5000; end endtask
   task ticks(input integer n); integer k; begin for (k = 0; k < n; k = k + 1) tick; end endtask

   task check(input [255:0] what, input [15:0] got, input [15:0] want, input [15:0] mask);
      if (mask != 16'h0 && ((got ^ want) & mask) !== 16'h0) begin
         $display("EXPECT FAIL %0d %0s: got %h want %h (mask %h)", cyc, what, got, want, mask);
         fails = fails + 1;
      end
   endtask

   task mcu_write(input [7:0] a, input [15:0] d); begin
      MCU_NCS = 1'b0; MCU_ADDR = a; mcu_drv = d; mcu_oe = 1'b1; MCU_NWE = 1'b0;
      ticks(3); MCU_NWE = 1'b1; tick; MCU_NCS = 1'b1; mcu_oe = 1'b0; tick;
      $display("W %0d mcu[%02h]<=%h", cyc, a, d);
   end endtask
   reg [15:0] rv;
   task mcu_read(input [7:0] a); begin
      MCU_NCS = 1'b0; MCU_ADDR = a; MCU_NRE = 1'b0;
      ticks(3); rv = MCU_DATA; $display("R %0d mcu[%02h]=%h", cyc, a, rv);
      MCU_NRE = 1'b1; tick; MCU_NCS = 1'b1; tick;
   end endtask
   task mcu_expect(input [7:0] a, input [15:0] want, input [15:0] mask); begin
      mcu_read(a); check("mcu read", rv, want, mask);
   end endtask

   localparam [1:0] CMD = 2'b10, CTL = 2'b01, IDLE = 2'b11;
   task ide_write(input [1:0] csn, input [2:0] a, input [15:0] d); begin
      DC_CSn = csn; DC_ADDR = a; dc_drv = d; dc_oe = 1'b1; tick;
      DC_WRn = 1'b0; ticks(5); DC_WRn = 1'b1; ticks(5);
      DC_CSn = IDLE; dc_oe = 1'b0; tick;
      $display("I %0d dc[%b,%0d]<=%h", cyc, csn, a, d);
   end endtask
   task ide_read(input [1:0] csn, input [2:0] a); begin
      DC_CSn = csn; DC_ADDR = a; tick;
      DC_RDn = 1'b0; ticks(4); rv = DC_DATA; $display("D %0d dc[%b,%0d]=%h", cyc, csn, a, rv);
      DC_RDn = 1'b1; ticks(4); DC_CSn = IDLE; tick;
   end endtask
   task ide_expect(input [1:0] csn, input [2:0] a, input [15:0] want, input [15:0] mask); begin
      ide_read(csn, a); check("ide read", rv, want, mask);
   end endtask
   // One DMA read cycle inside a burst (DMACKn already low, CSn idle).
   task dma_read(input [15:0] want); begin
      DC_RDn = 1'b0; ticks(3); rv = DC_DATA; $display("M %0d dma=%h", cyc, rv);
      check("dma read", rv, want, 16'hFFFF);
      DC_RDn = 1'b1; ticks(3);
   end endtask
   // Two DMA reads whose gap (10 ns, between a falling and the next rising
   // 48 MHz edge) is seen only by the wrapper's negative-edge sampler.
   task dma_read_narrow(input [15:0] want0, input [15:0] want1); begin
      DC_RDn = 1'b0; ticks(3); rv = DC_DATA; $display("M %0d dma=%h", cyc, rv);
      check("dma read (before narrow gap)", rv, want0, 16'hFFFF);
      #3000 DC_RDn = 1'b1; #10000 DC_RDn = 1'b0;   // edge+8 ns .. edge+18 ns
      ticks(3); rv = DC_DATA; $display("M %0d dma=%h", cyc, rv);
      check("dma read (after narrow gap)", rv, want1, 16'hFFFF);
      DC_RDn = 1'b1; ticks(3);
   end endtask
   task dma_write(input [15:0] d); begin
      dc_drv = d; dc_oe = 1'b1; tick;
      DC_WRn = 1'b0; ticks(5); DC_WRn = 1'b1; ticks(5); dc_oe = 1'b0; tick;
      $display("N %0d dmaw<=%h", cyc, d);
   end endtask

   // Sequence-number tokens, as the firmware builds them from slot 0.
   integer seq0 = 0, seq1 = 0;      // Command writes seen by each device (mod 128)
   function [15:0] ack_tok(input integer s); ack_tok = 16'h8000 | ((s & 127) << 8); endfunction
   function [15:0] st_tok(input integer s);  st_tok  = 16'h8000 | ((s & 63) << 9); endfunction

   integer i;
   reg [15:0] w;
   initial begin
      // ---- 0: power-up, MCU reset, IDE reset ------------------------------
      ticks(20);
      mcu_read(8'h0C);                           // during the core's reset (reads 0)
      MCU_RSTn = 1'b1; ticks(10);
      ide_read(CMD, 7);                          // DC_RSTn low: DC_DATA stays z
      DC_RSTn = 1'b1; ticks(20);

      // ---- 1: every slot, both windows ------------------------------------
      for (i = 0; i <= 34; i = i + 1) mcu_read(i);
      mcu_read(63); mcu_read(127);
      for (i = 0; i <= 34; i = i + 1) mcu_read(8'h80 | i);
      mcu_read(8'hFF);

      // ---- 2: register writes and read-back --------------------------------
      mcu_write(2, 16'h00A0); mcu_expect(2, 16'h00A0, 16'h00FF);
      mcu_write(3, 16'h0034); mcu_expect(3, 16'h0034, 16'hFFFF);
      ide_expect(CMD, 1, 16'h0034, 16'h00FF);
      mcu_write(3, 16'h0001); ide_expect(CMD, 1, 16'h0001, 16'h00FF);   // err_lo
      mcu_write(3, 16'h0057); ide_expect(CMD, 1, 16'h0057, 16'h00FF);
      mcu_write(3, 16'h000F); ide_expect(CMD, 1, 16'h0007, 16'h00FF);   // bit 3 not stored
      mcu_expect(3, 16'h0007, 16'hFFFF);
      mcu_write(3, 16'h0053);
      mcu_write(4, 16'h12EC); mcu_expect(4, 16'h12EC, 16'hFFFF);
      mcu_write(5, 16'hBEEF); mcu_expect(5, 16'h00EF, 16'h00FF);        // low byte only
      mcu_write(6, 16'h5A3C); mcu_write(32, 16'h0042);
      mcu_expect(32, 16'hA542, 16'hFFFF); mcu_expect(6, 16'h5A42, 16'hFFFF);
      mcu_write(7, 16'h7766); mcu_write(8, 16'h9988);
      mcu_expect(7, 16'h7766, 16'hFFFF); mcu_expect(8, 16'h9988, 16'hFFFF);
      ide_expect(CMD, 2, 16'h00EF, 16'h00FF);
      ide_expect(CMD, 3, 16'h0042, 16'h00FF);
      ide_expect(CMD, 4, 16'h005A, 16'h00FF);
      ide_expect(CMD, 5, 16'h0066, 16'h00FF);
      ide_expect(CMD, 6, 16'h00A0, 16'h00FF);
      mcu_write(1, 16'h0050); mcu_expect(1, 16'h0050, 16'h00FF);
      ide_expect(CTL, 6, 16'h0050, 16'h00FF);
      mcu_write(13, 16'h0001); ticks(2); mcu_expect(13, 16'h0001, 16'h0001);
      ide_read(CMD, 7);                          // Status read clears INTRQ
      mcu_expect(13, 16'h0000, 16'h0001);
      mcu_write(19, 16'h0808); mcu_expect(19, 16'h0008, 16'h001F);      // masked: pio_rd_fc
      mcu_write(19, 16'h0404); mcu_expect(19, 16'h000C, 16'h001F);      // masked: pio_wr_fc
      mcu_write(19, 16'h0800); mcu_expect(19, 16'h0004, 16'h001F);
      mcu_write(19, 16'h0000); mcu_expect(19, 16'h0000, 16'h001F);
      // secondary window: err_lo and slot 32 are per device
      mcu_write(8'h83, 16'h0002); mcu_expect(8'h83, 16'h0002, 16'hFFFF);
      mcu_write(8'hA0, 16'h0011); mcu_expect(8'hA0, 16'hA511, 16'hFFFF);
      mcu_expect(32, 16'hA542, 16'hFFFF);
      mcu_write(8'h81, 16'h00D8);
      ide_write(CMD, 6, 16'h0010);               // DEV=1: the secondary answers
      ide_expect(CTL, 6, 16'h00D8, 16'h00FF);
      ide_expect(CMD, 1, 16'h0002, 16'h00FF);
      ide_expect(CMD, 3, 16'h0011, 16'h00FF);
      ide_write(CMD, 6, 16'h0000);
      ide_expect(CTL, 6, 16'h0050, 16'h00FF);
      ide_write(CTL, 6, 16'h0002);               // nIEN
      mcu_expect(0, 16'h0008, 16'h0008);
      ide_write(CTL, 6, 16'h0000);
      mcu_read(26);

      // ---- 3: ATA PACKET, CDB, PIO data-in ---------------------------------
      ide_write(CMD, 6, 16'h0000);
      ide_write(CMD, 7, 16'h00A0); seq0 = seq0 + 1;
      ticks(3);
      ide_expect(CTL, 6, 16'h0058, 16'h00FF);    // CDB phase without the MCU
      ide_expect(CMD, 2, 16'h0001, 16'h00FF);    // IRR C/D=1
      ide_expect(CMD, 1, 16'h0053, 16'h00FF);    // ERROR kept over PACKET
      mcu_expect(0, ack_tok(seq0) | 16'h0002, 16'hFF02);
      mcu_expect(19, 16'h0010, 16'h0010);        // pkt_phase
      mcu_write(0, ack_tok(seq0));               // acknowledge by number
      mcu_expect(0, 16'h0000, 16'h0002);
      for (i = 0; i < 6; i = i + 1) begin
         ide_write(CMD, 0, 16'h0012 + 16'h1100 * i);
         if (i == 2) mcu_expect(1, 16'h0058, 16'h00FF);
      end
      ticks(4);
      mcu_expect(1, 16'h0080, 16'h0088);         // BSY=1 DRQ=0 after the last word
      mcu_expect(19, 16'h0000, 16'h0010);
      mcu_expect(11, 16'd6, 16'hFFFF);
      for (i = 0; i < 6; i = i + 1) mcu_expect(9, 16'h0012 + 16'h1100 * i, 16'hFFFF);
      mcu_expect(11, 16'd0, 16'hFFFF);
      // data-in: 8 words with read flow control
      mcu_write(19, 16'h0808);
      for (i = 0; i < 8; i = i + 1) mcu_write(9, 16'h8001 ^ (16'h0103 * i));
      mcu_expect(10, 16'd8, 16'hFFFF);
      mcu_write(5, 16'h0002);                    // IRR: I/O=1, C/D=0
      mcu_write(6, 16'h1042); mcu_write(7, 16'h5300);   // byte count 0x0010
      mcu_write(1, st_tok(seq0) | 16'h0158);     // DRQ + INTRQ, numbered
      ticks(2);
      ide_expect(CTL, 6, 16'h0058, 16'h00FF);
      ide_expect(CMD, 7, 16'h0058, 16'h00FF);    // clears INTRQ
      ide_expect(CMD, 4, 16'h0010, 16'h00FF);
      ide_expect(CMD, 5, 16'h0000, 16'h00FF);
      for (i = 0; i < 8; i = i + 1) ide_expect(CMD, 0, 16'h8001 ^ (16'h0103 * i), 16'hFFFF);
      ide_expect(CTL, 6, 16'h00D0, 16'h00FF);    // block drained: BSY by itself
      mcu_write(5, 16'h0003);
      mcu_write(1, st_tok(seq0) | 16'h0150);     // completion + INTRQ
      ticks(2);
      ide_expect(CMD, 7, 16'h0050, 16'h00FF);
      mcu_write(19, 16'h0800);
      mcu_read(24); mcu_read(25); mcu_read(27); mcu_read(28);
      mcu_read(15); mcu_read(16); mcu_read(17); mcu_read(22); mcu_read(23);
      // a stale numbered status (old command number) is dropped whole
      mcu_write(1, st_tok(seq0 + 5) | 16'h01D0);
      mcu_expect(1, 16'h0050, 16'h00FF);

      // ---- 4: PIO data-out on the secondary (WRITE SECTORS) ----------------
      ide_write(CMD, 6, 16'h0010);
      ide_write(CMD, 7, 16'h0030); seq1 = seq1 + 1;
      ticks(4);
      mcu_expect(8'h80, ack_tok(seq1) | 16'h0003, 16'hFF03);
      mcu_expect(0, ack_tok(seq0), 16'hFF02);    // the primary did not see it
      mcu_write(8'h80, ack_tok(seq1));
      mcu_write(8'h93, 16'h0004);                // write flow control
      mcu_write(8'h81, st_tok(seq1) | 16'h0058);
      ide_expect(CTL, 6, 16'h0058, 16'h00FF);
      for (i = 0; i < 8; i = i + 1) ide_write(CMD, 0, 16'h1000 + 16'h0111 * i);
      mcu_expect(8'h8B, 16'd8, 16'hFFFF);
      for (i = 0; i < 8; i = i + 1) mcu_expect(8'h89, 16'h1000 + 16'h0111 * i, 16'hFFFF);
      mcu_write(8'h93, 16'h0000);
      mcu_write(8'h81, st_tok(seq1) | 16'h0150);
      ticks(2);
      ide_expect(CMD, 7, 16'h0050, 16'h00FF);
      ide_write(CMD, 6, 16'h0000);

      // ---- 5: DMA ----------------------------------------------------------
      ide_write(CMD, 7, 16'h00A0); seq0 = seq0 + 1;
      ticks(3);
      mcu_write(0, ack_tok(seq0));
      for (i = 0; i < 6; i = i + 1) ide_write(CMD, 0, 16'h0028 + i);
      for (i = 0; i < 6; i = i + 1) mcu_read(9);
      for (i = 0; i < 32; i = i + 1) mcu_write(9, (16'h0101 * i) ^ 16'hA5C3);
      mcu_expect(10, 16'd32, 16'hFFFF);
      mcu_write(19, 16'h0001);                   // DMA to the host (whole-word write)
      mcu_write(1, st_tok(seq0) | 16'h0058);
      ticks(4);
      DC_DMACKn = 1'b0; tick;
      for (i = 0; i < 16; i = i + 1) dma_read((16'h0101 * i) ^ 16'hA5C3);
      DC_DMACKn = 1'b1; ticks(6);
      DC_DMACKn = 1'b0; tick;
      for (i = 16; i < 19; i = i + 1) dma_read((16'h0101 * i) ^ 16'hA5C3);
      dma_read_narrow((16'h0101 * 19) ^ 16'hA5C3, (16'h0101 * 20) ^ 16'hA5C3);
      for (i = 21; i < 32; i = i + 1) dma_read((16'h0101 * i) ^ 16'hA5C3);
      DC_DMACKn = 1'b1; ticks(6);
      mcu_expect(10, 16'd0, 16'hFFFF);
      mcu_write(19, 16'h0100);                   // masked: DMA off
      mcu_write(1, st_tok(seq0) | 16'h0150);
      ticks(2);
      ide_expect(CMD, 7, 16'h0050, 16'h00FF);
      // DMA host -> device, secondary
      ide_write(CMD, 6, 16'h0010);
      mcu_write(8'h93, 16'h0002);
      ticks(4);
      DC_DMACKn = 1'b0; tick;
      for (i = 0; i < 4; i = i + 1) dma_write(16'h5A00 + 16'h1111 * i);
      DC_DMACKn = 1'b1; ticks(4);
      mcu_expect(8'h8B, 16'd4, 16'hFFFF);
      for (i = 0; i < 4; i = i + 1) mcu_expect(8'h89, 16'h5A00 + 16'h1111 * i, 16'hFFFF);
      mcu_write(8'h93, 16'h0000);
      ide_write(CMD, 6, 16'h0000);

      // ---- 6: CDDA ---------------------------------------------------------
      mcu_read(29); mcu_read(30);
      mcu_write(29, 16'h0001);
      for (i = 0; i < 24; i = i + 1) begin
         w = (16'h1235 + 16'h2F0B * i) | 16'h0001;            // odd: LSBs visible
         if (i == 5) w = 16'h8001;
         if (i == 6) w = 16'h7FFF;
         mcu_write(31, w);
      end
      mcu_read(30); mcu_read(29);
      ticks(9000);                               // ~8 frames of 22.7 us
      mcu_read(30); mcu_read(29);
      ticks(9000);                               // the FIFO runs dry: underflow
      mcu_read(30); mcu_read(29);
      mcu_write(29, 16'h0003); mcu_read(29);     // clear underflow
      for (i = 0; i < 4; i = i + 1) mcu_write(31, 16'h4321 + i);
      mcu_read(30);
      mcu_write(29, 16'h0005); ticks(4); mcu_read(30);  // flush
      mcu_write(29, 16'h0000); mcu_read(29);
      ticks(600);

      // ---- 7: SRST, final scan ----------------------------------------------
      ide_write(CTL, 6, 16'h0004); ticks(4);
      mcu_expect(0, 16'h0014, 16'h0014);         // RESET_SEEN, IN_RESET
      mcu_expect(3, 16'h0000, 16'hFFFF);         // err_lo cleared by the reset
      ide_write(CTL, 6, 16'h0000); ticks(4);
      mcu_write(0, ack_tok(seq0) | 16'h0006);    // clear RESET_SEEN only
      mcu_expect(0, 16'h0000, 16'h0004);
      for (i = 0; i <= 34; i = i + 1) mcu_read(i);
      for (i = 0; i <= 34; i = i + 1) mcu_read(8'h80 | i);
      // ---- 8 (+audio_alive only): the xtal heartbeat ----------------------
      // audio_div[15] first rises after 2^15 xtal cycles (2.9 ms); slot 18
      // counts its edges. Long (~6.5 ms simulated), so not run by default.
      if ($test$plusargs("audio_alive")) begin
         mcu_read(8'h12);
         ticks(312000);
         mcu_expect(8'h12, 16'h0002, 16'hFFFF);  // both edges count: 2.9 and 5.8 ms
      end
      ticks(20);
      $display("END %0d expect_fails=%0d", cyc, fails);
      $finish;
   end
endmodule
