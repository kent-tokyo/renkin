"""Manifest and candidate-row schema contract tests (Commit 4).

Mirrors, from the Python consuming side, the same invariants
`renkin::pool_export::build_manifest`/`validate_candidate_rows`/
`validate_rows_consistent_with_group_index` enforce in Rust before writing
-- both sides must reject the same malformed input.
"""

import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import train_reranker as tr  # noqa: E402


def sha256_file(path):
    return tr.sha256_file(path)


class FeatureSchemaHashPinTests(unittest.TestCase):
    """Cross-language pin: this literal must match the one hardcoded in
    src/candidate.rs's feature_schema_hash_is_stable_and_pinned_for_cross_language_verification.
    If this test fails after an intentional FEATURE_NAMES_V1/FEATURE_SCHEMA_VERSION
    change, update BOTH literals together -- a mismatch here means Python and
    Rust have silently diverged on what the exported feature schema is.
    """

    def test_feature_schema_hash_matches_the_rust_pinned_literal(self):
        self.assertEqual(
            tr.feature_schema_hash(),
            "sha256:756404c59bbee9a65e194f92df3530e1b801028f333e01c67214917977061df1",
        )


class ManifestValidationTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.pool_path = os.path.join(self.tmp.name, "pool.jsonl")
        with open(self.pool_path, "w", encoding="utf-8") as f:
            f.write('{"candidate_id": "c1"}\n')
        self.groups_path = os.path.join(self.tmp.name, "groups.jsonl")
        with open(self.groups_path, "w", encoding="utf-8") as f:
            f.write('{"group_id": "g1"}\n')

    def tearDown(self):
        self.tmp.cleanup()

    def base_manifest(self):
        return {
            "manifest_schema_version": tr.MANIFEST_SCHEMA_VERSION,
            "feature_schema_version": tr.FEATURE_SCHEMA_VERSION,
            "feature_names": list(tr.FEATURE_NAMES_V1),
            "feature_schema_hash": tr.feature_schema_hash(),
            "proposal_mode": {"mode": "exhaustive"},
            "rules_content_hash": "sha256:deadbeef",
            "candidate_jsonl_sha256": sha256_file(self.pool_path),
            "target_group_index_sha256": sha256_file(self.groups_path),
            "stock_identity": None,
            "stock_content_sha256": None,
        }

    def assert_rejected(self, manifest):
        with self.assertRaises(ValueError):
            tr.validate_manifest(manifest, self.pool_path, self.groups_path)

    def test_valid_manifest_is_accepted(self):
        tr.validate_manifest(self.base_manifest(), self.pool_path, self.groups_path)  # no raise

    def test_manifest_schema_version_mismatch_rejected(self):
        self.assert_rejected({**self.base_manifest(), "manifest_schema_version": 999})

    def test_feature_schema_version_mismatch_rejected(self):
        self.assert_rejected({**self.base_manifest(), "feature_schema_version": 999})

    def test_feature_names_mismatch_rejected(self):
        self.assert_rejected({**self.base_manifest(), "feature_names": ["wrong"]})

    def test_feature_names_reorder_is_rejected(self):
        reordered = list(tr.FEATURE_NAMES_V1)
        reordered[0], reordered[1] = reordered[1], reordered[0]
        self.assert_rejected({**self.base_manifest(), "feature_names": reordered})

    def test_feature_schema_hash_differs_for_a_reordered_feature_list(self):
        # feature_names alone (a list comparison) already catches a reorder
        # (see test_feature_names_reorder_is_rejected) -- this confirms
        # feature_schema_hash is ALSO sensitive to order, so a consumer
        # that only compared feature_names by set (not by sequence) would
        # still be caught by the hash.
        reordered = list(tr.FEATURE_NAMES_V1)
        reordered[0], reordered[1] = reordered[1], reordered[0]
        original_names = tr.FEATURE_NAMES_V1
        tr.FEATURE_NAMES_V1 = reordered
        try:
            reordered_hash = tr.feature_schema_hash()
        finally:
            tr.FEATURE_NAMES_V1 = original_names
        self.assertNotEqual(reordered_hash, tr.feature_schema_hash())

    def test_feature_schema_hash_mismatch_rejected(self):
        self.assert_rejected({**self.base_manifest(), "feature_schema_hash": "sha256:wrong"})

    def test_missing_rules_content_hash_rejected(self):
        self.assert_rejected({**self.base_manifest(), "rules_content_hash": ""})

    def test_candidate_jsonl_hash_mismatch_rejected(self):
        self.assert_rejected({**self.base_manifest(), "candidate_jsonl_sha256": "sha256:wrong"})

    def test_target_group_index_hash_mismatch_rejected(self):
        self.assert_rejected({**self.base_manifest(), "target_group_index_sha256": "sha256:wrong"})

    def test_scorer_conditioned_with_non_available_status_rejected(self):
        manifest = {
            **self.base_manifest(),
            "proposal_mode": {"mode": "scorer_conditioned", "scorer_status": "inference_failed"},
        }
        self.assert_rejected(manifest)

    def test_scorer_conditioned_with_available_status_accepted(self):
        manifest = {
            **self.base_manifest(),
            "proposal_mode": {"mode": "scorer_conditioned", "scorer_status": "available"},
        }
        tr.validate_manifest(manifest, self.pool_path, self.groups_path)  # no raise

    def test_stock_identity_without_content_hash_rejected(self):
        manifest = {**self.base_manifest(), "stock_identity": "some/path.smi", "stock_content_sha256": None}
        self.assert_rejected(manifest)

    def test_stock_content_hash_without_identity_rejected(self):
        manifest = {**self.base_manifest(), "stock_identity": None, "stock_content_sha256": "sha256:abc"}
        self.assert_rejected(manifest)

    def test_stock_identity_with_content_hash_accepted(self):
        manifest = {**self.base_manifest(), "stock_identity": "some/path.smi", "stock_content_sha256": "sha256:abc"}
        tr.validate_manifest(manifest, self.pool_path, self.groups_path)  # no raise


