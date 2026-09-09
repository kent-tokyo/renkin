#!/usr/bin/env python3
"""Compare competitor route edges with a RENKIN canonical candidate pool.

This is a structural coverage diagnostic, not a route-success evaluator.  It
canonicalizes both inputs through ``renkin-canonicalize --clear-atom-maps``
and then performs exact target/precursor-multiset matching.  A missing edge
therefore means that the current pool cannot represent that edge; it does not
by itself prove that the edge is chemically valid or stock-reachable.
"""

from __future__ import annotations

import argparse
import json
import subprocess
from collections import defaultdict
from pathlib import Path


def canonicalize(binary: Path, values: list[str]) -> list[str]:
    """Canonicalize a bounded batch while preserving input/output alignment."""
    if not values:
        return []
    result = subprocess.run(
        [str(binary), "--clear-atom-maps"],
        input="".join(f"{value}\n" for value in values),
        text=True,
        capture_output=True,
        check=True,
    )
    output = result.stdout.splitlines()
    if len(output) != len(values):
        raise ValueError("canonicalizer output is not line-aligned")
    return output


def load_pool(path: Path) -> dict[str, set[tuple[str, ...]]]:
    values: dict[str, set[tuple[str, ...]]] = defaultdict(set)
    with path.open(encoding="utf-8") as stream:
        for line_number, line in enumerate(stream, 1):
            if not line.strip():
                continue
            row = json.loads(line)
            target = row.get("target_smiles")
            precursors = row.get("precursor_smiles")
            if not isinstance(target, str) or not isinstance(precursors, list) or not precursors:
                raise ValueError(f"pool line {line_number}: invalid target/precursors")
            if any(not isinstance(value, str) or not value for value in precursors):
                raise ValueError(f"pool line {line_number}: invalid precursor")
            values[target].add(tuple(sorted(precursors)))
    return values


def load_edges(path: Path, target_ids: set[str] | None) -> list[dict]:
    edges: list[dict] = []
    with path.open(encoding="utf-8") as stream:
        for line_number, line in enumerate(stream, 1):
            if not line.strip():
                continue
            row = json.loads(line)
            target_id = row.get("target_id")
            if target_ids is not None and target_id not in target_ids:
                continue
            snapshot = row.get("tool_specific", {}).get("aizynthfinder", {}).get(
                "route_edge_snapshot", []
            )
            if not isinstance(snapshot, list):
                raise ValueError(f"competitor line {line_number}: invalid route_edge_snapshot")
            for edge_index, edge in enumerate(snapshot):
                if not isinstance(edge, dict) or not isinstance(edge.get("target"), str):
                    raise ValueError(f"competitor line {line_number}: invalid edge {edge_index}")
                precursors = edge.get("precursors")
                if not isinstance(precursors, list) or not precursors or any(
                    not isinstance(value, str) or not value for value in precursors
                ):
                    raise ValueError(f"competitor line {line_number}: invalid edge precursors")
                edges.append(
                    {
                        "target_id": target_id,
                        "edge_index": edge_index,
                        "target": edge["target"],
                        "precursors": precursors,
                    }
                )
    return edges


def write_missing_target_groups(records: list[dict], path: Path) -> int:
    missing_targets = sorted(
        {record["canonical_target"] for record in records if record["pool_candidate_count"] == 0}
    )
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        "".join(
            json.dumps(
                {"group_id": f"competitor-missing-{index}", "target_id": target},
                ensure_ascii=False,
            )
            + "\n"
            for index, target in enumerate(missing_targets)
        ),
        encoding="utf-8",
    )
    return len(missing_targets)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--competitor-rows", type=Path, required=True)
    parser.add_argument("--pool", type=Path, required=True)
    parser.add_argument("--canonicalizer", type=Path, required=True)
    parser.add_argument("--target-id", action="append", dest="target_ids")
    parser.add_argument(
        "--target-list",
        type=Path,
        help="JSONL file containing target_id fields; useful for a fixed cohort",
    )
    parser.add_argument(
        "--missing-targets-output",
        type=Path,
        help="write canonical missing edge targets as renkin-pool-gen groups",
    )
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    pool = load_pool(args.pool)
    target_ids = set(args.target_ids) if args.target_ids else None
    if args.target_list:
        target_ids = set()
        with args.target_list.open(encoding="utf-8") as stream:
            for line_number, line in enumerate(stream, 1):
                if not line.strip():
                    continue
                row = json.loads(line)
                target_id = row.get("target_id")
                if not isinstance(target_id, str) or not target_id:
                    raise ValueError(f"target list line {line_number}: missing target_id")
                target_ids.add(target_id)
    edges = load_edges(args.competitor_rows, target_ids)
    raw_values = []
    for edge in edges:
        raw_values.append(edge["target"])
        raw_values.extend(edge["precursors"])
    canonical = canonicalize(args.canonicalizer, raw_values)
    cursor = 0
    records = []
    for edge in edges:
        target = canonical[cursor]
        cursor += 1
        precursors = canonical[cursor : cursor + len(edge["precursors"])]
        cursor += len(edge["precursors"])
        evaluable = target != "ERR" and all(value != "ERR" for value in precursors)
        signature = tuple(sorted(precursors)) if evaluable else None
        matches = pool.get(target, set()) if evaluable else set()
        records.append(
            {
                **edge,
                "canonical_target": target,
                "canonical_precursors": precursors,
                "canonicalization_evaluable": evaluable,
                "pool_candidate_count": len(matches),
                "exact_precursor_multiset_present": signature in matches if signature else False,
            }
        )

    result = {
        "schema_version": "renkin-competitor-route-coverage/1",
        "competitor_rows": str(args.competitor_rows),
        "pool": str(args.pool),
        "edge_count": len(records),
        "evaluable_edge_count": sum(r["canonicalization_evaluable"] for r in records),
        "edges_with_target_candidates": sum(r["pool_candidate_count"] > 0 for r in records),
        "exact_edge_match_count": sum(r["exact_precursor_multiset_present"] for r in records),
        "missing_target_count": sum(r["pool_candidate_count"] == 0 for r in records),
        "target_present_but_exact_missing_count": sum(
            r["canonicalization_evaluable"]
            and r["pool_candidate_count"] > 0
            and not r["exact_precursor_multiset_present"]
            for r in records
        ),
        "records": records,
    }
    if args.missing_targets_output:
        result["missing_target_group_count"] = write_missing_target_groups(
            records, args.missing_targets_output
        )
        result["missing_targets_output"] = str(args.missing_targets_output)
    else:
        result["missing_target_group_count"] = None
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({key: result[key] for key in (
        "edge_count", "evaluable_edge_count", "edges_with_target_candidates",
        "exact_edge_match_count", "missing_target_count",
        "target_present_but_exact_missing_count",
        "missing_target_group_count",
    )}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
