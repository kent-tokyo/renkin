import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from compare_renkin_batch import tool_reported_route_count  # noqa: E402


class TestToolReportedRouteCount(unittest.TestCase):
    def test_solved_row_preserves_tool_count(self):
        self.assertEqual(
            tool_reported_route_count({"solved": True, "routes_found": 3}),
            3,
        )

    def test_unsolved_row_uses_schema_null_not_zero(self):
        self.assertIsNone(
            tool_reported_route_count({"solved": False, "routes_found": 0})
        )


if __name__ == "__main__":
    unittest.main()
