import importlib.util
import tempfile
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).parents[1] / "select_retro_generator_graph.py"
SPEC = importlib.util.spec_from_file_location("graph_selector", MODULE_PATH)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class GraphSelectorTests(unittest.TestCase):
    def test_child_stock_terminality_breaks_same_parent_ties(self):
        rows = [
            {"target_smiles": "T", "precursor_smiles": ["A"], "candidate_id": "good"},
            {"target_smiles": "A", "precursor_smiles": ["STOCK"], "candidate_id": "good"},
            {"target_smiles": "B", "precursor_smiles": ["X"], "candidate_id": "bad"},
            {"target_smiles": "T", "precursor_smiles": ["B"], "candidate_id": "bad-parent"},
        ]
        selected, summary = MODULE.select(rows, {"STOCK"}, set(), 1)
        self.assertEqual(summary["output_rows"], 3)
        self.assertEqual([row["candidate_id"] for row in selected if row["target_smiles"] == "T"], ["good"])

    def test_bounded_multi_hop_lookahead_prefers_reachable_candidate(self):
        rows = [
            {"target_smiles": "T", "precursor_smiles": ["A"], "candidate_id": "two-hop"},
            {"target_smiles": "T", "precursor_smiles": ["B"], "candidate_id": "dead-end"},
            {"target_smiles": "A", "precursor_smiles": ["X"], "candidate_id": "a-to-x"},
            {"target_smiles": "B", "precursor_smiles": ["Y"], "candidate_id": "b-to-y"},
            {"target_smiles": "X", "precursor_smiles": ["STOCK"], "candidate_id": "x-stock"},
            {"target_smiles": "Y", "precursor_smiles": ["Z"], "candidate_id": "y-dead"},
        ]
        selected, _summary = MODULE.select(rows, {"STOCK"}, set(), 1, lookahead_depth=2)
        self.assertEqual(
            [row["candidate_id"] for row in selected if row["target_smiles"] == "T"],
            ["two-hop"],
        )

    def test_opt_in_diversity_selection_keeps_different_precursor_shapes(self):
        rows = [
            {"target_smiles": "T", "precursor_smiles": ["A"], "candidate_id": "one-a", "best_upstream_rank": 0},
            {"target_smiles": "T", "precursor_smiles": ["B"], "candidate_id": "one-b", "best_upstream_rank": 1},
            {"target_smiles": "T", "precursor_smiles": ["C", "D"], "candidate_id": "two", "best_upstream_rank": 2},
        ]
        selected, summary = MODULE.select(rows, set(), set(), 2, source_rank_bucket=8)
        ids = {row["candidate_id"] for row in selected if row["target_smiles"] == "T"}
        self.assertEqual(ids, {"one-a", "two"})
        self.assertEqual(summary["source_rank_bucket"], 8)

    def test_load_targets_accepts_canonical_smiles(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "roots.jsonl"
            path.write_text('{"canonical_smiles":"CCO","target_id":"id"}\n')
            self.assertEqual(MODULE.load_targets(path), {"CCO"})

    def test_preserved_intermediate_target_is_not_capped(self):
        rows = [
            {"target_smiles": "T", "precursor_smiles": ["A"], "candidate_id": "root"},
            {"target_smiles": "A", "precursor_smiles": ["X"], "candidate_id": "a1"},
            {"target_smiles": "A", "precursor_smiles": ["Y"], "candidate_id": "a2"},
        ]
        selected, summary = MODULE.select(rows, set(), {"T"}, 1, preserve_targets={"A"})
        self.assertEqual(summary["preserved_target_count"], 1)
        self.assertEqual({row["candidate_id"] for row in selected if row["target_smiles"] == "A"}, {"a1", "a2"})

    def test_load_artifact_preserves_provenance_and_shape(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "artifact.json"
            path.write_text(
                '{"schema_version":1,"proposals":{'
                '"T":[{"target":"T","precursors":["A"],'
                '"candidate_id":"c1","provenance":{"source_rank":3}}],'
                '"A":[{"target":"A","precursors":["STOCK"],'
                '"candidate_id":"c2","provenance":{"source_rank":1}}]}}'
            )
            artifact, rows = MODULE.load_artifact(path)
            self.assertEqual(artifact["schema_version"], 1)
            self.assertEqual(len(rows), 2)
            self.assertEqual(rows[0]["best_upstream_rank"], 3)
            self.assertEqual(rows[0]["_artifact_candidate"]["candidate_id"], "c1")

    def test_artifact_selection_keeps_root_candidates_and_writes_json(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            artifact = directory / "artifact.json"
            stock = directory / "stock.rstock"
            roots = directory / "roots.jsonl"
            output = directory / "selected.json"
            artifact.write_text(
                '{"schema_version":1,"proposals":{'
                '"ROOT":[{"target":"ROOT","precursors":["A"],"candidate_id":"r1"},'
                '{"target":"ROOT","precursors":["B"],"candidate_id":"r2"}],'
                '"A":[{"target":"A","precursors":["X"],"candidate_id":"a1"},'
                '{"target":"A","precursors":["STOCK"],"candidate_id":"a2"}]}}'
            )
            stock.write_text("RENKIN-COMPILED-STOCK-V1\n{}\nSTOCK\n")
            roots.write_text('{"target_smiles":"ROOT"}\n')
            self.assertEqual(MODULE.main.__name__, "main")
            import contextlib
            import io
            import sys
            old_argv = sys.argv
            try:
                sys.argv = [
                    "select_retro_generator_graph.py",
                    "--input", str(artifact), "--stock", str(stock),
                    "--root-targets", str(roots), "--output", str(output),
                    "--max-per-target", "1",
                ]
                with contextlib.redirect_stdout(io.StringIO()):
                    self.assertEqual(MODULE.main(), 0)
            finally:
                sys.argv = old_argv
            selected = __import__("json").loads(output.read_text())
            self.assertEqual(len(selected["proposals"]["ROOT"]), 2)
            self.assertEqual([c["candidate_id"] for c in selected["proposals"]["A"]], ["a2"])


if __name__ == "__main__":
    unittest.main()
