#!/usr/bin/env python3
"""Partition a partial-overlap diagnostic into reproducible family cohorts.

This is an offline preparation step.  It does not alter candidates, infer a
chemical reaction family, or claim route success.  The family label is copied
from ``diagnose_partial_overlap.py``'s explicitly heuristic field, and every
requested target must be present exactly once in the subset file.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any


def sha256_file(path: Path) -> str:
    return "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()


def load_subset(path: Path) -> set[str]:
    targets: set[str] = set()
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip():
            continue
        row = json.loads(line)
        target = row.get("canonical_target")
        if not isinstance(target, str) or not target:
            raise ValueError(f"line {line_number}: canonical_target is required")
        if target in targets:
            raise ValueError(f"line {line_number}: duplicate canonical_target {target!r}")
        targets.add(target)
    return targets


def partition(diagnostic: dict[str, Any], subset: set[str]) -> dict[str, Any]:
    records = diagnostic.get("records")
    if not isinstance(records, list):
        raise ValueError("diagnostic.records must be a list")
    indexed: dict[str, dict[str, Any]] = {}
    for record in records:
        target = record.get("canonical_target")
        family = record.get("reaction_family_proxy")
        if not isinstance(target, str) or not isinstance(family, str):
            raise ValueError("each diagnostic record needs target and family")
        if target in indexed:
            raise ValueError(f"duplicate diagnostic target {target!r}")
        indexed[target] = record

    missing = sorted(subset - indexed.keys())
    if missing:
        raise ValueError(f"subset targets missing from diagnostic: {missing[:3]}")
    selected = [indexed[target] for target in sorted(subset)]
    cohorts: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for record in selected:
        cohorts[record["reaction_family_proxy"]].append(record)
    return {
        "schema_version": "renkin-partial-overlap-family-cohorts/1",
        "evidence_level": diagnostic.get("evidence_level", "unknown"),
        "route_validity_unchanged": True,
        "subset_count": len(selected),
        "family_counts": dict(sorted(Counter(record["reaction_family_proxy"] for record in selected).items())),
        "cohorts": {
            family: {
                "count": len(rows),
                "records": rows,
            }
            for family, rows in sorted(cohorts.items())
        },
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--diagnostic", type=Path, required=True)
    parser.add_argument("--subset", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    result = partition(
        json.loads(args.diagnostic.read_text(encoding="utf-8")),
        load_subset(args.subset),
    )
    result["inputs"] = {
        "diagnostic_sha256": sha256_file(args.diagnostic),
        "subset_sha256": sha256_file(args.subset),
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"subset_count": result["subset_count"], "family_counts": result["family_counts"]}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
