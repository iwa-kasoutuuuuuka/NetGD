# v55s timing: v55g.sdc unchanged (same ports, same clocks). The new MCU
# input registers of v55s.v (2-FF synchronisers + filters, ADDR / DATA
# capture) are covered by the existing MCU_* -> CLK_48_MHz 20 ns bound.
# MCU_RSTn / G_WRAPn stay false paths (now synchronised in v55s.v too).
# v55g timing (EP2C5F256C8, Quartus II 13.0 SP1 TimeQuest). Hand-written.
# The v5.5 constraints (fpga/Quartus/OpenGDEMU.sdc) with the port lists
# extended for g1gate. The CI fails (no rbf) if any requirement is missed.

create_clock -name {CLK_11_2896_MHz} -period 88.577 -waveform { 0.000 44.290 } [get_ports {CLK_11_2896_MHz}]
derive_pll_clocks
derive_clock_uncertainty
create_clock -name {CLK_48_MHz} -period 20.833 -waveform { 0.000 10.416 } [get_ports {CLK_48_MHz}]

# CLK_48_MHz (core, g1gate) and CLK_11_2896_MHz (audio xtal, its PLL'd
# 33.8688 MHz CDCLK, and g1gate's H16 liveness watchdog) are unrelated.
# Crossings: 2-flop synchronisers (audio_tick, the CDDA request toggle,
# the watchdog's H16 divider and its 'alive' flag) or registers that settle
# long before they are sampled. The 'alive' flag also gates the G1 output
# enables combinationally (turn-off only), which needs no timing.
set_clock_groups -asynchronous \
    -group {CLK_48_MHz} \
    -group [get_clocks {CLK_11_2896_MHz audio_pll_inst|altpll_component|pll|clk[0]}]

# --- SAM3U SMC bus -------------------------------------------------------
# Read access: address / strobes / wrapper mode select -> MCU_DATA.
set_max_delay -from [get_ports {MCU_ADDR[*] MCU_NCS MCU_NRE MCU_NBS[*] G_WRAPn}] \
              -to   [get_ports {MCU_DATA[*]}] 60.000
set_min_delay -from [get_ports {MCU_ADDR[*] MCU_NCS MCU_NRE MCU_NBS[*] G_WRAPn}] \
              -to   [get_ports {MCU_DATA[*]}] 0.000
set_max_delay -from [get_ports {MCU_DATA[*] MCU_ADDR[*] MCU_NCS MCU_NWE MCU_NBS[*]}] \
              -to   [get_clocks {CLK_48_MHz}] 20.000

# --- Dreamcast G1 IDE bus ------------------------------------------------
# As v5.5: register -> pad 20 ns (now including g1gate's output mux).
set_max_delay -from [get_clocks {CLK_48_MHz}] \
              -to   [get_ports {DC_DATA[*] DC_INTRQ DC_DMARQ DC_IORDY}] 20.000
set_max_delay -from [get_ports {DC_DATA[*] DC_ADDR[*] DC_CSn[*] DC_RDn DC_WRn DC_DMACKn}] \
              -to   [get_clocks {CLK_48_MHz}] 20.000

# g1gate's immediate release: raw strobe / reset / master pads -> output
# enable of the G1 pads (combinational). Bounded so the "stop at once"
# property cannot silently degrade.
set_max_delay -from [get_ports {DC_RDn DC_WRn DC_DMACKn DC_CSn[*] DC_ADDR[*] DC_RSTn G_MASTERn}] \
              -to   [get_ports {DC_DATA[*] DC_INTRQ DC_DMARQ DC_IORDY DC_CDCLK DC_SCK DC_SDAT DC_LRCK DC_EMPH}] 30.000

# The pad readback (the same DC_* pads used as inputs by g1gate's collision
# monitor) is sampled asynchronously on both CLK_48_MHz edges; there is no
# external clock to time it against.
set_false_path -from [get_ports {DC_INTRQ DC_DMARQ DC_IORDY DC_CDCLK DC_SCK DC_SDAT DC_LRCK DC_EMPH}] \
               -to   [get_clocks {CLK_48_MHz}]

# Static / slow control inputs, synchronised in g1gate.
set_false_path -from [get_ports {G_MASTERn G_WRAPn MCU_RSTn}] -to [get_clocks {CLK_48_MHz}]

# DC_RSTn gates tri-states and the reset machine (as v5.5).
set_false_path -from [get_ports {DC_RSTn}] -to [get_clocks {CLK_48_MHz}]

# --- CDDA serial audio (as v5.5) ----------------------------------------
set_max_delay -from [get_clocks {CLK_11_2896_MHz}] \
              -to   [get_ports {DC_SCK DC_SDAT DC_LRCK}] 15.000
