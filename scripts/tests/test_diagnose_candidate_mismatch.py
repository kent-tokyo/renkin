import unittest

from scripts.diagnose_candidate_mismatch import classify


class CandidateMismatchTests(unittest.TestCase):
    def test_classifies_partial_and_no_overlap(self):
        atlas = {
            "records": [
                {
                    "target_id": "a",
                    "edge_index": 0,
                    "canonical_target": "T",
                    "canonical_precursors": ["A", "B"],
                    "canonicalization_evaluable": True,
                    "exact_precursor_multiset_present": False,
                },
                {
                    "target_id": "b",
                    "edge_index": 1,
                    "canonical_target": "U",
                    "canonical_precursors": ["C"],
                    "canonicalization_evaluable": True,
                    "exact_precursor_multiset_present": False,
                },
            ]
        }
        artifact = {
            "proposals": {
                "T": [{"candidate_id": "1", "precursors": ["A", "X"], "provenance": {"source_rank": 4}}],
                "U": [{"candidate_id": "2", "precursors": ["Y"], "provenance": {"source_rank": 5}}],
            }
        }
        result = classify(atlas, artifact)
        self.assertEqual(result["category_counts"], {"no_precursor_overlap": 1, "partial_precursor_overlap": 1})
        self.assertEqual(result["records"][0]["missing_expected_precursors"], ["B"])
        self.assertEqual(result["records"][0]["action_class"], "partial_downstream_precursor")

    def test_skips_exact_and_non_evaluable_edges(self):
        atlas = {
            "records": [
                {"canonicalization_evaluable": False},
                {"canonicalization_evaluable": True, "exact_precursor_multiset_present": True},
            ]
        }
        self.assertEqual(classify(atlas, {"proposals": {}})["mismatch_count"], 0)

    def test_computes_conservative_stock_reachability(self):
        atlas = {
            "records": [
                {
                    "target_id": "a",
                    "edge_index": 0,
                    "canonical_target": "T",
                    "canonical_precursors": ["A", "B"],
                    "canonicalization_evaluable": True,
                    "exact_precursor_multiset_present": False,
                }
            ]
        }
        artifact = {"proposals": {"T": [{"precursors": ["A", "X"]}]}}
        result = classify(atlas, artifact, {"A", "X"})
        self.assertTrue(result["records"][0]["stock_reachable"])
        result = classify(atlas, artifact, {"A"})
        self.assertFalse(result["records"][0]["stock_reachable"])


if __name__ == "__main__":
    unittest.main()
