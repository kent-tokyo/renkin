import importlib.util
import unittest
from pathlib import Path
from types import SimpleNamespace


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "phase_a5_pool_metrics", ROOT / "scripts" / "phase_a5_pool_metrics.py"
)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def stats(**overrides):
    value = {
        "rules_available": 3,
        "rules_considered": 2,
        "rules_filtered_out": 1,
        "rules_element_filtered_out": 0,
        "rules_attempted": 2,
        "rules_producing_candidates": 1,
        "rules_without_candidates": 1,
        "raw_candidates_generated": 3,
        "unique_candidates": 2,
        "duplicate_candidates": 1,
    }
    value.update(overrides)
    return value


class PhaseA5PoolMetricsTests(unittest.TestCase):
    def test_positive_recall_at_k_uses_upstream_order_and_keeps_zero_groups(self):
        labels = {
            "g1": SimpleNamespace(correct_precursor_sets={("good",)}),
            "g2": SimpleNamespace(correct_precursor_sets={("other",)}),
        }
        rows = [
            {"group_id": "g1", "candidate_id": "late", "best_upstream_rank": 2,
             "precursor_smiles": ["good"]},
            {"group_id": "g1", "candidate_id": "early", "best_upstream_rank": 1,
             "precursor_smiles": ["bad"]},
        ]
        records = [
            {"group_id": "g1", "candidate_count": 2},
            {"group_id": "g2", "candidate_count": 0},
        ]
        self.assertEqual(MODULE.positive_recall_at_k(rows, labels, records, 1), 0.0)
        self.assertEqual(MODULE.positive_recall_at_k(rows, labels, records, 10), 0.5)

    def test_positive_recall_rejects_non_positive_k(self):
        with self.assertRaises(ValueError):
            MODULE.positive_recall_at_k([], {}, [], 0)

    def test_persisted_accounting_is_aggregated_and_drives_dedup_rate(self):
        records = [
            {"group_id": "g1", "proposal_status": "ok", "candidate_count": 2,
             "candidate_pool_stats": stats()},
            {"group_id": "g2", "proposal_status": "target_parse_failed", "candidate_count": 0,
             "candidate_pool_stats": None},
        ]
        result = MODULE.candidate_pool_accounting(records)
        self.assertEqual(result["measured_successful_groups"], 1)
        self.assertEqual(result["unmeasured_successful_groups"], 0)
        self.assertEqual(result["raw_candidates_generated"], 3)
        self.assertEqual(result["unique_candidates"], 2)
        self.assertEqual(result["duplicate_candidates"], 1)
        self.assertAlmostEqual(result["dedup_rate"], 1 / 3)

    def test_legacy_index_without_stats_returns_none(self):
        records = [{"group_id": "legacy", "proposal_status": "ok", "candidate_count": 1}]
        self.assertIsNone(MODULE.candidate_pool_accounting(records))

    def test_inconsistent_persisted_stats_are_rejected(self):
        record = {"group_id": "bad", "proposal_status": "ok", "candidate_count": 2,
                  "candidate_pool_stats": stats(raw_candidates_generated=2)}
        with self.assertRaises(ValueError):
            MODULE.candidate_pool_accounting([record])


if __name__ == "__main__":
    unittest.main()
