# EP2C5F256 scan2 core: g1x register map, variants, build

Run A image of `ag16k-ps-plan-run4.md` (section 2). Derived from
`fpga/scan2-ep2c5/scan2_core.v`, `g1x.v`, `scan2.v` (generated),
`tools/pinfind/mk_scan2.py` and `tools/pinfind/ep2c5_scan2_v{0,1,2}.csv`.
Simulated (Icarus), fitted in CI; **not hardware-verified**.

## Build under test

- CI run 46019207933 (workflow `fpga-scan2-ep2c5`, all jobs green), branch
  `fpga/stage0-ep2c5` HEAD baca4a29. Quartus image (fallback):
  `ghcr.io/cepawiel/opengdemu/quartus2@sha256:d468cf045ad46c7c6ac5e4fb07e54e12e35ae4ba87d85841fc6fea82980e4184`.
- Fit (each variant): 4,554 / 4,608 LEs (99 %), 9 M4K (9,216 bits), 0 PLLs.
  TimeQuest slow model setup slack: H16 +6.29 ns, B4 +4.10 ns, RDn
  +4.86 ns (async counters at the nominal 100 MHz); timing met. The CI
  publishes an rbf only if the gate passes and timing is met.
- `ep2c5gate.py --scan2 K` PASS for v0, v1, v2 in CI and re-run locally on
  the downloaded reports.
- Bitstreams (artifacts only, never committed; local copies in
  `out-ci/scan2-run1/`, gitignored):

| variant | file | size (bytes) | sha256 |
|---|---|---|---|
| v0 | `out-ci/scan2-run1/scan2-ep2c5-v0-rbf/scan2_v0.rbf` | 84,146 | `d36c10dd661a7568f0463086312d20817fc277eee60c7e228ef3b7a4a1208c4d` |
| v1 | `out-ci/scan2-run1/scan2-ep2c5-v1-rbf/scan2_v1.rbf` | 84,135 | `d85c9e386ed678b3b915c415112c2a7a990686c8abea48a034d29b50cc48e450` |
| v2 | `out-ci/scan2-run1/scan2-ep2c5-v2-rbf/scan2_v2.rbf` | 84,178 | `96b534e572daf35bb15722b9ad99ec63be02c733605cd04aa7c1cb10fbdebae4` |

## What stays as scan

- `scan_core.v` from `fpga/scan-ep2c5` is instantiated **unchanged**; the
  address map, the commands 0xF0..0xF4, the T0 words, the raw / seen /
  toggle / clock / capture words (`ep2c5-scan.md`) are bit for bit those
  of the scan core. `tb_scan2.v` runs a reference `scan_core` beside
  `scan2_core` on the same pins and compares every read outside the g1x
  set (the 63 sweep-11 T0 addresses twice, all 256 addresses once, the
  whole `tb_scan.v` sequence).
- Ports, directions and pad numbering are those of `scan.v` (the
  wrapper differs only in the module name and the extra `.c4(p_C4)`).
  Outputs: only the 16 MCU_DATA balls, `d_oe = ~NCS3 & ~NRD`
  (combinational, from scan_core). Everything else is an input. No PLL.
  Stage-0 device options.

## Variants (I/O settings only; same logic)

| variant | change against scan (v0) | balls |
|---|---|---|
| v0 | none: weak pull-up ON on every user I/O input (134), OFF on the 8 clock balls and the 16 outputs; bus hold OFF | - |
| v1 | weak pull-up OFF | the 33 DC_* balls + C4, C5 (35) |
| v2 | weak pull-up OFF, bus hold ON | the 33 DC_* balls |

C3, also an unassigned neighbour of B4, keeps its pull-up in v1 (the plan
lists only C4 / C5). `ep2c5gate.py --scan2 K` checks the variant table
against `ep2c5_scan.csv` (same balls / directions / signals, outputs =
the 16 MCU_DATA balls, differing balls exactly the set above) and then
the fitter's reports per ball (weak pull-up and bus hold in the Input
Pins, Output Pins and All Package Pins tables, no PLL, device options).

## g1x addresses (addr = A8..A1)

Only 0x41..0x43 and 0x45..0x4E. 0x40 stays the one-hot word 0xB640 and
0x44 / 0x4F stay scan echo words (`0xA544`, `0xA54F`): both are in the
sweep-11 T0 list, so the plan's range "0x41..0x4F, 0x81..0x8F" is cut
to these 13 addresses. 0x81..0x8F are **not** used (still echo words);
the data is reached through a pointer instead.

| addr | read | side effect (NRD rising, NCS3 low) |
|---|---|---|
| 0x41 | ID 0x5C21 | - |
| 0x42 | status | - |
| 0x43 | ptr (9 bit) | - |
| 0x45 | item[ptr] bits 15..0 | - |
| 0x46 | item[ptr] bits 31..16 | - |
| 0x47 | WIN_LOG2 (28) | - |
| 0x48 | status | clear g1x: `clr_g` high until the end of the next read of another address; arm / stop off, ptr = 0, all registers reset, then all RAM words zeroed in 32 H16 ticks |
| 0x49 | status | arm: the window starts (H16) once zeroing is over |
| 0x4A | status | stop: ends a running window early (or prevents the start) |
| 0x4B | status | ptr = 0 |
| 0x4C | status | ptr += 1 |
| 0x4D | status | ptr += 32 (next region) |
| 0x4E | status | none |

