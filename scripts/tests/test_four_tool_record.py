import json
import unittest

from scripts.four_tool_record import FourToolRecord, FourToolRecordError, validate_unique_key


def completed(**overrides):
    values = {
        "target_id": "t0",
        "target_smiles": "CCO",
        "sample_rank": 0,
        "tool": "synplanner",
        "arm_id": "synplanner-1.6.0-shared-stock",
        "run_status": "completed",
        "route_found": True,
    }
    values.update(overrides)
    return FourToolRecord(**values)


class FourToolRecordTests(unittest.TestCase):
    def test_serializes_stable_schema(self):
        record = completed(
            common_route_parseable=True,
            strict_route_to_shared_stock=True,
            tool_specific={"synplanner": {"route_export_schema": "synplan-routes/1"}},
        )
        decoded = json.loads(record.to_json_line())
        self.assertEqual(decoded["schema_version"], "renkin-four-tool-row/1")
        self.assertEqual(decoded["tool_specific"]["synplanner"]["route_export_schema"], "synplan-routes/1")

    def test_separates_native_and_common_failure(self):
        record = completed(common_route_parseable=True, strict_route_to_shared_stock=False)
        self.assertTrue(record.route_found)
        self.assertFalse(record.strict_route_to_shared_stock)

    def test_rejects_route_without_native_success(self):
        with self.assertRaises(FourToolRecordError):
            completed(route_found=False, common_route_parseable=True)

    def test_requires_reason_for_terminal_noncompletion(self):
        with self.assertRaises(FourToolRecordError):
            FourToolRecord(
                target_id="t0",
                target_smiles="CCO",
                sample_rank=0,
                tool="renkin",
                arm_id="renkin-arm",
                run_status="timeout",
            )

    def test_load_rejects_duplicate_target_tool_arm(self):
        seen = set()
        key = ("t0", "synplanner", "synplanner-1.6.0-shared-stock")
        validate_unique_key(key, seen)
        with self.assertRaises(FourToolRecordError):
            validate_unique_key(key, seen)


if __name__ == "__main__":
    unittest.main()
