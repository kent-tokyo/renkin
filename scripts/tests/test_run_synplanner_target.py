import argparse
import unittest
from pathlib import Path

from scripts.run_synplanner_target import build_command


class SynPlannerRunnerTests(unittest.TestCase):
    def test_command_declares_all_frozen_inputs_and_export(self):
        args = argparse.Namespace(
            synplan="/venv/bin/synplan", config="config.yaml",
            reaction_rules="rules.tsv", building_blocks="stock.smi",
            policy_network="policy.pt", value_network="value.pt",
        )
        command = build_command(args, Path("target.smi"), Path("results"))
        self.assertEqual(command[0:2], ["/venv/bin/synplan", "planning"])
        self.assertIn("--export_routes", command)
        for value in ("config.yaml", "rules.tsv", "stock.smi", "policy.pt", "value.pt"):
            self.assertIn(value, command)


if __name__ == "__main__":
    unittest.main()
