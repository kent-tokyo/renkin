"""Paired-report generator for the Issue #66 open-source planner comparison.

Joins one tool's PlannerComparisonRow JSONL (`compare_run.py --tool renkin`)
against the other's (`--tool aizynthfinder`) on `target_id`, and emits the
paired bootstrap statistics (`paired_stats_<mode>.json`) and per-target join
table (`paired_table_<mode>.json`) that `docs/guides/open-source-retrosynthesis-comparison.md`
references but never had a checked-in generator for -- that guide's own
"Reproduction" section stops after `compare_run.py`'s per-tool JSONL/aggregate
output. This closes that gap so a future re-measurement round doesn't need to
reverse-engineer the original ad-hoc computation from the checked-in JSON
shape alone.

`--mode native` reports the plain `route_found` rate diff (paired bootstrap +
McNemar) plus a both-solved-only `total_elapsed_ms` diff. `--mode
shared_stock` instead leads with the arm's actual primary metric --
`route_to_shared_stock` (route_found AND route_tree_parseable AND
all_leaves_in_configured_stock, the independently-verified check) -- with
tool-native `route_found` reported only as a secondary/informational diff;
no elapsed-ms diff is reported for this arm (see the comparison guide's
disclosed shared-hardware wall-clock-contamination note for why the native
arm's latency diff isn't repeated here).
"""

from __future__ import annotations

import argparse
import json

from compare_schema import load_rows
from compare_stats import (
    BootstrapResult,
    McNemarResult,
    mcnemar_exact,
    mean_diff_statistic,
    paired_bootstrap_diff,
    rate_diff_statistic,
)


def join_rows(renkin_rows: list, aizynthfinder_rows: list) -> list[tuple]:
    """Join on target_id. Hard error on any set mismatch -- never silently intersect."""
    renkin_by_id = {r.target_id: r for r in renkin_rows}
    aizynth_by_id = {r.target_id: r for r in aizynthfinder_rows}
    if renkin_by_id.keys() != aizynth_by_id.keys():
        only_renkin = sorted(renkin_by_id.keys() - aizynth_by_id.keys())
        only_aizynth = sorted(aizynth_by_id.keys() - renkin_by_id.keys())
        raise ValueError(
            f"target_id sets differ between the two row files: "
            f"{len(only_renkin)} only in renkin (e.g. {only_renkin[:3]}), "
            f"{len(only_aizynth)} only in aizynthfinder (e.g. {only_aizynth[:3]})"
        )
    return [(tid, renkin_by_id[tid], aizynth_by_id[tid]) for tid in sorted(renkin_by_id)]


def _route_to_shared_stock(row) -> bool:
    return (
        row.route_found is True
        and row.route_tree_parseable is True
        and row.all_leaves_in_configured_stock is True
    )


def _route_found_mcnemar(joined: list[tuple]) -> McNemarResult:
    # (aizynthfinder, renkin) order -- discordant_a_only/b_only map to
    # aizynthfinder_only/renkin_only respectively, same convention both modes use.
    return mcnemar_exact([(a.route_found is True, r.route_found is True) for _, r, a in joined])


def _nested_diff_dict(result: BootstrapResult) -> dict:
    return {
        "observed": result.observed_diff,
        "ci_low": result.ci_low,
        "ci_high": result.ci_high,
        "n_iterations": result.n_iterations,
        "seed": result.seed,
    }


def _flat_diff_dict(result: BootstrapResult, mcnemar: McNemarResult) -> dict:
    return {
        "observed": result.observed_diff,
        "ci_low": result.ci_low,
        "ci_high": result.ci_high,
        "n_pairs": result.n_pairs,
        "mcnemar_renkin_only": mcnemar.discordant_b_only,
        "mcnemar_aizynthfinder_only": mcnemar.discordant_a_only,
        "mcnemar_p_value": mcnemar.p_value,
    }


