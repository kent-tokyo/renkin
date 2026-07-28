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
import math
import sys
from dataclasses import dataclass
from pathlib import Path

TRAIN_MAX_BUCKET = 70  # buckets [0, 70) -> train
VAL_MAX_BUCKET = 85  # buckets [70, 85) -> val, [85, 100) -> test

LABELS_SCHEMA_VERSION = 1
MANIFEST_SCHEMA_VERSION = 1
FEATURE_SCHEMA_VERSION = 1

# Mirrors `renkin::candidate::FEATURE_NAMES_V1` (src/candidate.rs) exactly --
# this script has no way to import that crate, so the schema is duplicated
# here. `validate_manifest` cross-checks this list (and `feature_schema_hash`
# below) against a real exported manifest, so a silent drift between the two
# copies is caught at load time, not discovered as a mysteriously-wrong
# feature column months later.
FEATURE_NAMES_V1 = [
    "num_precursors",
    "target_heavy_atom_count",
    "precursor_heavy_atom_count_sum",
    "precursor_heavy_atom_count_max",
    "heavy_atom_retention_ratio",
    "net_charge_balanced",
    "no_heavy_atom_gain",
    "source_template_count",
    "reaction_center_atom_count_min",
    "reaction_center_atom_count_max",
    "reaction_center_atom_count_mean",
    "reaction_center_extractable_fraction",
    "min_base_step_cost",
    "best_upstream_score",
    "fraction_precursors_in_stock",
    "all_precursors_in_stock",
    "max_template_log_frequency",
    "mean_template_log_frequency",
]


def feature_schema_hash() -> str:
    """Mirrors `renkin::candidate::feature_schema_hash` byte-for-byte (same
    domain tag, same big-endian length-prefixed framing) -- see that
    function's doc. If this Python copy and the Rust original ever silently
    drift apart (a renamed/reordered/added feature on one side only), this
    hash stops matching a manifest's `feature_schema_hash`, and
    `validate_manifest` turns that into a load-time hard error instead of a
    training run silently using the wrong column meanings.
    """
    h = hashlib.sha256()
    h.update(b"renkin-retrospect-feature-schema-v1\0")
    h.update(FEATURE_SCHEMA_VERSION.to_bytes(4, "big"))
    h.update(len(FEATURE_NAMES_V1).to_bytes(8, "big"))
    for name in FEATURE_NAMES_V1:
        name_bytes = name.encode("utf-8")
        h.update(len(name_bytes).to_bytes(8, "big"))
        h.update(name_bytes)
    return f"sha256:{h.hexdigest()}"


