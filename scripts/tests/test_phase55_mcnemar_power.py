import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import phase55_mcnemar_power as power  # noqa: E402


class TestPhase55McNemarPower(unittest.TestCase):
    def test_exact_p_value_matches_known_discordance(self):
        self.assertEqual(power.exact_mcnemar_p_value(0, 0), 1.0)
        self.assertEqual(power.exact_mcnemar_p_value(5, 0), 0.0625)
        self.assertLess(power.exact_mcnemar_p_value(6, 0), 0.05)

    def test_power_is_zero_without_discordance(self):
        self.assertEqual(power.exact_power(100, 0.0, 0.0, 0.05), 0.0)

    def test_power_grows_with_sample_size_for_directional_alternative(self):
        small = power.exact_power(50, 0.10, 0.02, 0.05)
        large = power.exact_power(200, 0.10, 0.02, 0.05)
        self.assertGreater(large, small)
        self.assertGreater(large, 0.8)

    def test_plan_returns_first_eligible_size(self):
        result = power.plan_sample_size(0.10, 0.02, 0.05, 0.8, 1, 300)
        self.assertTrue(result["eligible"])
        self.assertGreaterEqual(result["achieved_power"], 0.8)
        if result["sample_size"] > 1:
            prior = power.exact_power(result["sample_size"] - 1, 0.10, 0.02, 0.05)
            self.assertLess(prior, 0.8)

    def test_plan_fails_closed_when_cap_is_too_low(self):
        result = power.plan_sample_size(0.02, 0.01, 0.05, 0.95, 1, 5)
        self.assertFalse(result["eligible"])
        self.assertEqual(result["reason"], "target_power_not_reached_within_max_sample_size")

    def test_invalid_discordance_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "sum"):
            power.exact_power(10, 0.8, 0.3, 0.05)


if __name__ == "__main__":
    unittest.main()
