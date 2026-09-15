#!/usr/bin/env python3
"""Verify that staged recovery adds routes without displacing native success.

The verifier reads the recovery attempts embedded in compare-run JSONL output.
It deliberately compares the first native attempt with the selected final
result, so a later stage cannot hide a regression behind aggregate counts.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path


def strict_success(row: dict) -> bool:
    """Match the shared-stock strict metric used by compare_aggregate.py."""
    if row.get("all_leaves_in_configured_stock") is not True:
        return False
    if row.get("validator_confirmed_route_found") is True:
        return True
    return (
        row.get("route_found") is True
        and row.get("route_tree_parseable") is True
        and row.get("best_route_step_count") == 0
    )


def _rows_by_target(rows: list[dict], label: str) -> dict[str, dict]:
    indexed = {}
    for index, row in enumerate(rows, start=1):
        target_id = row.get("target_id")
        if not isinstance(target_id, str) or not target_id:
            raise ValueError(f"{label} row {index} has no target_id")
        if target_id in indexed:
            raise ValueError(f"{label} contains duplicate target_id {target_id!r}")
        indexed[target_id] = row
    return indexed


def summarize(
    rows: list[dict],
    budget_ms: float | None = None,
    process_budget_ms: float | None = None,
    baseline_rows: list[dict] | None = None,
) -> dict:
    baseline_success = 0
    final_success = 0
    recovered = 0
    regression = 0
    timeout = 0
    crash = 0
    missing_attempts = 0
    recovery_elapsed_values: list[float] = []
    process_elapsed_values: list[float] = []
    for row in rows:
        run_status = row.get("run_status")
        if run_status == "timeout":
            timeout += 1
        if run_status in {"crash", "crashed"}:
            crash += 1
        recovery = row.get("tool_specific", {}).get("renkin", {}).get("recovery", {})
        attempts = recovery.get("attempts") if isinstance(recovery, dict) else None
        if isinstance(recovery, dict) and isinstance(recovery.get("total_elapsed_ms"), (int, float)):
            recovery_elapsed_values.append(float(recovery["total_elapsed_ms"]))
        if isinstance(row.get("total_elapsed_ms"), (int, float)):
            process_elapsed_values.append(float(row["total_elapsed_ms"]))
        if not isinstance(attempts, list) or not attempts:
            missing_attempts += 1
            continue
        native = attempts[0]
        native_found = isinstance(native, dict) and native.get("routes_found", 0) > 0
        final_found = row.get("route_found") is True
        baseline_success += int(native_found)
        final_success += int(final_found)
        recovered += int(final_found and not native_found)
        regression += int(native_found and not final_found)
    result = {
        "schema_version": "renkin-non-displacing-recovery-verification/1",
        "row_count": len(rows),
        "baseline_success_count": baseline_success,
        "final_success_count": final_success,
        "recovered_count": recovered,
        "regression_count": regression,
        "timeout_count": timeout,
        "crash_count": crash,
        "missing_attempts_count": missing_attempts,
        "zero_regression": regression == 0,
    }
    if baseline_rows is not None:
        baseline_by_target = _rows_by_target(baseline_rows, "baseline")
        candidate_by_target = _rows_by_target(rows, "candidate")
        if set(baseline_by_target) != set(candidate_by_target):
            missing_from_candidate = sorted(set(baseline_by_target) - set(candidate_by_target))
            missing_from_baseline = sorted(set(candidate_by_target) - set(baseline_by_target))
            raise ValueError(
                "baseline/candidate target sets differ: "
                f"missing_from_candidate={missing_from_candidate[:5]!r}, "
                f"missing_from_baseline={missing_from_baseline[:5]!r}"
            )
        baseline_strict = sum(strict_success(row) for row in baseline_rows)
        final_strict = sum(strict_success(row) for row in rows)
        strict_regression = sum(
            strict_success(baseline_by_target[target_id])
            and not strict_success(candidate_by_target[target_id])
            for target_id in baseline_by_target
        )
        result.update(
            {
                "baseline_strict_success_count": baseline_strict,
                "final_strict_success_count": final_strict,
                "strict_regression_count": strict_regression,
                "zero_strict_regression": strict_regression == 0,
            }
        )
    if budget_ms is not None:
        over_budget = [elapsed for elapsed in recovery_elapsed_values if elapsed > budget_ms]
        result["budget_ms"] = budget_ms
        result["budget_observed_count"] = len(recovery_elapsed_values)
        result["budget_overrun_count"] = len(over_budget)
        result["budget_max_elapsed_ms"] = max(recovery_elapsed_values, default=None)
        result["within_budget"] = not over_budget
    if process_budget_ms is not None:
        over_process_budget = [elapsed for elapsed in process_elapsed_values if elapsed > process_budget_ms]
        result["process_budget_ms"] = process_budget_ms
        result["process_budget_observed_count"] = len(process_elapsed_values)
        result["process_budget_overrun_count"] = len(over_process_budget)
        result["process_budget_max_elapsed_ms"] = max(process_elapsed_values, default=None)
        result["within_process_budget"] = not over_process_budget
    return result


def load_rows(path: Path) -> list[dict]:
    rows = []
    with path.open(encoding="utf-8") as stream:
        for line_number, line in enumerate(stream, 1):
            if not line.strip():
                continue
            row = json.loads(line)
            if not isinstance(row, dict):
                raise ValueError(f"line {line_number}: row must be an object")
            rows.append(row)
    return rows


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rows", type=Path, required=True)
    parser.add_argument(
        "--baseline-rows",
        type=Path,
        help="same-cohort standard-search rows used to verify strict non-regression",
    )
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--require-zero-regression", action="store_true")
    parser.add_argument("--require-zero-strict-regression", action="store_true")
    parser.add_argument("--require-complete-attempts", action="store_true")
    parser.add_argument(
        "--budget-ms",
        type=float,
        help="optionally verify each recorded recovery total_elapsed_ms is within this bound",
    )
    parser.add_argument(
        "--process-budget-ms",
        type=float,
        help="optionally verify each row's externally observed process wall-clock is within this bound",
    )
    args = parser.parse_args()
    if args.require_zero_strict_regression and args.baseline_rows is None:
        parser.error("--require-zero-strict-regression requires --baseline-rows")
    result = summarize(
        load_rows(args.rows),
        args.budget_ms,
        args.process_budget_ms,
        load_rows(args.baseline_rows) if args.baseline_rows else None,
    )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result, ensure_ascii=False))
    if args.require_zero_regression and not result["zero_regression"]:
        return 1
    if args.require_zero_strict_regression and not result["zero_strict_regression"]:
        return 1
    if args.require_complete_attempts and result["missing_attempts_count"]:
        return 1
    if args.budget_ms is not None and not result["within_budget"]:
        return 1
    if args.process_budget_ms is not None and not result["within_process_budget"]:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
