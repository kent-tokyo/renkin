import unittest

from scripts.convert_existing_rows import convert_row


class LegacyConversionTests(unittest.TestCase):
    def test_completed_route_maps_to_common_record(self):
        record = convert_row({
            "target_id": "t0", "target_smiles": "CCO", "sample_rank": 0,
            "tool": "renkin", "run_status": "completed", "route_found": True,
            "configuration_id": "renkin-arm", "reaction_steps_parseable": True,
            "strict_validated_route_to_configured_stock": True,
            "total_elapsed_ms": 12.5, "peak_rss_bytes": 10,
            "tool_specific": {"route_selection": "rank1"},
            "adapter_warnings": [], "common_validation_warnings": [],
        })
        self.assertTrue(record.strict_route_to_shared_stock)
        self.assertEqual(record.tool_specific["renkin"]["route_selection"], "rank1")

    def test_crash_requires_reason_and_does_not_claim_route(self):
        record = convert_row({
            "target_id": "t0", "target_smiles": "CCO", "sample_rank": 0,
            "tool": "aizynthfinder", "run_status": "crashed", "route_found": None,
            "configuration_id": "aizynth-arm", "adapter_warnings": [],
            "common_validation_warnings": [],
        })
        self.assertIsNone(record.route_found)
        self.assertIn("legacy run", record.failure_reason)


if __name__ == "__main__":
    unittest.main()
