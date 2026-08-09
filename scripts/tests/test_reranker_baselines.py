"""Baseline arm A-G tests: correct ranking direction, not-computable
detection, identical candidate groups across arms, and row-order
independence (Commit 5c)."""

import math
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import train_reranker as tr  # noqa: E402

try:
    import lightgbm  # noqa: F401

    LIGHTGBM_AVAILABLE = True
except ImportError:
    LIGHTGBM_AVAILABLE = False


def arm_features(**overrides):
    base = {
        "num_precursors": 2.0, "target_heavy_atom_count": 10.0,
        "precursor_heavy_atom_count_sum": 8.0, "precursor_heavy_atom_count_max": 5.0,
        "heavy_atom_retention_ratio": 1.25, "net_charge_balanced": 1.0,
        "no_heavy_atom_gain": 1.0, "source_template_count": 1.0,
        "reaction_center_atom_count_min": 2.0, "reaction_center_atom_count_max": 2.0,
        "reaction_center_atom_count_mean": 2.0, "reaction_center_extractable_fraction": 1.0,
        "min_base_step_cost": 1.5, "best_upstream_score": 0.9,
        "fraction_precursors_in_stock": 1.0, "all_precursors_in_stock": 1.0,
        "max_template_log_frequency": float("nan"), "mean_template_log_frequency": float("nan"),
    }
    base.update(overrides)
    return [base[name] for name in tr.FEATURE_NAMES_V1]


class BaselineArmDirectionTests(unittest.TestCase):
    """Each arm must rank a chemically/structurally "better" candidate
    above a "worse" one -- verified per arm, on a fixed two-row fixture.
    """

    def setUp(self):
        self.freq_table = tr.fit_template_frequency([
            tr.LabeledRow("g0", "t0", "f1", [], 1, "train", source_template_ids=("t1",)),
            tr.LabeledRow("g0", "t0", "f2", [], 1, "train", source_template_ids=("t1",)),
            tr.LabeledRow("g0", "t0", "f3", [], 1, "train", source_template_ids=("t2",)),
        ])
        self.strong = tr.LabeledRow(
            "g1", "arm-t1", "strong", arm_features(), 1, "test",
            best_upstream_rank=0, source_template_ids=("t1",),
        )
        self.weak = tr.LabeledRow(
            "g1", "arm-t1", "weak",
            arm_features(
                num_precursors=3.0, net_charge_balanced=0.0, reaction_center_atom_count_mean=3.0,
                best_upstream_score=0.3, fraction_precursors_in_stock=0.5, all_precursors_in_stock=0.0,
            ),
            0, "test", best_upstream_rank=1, source_template_ids=("t2",),
        )
        self.arms_by_name = {a["name"]: a for a in tr.build_baseline_arms(self.freq_table)}

    def test_all_arms_present(self):
        expected = {
            "original_rank", "upstream_score", "template_frequency", "upstream_plus_frequency",
            "structural", "reaction_center", "availability",
        }
        self.assertEqual(set(self.arms_by_name), expected)

    def test_every_arm_ranks_the_strong_candidate_above_the_weak_one(self):
        for name, arm in self.arms_by_name.items():
            with self.subTest(arm=name):
                scores = arm["score_fn"]([self.strong, self.weak])
                self.assertEqual(len(scores), 2)
                self.assertTrue(all(math.isfinite(s) for s in scores))
                self.assertGreater(scores[0], scores[1])

    def test_every_arm_is_computable_when_the_relevant_signal_is_present(self):
        for name, arm in self.arms_by_name.items():
            with self.subTest(arm=name):
                self.assertTrue(arm["computable_fn"]([self.strong, self.weak]))

    def test_upstream_score_not_computable_without_a_scorer(self):
        no_scorer = tr.LabeledRow(
            "g1", "arm-t1", "x", arm_features(best_upstream_score=float("nan")), 0, "test",
        )
        self.assertFalse(self.arms_by_name["upstream_score"]["computable_fn"]([no_scorer]))

    def test_upstream_plus_frequency_still_computable_if_only_frequency_is(self):
        no_scorer = tr.LabeledRow(
            "g1", "arm-t1", "x", arm_features(best_upstream_score=float("nan")), 0, "test",
            source_template_ids=("t1",),
        )
        self.assertTrue(self.arms_by_name["upstream_plus_frequency"]["computable_fn"]([no_scorer]))

    def test_availability_not_computable_without_stock(self):
        no_stock = tr.LabeledRow(
            "g1", "arm-t1", "x", arm_features(fraction_precursors_in_stock=float("nan")), 0, "test",
        )
        self.assertFalse(self.arms_by_name["availability"]["computable_fn"]([no_stock]))

    def test_reaction_center_not_computable_when_nothing_extractable(self):
        not_extractable = tr.LabeledRow(
            "g1", "arm-t1", "x", arm_features(reaction_center_extractable_fraction=0.0), 0, "test",
        )
        self.assertFalse(self.arms_by_name["reaction_center"]["computable_fn"]([not_extractable]))

    def test_missing_feature_policy_uses_a_finite_sentinel_never_nan_or_inf(self):
        # A row missing the arm's relevant feature must still produce a
        # finite score (ranked last via the sentinel), never NaN/Inf --
        # evaluate() hard-rejects non-finite scores (see test_reranker_metrics.py).
        missing_upstream = tr.LabeledRow(
            "g1", "arm-t1", "missing", arm_features(best_upstream_score=float("nan")), 0, "test",
        )
        present_upstream = tr.LabeledRow(
            "g1", "arm-t1", "present", arm_features(best_upstream_score=0.1), 0, "test",
        )
        scores = self.arms_by_name["upstream_score"]["score_fn"]([missing_upstream, present_upstream])
        self.assertTrue(all(math.isfinite(s) for s in scores))
        self.assertLess(scores[0], scores[1], "a missing value must rank behind any present value")


