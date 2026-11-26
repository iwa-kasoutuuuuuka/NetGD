import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
import check_pins  # noqa: E402

HEAD = """ -- Copyright (C) 1991-2013 Altera Corporation
CHIP  "stage0"  ASSIGNED TO AN: EP2C5F256C8

Pin Name/Usage                 : Location  : Dir.   : I/O Standard      : Voltage : I/O Bank  : User Assignment
-------------------------------------------------------------------------------------------------------------
"""
GOOD = HEAD + """GND                            : A1        : gnd    :                   :         :           :
RESERVED_INPUT                 : A3        :        :                   :         : 3         :
~ASDO~ / RESERVED_INPUT        : C1        : input  : 3.3-V LVTTL       :         : 1         : N
nCONFIG                        : H5        :        :                   :         : 1         :
nSTATUS                        : F4        :        :                   :         : 1         :
DCLK                           : H1        : bidir  : 3.3-V LVTTL       :         : 1         : N
CONF_DONE                      : H14       :        :                   :         : 4         :
TDO                            : J4        : output :                   :         : 1         :
VCCIO1                         : J7        : power  :                   : 3.3V    : 1         :
"""


class CheckPins(unittest.TestCase):
    def test_good_report_passes(self):
        bad, n, n_res = check_pins.check(GOOD)
        self.assertEqual(bad, [])
        self.assertEqual((n, n_res), (9, 2))

    def test_gnd_plus_fails(self):
        bad, _, _ = check_pins.check(GOOD + "GND+                           : B3        :        :                   :         : 3         :\n")
        self.assertTrue(any("GND+" in b for b in bad))

    def test_user_output_fails(self):
        bad, _, _ = check_pins.check(GOOD + "LED                            : B4        : output : 3.3-V LVTTL       :         : 3         : Y\n")
        self.assertTrue(any("LED" in b for b in bad))

    def test_reserved_input_driving_fails(self):
        bad, _, _ = check_pins.check(GOOD + "RESERVED_INPUT                 : B5        : output :                   :         : 3         :\n")
        self.assertTrue(any("B5" in b for b in bad))

    def test_nceo_kept_as_programming_pin_fails(self):
        bad, _, _ = check_pins.check(GOOD + "nCEO                           : F16       : output : 3.3-V LVTTL       :         : 4         : N\n")
        self.assertTrue(any("nCEO" in b for b in bad))

    def test_no_reserved_inputs_fails(self):
        bad, _, _ = check_pins.check(HEAD + "GND                            : A1        : gnd    :                   :         :           :\n")
        self.assertTrue(bad)

    def test_empty_fails(self):
        self.assertTrue(check_pins.check("")[0])


if __name__ == "__main__":
    unittest.main()
