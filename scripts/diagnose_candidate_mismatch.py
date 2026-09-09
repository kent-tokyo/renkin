#!/usr/bin/env python3
"""Classify competitor edges whose target is present but precursor multiset differs.

This is deliberately a label-free candidate-pool diagnostic. It does not claim
that a partial overlap is chemically equivalent or that a candidate reaches
stock; those require the engine's structural and stock validators.
"""

from __future__ import annotations

import argparse
import json
from collections import Counter
from pathlib import Path


def _multiset_overlap(expected: list[str], actual: list[str]) -> int:
    return sum((Counter(expected) & Counter(actual)).values())


def _load_stock(path: Path | None) -> set[str] | None:
    if path is None:
        return None
    lines = path.read_text().splitlines()
    return {line.strip() for line in lines[1:] if line.strip() and not line.startswith("{")}


def _stock_reachable(target: str, proposals: dict, stock: set[str], memo: dict[str, bool], visiting: set[str]) -> bool:
    if target in stock:
        return True
    if target in memo:
        return memo[target]
    if target in visiting:
        return False
    visiting.add(target)
    result = any(
        all(_stock_reachable(child, proposals, stock, memo, visiting) for child in candidate.get("precursors", []))
        for candidate in proposals.get(target, [])
    )
    visiting.remove(target)
    memo[target] = result
    return result


def classify(atlas: dict, artifact: dict, stock: set[str] | None = None) -> dict:
    proposals = artifact["proposals"]
    reachability_memo: dict[str, bool] = {}
    records = []
    for edge in atlas["records"]:
        if not edge.get("canonicalization_evaluable") or edge.get("exact_precursor_multiset_present"):
            continue
        target = edge["canonical_target"]
        candidates = proposals.get(target, [])
        expected = edge["canonical_precursors"]
        scored = []
        for candidate in candidates:
            precursors = candidate.get("precursors", [])
            overlap = _multiset_overlap(expected, precursors)
            scored.append(
                {
                    "candidate_id": candidate.get("candidate_id"),
                    "overlap_count": overlap,
                    "expected_count": len(expected),
                    "source_rank": candidate.get("provenance", {}).get("source_rank"),
                }
            )
        best_overlap = max((item["overlap_count"] for item in scored), default=0)
        expected_count = len(expected)
        if not candidates:
            category = "target_missing"
        elif best_overlap == 0:
            category = "no_precursor_overlap"
        elif best_overlap < expected_count:
            category = "partial_precursor_overlap"
        else:
            category = "canonicalization_or_representation_mismatch"
        missing_expected = list((Counter(expected) - Counter(
            max(
                (candidate.get("precursors", []) for candidate in candidates),
                key=lambda values: _multiset_overlap(expected, values),
                default=[],
            )
        )).elements())
        missing_stock_count = (
            sum(value in stock for value in missing_expected) if stock is not None else None
        )
        if category == "partial_precursor_overlap" and missing_stock_count:
            action_class = "partial_missing_stock_precursor"
        elif category == "partial_precursor_overlap":
            action_class = "partial_downstream_precursor"
        else:
            action_class = category
        records.append(
            {
                "target_id": edge["target_id"],
                "edge_index": edge["edge_index"],
                "canonical_target": target,
                "expected_precursors": expected,
                "candidate_count": len(candidates),
                "best_overlap_count": best_overlap,
                "expected_precursor_count": expected_count,
                "best_overlap_fraction": (best_overlap / expected_count if expected_count else 1.0),
                "missing_expected_precursors": missing_expected,
                "missing_expected_stock_count": missing_stock_count,
                "action_class": action_class,
                "best_source_rank": min(
                    (item["source_rank"] for item in scored if item["source_rank"] is not None),
                    default=None,
                ),
                "stock_reachable": (
                    _stock_reachable(target, proposals, stock, reachability_memo, set())
                    if stock is not None
                    else None
                ),
                "category": category,
            }
        )
    counts = Counter(record["category"] for record in records)
    return {
        "schema_version": "renkin-candidate-mismatch-diagnostic/1",
        "mismatch_count": len(records),
        "category_counts": dict(sorted(counts.items())),
        "records": records,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--atlas", required=True, type=Path)
    parser.add_argument("--artifact", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--stock", type=Path)
    args = parser.parse_args()
    result = classify(
        json.loads(args.atlas.read_text()),
        json.loads(args.artifact.read_text()),
        _load_stock(args.stock),
    )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({"mismatch_count": result["mismatch_count"], "category_counts": result["category_counts"]}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
