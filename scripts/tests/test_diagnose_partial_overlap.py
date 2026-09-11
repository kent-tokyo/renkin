import unittest

from scripts.diagnose_partial_overlap import _load_jsonl_artifact, classify, reaction_family_proxy


class PartialOverlapDiagnosticTests(unittest.TestCase):
    def test_reports_representation_and_stock_dimensions(self):
        atlas = {"records": [{
            "target_id": "a", "edge_index": 0, "canonical_target": "CC(=O)NCC",
            "canonical_precursors": ["CC(=O)Cl", "NCC"],
            "canonicalization_evaluable": True, "exact_precursor_multiset_present": False,
        }]}
        artifact = {"proposals": {"CC(=O)NCC": [{
            "candidate_id": "c1", "precursors": ["CC(=O)Cl", "NCCO"],
        }]}}
        result = classify(atlas, artifact, {"CC(=O)Cl", "NCCO"})
        self.assertEqual(result["record_count"], 1)
        self.assertEqual(result["family_proxy_counts"], {"amide_like": 1})
        self.assertEqual(result["stock_status_counts"], {"downstream_or_unavailable": 1})
        self.assertEqual(result["records"][0]["missing_precursors"][0]["representation_status"], "not_equivalent_by_rdkit")

    def test_stereo_difference_is_not_called_exact(self):
        atlas = {"records": [{
            "target_id": "a", "edge_index": 0, "canonical_target": "CC(O)C(=O)O",
            "canonical_precursors": ["C[C@H](O)C(=O)O", "N"],
            "canonicalization_evaluable": True, "exact_precursor_multiset_present": False,
        }]}
        artifact = {"proposals": {"CC(O)C(=O)O": [{"precursors": ["C[C@@H](O)C(=O)O", "N"]}]}}
        result = classify(atlas, artifact)
        self.assertEqual(result["records"][0]["missing_precursors"][0]["representation_status"], "stereo_only_difference")

    def test_family_proxy_is_explicitly_heuristic(self):
        self.assertEqual(reaction_family_proxy("c1ccccc1", ["Brc1ccccc1"]), "other_or_unknown")

    def test_loads_candidate_pool_jsonl_and_indexes_canonical_target(self):
        artifact = _load_jsonl_artifact(['{"target_smiles":"OCC", "precursor_smiles":["C", "O"]}'])
        self.assertIn("CCO", artifact["proposals"])


if __name__ == "__main__":
    unittest.main()
