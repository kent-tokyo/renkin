import json
import os
import sys
import tempfile
import unittest
from dataclasses import asdict

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import compare_paired_report as paired_report  # noqa: E402
from compare_schema import PlannerComparisonRow  # noqa: E402


def _row(
    target_id,
    tool,
    route_found,
    total_elapsed_ms=1000.0,
    accounting=None,
    route_tree_parseable=None,
    all_leaves_in_configured_stock=None,
):
    return PlannerComparisonRow(
        target_id=target_id,
        target_smiles="CC",
        sample_rank=0,
        tool=tool,
        tool_version="0.0.0",
        configuration_id=f"{tool}-test",
        comparison_mode="native",
        run_status="completed",
        route_found=route_found,
        total_elapsed_ms=total_elapsed_ms,
        target_element_accounting_status=accounting,
        route_tree_parseable=route_tree_parseable,
        all_leaves_in_configured_stock=all_leaves_in_configured_stock,
    )


class TestJoinRows(unittest.TestCase):
    def test_joins_and_sorts_by_target_id(self):
        renkin = [_row("t2", "renkin", True), _row("t1", "renkin", False)]
        aizynth = [_row("t1", "aizynthfinder", True), _row("t2", "aizynthfinder", False)]
        joined = paired_report.join_rows(renkin, aizynth)
        self.assertEqual([tid for tid, _, _ in joined], ["t1", "t2"])

    def test_mismatched_target_id_sets_raises(self):
        renkin = [_row("t1", "renkin", True)]
        aizynth = [_row("t2", "aizynthfinder", True)]
        with self.assertRaises(ValueError):
            paired_report.join_rows(renkin, aizynth)


class TestComputePairedStatsNative(unittest.TestCase):
    def test_both_solved_n_and_mcnemar(self):
        joined = [
            ("t1", _row("t1", "renkin", True), _row("t1", "aizynthfinder", True)),
            ("t2", _row("t2", "renkin", False), _row("t2", "aizynthfinder", True)),
            ("t3", _row("t3", "renkin", True), _row("t3", "aizynthfinder", False)),
            ("t4", _row("t4", "renkin", False), _row("t4", "aizynthfinder", False)),
        ]
        stats = paired_report.compute_paired_stats(joined, "native")
        self.assertEqual(stats["n_pairs"], 4)
        self.assertEqual(stats["both_solved_n"], 1)
        self.assertEqual(stats["mcnemar"]["renkin_only"], 1)
        self.assertEqual(stats["mcnemar"]["aizynthfinder_only"], 1)
        diff = stats["route_found_rate_diff_renkin_minus_aizynthfinder"]
        self.assertAlmostEqual(diff["observed"], 0.0)  # 2/4 renkin - 2/4 aizynthfinder

    def test_elapsed_diff_only_over_both_solved_pairs(self):
        joined = [
            ("t1", _row("t1", "renkin", True, total_elapsed_ms=100.0),
             _row("t1", "aizynthfinder", True, total_elapsed_ms=300.0)),
            ("t2", _row("t2", "renkin", False, total_elapsed_ms=None),
             _row("t2", "aizynthfinder", True, total_elapsed_ms=500.0)),
        ]
        stats = paired_report.compute_paired_stats(joined, "native")
        diff = stats["total_elapsed_ms_diff_renkin_minus_aizynthfinder_both_solved"]
        self.assertAlmostEqual(diff["observed"], 100.0 - 300.0)

    def test_no_both_solved_pairs_omits_elapsed_diff(self):
        joined = [("t1", _row("t1", "renkin", False), _row("t1", "aizynthfinder", False))]
        stats = paired_report.compute_paired_stats(joined, "native")
        self.assertNotIn("total_elapsed_ms_diff_renkin_minus_aizynthfinder_both_solved", stats)


