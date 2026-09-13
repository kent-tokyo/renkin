import json
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import prepare_phase55_independent_cohort as cohort  # noqa: E402


class TestPreparePhase55IndependentCohort(unittest.TestCase):
    def _write(self, value, suffix=".jsonl"):
        handle = tempfile.NamedTemporaryFile(mode="w", suffix=suffix, delete=False, encoding="utf-8")
        if isinstance(value, list):
            for row in value:
                handle.write(json.dumps(row) + "\n")
        else:
            json.dump(value, handle)
        handle.close()
        self.addCleanup(os.unlink, handle.name)
        return handle.name

    @staticmethod
    def _source(n=5):
        return [
            {
                "target_id": f"target-{i}",
                "canonical_smiles": f"C{i}CO",
                "source_line_number": i + 1,
            }
            for i in range(n)
        ]

    def test_exclusion_by_id_and_canonical_smiles(self):
        source = self._write(self._source())
        exclusion = self._write(
            [
                {"target_id": "target-0", "canonical_smiles": "C0CO"},
                {"sample_id": "historical", "canonical_smiles": "C1CO"},
            ]
        )
        manifest = cohort.build_cohort(source, [exclusion], cohort_size=3)
        self.assertEqual(manifest["eligible_rows"], 3)
        self.assertEqual(manifest["targets"][0]["source_line_number"] in {3, 4, 5}, True)
        self.assertEqual(
            {row["target_id"] for row in manifest["targets"]},
            {"target-2", "target-3", "target-4"},
        )
        self.assertEqual(manifest["independence_status"], "pending_provenance_audit")

    def test_deterministic_and_order_is_prefix(self):
        source = self._write(self._source(8))
        small = cohort.build_cohort(source, [], cohort_size=3)
        large = cohort.build_cohort(source, [], cohort_size=6)
        self.assertEqual(small["cohort_targets_sha256"], cohort.build_cohort(source, [], 3)["cohort_targets_sha256"])
        self.assertEqual(small["targets"], large["targets"][:3])

    def test_duplicate_source_is_rejected(self):
        source = self._write(
            [
                {"target_id": "a", "canonical_smiles": "CCO"},
                {"target_id": "b", "canonical_smiles": "CCO"},
            ]
        )
        with self.assertRaisesRegex(ValueError, "duplicate canonical"):
            cohort.build_cohort(source, [], cohort_size=1)

    def test_insufficient_eligible_rows_is_rejected(self):
        source = self._write(self._source(2))
        exclusion = self._write([{"target_id": "target-0"}])
        with self.assertRaisesRegex(ValueError, "eligible targets"):
            cohort.build_cohort(source, [exclusion], cohort_size=2)


if __name__ == "__main__":
    unittest.main()
