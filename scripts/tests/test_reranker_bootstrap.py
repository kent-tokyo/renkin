"""Paired bootstrap and offline-gate tests: determinism, target_id cluster
resampling, CI, and PASS/FAIL boundary judging (Commit 5e).

Explicitly not run against any real/formal data -- see
train_reranker.py's own module doc and the repo's staged candidate-pool
gate.
"""

import os
import random
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import train_reranker as tr  # noqa: E402


def metric(top1, mrr, top10):
    return {
        "has_positive": True, "top1_hit": top1, "top10_hit": top10,
        "reciprocal_rank": mrr, "ndcg10": 0.0, "best_positive_rank": 1,
    }


class PairedBootstrapTests(unittest.TestCase):
    def setUp(self):
        # target_a2 hosts TWO groups -- must always resample together.
        self.target_to_groups = {"target_a1": ["g1"], "target_a2": ["g2", "g3"], "target_a3": ["g4"]}
        self.baseline_metrics = {g: metric(0, 0.0, 0) for g in ("g1", "g2", "g3", "g4")}
        self.treatment_metrics = {g: metric(1, 1.0, 1) for g in ("g1", "g2", "g3", "g4")}

    def test_same_seed_produces_identical_result(self):
        a = tr.paired_bootstrap(self.baseline_metrics, self.treatment_metrics, self.target_to_groups, n_resamples=200, seed=99)
        b = tr.paired_bootstrap(self.baseline_metrics, self.treatment_metrics, self.target_to_groups, n_resamples=200, seed=99)
        self.assertEqual(a, b)

    def test_different_seed_can_produce_a_different_result(self):
        # Only g1 (target_a1) improves -- how many times target_a1 happens
        # to be drawn varies by seed, so the resampled mean delta should
        # vary too (unlike a uniform-everywhere improvement, which would
        # give the same delta regardless of composition).
        mixed_baseline = {g: metric(0, 0.0, 0) for g in ("g1", "g2", "g3", "g4")}
        mixed_treatment = {**mixed_baseline, "g1": metric(1, 1.0, 1)}
        results = {
            seed: tr.paired_bootstrap(mixed_baseline, mixed_treatment, self.target_to_groups, n_resamples=50, seed=seed)["deltas"]["top1_hit_rate"]["mean_delta"]
            for seed in (1, 2, 3, 4, 5)
        }
        self.assertGreater(len(set(results.values())), 1, "different seeds should not all coincidentally agree")

    def test_uniform_improvement_gives_a_degenerate_ci_at_the_true_delta(self):
        result = tr.paired_bootstrap(self.baseline_metrics, self.treatment_metrics, self.target_to_groups, n_resamples=200, seed=1)
        self.assertEqual(result["deltas"]["top1_hit_rate"]["mean_delta"], 1.0)
        self.assertEqual(result["deltas"]["top1_hit_rate"]["ci_95"], [1.0, 1.0])

    def test_no_improvement_gives_zero_delta(self):
        result = tr.paired_bootstrap(self.baseline_metrics, self.baseline_metrics, self.target_to_groups, n_resamples=50, seed=1)
        self.assertEqual(result["deltas"]["top1_hit_rate"]["mean_delta"], 0.0)
        self.assertEqual(result["deltas"]["mean_reciprocal_rank"]["mean_delta"], 0.0)
        self.assertEqual(result["deltas"]["top10_hit_rate"]["mean_delta"], 0.0)

    def test_resample_unit_and_seed_are_recorded(self):
        result = tr.paired_bootstrap(self.baseline_metrics, self.treatment_metrics, self.target_to_groups, n_resamples=10, seed=42)
        self.assertEqual(result["seed"], 42)
        self.assertEqual(result["n_resamples"], 10)
        self.assertIn("target_id", result["resample_unit"])

    def test_cluster_resampling_matches_hand_replicated_algorithm(self):
        # Replicate the documented algorithm (sorted target_ids,
        # random.Random(seed), n draws with replacement, whole
        # target_to_groups[t] extended per draw) by hand for a single
        # resample, and confirm the result matches exactly -- this directly
        # exercises that target_a2's two groups move together (never g2
        # without g3).
        mixed_baseline = {"g1": metric(0, 0.0, 0), "g2": metric(0, 0.0, 0), "g3": metric(1, 1.0, 1), "g4": metric(0, 0.0, 0)}
        mixed_treatment = dict(mixed_baseline)
        target_ids_sorted = sorted(self.target_to_groups)
        n = len(target_ids_sorted)
        rng = random.Random(4242)
        expected_targets = [target_ids_sorted[rng.randrange(n)] for _ in range(n)]
        expected_groups = []
        for t in expected_targets:
            expected_groups.extend(self.target_to_groups[t])
        expected_delta = tr._mean(
            [float(mixed_treatment[g]["top1_hit"]) for g in expected_groups]
        ) - tr._mean([float(mixed_baseline[g]["top1_hit"]) for g in expected_groups])

        result = tr.paired_bootstrap(mixed_baseline, mixed_treatment, self.target_to_groups, n_resamples=1, seed=4242)
        self.assertEqual(result["deltas"]["top1_hit_rate"]["mean_delta"], expected_delta)

    def test_empty_target_set_produces_no_resamples(self):
        result = tr.paired_bootstrap({}, {}, {}, n_resamples=10, seed=1)
        self.assertEqual(result["n_target_ids"], 0)
        self.assertIsNone(result["deltas"]["top1_hit_rate"]["mean_delta"])

    def test_single_target_collapses_ci_to_the_observed_delta(self):
        one_target = {"target_only": ["g1"]}
        baseline = {"g1": metric(0, 0.0, 0)}
        treatment = {"g1": metric(1, 1.0, 1)}
        result = tr.paired_bootstrap(baseline, treatment, one_target, n_resamples=50, seed=1)
        self.assertEqual(result["deltas"]["top1_hit_rate"]["mean_delta"], 1.0)
        self.assertEqual(result["deltas"]["top1_hit_rate"]["ci_95"], [1.0, 1.0])


