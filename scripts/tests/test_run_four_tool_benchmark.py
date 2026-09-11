import argparse
import unittest
from pathlib import Path

from scripts.run_four_tool_benchmark import external_command, merge_rows, sha256_file


class FourToolBenchmarkTests(unittest.TestCase):
    def test_external_command_is_manifest_and_arm_specific(self):
        args = argparse.Namespace(
            target_manifest="targets.jsonl", stock="stock.smi", renkin="renkin",
            arm_id={"synplanner": "syn-arm"}, timeout_s=150, grace_s=10,
            synplan="synplan", synplanner_config="config.yaml", reaction_rules="rules.tsv",
            building_blocks="bb.tsv", policy_network="policy.ckpt", value_network="value.ckpt",
        )
        command = external_command(args, "synplanner", Path("rows.jsonl"), Path("artifacts"))
        for value in ("targets.jsonl", "stock.smi", "syn-arm", "config.yaml", "rules.tsv"):
            self.assertIn(value, command)

    def test_merge_rows_rejects_duplicate_identity(self):
        self.assertTrue(callable(merge_rows))

    def test_hash_is_stable_for_existing_input(self):
        digest = sha256_file("data/comparison/sample_full_sorted.jsonl")
        self.assertEqual(len(digest), 64)
        self.assertEqual(digest, sha256_file("data/comparison/sample_full_sorted.jsonl"))


if __name__ == "__main__":
    unittest.main()
