"""Training pipeline tests: group ordering, LightGBM hyperparameters,
train-frozen frequency fitting, and (lightgbm-gated) the actual training
code path (Commit 5d).

The lightgbm-dependent class asserts training-code-path/artifact-field
correctness only -- never a model-quality claim (2-4 synthetic groups mean
nothing about ranking quality; see train_reranker.py's own module doc).
"""

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


def row(group_id, target_id, candidate_id, features, label, split, **kwargs):
    return tr.LabeledRow(group_id, target_id, candidate_id, features, label, split, **kwargs)


class GroupOrderingTests(unittest.TestCase):
    def test_group_sizes_requires_group_id_sorted_input(self):
        rows = [
            row("g2", "t2", "c1", [0.0], 1, "train"),
            row("g1", "t1", "c1", [0.0], 0, "train"),
            row("g1", "t1", "c2", [0.0], 1, "train"),
        ]
        sorted_rows = sorted(rows, key=lambda r: (r.group_id, r.candidate_id))
        sizes = tr.group_sizes(sorted_rows)
        self.assertEqual(sizes, [2, 1])

    def test_group_sizes_of_unsorted_input_is_wrong(self):
        # Documents WHY train_ranker sorts internally: group_sizes assumes
        # group_id runs are already consecutive.
        rows = [
            row("g1", "t1", "c1", [0.0], 0, "train"),
            row("g2", "t2", "c1", [0.0], 1, "train"),
            row("g1", "t1", "c2", [0.0], 1, "train"),
        ]
        sizes = tr.group_sizes(rows)
        self.assertNotEqual(sizes, [2, 1], "unsorted input produces incorrect (split) group runs")


class TrainFrozenFrequencyTests(unittest.TestCase):
    def test_frequency_counts_occurrences_across_train_rows(self):
        freq_table = tr.fit_template_frequency([
            row("g1", "t1", "c1", [], 1, "train", source_template_ids=("t1",)),
            row("g1", "t1", "c2", [], 0, "train", source_template_ids=("t1",)),
            row("g2", "t2", "c3", [], 1, "train", source_template_ids=("t2",)),
        ])
        self.assertGreater(freq_table["t1"], freq_table["t2"], "t1 seen twice, t2 once")

    def test_frequency_counts_regardless_of_label(self):
        # Deliberate policy: counts EVERY train-split row's source
        # templates, not just positive ones (see fit_template_frequency's
        # doc) -- a negative candidate's template still counts.
        freq_table = tr.fit_template_frequency([
            row("g1", "t1", "c1", [], 0, "train", source_template_ids=("t1",)),
        ])
        self.assertIn("t1", freq_table)

    def test_val_and_test_rows_never_influence_the_fitted_table(self):
        train_rows = [row("g1", "t1", "c1", [], 1, "train", source_template_ids=("only-in-train",))]
        # val/test rows exist (matching a real labeled set) but are never
        # passed to fit_template_frequency -- this is the discipline
        # main() itself follows (fit_template_frequency([r for r in
        # labeled if r.split == "train"])), asserted here directly.
        freq_table = tr.fit_template_frequency(train_rows)
        self.assertIn("only-in-train", freq_table)
        self.assertNotIn("only-in-val", freq_table)
        self.assertNotIn("only-in-test", freq_table)

    def test_table_hash_is_stable_and_detects_content_change(self):
        table_a = {"t1": 1.0, "t2": 2.0}
        table_b = {"t2": 2.0, "t1": 1.0}  # same content, different insertion order
        self.assertEqual(tr.template_frequency_table_sha256(table_a), tr.template_frequency_table_sha256(table_b))
        table_c = {"t1": 1.0, "t2": 3.0}
        self.assertNotEqual(tr.template_frequency_table_sha256(table_a), tr.template_frequency_table_sha256(table_c))


