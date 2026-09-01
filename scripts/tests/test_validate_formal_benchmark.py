import hashlib
import json
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import validate_formal_benchmark as vfb  # noqa: E402


class TestFormalBenchmarkPreflight(unittest.TestCase):
    def make_target_list(self, directory: str, count: int = vfb.FORMAL_TARGET_COUNT) -> str:
        path = os.path.join(directory, "targets.jsonl")
        with open(path, "w", encoding="utf-8") as handle:
            for rank in range(count):
                handle.write(json.dumps({
                    "sample_rank": rank,
                    "target_id": f"target#{rank}",
                    "canonical_smiles": "CCO",
                    "sample_key": hashlib.sha256(str(rank).encode()).hexdigest(),
                }) + "\n")
        return path

    def test_rejects_non_formal_target_count(self):
        with tempfile.TemporaryDirectory() as directory:
            path = self.make_target_list(directory, count=1)
            with self.assertRaisesRegex(ValueError, "exactly 4903"):
                vfb.validate_target_list(path, None)

    def test_rejects_duplicate_ids(self):
        with tempfile.TemporaryDirectory() as directory:
            path = self.make_target_list(directory)
            with open(path, "r+", encoding="utf-8") as handle:
                rows = [json.loads(line) for line in handle]
                rows[1]["target_id"] = rows[0]["target_id"]
                handle.seek(0)
                handle.truncate()
                handle.writelines(json.dumps(row) + "\n" for row in rows)
            with self.assertRaisesRegex(ValueError, "duplicate"):
                vfb.validate_target_list(path, None)

    def test_rejects_hash_mismatch(self):
        with tempfile.TemporaryDirectory() as directory:
            path = self.make_target_list(directory)
            with self.assertRaisesRegex(ValueError, "SHA-256 mismatch"):
                vfb.validate_target_list(path, "0" * 64)


if __name__ == "__main__":
    unittest.main()
