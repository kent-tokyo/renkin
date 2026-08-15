import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import coverage_mode_formal_test_cohort_to_sample_list as convert_mod  # noqa: E402


class ConvertTests(unittest.TestCase):
    def test_shape_and_field_mapping(self):
        manifest = {
            "targets": [
                {
                    "cohort_rank": 0,
                    "group_id": "uspto50k_test#L1",
                    "target_id": "raw smiles here",
                    "canonical_smiles": "CCO",
                    "sample_key": "abc",
                },
                {
                    "cohort_rank": 1,
                    "group_id": "uspto50k_test#L2",
                    "target_id": "raw smiles here 2",
                    "canonical_smiles": "CCC",
                    "sample_key": "def",
                },
            ]
        }
        rows = convert_mod.convert(manifest)
        self.assertEqual(
            rows,
            [
                {"target_id": "uspto50k_test#L1", "canonical_smiles": "CCO", "sample_rank": 0},
                {"target_id": "uspto50k_test#L2", "canonical_smiles": "CCC", "sample_rank": 1},
            ],
        )

    def test_real_committed_cohort_manifest_converts_and_loads(self):
        # Round-trip against the actual committed 500-target manifest --
        # not just a synthetic fixture -- and confirm the result is
        # directly loadable by compare_sampling.load_sample, the real
        # consumer.
        import tempfile

        sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", ".."))
        repo_root = os.path.join(os.path.dirname(__file__), "..", "..")
        manifest_path = os.path.join(
            repo_root, "data", "coverage_mode_formal_test", "cohort_manifest.json"
        )
        if not os.path.exists(manifest_path):
            self.skipTest("cohort_manifest.json not present")

        import json

        with open(manifest_path, encoding="utf-8") as f:
            manifest = json.load(f)
        rows = convert_mod.convert(manifest)
        self.assertEqual(len(rows), manifest["cohort_size"])
        self.assertEqual(len({r["target_id"] for r in rows}), len(rows))
        self.assertEqual(sorted(r["sample_rank"] for r in rows), list(range(len(rows))))

        with tempfile.NamedTemporaryFile(
            mode="w", suffix=".jsonl", delete=False, encoding="utf-8"
        ) as f:
            for row in rows:
                f.write(json.dumps(row) + "\n")
            out_path = f.name
        try:
            import compare_sampling

            loaded = compare_sampling.load_sample(out_path)
            self.assertEqual(len(loaded), len(rows))
        finally:
            os.unlink(out_path)


if __name__ == "__main__":
    unittest.main()
