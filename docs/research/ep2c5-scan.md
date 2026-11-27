# EP2C5F256 scan core: register map and MCU protocol

Derived from `fpga/scan-ep2c5/scan_core.v`, `scan.v` (generated),
`tools/pinfind/mk_scan.py` and `tools/pinfind/ep2c5_scan.csv`.
Not hardware-verified.

## Build under test

- CI run 45855092435 (workflow `fpga-scan-ep2c5`), branch HEAD 49e1192c.
- `scan.rbf`: 68,796 bytes, sha256
  `600fced48f13100c986d32ab96be151d10ac6b26f4575ff6885d35c8cdb216c1`
  (re-verified locally against the downloaded artifact; never committed).
- `ep2c5gate.py --scan` re-run locally on the CI reports: PASS (same as CI).
- Quartus image digest: see `gate.txt` in the reports artifact.

## Pins

- Outputs: only the 16 MCU_DATA balls (tri-state, pull-up off):
  D0 M11, D1 L11, D2 T14, D3 R13, D4 R12, D5 N12, D6 T7, D7 T8,
  D8 K1, D9 K4, D10 K5, D11 J4, D12 P3, D13 L4, D14 M4, D15 T12.
- Control inputs: NCS3 = R8 (`MCU_NCS`), NRD = M12 (`MCU_NRE` in the table).
- Address inputs (addr[n-1] = A_n): A1 R11, A2 R5, A3 T5, A4 R4, A5 T4,
  A6 P4, A7 P5, A8 T3.
- Every other user I/O is an input, weak pull-up on; the 8 clock balls
  (H1 H2 J1 J2 H15 H16 J15 J16 = clk[0..7]) are inputs, pull-up off.
  142 inputs in total, no PLL, bus hold off. MCU_NWE (K2) and MCU_NBS are
  inputs and unused by the logic.
- 33 DC_* balls form the G1 group (order in mk_scan.DC_ORDER: DATA0..15 =
  0..15, ADDR0..2 = 16..18, CSn0/1 = 19/20, RDn 21, WRn 22, DMACKn 23,
  RSTn 24, INTRQ 25, DMARQ 26, IORDY 27, CDCLK 28, SCK 29, SDAT 30,
  LRCK 31, EMPH 32).
- Reference clock: clk[5] = H16 (CLK_48_MHz).

## Protocol (MCU side)

- The MCU only reads, 16 bits, with address on A8..A1 (value `addr[7:0]`
  below; mapping of the MCU's byte address to A1..A8 is not defined in
  these files and must be checked against the board wiring).
- D0..D15 are driven only while NCS3 AND NRD are both low
  (`d_oe = ~ncs & ~nrd`, combinational). The MCU must never drive them;
  data is valid while both are low (sample before NRD rises).
- There is no write path (NWE unused). Clear and arm are reads of
  reserved addresses; they take effect on the rising edge of NRD while
  NCS3 is low. The data returned by that read is the normal word.
- Unmapped addresses return `{8'hA5, addr}`.

## Commands (reads of 0xF0..0xF4, latched on NRD rising, NCS3 low)

| addr | effect | data read |
|------|--------|-----------|
| 0xF0 | clear: arm_w=arm_g=0; `clr` goes high and asynchronously zeroes every counter, window, seen0/seen1, toggles, captures, wcnt, oticks | status |
| 0xF1 | arm the gated window (arm_w=1) | status |
| 0xF2 | G1 observation on (arm_g=1) | status |
| 0xF3 | G1 observation off (arm_g=0) | status |
| 0xF4 | no effect (reads status) | status |

`clr` stays high until the end of the next read of any other address
(`clr <= (addr==0xF0)` at each NRD rising edge), so read some other
address after 0xF0 before arming. The window runs once (2^WIN_LOG2 H16
cycles, 22 -> 87.4 ms at 48 MHz) after 0xF1 and stays done until the next
clear. The H16 window/G1 logic needs H16 toggling; constant reads do not.

## Address map (addr = A8..A1)

| addr | contents |
|------|----------|
| 0x00 | 0xC0DE |
| 0x03 | 0x3F21 |
| 0x05 | 0x5CA1 |
| 0x06 | status |
| 0x07 | WIN_LOG2 (22) |
| 0x01,02,04,08,10,20,40,80 | one-hot address words 0xB001, B102, B204, B308, B410, B520, B640, B780 (override any other meaning of those addresses) |
| 0x30..0x3F | walking 1: `1 << (addr&15)` |
| 0x50..0x5F | walking 0: `~(1 << (addr&15))` |
| 0x60..0x68 | raw input levels, raw word n = bits 16n..16n+15 (raw index i = ep2c5_scan.csv `raw` column, pad order; upper bits of 0x68 read 0) |
| 0x70..0x78 | seen0 (input was low at least once while armed), same indexing |
| 0x90..0x98 | seen1 (input was high at least once while armed), same indexing |
| 0xA0..0xAF | G1 toggle counters (8 bit saturating, DC index n -> word n/2? see below) |
| 0xB0 | toggle counters tail (`togp[256+:16]`) |
| 0xC0..0xCF | free-running edge count per clock ball: 32 bit, clk index `addr[3:1]`, addr[0]=0 low half, 1 high half |
| 0xD0..0xDF | windowed edge count per clock ball: 24 bit, same indexing; addr[0]=1 returns bits 23:16 in low byte |
| 0xE0..0xEF | capture: addr[0]=0 -> captured DC_DATA word k (k = addr[3:1], 0..7); addr[0]=1 -> bit15 = capture k valid, bits 3:0 = {CSn1, ADDR2..0} |
| 0xF5 | wcnt: count of CS0-qualified DC_WRn falls while armed (16 bit, saturating) |
| 0xF6 / 0xF7 | oticks low / high 16 bits: H16 cycles with G1 observation on |

Status word: `{4'h5, cap_n[3:0], 2'b00, g_en, win_done, win_run, arm_g, arm_w, clr}`
(bit 0 clr, 1 arm_w, 2 arm_g, 3 win_run, 4 win_done, 5 g_en, 8..11 cap_n).

Capture: on each WRn fall (DC_WRn index 22) with CSn0 low while G1
observation is on, the first 8 take DC_DATA[15:0] and {CSn1, ADDR2..0}.

## Unclear / needs checking

- 0xA0..0xAF: the code is `togp[16*lo +: 16]` with 8-bit counters packed
  contiguously, so word lo holds counters 2*lo (low byte) and 2*lo+1 (high
  byte) in DC index order; 0xB0 holds counter 32 (low byte, high byte 0).
  Taken from the Verilog, not from the testbench.
- 0xC0..0xEF: only clk index `addr[3:1]` is decoded, i.e. all eight clocks
  fit in 0x_0..0x_F with two words each (low/high halves).
- Walking and one-hot words at addresses like 0x10, 0x20, 0x40 collide
  with the one-hot override (those addresses return the 0xBxxx words).
- Byte-vs-word MCU addressing (see Protocol) and the timing of `clr`
  relative to the first read after 0xF0 are not specified in the sources.
