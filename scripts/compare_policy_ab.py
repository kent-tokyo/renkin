"""Pairwise local A/B report for two RENKIN search configurations.

This report is deliberately narrower than a competitor comparison.  It is for
the same target/stock/template-set cohort, typically legacy ordering versus a
TemplatePolicy arm.  Route discovery, strict validation, latency, memory, and
route determinism are reported as separate axes; no single score is emitted.

Stdlib only.  Inputs are PlannerComparisonRow JSONL files produced by the
existing comparison harness.
"""

from __future__ import annotations

import argparse
import json
from collections import Counter

from compare_schema import load_rows
from compare_stats import (
    mean_diff_statistic,
    mcnemar_exact,
    paired_bootstrap_diff,
    percentile,
    rate_diff_statistic,
)


def _index(rows: list, label: str) -> dict:
    result = {}
    duplicates = []
    for row in rows:
        if row.target_id in result:
            duplicates.append(row.target_id)
        result[row.target_id] = row
    if duplicates:
        raise ValueError(f"{label} contains duplicate target_id(s): {sorted(set(duplicates))[:5]}")
    return result


def join_arms(left_rows: list, right_rows: list) -> list[tuple]:
    """Join two arms and reject cohort/configuration mismatches."""
    left = _index(left_rows, "left arm")
    right = _index(right_rows, "right arm")
    if left.keys() != right.keys():
        raise ValueError("A/B target_id sets differ; paired comparison requires an exact cohort")

    joined = []
    for target_id in sorted(left):
        l, r = left[target_id], right[target_id]
        if l.target_smiles != r.target_smiles:
            raise ValueError(f"target_smiles differs for target_id={target_id!r}")
        if l.comparison_mode != r.comparison_mode:
            raise ValueError(f"comparison_mode differs for target_id={target_id!r}")
        joined.append((target_id, l, r))
    if not joined:
        raise ValueError("A/B comparison requires at least one paired target")
    return joined


def _binary(row, field: str) -> bool:
    return getattr(row, field) is True


def _axis(joined: list[tuple], field: str) -> dict:
    pairs = [(_binary(left, field), _binary(right, field)) for _, left, right in joined]
    result = paired_bootstrap_diff(pairs, rate_diff_statistic)
    mcnemar = mcnemar_exact(pairs)
    both = sum(a and b for a, b in pairs)
    left_only = sum(a and not b for a, b in pairs)
    right_only = sum(b and not a for a, b in pairs)
    neither = sum(not a and not b for a, b in pairs)
    return {
        "field": field,
        "left_true": sum(a for a, _ in pairs),
        "right_true": sum(b for _, b in pairs),
        "both_true": both,
        "left_only": left_only,
        "right_only": right_only,
        "neither": neither,
        "left_minus_right_rate": {
            "observed": result.observed_diff,
            "ci_low": result.ci_low,
            "ci_high": result.ci_high,
            "ci_level": result.ci_level,
            "n_pairs": result.n_pairs,
            "n_iterations": result.n_iterations,
            "seed": result.seed,
        },
        "mcnemar_exact_two_sided_p": mcnemar.p_value,
    }


def _latency(joined: list[tuple]) -> dict:
    pairs = [
        (left.total_elapsed_ms, right.total_elapsed_ms)
        for _, left, right in joined
        if left.total_elapsed_ms is not None and right.total_elapsed_ms is not None
    ]
    left_values = [a for a, _ in pairs]
    right_values = [b for _, b in pairs]
    result = {
        "denominator_kind": "both_arms_non_null_total_elapsed_ms",
        "n_pairs": len(pairs),
        "left_ms": {"p50": percentile(left_values, 50), "p95": percentile(left_values, 95)},
        "right_ms": {"p50": percentile(right_values, 50), "p95": percentile(right_values, 95)},
    }
    if pairs:
        delta = paired_bootstrap_diff(pairs, mean_diff_statistic)
        result["paired_mean_delta_left_minus_right_ms"] = {
            "observed": delta.observed_diff,
            "ci_low": delta.ci_low,
            "ci_high": delta.ci_high,
            "ci_level": delta.ci_level,
            "n_pairs": delta.n_pairs,
            "n_iterations": delta.n_iterations,
            "seed": delta.seed,
        }
    return result


