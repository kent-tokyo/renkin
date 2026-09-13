import json
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import audit_phase55_cohort_provenance as audit  # noqa: E402


class TestAuditPhase55CohortProvenance(unittest.TestCase):
    def _write_json(self, value, suffix=".json"):
        handle = tempfile.NamedTemporaryFile(mode="w", suffix=suffix, delete=False, encoding="utf-8")
        json.dump(value, handle)
        handle.close()
        self.addCleanup(os.unlink, handle.name)
        return handle.name

    def _write_jsonl(self, rows):
        handle = tempfile.NamedTemporaryFile(mode="w", suffix=".jsonl", delete=False, encoding="utf-8")
        for row in rows:
            handle.write(json.dumps(row) + "\n")
        handle.close()
        self.addCleanup(os.unlink, handle.name)
        return handle.name

    def _manifest(self):
        return {
            "independence_status": "pending_provenance_audit",
            "targets": [
                {"target_id": "target-a", "canonical_smiles": "CCO"},
                {"target_id": "target-b", "canonical_smiles": "CCN"},
            ],
        }

    def test_split_overlap_is_reported_and_fail_closed(self):
        manifest = self._write_json(self._manifest())
        split = self._write_jsonl([{"id": "target-a", "product": "CCO"}])
        result = audit.audit_cohort(manifest, [split])
        self.assertFalse(result["eligible"])
        self.assertIn("split_overlap", result["blockers"])

    def test_no_observed_overlap_still_requires_provenance_audit(self):
        manifest = self._write_json(self._manifest())
        split = self._write_jsonl([{"id": "other", "product": "CCC"}])
        result = audit.audit_cohort(manifest, [split])
        self.assertFalse(result["eligible"])
        self.assertEqual(result["overlaps"], [])
        self.assertIn("unverified_training_and_template_provenance", result["blockers"])

    def test_matching_source_and_train_evidence_can_pass(self):
        manifest_value = self._manifest()
        manifest_value["source"] = {"sha256": "list-hash"}
        manifest = self._write_json(manifest_value)
        split = self._write_jsonl([{"id": "other", "product": "CCC"}])
        source_evidence = self._write_json(
            {"ordered_list_sha256": "list-hash", "source": {"split": "test"}}
        )
        train_evidence = self._write_json(
            {"source": {"split": "train"}, "model": "frozen"}
        )
        result = audit.audit_cohort(
            manifest,
            [split],
            source_provenance_paths=[source_evidence],
            training_provenance_paths=[train_evidence],
        )
        self.assertTrue(result["eligible"])
        self.assertEqual(result["provenance_status"], "audited")

    def test_manifest_duplicate_is_rejected(self):
        value = self._manifest()
        value["targets"].append({"target_id": "target-c", "canonical_smiles": "CCO"})
        manifest = self._write_json(value)
        split = self._write_jsonl([])
        with self.assertRaisesRegex(ValueError, "duplicate"):
            audit.audit_cohort(manifest, [split])

    def test_scaffold_key_is_deterministic_for_acyclic_molecules(self):
        self.assertEqual(audit._scaffold_key("CCO"), "<acyclic>")


if __name__ == "__main__":
    unittest.main()
