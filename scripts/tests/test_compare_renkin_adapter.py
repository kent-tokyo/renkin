import os
import sys
import unittest
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import compare_renkin_adapter as adapter  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
RENKIN_BIN = REPO_ROOT / "target" / "release" / "renkin"
BUILDING_BLOCKS = REPO_ROOT / "data" / "building_blocks.smi"
TEMPLATES = REPO_ROOT / "data" / "templates_extracted_500.smi"

requires_renkin_bin = unittest.skipUnless(
    RENKIN_BIN.exists(),
    f"requires a built renkin binary at {RENKIN_BIN} (cargo build --release --bin renkin)",
)

ASPIRIN = "CC(=O)Oc1ccccc1C(=O)O"


def load_stock():
    stock = []
    with open(BUILDING_BLOCKS, "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if line and not line.startswith("#"):
                stock.append(line.split()[0])
    return stock


@requires_renkin_bin
class TestRenkinAdapterSmoke(unittest.TestCase):
    def setUp(self):
        self.version = adapter.resolve_tool_version(str(REPO_ROOT))
        self.stock = load_stock()
        self.base_config = adapter.RenkinConfig(
            binary_path=str(RENKIN_BIN),
            building_blocks_path=str(BUILDING_BLOCKS),
            templates_path=str(TEMPLATES),
            depth=5,
            beam_width=100,
            max_routes=1,
            external_timeout_s=30,
        )

    def _run(self, target, config=None, target_id="smoke#test"):
        return adapter.run_one_target(
            target, target_id, 0, config or self.base_config, "native", "cfg1", self.version, self.stock
        )

    def test_route_found_case_fully_populated_and_valid(self):
        row = self._run(ASPIRIN)
        self.assertEqual(row.run_status, "completed")
        self.assertTrue(row.route_found)
        self.assertTrue(row.route_tree_parseable)
        self.assertTrue(row.reaction_steps_parseable)
        self.assertTrue(row.all_leaves_in_configured_stock)
        self.assertEqual(row.target_element_accounting_status, "accounted")
        self.assertIsNotNone(row.normalized_route_sha256)
        self.assertIsNotNone(row.raw_output_sha256)
        self.assertGreater(row.total_elapsed_ms, 0)

    def test_reranker_failures_is_captured_in_tool_specific(self):
        model = REPO_ROOT / "data" / "phase3e_reranker_training" / "model.txt"
        freq_table = REPO_ROOT / "data" / "phase3e_reranker_training" / "frequency_table.json"
        if not (model.exists() and freq_table.exists()):
            self.skipTest(f"requires frozen reranker artifacts at {model} / {freq_table}")
        config = adapter.RenkinConfig(
            binary_path=str(RENKIN_BIN),
            building_blocks_path=str(BUILDING_BLOCKS),
            templates_path=str(TEMPLATES),
            depth=5,
            beam_width=100,
            max_routes=1,
            external_timeout_s=30,
            reranker_model=str(model),
            reranker_freq_table=str(freq_table),
        )
        row = self._run(ASPIRIN, config=config)
        self.assertEqual(row.run_status, "completed")
        self.assertIn("reranker_failures", row.tool_specific["renkin"])
        self.assertEqual(row.tool_specific["renkin"]["reranker_failures"], 0)

    def test_no_route_case_has_diagnostics_and_null_route_fields(self):
        config = adapter.RenkinConfig(
            binary_path=str(RENKIN_BIN),
            building_blocks_path=str(BUILDING_BLOCKS),
            templates_path=str(TEMPLATES),
            depth=0,
            beam_width=100,
            max_routes=1,
            external_timeout_s=30,
        )
        row = self._run(ASPIRIN, config=config)
        self.assertEqual(row.run_status, "completed")
        self.assertFalse(row.route_found)
        self.assertIsNone(row.best_route_depth)
        self.assertIsNone(row.route_tree_parseable)
        self.assertIn("renkin", row.tool_specific)

    def test_timeout_enforced_and_reported(self):
        config = adapter.RenkinConfig(
            binary_path=str(RENKIN_BIN),
            building_blocks_path=str(BUILDING_BLOCKS),
            templates_path=str(TEMPLATES),
            depth=5,
            beam_width=100,
            max_routes=1,
            external_timeout_s=0.0005,
            grace_s=1.0,
        )
        row = self._run(ASPIRIN, config=config)
        self.assertEqual(row.run_status, "timeout")
        self.assertIsNone(row.route_found)

    def test_crash_converts_to_structured_status(self):
        config = adapter.RenkinConfig(
            binary_path=str(RENKIN_BIN),
            building_blocks_path=str(REPO_ROOT / "data" / "does_not_exist.smi"),
            templates_path=str(TEMPLATES),
            depth=5,
            beam_width=100,
            max_routes=1,
            external_timeout_s=30,
        )
        row = self._run(ASPIRIN, config=config)
        self.assertEqual(row.run_status, "crashed")
        self.assertTrue(any(w["code"] == "renkin_nonzero_exit" for w in row.adapter_warnings))

    def test_same_input_twice_gives_consistent_status_and_route_hash(self):
        # RENKIN's search is deterministic (no RNG/seed) -- byte-identical
        # route hash across repeated runs is a real, meaningful invariant,
        # not a soft check (contrast with AiZynthFinder's MCTS, which is
        # only checked for status stability, never route-hash equality).
        row1 = self._run(ASPIRIN)
        row2 = self._run(ASPIRIN)
        self.assertEqual(row1.run_status, row2.run_status)
        self.assertEqual(row1.normalized_route_sha256, row2.normalized_route_sha256)

    def test_malformed_stock_leaf_when_shared_stock_missing_entries(self):
        # A shared_stock-style run where the configured stock doesn't
        # actually cover a leaf the route needs -- all_leaves_in_configured_stock
        # must go false, not silently pass.
        row = adapter.run_one_target(
            ASPIRIN, "smoke#partial_stock", 0, self.base_config, "shared_stock", "cfg2",
            self.version, ["CCO"],  # deliberately missing salicylic acid / acetic anhydride
        )
        self.assertTrue(row.route_found)
        self.assertFalse(row.all_leaves_in_configured_stock)


if __name__ == "__main__":
    unittest.main()
