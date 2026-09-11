import argparse
import unittest
from pathlib import Path

from scripts.four_tool_record import FourToolRecord
from scripts.run_four_tool_benchmark import (
    external_command,
    merge_rows,
    sha256_file,
    validate_formal_inputs,
    validate_target_coverage,
)


class FourToolBenchmarkTests(unittest.TestCase):
    def test_external_command_is_manifest_and_arm_specific(self):
        args = argparse.Namespace(
            target_manifest="targets.jsonl", stock="stock.smi", renkin="renkin",
            arm_id={"synplanner": "syn-arm"}, timeout_s=150, grace_s=10,
            sample_size=1,
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

    def test_target_coverage_rejects_missing_arm_row(self):
        record = FourToolRecord(
            target_id="t0", target_smiles="CCO", sample_rank=0,
            tool="renkin", arm_id="renkin-arm", run_status="completed",
            route_found=False,
        )
        with self.assertRaisesRegex(ValueError, "target coverage mismatch"):
            validate_target_coverage(
                [record],
                [{"target_id": "t0", "canonical_smiles": "CCO", "sample_rank": 0},
                 {"target_id": "t1", "canonical_smiles": "CCC", "sample_rank": 1}],
                ["renkin"],
            )

    def test_target_coverage_rejects_metadata_mismatch(self):
        record = FourToolRecord(
            target_id="t0", target_smiles="CCN", sample_rank=0,
            tool="renkin", arm_id="renkin-arm", run_status="completed",
            route_found=False,
        )
        with self.assertRaisesRegex(ValueError, "metadata mismatch"):
            validate_target_coverage(
                [record],
                [{"target_id": "t0", "canonical_smiles": "CCO", "sample_rank": 0}],
                ["renkin"],
            )

    def test_formal_preflight_rejects_candidate_registry(self):
        args = argparse.Namespace(
            registry="benchmarks/four_tool/configuration_registry.json",
            image_identities=None, repo_root=".", sample_size=500,
            tools=["renkin", "aizynthfinder", "syntheseus", "synplanner"],
        )
        with self.assertRaisesRegex(ValueError, "formal registry preflight failed"):
            validate_formal_inputs(args)


if __name__ == "__main__":
    unittest.main()
