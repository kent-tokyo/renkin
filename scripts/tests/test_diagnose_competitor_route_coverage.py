import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).parents[1] / "diagnose_competitor_route_coverage.py"
SPEC = importlib.util.spec_from_file_location("coverage", MODULE_PATH)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class CoverageDiagnosticTests(unittest.TestCase):
    def test_load_pool_groups_precursor_multisets(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "pool.jsonl"
            rows = [
                {"target_smiles": "T", "precursor_smiles": ["B", "A"]},
                {"target_smiles": "T", "precursor_smiles": ["A", "B"]},
                {"target_smiles": "T", "precursor_smiles": ["C"]},
            ]
            path.write_text("\n".join(json.dumps(row) for row in rows) + "\n")
            loaded = MODULE.load_pool(path)
            self.assertEqual(loaded["T"], {("A", "B"), ("C",)})

    def test_load_edges_respects_fixed_cohort(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "competitor.jsonl"
            row = {
                "target_id": "keep",
                "tool_specific": {
                    "aizynthfinder": {
                        "route_edge_snapshot": [
                            {"target": "T", "precursors": ["A", "B"]}
                        ]
                    }
                },
            }
            other = {**row, "target_id": "skip"}
            path.write_text("\n".join(json.dumps(item) for item in (row, other)) + "\n")
            edges = MODULE.load_edges(path, {"keep"})
            self.assertEqual(len(edges), 1)
            self.assertEqual(edges[0]["target_id"], "keep")
            self.assertEqual(edges[0]["precursors"], ["A", "B"])

    def test_missing_target_groups_are_unique_and_pool_gen_compatible(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "out.jsonl"
            count = MODULE.write_missing_target_groups(
                [
                    {"canonical_target": "CCO", "pool_candidate_count": 0},
                    {"canonical_target": "CCO", "pool_candidate_count": 0},
                    {"canonical_target": "c1ccccc1", "pool_candidate_count": 1},
                ],
                path,
            )
            rows = [json.loads(line) for line in path.read_text().splitlines()]
            self.assertEqual(count, 1)
            self.assertEqual(rows, [{"group_id": "competitor-missing-0", "target_id": "CCO"}])


if __name__ == "__main__":
    unittest.main()
