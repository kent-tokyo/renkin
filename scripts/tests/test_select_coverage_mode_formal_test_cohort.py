import json
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import select_coverage_mode_formal_test_cohort as cohort  # noqa: E402


@unittest.skipUnless(
    cohort.HAVE_RDKIT, "requires rdkit (see scripts/requirements-compare-66.txt)"
)
class TestSelectCoverageModeFormalTestCohort(unittest.TestCase):
    def _write_groups(self, rows):
        f = tempfile.NamedTemporaryFile(
            mode="w", suffix=".jsonl", delete=False, encoding="utf-8"
        )
        for r in rows:
            f.write(json.dumps(r) + "\n")
        f.close()
        self.addCleanup(os.unlink, f.name)
        return f.name

    def _rows(self, n):
        # Distinct benzene-ring-substituted SMILES, all RDKit-parseable.
        return [
            {"group_id": f"uspto50k_test#L{i}", "target_id": f"c1ccccc1{'C' * i}"}
            for i in range(1, n + 1)
        ]

    def test_deterministic_across_runs(self):
        path = self._write_groups(self._rows(20))
        m1 = cohort.build_cohort(path, cohort_size=5)
        m2 = cohort.build_cohort(path, cohort_size=5)
        self.assertEqual(m1["cohort_targets_sha256"], m2["cohort_targets_sha256"])
        self.assertEqual(
            [t["group_id"] for t in m1["targets"]],
            [t["group_id"] for t in m2["targets"]],
        )

    def test_selection_is_a_stable_prefix(self):
        path = self._write_groups(self._rows(20))
        small = cohort.build_cohort(path, cohort_size=5)
        large = cohort.build_cohort(path, cohort_size=10)
        self.assertEqual(
            [t["group_id"] for t in small["targets"]],
            [t["group_id"] for t in large["targets"]][:5],
        )

    def test_unparseable_smiles_excluded_and_reported(self):
        rows = self._rows(5)
        rows.append({"group_id": "uspto50k_test#Lbad", "target_id": "not_a_smiles((("})
        path = self._write_groups(rows)
        m = cohort.build_cohort(path, cohort_size=5)
        self.assertEqual(m["source_unparseable_by_rdkit"], ["uspto50k_test#Lbad"])
        self.assertEqual(len(m["targets"]), 5)

    def test_raises_if_cohort_size_exceeds_available(self):
        path = self._write_groups(self._rows(3))
        with self.assertRaises(ValueError):
            cohort.build_cohort(path, cohort_size=5)

    def test_different_protocol_version_gives_different_order(self):
        path = self._write_groups(self._rows(20))
        original = cohort.PROTOCOL_VERSION
        try:
            m1 = cohort.build_cohort(path, cohort_size=20)
            cohort.PROTOCOL_VERSION = "a-different-protocol-v1"
            m2 = cohort.build_cohort(path, cohort_size=20)
        finally:
            cohort.PROTOCOL_VERSION = original
        self.assertNotEqual(
            [t["group_id"] for t in m1["targets"]],
            [t["group_id"] for t in m2["targets"]],
        )


if __name__ == "__main__":
    unittest.main()
