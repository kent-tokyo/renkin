import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import compare_stats as stats  # noqa: E402


class TestPairedBootstrap(unittest.TestCase):
    def test_deterministic_across_runs_with_fixed_seed(self):
        pairs = [(True, False), (True, True), (False, False), (True, False)] * 10
        r1 = stats.paired_bootstrap_diff(pairs, stats.rate_diff_statistic, n_iterations=500)
        r2 = stats.paired_bootstrap_diff(pairs, stats.rate_diff_statistic, n_iterations=500)
        self.assertEqual(r1.ci_low, r2.ci_low)
        self.assertEqual(r1.ci_high, r2.ci_high)
        self.assertEqual(r1.observed_diff, r2.observed_diff)

    def test_observed_diff_matches_direct_computation(self):
        pairs = [(True, False)] * 5 + [(False, False)] * 5  # a: 5/10=0.5, b: 0/10=0.0
        result = stats.paired_bootstrap_diff(pairs, stats.rate_diff_statistic, n_iterations=200)
        self.assertAlmostEqual(result.observed_diff, 0.5)

    def test_ci_contains_observed_diff_for_identical_arms(self):
        pairs = [(True, True), (False, False)] * 20
        result = stats.paired_bootstrap_diff(pairs, stats.rate_diff_statistic, n_iterations=2000)
        self.assertEqual(result.observed_diff, 0.0)
        self.assertLessEqual(result.ci_low, 0.0)
        self.assertGreaterEqual(result.ci_high, 0.0)

    def test_none_treated_as_false_in_rate_diff(self):
        pairs = [(None, True), (False, True), (False, True)]
        # a: 0/3 True (None -> not True), b: 3/3 True
        diff = stats.rate_diff_statistic(pairs)
        self.assertAlmostEqual(diff, 0.0 - 1.0)

    def test_mean_diff_statistic(self):
        pairs = [(10.0, 5.0), (20.0, 15.0)]
        self.assertAlmostEqual(stats.mean_diff_statistic(pairs), 5.0)

    def test_resamples_whole_pairs_not_arms_independently(self):
        # If arms were resampled independently, a perfectly correlated
        # (identical) pair set could show bootstrap variance in the
        # difference; since diff is always exactly 0 per pair, resampling
        # whole pairs must yield diff==0 in EVERY bootstrap replicate.
        pairs = [(True, True), (False, False), (True, True), (False, False)]
        result = stats.paired_bootstrap_diff(pairs, stats.rate_diff_statistic, n_iterations=500)
        self.assertEqual(result.ci_low, 0.0)
        self.assertEqual(result.ci_high, 0.0)

    def test_zero_pairs_raises(self):
        with self.assertRaises(ValueError):
            stats.paired_bootstrap_diff([], stats.rate_diff_statistic)


class TestMcNemarExact(unittest.TestCase):
    def test_no_discordant_pairs_p_value_one(self):
        pairs = [(True, True), (False, False)] * 10
        result = stats.mcnemar_exact(pairs)
        self.assertEqual(result.discordant_a_only, 0)
        self.assertEqual(result.discordant_b_only, 0)
        self.assertEqual(result.p_value, 1.0)

    def test_symmetric_discordance_p_value_one(self):
        pairs = [(True, False), (False, True)] * 5
        result = stats.mcnemar_exact(pairs)
        self.assertEqual(result.discordant_a_only, 5)
        self.assertEqual(result.discordant_b_only, 5)
        self.assertEqual(result.p_value, 1.0)

    def test_strongly_asymmetric_discordance_small_p_value(self):
        pairs = [(True, False)] * 20 + [(False, True)] * 1
        result = stats.mcnemar_exact(pairs)
        self.assertEqual(result.discordant_a_only, 20)
        self.assertEqual(result.discordant_b_only, 1)
        self.assertLess(result.p_value, 0.05)


class TestPercentile(unittest.TestCase):
    def test_p50_of_sorted_list(self):
        self.assertEqual(stats.percentile([1, 2, 3, 4, 5], 50), 3)

    def test_empty_list_returns_none(self):
        self.assertIsNone(stats.percentile([], 50))

    def test_p100_returns_max(self):
        self.assertEqual(stats.percentile([5, 1, 3], 100), 5)


if __name__ == "__main__":
    unittest.main()
