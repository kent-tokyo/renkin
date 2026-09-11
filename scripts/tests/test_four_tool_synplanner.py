import json
import unittest
from pathlib import Path

from scripts.four_tool_synplanner import load_export, record_for_target


FIXTURE = Path(__file__).parents[2] / "tests/fixtures/synplanner/v1.6.0/real_planning_export.results.json"


class SynPlannerAdapterTests(unittest.TestCase):
    def test_loads_target_keyed_export_and_preserves_order(self):
        export = load_export(FIXTURE)
        self.assertEqual(list(export), ["CC(=O)Oc1ccccc1C(=O)O"])
        self.assertEqual(len(export[next(iter(export))]), 2)

    def test_common_audit_is_rank_one_only(self):
        routes = load_export(FIXTURE)["CC(=O)Oc1ccccc1C(=O)O"]
        record = record_for_target(
            target_id="aspirin", target_smiles="CC(=O)Oc1ccccc1C(=O)O",
            sample_rank=0, arm_id="synplanner-1.6.0-shared-stock", routes=routes,
            audit={"status": "pass"}, raw_output_sha256="x",
        )
        decoded = json.loads(record.to_json_line())
        self.assertTrue(decoded["route_found"])
        self.assertEqual(decoded["tool_reported_route_count"], 2)
        self.assertTrue(decoded["strict_route_to_shared_stock"])
        self.assertEqual(decoded["tool_specific"]["synplanner"]["audit_rank"], 1)

    def test_no_native_routes_have_no_common_claim(self):
        record = record_for_target(
            target_id="t0", target_smiles="CCO", sample_rank=0,
            arm_id="synplanner-1.6.0-shared-stock", routes=[], raw_output_sha256="x",
        )
        self.assertFalse(record.route_found)
        self.assertIsNone(record.common_route_parseable)
        self.assertIsNone(record.strict_route_to_shared_stock)


if __name__ == "__main__":
    unittest.main()