class TestComputePairedStatsSharedStock(unittest.TestCase):
    def test_primary_metric_uses_route_to_shared_stock_not_route_found(self):
        joined = [
            # route_found True but stock check fails -> route_to_shared_stock False
            ("t1", _row("t1", "renkin", True, route_tree_parseable=True,
                         all_leaves_in_configured_stock=False),
             _row("t1", "aizynthfinder", False)),
            ("t2", _row("t2", "renkin", True, route_tree_parseable=True,
                         all_leaves_in_configured_stock=True),
             _row("t2", "aizynthfinder", True, route_tree_parseable=True,
                  all_leaves_in_configured_stock=True)),
        ]
        stats = paired_report.compute_paired_stats(joined, "shared_stock")
        primary = stats["route_to_shared_stock_rate_diff_renkin_minus_aizynthfinder"]
        secondary = stats["secondary_tool_native_route_found_rate_diff_renkin_minus_aizynthfinder"]
        # primary: renkin 1/2 route_to_shared_stock, aizynthfinder 1/2 -> diff 0
        self.assertAlmostEqual(primary["observed"], 0.0)
        # secondary (tool-native route_found): renkin 2/2, aizynthfinder 1/2 -> diff 0.5
        self.assertAlmostEqual(secondary["observed"], 0.5)
        self.assertNotIn("total_elapsed_ms_diff_renkin_minus_aizynthfinder_both_solved", stats)
        self.assertIn("primary_metric", stats)


class TestComputePairedTable(unittest.TestCase):
    def test_native_table_rows_omit_shared_stock_field(self):
        joined = [
            ("t1", _row("t1", "renkin", True, accounting="accounted"),
             _row("t1", "aizynthfinder", False)),
        ]
        table = paired_report.compute_paired_table(joined, "native")
        self.assertEqual(
            table,
            [
                {
                    "target_id": "t1",
                    "renkin_route_found": True,
                    "aizynthfinder_route_found": False,
                    "renkin_target_element_accounting_status": "accounted",
                    "aizynthfinder_target_element_accounting_status": None,
                }
            ],
        )

    def test_shared_stock_table_rows_include_shared_stock_field(self):
        joined = [
            ("t1", _row("t1", "renkin", True, route_tree_parseable=True,
                         all_leaves_in_configured_stock=True),
             _row("t1", "aizynthfinder", False)),
        ]
        table = paired_report.compute_paired_table(joined, "shared_stock")
        self.assertTrue(table[0]["renkin_route_to_shared_stock"])
        self.assertFalse(table[0]["aizynthfinder_route_to_shared_stock"])


class TestMainCli(unittest.TestCase):
    def test_end_to_end_writes_stats_and_table(self):
        with tempfile.TemporaryDirectory() as tmp:
            renkin_path = os.path.join(tmp, "renkin.jsonl")
            aizynth_path = os.path.join(tmp, "aizynthfinder.jsonl")
            stats_path = os.path.join(tmp, "paired_stats.json")
            table_path = os.path.join(tmp, "paired_table.json")

            with open(renkin_path, "w", encoding="utf-8") as f:
                f.write(json.dumps(asdict(_row("t1", "renkin", True))) + "\n")
                f.write(json.dumps(asdict(_row("t2", "renkin", False))) + "\n")
            with open(aizynth_path, "w", encoding="utf-8") as f:
                f.write(json.dumps(asdict(_row("t1", "aizynthfinder", True))) + "\n")
                f.write(json.dumps(asdict(_row("t2", "aizynthfinder", True))) + "\n")

            rc = paired_report.main(
                [
                    "--renkin-rows", renkin_path,
                    "--aizynthfinder-rows", aizynth_path,
                    "--output-stats", stats_path,
                    "--output-table", table_path,
                ]
            )
            self.assertEqual(rc, 0)

            with open(stats_path, encoding="utf-8") as f:
                stats = json.load(f)
            self.assertEqual(stats["n_pairs"], 2)
            self.assertEqual(stats["both_solved_n"], 1)

            with open(table_path, encoding="utf-8") as f:
                table = json.load(f)
            self.assertEqual(len(table), 2)


if __name__ == "__main__":
    unittest.main()