class CandidateRowValidationTests(unittest.TestCase):
    GROUPS = [
        {"group_id": "g1", "target_id": "t1", "target_smiles": "CCO", "candidate_count": 1, "proposal_status": "ok"},
    ]

    def good_row(self, **overrides):
        row = {
            "group_id": "g1",
            "target_id": "t1",
            "target_smiles": "CCO",
            "candidate_id": "c1",
            "precursor_smiles": ["CCO"],
            "sources": [{"template_id": "rule:x"}],
            "feature_schema_version": tr.FEATURE_SCHEMA_VERSION,
            "feature_values": [0.0] * len(tr.FEATURE_NAMES_V1),
            "feature_missing": [True] * len(tr.FEATURE_NAMES_V1),
        }
        row.update(overrides)
        return row

    def test_valid_row_is_accepted(self):
        tr.validate_pool_rows([self.good_row()], self.GROUPS)  # no raise

    def test_feature_schema_version_mismatch_rejected(self):
        with self.assertRaises(ValueError):
            tr.validate_pool_rows([self.good_row(feature_schema_version=999)], self.GROUPS)

    def test_feature_values_length_mismatch_rejected(self):
        row = self.good_row(feature_values=[0.0] * (len(tr.FEATURE_NAMES_V1) - 1))
        with self.assertRaises(ValueError):
            tr.validate_pool_rows([row], self.GROUPS)

    def test_feature_missing_length_mismatch_rejected(self):
        row = self.good_row(feature_missing=[True] * (len(tr.FEATURE_NAMES_V1) - 1))
        with self.assertRaises(ValueError):
            tr.validate_pool_rows([row], self.GROUPS)

    def test_non_finite_non_missing_value_rejected(self):
        row = self.good_row(
            feature_values=[float("nan")] + [0.0] * (len(tr.FEATURE_NAMES_V1) - 1),
            feature_missing=[False] * len(tr.FEATURE_NAMES_V1),
        )
        with self.assertRaises(ValueError):
            tr.validate_pool_rows([row], self.GROUPS)

    def test_infinite_non_missing_value_rejected(self):
        row = self.good_row(
            feature_values=[float("inf")] + [0.0] * (len(tr.FEATURE_NAMES_V1) - 1),
            feature_missing=[False] * len(tr.FEATURE_NAMES_V1),
        )
        with self.assertRaises(ValueError):
            tr.validate_pool_rows([row], self.GROUPS)

    def test_missing_value_may_be_non_finite(self):
        row = self.good_row(
            feature_values=[float("nan")] + [0.0] * (len(tr.FEATURE_NAMES_V1) - 1),
            feature_missing=[True] * len(tr.FEATURE_NAMES_V1),
        )
        tr.validate_pool_rows([row], self.GROUPS)  # no raise

    def test_empty_precursor_smiles_rejected(self):
        with self.assertRaises(ValueError):
            tr.validate_pool_rows([self.good_row(precursor_smiles=[])], self.GROUPS)

    def test_empty_sources_rejected(self):
        with self.assertRaises(ValueError):
            tr.validate_pool_rows([self.good_row(sources=[])], self.GROUPS)

    def test_duplicate_candidate_id_within_one_group_rejected(self):
        with self.assertRaises(ValueError):
            tr.validate_pool_rows([self.good_row(), self.good_row()], self.GROUPS)

    def test_same_candidate_id_across_different_groups_is_allowed(self):
        groups = self.GROUPS + [
            {"group_id": "g2", "target_id": "t1", "target_smiles": "CCO", "candidate_count": 1, "proposal_status": "ok"},
        ]
        rows = [self.good_row(), self.good_row(group_id="g2")]
        tr.validate_pool_rows(rows, groups)  # no raise -- see CandidatePool's group_id/target_id doc

    def test_group_id_missing_from_group_index_rejected(self):
        with self.assertRaises(ValueError):
            tr.validate_pool_rows([self.good_row(group_id="g-missing")], self.GROUPS)

    def test_target_id_inconsistent_with_group_index_rejected(self):
        with self.assertRaises(ValueError):
            tr.validate_pool_rows([self.good_row(target_id="t-wrong")], self.GROUPS)

    def test_target_smiles_inconsistent_with_group_index_rejected(self):
        with self.assertRaises(ValueError):
            tr.validate_pool_rows([self.good_row(target_smiles="CCN")], self.GROUPS)

    def test_candidate_count_mismatch_rejected(self):
        # GROUPS claims candidate_count=1, but two distinct candidates are
        # supplied for g1 -- a group-index/pool consistency violation, not
        # a duplicate-candidate_id violation (the ids differ).
        rows = [self.good_row(candidate_id="c1"), self.good_row(candidate_id="c2")]
        with self.assertRaises(ValueError):
            tr.validate_pool_rows(rows, self.GROUPS)

    def test_candidate_count_matching_actual_rows_is_accepted(self):
        groups = [
            {"group_id": "g1", "target_id": "t1", "target_smiles": "CCO", "candidate_count": 2, "proposal_status": "ok"},
        ]
        rows = [self.good_row(candidate_id="c1"), self.good_row(candidate_id="c2")]
        tr.validate_pool_rows(rows, groups)  # no raise

    def test_zero_candidate_group_with_no_rows_is_accepted(self):
        groups = [
            {"group_id": "g-empty", "target_id": "t-empty", "target_smiles": "CCN", "candidate_count": 0, "proposal_status": "ok"},
        ]
        tr.validate_pool_rows([], groups)  # no raise


if __name__ == "__main__":
    unittest.main()