def compute_paired_stats_native(joined: list[tuple]) -> dict:
    route_found_pairs = [(r.route_found, a.route_found) for _, r, a in joined]
    both_solved = [
        (tid, r, a) for tid, r, a in joined if r.route_found is True and a.route_found is True
    ]

    route_found_diff = paired_bootstrap_diff(route_found_pairs, rate_diff_statistic)
    mcnemar = _route_found_mcnemar(joined)

    stats = {
        "n_pairs": len(joined),
        "both_solved_n": len(both_solved),
        "route_found_rate_diff_renkin_minus_aizynthfinder": _nested_diff_dict(route_found_diff),
        "mcnemar": {
            "renkin_only": mcnemar.discordant_b_only,
            "aizynthfinder_only": mcnemar.discordant_a_only,
            "p_value": mcnemar.p_value,
        },
    }

    elapsed_pairs = [
        (r.total_elapsed_ms, a.total_elapsed_ms)
        for _, r, a in both_solved
        if r.total_elapsed_ms is not None and a.total_elapsed_ms is not None
    ]
    if elapsed_pairs:
        elapsed_diff = paired_bootstrap_diff(elapsed_pairs, mean_diff_statistic)
        stats["total_elapsed_ms_diff_renkin_minus_aizynthfinder_both_solved"] = _nested_diff_dict(
            elapsed_diff
        )

    return stats


def compute_paired_stats_shared_stock(joined: list[tuple]) -> dict:
    shared_stock_pairs = [
        (_route_to_shared_stock(r), _route_to_shared_stock(a)) for _, r, a in joined
    ]
    shared_stock_diff = paired_bootstrap_diff(shared_stock_pairs, rate_diff_statistic)
    shared_stock_mcnemar = mcnemar_exact(
        [(_route_to_shared_stock(a), _route_to_shared_stock(r)) for _, r, a in joined]
    )

    route_found_pairs = [(r.route_found, a.route_found) for _, r, a in joined]
    route_found_diff = paired_bootstrap_diff(route_found_pairs, rate_diff_statistic)
    route_found_mcnemar = _route_found_mcnemar(joined)

    return {
        "primary_metric": (
            "route_to_shared_stock (route_found AND route_tree_parseable AND "
            "all_leaves_in_configured_stock)"
        ),
        "route_to_shared_stock_rate_diff_renkin_minus_aizynthfinder": _flat_diff_dict(
            shared_stock_diff, shared_stock_mcnemar
        ),
        "secondary_tool_native_route_found_rate_diff_renkin_minus_aizynthfinder": _flat_diff_dict(
            route_found_diff, route_found_mcnemar
        ),
    }


def compute_paired_stats(joined: list[tuple], mode: str) -> dict:
    if mode == "shared_stock":
        return compute_paired_stats_shared_stock(joined)
    return compute_paired_stats_native(joined)


def compute_paired_table(joined: list[tuple], mode: str) -> list[dict]:
    rows = []
    for tid, r, a in joined:
        row = {
            "target_id": tid,
            "renkin_route_found": r.route_found,
            "aizynthfinder_route_found": a.route_found,
            "renkin_target_element_accounting_status": r.target_element_accounting_status,
            "aizynthfinder_target_element_accounting_status": a.target_element_accounting_status,
        }
        if mode == "shared_stock":
            row["renkin_route_to_shared_stock"] = _route_to_shared_stock(r)
            row["aizynthfinder_route_to_shared_stock"] = _route_to_shared_stock(a)
        rows.append(row)
    return rows


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--renkin-rows", required=True)
    parser.add_argument("--aizynthfinder-rows", required=True)
    parser.add_argument("--mode", choices=["native", "shared_stock"], default="native")
    parser.add_argument("--output-stats", required=True)
    parser.add_argument("--output-table", required=True)
    args = parser.parse_args(argv)

    joined = join_rows(load_rows(args.renkin_rows), load_rows(args.aizynthfinder_rows))
    stats = compute_paired_stats(joined, args.mode)
    table = compute_paired_table(joined, args.mode)

    with open(args.output_stats, "w", encoding="utf-8") as f:
        json.dump(stats, f, indent=2, sort_keys=True)
        f.write("\n")
    with open(args.output_table, "w", encoding="utf-8") as f:
        json.dump(table, f, indent=2, sort_keys=True)
        f.write("\n")

    print(f"mode={args.mode} n_pairs={len(joined)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
