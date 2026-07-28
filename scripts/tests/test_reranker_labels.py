"""Labels schema v1 and label/split assignment tests (Commit 1)."""

import json
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import train_reranker as tr  # noqa: E402


def write_jsonl_lines(path, rows):
    with open(path, "w", encoding="utf-8") as f:
        for row in rows:
            f.write(json.dumps(row) + "\n")


class LoadLabelsTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()

    def tearDown(self):
        self.tmp.cleanup()

    def path(self, name):
        return os.path.join(self.tmp.name, name)

    def test_multiple_correct_precursor_sets(self):
        p = self.path("labels.jsonl")
        write_jsonl_lines(p, [
            {"schema_version": 1, "group_id": "rxn-multi", "target_id": "target_multi",
             "correct_precursor_sets": [["CC(=O)O", "CCO"], ["CCO", "CCl"]]},
        ])
        labels = tr.load_labels(p)
        self.assertIn(("CC(=O)O", "CCO"), labels["rxn-multi"].correct_precursor_sets)
        self.assertIn(("CCO", "CCl"), labels["rxn-multi"].correct_precursor_sets)

    def test_precursor_multiplicity_is_preserved_not_deduplicated(self):
        # A symmetric split can legitimately produce the SAME precursor
        # twice (e.g. bond-breaking down the middle of a symmetric
        # molecule) -- ["CO", "CO"] and ["CO"] must be treated as different
        # answer sets, not silently collapsed to a set that loses the count.
        p = self.path("labels.jsonl")
        write_jsonl_lines(p, [
            {"schema_version": 1, "group_id": "rxn-sym", "target_id": "target_sym",
             "correct_precursor_sets": [["CO", "CO"]]},
        ])
        labels = tr.load_labels(p)
        self.assertIn(("CO", "CO"), labels["rxn-sym"].correct_precursor_sets)
        self.assertNotIn(("CO",), labels["rxn-sym"].correct_precursor_sets)

    def test_identical_duplicate_group_id_tolerated(self):
        p = self.path("labels.jsonl")
        write_jsonl_lines(p, [
            {"schema_version": 1, "group_id": "rxn-dup", "target_id": "target_dup",
             "correct_precursor_sets": [["CC(=O)O", "CCO"]]},
            {"schema_version": 1, "group_id": "rxn-dup", "target_id": "target_dup",
             "correct_precursor_sets": [["CC(=O)O", "CCO"]]},
        ])
        labels = tr.load_labels(p)
        self.assertEqual(len(labels), 1)

    def test_conflicting_duplicate_group_id_rejected(self):
        p = self.path("labels.jsonl")
        write_jsonl_lines(p, [
            {"schema_version": 1, "group_id": "rxn-x", "target_id": "target_x",
             "correct_precursor_sets": [["A", "B"]]},
            {"schema_version": 1, "group_id": "rxn-x", "target_id": "target_x",
             "correct_precursor_sets": [["C", "D"]]},
        ])
        with self.assertRaises(ValueError):
            tr.load_labels(p)

    def test_unsorted_precursor_set_rejected(self):
        p = self.path("labels.jsonl")
        write_jsonl_lines(p, [
            {"schema_version": 1, "group_id": "rxn-y", "target_id": "target_y",
             "correct_precursor_sets": [["CCO", "CC(=O)O"]]},  # not sorted
        ])
        with self.assertRaises(ValueError):
            tr.load_labels(p)

    def test_empty_correct_precursor_sets_rejected(self):
        p = self.path("labels.jsonl")
        write_jsonl_lines(p, [
            {"schema_version": 1, "group_id": "rxn-empty", "target_id": "target_empty",
             "correct_precursor_sets": []},
        ])
        with self.assertRaises(ValueError):
            tr.load_labels(p)

    def test_wrong_schema_version_rejected(self):
        p = self.path("labels.jsonl")
        write_jsonl_lines(p, [
            {"schema_version": 2, "group_id": "rxn-z", "target_id": "target_z",
             "correct_precursor_sets": [["A"]]},
        ])
        with self.assertRaises(ValueError):
            tr.load_labels(p)


