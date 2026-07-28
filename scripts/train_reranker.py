#!/usr/bin/env python3
"""
Train and evaluate a LambdaMART candidate reranker over an offline
candidate pool exported by `renkin::pool_export` (see src/pool_export.rs).

This script does NOT decide what pool to run it against, and it does not
gate anything -- it is a mechanism, not a decision. Whether/when to run it
against a real 100/500/full-target pool, and what to conclude from the
result, is a separate call (see this repo's staged candidate-pool gate).

Two identifiers matter and are never conflated:
  - `target_id`: the canonical target structure. Used ONLY as the
    leakage-safe train/val/test split key (SHA-256(target_id) mod 100).
  - `group_id`: one dataset reaction/example. Used ONLY as the LightGBM
    ranking group. The same target structure can be the product of two
    different literature reactions -- those share `target_id` (same split)
    but must each get their own `group_id` (separate ranking groups).

Pipeline:
  1. Load a JSONL candidate pool (one row per candidate, schema per
     PoolManifest.feature_schema_version), its sidecar manifest, and the
     JSONL group index (`renkin::pool_export::TargetPoolRecord`, one record
     per (group_id, target) proposal attempt -- including groups with zero
     candidates or a target-parse failure). Coverage is always computed from
     this group index plus labels, never inferred from which group_ids
     happen to appear in the candidate pool -- a zero-candidate group would
     otherwise silently vanish from the denominator instead of counting as a
     real coverage gap.
  2. Load ground-truth labels (schema v1): one line per group_id, giving one
     or more accepted correct precursor multisets for that group's target. A
     candidate row is positive (label=1) iff its precursor_smiles (as a
     sorted list) exactly matches ANY of the accepted sets.
  3. Split by target_id (SHA-256(target_id) mod 100 -> bucket), NEVER by
     candidate and NEVER by group_id -- every candidate for one target lands
     in exactly one of train/val/test, even across different group_ids.
  4. Train a LightGBM LGBMRanker (objective="lambdarank") on the train
     split, with one "group" per group_id (not per target_id).
  5. Evaluate on val/test: top-1 hit rate (does the highest-scored candidate
     in a ranking group carry the positive label) and mean reciprocal rank,
     plus a separate "zero-positive-in-pool" count -- a group with no
     positive candidate at all (including one with zero candidates) is a
     *candidate-generation* coverage gap, not something any reranker could
     fix, and conflating the two would make ranking quality and pool
     coverage indistinguishable.

Requires (not declared in pyproject.toml -- this is a standalone dev
script, like the other scripts/*.py in this repo, e.g.
train_template_scorer.py's torch/datasets/rdchiral): `pip install lightgbm`.
Missing lightgbm is a hard error for --train/--evaluate; `--self-test`
still exercises everything that doesn't need it and reports what it
skipped.

Usage:
    python3 scripts/train_reranker.py \
        --pool data/pool.jsonl --manifest data/pool.manifest.json \
        --groups data/pool.groups.jsonl --labels data/labels.jsonl \
        --model-out data/reranker.txt --eval-out data/reranker_eval.json

    # Exercise the deterministic logic without needing real data or lightgbm:
    python3 scripts/train_reranker.py --self-test
"""

import argparse
import hashlib
import json
import sys
from dataclasses import dataclass
from pathlib import Path

TRAIN_MAX_BUCKET = 70  # buckets [0, 70) -> train
VAL_MAX_BUCKET = 85  # buckets [70, 85) -> val, [85, 100) -> test

LABELS_SCHEMA_VERSION = 1


def target_split_bucket(target_id: str) -> int:
    """Deterministic bucket in [0, 100) for a target_id, via SHA-256 -- not
    Python's randomized `hash()` (unstable across runs/processes) and not a
    seeded PRNG (would require carrying a seed as extra state). The same
    target_id always maps to the same bucket, in this process or any other.
    """
    digest = hashlib.sha256(target_id.encode("utf-8")).digest()
    return int.from_bytes(digest[:4], "big") % 100


