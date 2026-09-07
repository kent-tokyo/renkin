import os
import sys
import unittest
from dataclasses import replace

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from compare_policy_ab import compute_report, join_arms, _repeat_determinism  # noqa: E402
from compare_schema import PlannerComparisonRow  # noqa: E402


def row(target_id, **kwargs):
    base = PlannerComparisonRow(
        target_id=target_id,
        target_smiles="CCO",
        sample_rank=0,
        tool="renkin",
        tool_version="1.0.3",
        configuration_id="legacy",
        comparison_mode="native",
        run_status="completed",
        route_found=False,
        total_elapsed_ms=100.0,
    )
    return replace(base, **kwargs)


class TestPolicyAb(unittest.TestCase):
    def test_separates_route_and_strict_axes(self):
        left = [
            row("a", route_found=True, validator_confirmed_route_found=False),
            row("b", route_found=False, total_elapsed_ms=200.0),
        ]
        right = [
            row("a", route_found=True, validator_confirmed_route_found=True),
            row("b", route_found=True, validator_confirmed_route_found=True),
        ]
        report = compute_report(join_arms(left, right), "legacy", "policy")
        route = report["axes"]["route_found"]
        strict = report["axes"]["strict_validator_confirmed_route_found"]
        self.assertEqual((route["left_true"], route["right_true"]), (1, 2))
        self.assertEqual((strict["left_true"], strict["right_true"]), (0, 2))
        self.assertEqual((strict["left_only"], strict["right_only"]), (0, 2))

    def test_rejects_duplicate_target_ids(self):
        with self.assertRaisesRegex(ValueError, "duplicate"):
            join_arms([row("a"), row("a")], [row("a")])

    def test_rejects_smiles_mismatch(self):
        with self.assertRaisesRegex(ValueError, "target_smiles"):
            join_arms([row("a")], [row("a", target_smiles="CCC")])

    def test_latency_uses_only_paired_measurements(self):
        report = compute_report(
            join_arms(
                [row("a", total_elapsed_ms=10.0), row("b", total_elapsed_ms=None)],
                [row("a", total_elapsed_ms=30.0), row("b", total_elapsed_ms=40.0)],
            ),
            "legacy",
            "policy",
        )
        latency = report["axes"]["latency"]
        self.assertEqual(latency["n_pairs"], 1)
        self.assertEqual(latency["paired_mean_delta_left_minus_right_ms"]["observed"], -20.0)

    def test_memory_is_a_separate_paired_axis(self):
        report = compute_report(
            join_arms(
                [row("a", peak_rss_bytes=100), row("b", peak_rss_bytes=None)],
                [row("a", peak_rss_bytes=250), row("b", peak_rss_bytes=300)],
            ),
            "legacy",
            "policy",
        )
        memory = report["axes"]["memory"]
        self.assertEqual(memory["n_pairs"], 1)
        self.assertEqual(memory["paired_mean_delta_left_minus_right_bytes"]["observed"], -150)

    def test_repeat_determinism_is_not_arm_agreement(self):
        first = [row("a", route_found=True, normalized_route_sha256="sha256:x")]
        repeat = [row("a", route_found=True, normalized_route_sha256="sha256:x")]
        result = _repeat_determinism(first, repeat)
        self.assertEqual(result["outcome_equal_rate"], 1.0)
        self.assertEqual(result["route_hash_equal_rate"], 1.0)


if __name__ == "__main__":
    unittest.main()