Status: bit 0 `clr_g`, 1 arm (latched), 2 stop (latched), 3 zeroing,
4 window running, 5 window done, 6 frozen, 7 arm seen in H16, 15..12 =
0xA. Expected sequence: 0x48 -> 0xA009, then (another read, >= 32 H16
ticks) 0xA000, 0x49, running 0xA092, frozen 0xA0E2.

Scan's 0xF0 does **not** clear g1x, and 0x48 does not clear scan.
Re-arming needs a clear. After configuration the RAM is zero; the g1x
reset synchroniser powers up asserted (Quartus pushes the async preset
back to a power-up-high level), so g1x also zeroes itself once H16 runs.
The first run should start with 0x48 anyway.

### Protocol

1. 0x48, then any other read; wait >= 1 us (zeroing); 0x49.
2. Wait the window: 2^28 H16 ticks = 5.592 s at 48 MHz (or 0x4A to end
   it early). H16 (PA21 PCK0) must run for the whole window and for the
   readout.
3. Poll 0x42 until bit 6 (frozen): the window end + 256 ticks (flush of
   the scalar pre-counters, async snapshot).
4. 0x4B; then for each item: read 0x45, 0x46, then 0x4C. Leave >= 100 ns
   between the end of a ptr command and the next data read (the RAM read
   address is re-latched by H16; 3 ticks = 63 ns).

## Items (ptr = region * 32 + entry; 32-bit words)

Signal index k: 0 B4 (DC_INTRQ), 1 D10 (DC_RDn), 2 F6 (DC_CSn[1]),
3 CSn0, 4 WRn, 5 DMACKn, 6 RSTn, 7 C4 (control ball). All of them are
2-flop synchronised to H16 (rising edge). Every count is of the window
only (`run`). Counters are 32 bit, no saturation (a window is <= 2^28
ticks).

| region | ptr | contents |
|---|---|---|
| 0 | 0..7 | (a) rising edges of signal k |
| 0 | 8..15 | (a) falling edges of signal k |
| 0 | 16..19 | (f) ticks with {B4, RDn} = 00, 01, 10, 11 (sum = ticks) |
| 0 | 20 | (e) RDn falls followed by a B4 rise 0..4 ticks later (once per fall) |
| 0 | 21 | (g) reads still low 2 ticks after the RDn fall (the AND/OR reads) |
| 0 | 22..31 | 0 |
| 1 | 32..39 | (e) per B4 rise: ticks since the last RDn fall, bins 0,1,2,3,4-7,8-15,16-63,>=64 (none yet = >=64) |
| 2 | 64..71 | (e) same, since the last RDn rise |
| 3 | 96..103 | (e) same, since the last CSn1 fall |
| 4 | 128..135 | (e) same, since the last CSn1 rise |
| 5 | 160..167 | (d) B4 high-pulse width in ticks, bins 1,2,3-4,5-8,9-16,17-64,65-1024,>1024 (pulses that start in the window, binned at their end) |
| 6 | 192..199 | (d) RDn low-pulse width, same bins |
| 7 | 224..255 | (g) per RDn fall, 2 ticks later: bin {CSn0, CSn1, A2, A1, A0} (entry = that 5-bit value; AltStatus poll = 22 = 0b10110) |
| 8 | 256..287 | (g) the same per WRn fall |
| 9 | 288 | ticks in the window |
| 9 | 289 | (b) async B4 rising edges (24 bit) |
| 9 | 290 | (b) async RDn falling edges (24 bit) |
| 9 | 291 | (h) RDn low width min (15..0) / max (31..16), half-ticks (10.4 ns) |
| 9 | 292 | (h) RDn fall-to-fall period min / max, half-ticks |
| 9 | 293 | (g) DATA[15:0] AND (15..0) / OR (31..16) over the reads of entry 21 (no read: FFFF / 0000) |
| 9 | 294 | signal levels: bits 7..0 before the first window tick, 15..8 at the last (rises - falls = end - start, per k) |
| 9 | 295 | WIN_LOG2 |
| 9..15 | other | 0 |

Unused histogram entries (regions 1..6, entries 8..31) read 0.
(c) duty is derived: B4 high ticks = item 18 + 19, RDn low ticks =
item 16 + 18.

### Timing (h)

RDn is also synchronised on the falling edge of H16 (2 flops) and
re-registered on the rising edge before any logic (flop to flop, half a
cycle), paired with the rising-edge sample one tick older: a half-tick
sample stream, two samples per tick. A pair holds at most one fall and
one rise, so min / max are updated once per tick. Low width = samples from the fall to the rise;
period = samples between two falls; both saturate at 0xFFFF, only for
pulses / periods that start in the window. Min starts at 0xFFFF, max at 0.

