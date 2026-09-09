#!/usr/bin/env python3
"""Select bounded retro-generator candidates with a one-step stock lookahead.

The selector is intentionally offline and dependency-free.  It never invents
or rewrites a proposal: it only ranks the proposals already present in an
artifact, using exact compiled-stock membership and the artifact's own
next-target keys.  The result can be converted by
``prepare_static_retro_generator.py`` and remains an opt-in fixture.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path


def load_stock(path: Path) -> set[str]:
    rows = path.read_text(encoding="utf-8").splitlines()
    if len(rows) < 2:
        raise ValueError("compiled stock must contain a magic line and manifest line")
    return {row for row in rows[2:] if row}


def load_root_targets(path: Path) -> set[str]:
    roots = set()
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line:
            continue
        row = json.loads(line)
        target = row.get("canonical_smiles") or row.get("target_smiles")
        if not isinstance(target, str) or not target:
            raise ValueError(f"root-targets line {line_number}: missing target SMILES")
        roots.add(target)
    return roots


def stock_hits(precursors: list[str], stock: set[str]) -> int:
    return sum(precursor in stock for precursor in precursors)


def downstream_best(target: str, proposals: dict[str, list[dict]], stock: set[str]) -> tuple[int, int]:
    """Return (terminal-child count, best child stock hits) for one precursor."""
    children = proposals.get(target, [])
    if not children:
        return (0, 0)
    child_hits = [
        stock_hits(child["precursors"], stock)
        for child in children
        if isinstance(child.get("precursors"), list) and child["precursors"]
    ]
    if not child_hits:
        return (0, 0)
    max_hits = max(child_hits)
    child_width = max(len(child["precursors"]) for child in children if child.get("precursors"))
    return (int(max_hits == child_width), max_hits)


def candidate_key(candidate: dict, proposals: dict[str, list[dict]], stock: set[str]) -> tuple:
    precursors = candidate["precursors"]
    immediate_hits = stock_hits(precursors, stock)
    terminal_children = 0
    best_child_hits = 0
    for precursor in precursors:
        if precursor in stock:
            continue
        terminal, best = downstream_best(precursor, proposals, stock)
        terminal_children += terminal
        best_child_hits += best
    # Keep the established immediate-stock preference first.  Lookahead is a
    # deterministic tie-breaker before source rank, so it can improve
    # downstream recall without turning stock proximity into a chemistry cost.
    return (
        int(immediate_hits == len(precursors)),
        immediate_hits,
        terminal_children,
        best_child_hits,
        -int(candidate["provenance"].get("source_rank", 0)),
        candidate["candidate_id"],
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifact", type=Path, required=True)
    parser.add_argument("--stock", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument(
        "--root-targets",
        type=Path,
        help="JSONL sample whose root targets must be preserved without truncation",
    )
    parser.add_argument("--max-per-target", type=int, default=32)
    args = parser.parse_args(argv)
    if args.max_per_target <= 0:
        parser.error("--max-per-target must be positive")

    artifact = json.loads(args.artifact.read_text(encoding="utf-8"))
    proposals = artifact.get("proposals")
    if not isinstance(proposals, dict):
        raise ValueError("artifact.proposals must be an object")
    stock = load_stock(args.stock)
    root_targets = load_root_targets(args.root_targets) if args.root_targets else set()
    selected: dict[str, list[dict]] = {}
    for target, candidates in proposals.items():
        if not isinstance(target, str) or not target:
            raise ValueError("artifact contains an empty target key")
        if not isinstance(candidates, list):
            raise ValueError(f"proposals for {target!r} must be a list")
        for candidate in candidates:
            if not isinstance(candidate, dict) or candidate.get("target") != target:
                raise ValueError(f"candidate target mismatch under {target!r}")
            precursors = candidate.get("precursors")
            if not isinstance(precursors, list) or not precursors or any(
                not isinstance(value, str) or not value for value in precursors
            ):
                raise ValueError(f"invalid precursors under {target!r}")
        if target in root_targets:
            # Root candidates are the original search frontier and must not
            # be silently reduced by an intermediate-candidate experiment.
            selected[target] = list(candidates)
        else:
            selected[target] = sorted(
                candidates,
                key=lambda candidate: candidate_key(candidate, proposals, stock),
                reverse=True,
            )[: args.max_per_target]

    output = {
        "schema_version": artifact.get("schema_version", 1),
        "proposals": selected,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(output, ensure_ascii=False, sort_keys=True, separators=(",", ":")),
        encoding="utf-8",
    )
    print(
        json.dumps(
            {
                "targets": len(selected),
                "input_candidates": sum(len(rows) for rows in proposals.values()),
                "output_candidates": sum(len(rows) for rows in selected.values()),
                "max_per_target": args.max_per_target,
            },
            ensure_ascii=False,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
