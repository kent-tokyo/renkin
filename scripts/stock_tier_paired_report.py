"""Paired report for the v0.36.0 scalable-stock pilot (Phase B/C/D).

Joins two `compare_run.py --tool renkin` JSONL outputs on `target_id` --
one run against the default 402-compound stock (baseline), one against a
10k/100k/1M deterministic tier (candidate, see `scripts/build_stock_tiers.py`)
-- and reports the 4 axes as separate, non-conflated numbers (this
project's own standing rule): search capability (raw route_found), route
quality (validator_confirmed_route_found), side effects (timeouts,
not_evaluable, latency, peak RSS), and safety (gated_out_candidate_count).

This is a RENKIN-vs-RENKIN stock-size ablation, not a cross-tool
comparison -- kept separate from `compare_paired_report.py` (which is
hardwired to the renkin/aizynthfinder pairing and computes
`route_to_shared_stock`, not meaningful here) but reuses its same join
discipline (hard error on any target_id mismatch) and `compare_stats`'s
generic, tool-agnostic paired-bootstrap/McNemar machinery directly.

At n=20 (Phase B smoke) or n=100 (Phase C), every result here is
DESCRIPTIVE ONLY -- wide confidence intervals are expected, no
"statistically significant" claim is licensed at this sample size (same
discipline `compare_stats.py`'s own module doc states).

Usage:
    python3 scripts/stock_tier_paired_report.py \
        --baseline-rows data/stock_tiers/gate_10k/baseline.jsonl \
        --candidate-rows data/stock_tiers/gate_10k/candidate.jsonl \
        --output-summary data/stock_tiers/gate_10k/summary.json
"""

from __future__ import annotations

import argparse
import json

from compare_schema import load_rows
from compare_stats import mcnemar_exact, paired_bootstrap_diff, percentile, rate_diff_statistic


def join_rows(baseline_rows: list, candidate_rows: list) -> list[tuple]:
    """Join on target_id. Hard error on any set mismatch -- never silently intersect."""
    baseline_by_id = {r.target_id: r for r in baseline_rows}
    candidate_by_id = {r.target_id: r for r in candidate_rows}
    if baseline_by_id.keys() != candidate_by_id.keys():
        only_baseline = sorted(baseline_by_id.keys() - candidate_by_id.keys())
        only_candidate = sorted(candidate_by_id.keys() - baseline_by_id.keys())
        raise ValueError(
            f"target_id sets differ between the two row files: "
            f"{len(only_baseline)} only in baseline (e.g. {only_baseline[:3]}), "
            f"{len(only_candidate)} only in candidate (e.g. {only_candidate[:3]})"
        )
    return [(tid, baseline_by_id[tid], candidate_by_id[tid]) for tid in sorted(baseline_by_id)]


def _rate(rows, predicate) -> dict:
    n = len(rows)
    numerator = sum(1 for r in rows if predicate(r))
    return {
        "denominator_kind": "all_sampled",
        "n_denominator": n,
        "n_numerator": numerator,
        "value": numerator / n if n else None,
    }


