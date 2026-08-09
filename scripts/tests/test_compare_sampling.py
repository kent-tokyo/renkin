import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import compare_sampling as sampling  # noqa: E402


@unittest.skipUnless(
    sampling.HAVE_RDKIT, "requires rdkit (see scripts/requirements-compare-66.txt)"
)
class TestCompareSampling(unittest.TestCase):
    def _write_corpus(self, lines):
        f = tempfile.NamedTemporaryFile(mode="w", suffix=".smi", delete=False, encoding="utf-8")
        f.write("\n".join(lines) + "\n")
        f.close()
        self.addCleanup(os.unlink, f.name)
        return f.name

    def test_comments_and_blanks_skipped(self):
        corpus = self._write_corpus(
            [
                "# header comment",
                "",
                "CCO\tUNK",
                "c1ccccc1\tUNK",
            ]
        )
        result = sampling.build_sample(corpus)
        self.assertEqual(result.manifest["raw_lines_total"], 4)
        self.assertEqual(result.manifest["comment_or_blank_lines"], 2)
        self.assertEqual(result.manifest["raw_candidate_lines"], 2)
        self.assertEqual(result.manifest["unique_canonical_targets"], 2)

    def test_unparseable_smiles_counted_not_dropped_silently(self):
        corpus = self._write_corpus(["not_a_smiles(((\tUNK", "CCO\tUNK"])
        result = sampling.build_sample(corpus)
        self.assertEqual(result.manifest["unparseable_count"], 1)
        self.assertEqual(result.manifest["unparseable_lines"][0]["line_number"], 1)
        self.assertEqual(result.manifest["unique_canonical_targets"], 1)

    def test_canonical_duplicate_detected_across_different_notations(self):
        # Same molecule (ethanol), two different but chemically-identical SMILES.
        corpus = self._write_corpus(["CCO\tUNK", "OCC\tUNK"])
        result = sampling.build_sample(corpus)
        self.assertEqual(result.manifest["unique_canonical_targets"], 1)
        self.assertEqual(result.manifest["canonical_duplicate_groups"], 1)
        dup = result.manifest["canonical_duplicate_detail"][0]
        self.assertEqual(dup["raw_line_numbers"], [1, 2])
        self.assertEqual(dup["kept_line_number"], 1)  # lowest line number wins

    def test_nesting_property_100_is_prefix_of_500_and_full(self):
        # 120 distinct small molecules so 100 < unique_count.
        smis = [f"C{'C' * i}O" for i in range(120)]
        corpus = self._write_corpus([f"{s}\tUNK" for s in smis])
        result = sampling.build_sample(corpus)
        self.assertEqual(result.manifest["unique_canonical_targets"], 120)

        with tempfile.TemporaryDirectory() as tmp:
            list_path = os.path.join(tmp, "sample.jsonl")
            with open(list_path, "w", encoding="utf-8") as f:
                for row in result.ordered_rows:
                    f.write(__import__("json").dumps(row, sort_keys=True) + "\n")

            sample_100 = sampling.load_sample(list_path, 100)
            sample_full = sampling.load_sample(list_path)
            self.assertEqual(sample_100, sample_full[:100])
            self.assertEqual([r["sample_rank"] for r in sample_100], list(range(100)))

    def test_sample_key_deterministic_across_runs(self):
        corpus = self._write_corpus(["CCO\tUNK", "c1ccccc1\tUNK"])
        result1 = sampling.build_sample(corpus)
        result2 = sampling.build_sample(corpus)
        self.assertEqual(
            result1.manifest["ordered_list_sha256"], result2.manifest["ordered_list_sha256"]
        )
        self.assertEqual(
            [r["sample_key"] for r in result1.ordered_rows],
            [r["sample_key"] for r in result2.ordered_rows],
        )

    def test_sample_key_uses_domain_separator(self):
        canon = sampling.canonical_smiles("CCO")
        key = sampling.sample_key(canon)
        import hashlib

        expected = hashlib.sha256(f"renkin-issue66-sample-v1|{canon}".encode()).hexdigest()
        self.assertEqual(key, expected)


if __name__ == "__main__":
    unittest.main()
