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

    def test_optional_budget_check_reports_overrun(self):
        rows = [
            {
                "route_found": False,
                "run_status": "completed",
                "tool_specific": {
                    "renkin": {
                        "recovery": {
                            "attempts": [{"routes_found": 0}],
                            "total_elapsed_ms": 1250,
                        }
                    }
                },
            }
        ]
        result = MODULE.summarize(rows, budget_ms=1000)
        self.assertEqual(result["budget_observed_count"], 1)
        self.assertEqual(result["budget_overrun_count"], 1)
        self.assertEqual(result["budget_max_elapsed_ms"], 1250)
        self.assertFalse(result["within_budget"])

    def test_process_budget_is_distinct_from_recovery_budget(self):
        rows = [
            {
                "route_found": False,
                "run_status": "completed",
                "total_elapsed_ms": 1010,
                "tool_specific": {
                    "renkin": {
                        "recovery": {
                            "attempts": [{"routes_found": 0}],
                            "total_elapsed_ms": 990,
                        }
                    }
                },
            }
        ]
        result = MODULE.summarize(rows, budget_ms=1000, process_budget_ms=1000)
        self.assertTrue(result["within_budget"])
        self.assertEqual(result["process_budget_overrun_count"], 1)
        self.assertFalse(result["within_process_budget"])

    def test_compares_strict_success_against_same_cohort_baseline(self):
        baseline = [
            {
                "target_id": "a",
                "route_found": True,
                "all_leaves_in_configured_stock": True,
                "validator_confirmed_route_found": True,
            },
            {"target_id": "b", "route_found": False},
        ]
        candidate = [
            {
                "target_id": "a",
                "route_found": True,
                "all_leaves_in_configured_stock": True,
                "validator_confirmed_route_found": True,
                "run_status": "completed",
                "tool_specific": {"renkin": {"recovery": {"attempts": [{"routes_found": 1}]}}},
            },
            {
                "target_id": "b",
                "route_found": True,
                "all_leaves_in_configured_stock": True,
                "validator_confirmed_route_found": True,
                "run_status": "completed",
                "tool_specific": {"renkin": {"recovery": {"attempts": [{"routes_found": 0}]}}},
            },
        ]
        result = MODULE.summarize(candidate, baseline_rows=baseline)
        self.assertEqual(result["baseline_strict_success_count"], 1)
        self.assertEqual(result["final_strict_success_count"], 2)
        self.assertEqual(result["strict_regression_count"], 0)
        self.assertTrue(result["zero_strict_regression"])

    def test_rejects_mismatched_baseline_target_set(self):
        with self.assertRaisesRegex(ValueError, "target sets differ"):
            MODULE.summarize(
                [{"target_id": "candidate", "tool_specific": {}}],
                baseline_rows=[{"target_id": "baseline"}],
            )


if __name__ == "__main__":
    unittest.main()
