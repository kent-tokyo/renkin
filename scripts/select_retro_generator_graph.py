#!/usr/bin/env python3
"""Select bounded direct-generator candidates using candidate-graph reachability.

The selector only reorders rows already present in a JSONL candidate pool. It
preserves every root-target row and caps only intermediate targets.  Scores use
exact compiled-stock membership, child-candidate count, and child stock
termination; no validation labels or competitor routes are read.
"""

from __future__ import annotations

import argparse
import json
from collections import defaultdict
from pathlib import Path


def load_stock(path: Path) -> set[str]:
    rows = path.read_text(encoding="utf-8").splitlines()
    if len(rows) < 2:
        raise ValueError("compiled stock must contain a magic line and manifest line")
    return {row for row in rows[2:] if row}


def load_targets(path: Path) -> set[str]:
    targets = set()
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip():
            continue
        row = json.loads(line)
        target = row.get("canonical_smiles") or row.get("canonical_target") or row.get("target_smiles")
        if not isinstance(target, str) or not target:
            raise ValueError(f"root-targets line {line_number}: missing target SMILES")
        targets.add(target)
    return targets


def load_rows(path: Path) -> list[dict]:
    rows = []
    with path.open(encoding="utf-8") as stream:
        for line_number, line in enumerate(stream, 1):
            if not line.strip():
                continue
            row = json.loads(line)
            if not isinstance(row, dict):
                raise ValueError(f"line {line_number}: row must be an object")
            target = row.get("target_smiles")
            precursors = row.get("precursor_smiles")
            if not isinstance(target, str) or not target:
                raise ValueError(f"line {line_number}: missing target_smiles")
            if not isinstance(precursors, list) or not precursors or any(
                not isinstance(value, str) or not value for value in precursors
            ):
                raise ValueError(f"line {line_number}: invalid precursor_smiles")
            rows.append(row)
    return rows


def load_artifact(path: Path) -> tuple[dict, list[dict]]:
    """Load a static-generator artifact as selector rows.

    The JSONL pool and the static generator artifact carry the same proposal
    information in different envelopes.  Keep the selector's ranking logic
    single-sourced and retain the original candidate object for artifact
    output so no provenance fields are lost.
    """
    artifact = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(artifact, dict) or not isinstance(artifact.get("proposals"), dict):
        raise ValueError("artifact must contain a proposals object")
    rows = []
    for target, candidates in artifact["proposals"].items():
        if not isinstance(target, str) or not target or not isinstance(candidates, list):
            raise ValueError("artifact contains invalid target proposals")
        for candidate in candidates:
            if not isinstance(candidate, dict) or candidate.get("target") != target:
                raise ValueError(f"candidate target mismatch under {target!r}")
            precursors = candidate.get("precursors")
            if not isinstance(precursors, list) or not precursors or any(
                not isinstance(value, str) or not value for value in precursors
            ):
                raise ValueError(f"invalid precursors under {target!r}")
            provenance = candidate.get("provenance", {})
            source_rank = provenance.get("source_rank", 0) if isinstance(provenance, dict) else 0
            rows.append(
                {
                    "target_smiles": target,
                    "precursor_smiles": precursors,
                    "candidate_id": candidate.get("candidate_id", ""),
                    "best_upstream_rank": source_rank,
                    "_artifact_candidate": candidate,
                }
            )
    return artifact, rows


def candidate_reachability(
    row: dict,
    by_target: dict[str, list[dict]],
    stock: set[str],
    depth: int,
    memo: dict[tuple[str, int], tuple[bool, int]],
) -> tuple[bool, int]:
    """Return whether a candidate can reach stock and its reachable count."""
    reachable = 0
    for precursor in row["precursor_smiles"]:
        if precursor in stock:
            reachable += 1
        elif depth > 0 and target_reachability(precursor, by_target, stock, depth - 1, memo)[0]:
            reachable += 1
    return reachable == len(row["precursor_smiles"]), reachable


def target_reachability(
    target: str,
    by_target: dict[str, list[dict]],
    stock: set[str],
    depth: int,
    memo: dict[tuple[str, int], tuple[bool, int]],
) -> tuple[bool, int]:
    key = (target, depth)
    if key in memo:
        return memo[key]
    if target in stock:
        result = (True, 1)
    elif depth < 0:
        result = (False, 0)
    else:
        # Install a conservative provisional value before following edges so
        # cyclic candidate graphs cannot recurse indefinitely.
        memo[key] = (False, 0)
        candidate_scores = [
            candidate_reachability(row, by_target, stock, depth, memo)
            for row in by_target.get(target, [])
        ]
        result = (
            any(full for full, _reachable in candidate_scores),
            max((reachable for _full, reachable in candidate_scores), default=0),
        )
    memo[key] = result
    return result


def graph_key(
    row: dict,
    by_target: dict[str, list[dict]],
    stock: set[str],
    lookahead_depth: int = 0,
    memo: dict[tuple[str, int], tuple[bool, int]] | None = None,
) -> tuple:
    precursors = row["precursor_smiles"]
    immediate_hits = sum(value in stock for value in precursors)
    child_rows = [child for value in precursors for child in by_target.get(value, [])]
    terminal_children = sum(
        all(value in stock for value in child["precursor_smiles"]) for child in child_rows
    )
    child_stock_hits = [
        sum(value in stock for value in child["precursor_smiles"])
        / len(child["precursor_smiles"])
        for child in child_rows
    ]
    source_rank = row.get("best_upstream_rank", 0)
    if not isinstance(source_rank, int):
        source_rank = 0
    if memo is None:
        memo = {}
    reachable_full, reachable_count = candidate_reachability(
        row, by_target, stock, lookahead_depth, memo
    )
    return (
        int(immediate_hits == len(precursors)),
        immediate_hits,
        int(reachable_full),
        reachable_count,
        terminal_children,
        max(child_stock_hits, default=0.0),
        len(child_rows),
        -source_rank,
        row.get("candidate_id", ""),
    )


