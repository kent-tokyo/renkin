import unittest

from scripts.prepare_partial_overlap_family_cohorts import partition


class PreparePartialOverlapFamilyCohortsTests(unittest.TestCase):
    def test_partitions_requested_subset_without_changing_records(self):
        diagnostic = {
            "schema_version": "test",
            "evidence_level": "heuristic_diagnostic_only",
            "records": [
                {"canonical_target": "A", "target_id": "a", "reaction_family_proxy": "amide_like", "x": 1},
                {"canonical_target": "B", "target_id": "b", "reaction_family_proxy": "other_or_unknown", "x": 2},
            ],
        }
        result = partition(diagnostic, {"B", "A"})
        self.assertEqual(result["subset_count"], 2)
        self.assertEqual(result["family_counts"], {"amide_like": 1, "other_or_unknown": 1})
        self.assertEqual(result["cohorts"]["amide_like"]["records"][0]["x"], 1)
        self.assertEqual(result["cohorts"]["amide_like"]["sample_rows"][0]["target_smiles"], "A")
        self.assertEqual(result["cohorts"]["amide_like"]["sample_rows"][0]["sample_rank"], 0)
        self.assertEqual(result["cohorts"]["amide_like"]["sample_rows"][0]["canonical_smiles"], "A")
        self.assertEqual(len(result["cohorts"]["amide_like"]["sample_rows"][0]["sample_key"]), 64)
        self.assertEqual(result["cohorts"]["amide_like"]["unique_target_count"], 1)

    def test_missing_target_fails_closed(self):
        with self.assertRaises(ValueError):
            partition(
                {"records": [{"canonical_target": "A", "reaction_family_proxy": "other_or_unknown"}]},
                {"B"},
            )


if __name__ == "__main__":
    unittest.main()
