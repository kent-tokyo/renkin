import json
import unittest

from scripts.four_tool_record import FourToolRecord
from scripts.four_tool_report import render_markdown, summarize, wilson_interval


def row(tool, target, strict, elapsed):
    return FourToolRecord(
        target_id=target, target_smiles="CCO", sample_rank=0, tool=tool,
        arm_id=f"{tool}-arm", run_status="completed", route_found=True,
        common_route_parseable=True, strict_route_to_shared_stock=strict,
        common_audit_status="pass" if strict else "fail",
        planning_elapsed_ms=elapsed,
    )


class FourToolReportTests(unittest.TestCase):
    def test_wilson_handles_no_trials(self):
        self.assertIsNone(wilson_interval(0, 0))
        self.assertEqual(len(wilson_interval(1, 1)), 2)

    def test_summary_keeps_unmeasured_out_of_denominator(self):
        records = [row("renkin", "t0", True, 10), row("renkin", "t1", None, 20)]
        payload = summarize(records)
        group = payload["groups"][0]
        self.assertEqual(group["strict_route_to_shared_stock"]["successes"], 1)
        self.assertEqual(group["strict_route_to_shared_stock"]["trials"], 1)
        self.assertEqual(group["planning_elapsed_ms"]["median"], 15)

    def test_groups_are_tool_and_arm_specific(self):
        payload = summarize([row("renkin", "t0", True, 10), row("aizynthfinder", "t0", False, 20)])
        self.assertEqual([(g["tool"], g["arm_id"]) for g in payload["groups"]],
                         [("aizynthfinder", "aizynthfinder-arm"), ("renkin", "renkin-arm")])

    def test_markdown_has_denominators_and_excludes_popularity_metrics(self):
        markdown = render_markdown(summarize([row("renkin", "t0", True, 10)]))
        self.assertIn("1/1", markdown)
        self.assertIn("95% CI", markdown)
        self.assertIn("Popularity metrics: not included", markdown)
        self.assertNotIn("star", markdown.lower())

    def test_paired_comparison_uses_common_evaluable_targets(self):
        payload = summarize([
            row("renkin", "t0", True, 10),
            row("renkin", "t1", False, 10),
            row("aizynthfinder", "t0", False, 10),
            row("aizynthfinder", "t1", False, 10),
        ])
        comparison = next(iter(payload["paired_comparisons"].values()))
        self.assertEqual(comparison["paired_trials"], 2)
        self.assertEqual(comparison["left_wins"], 0)
        self.assertEqual(comparison["right_wins"], 1)
        self.assertEqual(comparison["ties"], 1)
        self.assertEqual(comparison["difference_left_minus_right"], -0.5)
        self.assertEqual(len(comparison["paired_bootstrap_95"]), 2)
        self.assertEqual(comparison["mcnemar_exact_two_sided_p"], 1.0)


if __name__ == "__main__":
    unittest.main()
