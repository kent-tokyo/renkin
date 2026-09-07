import json
import tempfile
import unittest
from pathlib import Path

import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import prepare_static_retro_generator as converter  # noqa: E402


class TestPrepareStaticRetroGenerator(unittest.TestCase):
    def test_converts_rows_and_pins_artifact_hash(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "model.jsonl"
            artifact = root / "artifact.json"
            manifest = root / "manifest.json"
            source.write_text(
                json.dumps(
                    {
                        "target_smiles": "CCO",
                        "candidate_id": "c1",
                        "precursor_smiles": ["CC", "O"],
                        "model_confidence": 0.75,
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            self.assertEqual(
                converter.main(
                    [
                        "--input",
                        str(source),
                        "--artifact-output",
                        str(artifact),
                        "--manifest-output",
                        str(manifest),
                        "--generator-id",
                        "fixture",
                        "--generator-version",
                        "1",
                    ]
                ),
                0,
            )
            payload = json.loads(artifact.read_text(encoding="utf-8"))
            self.assertEqual(payload["proposals"]["CCO"][0]["candidate_id"], "c1")
            self.assertEqual(payload["proposals"]["CCO"][0]["provenance"]["source_rank"], 0)
            expected = "sha256:" + __import__("hashlib").sha256(artifact.read_bytes()).hexdigest()
            self.assertEqual(json.loads(manifest.read_text())["model_sha256"], expected)

    def test_rejects_invalid_confidence(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "model.jsonl"
            source.write_text(
                '{"target_smiles":"CCO","precursor_smiles":["CC"],"model_confidence":2}\n',
                encoding="utf-8",
            )
            with self.assertRaisesRegex(ValueError, "model_confidence"):
                converter.load_rows(source, "fixture", "1", 10)


if __name__ == "__main__":
    unittest.main()
