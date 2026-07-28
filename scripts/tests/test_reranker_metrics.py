"""Conditional/end-to-end metrics tests: top-1, top-10, MRR, NDCG@10, mean
best-positive rank, coverage handling, and scorer-output hard-validation
(Commit 5b)."""

import math
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import train_reranker as tr  # noqa: E402


def row(group_id, target_id, candidate_id, label, split):
    return tr.LabeledRow(group_id, target_id, candidate_id, [0.0], label, split)


class MetricsTests(unittest.TestCase):
    def setUp(self):
        # g1: 4 candidates, single positive ranked 3rd (by descending score).
        # g2: labeled but zero positive in its own pool (a coverage gap).
        self.target_1 = "metrics-t1"
        self.split = tr.split_for_target(self.target_1)
        self.target_2 = next(
            t for t in (f"metrics-t2-{i}" for i in range(10_000)) if tr.split_for_target(t) == self.split
        )
        self.group_records = [
            {"group_id": "g1", "target_id": self.target_1, "target_smiles": self.target_1,
             "candidate_count": 4, "proposal_status": "ok"},
            {"group_id": "g2", "target_id": self.target_2, "target_smiles": self.target_2,
             "candidate_count": 2, "proposal_status": "ok"},
        ]
        self.labels = {
            "g1": tr.GroupLabel(target_id=self.target_1, correct_precursor_sets=frozenset({("pos",)})),
            "g2": tr.GroupLabel(target_id=self.target_2, correct_precursor_sets=frozenset({("nomatch",)})),
        }
        self.g1_rows = [
            row("g1", self.target_1, "c-rank1", 0, self.split),
            row("g1", self.target_1, "c-rank2", 0, self.split),
            row("g1", self.target_1, "c-rank3-positive", 1, self.split),
            row("g1", self.target_1, "c-rank4", 0, self.split),
        ]
        self.g1_scores = {"c-rank1": 4.0, "c-rank2": 3.0, "c-rank3-positive": 2.0, "c-rank4": 1.0}
        self.g2_rows = [
            row("g2", self.target_2, "c2-a", 0, self.split),
            row("g2", self.target_2, "c2-b", 0, self.split),
        ]

    def score_fn(self, rows):
        return [self.g1_scores.get(r.candidate_id, 0.0) for r in rows]

    def evaluate(self):
        return tr.evaluate(self.score_fn, self.g1_rows + self.g2_rows, self.group_records, self.labels, self.split)

    def test_conditional_denominator_excludes_zero_positive_groups(self):
        report = self.evaluate()
        self.assertEqual(report["conditional"]["n_groups"], 1, "only g1 has a positive candidate")

    def test_conditional_top1_hit_rate(self):
        self.assertEqual(self.evaluate()["conditional"]["top1_hit_rate"], 0.0)

    def test_conditional_top10_hit_rate(self):
        self.assertEqual(self.evaluate()["conditional"]["top10_hit_rate"], 1.0)

    def test_conditional_mean_reciprocal_rank(self):
        self.assertEqual(self.evaluate()["conditional"]["mean_reciprocal_rank"], 1.0 / 3.0)

    def test_conditional_mean_best_positive_rank(self):
        self.assertEqual(self.evaluate()["conditional"]["mean_best_positive_rank"], 3)

    def test_conditional_ndcg_at_10(self):
        expected = (1.0 / math.log2(3 + 1)) / (1.0 / math.log2(1 + 1))
        self.assertAlmostEqual(self.evaluate()["conditional"]["ndcg_at_10"], expected)

    def test_end_to_end_denominator_includes_zero_positive_groups(self):
        report = self.evaluate()
        self.assertEqual(report["end_to_end"]["n_groups"], 2, "g2's coverage gap still counts")

    def test_end_to_end_top1_hit_rate(self):
        self.assertEqual(self.evaluate()["end_to_end"]["top1_hit_rate"], 0.0)

    def test_end_to_end_top10_hit_rate_averages_in_the_coverage_miss(self):
        # g1 contributes 1 (hit within top 10), g2 (coverage miss) contributes 0.
        self.assertEqual(self.evaluate()["end_to_end"]["top10_hit_rate"], 0.5)

    def test_end_to_end_mean_reciprocal_rank_averages_in_the_coverage_miss(self):
        self.assertEqual(self.evaluate()["end_to_end"]["mean_reciprocal_rank"], (1.0 / 3.0) / 2)

    def test_end_to_end_ndcg_at_10_averages_in_the_coverage_miss(self):
        expected = ((1.0 / math.log2(3 + 1)) / (1.0 / math.log2(1 + 1))) / 2
        self.assertAlmostEqual(self.evaluate()["end_to_end"]["ndcg_at_10"], expected)

    def test_multiple_positives_in_one_group(self):
        rows = [
            row("g1", self.target_1, "c1", 1, self.split),
            row("g1", self.target_1, "c2", 1, self.split),
            row("g1", self.target_1, "c3", 0, self.split),
        ]

        scores_by_id = {"c1": 3.0, "c2": 2.0, "c3": 1.0}

        def score_fn(rs):
            return [scores_by_id[r.candidate_id] for r in rs]

        report = tr.evaluate(
            score_fn, rows,
            [{"group_id": "g1", "target_id": self.target_1, "target_smiles": self.target_1,
              "candidate_count": 3, "proposal_status": "ok"}],
            {"g1": tr.GroupLabel(target_id=self.target_1, correct_precursor_sets=frozenset({("x",), ("y",)}))},
            self.split,
        )
        self.assertEqual(report["conditional"]["top1_hit_rate"], 1.0)
        self.assertEqual(report["conditional"]["mean_best_positive_rank"], 1)

    def test_exact_score_tie_breaks_by_candidate_id_deterministically(self):
        rows_forward = [
            row("t", "tt", "sha256:bbb", 1, "test"),
            row("t", "tt", "sha256:aaa", 0, "test"),
        ]
        rows_reversed = list(reversed(rows_forward))
        group_records = [{"group_id": "t", "target_id": "tt", "target_smiles": "tt", "candidate_count": 2, "proposal_status": "ok"}]
        labels = {"t": tr.GroupLabel(target_id="tt", correct_precursor_sets=frozenset({("x",)}))}

        def constant_score_fn(rs):
            return [0.0] * len(rs)

        report_forward = tr.evaluate(constant_score_fn, rows_forward, group_records, labels, "test")
        report_reversed = tr.evaluate(constant_score_fn, rows_reversed, group_records, labels, "test")
        self.assertEqual(report_forward, report_reversed, "tied scores must rank identically regardless of input row order")

    def test_score_fn_length_mismatch_is_hard_error(self):
        def bad_score_fn(rows):
            return [0.0] * (len(rows) - 1)

        with self.assertRaises(ValueError):
            tr.evaluate(bad_score_fn, self.g1_rows, self.group_records, self.labels, self.split)

    def test_score_fn_nan_output_is_hard_error(self):
        def nan_score_fn(rows):
            return [float("nan")] * len(rows)

        with self.assertRaises(ValueError):
            tr.evaluate(nan_score_fn, self.g1_rows, self.group_records, self.labels, self.split)

    def test_score_fn_inf_output_is_hard_error(self):
        def inf_score_fn(rows):
            return [float("inf")] * len(rows)

        with self.assertRaises(ValueError):
            tr.evaluate(inf_score_fn, self.g1_rows, self.group_records, self.labels, self.split)


if __name__ == "__main__":
    unittest.main()