def sha256_file(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return f"sha256:{h.hexdigest()}"


def validate_manifest(manifest: dict, pool_path: str, groups_path: str) -> None:
    """Hard-validate every manifest field this script depends on. A
    manifest is a claim about what `--pool`/`--groups` actually are; every
    field here is cross-checked against the real bytes/constants it claims
    to describe -- never trusted at face value. Mirrors
    `renkin::pool_export::build_manifest`'s own invariants (see that
    function's doc) from the consuming side.
    """
    if manifest.get("manifest_schema_version") != MANIFEST_SCHEMA_VERSION:
        raise ValueError(
            f"manifest_schema_version={manifest.get('manifest_schema_version')!r}, "
            f"expected {MANIFEST_SCHEMA_VERSION}"
        )
    if manifest.get("feature_schema_version") != FEATURE_SCHEMA_VERSION:
        raise ValueError(
            f"feature_schema_version={manifest.get('feature_schema_version')!r}, "
            f"expected {FEATURE_SCHEMA_VERSION} -- this script's FEATURE_NAMES_V1 "
            "mirror is only valid for that schema version"
        )
    if manifest.get("feature_names") != FEATURE_NAMES_V1:
        raise ValueError(
            "manifest.feature_names does not exactly match this script's "
            "FEATURE_NAMES_V1 mirror -- refusing to train on a feature vector "
            "whose column meaning this script can't guarantee"
        )
    expected_feature_hash = feature_schema_hash()
    if manifest.get("feature_schema_hash") != expected_feature_hash:
        raise ValueError(
            f"manifest.feature_schema_hash={manifest.get('feature_schema_hash')!r} "
            f"does not match this script's recomputed {expected_feature_hash!r} -- "
            "the Rust and Python feature-schema mirrors have drifted apart"
        )
    proposal_mode = manifest.get("proposal_mode")
    if not isinstance(proposal_mode, dict) or "mode" not in proposal_mode:
        raise ValueError("manifest.proposal_mode is missing or malformed")
    if not manifest.get("rules_content_hash"):
        raise ValueError("manifest.rules_content_hash is missing")

    expected_pool_hash = sha256_file(pool_path)
    if manifest.get("candidate_jsonl_sha256") != expected_pool_hash:
        raise ValueError(
            f"manifest.candidate_jsonl_sha256={manifest.get('candidate_jsonl_sha256')!r} "
            f"does not match --pool's actual hash {expected_pool_hash!r} -- this "
            "manifest was not produced alongside this exact --pool file"
        )
    expected_groups_hash = sha256_file(groups_path)
    if manifest.get("target_group_index_sha256") != expected_groups_hash:
        raise ValueError(
            "manifest.target_group_index_sha256="
            f"{manifest.get('target_group_index_sha256')!r} does not match --groups's "
            f"actual hash {expected_groups_hash!r} -- this manifest was not produced "
            "alongside this exact --groups file"
        )

    if proposal_mode["mode"] == "scorer_conditioned" and proposal_mode.get("scorer_status") != "available":
        raise ValueError(
            "manifest.proposal_mode.scorer_status="
            f"{proposal_mode.get('scorer_status')!r} for a scorer_conditioned pool -- "
            "a pool exported from a failed/unavailable scorer must never be trained "
            "on as if it were a real narrowed pool"
        )

    stock_identity_present = manifest.get("stock_identity") is not None
    stock_hash_present = manifest.get("stock_content_sha256") is not None
    if stock_identity_present != stock_hash_present:
        raise ValueError(
            "manifest.stock_identity and stock_content_sha256 must both be present "
            "or both be absent (got stock_identity="
            f"{manifest.get('stock_identity')!r}, "
            f"stock_content_sha256={manifest.get('stock_content_sha256')!r}) -- "
            "an inconsistent stock provenance pair is never trustworthy"
        )


def validate_pool_rows(pool_rows: list, group_records: list) -> None:
    """Hard-validate every candidate row against the fixed feature schema
    and the group index, before any row is used for labeling/training.
    Mirrors `renkin::pool_export::validate_candidate_rows` /
    `validate_rows_consistent_with_group_index` from the consuming side --
    both sides reject the same malformed input, so a bad row can never
    sneak past whichever side happens to validate more loosely.
    """
    target_id_by_group = {r["group_id"]: r["target_id"] for r in group_records}
    target_smiles_by_group = {r["group_id"]: r["target_smiles"] for r in group_records}
    seen_within_group: dict = {}

    for row in pool_rows:
        group_id = row["group_id"]
        candidate_id = row["candidate_id"]

        if row.get("feature_schema_version") != FEATURE_SCHEMA_VERSION:
            raise ValueError(
                f"candidate {candidate_id!r} has feature_schema_version="
                f"{row.get('feature_schema_version')!r}, expected {FEATURE_SCHEMA_VERSION}"
            )

        values = row["feature_values"]
        missing = row["feature_missing"]
        if len(values) != len(FEATURE_NAMES_V1) or len(missing) != len(FEATURE_NAMES_V1):
            raise ValueError(
                f"candidate {candidate_id!r} has feature_values (len={len(values)}) / "
                f"feature_missing (len={len(missing)}) that don't both match "
                f"len(FEATURE_NAMES_V1)={len(FEATURE_NAMES_V1)} -- refusing to zip() "
                "them together, which would silently truncate to the shorter one"
            )
        for i, (v, m) in enumerate(zip(values, missing)):
            if not m and not math.isfinite(v):
                raise ValueError(
                    f"candidate {candidate_id!r} feature[{i}] "
                    f"({FEATURE_NAMES_V1[i]!r}) is non-finite ({v!r}) but not marked missing"
                )

        if not row.get("precursor_smiles"):
            raise ValueError(f"candidate {candidate_id!r} has an empty precursor_smiles list")
        if not row.get("sources"):
            raise ValueError(f"candidate {candidate_id!r} has an empty sources list")

        if group_id not in target_id_by_group:
            raise ValueError(
                f"candidate {candidate_id!r}'s group_id {group_id!r} has no entry in --groups"
            )
        if row["target_id"] != target_id_by_group[group_id]:
            raise ValueError(
                f"group_id {group_id!r}: candidate row target_id {row['target_id']!r} "
                f"does not match group index target_id {target_id_by_group[group_id]!r}"
            )
        if row.get("target_smiles") != target_smiles_by_group[group_id]:
            raise ValueError(
                f"group_id {group_id!r}: candidate row target_smiles "
                f"{row.get('target_smiles')!r} does not match group index "
                f"target_smiles {target_smiles_by_group[group_id]!r}"
            )

        seen = seen_within_group.setdefault(group_id, set())
        if candidate_id in seen:
            raise ValueError(f"duplicate candidate_id {candidate_id!r} within group_id {group_id!r}")
        seen.add(candidate_id)


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
        values = row["feature_values"]
        missing = row["feature_missing"]
        if len(values) != len(missing):
            raise ValueError(
                f"candidate_id={row['candidate_id']!r}: feature_values (len="
                f"{len(values)}) and feature_missing (len={len(missing)}) have "
                "different lengths -- refusing to zip() them together, which "
                "would silently truncate to the shorter one"
            )
        features = [float("nan") if m else v for v, m in zip(values, missing)]
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

    # -- feature_schema_hash is pinned against the Rust implementation --
    # (see `feature_schema_hash_is_stable_and_pinned_for_cross_language_verification`
    # in src/candidate.rs -- both literals were computed from the same
    # algorithm and must be updated together on any intentional schema change).
    assert feature_schema_hash() == (
        "sha256:756404c59bbee9a65e194f92df3530e1b801028f333e01c67214917977061df1"
    ), "feature_schema_hash() drifted from the pinned Rust-side value"

    # -- validate_manifest: every field is cross-checked, not trusted --
    with tempfile.TemporaryDirectory() as tmp:
        pool_path = os.path.join(tmp, "pool.jsonl")
        with open(pool_path, "w", encoding="utf-8") as f:
            f.write('{"candidate_id": "c1"}\n')
        groups_path = os.path.join(tmp, "groups.jsonl")
        with open(groups_path, "w", encoding="utf-8") as f:
            f.write('{"group_id": "g1"}\n')

        def base_manifest() -> dict:
            return {
                "manifest_schema_version": MANIFEST_SCHEMA_VERSION,
                "feature_schema_version": FEATURE_SCHEMA_VERSION,
                "feature_names": list(FEATURE_NAMES_V1),
                "feature_schema_hash": feature_schema_hash(),
                "proposal_mode": {"mode": "exhaustive"},
                "rules_content_hash": "sha256:deadbeef",
                "candidate_jsonl_sha256": sha256_file(pool_path),
                "target_group_index_sha256": sha256_file(groups_path),
                "stock_identity": None,
                "stock_content_sha256": None,
            }

        validate_manifest(base_manifest(), pool_path, groups_path)  # must not raise

        bad_cases = [
            {**base_manifest(), "manifest_schema_version": 999},
            {**base_manifest(), "feature_schema_version": 999},
            {**base_manifest(), "feature_names": ["wrong"]},
            {**base_manifest(), "feature_schema_hash": "sha256:wrong"},
            {**base_manifest(), "rules_content_hash": ""},
            {**base_manifest(), "candidate_jsonl_sha256": "sha256:wrong"},
            {**base_manifest(), "target_group_index_sha256": "sha256:wrong"},
            {
                **base_manifest(),
                "proposal_mode": {"mode": "scorer_conditioned", "scorer_status": "inference_failed"},
            },
            {**base_manifest(), "stock_identity": "some/path.smi", "stock_content_sha256": None},
        ]
        for i, bad in enumerate(bad_cases):
            try:
                validate_manifest(bad, pool_path, groups_path)
                raise AssertionError(f"bad_cases[{i}] should have been rejected: {bad}")
            except ValueError:
                pass

    print("self-test: validate_manifest rejects every mismatched field OK", flush=True)

    # -- validate_pool_rows: schema/group-index consistency, no silent zip() truncation --
    good_groups = [
        {"group_id": "g1", "target_id": "t1", "target_smiles": "CCO", "candidate_count": 1, "proposal_status": "ok"},
    ]

    def good_row(**overrides) -> dict:
        row = {
            "group_id": "g1",
            "target_id": "t1",
            "target_smiles": "CCO",
            "candidate_id": "c1",
            "precursor_smiles": ["CCO"],
            "sources": [{"template_id": "rule:x"}],
            "feature_schema_version": FEATURE_SCHEMA_VERSION,
            "feature_values": [0.0] * len(FEATURE_NAMES_V1),
            "feature_missing": [True] * len(FEATURE_NAMES_V1),
        }
        row.update(overrides)
        return row

    validate_pool_rows([good_row()], good_groups)  # must not raise

    row_bad_lengths = good_row(feature_values=[0.0] * (len(FEATURE_NAMES_V1) - 1))
    row_non_finite = good_row(
        feature_values=[float("nan")] + [0.0] * (len(FEATURE_NAMES_V1) - 1),
        feature_missing=[False] * len(FEATURE_NAMES_V1),
    )
    row_empty_precursors = good_row(precursor_smiles=[])
    row_empty_sources = good_row(sources=[])
    row_unknown_group = good_row(group_id="g-missing")
    row_wrong_target_id = good_row(target_id="t-wrong")
    row_wrong_target_smiles = good_row(target_smiles="CCN")

    for label, bad_rows in [
        ("mismatched feature_values/feature_missing length", [row_bad_lengths]),
        ("non-finite non-missing feature value", [row_non_finite]),
        ("empty precursor_smiles", [row_empty_precursors]),
        ("empty sources", [row_empty_sources]),
        ("group_id absent from --groups", [row_unknown_group]),
        ("target_id inconsistent with group index", [row_wrong_target_id]),
        ("target_smiles inconsistent with group index", [row_wrong_target_smiles]),
        ("duplicate candidate_id within one group", [good_row(), good_row()]),
    ]:
        try:
            validate_pool_rows(bad_rows, good_groups)
            raise AssertionError(f"should have been rejected: {label}")
        except ValueError:
            pass

    print("self-test: validate_pool_rows rejects every malformed row OK", flush=True)

    try:
        label_and_split_rows(
            [{
                "group_id": "g1", "target_id": "t1", "candidate_id": "c1",
                "precursor_smiles": ["CCO"],
                "feature_values": [0.0, 1.0],
                "feature_missing": [False],  # deliberately shorter -- must not silently zip()-truncate
            }],
            {"g1": GroupLabel(target_id="t1", correct_precursor_sets=frozenset({("CCO",)}))},
            [{"group_id": "g1", "target_id": "t1", "target_smiles": "CCO", "candidate_count": 1, "proposal_status": "ok"}],
        )
        raise AssertionError("mismatched feature_values/feature_missing lengths must be a hard error")
    except ValueError:
        pass

    print("self-test: label_and_split_rows rejects mismatched feature-vector lengths OK", flush=True)

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
    validate_manifest(manifest, args.pool, args.groups)
    from_export_schema = manifest.get("feature_schema_version")
    print(
        f"Loaded manifest (validated: schema/feature_names/feature_schema_hash/"
        f"candidate_jsonl_sha256/target_group_index_sha256 all match): "
        f"feature_schema_version={from_export_schema}, "
        f"proposal_mode={manifest.get('proposal_mode')}, "
        f"rules_content_hash={manifest.get('rules_content_hash')}",
        flush=True,
    )
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
    validate_pool_rows(pool_rows, group_records)
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
