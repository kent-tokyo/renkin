#!/usr/bin/env python3
"""Build a deterministic paired failure atlas from comparison row JSONL files.

The atlas is diagnostic only: it never turns a route-found claim into a
chemical-success claim.  It identifies AiZynthFinder-only targets and records
the RENKIN-side search signals that determine the next recovery arm.
"""

from __future__ import annotations

import argparse
import json
from collections import Counter
from pathlib import Path


def load_rows(path: str) -> dict[str, dict]:
    rows = {}
    with open(path, encoding="utf-8") as handle:
        for line in handle:
            row = json.loads(line)
            rows[row["target_id"]] = row
    return rows


def renkin_reason(row: dict) -> str:
    if row.get("validator_confirmed_route_found"):
        return "both_solved"
    diag = row.get("tool_specific", {}).get("renkin", {})
    # Older comparison rows may not have search diagnostics.  Missing data is
    # not evidence of a zero candidate pool; keep it explicitly unclassified
    # so a later recovery arm cannot be selected from an inference.
    if not diag or diag.get("matched_templates") is None:
        return "diagnostics_missing"
    if diag.get("max_depth_reached"):
        return "depth_limit"
    if diag.get("beam_limit_hit"):
        return "candidate_crowdout"
    if not diag.get("matched_templates", 0):
        return "no_candidate"
    if not diag.get("stock_hits", 0):
        return "stock_or_normalization"
    if row.get("run_status") != "completed":
        return "runtime_failure"
    return "validation_or_route_completion"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--aizynthfinder", required=True)
    parser.add_argument("--renkin", required=True)
    parser.add_argument("--direct-diagnostics")
    parser.add_argument("--output", required=True)
    args = parser.parse_args()

    ai = load_rows(args.aizynthfinder)
    renkin = load_rows(args.renkin)
    direct_ids = set()
    if args.direct_diagnostics:
        diagnostic_report = json.load(open(args.direct_diagnostics, encoding="utf-8"))
        direct_ids = {
            x["target_id"]
            for x in diagnostic_report["records"]
            if x.get("status") == "depth_zero_stock_route"
        }
    records = []
    for target_id in sorted(set(ai) | set(renkin)):
        a = ai.get(target_id)
        r = renkin.get(target_id)
        if not a or not r:
            continue
        ai_solved = bool(a.get("all_leaves_in_configured_stock"))
        renkin_solved = bool(r.get("validator_confirmed_route_found")) or target_id in direct_ids
        if not (ai_solved and not renkin_solved):
            continue
        diag = r.get("tool_specific", {}).get("renkin", {})
        records.append(
            {
                "target_id": target_id,
                "target_smiles": r.get("target_smiles"),
                "reason": renkin_reason(r),
                "run_status": r.get("run_status"),
                "route_found": r.get("route_found"),
                "validator_confirmed_route_found": r.get(
                    "validator_confirmed_route_found"
                ),
                "direct_purchase_in_stock": target_id in direct_ids,
                "matched_templates": diag.get("matched_templates"),
                "stock_hits": diag.get("stock_hits"),
                "nodes_expanded": diag.get("nodes_expanded"),
                "max_depth_reached": diag.get("max_depth_reached"),
                "beam_limit_hit": diag.get("beam_limit_hit"),
                "total_elapsed_ms": r.get("total_elapsed_ms"),
            }
        )

    report = {
        "schema_version": "renkin-paired-failure-atlas/1",
        "aizynthfinder_rows": args.aizynthfinder,
        "renkin_rows": args.renkin,
        "aizynthfinder_only_count": len(records),
        "reason_counts": dict(sorted(Counter(x["reason"] for x in records).items())),
        "records": records,
    }
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({k: report[k] for k in ("aizynthfinder_only_count", "reason_counts")}, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
