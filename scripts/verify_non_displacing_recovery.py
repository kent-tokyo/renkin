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


def summarize(rows: list[dict], budget_ms: float | None = None) -> dict:
    baseline_success = 0
    final_success = 0
    recovered = 0
    regression = 0
    timeout = 0
    crash = 0
    missing_attempts = 0
    recovery_elapsed_values: list[float] = []
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
    if budget_ms is not None:
        over_budget = [elapsed for elapsed in recovery_elapsed_values if elapsed > budget_ms]
        result["budget_ms"] = budget_ms
        result["budget_observed_count"] = len(recovery_elapsed_values)
        result["budget_overrun_count"] = len(over_budget)
        result["budget_max_elapsed_ms"] = max(recovery_elapsed_values, default=None)
        result["within_budget"] = not over_budget
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
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--require-zero-regression", action="store_true")
    parser.add_argument(
        "--budget-ms",
        type=float,
        help="optionally verify each recorded recovery total_elapsed_ms is within this bound",
    )
    args = parser.parse_args()
    result = summarize(load_rows(args.rows), args.budget_ms)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result, ensure_ascii=False))
    if args.require_zero_regression and not result["zero_regression"]:
        return 1
    if args.budget_ms is not None and not result["within_budget"]:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
