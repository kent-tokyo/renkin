import json
import hashlib
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import freeze_phase55_cohort as freeze  # noqa: E402


class TestFreezePhase55Cohort(unittest.TestCase):
    def _write(self, value):
        handle = tempfile.NamedTemporaryFile(mode="w", suffix=".json", delete=False, encoding="utf-8")
        json.dump(value, handle)
        handle.close()
        self.addCleanup(os.unlink, handle.name)
        return handle.name

    def _candidate(self):
        target = {
            "sample_rank": 0,
            "sample_key": "a" * 64,
            "target_id": "a",
            "canonical_smiles": "CCO",
        }
        target_text = json.dumps(target, sort_keys=True, separators=(",", ":")) + "\n"
        return {
            "protocol_version": "phase55-test",
            "source": {"sha256": "source-hash"},
            "cohort_size": 1,
            "cohort_targets_sha256": hashlib.sha256(target_text.encode()).hexdigest(),
            "targets": [target],
        }

    def test_freezes_only_matching_eligible_audit(self):
        candidate_path = self._write(self._candidate())
        import hashlib

        with open(candidate_path, "rb") as handle:
            candidate_hash = hashlib.sha256(handle.read()).hexdigest()
        audit_path = self._write(
            {"eligible": True, "blockers": [], "manifest": {"sha256": candidate_hash}}
        )
        output = os.path.join(tempfile.mkdtemp(), "frozen.json")
        frozen = freeze.freeze_candidate(candidate_path, audit_path, output, "phase55-test-001")
        self.assertEqual(frozen["freeze_status"], "frozen")
        self.assertEqual(frozen["targets"][0]["target_id"], "a")

    def test_rejects_ineligible_audit(self):
        candidate_path = self._write(self._candidate())
        audit_path = self._write({"eligible": False, "blockers": ["overlap"]})
        output = os.path.join(tempfile.mkdtemp(), "frozen.json")
        with self.assertRaisesRegex(ValueError, "not eligible"):
            freeze.freeze_candidate(candidate_path, audit_path, output, "phase55-test-001")

    def test_refuses_overwrite(self):
        candidate_path = self._write(self._candidate())
        import hashlib

        with open(candidate_path, "rb") as handle:
            candidate_hash = hashlib.sha256(handle.read()).hexdigest()
        audit_path = self._write(
            {"eligible": True, "blockers": [], "manifest": {"sha256": candidate_hash}}
        )
        output = self._write({"already": "frozen"})
        with self.assertRaises(FileExistsError):
            freeze.freeze_candidate(candidate_path, audit_path, output, "phase55-test-001")

    def test_verifies_saved_manifest(self):
        candidate_path = self._write(self._candidate())

        with open(candidate_path, "rb") as handle:
            candidate_hash = hashlib.sha256(handle.read()).hexdigest()
        audit_path = self._write(
            {"eligible": True, "blockers": [], "manifest": {"sha256": candidate_hash}}
        )
        output = os.path.join(tempfile.mkdtemp(), "frozen.json")
        freeze.freeze_candidate(candidate_path, audit_path, output, "phase55-test-001")
        result = freeze.verify_frozen_manifest(output)
        self.assertTrue(result["eligible"])
        self.assertEqual(result["target_count"], 1)

    def test_freeze_records_and_verifies_sample_list(self):
        candidate_path = self._write(self._candidate())
        with open(candidate_path, "rb") as handle:
            candidate_hash = hashlib.sha256(handle.read()).hexdigest()
        audit_path = self._write(
            {"eligible": True, "blockers": [], "manifest": {"sha256": candidate_hash}}
        )
        directory = tempfile.mkdtemp()
        output = os.path.join(directory, "frozen.json")
        sample_output = os.path.join(directory, "sample.jsonl")
        freeze.freeze_candidate(
            candidate_path,
            audit_path,
            output,
            "phase55-test-002",
            sample_list_output=sample_output,
        )
        with open(output, encoding="utf-8") as handle:
            saved = json.load(handle)
        self.assertEqual(saved["sample_list"]["rows"], 1)
        self.assertTrue(freeze.verify_frozen_manifest(output)["eligible"])


if __name__ == "__main__":
    unittest.main()
