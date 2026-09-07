#!/usr/bin/env python3
"""Classify recovery-run quality signals without changing comparison metrics.

The report keeps the historical strict metric untouched while separating
direct-purchase semantics, informational warnings, and actual validation
failures.  This prevents a depth-zero stock route from being mistaken for a
chemical validation defect and makes the next optimization arm auditable.
"""

from __future__ import annotations

import argparse
import json
from collections import Counter
from pathlib import Path


def load_rows(path: str) -> list[dict]:
    with open(path, encoding="utf-8") as handle:
        return [json.loads(line) for line in handle if line.strip()]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--renkin", required=True, help="recovery-run JSONL")
    parser.add_argument("--output", required=True)
    args = parser.parse_args()

    records = load_rows(args.renkin)
    direct_purchase = []
    strict_pass = []
    not_evaluable = []
    warning_counts: Counter[str] = Counter()
    stage_counts: Counter[str] = Counter()
    failure_counts: Counter[str] = Counter()

    for row in records:
        if row.get("best_route_depth") == 0 and row.get("route_found"):
            direct_purchase.append(row["target_id"])
        if row.get("validator_confirmed_route_found") is True:
            strict_pass.append(row["target_id"])
        if row.get("not_evaluable") is True:
            not_evaluable.append(row["target_id"])

        for warning in row.get("common_validation_warnings") or []:
            warning_counts[warning] += 1

        renkin_specific = row.get("tool_specific", {}).get("renkin", {})
        recovery = renkin_specific.get("recovery") or {}
        stage = recovery.get("selected_stage") or renkin_specific.get("selected_stage")
        if stage is None:
            stage = "baseline_or_unreported"
        stage_counts[stage] += 1

        if row.get("route_found") and not row.get("validator_confirmed_route_found"):
            if row.get("not_evaluable"):
                reason = "not_evaluable"
            elif row.get("common_validation_warnings"):
                reason = "validation_warning_or_failure"
            else:
                reason = "unconfirmed_without_warning"
            failure_counts[reason] += 1

    report = {
        "schema_version": "renkin-recovery-quality/1",
        "renkin_rows": args.renkin,
        "n_targets": len(records),
        "strict_metric": {
            "validator_confirmed": len(strict_pass),
            "rate": len(strict_pass) / len(records) if records else None,
        },
        "direct_purchase_contract": {
            "route_found_depth_zero": len(direct_purchase),
            "target_ids": sorted(direct_purchase),
            "interpretation": "Complete stock purchases are reported separately; they are not added to the historical non-empty-route strict metric.",
        },
        "not_evaluable": {
            "count": len(not_evaluable),
            "target_ids": sorted(not_evaluable),
        },
        "warning_counts": dict(sorted(warning_counts.items())),
        "selected_stage_counts": dict(sorted(stage_counts.items())),
        "route_found_but_not_strict_counts": dict(sorted(failure_counts.items())),
        "next_action": "Prioritize rows with validation_warning_or_failure or unconfirmed_without_warning; keep direct-purchase and informational stereo warnings out of chemistry-fix claims.",
    }
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
