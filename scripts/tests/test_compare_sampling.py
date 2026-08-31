import os
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import compare_sampling as cs  # noqa: E402


class TestSampleInputBounds(unittest.TestCase):
    def test_candidate_loader_rejects_oversized_file(self):
        with tempfile.NamedTemporaryFile("wb") as handle:
            handle.write(b"CCO ethanol\n")
            handle.flush()
            with patch.object(cs, "MAX_SAMPLE_BYTES", 4):
                with self.assertRaisesRegex(ValueError, "exceeds"):
                    cs.load_candidate_lines(handle.name)

    def test_candidate_loader_rejects_oversized_line(self):
        with tempfile.NamedTemporaryFile("wb") as handle:
            handle.write(b"CCO ethanol\n")
            handle.flush()
            with patch.object(cs, "MAX_SAMPLE_LINE_BYTES", 4):
                with self.assertRaisesRegex(ValueError, "line exceeds"):
                    cs.load_candidate_lines(handle.name)

    def test_candidate_loader_rejects_symlink(self):
        with tempfile.TemporaryDirectory() as directory:
            target = os.path.join(directory, "corpus.smi")
            link = os.path.join(directory, "corpus-link.smi")
            with open(target, "wb") as handle:
                handle.write(b"CCO ethanol\n")
            try:
                os.symlink(target, link)
            except (NotImplementedError, OSError) as exc:
                self.skipTest(f"symlinks unavailable: {exc}")
            with self.assertRaisesRegex(ValueError, "symlink"):
                cs.load_candidate_lines(link)

    def test_candidate_loader_preserves_utf8_and_line_numbers(self):
        with tempfile.NamedTemporaryFile("wb") as handle:
            handle.write(b"# comment\nCCO ethanol\n")
            handle.flush()
            candidates, total, comments = cs.load_candidate_lines(handle.name)
        self.assertEqual(total, 2)
        self.assertEqual(comments, 1)
        self.assertEqual(candidates[0].line_number, 2)
        self.assertEqual(candidates[0].raw_smiles, "CCO")

    def test_sample_loader_rejects_oversized_line(self):
        with tempfile.NamedTemporaryFile("wb") as handle:
            handle.write(b'{"sample_rank": 0}\n')
            handle.flush()
            with patch.object(cs, "MAX_SAMPLE_LINE_BYTES", 4):
                with self.assertRaisesRegex(ValueError, "line exceeds"):
                    cs.load_sample(handle.name)

    def test_sample_loader_rejects_symlink(self):
        with tempfile.TemporaryDirectory() as directory:
            target = os.path.join(directory, "sample.jsonl")
            link = os.path.join(directory, "sample-link.jsonl")
            with open(target, "w", encoding="utf-8") as handle:
                handle.write('{"sample_rank": 0}\n')
            try:
                os.symlink(target, link)
            except (NotImplementedError, OSError) as exc:
                self.skipTest(f"symlinks unavailable: {exc}")
            with self.assertRaisesRegex(ValueError, "symlink"):
                cs.load_sample(link)

    def test_sample_loader_returns_ranked_rows(self):
        with tempfile.NamedTemporaryFile("w", encoding="utf-8") as handle:
            handle.write('{"sample_rank": 1, "target_id": "b"}\n')
            handle.write('{"sample_rank": 0, "target_id": "a"}\n')
            handle.flush()
            self.assertEqual([row["target_id"] for row in cs.load_sample(handle.name)], ["a", "b"])

    def test_sample_loader_rejects_invalid_row_schema(self):
        cases = [
            "[]\n",
            '{"target_id": "a"}\n',
            '{"sample_rank": true, "target_id": "a"}\n',
            '{"sample_rank": 0, "target_id": ""}\n',
        ]
        for content in cases:
            with self.subTest(content=content):
                with tempfile.NamedTemporaryFile("w", encoding="utf-8") as handle:
                    handle.write(content)
                    handle.flush()
                    with self.assertRaises(ValueError):
                        cs.load_sample(handle.name)

    def test_sample_loader_rejects_duplicate_or_non_contiguous_rows(self):
        cases = [
            '{"sample_rank": 0, "target_id": "a"}\n{"sample_rank": 0, "target_id": "b"}\n',
            '{"sample_rank": 0, "target_id": "a"}\n{"sample_rank": 1, "target_id": "a"}\n',
            '{"sample_rank": 0, "target_id": "a"}\n{"sample_rank": 2, "target_id": "b"}\n',
        ]
        for content in cases:
            with self.subTest(content=content):
                with tempfile.NamedTemporaryFile("w", encoding="utf-8") as handle:
                    handle.write(content)
                    handle.flush()
                    with self.assertRaisesRegex(ValueError, "duplicate|contiguous"):
                        cs.load_sample(handle.name)

    def test_sample_loader_rejects_negative_size(self):
        with tempfile.NamedTemporaryFile("w", encoding="utf-8") as handle:
            handle.write('{"sample_rank": 0, "target_id": "a"}\n')
            handle.flush()
            with self.assertRaisesRegex(ValueError, "non-negative"):
                cs.load_sample(handle.name, -1)

    def test_write_text_atomic_round_trips_without_temp_file(self):
        with tempfile.TemporaryDirectory() as directory:
            path = os.path.join(directory, "sample.jsonl")
            cs.write_text_atomic(path, "first\nsecond\n")
            with open(path, encoding="utf-8") as handle:
                self.assertEqual(handle.read(), "first\nsecond\n")
            self.assertEqual(
                [name for name in os.listdir(directory) if name.endswith(".tmp")], []
            )


if __name__ == "__main__":
    unittest.main()
