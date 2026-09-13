import json
import os
import sys
import tempfile
import unittest


sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from train_template_scorer import (  # noqa: E402
    TemplateScorer,
    _load_rows,
    export_onnx,
    parse_hidden_dims,
)


class TestTrainTemplateScorerInput(unittest.TestCase):
    def test_loads_mapped_reaction_jsonl(self):
        with tempfile.TemporaryDirectory() as directory:
            path = os.path.join(directory, "train.jsonl")
            with open(path, "w", encoding="utf-8") as handle:
                handle.write(json.dumps({
                    "id": "train-1",
                    "reactants": "[CH3:1][OH:2]",
                    "product": "[CH3:1][O:2][CH3:3]",
                }) + "\n")
            rows = _load_rows(reactions_jsonl_path=path)
        self.assertEqual(rows, [{
            "_id": "train-1",
            "reactants": "[CH3:1][OH:2]",
            "products": "[CH3:1][O:2][CH3:3]",
        }])

    def test_rejects_missing_reaction_fields(self):
        with tempfile.TemporaryDirectory() as directory:
            path = os.path.join(directory, "invalid.jsonl")
            with open(path, "w", encoding="utf-8") as handle:
                handle.write('{"reactants":"CC"}\n')
            with self.assertRaisesRegex(ValueError, "missing string reactants/product"):
                _load_rows(reactions_jsonl_path=path)

    def test_rejects_mixed_local_input_formats(self):
        with self.assertRaisesRegex(ValueError, "cannot be combined"):
            _load_rows(reactions_path="a.smi", reactions_jsonl_path="b.jsonl")

    def test_export_moves_an_mps_model_back_to_cpu(self):
        if not __import__("torch").backends.mps.is_available():
            self.skipTest("MPS is unavailable")
        with tempfile.TemporaryDirectory() as directory:
            output = os.path.join(directory, "scorer.onnx")
            export_onnx(TemplateScorer(2).to("mps"), output)
            self.assertTrue(os.path.isfile(output))

    def test_compact_hidden_dimensions_keep_the_full_template_axis(self):
        import torch
        model = TemplateScorer(17, (32, 8))
        self.assertEqual(tuple(model(torch.zeros(3, 2048)).shape), (3, 17))

    def test_hidden_dimension_parser_rejects_invalid_values(self):
        for value in ("", "0", "16,0", "-1,8", "wide,8"):
            with self.assertRaises(ValueError):
                parse_hidden_dims(value)


if __name__ == "__main__":
    unittest.main()
