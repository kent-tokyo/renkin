import argparse
import unittest
from pathlib import Path

from scripts.run_external_targets import command_for
from scripts.run_external_targets import container_command
from scripts.run_external_targets import parse_docker_memory_usage


TARGET = {"target_id": "t0", "canonical_smiles": "CCO", "sample_rank": 0}


class ExternalOrchestratorTests(unittest.TestCase):
    def test_synplanner_command_uses_frozen_target_and_all_artifacts(self):
        args = argparse.Namespace(
            tool="synplanner", stock="stock.smi", renkin="renkin",
            synplan="synplan", config="config.yaml", reaction_rules="rules.tsv",
            building_blocks="bb.tsv", policy_network="policy.ckpt", value_network="value.ckpt",
            timeout_s=150, grace_s=10, sample_size=1,
        )
        command = command_for(args, TARGET, Path("out.jsonl"))
        self.assertIn("t0", command)
        self.assertIn("CCO", command)
        for value in ("config.yaml", "rules.tsv", "bb.tsv", "policy.ckpt", "value.ckpt"):
            self.assertIn(value, command)

    def test_syntheseus_command_is_selectable(self):
        args = argparse.Namespace(tool="syntheseus", stock="stock.smi", renkin="renkin",
                                  python="python", model_dir="model", timeout_s=120)
        command = command_for(args, TARGET, Path("out.jsonl"))
        self.assertIn("run_syntheseus_target.py", command[1])
        self.assertIn("model", command)

    def test_container_command_declares_bounded_mounts(self):
        args = argparse.Namespace(repo_root="/repo", artifact_dir="/artifacts", cpus=8,
                                  memory="6g", container_image="planner:dev")
        command = container_command(args, ["/usr/bin/python", "/repo/scripts/run.py",
                                           "--output", "/artifacts/row.jsonl"],
                                     Path("/artifacts/row.jsonl"))
        self.assertIn("--network", command)
        self.assertIn("none", command)
        self.assertIn("--memory", command)
        self.assertIn("planner:dev", command)
        self.assertEqual(command[-2:], ["--output", "/artifacts/row.jsonl"])

    def test_docker_memory_usage_parser_handles_binary_units(self):
        self.assertEqual(parse_docker_memory_usage("12.5MiB / 6GiB"), 13_107_200)
        self.assertEqual(parse_docker_memory_usage("512kB / 6GB"), 512_000)
        self.assertIsNone(parse_docker_memory_usage("not available"))

    def test_container_command_can_name_container_for_stats_sampling(self):
        args = argparse.Namespace(repo_root="/repo", artifact_dir="/artifacts", cpus=8,
                                  memory="6g", container_image="planner:dev")
        command = container_command(args, ["python", "/repo/scripts/run.py"],
                                     Path("/artifacts/row.jsonl"), "renkin-synplanner-1")
        self.assertIn("--name", command)
        self.assertIn("renkin-synplanner-1", command)


if __name__ == "__main__":
    unittest.main()
