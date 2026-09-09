import importlib.util
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).parents[1] / "verify_non_displacing_recovery.py"
SPEC = importlib.util.spec_from_file_location("recovery_verifier", MODULE_PATH)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class NonDisplacingRecoveryTests(unittest.TestCase):
    def test_counts_recovery_without_regression(self):
        rows = [
            {
                "route_found": True,
                "run_status": "completed",
                "tool_specific": {"renkin": {"recovery": {"attempts": [{"routes_found": 1}]}}},
            },
            {
                "route_found": True,
                "run_status": "completed",
                "tool_specific": {"renkin": {"recovery": {"attempts": [{"routes_found": 0}]}}},
            },
            {
                "route_found": False,
                "run_status": "timeout",
                "tool_specific": {"renkin": {"recovery": {"attempts": []}}},
            },
        ]
        result = MODULE.summarize(rows)
        self.assertEqual(result["baseline_success_count"], 1)
        self.assertEqual(result["final_success_count"], 2)
        self.assertEqual(result["recovered_count"], 1)
        self.assertEqual(result["regression_count"], 0)
        self.assertEqual(result["timeout_count"], 1)
        self.assertEqual(result["missing_attempts_count"], 1)
        self.assertTrue(result["zero_regression"])

    def test_detects_displaced_native_success(self):
        result = MODULE.summarize(
            [
                {
                    "route_found": False,
                    "run_status": "completed",
                    "tool_specific": {
                        "renkin": {"recovery": {"attempts": [{"routes_found": 1}]}}
                    },
                }
            ]
        )
        self.assertEqual(result["regression_count"], 1)
        self.assertFalse(result["zero_regression"])


if __name__ == "__main__":
    unittest.main()
