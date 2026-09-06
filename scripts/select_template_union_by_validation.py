#!/usr/bin/env python3
"""Select a bounded TRAIN-derived template union on a disjoint validation pool.

The candidate rules must already have been generated from TRAIN.  This tool
uses validation labels only to select among those fixed rules; it never creates
or edits SMIRKS from validation or test molecules.  Selection is deterministic:
maximise newly covered validation groups, then minimise marginal candidate
growth, prefer greater TRAIN support, and finally sort by stable template ID.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
from collections import OrderedDict, defaultdict
from pathlib import Path


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def template_id(smirks: str) -> str:
    return "smirks-sha256:" + hashlib.sha256(smirks.strip().encode()).hexdigest()


def load_templates(path: Path) -> OrderedDict[str, tuple[str, int]]:
    rows: OrderedDict[str, tuple[str, int]] = OrderedDict()
    seen_smirks = set()
    for line_number, raw in enumerate(path.read_text().splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        fields = line.split("\t")
        if len(fields) != 2:
            raise ValueError(f"{path}:{line_number}: expected SMIRKS<TAB>count")
        smirks, count_text = fields
        count = int(count_text)
        if count <= 0:
            raise ValueError(f"{path}:{line_number}: count must be positive")
        if smirks in seen_smirks:
            raise ValueError(f"{path}:{line_number}: duplicate SMIRKS")
        seen_smirks.add(smirks)
        rows[template_id(smirks)] = (smirks, count)
    return rows


def load_jsonl(path: Path) -> list[dict]:
    rows = []
    for line_number, raw in enumerate(path.read_text().splitlines(), 1):
        if not raw.strip():
            continue
        try:
            value = json.loads(raw)
        except json.JSONDecodeError as exc:
            raise ValueError(f"{path}:{line_number}: invalid JSON: {exc}") from exc
        if not isinstance(value, dict):
            raise ValueError(f"{path}:{line_number}: expected a JSON object")
        rows.append(value)
    return rows


def group_identity(rows: list[dict], required_prefix: str) -> dict[str, tuple]:
    groups = {}
    for row in rows:
        group_id = row.get("group_id")
        if not isinstance(group_id, str) or not group_id.startswith(required_prefix):
            raise ValueError(
                f"group_id {group_id!r} does not match required validation prefix "
                f"{required_prefix!r}"
            )
        if group_id in groups:
            raise ValueError(f"duplicate group_id {group_id!r}")
        groups[group_id] = (
            row.get("target_id"),
            row.get("target_smiles"),
            row.get("proposal_status"),
        )
    return groups


def load_labels(path: Path, allowed_groups: set[str]) -> dict[str, set[tuple[str, ...]]]:
    labels = {}
    for row in load_jsonl(path):
        group_id = row.get("group_id")
        if group_id not in allowed_groups:
            continue
        if group_id in labels:
            raise ValueError(f"duplicate label group_id {group_id!r}")
        correct = row.get("correct_precursor_sets")
        if not isinstance(correct, list) or not correct:
            raise ValueError(f"group_id {group_id!r}: missing correct_precursor_sets")
        labels[group_id] = {tuple(sorted(value)) for value in correct}
    missing = allowed_groups - labels.keys()
    if missing:
        raise ValueError(f"labels missing {len(missing)} validation groups")
    return labels


def candidate_key(row: dict) -> tuple[str, str]:
    return row["group_id"], row["candidate_id"]


def is_positive(row: dict, labels: dict[str, set[tuple[str, ...]]]) -> bool:
    return tuple(sorted(row["precursor_smiles"])) in labels[row["group_id"]]


def select(
    base_templates: OrderedDict[str, tuple[str, int]],
    additional_templates: OrderedDict[str, tuple[str, int]],
    base_pool: list[dict],
    full_pool: list[dict],
    labels: dict[str, set[tuple[str, ...]]],
    max_candidate_growth: float,
) -> tuple[list[str], dict]:
    if max_candidate_growth < 1.0 or not math.isfinite(max_candidate_growth):
        raise ValueError("max_candidate_growth must be finite and at least 1.0")

    additional = {
        tid: row for tid, row in additional_templates.items() if tid not in base_templates
    }
    base_candidates = {candidate_key(row) for row in base_pool}
    budget = math.floor(len(base_candidates) * max_candidate_growth)
    base_positive_keys = {
        candidate_key(row) for row in base_pool if is_positive(row, labels)
    }
    base_positive_groups = {group_id for group_id, _candidate_id in base_positive_keys}
    covered = set(base_positive_groups)

    candidate_rows_by_template: dict[str, set[tuple[str, str]]] = defaultdict(set)
    positive_groups_by_template: dict[str, set[str]] = defaultdict(set)
    full_candidate_keys = set()
    full_positive_keys = set()
    for row in full_pool:
        key = candidate_key(row)
        full_candidate_keys.add(key)
        positive = is_positive(row, labels)
        if positive:
            full_positive_keys.add(key)
        sources = row.get("sources")
        if not isinstance(sources, list):
            raise ValueError(f"candidate {key!r}: missing sources")
        for source in sources:
            tid = source.get("template_id")
            if tid not in additional:
                continue
            candidate_rows_by_template[tid].add(key)
            if positive and row["group_id"] not in covered:
                positive_groups_by_template[tid].add(row["group_id"])

    missing_base_candidates = base_candidates - full_candidate_keys
    if missing_base_candidates:
        raise ValueError(
            "full union pool is missing candidates from the baseline pool"
        )

    selected = []
    selected_candidates = set(base_candidates)
    selection_steps = []
    while True:
        choices = []
        for tid, (_smirks, train_count) in additional.items():
            if tid in selected:
                continue
            gain = positive_groups_by_template[tid] - covered
            added_candidates = candidate_rows_by_template[tid] - selected_candidates
            if not gain or len(selected_candidates) + len(added_candidates) > budget:
                continue
            choices.append(
                (-len(gain), len(added_candidates), -train_count, tid, gain, added_candidates)
            )
        if not choices:
            break
        _negative_gain, cost, negative_support, tid, gain, added_candidates = min(choices)
        selected.append(tid)
        covered.update(gain)
        selected_candidates.update(added_candidates)
        selection_steps.append(
            {
                "template_id": tid,
                "train_support": -negative_support,
                "new_positive_group_count": len(gain),
                "marginal_candidate_count": cost,
                "cumulative_candidate_count": len(selected_candidates),
            }
        )

    selected_positive_groups = {
        group_id
        for group_id, _candidate_id in selected_candidates
        if (group_id, _candidate_id) in full_positive_keys | base_positive_keys
    }
    accounting = {
        "base_template_count": len(base_templates),
        "additional_input_count": len(additional_templates),
        "additional_unique_count": len(additional),
        "selected_template_count": len(selected),
        "base_candidate_count": len(base_candidates),
        "selected_candidate_count": len(selected_candidates),
        "candidate_growth": len(selected_candidates) / len(base_candidates)
        if base_candidates
        else None,
        "candidate_budget": budget,
        "max_candidate_growth": max_candidate_growth,
        "base_positive_group_count": len(base_positive_groups),
        "selected_positive_group_count": len(selected_positive_groups),
        "new_positive_group_count": len(selected_positive_groups)
        - len(base_positive_groups),
        "selection_steps": selection_steps,
    }
    return selected, accounting


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-templates", required=True, type=Path)
    parser.add_argument("--additional-templates", required=True, type=Path)
    parser.add_argument("--base-pool", required=True, type=Path)
    parser.add_argument("--full-pool", required=True, type=Path)
    parser.add_argument("--base-groups", required=True, type=Path)
    parser.add_argument("--full-groups", required=True, type=Path)
    parser.add_argument("--labels", required=True, type=Path)
    parser.add_argument("--required-group-prefix", required=True)
    parser.add_argument("--max-candidate-growth", required=True, type=float)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--manifest", required=True, type=Path)
    args = parser.parse_args()

    base_groups = group_identity(load_jsonl(args.base_groups), args.required_group_prefix)
    full_groups = group_identity(load_jsonl(args.full_groups), args.required_group_prefix)
    if base_groups != full_groups:
        raise ValueError("base and full candidate pools do not cover identical validation groups")
    failed_groups = {
        group_id: identity[2]
        for group_id, identity in base_groups.items()
        if identity[2] != "ok"
    }
    if failed_groups:
        raise ValueError(
            f"validation group index contains {len(failed_groups)} non-ok proposal statuses"
        )
    labels = load_labels(args.labels, set(base_groups))
    base_templates = load_templates(args.base_templates)
    additional_templates = load_templates(args.additional_templates)
    base_pool = load_jsonl(args.base_pool)
    full_pool = load_jsonl(args.full_pool)
    for name, pool in (("base", base_pool), ("full", full_pool)):
        unknown = {row.get("group_id") for row in pool} - base_groups.keys()
        if unknown:
            raise ValueError(f"{name} pool contains unknown validation groups")

    selected, accounting = select(
        base_templates,
        additional_templates,
        base_pool,
        full_pool,
        labels,
        args.max_candidate_growth,
    )
    output_rows = list(base_templates.values()) + [additional_templates[tid] for tid in selected]
    output_text = "\n".join(
        [
            "# RENKIN validation-selected TRAIN-derived template union",
            f"# Maximum validation candidate growth: {args.max_candidate_growth}",
            "# Format: SMIRKS<TAB>count",
            *(f"{smirks}\t{count}" for smirks, count in output_rows),
            "",
        ]
    )
    args.output.write_text(output_text)
    manifest = {
        "schema_version": 1,
        "selection_policy": (
            "max_new_positive_groups_then_min_marginal_candidates_then_"
            "max_train_support_then_template_id"
        ),
        "required_group_prefix": args.required_group_prefix,
        "benchmark_targets_used_for_rule_generation": False,
        "validation_labels_used_for_selection": True,
        "test_or_holdout_used_for_selection": False,
        "inputs": {
            name: {"path": str(path), "sha256": sha256(path)}
            for name, path in (
                ("base_templates", args.base_templates),
                ("additional_templates", args.additional_templates),
                ("base_pool", args.base_pool),
                ("full_pool", args.full_pool),
                ("base_groups", args.base_groups),
                ("full_groups", args.full_groups),
                ("labels", args.labels),
            )
        },
        "selected_template_ids": selected,
        "accounting": accounting,
        "output_path": str(args.output),
        "output_template_count": len(output_rows),
        "output_sha256": hashlib.sha256(output_text.encode()).hexdigest(),
        "selector_sha256": sha256(Path(__file__)),
    }
    args.manifest.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    print(json.dumps(manifest, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
