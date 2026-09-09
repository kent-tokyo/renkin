import json
import tempfile
import unittest
from pathlib import Path

import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import select_retro_generator_lookahead as selector  # noqa: E402


class TestSelectRetroGeneratorLookahead(unittest.TestCase):
    def test_prefers_terminal_child_without_inventing_proposals(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            stock = root / "stock.rstock"
            stock.write_text("magic\nmanifest\nA\nB\n", encoding="utf-8")
            artifact = root / "artifact.json"
            artifact.write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "proposals": {
                            "T": [
                                {
                                    "candidate_id": "direct",
                                    "target": "T",
                                    "precursors": ["X"],
                                    "provenance": {"source_rank": 2},
                                },
                                {
                                    "candidate_id": "terminal",
                                    "target": "T",
                                    "precursors": ["Y"],
                                    "provenance": {"source_rank": 1},
                                },
                            ],
                            "X": [
                                {
                                    "candidate_id": "x-child",
                                    "target": "X",
                                    "precursors": ["A"],
                                    "provenance": {"source_rank": 1},
                                }
                            ],
                        },
                    }
                ),
                encoding="utf-8",
            )
            output = root / "selected.json"
            self.assertEqual(
                selector.main(
                    [
                        "--artifact",
                        str(artifact),
                        "--stock",
                        str(stock),
                        "--output",
                        str(output),
                        "--max-per-target",
                        "1",
                    ]
                ),
                0,
            )
            selected = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(selected["proposals"]["T"][0]["candidate_id"], "direct")
            self.assertEqual(len(selected["proposals"]["T"]), 1)

    def test_rejects_malformed_stock(self):
        with tempfile.TemporaryDirectory() as directory:
            stock = Path(directory) / "stock.rstock"
            stock.write_text("only-one-line\n", encoding="utf-8")
            with self.assertRaises(ValueError):
                selector.load_stock(stock)

    def test_preserves_root_candidates_when_bounded_intermediates_are_selected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            stock = root / "stock.rstock"
            stock.write_text("magic\nmanifest\nA\n", encoding="utf-8")
            artifact = root / "artifact.json"
            artifact.write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "proposals": {
                            "T": [
                                {"candidate_id": "one", "target": "T", "precursors": ["X"], "provenance": {}},
                                {"candidate_id": "two", "target": "T", "precursors": ["Y"], "provenance": {}},
                            ]
                        },
                    }
                ),
                encoding="utf-8",
            )
            roots = root / "roots.jsonl"
            roots.write_text(json.dumps({"canonical_smiles": "T"}) + "\n", encoding="utf-8")
            output = root / "selected.json"
            selector.main(
                [
                    "--artifact", str(artifact), "--stock", str(stock),
                    "--root-targets", str(roots), "--output", str(output),
                    "--max-per-target", "1",
                ]
            )
            selected = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(len(selected["proposals"]["T"]), 2)


if __name__ == "__main__":
    unittest.main()
