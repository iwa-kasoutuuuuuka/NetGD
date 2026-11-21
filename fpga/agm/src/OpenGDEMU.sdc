create_clock -name {CLK_11_2896_MHz} -period 88.577 -waveform { 0.000 44.290 } [get_ports {CLK_11_2896_MHz}]
create_clock -name {CLK_48_MHz} -period 20.833 -waveform { 0.000 10.416 } [get_ports {CLK_48_MHz}]
derive_pll_clocks
set_clock_groups -asynchronous -group {CLK_48_MHz} -group {CLK_11_2896_MHz}
set_max_delay -from [get_ports {MCU_ADDR[*] MCU_NCS MCU_NRE MCU_NBS[*]}] -to [get_ports {MCU_DATA[*]}] 60.000
set_max_delay -from [get_ports {MCU_DATA[*] MCU_ADDR[*] MCU_NCS MCU_NWE MCU_NBS[*]}] -to [get_clocks {CLK_48_MHz}] 20.000
set_max_delay -from [get_clocks {CLK_48_MHz}] -to [get_ports {DC_DATA[*] DC_INTRQ DC_DMARQ DC_IORDY}] 20.000
set_max_delay -from [get_ports {DC_DATA[*] DC_ADDR[*] DC_CSn[*] DC_RDn DC_WRn DC_DMACKn}] -to [get_clocks {CLK_48_MHz}] 20.000
set_false_path -from [get_ports {DC_RSTn}]
set_max_delay -from [get_clocks {CLK_11_2896_MHz}] -to [get_ports {DC_SCK DC_SDAT DC_LRCK}] 15.000
