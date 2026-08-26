import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import compare_aggregate as agg  # noqa: E402
from compare_schema import PlannerComparisonRow  # noqa: E402


def make_row(**kwargs):
    defaults = dict(
        target_id="t",
        target_smiles="CCO",
        sample_rank=0,
        tool="renkin",
        tool_version="1.0",
        configuration_id="cfg1",
        comparison_mode="native",
        run_status="completed",
    )
    defaults.update(kwargs)
    return PlannerComparisonRow(**defaults)


class TestComputeAggregate(unittest.TestCase):
    def test_route_found_rate_denominator_is_all_sampled_including_timeouts(self):
        rows = [
            make_row(route_found=True, total_elapsed_ms=10.0),
            make_row(route_found=False, total_elapsed_ms=10.0),
            make_row(run_status="timeout", total_elapsed_ms=200.0),
        ]
        result = agg.compute_aggregate(rows)
        self.assertEqual(result["n_all_sampled"], 3)
        self.assertEqual(result["route_found_rate"]["n_numerator"], 1)
        self.assertEqual(result["route_found_rate"]["n_denominator"], 3)
        self.assertAlmostEqual(result["route_found_rate"]["value"], 1 / 3)

    def test_route_tree_parseable_rate_denominator_is_route_found_runs_only(self):
        rows = [
            make_row(route_found=True, route_tree_parseable=True, total_elapsed_ms=1.0),
            make_row(route_found=True, route_tree_parseable=False, total_elapsed_ms=1.0),
            make_row(route_found=False, total_elapsed_ms=1.0),  # excluded from this denominator
        ]
        result = agg.compute_aggregate(rows)
        self.assertEqual(result["route_tree_parseable_rate"]["n_denominator"], 2)
        self.assertEqual(result["route_tree_parseable_rate"]["n_numerator"], 1)

    def test_route_found_but_tree_not_parseable_is_a_visible_headline_count(self):
        rows = [
            make_row(route_found=True, route_tree_parseable=False, total_elapsed_ms=1.0),
            make_row(route_found=True, route_tree_parseable=True, total_elapsed_ms=1.0),
        ]
        result = agg.compute_aggregate(rows)
        self.assertEqual(result["route_found_but_tree_not_parseable_count"], 1)

    def test_empty_rows_all_denominators_zero_and_values_none(self):
        result = agg.compute_aggregate([])
        self.assertEqual(result["n_all_sampled"], 0)
        self.assertIsNone(result["route_found_rate"]["value"])

    def test_solved_only_latency_excludes_unsolved_targets(self):
        rows = [
            make_row(route_found=True, total_elapsed_ms=100.0),
            make_row(route_found=False, total_elapsed_ms=5000.0),  # would skew mean if included
        ]
        result = agg.compute_aggregate(rows)
        self.assertEqual(result["solved_only_total_elapsed_ms_percentiles"]["n"], 1)
        self.assertEqual(result["solved_only_total_elapsed_ms_percentiles"]["p50"], 100.0)

    def test_setup_error_rows_excluded_from_measured_runs_latency(self):
        rows = [
            make_row(run_status="setup_error", total_elapsed_ms=None, route_found=None),
            make_row(route_found=True, total_elapsed_ms=50.0),
        ]
        result = agg.compute_aggregate(rows)
        self.assertEqual(result["total_elapsed_ms_percentiles"]["n"], 1)

    def test_validator_confirmed_route_found_rate_denominator_is_all_sampled(self):
        rows = [
            make_row(route_found=True, validator_confirmed_route_found=True, total_elapsed_ms=1.0),
            make_row(route_found=True, validator_confirmed_route_found=False, total_elapsed_ms=1.0),
            make_row(route_found=False, total_elapsed_ms=1.0),
        ]
        result = agg.compute_aggregate(rows)
        self.assertEqual(result["validator_confirmed_route_found_rate"]["n_numerator"], 1)
        self.assertEqual(result["validator_confirmed_route_found_rate"]["n_denominator"], 3)

    def test_not_evaluable_rate(self):
        rows = [
            make_row(route_found=True, not_evaluable=True, total_elapsed_ms=1.0),
            make_row(route_found=True, not_evaluable=False, total_elapsed_ms=1.0),
        ]
        result = agg.compute_aggregate(rows)
        self.assertEqual(result["not_evaluable_rate"]["n_numerator"], 1)
        self.assertEqual(result["not_evaluable_rate"]["n_denominator"], 2)

    def test_gated_out_summary_omitted_when_no_row_ran_gated(self):
        rows = [make_row(route_found=True, total_elapsed_ms=1.0)]
        result = agg.compute_aggregate(rows)
        self.assertNotIn("gated_out_candidates_total", result)

    def test_gated_out_summary_present_and_merged_when_gated(self):
        rows = [
            make_row(
                route_found=True,
                total_elapsed_ms=1.0,
                gated_out_candidate_count=2,
                gated_out_reasons={"n_benzylation_retro": 2},
            ),
            make_row(
                route_found=False,
                total_elapsed_ms=1.0,
                gated_out_candidate_count=0,
                gated_out_reasons={},
            ),
        ]
        result = agg.compute_aggregate(rows)
        self.assertEqual(result["gated_out_candidates_total"], 2)
        self.assertEqual(result["gated_out_candidates_mean_per_target"], 1.0)
        self.assertEqual(result["gated_out_reasons_total"], {"n_benzylation_retro": 2})


if __name__ == "__main__":
    unittest.main()
