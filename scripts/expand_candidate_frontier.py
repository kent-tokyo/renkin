#!/usr/bin/env python3
"""Expand a RENKIN candidate pool through bounded intermediate frontiers.

Each round runs the existing ``renkin-pool-gen`` on newly discovered
precursors, then unions the rows with all earlier rounds.  No candidate is
re-ranked or removed by this tool.  An optional compiled stock file prevents
stock-terminal molecules from becoming another generation target.  This is
offline fixture preparation; it does not use labels or competitor routes.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import tempfile
from pathlib import Path


def load_stock(path: Path | None) -> set[str]:
    if path is None:
        return set()
    return {line for line in path.read_text(encoding="utf-8").splitlines()[2:] if line}


def read_jsonl(path: Path) -> list[dict]:
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]


def target_value(row: dict) -> str | None:
    return row.get("target_smiles") or row.get("canonical_smiles") or row.get("target_id")


def select_frontier_targets(rows: list[dict], stock: set[str], limit: int) -> list[str]:
    """Select non-stock intermediates using parent-candidate stock evidence.

    A precursor is a better next-round target when its parent candidate is
    already close to the configured stock.  The score is only an input
    scheduling policy: every candidate row remains in the union artifact.
    """
    evidence: dict[str, list[tuple[float, int, int]]] = {}
    for row in rows:
        precursors = row.get("precursor_smiles", [])
        if not isinstance(precursors, list) or not precursors:
            continue
        hits = sum(value in stock for value in precursors)
        ratio = hits / len(precursors)
        source_rank = row.get("best_upstream_rank", 0)
        if not isinstance(source_rank, int):
            source_rank = 0
        for precursor in precursors:
            if isinstance(precursor, str) and precursor and precursor not in stock:
                evidence.setdefault(precursor, []).append((ratio, hits, source_rank))
    ranked = []
    for target, values in evidence.items():
        best_ratio, best_hits, best_rank = max(
            values, key=lambda value: (value[0], value[1], -value[2])
        )
        ranked.append(
            (
                best_ratio,
                best_hits,
                len(values),
                -best_rank,
                target,
            )
        )
    ranked.sort(reverse=True)
    return [target for *_score, target in ranked[:limit]]


def run_pool_gen(binary: Path, groups: Path, templates: Path, directory: Path, round_number: int) -> list[dict]:
    pool = directory / f"round-{round_number}-pool.jsonl"
    groups_out = directory / f"round-{round_number}-groups.jsonl"
    manifest = directory / f"round-{round_number}-manifest.json"
    subprocess.run(
        [
            str(binary),
            "--groups",
            str(groups),
            "--templates",
            str(templates),
            "--pool-output",
            str(pool),
            "--groups-output",
            str(groups_out),
            "--manifest-output",
            str(manifest),
        ],
        check=True,
    )
    return read_jsonl(pool)


def write_groups(path: Path, targets: list[str], round_number: int) -> None:
    path.write_text(
        "".join(
            json.dumps(
                {"group_id": f"frontier-r{round_number}-{index}", "target_id": target},
                ensure_ascii=False,
            )
            + "\n"
            for index, target in enumerate(targets)
        ),
        encoding="utf-8",
    )


def expand(
    binary: Path,
    templates: Path,
    initial_groups: Path,
    output: Path,
    rounds: int,
    max_targets_per_round: int,
    stock: set[str],
) -> dict:
    if rounds <= 0 or max_targets_per_round <= 0:
        raise ValueError("rounds and max_targets_per_round must be positive")
    initial = read_jsonl(initial_groups)
    targets = sorted(
        {
            target_value(row)
            for row in initial
        }
    )
    if any(not isinstance(target, str) or not target for target in targets):
        raise ValueError("initial groups contain a missing target")

    all_rows: list[dict] = []
    seen_candidates: set[str] = set()
    round_summaries = []
    with tempfile.TemporaryDirectory(prefix="renkin-frontier-") as temp:
        directory = Path(temp)
        for round_number in range(rounds):
            targets = [target for target in targets if target not in stock][:max_targets_per_round]
            if not targets:
                break
            groups = directory / f"round-{round_number}-groups.jsonl"
            write_groups(groups, targets, round_number)
            rows = run_pool_gen(binary, groups, templates, directory, round_number)
            added = 0
            next_targets: set[str] = set()
            for row in rows:
                candidate_id = row.get("candidate_id")
                if not isinstance(candidate_id, str) or not candidate_id:
                    raise ValueError("pool row has no candidate_id")
                if candidate_id not in seen_candidates:
                    seen_candidates.add(candidate_id)
                    all_rows.append(row)
                    added += 1
                for precursor in row.get("precursor_smiles", []):
                    if isinstance(precursor, str) and precursor not in stock:
                        next_targets.add(precursor)
            round_summaries.append(
                {
                    "round": round_number,
                    "input_target_count": len(targets),
                    "pool_row_count": len(rows),
                    "new_row_count": added,
                    "next_target_count_before_cap": len(next_targets),
                }
            )
            targets = select_frontier_targets(rows, stock, max_targets_per_round)

    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text("".join(json.dumps(row, ensure_ascii=False) + "\n" for row in all_rows), encoding="utf-8")
    digest = hashlib.sha256(output.read_bytes()).hexdigest()
    result = {
        "schema_version": "renkin-candidate-frontier/1",
        "input_groups": str(initial_groups),
        "templates": str(templates),
        "rounds_requested": rounds,
        "rounds_completed": len(round_summaries),
        "stock_excluded_count": len(stock),
        "candidate_row_count": len(all_rows),
        "candidate_jsonl_sha256": f"sha256:{digest}",
        "rounds": round_summaries,
    }
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pool-gen", type=Path, required=True)
    parser.add_argument("--templates", type=Path, required=True)
    parser.add_argument("--groups", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--rounds", type=int, default=2)
    parser.add_argument("--max-targets-per-round", type=int, default=256)
    parser.add_argument("--stock", type=Path)
    args = parser.parse_args()
    result = expand(
        args.pool_gen,
        args.templates,
        args.groups,
        args.output,
        args.rounds,
        args.max_targets_per_round,
        load_stock(args.stock),
    )
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