class OfflineGateTests(unittest.TestCase):
    def setUp(self):
        self.target_to_groups = {"target_a1": ["g1"], "target_a2": ["g2", "g3"], "target_a3": ["g4"]}
        self.baseline_metrics = {g: metric(0, 0.0, 0) for g in ("g1", "g2", "g3", "g4")}
        self.treatment_metrics = {g: metric(1, 1.0, 1) for g in ("g1", "g2", "g3", "g4")}
        self.big_improvement = tr.paired_bootstrap(
            self.baseline_metrics, self.treatment_metrics, self.target_to_groups, n_resamples=200, seed=1
        )
        self.no_improvement = tr.paired_bootstrap(
            self.baseline_metrics, self.baseline_metrics, self.target_to_groups, n_resamples=50, seed=1
        )

    def test_pass_when_every_criterion_is_met(self):
        result = tr.evaluate_offline_gate(self.big_improvement, coverage_identical=True, baseline_arm="a", treatment_arm="b")
        self.assertEqual(result["result"], "PASS")
        self.assertTrue(all(result["checks"].values()))

    def test_fail_when_coverage_changed_regardless_of_deltas(self):
        result = tr.evaluate_offline_gate(self.big_improvement, coverage_identical=False, baseline_arm="a", treatment_arm="b")
        self.assertEqual(result["result"], "FAIL")
        self.assertFalse(result["checks"]["coverage_unchanged"])

    def test_fail_when_no_improvement(self):
        result = tr.evaluate_offline_gate(self.no_improvement, coverage_identical=True, baseline_arm="a", treatment_arm="a")
        self.assertEqual(result["result"], "FAIL")
        self.assertFalse(result["checks"]["top1_hit_rate_delta_meets_threshold"])
        self.assertFalse(result["checks"]["top1_hit_rate_ci_lower_bound_positive"])

    def test_top10_regression_beyond_threshold_fails(self):
        baseline_full_top10 = {g: metric(1, 1.0, 1) for g in ("g1", "g2", "g3", "g4")}
        treatment_with_top10_regression = {g: metric(1, 1.0, 0) for g in ("g1", "g2", "g3", "g4")}
        bootstrap = tr.paired_bootstrap(
            baseline_full_top10, treatment_with_top10_regression, self.target_to_groups, n_resamples=50, seed=1
        )
        result = tr.evaluate_offline_gate(bootstrap, coverage_identical=True, baseline_arm="a", treatment_arm="b")
        self.assertFalse(result["checks"]["top10_hit_rate_regression_within_threshold"])
        self.assertEqual(result["result"], "FAIL")

    def test_baseline_and_treatment_arm_names_are_echoed(self):
        result = tr.evaluate_offline_gate(self.big_improvement, coverage_identical=True, baseline_arm="original_rank", treatment_arm="full_configured_model")
        self.assertEqual(result["baseline_arm"], "original_rank")
        self.assertEqual(result["treatment_arm"], "full_configured_model")

    def test_thresholds_are_recorded_in_the_result(self):
        result = tr.evaluate_offline_gate(self.big_improvement, coverage_identical=True, baseline_arm="a", treatment_arm="b")
        self.assertEqual(result["thresholds"], tr.GATE_THRESHOLDS)


class RunOfflineGateIntegrationTests(unittest.TestCase):
    """run_offline_gate's own coverage-identical check: since both arms
    are always scored over the SAME rows/split, this is trivially satisfied
    by construction (see evaluate_offline_gate's doc) -- this test confirms
    that plumbing, not the (separately, directly tested above) FAIL path.
    """

    def test_run_offline_gate_produces_a_pass_or_fail_report(self):
        split = tr.split_for_target("gate-t1")
        group_records = [
            {"group_id": "g1", "target_id": "gate-t1", "target_smiles": "gate-t1", "candidate_count": 2, "proposal_status": "ok"},
        ]
        labels = {"g1": tr.GroupLabel(target_id="gate-t1", correct_precursor_sets=frozenset({("x",)}))}
        rows = [
            tr.LabeledRow("g1", "gate-t1", "c1", [0.0], 1, split, best_upstream_rank=0),
            tr.LabeledRow("g1", "gate-t1", "c2", [0.0], 0, split, best_upstream_rank=1),
        ]

        def baseline_score_fn(rs):
            return [-float(r.best_upstream_rank) for r in rs]

        def treatment_score_fn(rs):
            return [1.0 if r.candidate_id == "c1" else 0.0 for r in rs]

        result = tr.run_offline_gate(
            baseline_score_fn, treatment_score_fn, rows, group_records, labels, split,
            baseline_arm="a", treatment_arm="b", n_resamples=10, seed=1,
        )
        self.assertIn(result["result"], ("PASS", "FAIL"))
        self.assertTrue(result["checks"]["coverage_unchanged"])


if __name__ == "__main__":
    unittest.main()
