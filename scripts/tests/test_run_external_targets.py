import argparse
import unittest
from pathlib import Path

from scripts.run_external_targets import command_for


TARGET = {"target_id": "t0", "canonical_smiles": "CCO", "sample_rank": 0}


class ExternalOrchestratorTests(unittest.TestCase):
    def test_synplanner_command_uses_frozen_target_and_all_artifacts(self):
        args = argparse.Namespace(
            tool="synplanner", stock="stock.smi", renkin="renkin",
            synplan="synplan", config="config.yaml", reaction_rules="rules.tsv",
            building_blocks="bb.tsv", policy_network="policy.ckpt", value_network="value.ckpt",
            timeout_s=150, grace_s=10,
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


if __name__ == "__main__":
    unittest.main()
