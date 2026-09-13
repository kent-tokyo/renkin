#!/usr/bin/env python3
"""Build a conservative Phase 55 failure atlas from existing comparison rows.

This is a diagnostic report, not a route solver.  It never imports competitor
routes into RENKIN and does not infer a chemical cause from a missing route.
Only signals explicitly recorded in a RENKIN row are promoted to a primary
classification; ambiguous rows remain ``unknown``.
"""

from __future__ import annotations

import argparse
import json
from collections import Counter
from pathlib import Path
from typing import Any


SCHEMA_VERSION = "renkin-phase55-failure-atlas/2"


def load_jsonl(path: Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    with path.open(encoding="utf-8") as stream:
        for line_number, line in enumerate(stream, 1):
            if not line.strip():
                continue
            value = json.loads(line)
            if not isinstance(value, dict):
                raise ValueError(f"{path}:{line_number}: row must be an object")
            rows.append(value)
    return rows


def load_paired(path: Path) -> dict[str, dict[str, Any]]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, list):
        raise ValueError(f"{path}: paired table must be a JSON array")
    result: dict[str, dict[str, Any]] = {}
    for index, row in enumerate(value, 1):
        if not isinstance(row, dict) or not isinstance(row.get("target_id"), str):
            raise ValueError(f"{path}: row {index} is missing target_id")
        target_id = row["target_id"]
        if target_id in result:
            raise ValueError(f"{path}: duplicate target_id={target_id!r}")
        result[target_id] = row
    return result


def attempts(row: dict[str, Any]) -> list[dict[str, Any]]:
    recovery = row.get("tool_specific", {}).get("renkin", {}).get("recovery", {})
    values = recovery.get("attempts", []) if isinstance(recovery, dict) else []
    return [value for value in values if isinstance(value, dict)]


def attempt_ledger(row: dict[str, Any]) -> list[dict[str, Any]]:
    """Retain observed recovery fields without inferring a chemical cause."""
    fields = (
        "stage", "coverage_tier", "termination", "max_depth", "beam_width",
        "rule_count", "routes_found", "nodes_expanded", "beam_limit_hit",
        "max_depth_reached", "routes_rejected", "unaccounted_target_element",
        "direct_generator_proposals", "direct_generator_admitted",
        "direct_generator_integrity_rejected", "direct_generator_parse_rejected",
        "direct_generator_same_target_rejected", "elapsed_ms",
    )
    return [{field: attempt.get(field) for field in fields} for attempt in attempts(row)]


def competitor_relation(row: dict[str, Any] | None) -> str:
    if row is None:
        return "not_paired"
    renkin = row.get("renkin_route_found") is True
    competitor = row.get("aizynthfinder_route_found") is True
    if renkin and competitor:
        return "both_solved"
    if renkin:
        return "renkin_only"
    if competitor:
        return "aizynthfinder_only"
    return "neither_solved"


def classify(row: dict[str, Any]) -> tuple[str, list[str]]:
    """Return a conservative primary signal and all observed signals."""
    if row.get("run_status") != "completed":
        return "run_failure", [str(row.get("run_status", "unknown"))]
    if row.get("route_found") is True:
        if row.get("validator_confirmed_route_found") is True:
            return "solved_and_validator_confirmed", []
        if row.get("validator_confirmed_route_found") is False:
            return "validator_rejected", ["validator_confirmed_route_found=false"]
        if row.get("not_evaluable") is True:
            return "validator_not_evaluable", ["not_evaluable=true"]
        return "validator_unknown", ["validator_confirmed_route_found=null"]

    observed: list[str] = []
    rows = attempts(row)
    if any(attempt.get("termination") == "deadline_exceeded" for attempt in rows):
        observed.append("deadline_exceeded")
    if any(attempt.get("max_depth_reached") is True for attempt in rows):
        observed.append("max_depth_reached")
    if any(attempt.get("beam_limit_hit") is True for attempt in rows):
        observed.append("beam_limit_hit")
    if any(attempt.get("direct_generator_proposals", 0) for attempt in rows):
        observed.append("direct_generator_proposals_present")

    if "deadline_exceeded" in observed:
        primary = "budget_exhausted"
    elif "max_depth_reached" in observed and "beam_limit_hit" in observed:
        primary = "depth_and_beam_limit"
    elif "max_depth_reached" in observed:
        primary = "depth_limit"
    elif "beam_limit_hit" in observed:
        primary = "beam_pruned"
    elif observed:
        primary = "search_exhausted_with_signal"
    else:
        primary = "unknown"
    return primary, observed


def build_atlas(rows: list[dict[str, Any]], paired: dict[str, dict[str, Any]]) -> dict[str, Any]:
    records = []
    for row in rows:
        target_id = row.get("target_id")
        if not isinstance(target_id, str) or not target_id:
            raise ValueError("RENKIN row is missing target_id")
        primary, signals = classify(row)
        ledger = attempt_ledger(row)
        records.append(
            {
                "target_id": target_id,
                "target_smiles": row.get("target_smiles"),
                "sample_rank": row.get("sample_rank"),
                "competitor_relation": competitor_relation(paired.get(target_id)),
                "run_status": row.get("run_status"),
                "route_found": row.get("route_found"),
                "validator_confirmed_route_found": row.get("validator_confirmed_route_found"),
                "primary_signal": primary,
                "observed_signals": signals,
                "attempt_count": len(ledger),
                "attempt_ledger": ledger,
                "diagnostic_scope": "observed_row_signals_only",
            }
        )
    records.sort(key=lambda record: (record["sample_rank"] is None, record["sample_rank"] or 0, record["target_id"]))
    return {
        "schema_version": SCHEMA_VERSION,
        "row_count": len(records),
        "paired_row_count": len(paired),
        "primary_signal_counts": dict(Counter(record["primary_signal"] for record in records)),
        "competitor_relation_counts": dict(Counter(record["competitor_relation"] for record in records)),
        "records": records,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--renkin-rows", type=Path, required=True)
    parser.add_argument("--paired-table", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    atlas = build_atlas(load_jsonl(args.renkin_rows), load_paired(args.paired_table))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(atlas, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({key: atlas[key] for key in atlas if key != "records"}, ensure_ascii=False, sort_keys=True))


if __name__ == "__main__":
    main()