## Clock domains and metastability

- **H16**: all of g1x except below. Inputs: 2-flop synchronisers (same
  structure as scan_core's c1/c2). The g1x reset is asserted
  asynchronously by `clr_g` and released through 2 H16 flops.
- **NRD**: the command latch and ptr (as scan_core). ptr is used directly
  as the RAM read address; a ptr change can be latched half-way for one
  H16 tick, which is why the readout waits >= 3 ticks after a command.
- **B4 rising / RDn falling** (`g1x_acnt`, global clocks from regular I/O
  balls, constrained as 100 MHz clocks in `scan2.sdc`, asynchronous to
  everything): each counter's enable (`run`) passes 2 flops in its own
  domain, so the first 2 edges of a window are not counted and up to 2
  edges after the end may be. The counter is binary, with a registered
  Gray copy (one bit changes per count); H16 samples the Gray word with
  2 flops and converts it to binary, so every sample is the old or the
  new count, never a mix. The H16 snapshot stops updating at `frozen`,
  so the value the MCU reads cannot change. The counters are reset by
  `clr_g` only while the window is off (enable 0, so the release cannot
  change them). Residual risk: a glitch shorter than the flops' minimum
  clock pulse width can corrupt the async counter itself; the sampled
  counters (region 0) are the reference. The simulated H1 case (4 ns
  glitches) shows async 1138 vs sampled 678 B4 rises.
- **MCU reads of H16 registers** are asynchronous to H16: read only when
  frozen (nothing changes then), or read twice.

## Storage

The scan core already uses 2,940 of 4,608 LEs, so g1x keeps its counters
in M4K RAM (9 blocks of 32 x 32 bit):

- region 0: 22 scalar counters, each a 6-bit pre-counter in logic; a
  scanner adds one pre-counter per tick to its RAM word (2-tick
  read-modify-write, 32-tick round, at most 32 events per round per
  counter). 256 ticks after the window the pre-counters are flushed
  (8 rounds) and `frozen` is set.
- regions 1..8: one RAM per event source (B4 rise x4, B4 fall, RDn rise,
  RDn fall + 2, WRn fall + 2). An edge source fires at most every 2
  ticks, so the 2-tick read-modify-write never meets itself.
- the RAMs are written by H16 only and have a registered read-data port
  (old-data read-during-write, so Quartus adds no pass-through logic; no
  read-during-write case occurs in these schedules anyway).

## Simulation (tb/tb_scan2.v)

Holly model: CS1 low, CS0 high, A2..A0 = 110, DATA all 1, RDn low 125 ns
every 600 ns; INTRQ and DMARQ rest 0. g1x window 2^14 ticks (341 us).
Results (also printed in CI):

| scenario | B4 rises (async) | B4 widths | coincident | verdict |
|---|---|---|---|---|
| quiet | 0 (0) | - | - | none |
| H1 crosstalk: 22 ns pulses / 4 ns glitches 3..33 ns after each RDn edge | 678 (1138) | all 1-2 ticks | 678 (all within 0-3 ticks) | H1 |
| H2: B4 = ~RDn | 569 (569) | = RDn low widths, bin by bin | 569 (age 0) | H2 |
| H3: random 3..20 us pulses, plus 5 C4 toggles and 3 CS0 writes (A=7) | 14 (12) | all 65-1024 | 6 | H3 |

Every scenario also checks: 569 reads all in bin 22 and no other bin,
DATA AND/OR = FFFF/FFFF, RDn low 12 half-ticks (125 ns), period 57..58
half-ticks, async RDn = sampled +-2, contingency sum = ticks, rises -
falls = end - start for all 8 signals, the coincidence histograms each
sum to the B4 rises; H3 the 3 writes in WR bin 15 and C4 5 / 5 edges;
the stop command; ptr arithmetic. Verdict rule of the testbench: H2 if
{B4, RDn} never equal (items 16 and 19 zero); H1 if >= 90 % of B4 pulses
are 1-2 ticks and >= 90 % of B4 rises are 0-3 ticks after an RDn edge;
else H3 (none if no B4 rise).

Locally: Icarus Verilog in WSL (Ubuntu 24.04, `/usr/bin/iverilog`), both
`tb_scan.v` and `tb_scan2.v` print TB PASS.

## Open points

- Not hardware-verified. MCU firmware (psload sweep12 / pinfind_s12) is a
  separate step; it must use the addresses above (pointer readout), not
  the 0x81..0x8F range of the plan.
- The device is full (99 % LEs, scan_core alone is 2,940): any further
  g1x addition needs cuts (e.g. 24-bit histogram words, 16-bit async
  counters) or a smaller scan_core.
- Run A2's write-capture extension (plan 2.4) is not in this image.
- The g1x clear is separate from scan's 0xF0.