class LabelAndSplitRowsTests(unittest.TestCase):
    def setUp(self):
        self.pool_rows = [
            {
                "group_id": "rxn-a1", "target_id": "target_a", "target_smiles": "CC(=O)OCC",
                "candidate_id": "sha256:aaa", "precursor_smiles": ["CC(=O)O", "CCO"],
                "source_template_count": 1, "best_upstream_rank": 0, "feature_schema_version": 1,
                "feature_values": [0.0] * len(tr.FEATURE_NAMES_V1),
                "feature_missing": [False] * 13 + [True] * 5,
                "sources": [{"template_id": "rule:a"}],
            },
            {
                "group_id": "rxn-a1", "target_id": "target_a", "target_smiles": "CC(=O)OCC",
                "candidate_id": "sha256:bbb", "precursor_smiles": ["CCl", "CCO"],
                "source_template_count": 1, "best_upstream_rank": 1, "feature_schema_version": 1,
                "feature_values": [0.0] * len(tr.FEATURE_NAMES_V1),
                "feature_missing": [False] * 13 + [True] * 5,
                "sources": [{"template_id": "rule:b"}],
            },
            {
                "group_id": "rxn-b1", "target_id": "target_b", "target_smiles": "CCN",
                "candidate_id": "sha256:ccc", "precursor_smiles": ["CCBr"],
                "source_template_count": 1, "best_upstream_rank": 0, "feature_schema_version": 1,
                "feature_values": [0.0] * len(tr.FEATURE_NAMES_V1),
                "feature_missing": [False] * 13 + [True] * 5,
                "sources": [{"template_id": "rule:c"}],
            },
        ]
        # rxn-c1/rxn-d1 have zero candidates -- present only in the group index.
        self.group_records = [
            {"group_id": "rxn-a1", "target_id": "target_a", "target_smiles": "CC(=O)OCC",
             "candidate_count": 2, "proposal_status": "ok"},
            {"group_id": "rxn-b1", "target_id": "target_b", "target_smiles": "CCN",
             "candidate_count": 1, "proposal_status": "ok"},
            {"group_id": "rxn-c1", "target_id": "target_c", "target_smiles": "CCC",
             "candidate_count": 0, "proposal_status": "ok"},
            {"group_id": "rxn-d1", "target_id": "target_d", "target_smiles": "CCCC",
             "candidate_count": 0, "proposal_status": "target_parse_failed"},
        ]
        self.labels = {
            "rxn-a1": tr.GroupLabel(target_id="target_a", correct_precursor_sets=frozenset({("CC(=O)O", "CCO")})),
            "rxn-c1": tr.GroupLabel(target_id="target_c", correct_precursor_sets=frozenset({("X", "Y")})),
            "rxn-d1": tr.GroupLabel(target_id="target_d", correct_precursor_sets=frozenset({("X", "Y")})),
            # rxn-b1 deliberately absent -> --allow-unlabeled.
        }

    def test_unlabeled_group_is_hard_error_by_default(self):
        with self.assertRaises(ValueError):
            tr.label_and_split_rows(self.pool_rows, self.labels, self.group_records, allow_unlabeled=False)

    def test_allow_unlabeled_excludes_and_counts(self):
        labeled, unlabeled_count = tr.label_and_split_rows(
            self.pool_rows, self.labels, self.group_records, allow_unlabeled=True
        )
        self.assertEqual(unlabeled_count, 1)
        self.assertTrue(all(r.group_id != "rxn-b1" for r in labeled))

    def test_unlabeled_group_is_excluded_not_defaulted_to_negative(self):
        labeled, _ = tr.label_and_split_rows(self.pool_rows, self.labels, self.group_records, allow_unlabeled=True)
        group_ids_present = {r.group_id for r in labeled}
        self.assertNotIn("rxn-b1", group_ids_present)

    def test_exact_precursor_match_is_positive_non_match_is_negative(self):
        labeled, _ = tr.label_and_split_rows(self.pool_rows, self.labels, self.group_records, allow_unlabeled=True)
        by_id = {r.candidate_id: r for r in labeled}
        self.assertEqual(by_id["sha256:aaa"].label, 1)
        self.assertEqual(by_id["sha256:bbb"].label, 0)

    def test_missing_feature_becomes_nan_not_zero(self):
        import math

        labeled, _ = tr.label_and_split_rows(self.pool_rows, self.labels, self.group_records, allow_unlabeled=True)
        by_id = {r.candidate_id: r for r in labeled}
        self.assertTrue(math.isnan(by_id["sha256:aaa"].features[-1]))

    def test_zero_candidate_group_counted_in_coverage_denominator(self):
        labeled, _ = tr.label_and_split_rows(self.pool_rows, self.labels, self.group_records, allow_unlabeled=True)
        total_group_count = 0
        total_target_count = 0
        for split in ("train", "val", "test"):
            cov = tr.summarize_coverage(labeled, self.group_records, self.labels, split)
            total_group_count += cov.group_count
            total_target_count += cov.target_count
        self.assertEqual(total_group_count, 3, "rxn-a1, rxn-c1, rxn-d1 (rxn-b1 excluded, unlabeled)")
        self.assertEqual(total_target_count, 3)

    def test_same_target_different_group_shares_split_but_not_group(self):
        group_records = [
            {"group_id": "rxn-1", "target_id": "target_shared", "target_smiles": "CC(=O)OCC",
             "candidate_count": 1, "proposal_status": "ok"},
            {"group_id": "rxn-2", "target_id": "target_shared", "target_smiles": "CC(=O)OCC",
             "candidate_count": 1, "proposal_status": "ok"},
        ]
        pool_rows = [
            {"group_id": "rxn-1", "target_id": "target_shared", "target_smiles": "CC(=O)OCC",
             "candidate_id": "sha256:a1", "precursor_smiles": ["CC(=O)O", "CCO"],
             "feature_values": [0.0], "feature_missing": [False], "sources": [{"template_id": "rule:a"}]},
            {"group_id": "rxn-2", "target_id": "target_shared", "target_smiles": "CC(=O)OCC",
             "candidate_id": "sha256:a2", "precursor_smiles": ["CC(=O)O", "CCO"],
             "feature_values": [0.0], "feature_missing": [False], "sources": [{"template_id": "rule:a"}]},
        ]
        labels = {
            "rxn-1": tr.GroupLabel(target_id="target_shared", correct_precursor_sets=frozenset({("CC(=O)O", "CCO")})),
            "rxn-2": tr.GroupLabel(target_id="target_shared", correct_precursor_sets=frozenset({("CC(=O)O", "CCO")})),
        }
        labeled, unlabeled_count = tr.label_and_split_rows(pool_rows, labels, group_records)
        self.assertEqual(unlabeled_count, 0)
        splits = {r.group_id: r.split for r in labeled}
        self.assertEqual(splits["rxn-1"], splits["rxn-2"], "same target_id must land in the same split")
        sizes = tr.group_sizes(sorted(labeled, key=lambda r: (r.group_id, r.candidate_id)))
        self.assertEqual(sizes, [1, 1], "different group_id must form separate LightGBM ranking groups")

    def test_labels_target_id_mismatch_with_group_index_is_hard_error(self):
        group_records = [
            {"group_id": "g1", "target_id": "real-target", "target_smiles": "CCO", "candidate_count": 1, "proposal_status": "ok"},
        ]
        pool_rows = [
            {"group_id": "g1", "target_id": "real-target", "target_smiles": "CCO",
             "candidate_id": "c1", "precursor_smiles": ["CCO"], "feature_values": [0.0],
             "feature_missing": [False], "sources": [{"template_id": "rule:a"}]},
        ]
        labels = {"g1": tr.GroupLabel(target_id="different-target", correct_precursor_sets=frozenset({("CCO",)}))}
        with self.assertRaises(ValueError):
            tr.label_and_split_rows(pool_rows, labels, group_records)


if __name__ == "__main__":
    unittest.main()
