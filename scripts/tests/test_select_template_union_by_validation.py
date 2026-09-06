import sys
import unittest
from collections import OrderedDict
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import select_template_union_by_validation as selector  # noqa: E402


def candidate(group, candidate, precursors, sources):
    return {
        "group_id": group,
        "candidate_id": candidate,
        "precursor_smiles": precursors,
        "sources": [{"template_id": source} for source in sources],
    }


class ValidationSelectionTests(unittest.TestCase):
    def setUp(self):
        self.base = OrderedDict([("base", ("A>>B", 10))])
        self.additional = OrderedDict(
            [
                ("wide", ("C>>D", 2)),
                ("narrow", ("E>>F", 8)),
                ("no_gain", ("G>>H", 99)),
            ]
        )
        self.labels = {
            "val#1": {("P1",)},
            "val#2": {("P2",)},
            "val#3": {("P3",)},
        }
        self.base_pool = [candidate("val#1", "b1", ["P1"], ["base"])]
        self.full_pool = [
            candidate("val#1", "b1", ["P1"], ["base"]),
            candidate("val#2", "w2", ["P2"], ["wide"]),
            candidate("val#3", "w3", ["P3"], ["wide"]),
            candidate("val#2", "n2", ["P2"], ["narrow"]),
            candidate("val#1", "junk", ["J"], ["no_gain"]),
        ]

    def test_greedy_prefers_more_new_positive_groups(self):
        selected, accounting = selector.select(
            self.base,
            self.additional,
            self.base_pool,
            self.full_pool,
            self.labels,
            3.0,
        )
        self.assertEqual(selected, ["wide"])
        self.assertEqual(accounting["new_positive_group_count"], 2)
        self.assertEqual(accounting["selected_candidate_count"], 3)

    def test_candidate_budget_is_fail_closed(self):
        selected, accounting = selector.select(
            self.base,
            self.additional,
            self.base_pool,
            self.full_pool,
            self.labels,
            1.0,
        )
        self.assertEqual(selected, [])
        self.assertEqual(accounting["candidate_growth"], 1.0)

    def test_invalid_growth_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "at least 1.0"):
            selector.select(
                self.base,
                self.additional,
                self.base_pool,
                self.full_pool,
                self.labels,
                0.9,
            )

    def test_group_prefix_rejects_non_validation_input(self):
        with self.assertRaisesRegex(ValueError, "required validation prefix"):
            selector.group_identity(
                [{"group_id": "test#1", "target_id": "T"}], "val#"
            )

    def test_group_identity_retains_status_for_fail_closed_gate(self):
        groups = selector.group_identity(
            [
                {
                    "group_id": "val#1",
                    "target_id": "T",
                    "target_smiles": "T",
                    "proposal_status": "parse_failed",
                }
            ],
            "val#",
        )
        self.assertEqual(groups["val#1"][2], "parse_failed")


if __name__ == "__main__":
    unittest.main()