class RunBaselineArmsTests(unittest.TestCase):
    def setUp(self):
        self.freq_table = {}
        self.group_records = [
            {"group_id": "g1", "target_id": "arm-t1", "target_smiles": "arm-t1", "candidate_count": 2, "proposal_status": "ok"},
        ]
        self.labels = {"g1": tr.GroupLabel(target_id="arm-t1", correct_precursor_sets=frozenset({("dummy",)}))}
        self.split = tr.split_for_target("arm-t1")
        self.rows = [
            tr.LabeledRow("g1", "arm-t1", "strong", arm_features(), 1, self.split),
            tr.LabeledRow("g1", "arm-t1", "weak", arm_features(best_upstream_score=0.1), 0, self.split),
        ]

    def test_all_arms_score_the_identical_candidate_groups(self):
        reports = tr.run_baseline_arms(self.freq_table, self.rows, self.group_records, self.labels, self.split)
        ok_reports = {name: r for name, r in reports.items() if r["status"] == "ok"}
        self.assertGreater(len(ok_reports), 0)
        group_counts = {name: r["group_count"] for name, r in ok_reports.items()}
        self.assertEqual(len(set(group_counts.values())), 1, "every computable arm must see the same group_count")

    def test_not_computable_arms_report_status_without_numeric_fields(self):
        no_scorer_rows = [
            tr.LabeledRow("g1", "arm-t1", "strong", arm_features(best_upstream_score=float("nan")), 1, self.split),
            tr.LabeledRow("g1", "arm-t1", "weak", arm_features(best_upstream_score=float("nan")), 0, self.split),
        ]
        reports = tr.run_baseline_arms(self.freq_table, no_scorer_rows, self.group_records, self.labels, self.split)
        self.assertEqual(reports["upstream_score"]["status"], "not_computable")
        self.assertNotIn("top1_hit_rate", reports["upstream_score"])

    def test_reversing_row_order_gives_identical_metrics(self):
        forward = tr.run_baseline_arms(self.freq_table, self.rows, self.group_records, self.labels, self.split)
        reversed_reports = tr.run_baseline_arms(
            self.freq_table, list(reversed(self.rows)), self.group_records, self.labels, self.split
        )
        self.assertEqual(forward, reversed_reports)


@unittest.skipUnless(LIGHTGBM_AVAILABLE, "lightgbm not installed")
class FullConfiguredArmIntegrationTests(unittest.TestCase):
    """Arm H (the trained model) needs lightgbm -- kept in its own
    skipUnless-gated class so the rest of this file runs with no
    dependency and no model-quality claim (see this file's module doc and
    train_reranker.py's own)."""

    def test_full_configured_model_is_evaluable_via_the_same_evaluate_path(self):
        freq_table = tr.fit_template_frequency([
            tr.LabeledRow("g0", "t0", "f1", [0.0] * len(tr.FEATURE_NAMES_V1), 1, "train", source_template_ids=("t1",)),
        ])
        split = tr.split_for_target("arm-h-t1")
        rows = [
            tr.LabeledRow("g1", "arm-h-t1", "c1", arm_features(), 1, split),
            tr.LabeledRow("g1", "arm-h-t1", "c2", arm_features(best_upstream_score=0.1), 0, split),
        ]
        imputed = tr.impute_frequency_features(rows, freq_table)
        train_result = tr.train_ranker(imputed)
        score_fn = tr.lightgbm_score_fn(train_result["ranker"])
        group_records = [
            {"group_id": "g1", "target_id": "arm-h-t1", "target_smiles": "arm-h-t1", "candidate_count": 2, "proposal_status": "ok"},
        ]
        labels = {"g1": tr.GroupLabel(target_id="arm-h-t1", correct_precursor_sets=frozenset({("dummy",)}))}
        report = tr.evaluate(score_fn, imputed, group_records, labels, split)
        self.assertIn("conditional", report)
        self.assertIn("end_to_end", report)


if __name__ == "__main__":
    unittest.main()
