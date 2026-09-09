#!/usr/bin/env python3
"""Classify a generated candidate pool by configured-stock frontier reach.

The pool and compiled stock must use RENKIN's canonical-SMILES contract.  This
tool intentionally performs exact string membership only; it does not parse,
standardize, or infer chemical equivalence.  Its purpose is to distinguish a
one-step stock-terminal candidate from a candidate that requires downstream
search, without claiming that partial stock overlap is route success.
"""

from __future__ import annotations

import argparse
import json
from collections import defaultdict
from pathlib import Path


def load_stock(path: Path) -> set[str]:
    rows = path.read_text(encoding="utf-8").splitlines()
    # The first line is the compiled-stock magic; the second is its manifest.
    return {line for line in rows[2:] if line}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pool", type=Path, required=True)
    parser.add_argument("--stock", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    stock = load_stock(args.stock)
    by_target: dict[str, list[dict]] = defaultdict(list)
    for line_number, line in enumerate(args.pool.read_text(encoding="utf-8").splitlines(), 1):
        if not line:
            continue
        row = json.loads(line)
        target = row.get("target_id") or row.get("target_smiles")
        precursors = row.get("precursor_smiles")
        if not isinstance(target, str) or not target:
            raise ValueError(f"line {line_number}: missing target_id/target_smiles")
        if not isinstance(precursors, list) or not precursors or any(
            not isinstance(value, str) or not value for value in precursors
        ):
            raise ValueError(f"line {line_number}: invalid precursor_smiles")
        hits = sum(value in stock for value in precursors)
        by_target[target].append(
            {
                "candidate_id": row.get("candidate_id"),
                "source_rank": row.get("best_upstream_rank"),
                "precursor_count": len(precursors),
                "stock_hit_count": hits,
                "all_precursors_in_stock": hits == len(precursors),
            }
        )

    records = []
    for target in sorted(by_target):
        candidates = by_target[target]
        all_stock = [r for r in candidates if r["all_precursors_in_stock"]]
        partial = [r for r in candidates if r["stock_hit_count"] > 0]
        records.append(
            {
                "target_id": target,
                "candidate_count": len(candidates),
                "all_stock_candidate_count": len(all_stock),
                "partial_stock_candidate_count": len(partial),
                "max_stock_hit_count": max(r["stock_hit_count"] for r in candidates),
                "max_precursor_count": max(r["precursor_count"] for r in candidates),
                "all_stock_candidates": all_stock,
            }
        )

    result = {
        "schema_version": "renkin-candidate-stock-frontier/1",
        "pool": str(args.pool),
        "stock": str(args.stock),
        "stock_molecule_count": len(stock),
        "target_count": len(records),
        "candidate_count": sum(r["candidate_count"] for r in records),
        "targets_with_all_stock_candidate": sum(
            r["all_stock_candidate_count"] > 0 for r in records
        ),
        "all_stock_candidate_count": sum(r["all_stock_candidate_count"] for r in records),
        "targets_requiring_downstream_search": sum(
            r["all_stock_candidate_count"] == 0 for r in records
        ),
        "records": records,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(
        json.dumps(
            {
                key: result[key]
                for key in (
                    "target_count",
                    "candidate_count",
                    "targets_with_all_stock_candidate",
                    "all_stock_candidate_count",
                    "targets_requiring_downstream_search",
                )
            },
            ensure_ascii=False,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