def split_for_target(target_id: str) -> str:
    bucket = target_split_bucket(target_id)
    if bucket < TRAIN_MAX_BUCKET:
        return "train"
    if bucket < VAL_MAX_BUCKET:
        return "val"
    return "test"


def load_jsonl(path: str) -> list:
    rows = []
    with open(path, "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if line:
                rows.append(json.loads(line))
    return rows


@dataclass(frozen=True)
class GroupLabel:
    target_id: str
    # frozenset of tuple(sorted(precursor SMILES)) -- multiple accepted
    # correct precursor multisets per group are allowed (e.g. more than one
    # literature-reported disconnection counts as correct).
    correct_precursor_sets: frozenset


def load_labels(path: str) -> dict:
    """group_id -> GroupLabel, schema v1 only.

    Hard errors (never silently coerced or dropped):
      - a row whose schema_version isn't LABELS_SCHEMA_VERSION;
      - a correct_precursor_sets entry that isn't already sorted (the pool
        exporter always sorts precursor_smiles before hashing/export, so a
        correct label must be supplied pre-sorted the same way -- this
        script has no chemistry library to independently canonicalize
        SMILES, so sortedness is the checkable proxy for "matches the
        exporter's own canonical/sorted convention");
      - an empty correct_precursor_sets list (omit the group entirely for a
        genuinely unlabeled group -- see --allow-unlabeled -- rather than
        recording an empty answer set);
      - a duplicate group_id whose recorded data conflicts with what was
        already loaded. An identical duplicate is tolerated (idempotent),
        but is still checked against the existing entry rather than being
        silently written over it.
    """
    labels: dict = {}
    for row in load_jsonl(path):
        group_id = row.get("group_id")
        schema_version = row.get("schema_version")
        if schema_version != LABELS_SCHEMA_VERSION:
            raise ValueError(
                f"labels row for group_id={group_id!r} has "
                f"schema_version={schema_version!r}, expected "
                f"{LABELS_SCHEMA_VERSION} -- this script only understands "
                "labels schema v1"
            )
        target_id = row["target_id"]
        raw_sets = row["correct_precursor_sets"]
        if not raw_sets:
            raise ValueError(
                f"labels row for group_id={group_id!r} has an empty "
                "correct_precursor_sets -- omit the group entirely if it is "
                "genuinely unlabeled (see --allow-unlabeled) instead of "
                "recording an empty answer set"
            )
        sets = []
        for s in raw_sets:
            if list(s) != sorted(s):
                raise ValueError(
                    f"labels row for group_id={group_id!r} has a "
                    f"correct_precursor_sets entry {s!r} that is not "
                    "sorted -- supply it pre-sorted, matching the pool "
                    "exporter's own convention"
                )
            sets.append(tuple(s))
        new_label = GroupLabel(target_id=target_id, correct_precursor_sets=frozenset(sets))

        existing = labels.get(group_id)
        if existing is not None and existing != new_label:
            raise ValueError(
                f"duplicate group_id={group_id!r} in labels with "
                f"conflicting data: {existing} vs {new_label} -- a labels "
                "file must never silently overwrite one group's data with "
                "another's"
            )
        labels[group_id] = new_label
    return labels


@dataclass
class LabeledRow:
    group_id: str
    target_id: str
    candidate_id: str
    features: list  # float, NaN where feature_missing[i] is True
    label: int
    split: str


def label_and_split_rows(
    pool_rows: list, labels: dict, group_records: list, allow_unlabeled: bool = False
) -> tuple:
    """Attach a leakage-safe split and a binary label to every pool row.

    The set of groups to consider comes from `group_records` (the JSONL
    group index), not from which group_ids happen to appear in `pool_rows`
    -- a zero-candidate group must still be accounted for.

    A group present in `group_records` but absent from `labels` is a hard
    error by default (an unlabeled group must never be silently treated as
    "every candidate is negative" -- that would make a candidate-generation
    gap indistinguishable from a real negative). Pass `allow_unlabeled=True`
    to exclude such groups from training/evaluation instead; the excluded
    count is returned separately so it's reported, not swallowed.

    A label's `target_id` is cross-checked against the group index's
    `target_id` for the same group_id -- a mismatch would silently corrupt
    the leakage-safe split (every candidate row's split is derived from
    `target_id`), so it is a hard error, not a value to prefer one source
    over the other for.

    Returns `(labeled_rows, unlabeled_group_count)`.
    """
    target_id_by_group = {r["group_id"]: r["target_id"] for r in group_records}
    all_group_ids = set(target_id_by_group)
    unlabeled = sorted(g for g in all_group_ids if g not in labels)
    if unlabeled and not allow_unlabeled:
        preview = unlabeled[:5]
        raise ValueError(
            f"{len(unlabeled)} group(s) in --groups have no entry in "
            f"--labels (e.g. {preview!r}) -- pass --allow-unlabeled to "
            "exclude them from training/evaluation instead of treating "
            "this as a hard error"
        )
    unlabeled_set = set(unlabeled)

    out = []
    for row in pool_rows:
        group_id = row["group_id"]
        if group_id in unlabeled_set:
            continue
        label_entry = labels.get(group_id)
        if label_entry is None:
            continue  # not in group_records at all -- not this script's problem to invent
        expected_target = target_id_by_group.get(group_id)
        if expected_target is not None and label_entry.target_id != expected_target:
            raise ValueError(
                f"group_id={group_id!r}: labels target_id="
                f"{label_entry.target_id!r} does not match the group "
                f"index's target_id={expected_target!r} for this group"
            )
        precursors = tuple(sorted(row["precursor_smiles"]))
        label = 1 if precursors in label_entry.correct_precursor_sets else 0
        features = [
            float("nan") if m else v
            for v, m in zip(row["feature_values"], row["feature_missing"])
        ]
        out.append(
            LabeledRow(
                group_id=group_id,
                target_id=row["target_id"],
                candidate_id=row["candidate_id"],
                features=features,
                label=label,
                split=split_for_target(row["target_id"]),
            )
        )
    return out, len(unlabeled_set)


@dataclass
class CoverageSummary:
    target_count: int = 0
    group_count: int = 0
    groups_with_zero_positive: int = 0
    positive_candidate_count: int = 0
    total_candidate_count: int = 0


def summarize_coverage(
    labeled_rows: list, group_records: list, labels: dict, split: str
) -> CoverageSummary:
    """Coverage denominator is built from `group_records` + `labels` --
    i.e. every labeled group whose target falls in `split` -- never by
    counting distinct group_ids that happen to appear in `labeled_rows`. A
    group with zero candidates still has a `group_records` entry and so is
    still counted here, with `positives == 0` contributing to
    `groups_with_zero_positive` exactly like a group that has candidates but
    none of them positive.
    """
    by_group: dict = {}
    for r in labeled_rows:
        if r.split == split:
            by_group.setdefault(r.group_id, []).append(r)

    summary = CoverageSummary()
    target_ids_in_split: set = set()
    for record in group_records:
        group_id = record["group_id"]
        target_id = record["target_id"]
        if group_id not in labels:
            continue  # unlabeled -- excluded entirely, see --allow-unlabeled
        if split_for_target(target_id) != split:
            continue
        rows = by_group.get(group_id, [])
        summary.group_count += 1
        summary.total_candidate_count += len(rows)
        positives = sum(1 for r in rows if r.label == 1)
        summary.positive_candidate_count += positives
        if positives == 0:
            summary.groups_with_zero_positive += 1
        target_ids_in_split.add(target_id)
    summary.target_count = len(target_ids_in_split)
    return summary


def group_sizes(rows: list) -> list:
    """LightGBM group sizes: consecutive run-lengths per group_id. Callers
    must sort `rows` by group_id first (see `train_ranker`) -- LightGBM's
    `group` parameter is defined as consecutive counts, not a labeled
    grouping key.
    """
    sizes = []
    current = None
    count = 0
    for r in rows:
        if r.group_id != current:
            if count:
                sizes.append(count)
            current = r.group_id
            count = 0
        count += 1
    if count:
        sizes.append(count)
    return sizes


def train_ranker(train_rows: list):
    """Fit an LGBMRanker (lambdarank objective = LambdaMART). Requires
    lightgbm; raises ImportError with an actionable message if missing.
    """
    try:
        import lightgbm as lgb
    except ImportError as e:
        raise ImportError(
            "lightgbm is required for --train/--evaluate. Install it with "
            "`pip install lightgbm` (not a pyproject.toml dependency -- "
            "this script is a standalone dev tool, like the other "
            "scripts/*.py training scripts in this repo)."
        ) from e

    # Sort by (group_id, candidate_id): group_sizes() only needs group_id
    # runs to be consecutive, but the secondary candidate_id key makes row
    # order -- and therefore training input -- independent of whatever
    # order the JSONL happened to list candidates in.
    rows = sorted(train_rows, key=lambda r: (r.group_id, r.candidate_id))
    X = [r.features for r in rows]
    y = [r.label for r in rows]
    groups = group_sizes(rows)

    ranker = lgb.LGBMRanker(objective="lambdarank", verbosity=-1)
    ranker.fit(X, y, group=groups)
    return ranker


def evaluate(ranker, rows: list, group_records: list, labels: dict, split: str) -> dict:
    """Top-1 hit rate and mean reciprocal rank on `split`, computed per
    ranking group (`group_id`), plus the zero-positive-in-pool coverage
    count (a group with no positive candidate in its pool -- including one
    with zero candidates at all -- can't be salvaged by any reranker;
    reporting it alongside hit rate keeps the two failure modes
    distinguishable).
    """
    by_group: dict = {}
    for r in rows:
        if r.split == split:
            by_group.setdefault(r.group_id, []).append(r)

    top1_hits = 0
    reciprocal_ranks = []
    scored_groups = 0
    for group_rows in by_group.values():
        if not any(r.label == 1 for r in group_rows):
            continue  # coverage gap, not a ranking failure -- see summarize_coverage
        scored_groups += 1
        scores = ranker.predict([r.features for r in group_rows])
        # Explicit candidate_id secondary key: LightGBM produces exact score
        # ties on this feature set (many candidates share identical group-1
        # values), and a tie broken by whatever order the rows arrived in
        # would make top1_hit_rate/mean_reciprocal_rank depend on JSONL line
        # order rather than on the model. Score is negated so ascending sort
        # puts the highest score first -- `reverse=True` would also reverse
        # the candidate_id tie-break, which is not what we want.
        ranked = sorted(
            zip(scores, group_rows), key=lambda p: (-p[0], p[1].candidate_id)
        )
        if ranked[0][1].label == 1:
            top1_hits += 1
        rank = next(
            i + 1 for i, (_, r) in enumerate(ranked) if r.label == 1
        )
        reciprocal_ranks.append(1.0 / rank)

    coverage = summarize_coverage(rows, group_records, labels, split)
    return {
        "split": split,
        "target_count": coverage.target_count,
        "group_count": coverage.group_count,
        "scored_groups": scored_groups,
        "groups_with_zero_positive_in_pool": coverage.groups_with_zero_positive,
        "top1_hit_rate": (top1_hits / scored_groups) if scored_groups else None,
        "mean_reciprocal_rank": (
            sum(reciprocal_ranks) / len(reciprocal_ranks) if reciprocal_ranks else None
        ),
    }


def self_test() -> None:
    """Assert-based self-check of every piece of logic that doesn't need
    lightgbm; if lightgbm IS importable, also runs a tiny end-to-end
    train+evaluate pass and asserts it produces a well-formed report (not
    that any particular metric value is achieved -- 2-3 synthetic groups
    is far too small to mean anything, and this is a code-path check, not a
    model-quality check).
    """
    # -- split determinism and no target crosses splits --
    ids = ["target_a", "target_b", "target_c", "target_d", "target_e"]
    first = {t: split_for_target(t) for t in ids}
    second = {t: split_for_target(t) for t in ids}
    assert first == second, "split assignment must be deterministic"

    # -- same target, different group: same split, different ranking group --
    assert split_for_target("target_a") == split_for_target("target_a")
    group_records_same_target = [
        {"group_id": "rxn-1", "target_id": "target_a", "target_smiles": "CC(=O)OCC",
         "candidate_count": 1, "proposal_status": "ok"},
        {"group_id": "rxn-2", "target_id": "target_a", "target_smiles": "CC(=O)OCC",
         "candidate_count": 1, "proposal_status": "ok"},
    ]
    rows_same_target = [
        {"group_id": "rxn-1", "target_id": "target_a", "candidate_id": "sha256:a1",
         "precursor_smiles": ["CC(=O)O", "CCO"], "feature_values": [0.0], "feature_missing": [False]},
        {"group_id": "rxn-2", "target_id": "target_a", "candidate_id": "sha256:a2",
         "precursor_smiles": ["CC(=O)O", "CCO"], "feature_values": [0.0], "feature_missing": [False]},
    ]
    labels_same_target = {
        "rxn-1": GroupLabel(target_id="target_a", correct_precursor_sets=frozenset({("CC(=O)O", "CCO")})),
        "rxn-2": GroupLabel(target_id="target_a", correct_precursor_sets=frozenset({("CC(=O)O", "CCO")})),
    }
    labeled_same_target, unlabeled_n = label_and_split_rows(
        rows_same_target, labels_same_target, group_records_same_target
    )
    assert unlabeled_n == 0
    splits = {r.group_id: r.split for r in labeled_same_target}
    assert splits["rxn-1"] == splits["rxn-2"], "same target_id must land in the same split"
    sizes = group_sizes(sorted(labeled_same_target, key=lambda r: (r.group_id, r.candidate_id)))
    assert sizes == [1, 1], "different group_id must form separate LightGBM ranking groups"

    # -- labels schema v1: multiple correct precursor sets, sortedness, duplicates --
    import tempfile
    import os

    with tempfile.TemporaryDirectory() as tmp:
        labels_path = os.path.join(tmp, "labels.jsonl")
        with open(labels_path, "w", encoding="utf-8") as f:
            f.write(json.dumps({
                "schema_version": 1, "group_id": "rxn-multi", "target_id": "target_multi",
                "correct_precursor_sets": [["CC(=O)O", "CCO"], ["CCO", "CCl"]],
            }) + "\n")
            # An identical duplicate is tolerated, not an error.
            f.write(json.dumps({
                "schema_version": 1, "group_id": "rxn-multi", "target_id": "target_multi",
                "correct_precursor_sets": [["CCO", "CCl"], ["CC(=O)O", "CCO"]],
            }) + "\n")
        multi_labels = load_labels(labels_path)
        assert len(multi_labels) == 1, "an identical duplicate group_id must not raise or double-count"
        assert ("CC(=O)O", "CCO") in multi_labels["rxn-multi"].correct_precursor_sets
        assert ("CCO", "CCl") in multi_labels["rxn-multi"].correct_precursor_sets

        conflicting_path = os.path.join(tmp, "labels_conflict.jsonl")
        with open(conflicting_path, "w", encoding="utf-8") as f:
            f.write(json.dumps({
                "schema_version": 1, "group_id": "rxn-x", "target_id": "target_x",
                "correct_precursor_sets": [["A", "B"]],
            }) + "\n")
            f.write(json.dumps({
                "schema_version": 1, "group_id": "rxn-x", "target_id": "target_x",
                "correct_precursor_sets": [["C", "D"]],
            }) + "\n")
        try:
            load_labels(conflicting_path)
            raise AssertionError("conflicting duplicate group_id must be a hard error")
        except ValueError:
            pass

        unsorted_path = os.path.join(tmp, "labels_unsorted.jsonl")
        with open(unsorted_path, "w", encoding="utf-8") as f:
            f.write(json.dumps({
                "schema_version": 1, "group_id": "rxn-y", "target_id": "target_y",
                "correct_precursor_sets": [["CCO", "CC(=O)O"]],  # not sorted
            }) + "\n")
        try:
            load_labels(unsorted_path)
            raise AssertionError("an unsorted correct_precursor_sets entry must be a hard error")
        except ValueError:
            pass

        wrong_schema_path = os.path.join(tmp, "labels_wrong_schema.jsonl")
        with open(wrong_schema_path, "w", encoding="utf-8") as f:
            f.write(json.dumps({
                "schema_version": 2, "group_id": "rxn-z", "target_id": "target_z",
                "correct_precursor_sets": [["A"]],
            }) + "\n")
        try:
            load_labels(wrong_schema_path)
            raise AssertionError("a non-v1 schema_version must be a hard error")
        except ValueError:
            pass

    print("self-test: labels schema v1 (multi-set, duplicates, sortedness) OK", flush=True)

    # -- label assignment, unlabeled != negative, zero-candidate group coverage --
    pool_rows = [
        {
            "group_id": "rxn-a1",
            "target_id": "target_a",
            "target_smiles": "CC(=O)OCC",
            "candidate_id": "sha256:aaa",
            "precursor_smiles": ["CC(=O)O", "CCO"],
            "source_template_count": 1,
            "best_upstream_rank": 0,
            "feature_schema_version": 1,
            "feature_values": [2.0, 6.0, 6.0, 4.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.5, 0.0],
            "feature_missing": [False] * 13 + [True],
        },
        {
            "group_id": "rxn-a1",
            "target_id": "target_a",
            "target_smiles": "CC(=O)OCC",
            "candidate_id": "sha256:bbb",
            "precursor_smiles": ["CCl", "CCO"],
            "source_template_count": 1,
            "best_upstream_rank": 1,
            "feature_schema_version": 1,
            "feature_values": [2.0] + [0.0] * 12 + [0.0],
            "feature_missing": [False] * 13 + [True],
        },
        {
            "group_id": "rxn-b1",
            "target_id": "target_b",
            "target_smiles": "CCN",
            "candidate_id": "sha256:ccc",
            "precursor_smiles": ["CCBr"],
            "source_template_count": 1,
            "best_upstream_rank": 0,
            "feature_schema_version": 1,
            "feature_values": [1.0] + [0.0] * 12 + [0.0],
            "feature_missing": [False] * 13 + [True],
        },
    ]
    # rxn-c1 has zero candidates -- present only in the group index.
    group_records = [
        {"group_id": "rxn-a1", "target_id": "target_a", "target_smiles": "CC(=O)OCC",
         "candidate_count": 2, "proposal_status": "ok"},
        {"group_id": "rxn-b1", "target_id": "target_b", "target_smiles": "CCN",
         "candidate_count": 1, "proposal_status": "ok"},
        {"group_id": "rxn-c1", "target_id": "target_c", "target_smiles": "CCC",
         "candidate_count": 0, "proposal_status": "ok"},
        {"group_id": "rxn-d1", "target_id": "target_d", "target_smiles": "CCCC",
         "candidate_count": 0, "proposal_status": "target_parse_failed"},
    ]
    labels = {
        "rxn-a1": GroupLabel(target_id="target_a", correct_precursor_sets=frozenset({("CC(=O)O", "CCO")})),
        "rxn-c1": GroupLabel(target_id="target_c", correct_precursor_sets=frozenset({("X", "Y")})),
        "rxn-d1": GroupLabel(target_id="target_d", correct_precursor_sets=frozenset({("X", "Y")})),
        # rxn-b1 deliberately absent -> exercised via --allow-unlabeled below.
    }

    try:
        label_and_split_rows(pool_rows, labels, group_records, allow_unlabeled=False)
        raise AssertionError("an unlabeled group must be a hard error by default")
    except ValueError:
        pass

    labeled, unlabeled_count = label_and_split_rows(
        pool_rows, labels, group_records, allow_unlabeled=True
    )
    assert unlabeled_count == 1, "exactly rxn-b1 is unlabeled"
    assert all(r.group_id != "rxn-b1" for r in labeled), (
        "an unlabeled group must be excluded entirely, never defaulted to all-negative"
    )

    by_id = {r.candidate_id: r for r in labeled}
    assert by_id["sha256:aaa"].label == 1, "exact precursor-set match must be labeled positive"
    assert by_id["sha256:bbb"].label == 0, "non-matching precursor set must be labeled negative"

    # -- feature_missing -> NaN, not silently 0 --
    import math

    assert math.isnan(by_id["sha256:aaa"].features[-1]), "a missing feature must become NaN, not 0"

    # -- zero-candidate group counted in coverage denominator --
    # Force every target into "test" for this check by using the real
    # split_for_target output rather than overriding it, then compute
    # coverage per each split actually produced and assert the totals add up
    # across all three splits (whichever split each synthetic target_id
    # really falls into).
    all_splits = {"train", "val", "test"}
    total_group_count = 0
    total_target_count = 0
    for split in all_splits:
        cov = summarize_coverage(labeled, group_records, labels, split)
        total_group_count += cov.group_count
        total_target_count += cov.target_count
    # rxn-b1 is unlabeled (excluded) -- only rxn-a1, rxn-c1, rxn-d1 count.
    assert total_group_count == 3, "labeled groups (including zero-candidate ones) must all be counted exactly once across splits"
    assert total_target_count == 3, "target_a, target_c, target_d -- one each"

    print("self-test: labeling, unlabeled-group handling, and zero-candidate-group coverage OK", flush=True)

    # -- group_sizes requires group_id-sorted input --
    sorted_rows = sorted(labeled, key=lambda r: (r.group_id, r.candidate_id))
    sizes = group_sizes(sorted_rows)
    assert sum(sizes) == len(sorted_rows)
    assert len(sizes) == len({r.group_id for r in labeled})

    print("self-test: deterministic split, labeling, and grouping logic OK", flush=True)

    # -- evaluate() ranking tie-break is deterministic, independent of lightgbm --
    class _ConstantRanker:
        def predict(self, X):
            return [0.0] * len(X)

    tie_group_records = [
        {"group_id": "t", "target_id": "tt", "target_smiles": "tt", "candidate_count": 2, "proposal_status": "ok"},
    ]
    tie_labels = {"t": GroupLabel(target_id="tt", correct_precursor_sets=frozenset({("x",)}))}
    tied_a = [
        LabeledRow("t", "tt", "sha256:bbb", [1.0], 1, "test"),
        LabeledRow("t", "tt", "sha256:aaa", [1.0], 0, "test"),
    ]
    tied_b = list(reversed(tied_a))
    report_a = evaluate(_ConstantRanker(), tied_a, tie_group_records, tie_labels, "test")
    report_b = evaluate(_ConstantRanker(), tied_b, tie_group_records, tie_labels, "test")
    assert report_a == report_b, "tied scores must rank identically regardless of input row order"
    print("self-test: evaluate() tie-break is order-independent OK", flush=True)

    try:
        import lightgbm  # noqa: F401
    except ImportError:
        print(
            "self-test: lightgbm not installed -- skipping end-to-end train+evaluate "
            "(pip install lightgbm to exercise it)",
            flush=True,
        )
        return

    # `train_ranker` doesn't care what `.split` any row carries -- it just
    # fits on whatever rows it's handed -- so training needs no override.
    # Evaluation's coverage fields, though, are computed from `group_records`
    # via the REAL `split_for_target(target_id)` (see `summarize_coverage`),
    # not from `LabeledRow.split` -- so unlike the pre-group_id version of
    # this script, an artificial "force every row into one split" override
    # would silently desync row-level `.split` from group-index coverage.
    # Summing `evaluate()` over all three real splits instead is robust to
    # wherever these tiny synthetic target_ids actually hash to.
    ranker = train_ranker(labeled)
    total_group_count = 0
    total_scored_groups = 0
    total_zero_positive = 0
    for split in ("train", "val", "test"):
        report = evaluate(ranker, labeled, group_records, labels, split)
        assert report["split"] == split
        total_group_count += report["group_count"]
        total_scored_groups += report["scored_groups"]
        total_zero_positive += report["groups_with_zero_positive_in_pool"]
    assert total_group_count == 3, "rxn-a1, rxn-c1, rxn-d1 (rxn-b1 excluded, unlabeled)"
    assert total_scored_groups == 1, "only rxn-a1 has a positive candidate"
    assert total_zero_positive == 2, "rxn-c1 and rxn-d1 have none"
    print(
        f"self-test: end-to-end train+evaluate OK, "
        f"group_count={total_group_count} scored_groups={total_scored_groups}",
        flush=True,
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--pool", help="JSONL candidate pool (renkin::pool_export::write_jsonl output)")
    parser.add_argument("--manifest", help="Sidecar PoolManifest JSON (for feature_schema_version validation)")
    parser.add_argument(
        "--groups",
        help="JSONL group index (renkin::pool_export::write_target_pool_jsonl output) -- "
             "one record per (group_id, target) proposal attempt, including zero-candidate "
             "and parse-failure groups. Coverage is computed from this file, not from --pool.",
    )
    parser.add_argument(
        "--labels",
        help="JSONL ground-truth labels, schema v1: "
             "{schema_version, group_id, target_id, correct_precursor_sets}",
    )
    parser.add_argument(
        "--allow-unlabeled", action="store_true",
        help="Exclude groups present in --groups but absent from --labels from training/"
             "evaluation instead of treating them as a hard error. The excluded count is "
             "printed, never silently dropped.",
    )
    parser.add_argument("--model-out", help="Path to save the trained LightGBM booster (text format)")
    parser.add_argument("--eval-out", help="Path to save the evaluation report JSON")
    parser.add_argument(
        "--self-test", action="store_true",
        help="Run the deterministic-logic self-check (and, if lightgbm is installed, "
             "a tiny end-to-end smoke pass) against an embedded synthetic fixture, "
             "then exit. Does not read --pool/--manifest/--groups/--labels.",
    )
    args = parser.parse_args()

    if args.self_test:
        self_test()
        return

    if not (args.pool and args.manifest and args.groups and args.labels):
        parser.error("--pool, --manifest, --groups, and --labels are all required unless --self-test is given")

    manifest = json.load(open(args.manifest, "r", encoding="utf-8"))
    from_export_schema = manifest.get("feature_schema_version")
    print(f"Loaded manifest: feature_schema_version={from_export_schema}, "
          f"proposal_mode={manifest.get('proposal_mode')}, "
          f"rules_content_hash={manifest.get('rules_content_hash')}", flush=True)
    if manifest.get("proposal_mode", {}).get("mode") != "exhaustive":
        print(
            "WARNING: manifest.proposal_mode is not 'exhaustive' -- training a reranker "
            "on a narrowed candidate set (bond_indexed/scorer_conditioned) means it never "
            "sees candidates outside that narrowing, which biases what it can learn to "
            "rank. See src/candidate.rs's module doc.",
            file=sys.stderr,
        )

    pool_rows = load_jsonl(args.pool)
    group_records = load_jsonl(args.groups)
    labels = load_labels(args.labels)
    labeled, unlabeled_group_count = label_and_split_rows(
        pool_rows, labels, group_records, allow_unlabeled=args.allow_unlabeled
    )
    if unlabeled_group_count:
        print(
            f"NOTE: {unlabeled_group_count} unlabeled group(s) excluded from "
            "training/evaluation (--allow-unlabeled)",
            file=sys.stderr,
        )

    train_rows = [r for r in labeled if r.split == "train"]
    if not train_rows:
        print("ERROR: no rows in the train split.", file=sys.stderr)
        sys.exit(1)

    ranker = train_ranker(train_rows)

    if args.model_out:
        Path(args.model_out).parent.mkdir(parents=True, exist_ok=True)
        ranker.booster_.save_model(args.model_out)
        print(f"Saved model to {args.model_out}", flush=True)

    reports = {
        split: evaluate(ranker, labeled, group_records, labels, split)
        for split in ("train", "val", "test")
    }
    result = {
        "feature_schema_version": from_export_schema,
        "proposal_mode": manifest.get("proposal_mode"),
        "rules_content_hash": manifest.get("rules_content_hash"),
        "unlabeled_group_count": unlabeled_group_count,
        "reports": reports,
    }
    print(json.dumps(result, indent=2), flush=True)

    if args.eval_out:
        Path(args.eval_out).parent.mkdir(parents=True, exist_ok=True)
        with open(args.eval_out, "w", encoding="utf-8") as f:
            json.dump(result, f, indent=2)
        print(f"Saved evaluation report to {args.eval_out}", flush=True)


if __name__ == "__main__":
    main()