def build_summary(joined: list[tuple]) -> dict:
    baseline_rows = [b for _, b, _ in joined]
    candidate_rows = [c for _, _, c in joined]
    n = len(joined)

    summary = {
        "n_targets": n,
        # ---- Axis 1: search capability (raw route_found) ----
        "baseline_route_found_rate": _rate(baseline_rows, lambda r: r.route_found is True),
        "candidate_route_found_rate": _rate(candidate_rows, lambda r: r.route_found is True),
        # ---- Axis 2: route quality (validator-confirmed) ----
        "baseline_validator_confirmed_rate": _rate(
            baseline_rows, lambda r: r.validator_confirmed_route_found is True
        ),
        "candidate_validator_confirmed_rate": _rate(
            candidate_rows, lambda r: r.validator_confirmed_route_found is True
        ),
        # ---- Axis 3: side effects ----
        "baseline_timeout_count": sum(1 for r in baseline_rows if r.run_status == "timeout"),
        "candidate_timeout_count": sum(1 for r in candidate_rows if r.run_status == "timeout"),
        "baseline_not_evaluable_count": sum(1 for r in baseline_rows if r.not_evaluable is True),
        "candidate_not_evaluable_count": sum(1 for r in candidate_rows if r.not_evaluable is True),
        "baseline_peak_rss_bytes_p50": percentile(
            [r.peak_rss_bytes for r in baseline_rows if r.peak_rss_bytes is not None], 50
        ),
        "candidate_peak_rss_bytes_p50": percentile(
            [r.peak_rss_bytes for r in candidate_rows if r.peak_rss_bytes is not None], 50
        ),
        "baseline_peak_rss_bytes_max": max(
            (r.peak_rss_bytes for r in baseline_rows if r.peak_rss_bytes is not None), default=None
        ),
        "candidate_peak_rss_bytes_max": max(
            (r.peak_rss_bytes for r in candidate_rows if r.peak_rss_bytes is not None), default=None
        ),
        # ---- Axis 4: safety (spectator-bond gated-out) ----
        "baseline_gated_out_candidate_count_total": sum(
            r.gated_out_candidate_count or 0 for r in baseline_rows
        ),
        "candidate_gated_out_candidate_count_total": sum(
            r.gated_out_candidate_count or 0 for r in candidate_rows
        ),
    }

    # Paired diff on raw route_found (bootstrap + McNemar), same convention
    # as compare_paired_report.py's native-mode diff.
    route_found_pairs = [(c.route_found is True, b.route_found is True) for _, b, c in joined]
    boot = paired_bootstrap_diff(route_found_pairs, rate_diff_statistic)
    mcnemar = mcnemar_exact(route_found_pairs)
    summary["route_found_rate_diff_candidate_minus_baseline"] = {
        "observed": boot.observed_diff,
        "ci_low": boot.ci_low,
        "ci_high": boot.ci_high,
        "n_iterations": boot.n_iterations,
        "seed": boot.seed,
        "mcnemar_p_value": mcnemar.p_value,
        "discordant_candidate_only": mcnemar.discordant_a_only,
        "discordant_baseline_only": mcnemar.discordant_b_only,
    }

    # Paired per-target latency deltas, both-completed-only -- per this
    # project's own beam-diversity findings, a bare p95 can look like a
    # regression while paired deltas show the opposite; report both.
    both_completed = [
        (b, c)
        for _, b, c in joined
        if b.run_status == "completed" and c.run_status == "completed"
    ]
    deltas = [c.total_elapsed_ms - b.total_elapsed_ms for b, c in both_completed]
    summary["latency_paired_deltas_ms"] = {
        "n_both_completed": len(both_completed),
        "sum_candidate_minus_baseline": sum(deltas) if deltas else None,
        "mean_candidate_minus_baseline": (sum(deltas) / len(deltas)) if deltas else None,
    }

    # Regressions: baseline solved, candidate didn't -- the single most
    # important safety signal, reported explicitly and never buried in a
    # rate.
    regressions = [tid for tid, b, c in joined if b.route_found is True and c.route_found is not True]
    new_solves = [tid for tid, b, c in joined if c.route_found is True and b.route_found is not True]
    summary["regression_count"] = len(regressions)
    summary["regressions"] = regressions
    summary["new_solve_count"] = len(new_solves)
    summary["new_solves"] = new_solves

    return summary


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--baseline-rows", required=True)
    parser.add_argument("--candidate-rows", required=True)
    parser.add_argument("--output-summary", required=True)
    args = parser.parse_args(argv)

    baseline_rows = load_rows(args.baseline_rows)
    candidate_rows = load_rows(args.candidate_rows)
    joined = join_rows(baseline_rows, candidate_rows)
    summary = build_summary(joined)

    with open(args.output_summary, "w", encoding="utf-8") as f:
        json.dump(summary, f, indent=2, sort_keys=True)
        f.write("\n")

    print(json.dumps(summary, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    import sys

    sys.exit(main())
