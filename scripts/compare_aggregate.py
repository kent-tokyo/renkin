"""Common aggregate schema for the Issue #66 open-source planner comparison.

Computed once per (tool, comparison_mode) group of PlannerComparisonRow
rows. Every rate's denominator is explicit and stated per-metric -- see
docs/guides/open-source-retrosynthesis-comparison.md, "Common post-hoc
validation" -- never silently reused across metrics that mean different
things.
"""

from __future__ import annotations

from compare_stats import percentile


def _rate(numerator: int, denominator: int) -> dict:
    return {
        "value": (numerator / denominator) if denominator > 0 else None,
        "n_numerator": numerator,
        "n_denominator": denominator,
    }


def compute_aggregate(rows: list) -> dict:
    all_sampled = rows
    n_all = len(all_sampled)

    measured_runs = [r for r in rows if r.run_status != "setup_error"]
    route_found_runs = [r for r in rows if r.route_found is True]
    parseable_routes = [r for r in route_found_runs if r.route_tree_parseable is True]

    agg = {
        "n_all_sampled": n_all,
        "route_found_rate": {
            **_rate(sum(1 for r in all_sampled if r.route_found is True), n_all),
            "denominator_kind": "all_sampled",
        },
        "route_to_configured_stock_rate": {
            **_rate(
                sum(1 for r in all_sampled if r.all_leaves_in_configured_stock is True), n_all
            ),
            "denominator_kind": "all_sampled",
        },
        "timeout_rate": {
            **_rate(sum(1 for r in all_sampled if r.run_status == "timeout"), n_all),
            "denominator_kind": "all_sampled",
        },
        "crash_rate": {
            **_rate(sum(1 for r in all_sampled if r.run_status == "crashed"), n_all),
            "denominator_kind": "all_sampled",
        },
        "setup_error_rate": {
            **_rate(sum(1 for r in all_sampled if r.run_status == "setup_error"), n_all),
            "denominator_kind": "all_sampled",
        },
        "invalid_input_rate": {
            **_rate(sum(1 for r in all_sampled if r.run_status == "invalid_input"), n_all),
            "denominator_kind": "all_sampled",
        },
        "target_elements_accounted_route_rate": {
            **_rate(
                sum(1 for r in all_sampled if r.target_element_accounting_status == "accounted"),
                n_all,
            ),
            "denominator_kind": "all_sampled",
        },
        "common_structural_warning_rate": {
            **_rate(sum(1 for r in all_sampled if r.common_validation_warnings), n_all),
            "denominator_kind": "all_sampled",
        },
        "route_tree_parseable_rate": {
            **_rate(
                sum(1 for r in route_found_runs if r.route_tree_parseable is True),
                len(route_found_runs),
            ),
            "denominator_kind": "route_found_runs",
        },
        "route_found_but_tree_not_parseable_count": sum(
            1 for r in route_found_runs if r.route_tree_parseable is False
        ),
        "reaction_steps_parseable_rate": {
            **_rate(
                sum(1 for r in parseable_routes if r.reaction_steps_parseable is True),
                len(parseable_routes),
            ),
            "denominator_kind": "parseable_routes",
        },
    }

    tt_first = [r.time_to_first_route_ms for r in route_found_runs if r.time_to_first_route_ms is not None]
    total_elapsed = [r.total_elapsed_ms for r in measured_runs if r.total_elapsed_ms is not None]
    peak_rss = [r.peak_rss_bytes for r in measured_runs if r.peak_rss_bytes is not None]
    depths = [r.best_route_depth for r in route_found_runs if r.best_route_depth is not None]
    step_counts = [
        r.best_route_step_count for r in route_found_runs if r.best_route_step_count is not None
    ]
    leaf_counts = [
        r.best_route_leaf_count for r in parseable_routes if r.best_route_leaf_count is not None
    ]

    agg["time_to_first_route_ms_percentiles"] = {
        "p50": percentile(tt_first, 50),
        "p90": percentile(tt_first, 90),
        "p95": percentile(tt_first, 95),
        "p99": percentile(tt_first, 99),
        "max": percentile(tt_first, 100),
        "denominator_kind": "route_found_runs",
        "n": len(tt_first),
    }
    agg["total_elapsed_ms_percentiles"] = {
        "p50": percentile(total_elapsed, 50),
        "p95": percentile(total_elapsed, 95),
        "max": percentile(total_elapsed, 100),
        "denominator_kind": "measured_runs",
        "n": len(total_elapsed),
    }
    agg["peak_rss_bytes_percentiles"] = {
        "p50": percentile(peak_rss, 50),
        "p95": percentile(peak_rss, 95),
        "max": percentile(peak_rss, 100),
        "denominator_kind": "measured_runs",
        "n": len(peak_rss),
    }
    agg["best_route_depth_distribution"] = {
        "p50": percentile(depths, 50),
        "p95": percentile(depths, 95),
        "max": percentile(depths, 100),
        "denominator_kind": "route_found_runs",
        "n": len(depths),
    }
    agg["best_route_step_count_distribution"] = {
        "p50": percentile(step_counts, 50),
        "p95": percentile(step_counts, 95),
        "max": percentile(step_counts, 100),
        "denominator_kind": "route_found_runs",
        "n": len(step_counts),
    }
    agg["best_route_leaf_count_distribution"] = {
        "p50": percentile(leaf_counts, 50),
        "p95": percentile(leaf_counts, 95),
        "max": percentile(leaf_counts, 100),
        "denominator_kind": "parseable_routes",
        "n": len(leaf_counts),
    }

    # Solved-only latency -- the ONLY comparative latency number this
    # harness licenses across tools (see "Latency comparison firewall").
    solved_elapsed = [
        r.total_elapsed_ms
        for r in route_found_runs
        if r.total_elapsed_ms is not None
    ]
    agg["solved_only_total_elapsed_ms_percentiles"] = {
        "p50": percentile(solved_elapsed, 50),
        "p95": percentile(solved_elapsed, 95),
        "max": percentile(solved_elapsed, 100),
        "denominator_kind": "route_found_runs",
        "n": len(solved_elapsed),
    }

    return agg
