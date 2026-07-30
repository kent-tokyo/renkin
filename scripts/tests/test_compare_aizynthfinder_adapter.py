import os
import subprocess
import sys
import unittest
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import compare_aizynthfinder_adapter as adapter  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
PUBLIC_DATA_DIR = REPO_ROOT / "data" / "comparison" / "aizynthfinder_public_data"
IMAGE = "renkin-compare-66/aizynthfinder:4.4.1"


def _docker_image_exists(image: str) -> bool:
    try:
        result = subprocess.run(
            ["docker", "image", "inspect", image], capture_output=True, timeout=10
        )
        return result.returncode == 0
    except Exception:
        return False


requires_aizynthfinder_stack = unittest.skipUnless(
    PUBLIC_DATA_DIR.exists() and _docker_image_exists(IMAGE),
    f"requires the built {IMAGE} image and downloaded public data at {PUBLIC_DATA_DIR} "
    "(see docs/guides/open-source-retrosynthesis-comparison.md, 'Reproduction')",
)

ACETANILIDE = "CC(=O)Nc1ccccc1"  # a trivially-solved target under default settings


@requires_aizynthfinder_stack
class TestAizynthfinderAdapterRealContainer(unittest.TestCase):
    def setUp(self):
        self.config = adapter.AizynthfinderConfig(
            image=IMAGE,
            public_data_dir=str(PUBLIC_DATA_DIR),
            external_timeout_s=180,
            grace_s=15,
        )

    def test_output_envelope_is_pandas_table_schema_not_bare_records(self):
        # Regression test for a real bug: aizynthcli's --output JSON is
        # {"schema": {...}, "data": [<record>]}, NOT a bare list of records
        # and NOT a bare per-target dict. A naive `parsed[0]` or `parsed`
        # read silently produced an empty "trees" lookup, making every row
        # report route_found=False regardless of the tool's real answer.
        row = adapter.run_one_target(
            ACETANILIDE, "test#acetanilide", 0, self.config, "native", "cfg1", "4.4.1", []
        )
        self.assertEqual(row.run_status, "completed")
        self.assertTrue(row.route_found)
        self.assertGreater(row.tool_reported_route_count, 0)

    def test_route_found_reflects_is_solved_not_nonempty_trees(self):
        # AiZynthFinder returns best-effort candidate trees even when
        # is_solved=False -- route_found must track is_solved, never
        # "trees list is non-empty".
        row = adapter.run_one_target(
            ACETANILIDE, "test#acetanilide", 0, self.config, "native", "cfg1", "4.4.1", []
        )
        self.assertIsInstance(row.route_found, bool)
        if row.route_found:
            self.assertGreater(row.tool_specific["aizynthfinder"]["number_of_solved_routes"], 0)

    def test_solved_route_is_structurally_valid(self):
        row = adapter.run_one_target(
            ACETANILIDE, "test#acetanilide", 0, self.config, "native", "cfg1", "4.4.1", []
        )
        self.assertTrue(row.route_found)
        self.assertTrue(row.route_tree_parseable)
        self.assertTrue(row.reaction_steps_parseable)
        self.assertIn(row.common_mass_conservation_status, ("balanced", "imbalanced"))
        self.assertIsNotNone(row.normalized_route_sha256)

    def test_native_mode_trusts_tool_stock_claim_with_disclosed_warning(self):
        # Native mode's real stock is ~17.4M ZINC compounds -- too large to
        # independently re-canonicalize this round. Passing an empty
        # configured_stock_smiles list (as compare_run.py does for native
        # mode) must fall back to the tool's own per-leaf claim, with an
        # explicit adapter_warning, never a silent False.
        row = adapter.run_one_target(
            ACETANILIDE, "test#acetanilide", 0, self.config, "native", "cfg1", "4.4.1", []
        )
        self.assertTrue(row.route_found)
        self.assertIsNotNone(row.all_leaves_in_configured_stock)
        self.assertTrue(
            any(
                w["code"] == "native_stock_trusted_not_independently_verified"
                for w in row.adapter_warnings
            )
        )

    def test_timeout_enforced_via_docker_kill(self):
        config = adapter.AizynthfinderConfig(
            image=IMAGE,
            public_data_dir=str(PUBLIC_DATA_DIR),
            external_timeout_s=1.5,
            grace_s=3,
        )
        row = adapter.run_one_target(
            ACETANILIDE, "test#timeout", 0, config, "native", "cfg1", "4.4.1", []
        )
        self.assertEqual(row.run_status, "timeout")
        self.assertIsNone(row.route_found)

    def test_crash_on_missing_public_data_converts_to_structured_status(self):
        config = adapter.AizynthfinderConfig(
            image=IMAGE,
            public_data_dir=str(REPO_ROOT / "data" / "does_not_exist_dir"),
            external_timeout_s=60,
            grace_s=10,
        )
        row = adapter.run_one_target(
            ACETANILIDE, "test#crash", 0, config, "native", "cfg1", "4.4.1", []
        )
        self.assertIn(row.run_status, ("crashed", "invalid_input"))


if __name__ == "__main__":
    unittest.main()
