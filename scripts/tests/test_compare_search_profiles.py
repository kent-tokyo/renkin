import os
import sys
import unittest
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from compare_search_profiles import (  # noqa: E402
    build_compare_command,
    parse_profiles,
)


class TestCompareSearchProfiles(unittest.TestCase):
    def test_profiles_accept_comma_and_space_separated_values(self):
        self.assertEqual(parse_profiles(["fast,balanced", "deep"]), ["fast", "balanced", "deep"])

    def test_profiles_reject_duplicates(self):
        with self.assertRaisesRegex(ValueError, "duplicate profile"):
            parse_profiles(["fast", "fast"])

    def test_command_keeps_profile_and_output_artifacts_distinct(self):
        command = build_compare_command(
            compare_run=Path("scripts/compare_run.py"),
            repo_root=Path("/repo"),
            sample_list="samples.jsonl",
            sample_size=200,
            comparison_mode="native",
            renkin_binary="target/release/renkin",
            building_blocks="data/building_blocks.smi",
            shared_stock_smi="data/shared.smi",
            templates="data/templates.smi",
            timeout_s=150.0,
            grace_s=10.0,
            max_routes=1,
            route_selection="rank1",
            output_rows=Path("out/renkin_balanced.jsonl"),
            output_aggregate=Path("out/renkin_balanced_aggregate.json"),
            output_manifest=Path("out/renkin_balanced_manifest.json"),
            profile="balanced",
        )
        self.assertEqual(command[command.index("--search-profile") + 1], "balanced")
        self.assertTrue(any(value.endswith("renkin_balanced.jsonl") for value in command))
        self.assertTrue(any(value.endswith("renkin_balanced_aggregate.json") for value in command))
        self.assertTrue(any(value.endswith("renkin_balanced_manifest.json") for value in command))


if __name__ == "__main__":
    unittest.main()
