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


if __name__ == "__main__":
    unittest.main()
