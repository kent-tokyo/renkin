import importlib.util
import tempfile
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).parents[1] / "expand_candidate_frontier.py"
SPEC = importlib.util.spec_from_file_location("frontier", MODULE_PATH)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class FrontierTests(unittest.TestCase):
    def test_write_groups_is_sorted_and_round_scoped(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "groups.jsonl"
            MODULE.write_groups(path, ["B", "A"], 3)
            self.assertEqual(
                path.read_text().splitlines(),
                [
                    '{"group_id": "frontier-r3-0", "target_id": "B"}',
                    '{"group_id": "frontier-r3-1", "target_id": "A"}',
                ],
            )

    def test_load_stock_uses_compiled_stock_payload(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "stock.rstock"
            path.write_text("RENKIN-STOCK-V1\nmanifest\nCCO\n\nCCC\n")
            self.assertEqual(MODULE.load_stock(path), {"CCO", "CCC"})

    def test_target_value_accepts_canonical_smiles_group_inputs(self):
        self.assertEqual(MODULE.target_value({"canonical_smiles": "CCO", "target_id": "id"}), "CCO")

    def test_frontier_selection_prefers_stock_near_parent(self):
        rows = [
            {"precursor_smiles": ["STOCK", "far", "far2"], "best_upstream_rank": 1},
            {"precursor_smiles": ["near", "STOCK"], "best_upstream_rank": 2},
        ]
        self.assertEqual(MODULE.select_frontier_targets(rows, {"STOCK"}, 1), ["near"])


if __name__ == "__main__":
    unittest.main()