def _memory(joined: list[tuple]) -> dict:
    pairs = [
        (left.peak_rss_bytes, right.peak_rss_bytes)
        for _, left, right in joined
        if left.peak_rss_bytes is not None and right.peak_rss_bytes is not None
    ]
    left_values = [a for a, _ in pairs]
    right_values = [b for _, b in pairs]
    result = {
        "denominator_kind": "both_arms_non_null_peak_rss_bytes",
        "n_pairs": len(pairs),
        "left_bytes": {
            "p50": percentile(left_values, 50),
            "p95": percentile(left_values, 95),
        },
        "right_bytes": {
            "p50": percentile(right_values, 50),
            "p95": percentile(right_values, 95),
        },
    }
    if pairs:
        delta = paired_bootstrap_diff(pairs, mean_diff_statistic)
        result["paired_mean_delta_left_minus_right_bytes"] = {
            "observed": delta.observed_diff,
            "ci_low": delta.ci_low,
            "ci_high": delta.ci_high,
            "ci_level": delta.ci_level,
            "n_pairs": delta.n_pairs,
            "n_iterations": delta.n_iterations,
            "seed": delta.seed,
        }
    return result


def _cross_arm_route_hashes(joined: list[tuple]) -> dict:
    comparable = [
        (left.normalized_route_sha256, right.normalized_route_sha256)
        for _, left, right in joined
        if left.normalized_route_sha256 is not None and right.normalized_route_sha256 is not None
    ]
    equal = sum(a == b for a, b in comparable)
    return {
        "denominator_kind": "both_arms_non_null_normalized_route_sha256",
        "interpretation": "route-content agreement between the two arms, not repeat-run determinism",
        "n_pairs": len(comparable),
        "equal": equal,
        "different": len(comparable) - equal,
        "equal_rate": equal / len(comparable) if comparable else None,
    }


def _repeat_determinism(first_rows: list, repeat_rows: list) -> dict:
    joined = join_arms(first_rows, repeat_rows)
    outcome_fields = ("run_status", "route_found", "validator_confirmed_route_found")
    outcome_equal = sum(
        all(getattr(first, field) == getattr(repeat, field) for field in outcome_fields)
        for _, first, repeat in joined
    )
    hash_comparable = [
        (first.normalized_route_sha256, repeat.normalized_route_sha256)
        for _, first, repeat in joined
        if first.normalized_route_sha256 is not None
        and repeat.normalized_route_sha256 is not None
    ]
    hash_equal = sum(first == repeat for first, repeat in hash_comparable)
    return {
        "denominator_kind": "same_arm_repeat_target_pairs",
        "n_pairs": len(joined),
        "outcome_equal": outcome_equal,
        "outcome_equal_rate": outcome_equal / len(joined),
        "route_hash_denominator_kind": "both_runs_non_null_normalized_route_sha256",
        "route_hash_n": len(hash_comparable),
        "route_hash_equal": hash_equal,
        "route_hash_equal_rate": hash_equal / len(hash_comparable) if hash_comparable else None,
    }


def compute_report(joined: list[tuple], left_label: str, right_label: str) -> dict:
    return {
        "schema_version": "1",
        "comparison_kind": "same_cohort_renkin_configuration_ab",
        "left_label": left_label,
        "right_label": right_label,
        "n_pairs": len(joined),
        "run_status": {
            "left": dict(Counter(row.run_status for _, row, _ in joined)),
            "right": dict(Counter(row.run_status for _, _, row in joined)),
        },
        "axes": {
            "route_found": _axis(joined, "route_found"),
            "strict_validator_confirmed_route_found": _axis(
                joined, "validator_confirmed_route_found"
            ),
            "latency": _latency(joined),
            "memory": _memory(joined),
            "cross_arm_route_content_agreement": _cross_arm_route_hashes(joined),
        },
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--left-rows", required=True)
    parser.add_argument("--right-rows", required=True)
    parser.add_argument("--left-label", default="legacy")
    parser.add_argument("--right-label", default="policy")
    parser.add_argument("--left-repeat-rows")
    parser.add_argument("--right-repeat-rows")
    parser.add_argument("--output", required=True)
    args = parser.parse_args(argv)

    left_rows = load_rows(args.left_rows)
    right_rows = load_rows(args.right_rows)
    report = compute_report(
        join_arms(left_rows, right_rows),
        args.left_label,
        args.right_label,
    )
    repeat_paths = (args.left_repeat_rows, args.right_repeat_rows)
    if any(repeat_paths):
        if not all(repeat_paths):
            parser.error("--left-repeat-rows and --right-repeat-rows must be given together")
        report["axes"]["repeat_run_determinism"] = {
            "left": _repeat_determinism(left_rows, load_rows(args.left_repeat_rows)),
            "right": _repeat_determinism(right_rows, load_rows(args.right_repeat_rows)),
        }
    with open(args.output, "w", encoding="utf-8") as f:
        json.dump(report, f, indent=2, sort_keys=True)
        f.write("\n")
    print(f"n_pairs={report['n_pairs']} output={args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
