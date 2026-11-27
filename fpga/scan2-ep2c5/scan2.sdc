# EP2C5F256C8 scan2 core timing (Quartus II 13.0 SP1 TimeQuest).
# Hand-written. As scan.sdc, plus the two g1x async-counter clocks:
# B4 (DC_INTRQ, counted on its rising edge) and D10 (DC_RDn, counted on its
# falling edge). Their periods are nominal (they are G1 signals, not
# clocks); 10 ns makes TimeQuest check the counters at 100 MHz. Every
# clock is asynchronous to every other one: the cross-domain paths are
# 2-flop synchronisers, the Gray-coded async counters, or registers that
# are read only when frozen (docs/research/ep2c5-scan2.md).
# H16's falling edge (g1x RDn half-tick sampler) is part of clk_h16, so
# the half-cycle transfer to the rising edge is timed.
create_clock -name clk_h16 -period 20.833 [get_ports p_H16]
create_clock -name clk_j2  -period 88.577 [get_ports p_J2]
create_clock -name clk_h1  -period 10.000 [get_ports p_H1]
create_clock -name clk_h2  -period 10.000 [get_ports p_H2]
create_clock -name clk_j1  -period 10.000 [get_ports p_J1]
create_clock -name clk_h15 -period 10.000 [get_ports p_H15]
create_clock -name clk_j15 -period 10.000 [get_ports p_J15]
create_clock -name clk_j16 -period 10.000 [get_ports p_J16]
create_clock -name clk_nrd -period 1000.000 [get_ports p_M12]
create_clock -name clk_b4  -period 10.000 [get_ports p_B4]
create_clock -name clk_rdn -period 10.000 [get_ports p_D10]
set_clock_groups -asynchronous -group {clk_h16} -group {clk_j2} -group {clk_h1} \
    -group {clk_h2} -group {clk_j1} -group {clk_h15} -group {clk_j15} \
    -group {clk_j16} -group {clk_nrd} -group {clk_b4} -group {clk_rdn}
