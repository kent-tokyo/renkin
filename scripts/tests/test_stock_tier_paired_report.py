import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import stock_tier_paired_report as report  # noqa: E402
from compare_schema import PlannerComparisonRow  # noqa: E402


def _row(
    target_id,
    route_found,
    run_status="completed",
    total_elapsed_ms=1000.0,
    validator_confirmed_route_found=None,
    not_evaluable=False,
    peak_rss_bytes=None,
    gated_out_candidate_count=None,
):
    return PlannerComparisonRow(
        target_id=target_id,
        target_smiles="CC",
        sample_rank=0,
        tool="renkin",
        tool_version="0.0.0",
        configuration_id="renkin-native-test",
        comparison_mode="native",
        run_status=run_status,
        route_found=route_found,
        total_elapsed_ms=total_elapsed_ms,
        validator_confirmed_route_found=validator_confirmed_route_found,
        not_evaluable=not_evaluable,
        peak_rss_bytes=peak_rss_bytes,
        gated_out_candidate_count=gated_out_candidate_count,
    )


class TestJoinRows(unittest.TestCase):
    def test_joins_and_sorts_by_target_id(self):
        baseline = [_row("t2", True), _row("t1", False)]
        candidate = [_row("t1", True), _row("t2", False)]
        joined = report.join_rows(baseline, candidate)
        self.assertEqual([tid for tid, _, _ in joined], ["t1", "t2"])

    def test_mismatched_target_id_sets_raises(self):
        baseline = [_row("t1", True)]
        candidate = [_row("t2", True)]
        with self.assertRaises(ValueError):
            report.join_rows(baseline, candidate)


class TestBuildSummary(unittest.TestCase):
    def test_regression_and_new_solve_detection(self):
        joined = [
            ("t1", _row("t1", True), _row("t1", True)),  # unchanged solved
            ("t2", _row("t2", False), _row("t2", False)),  # unchanged unsolved
            ("t3", _row("t3", True), _row("t3", False)),  # regression
            ("t4", _row("t4", False), _row("t4", True)),  # new solve
        ]
        summary = report.build_summary(joined)
        self.assertEqual(summary["regression_count"], 1)
        self.assertEqual(summary["regressions"], ["t3"])
        self.assertEqual(summary["new_solve_count"], 1)
        self.assertEqual(summary["new_solves"], ["t4"])
        self.assertEqual(summary["baseline_route_found_rate"]["n_numerator"], 2)
        self.assertEqual(summary["candidate_route_found_rate"]["n_numerator"], 2)

    def test_validator_confirmed_rate_separate_from_route_found(self):
        # route_found=True but validator_confirmed_route_found=False must
        # NOT count toward the validator-confirmed rate -- the two axes
        # must never be conflated.
        joined = [
            ("t1", _row("t1", True, validator_confirmed_route_found=True), _row("t1", True, validator_confirmed_route_found=True)),
            ("t2", _row("t2", True, validator_confirmed_route_found=False), _row("t2", True, validator_confirmed_route_found=False)),
        ]
        summary = report.build_summary(joined)
        self.assertEqual(summary["baseline_route_found_rate"]["n_numerator"], 2)
        self.assertEqual(summary["baseline_validator_confirmed_rate"]["n_numerator"], 1)

    def test_timeout_and_not_evaluable_counts(self):
        joined = [
            ("t1", _row("t1", None, run_status="timeout"), _row("t1", True)),
            ("t2", _row("t2", True, not_evaluable=False), _row("t2", None, not_evaluable=True)),
        ]
        summary = report.build_summary(joined)
        self.assertEqual(summary["baseline_timeout_count"], 1)
        self.assertEqual(summary["candidate_timeout_count"], 0)
        self.assertEqual(summary["candidate_not_evaluable_count"], 1)

    def test_latency_paired_deltas_only_both_completed(self):
        joined = [
            ("t1", _row("t1", True, total_elapsed_ms=10.0), _row("t1", True, total_elapsed_ms=8.0)),
            ("t2", _row("t2", None, run_status="timeout", total_elapsed_ms=150.0), _row("t2", True, total_elapsed_ms=5.0)),
        ]
        summary = report.build_summary(joined)
        deltas = summary["latency_paired_deltas_ms"]
        self.assertEqual(deltas["n_both_completed"], 1)  # t2 excluded, baseline timed out
        self.assertAlmostEqual(deltas["sum_candidate_minus_baseline"], -2.0)

    def test_gated_out_candidate_count_sums_and_treats_none_as_zero(self):
        joined = [
            ("t1", _row("t1", True, gated_out_candidate_count=3), _row("t1", True, gated_out_candidate_count=5)),
            ("t2", _row("t2", True, gated_out_candidate_count=None), _row("t2", True, gated_out_candidate_count=2)),
        ]
        summary = report.build_summary(joined)
        self.assertEqual(summary["baseline_gated_out_candidate_count_total"], 3)
        self.assertEqual(summary["candidate_gated_out_candidate_count_total"], 7)


if __name__ == "__main__":
    unittest.main()