def diversity_key(row: dict, source_rank_bucket: int) -> tuple[int, int]:
    """Return a label-free proxy for distinct proposal families."""
    source_rank = row.get("best_upstream_rank", 0)
    if not isinstance(source_rank, int):
        source_rank = 0
    return (len(row["precursor_smiles"]), source_rank // source_rank_bucket)


def select_diverse(
    candidates: list[dict],
    by_target: dict[str, list[dict]],
    stock: set[str],
    max_per_target: int,
    lookahead_depth: int,
    source_rank_bucket: int,
    memo: dict[tuple[str, int], tuple[bool, int]],
) -> list[dict]:
    ranked = sorted(
        candidates,
        key=lambda row: graph_key(row, by_target, stock, lookahead_depth, memo),
        reverse=True,
    )
    selected = []
    seen_diversity = set()
    for row in ranked:
        key = diversity_key(row, source_rank_bucket)
        if key in seen_diversity:
            continue
        seen_diversity.add(key)
        selected.append(row)
        if len(selected) == max_per_target:
            return selected
    selected_ids = {row.get("candidate_id") for row in selected}
    selected.extend(
        row for row in ranked if row.get("candidate_id") not in selected_ids
    )
    return selected[:max_per_target]


def select(
    rows: list[dict],
    stock: set[str],
    roots: set[str],
    max_per_target: int,
    lookahead_depth: int = 0,
    source_rank_bucket: int | None = None,
    preserve_targets: set[str] | None = None,
) -> tuple[list[dict], dict]:
    by_target: dict[str, list[dict]] = defaultdict(list)
    for row in rows:
        by_target[row["target_smiles"]].append(row)
    selected = []
    capped_targets = 0
    memo: dict[tuple[str, int], tuple[bool, int]] = {}
    for target in sorted(by_target):
        candidates = by_target[target]
        if target in roots or (preserve_targets is not None and target in preserve_targets):
            selected.extend(candidates)
            continue
        if source_rank_bucket is not None:
            ranked = select_diverse(
                candidates,
                by_target,
                stock,
                max_per_target,
                lookahead_depth,
                source_rank_bucket,
                memo,
            )
        else:
            ranked = sorted(
                candidates,
                key=lambda row: graph_key(row, by_target, stock, lookahead_depth, memo),
                reverse=True,
            )[:max_per_target]
        selected.extend(ranked)
        if len(candidates) > max_per_target:
            capped_targets += 1
    return selected, {
        "input_rows": len(rows),
        "output_rows": len(selected),
        "target_count": len(by_target),
        "root_target_count": len(roots & set(by_target)),
        "capped_intermediate_target_count": capped_targets,
        "max_per_target": max_per_target,
        "lookahead_depth": lookahead_depth,
        "source_rank_bucket": source_rank_bucket,
        "preserved_target_count": len((preserve_targets or set()) & set(by_target)),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--stock", type=Path, required=True)
    parser.add_argument("--root-targets", type=Path, required=True)
    parser.add_argument(
        "--preserve-targets",
        type=Path,
        help="opt-in JSONL target set to keep unbounded; for diagnostics only",
    )
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--max-per-target", type=int, default=32)
    parser.add_argument("--lookahead-depth", type=int, default=0)
    parser.add_argument(
        "--source-rank-bucket",
        type=int,
        help="opt-in diversity selection bucket; combines source-rank band and precursor count",
    )
    args = parser.parse_args()
    if args.max_per_target <= 0 or args.lookahead_depth < 0 or (
        args.source_rank_bucket is not None and args.source_rank_bucket <= 0
    ):
        parser.error("invalid max-per-target, lookahead-depth, or source-rank-bucket")
    raw = args.input.read_text(encoding="utf-8")
    if raw.lstrip().startswith("{"):
        artifact, rows = load_artifact(args.input)
        selected, summary = select(
            rows,
            load_stock(args.stock),
            load_targets(args.root_targets),
            args.max_per_target,
            args.lookahead_depth,
            args.source_rank_bucket,
            load_targets(args.preserve_targets) if args.preserve_targets else None,
        )
        by_target = defaultdict(list)
        for row in selected:
            by_target[row["target_smiles"]].append(row["_artifact_candidate"])
        artifact["proposals"] = dict(by_target)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(
            json.dumps(artifact, ensure_ascii=False, sort_keys=True, separators=(",", ":")),
            encoding="utf-8",
        )
        summary["format"] = "static_generator_artifact"
        print(json.dumps(summary, ensure_ascii=False))
        return 0
    selected, summary = select(
        load_rows(args.input),
        load_stock(args.stock),
        load_targets(args.root_targets),
        args.max_per_target,
        args.lookahead_depth,
        args.source_rank_bucket,
        load_targets(args.preserve_targets) if args.preserve_targets else None,
    )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        "".join(json.dumps(row, ensure_ascii=False) + "\n" for row in selected),
        encoding="utf-8",
    )
    print(json.dumps(summary, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