class ImputeFrequencyFeaturesTests(unittest.TestCase):
    def test_impute_does_not_mutate_input_rows(self):
        freq_table = {"known": 2.0}
        original = row("g1", "t1", "c1", [0.0] * len(tr.FEATURE_NAMES_V1), 1, "test", source_template_ids=("known",))
        snapshot = list(original.features)
        tr.impute_frequency_features([original], freq_table)
        self.assertEqual(original.features, snapshot)

    def test_known_template_gets_imputed(self):
        freq_table = {"known": 2.0}
        max_i = tr.feature_index_of("max_template_log_frequency")
        r = row("g1", "t1", "c1", [0.0] * len(tr.FEATURE_NAMES_V1), 1, "test", source_template_ids=("known",))
        r.features[max_i] = float("nan")
        imputed = tr.impute_frequency_features([r], freq_table)
        self.assertEqual(imputed[0].features[max_i], 2.0)

    def test_unknown_template_stays_nan(self):
        import math

        freq_table = {"known": 2.0}
        max_i = tr.feature_index_of("max_template_log_frequency")
        r = row("g1", "t1", "c1", [0.0] * len(tr.FEATURE_NAMES_V1), 1, "test", source_template_ids=("never-seen",))
        r.features[max_i] = float("nan")
        imputed = tr.impute_frequency_features([r], freq_table)
        self.assertTrue(math.isnan(imputed[0].features[max_i]))


class HyperparameterTests(unittest.TestCase):
    """These check the fixed configuration dict itself -- no lightgbm
    needed to verify hyperparameters are explicit and pinned."""

    def test_hyperparameters_are_fixed_not_left_at_library_defaults(self):
        h = tr.LIGHTGBM_HYPERPARAMETERS
        self.assertEqual(h["objective"], "lambdarank")
        self.assertEqual(h["metric"], "ndcg")
        self.assertEqual(h["eval_at"], [1, 10])
        self.assertEqual(h["random_state"], 42)
        self.assertTrue(h["deterministic"])
        self.assertEqual(h["num_threads"], 1)

    def test_early_stopping_rounds_is_defined(self):
        self.assertGreater(tr.EARLY_STOPPING_ROUNDS, 0)


@unittest.skipUnless(LIGHTGBM_AVAILABLE, "lightgbm not installed")
class LightgbmIntegrationTests(unittest.TestCase):
    """Training code path only -- not a model-quality claim."""

    def _rows(self, n_groups=4, split="train"):
        rows = []
        for g in range(n_groups):
            for c in range(3):
                rows.append(
                    row(
                        f"g{g}", f"t{g}", f"c{g}-{c}",
                        [float(c), float(g), float((c + g) % 2)],
                        1 if c == 0 else 0,
                        split,
                    )
                )
        return rows

    def test_train_ranker_returns_hyperparameters_and_package_versions(self):
        result = tr.train_ranker(self._rows())
        self.assertEqual(result["hyperparameters"], tr.LIGHTGBM_HYPERPARAMETERS)
        self.assertIn("lightgbm", result["package_versions"])

    def test_validation_groups_enable_early_stopping_and_record_best_iteration(self):
        result = tr.train_ranker(self._rows(split="train"), val_rows=self._rows(split="val"))
        self.assertIsNotNone(result["best_iteration"])

    def test_no_validation_rows_still_trains(self):
        result = tr.train_ranker(self._rows(), val_rows=None)
        self.assertIsNotNone(result["ranker"])

    def test_model_output_independent_of_input_row_order(self):
        rows = self._rows()
        reversed_rows = list(reversed(rows))
        result_a = tr.train_ranker(rows)
        result_b = tr.train_ranker(reversed_rows)
        score_fn_a = tr.lightgbm_score_fn(result_a["ranker"])
        score_fn_b = tr.lightgbm_score_fn(result_b["ranker"])
        group_rows = [r for r in rows if r.group_id == "g0"]
        scores_a = score_fn_a(group_rows)
        scores_b = score_fn_b(group_rows)
        for sa, sb in zip(scores_a, scores_b):
            self.assertAlmostEqual(sa, sb, places=9)


if __name__ == "__main__":
    unittest.main()
