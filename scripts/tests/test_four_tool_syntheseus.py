import json
import unittest
from pathlib import Path

from scripts.four_tool_syntheseus import load_route, record_for_route


FIXTURE = Path(__file__).parents[2] / "tests/fixtures/syntheseus/0.8.0/linear_two_leaf_route.json"


class SyntheseusAdapterTests(unittest.TestCase):
    def test_loads_public_route_artifact(self):
        route = load_route(FIXTURE)
        self.assertEqual(route["source_tool"], "syntheseus")
        self.assertEqual(route["schema_version"], 1)
        self.assertGreaterEqual(len(route["steps"]), 1)

    def test_record_separates_common_audit(self):
        route = load_route(FIXTURE)
        record = record_for_route(
            route, target_id="t0", sample_rank=0,
            arm_id="syntheseus-0.8.0-localretro-retrostar-shared-stock",
            raw_output_sha256="x", audit={"status": "partial"},
        )
        decoded = json.loads(record.to_json_line())
        self.assertTrue(decoded["route_found"])
        self.assertTrue(decoded["common_route_parseable"])
        self.assertFalse(decoded["strict_route_to_shared_stock"])
        self.assertEqual(decoded["tool_specific"]["syntheseus"]["audit_rank"], 1)


if __name__ == "__main__":
    unittest.main()
