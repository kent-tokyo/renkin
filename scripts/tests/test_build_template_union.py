import sys
import unittest
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import build_template_union as btu  # noqa: E402


class BuildTemplateUnionTests(unittest.TestCase):
    def test_filters_additional_and_preserves_base_order(self):
        rows, accounting = btu.build(
            [("A>>B", 2), ("C>>D", 4)],
            [("E>>F", 24), ("G>>H", 25)],
            25,
        )
        self.assertEqual(rows, [("A>>B", 2), ("C>>D", 4), ("G>>H", 25)])
        self.assertEqual(accounting["additional_unique_added_count"], 1)

    def test_overlap_uses_max_count_without_reordering(self):
        rows, accounting = btu.build(
            [("A>>B", 2), ("C>>D", 4)],
            [("A>>B", 7), ("E>>F", 8)],
            1,
        )
        self.assertEqual(rows, [("A>>B", 7), ("C>>D", 4), ("E>>F", 8)])
        self.assertEqual(accounting["additional_overlap_count"], 1)

    def test_non_positive_threshold_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "must be positive"):
            btu.build([], [], 0)


if __name__ == "__main__":
    unittest.main()
