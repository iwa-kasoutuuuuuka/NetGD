# NetGD: have Supra itself work out the AG16K PLL settings for the audio clock
# (11.2896 MHz x 3 = 33.8688 MHz). Run by build.sh as `af_cmd -f pll.tcl` in
# an empty directory; writes Audio_PLL_PLLVE.v / .ip there. Those two files are
# Supra's output and stay in the work directory: mkpll.py lifts the parameter
# values out of them, nothing of them is checked in.
load_architect -no_work -no_route -type AG16KDDF256 1 1 1000 1000
generate_pll Audio_PLL_PLLVE -pll_type PLLVE -input_freq 11.2896 -output_freqs {33.8688} -fb_mode 0
exit
