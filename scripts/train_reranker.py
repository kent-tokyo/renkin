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
  4. Fit train-frozen template frequency (fit_template_frequency, train
     rows only) and train a LightGBM LGBMRanker (objective="lambdarank",
     LIGHTGBM_HYPERPARAMETERS, early stopping on val) on the train split,
     with one "group" per group_id (not per target_id).
  5. Evaluate every arm -- the trained model (arm H) and seven deterministic
     baseline arms (A-G: original_rank, upstream_score, template_frequency,
     upstream_plus_frequency, structural, reaction_center, availability) --
     through the SAME score_fn/evaluate() path, reporting both conditional
     (denominator = groups with a positive candidate in-pool) and
     end-to-end (denominator = every labeled group; a coverage miss scores
     0, never excluded) top-1/top-10/MRR/NDCG@10/mean-best-positive-rank.
     A "zero-positive-in-pool" group is a *candidate-generation* coverage
     gap, not something any reranker could fix, and end-to-end metrics
     keep that gap visible rather than excluding it.
  6. Optionally (--gate-baseline-arm/--gate-treatment-arm) run a paired
     bootstrap (clustered at target_id, never group_id alone) between two
     arms and judge PASS/FAIL against a fixed set of thresholds
     (GATE_THRESHOLDS) -- coverage must be identical between the two arms,
     and the top-1 delta's 95% CI lower bound must be positive, not just
     its mean.

Requires (not declared in pyproject.toml -- this is a standalone dev
script, like the other scripts/*.py in this repo, e.g.
train_template_scorer.py's torch/datasets/rdchiral): `pip install lightgbm`.
Missing lightgbm is a hard error for --train/--evaluate (and for baseline
arm H / the lightgbm-gated smoke in --self-test); every deterministic
baseline arm (A-G) and the bootstrap/gate machinery need no lightgbm at
all.

`--self-test` is a fast (~1-2s), dependency-minimal smoke test of the
deterministic core (split determinism, minimal manifest/row schema
round-trip, labeling/missing-to-NaN, evaluate()'s tie-break, a tiny
paired-bootstrap + gate PASS smoke, plus a minimal lightgbm end-to-end
smoke if lightgbm is importable) -- it is NOT where detailed regression
coverage lives. That lives in `scripts/tests/` (six files, run with
`python3 -m unittest discover -s scripts/tests -p "test_*.py"`, wired into
CI as the `reranker-tests` job).

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
import random
import sys
from dataclasses import dataclass, replace
from pathlib import Path

TRAIN_MAX_BUCKET = 70  # buckets [0, 70) -> train
VAL_MAX_BUCKET = 85  # buckets [70, 85) -> val, [85, 100) -> test

LABELS_SCHEMA_VERSION = 1
# Bumped to 2 alongside src/pool_export.rs::MANIFEST_SCHEMA_VERSION -- the
# manifest shape changed (five new required fields, plus a
# rules_content_hash algorithm change), so a v1 and a v2 manifest must never
# be silently treated as the same shape.
MANIFEST_SCHEMA_VERSION = 2
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

    # Every group_index record's `candidate_count` is a claim about how many
    # rows exist for it -- checked against the rows actually present, not
    # just trusted. A group with rows but no index entry was already
    # rejected above; a group with an index entry but zero matching rows
    # falls out here as `actual == 0`.
    row_count_by_group: dict = {}
    for group_id in seen_within_group:
        row_count_by_group[group_id] = len(seen_within_group[group_id])
    for record in group_records:
        group_id = record["group_id"]
        actual = row_count_by_group.get(group_id, 0)
        expected = record["candidate_count"]
        if actual != expected:
            raise ValueError(
                f"group_id {group_id!r}: group index claims candidate_count={expected}, "
                f"but {actual} candidate row(s) were actually found in --pool"
            )


def target_split_bucket(target_id: str) -> int:
    """Deterministic bucket in [0, 100) for a target_id, via SHA-256 -- not
    Python's randomized `hash()` (unstable across runs/processes) and not a
    seeded PRNG (would require carrying a seed as extra state). The same
    target_id always maps to the same bucket, in this process or any other.
    """
    digest = hashlib.sha256(target_id.encode("utf-8")).digest()
    return int.from_bytes(digest[:4], "big") % 100


_SPLIT_OVERRIDE: dict = {}


def configure_split_override(mapping: dict | None) -> None:
    """Set (or clear, via `None`) an explicit target_id -> split override
    consulted by every `split_for_target` call in this process, taking
    precedence over the SHA-256 hash bucket below.

    Exists for Phase 3's formal competitive-benchmark quarantine (Issue
    #101): the existing hash bucket re-splits ANY target_id 70/15/15
    regardless of provenance, which is wrong once train/val labels are
    sourced from the USPTO-50k original train/val splits directly (their
    target_ids must land in "train"/"val" by origin, not by hash) -- see
    `generate_train_val_labels.py` and `--split-manifest` below. The
    default (no manifest given) leaves this empty, so `split_for_target`'s
    behavior for every existing caller is completely unchanged.

    A module-level override (rather than threading a parameter through
    `split_for_target`'s five call sites across this file) is deliberate:
    a single override point every call site automatically respects is
    safer here than five places to remember to update in lockstep. Callers
    that share a process across multiple runs (e.g. a test suite) must
    reset with `configure_split_override(None)` when done -- see
    `scripts/tests/test_reranker_schema.py`'s split-manifest tests for the
    pattern.
    """
    global _SPLIT_OVERRIDE
    _SPLIT_OVERRIDE = dict(mapping) if mapping else {}


def load_split_manifest(path: str, known_target_ids: set) -> dict:
    """Load an explicit target_id -> split assignment, schema: one JSON
    object per line, `{"target_id": ..., "split": "train"|"val"|"test"}`.

    Hard errors (never silently coerced):
      - a split value outside {"train", "val", "test"};
      - a target_id with conflicting split assignments within the manifest;
      - a target_id in the manifest that isn't in `known_target_ids`
        ("unknown target" -- almost always a stale manifest built against a
        different --groups file);
      - a target_id in `known_target_ids` missing from the manifest
        ("missing assignment" -- a manifest must cover every target this
        run will actually touch, not silently fall back to hash bucketing
        for the ones it forgot).
    """
    assignments: dict = {}
    for row in load_jsonl(path):
        target_id = row["target_id"]
        split = row["split"]
        if split not in ("train", "val", "test"):
            raise ValueError(
                f"split manifest entry for target_id={target_id!r} has split="
                f"{split!r}, expected one of 'train'/'val'/'test'"
            )
        existing = assignments.get(target_id)
        if existing is not None and existing != split:
            raise ValueError(
                f"split manifest has conflicting assignments for "
                f"target_id={target_id!r}: {existing!r} vs {split!r}"
            )
        assignments[target_id] = split

    unknown = sorted(set(assignments) - known_target_ids)
    if unknown:
        raise ValueError(
            f"split manifest has {len(unknown)} target_id(s) not present in "
            f"--groups for this run (e.g. {unknown[:5]!r}) -- built against "
            "a different --groups file?"
        )
    missing = sorted(known_target_ids - set(assignments))
    if missing:
        raise ValueError(
            f"split manifest is missing {len(missing)} target_id(s) that "
            f"appear in --groups (e.g. {missing[:5]!r}) -- every target "
            "this run touches must have an explicit assignment, not a "
            "silent fallback to hash bucketing for the ones it forgot"
        )
    return assignments


def split_for_target(target_id: str) -> str:
    override = _SPLIT_OVERRIDE.get(target_id)
    if override is not None:
        return override
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
    # Carried through for the deterministic baseline arms (see BASELINE_ARMS)
    # that don't use the feature vector at all: `best_upstream_rank` backs
    # the "original rank" arm, `source_template_ids` backs the train-frozen
    # frequency arm. Defaulted (not required) so every pre-existing
    # `LabeledRow(...)` fixture in this file's own tests keeps working
    # unchanged.
    best_upstream_rank: int = 0
    source_template_ids: tuple = ()


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
        source_template_ids = tuple(sorted({s["template_id"] for s in row.get("sources", [])}))
        out.append(
            LabeledRow(
                group_id=group_id,
                target_id=row["target_id"],
                candidate_id=row["candidate_id"],
                features=features,
                label=label,
                split=split_for_target(row["target_id"]),
                best_upstream_rank=row.get("best_upstream_rank", 0),
                source_template_ids=source_template_ids,
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


# Fixed, explicit hyperparameters -- never left at library defaults, so a
# training run is reproducible and its exact configuration is recorded in
# the eval artifact (see `main`/`run_offline_evaluation`), rather than
# silently depending on whatever lightgbm's own defaults happen to be on
# whatever version is installed. Chosen as reasonable, conservative values
# for a small-to-medium tabular ranking problem -- NOT tuned against any
# real corpus (none has been run yet; see this script's module doc).
LIGHTGBM_HYPERPARAMETERS = {
    "objective": "lambdarank",
    "metric": "ndcg",
    "eval_at": [1, 10],
    "boosting_type": "gbdt",
    "n_estimators": 200,
    "learning_rate": 0.05,
    "num_leaves": 31,
    "max_depth": -1,
    "min_child_weight": 1e-3,
    "min_child_samples": 5,
    "subsample": 0.8,
    "subsample_freq": 1,
    "colsample_bytree": 0.8,
    "reg_alpha": 0.0,
    "reg_lambda": 0.1,
    "random_state": 42,
    "deterministic": True,
    "num_threads": 1,
    "verbosity": -1,
}

EARLY_STOPPING_ROUNDS = 20


def train_ranker(train_rows: list, val_rows: list = None) -> dict:
    """Fit an LGBMRanker (lambdarank objective = LambdaMART) with fixed,
    explicit hyperparameters (`LIGHTGBM_HYPERPARAMETERS`) -- deterministic
    given the same input (fixed `random_state`, `deterministic=True`,
    single-threaded). Requires lightgbm; raises ImportError with an
    actionable message if missing.

    If `val_rows` is given, training early-stops on it
    (`EARLY_STOPPING_ROUNDS`, `metric`/`eval_at` from
    `LIGHTGBM_HYPERPARAMETERS`) -- `val_rows` must be a genuinely different
    split from `train_rows` (leakage-safe by the caller's own
    train/val/test split, not re-checked here).

    Returns `{"ranker", "best_iteration", "hyperparameters",
    "package_versions"}` -- everything an eval artifact needs to record
    and audit this exact training run.
    """
    try:
        import lightgbm as lgb
        import numpy as np
    except ImportError as e:
        raise ImportError(
            "lightgbm is required for --train/--evaluate. Install it with "
            "`pip install lightgbm` (not a pyproject.toml dependency -- "
            "this script is a standalone dev tool, like the other "
            "scripts/*.py training scripts in this repo). numpy comes in "
            "as lightgbm's own dependency."
        ) from e

    # Sort by (group_id, candidate_id): group_sizes() only needs group_id
    # runs to be consecutive, but the secondary candidate_id key makes row
    # order -- and therefore training input -- independent of whatever
    # order the JSONL happened to list candidates in.
    rows = sorted(train_rows, key=lambda r: (r.group_id, r.candidate_id))
    X = np.asarray([r.features for r in rows], dtype=np.float64)
    y = np.asarray([r.label for r in rows], dtype=np.float64)
    groups = group_sizes(rows)

    ranker = lgb.LGBMRanker(**LIGHTGBM_HYPERPARAMETERS)
    fit_kwargs = {}
    if val_rows:
        # lightgbm 4.7's eval_set validation rejects a plain list-of-lists
        # ("Data list can only be of ndarray or Sequence") even though the
        # primary X/y accept one fine -- explicit ndarrays sidestep that
        # version-specific quirk.
        val_sorted = sorted(val_rows, key=lambda r: (r.group_id, r.candidate_id))
        X_val = np.asarray([r.features for r in val_sorted], dtype=np.float64)
        y_val = np.asarray([r.label for r in val_sorted], dtype=np.float64)
        fit_kwargs["eval_set"] = [(X_val, y_val)]
        fit_kwargs["eval_group"] = [group_sizes(val_sorted)]
        fit_kwargs["callbacks"] = [lgb.early_stopping(EARLY_STOPPING_ROUNDS, verbose=False)]
    # `eval_at` is deliberately fixed via the constructor (part of
    # LIGHTGBM_HYPERPARAMETERS, for provenance) rather than passed to
    # fit() -- lightgbm logs a purely informational "Found 'eval_at' in
    # params" notice about that precedence on every call (via its own
    # internal logger, not Python's `warnings` module, so it can't be
    # filtered from here); it does not indicate a problem.
    ranker.fit(X, y, group=groups, **fit_kwargs)

    return {
        "ranker": ranker,
        "best_iteration": getattr(ranker, "best_iteration_", None),
        "hyperparameters": dict(LIGHTGBM_HYPERPARAMETERS),
        "package_versions": {"lightgbm": lgb.__version__},
    }


def lightgbm_score_fn(ranker):
    """Wraps a trained `LGBMRanker` as an `evaluate()`-compatible `score_fn`
    -- the same scoring interface every deterministic baseline arm uses
    (see `BASELINE_ARMS`), so the trained model and the arms share one
    evaluation code path.
    """

    def score_fn(rows):
        return list(ranker.predict([r.features for r in rows]))

    return score_fn


def compute_group_metrics(ranked_rows: list) -> dict:
    """Per-group raw metrics from `ranked_rows`, already sorted by (score
    descending, candidate_id ascending) -- see `evaluate`'s tie-break.
    Binary relevance (label 0/1); NDCG@10's ideal DCG is capped at
    `min(n_positives, 10)`, the standard binary-gain NDCG@k normalization.
    `has_positive=False` marks a coverage gap (see `summarize_coverage`) --
    every field here is well-defined only when `has_positive` is true;
    `evaluate`'s end-to-end aggregation is what decides how a coverage gap
    contributes (0, not "skip"; see `aggregate_metrics`).
    """
    labels = [r.label for r in ranked_rows]
    n_pos = sum(labels)
    if n_pos == 0:
        return {
            "has_positive": False,
            "top1_hit": 0,
            "top10_hit": 0,
            "reciprocal_rank": 0.0,
            "ndcg10": 0.0,
            "best_positive_rank": None,
        }
    best_positive_rank = next(i + 1 for i, label in enumerate(labels) if label == 1)
    dcg = sum(1.0 / math.log2(i + 2) for i, label in enumerate(labels[:10]) if label == 1)
    idcg = sum(1.0 / math.log2(i + 2) for i in range(min(n_pos, 10)))
    return {
        "has_positive": True,
        "top1_hit": 1 if labels[0] == 1 else 0,
        "top10_hit": 1 if any(labels[:10]) else 0,
        "reciprocal_rank": 1.0 / best_positive_rank,
        "ndcg10": (dcg / idcg) if idcg > 0 else 0.0,
        "best_positive_rank": best_positive_rank,
    }


def _mean(xs: list):
    return (sum(xs) / len(xs)) if xs else None


def aggregate_metrics(per_group_metrics: dict, group_ids_all: list, group_ids_with_positive: list) -> dict:
    """Two denominators, computed from the SAME `per_group_metrics`:

    - `conditional`: only groups with a positive candidate in their own
      pool (`group_ids_with_positive`) -- "given that ranking is possible at
      all, how good is it".
    - `end_to_end`: every labeled group for this split (`group_ids_all`,
      built from the group/target index -- see `evaluate`), including a
      group that was never scored (no rows, or no positive) at all. A
      missing/zero-positive group contributes 0 to every metric here
      (coverage miss = 0), it is never excluded from the denominator --
      that is precisely what makes this variant "end-to-end" rather than
      conditional.
    """
    cond = [per_group_metrics[g] for g in group_ids_with_positive]
    e2e = [per_group_metrics.get(g, {"top1_hit": 0, "top10_hit": 0, "reciprocal_rank": 0.0, "ndcg10": 0.0}) for g in group_ids_all]
    return {
        "conditional": {
            "top1_hit_rate": _mean([m["top1_hit"] for m in cond]),
            "top10_hit_rate": _mean([m["top10_hit"] for m in cond]),
            "mean_reciprocal_rank": _mean([m["reciprocal_rank"] for m in cond]),
            "ndcg_at_10": _mean([m["ndcg10"] for m in cond]),
            "mean_best_positive_rank": _mean([m["best_positive_rank"] for m in cond]),
            "n_groups": len(cond),
        },
        "end_to_end": {
            "top1_hit_rate": _mean([m["top1_hit"] for m in e2e]),
            "top10_hit_rate": _mean([m["top10_hit"] for m in e2e]),
            "mean_reciprocal_rank": _mean([m["reciprocal_rank"] for m in e2e]),
            "ndcg_at_10": _mean([m["ndcg10"] for m in e2e]),
            "n_groups": len(e2e),
        },
    }


def compute_arm_group_metrics(score_fn, rows: list, split: str) -> dict:
    """Score every group in `split` with `score_fn` and return the raw
    `group_id -> compute_group_metrics(...)` dict -- the shared inner loop
    behind both `evaluate()` (which aggregates it into conditional/
    end-to-end summaries) and `paired_bootstrap` (which resamples which
    groups' ALREADY-COMPUTED metrics contribute, without rescoring).

    `score_fn(group_rows: list[LabeledRow]) -> list[float]` is the ONE
    scoring interface shared by the trained LightGBM ranker (see
    `lightgbm_score_fn`) and every deterministic baseline arm (see
    `build_baseline_arms`) -- only what produces the scores differs.
    """
    by_group: dict = {}
    for r in rows:
        if r.split == split:
            by_group.setdefault(r.group_id, []).append(r)

    per_group_metrics: dict = {}
    for group_id, group_rows in by_group.items():
        scores = score_fn(group_rows)
        if len(scores) != len(group_rows):
            raise ValueError(
                f"score_fn returned {len(scores)} score(s) for {len(group_rows)} "
                f"row(s) in group_id={group_id!r} -- a scorer must return exactly "
                "one score per row"
            )
        for s in scores:
            if not math.isfinite(s):
                raise ValueError(
                    f"score_fn returned a non-finite score ({s!r}) for "
                    f"group_id={group_id!r}"
                )
        # Explicit candidate_id secondary key: many scorers (a trained
        # LightGBM ranker, or a deterministic arm reading a coarse feature)
        # produce exact score ties, and a tie broken by whatever order the
        # rows arrived in would make every metric here depend on JSONL line
        # order rather than on the scorer. Score is negated so ascending
        # sort puts the highest score first -- `reverse=True` would also
        # reverse the candidate_id tie-break, which is not what we want.
        ranked = sorted(zip(scores, group_rows), key=lambda p: (-p[0], p[1].candidate_id))
        per_group_metrics[group_id] = compute_group_metrics([r for _, r in ranked])
    return per_group_metrics


def evaluate(score_fn, rows: list, group_records: list, labels: dict, split: str) -> dict:
    """Evaluate an arbitrary scoring method on `split`, computed per ranking
    group (`group_id`) via `compute_arm_group_metrics`.

    Returns both `conditional` (only groups with a positive in their own
    pool) and `end_to_end` (every labeled group for this split, coverage
    miss = 0) metrics -- see `aggregate_metrics`. Top-level
    `top1_hit_rate`/`mean_reciprocal_rank` mirror the conditional variant,
    for backward compatibility with earlier reports.
    """
    per_group_metrics = compute_arm_group_metrics(score_fn, rows, split)

    coverage = summarize_coverage(rows, group_records, labels, split)
    group_ids_with_positive = [g for g, m in per_group_metrics.items() if m["has_positive"]]
    group_ids_all = [
        record["group_id"]
        for record in group_records
        if record["group_id"] in labels and split_for_target(record["target_id"]) == split
    ]

    metrics = aggregate_metrics(per_group_metrics, group_ids_all, group_ids_with_positive)
    return {
        "split": split,
        "target_count": coverage.target_count,
        "group_count": coverage.group_count,
        "scored_groups": len(group_ids_with_positive),
        "groups_with_zero_positive_in_pool": coverage.groups_with_zero_positive,
        "top1_hit_rate": metrics["conditional"]["top1_hit_rate"],
        "mean_reciprocal_rank": metrics["conditional"]["mean_reciprocal_rank"],
        "conditional": metrics["conditional"],
        "end_to_end": metrics["end_to_end"],
    }


# ---------------------------------------------------------------------------
# Baseline arms (A-H)
#
# Arms A-G are deterministic scoring functions over already-loaded rows --
# no LightGBM, no training -- all sharing `evaluate()`'s one code path (see
# that function's doc), computed on the SAME candidate pool as arm H. Arm H
# (the fully trained model) is NOT built here -- it needs a fitted
# LGBMRanker, wired in separately by the caller (see `run_baseline_arms`).
#
# A row missing an arm's relevant feature (e.g. best_upstream_score under
# Exhaustive/BondIndexed mode, where no scorer ran at all) is scored with
# `_MISSING_SENTINEL` -- a large-but-finite negative value, never NaN/Inf
# (`evaluate()` hard-rejects non-finite scores) -- so it deterministically
# ranks last instead of corrupting the comparison. If EVERY row in a split
# is missing that feature, the whole arm is reported as "not computable"
# for that pool (see `arm_is_computable`) rather than silently reporting a
# same-sentinel-value tie for every candidate as if it were a real result.
# ---------------------------------------------------------------------------

_MISSING_SENTINEL = -1e18


def feature_index_of(name: str) -> int:
    return FEATURE_NAMES_V1.index(name)


def fit_template_frequency(train_rows: list) -> dict:
    """Fit a train-frozen `template_id -> log-frequency` table from
    TRAIN-split rows' `source_template_ids` ONLY -- never from val/test
    rows, labels, or counts (this table is later looked up for val/test
    rows too, so it must never have seen them; that is what makes it
    leakage-safe).

    Fit policy (documented deliberately, not incidental): counts each
    template_id's occurrence across every TRAIN-split candidate row's
    `sources`, regardless of that candidate's own label -- i.e. "how often
    is this template proposed as a disconnection option in the training
    data", not "how often is it the CORRECT answer". The latter would only
    be definable for positive candidates and would leak label information
    into what is meant to be a purely template-prevalence feature; the
    former is computable from `sources` alone (present on every row,
    positive or not) and mirrors what `RetroRule.weight`'s pre-existing
    "raw" frequency already measures -- just recomputed from the actual
    train split instead of whatever full rule set a caller happened to
    pass to `propose_one_step`.
    """
    counts: dict = {}
    for row in train_rows:
        for template_id in row.source_template_ids:
            counts[template_id] = counts.get(template_id, 0) + 1
    return {template_id: math.log(count + 1) for template_id, count in counts.items()}


def template_frequency_table_sha256(freq_table: dict) -> str:
    """SHA-256 over the fitted table (sorted by template_id), so a fitted
    table can be persisted and later verified unchanged without re-fitting."""
    h = hashlib.sha256()
    for template_id in sorted(freq_table):
        h.update(template_id.encode("utf-8"))
        h.update(b"\0")
        h.update(repr(freq_table[template_id]).encode("utf-8"))
        h.update(b"\0")
    return f"sha256:{h.hexdigest()}"


def impute_frequency_features(rows: list, freq_table: dict) -> list:
    """Return NEW `LabeledRow` copies with `max_template_log_frequency`/
    `mean_template_log_frequency` (feature indices 16/17, always `missing`
    in every exported row -- see `FEATURE_NAMES_V1`'s doc) populated from
    `freq_table` instead of left NaN.

    This is used ONLY for arm H (the "full configured model" baseline),
    which is meant to use every available signal, including template
    frequency -- arms A-G and the exported `FEATURE_NAMES_V1` schema itself
    are entirely unaffected. This is a deliberate, documented post-hoc
    imputation performed by this training script alone: `feature_schema_v1`
    stays frozen and the Rust exporter never populates these two features
    (that would require split-aware recomputation baked into the export
    step itself, which does not exist), so arm H's own training/evaluation
    input is the one place index 16/17 get a real value, and that value is
    never written back to any exported pool file.

    A candidate with no known (TRAIN-seen) source template is left NaN --
    genuinely no information to impute, not a value to guess.
    """
    max_i = feature_index_of("max_template_log_frequency")
    mean_i = feature_index_of("mean_template_log_frequency")
    imputed = []
    for r in rows:
        known = [freq_table[t] for t in r.source_template_ids if t in freq_table]
        new_features = list(r.features)
        if known:
            new_features[max_i] = max(known)
            new_features[mean_i] = sum(known) / len(known)
        imputed.append(replace(r, features=new_features))
    return imputed


def _feature_score(index: int):
    def score_fn(rows):
        return [
            _MISSING_SENTINEL if math.isnan(r.features[index]) else r.features[index]
            for r in rows
        ]

    return score_fn


def _frequency_score(freq_table: dict):
    def score_fn(rows):
        scores = []
        for r in rows:
            # A candidate can have multiple contributing templates (merged
            # sources) -- score by the MAX fitted frequency among them (the
            # single strongest prior-frequency justification for this
            # candidate), not the mean, so one well-attested template isn't
            # diluted by other, rarer co-contributing ones.
            known = [freq_table[t] for t in r.source_template_ids if t in freq_table]
            scores.append(max(known) if known else _MISSING_SENTINEL)
        return scores

    return score_fn


def _rank_fusion_score(component_score_fns: list):
    """Combine several component `score_fn`s via rank fusion: within the
    rows being scored, each component ranks all of them (ties broken by
    candidate_id, matching `evaluate()`'s own tie-break), and the fused
    score is the negative sum of those ranks (lower total rank -> higher
    fused score). This avoids needing to normalize component scores onto a
    common scale -- upstream logits and log-frequencies live on very
    different scales -- a well-known, robust alternative to a hand-tuned
    weighted sum (reciprocal/Borda rank fusion).
    """

    def score_fn(rows):
        total_rank = [0] * len(rows)
        for component in component_score_fns:
            component_scores = component(rows)
            order = sorted(
                range(len(rows)), key=lambda i: (-component_scores[i], rows[i].candidate_id)
            )
            for rank, i in enumerate(order):
                total_rank[i] += rank
        return [float(-r) for r in total_rank]

    return score_fn


def _reaction_center_score(rows):
    scores = []
    extractable_i = feature_index_of("reaction_center_extractable_fraction")
    mean_i = feature_index_of("reaction_center_atom_count_mean")
    for r in rows:
        extractable_fraction = r.features[extractable_i]
        if math.isnan(extractable_fraction) or extractable_fraction == 0.0:
            # extractable_fraction == 0.0 means "no source template's
            # reaction center was extractable", not "a reaction center of
            # size 0" -- treating it as the smallest (best) reaction center
            # would silently reward a candidate this arm has no real signal
            # about, so it is scored as not-computable-for-this-candidate
            # instead.
            scores.append(_MISSING_SENTINEL)
        else:
            scores.append(-r.features[mean_i])
    return scores


def build_baseline_arms(freq_table: dict) -> list:
    """Arms A-G, each `{name, description, score_fn, computable_fn}`.
    `computable_fn(rows) -> bool` decides, for a given split's rows,
    whether this arm has any real signal at all -- see the module
    docstring above `_MISSING_SENTINEL`.
    """
    upstream_i = feature_index_of("best_upstream_score")
    stock_frac_i = feature_index_of("fraction_precursors_in_stock")
    stock_all_i = feature_index_of("all_precursors_in_stock")
    charge_i = feature_index_of("net_charge_balanced")
    no_gain_i = feature_index_of("no_heavy_atom_gain")
    n_precursors_i = feature_index_of("num_precursors")
    extractable_i = feature_index_of("reaction_center_extractable_fraction")

    upstream_score_fn = _feature_score(upstream_i)
    frequency_score_fn = _frequency_score(freq_table)

    def upstream_computable(rows):
        return any(not math.isnan(r.features[upstream_i]) for r in rows)

    def frequency_computable(rows):
        return any(t in freq_table for r in rows for t in r.source_template_ids)

    def availability_score_fn(rows):
        scores = []
        for r in rows:
            frac = r.features[stock_frac_i]
            if math.isnan(frac):
                scores.append(_MISSING_SENTINEL)
            else:
                scores.append(frac + r.features[stock_all_i])
        return scores

    def availability_computable(rows):
        return any(not math.isnan(r.features[stock_frac_i]) for r in rows)

    def reaction_center_computable(rows):
        return any(
            not math.isnan(r.features[extractable_i]) and r.features[extractable_i] > 0.0
            for r in rows
        )

    return [
        {
            "name": "original_rank",
            "description": (
                "Ascending best_upstream_rank (the order the proposal/scorer "
                "originally produced candidates in) -- a sanity-check baseline: "
                "if the reranker can't beat 'trust the upstream order', it isn't "
                "adding anything."
            ),
            "score_fn": lambda rows: [-float(r.best_upstream_rank) for r in rows],
            "computable_fn": lambda rows: True,
        },
        {
            "name": "upstream_score",
            "description": (
                "best_upstream_score alone -- only meaningful for a "
                "ScorerConditioned pool (always missing under Exhaustive/"
                "BondIndexed, see FEATURE_NAMES_V1's doc)."
            ),
            "score_fn": upstream_score_fn,
            "computable_fn": upstream_computable,
        },
        {
            "name": "template_frequency",
            "description": (
                "Train-frozen template-frequency table (see "
                "fit_template_frequency) -- 'prefer whatever templates were "
                "common in training', independent of this candidate's own "
                "chemistry."
            ),
            "score_fn": frequency_score_fn,
            "computable_fn": frequency_computable,
        },
        {
            "name": "upstream_plus_frequency",
            "description": "Rank fusion (see _rank_fusion_score) of upstream_score and template_frequency.",
            "score_fn": _rank_fusion_score([upstream_score_fn, frequency_score_fn]),
            "computable_fn": lambda rows: upstream_computable(rows) or frequency_computable(rows),
        },
        {
            "name": "structural",
            "description": "net_charge_balanced + no_heavy_atom_gain - num_precursors -- prefer chemistry-valid, low-precursor-count candidates.",
            "score_fn": lambda rows: [
                r.features[charge_i] + r.features[no_gain_i] - r.features[n_precursors_i]
                for r in rows
            ],
            "computable_fn": lambda rows: any(not math.isnan(r.features[n_precursors_i]) for r in rows),
        },
        {
            "name": "reaction_center",
            "description": "Prefer a smaller, well-defined (extractable) reaction center -- see _reaction_center_score.",
            "score_fn": _reaction_center_score,
            "computable_fn": reaction_center_computable,
        },
        {
            "name": "availability",
            "description": (
                "fraction_precursors_in_stock + all_precursors_in_stock -- "
                "prefer candidates whose precursors are actually purchasable. "
                "Not computable without a stock supplied to feature extraction."
            ),
            "score_fn": availability_score_fn,
            "computable_fn": availability_computable,
        },
    ]


def run_baseline_arms(freq_table: dict, rows: list, group_records: list, labels: dict, split: str) -> dict:
    """Evaluate every arm A-G on `split`, reporting `{"status": "not_computable"}`
    (never a misleading numeric result) for an arm whose `computable_fn`
    returns false on this split's rows. Arm H (the trained model) is not
    included -- see `build_baseline_arms`'s doc.
    """
    split_rows = [r for r in rows if r.split == split]
    results = {}
    for arm in build_baseline_arms(freq_table):
        if not arm["computable_fn"](split_rows):
            results[arm["name"]] = {"status": "not_computable", "description": arm["description"]}
            continue
        report = evaluate(arm["score_fn"], rows, group_records, labels, split)
        report["description"] = arm["description"]
        report["status"] = "ok"
        results[arm["name"]] = report
    return results


# ---------------------------------------------------------------------------
# Paired bootstrap + offline gate
#
# Explicitly NOT run against any real/formal data in this commit -- this is
# the tooling itself, exercised only against synthetic self-test fixtures
# until a real (30-target or larger) evaluation exists (see this script's
# module doc and the repo's staged candidate-pool gate).
# ---------------------------------------------------------------------------

GATE_THRESHOLDS = {
    "top1_hit_rate_min_delta": 0.01,
    "mean_reciprocal_rank_min_delta": 0.01,
    "top10_hit_rate_max_regression": 0.002,
}

_E2E_METRIC_KEYS = ("top1_hit_rate", "top10_hit_rate", "mean_reciprocal_rank")


def _e2e_group_value(per_group_metrics: dict, group_id: str, key: str) -> float:
    """One group's end-to-end contribution for `key` -- 0.0 (coverage
    miss) if the group was never scored (no rows) or has no positive
    candidate, matching `aggregate_metrics`'s end-to-end semantics exactly.
    """
    m = per_group_metrics.get(group_id)
    if m is None or not m["has_positive"]:
        return 0.0
    return {
        "top1_hit_rate": float(m["top1_hit"]),
        "top10_hit_rate": float(m["top10_hit"]),
        "mean_reciprocal_rank": m["reciprocal_rank"],
    }[key]


def _percentile(values: list, p: float):
    if not values:
        return None
    values_sorted = sorted(values)
    index = min(len(values_sorted) - 1, max(0, round(p * (len(values_sorted) - 1))))
    return values_sorted[index]


def paired_bootstrap(
    baseline_metrics: dict,
    treatment_metrics: dict,
    target_to_groups: dict,
    n_resamples: int = 1000,
    seed: int = 1234,
) -> dict:
    """Paired bootstrap over TARGET_ID clusters -- never individual groups.
    `target_id` is the leakage-safe split key (see `split_for_target`), so
    two groups sharing a `target_id` must always resample together, exactly
    like the train/val/test split itself never separates them. Resampling
    an individual `group_id` instead would silently violate that same
    invariant the split was built to protect.

    `baseline_metrics`/`treatment_metrics` are per-`group_id` metrics dicts
    (see `compute_arm_group_metrics`), already computed ONCE over the real
    (non-resampled) data -- each iteration only changes which groups'
    already-computed end-to-end values contribute to the mean (see
    `_e2e_group_value`); no rescoring happens per resample.

    "Paired": baseline and treatment are evaluated on the IDENTICAL
    resampled group set every iteration, so the delta directly reflects a
    real difference between the two arms, not resampling noise common to
    both.

    Uses Python's stdlib `random.Random(seed)`, not numpy -- deterministic
    given the same seed, independent of whatever numpy version (if any) is
    installed. `seed` is recorded in the result, never left implicit.
    """
    target_ids = sorted(target_to_groups)
    n = len(target_ids)
    rng = random.Random(seed)
    deltas = {key: [] for key in _E2E_METRIC_KEYS}

    for _ in range(n_resamples):
        sampled_groups = []
        for _ in range(n):
            t = target_ids[rng.randrange(n)] if n else None
            if t is not None:
                sampled_groups.extend(target_to_groups[t])
        for key in _E2E_METRIC_KEYS:
            baseline_mean = _mean([_e2e_group_value(baseline_metrics, g, key) for g in sampled_groups])
            treatment_mean = _mean([_e2e_group_value(treatment_metrics, g, key) for g in sampled_groups])
            if baseline_mean is None or treatment_mean is None:
                continue  # n == 0 (no target_ids at all) -- nothing to resample
            deltas[key].append(treatment_mean - baseline_mean)

    return {
        "n_resamples": n_resamples,
        "seed": seed,
        "resample_unit": "target_id (cluster bootstrap -- all groups sharing a target_id resample together)",
        "n_target_ids": n,
        "deltas": {
            key: {
                "mean_delta": _mean(values),
                "ci_95": [_percentile(values, 0.025), _percentile(values, 0.975)],
                "n_resamples_used": len(values),
            }
            for key, values in deltas.items()
        },
    }


def run_offline_gate(
    baseline_score_fn,
    treatment_score_fn,
    rows: list,
    group_records: list,
    labels: dict,
    split: str,
    baseline_arm: str,
    treatment_arm: str,
    n_resamples: int = 1000,
    seed: int = 1234,
) -> dict:
    """Compute both arms' per-group metrics on `split`, hard-verify they
    scored the IDENTICAL group set (see `evaluate_offline_gate`'s doc on
    why this is a structural assertion, not a metric comparison), bootstrap
    the paired delta, and judge it against the predefined offline gate.
    """
    baseline_metrics = compute_arm_group_metrics(baseline_score_fn, rows, split)
    treatment_metrics = compute_arm_group_metrics(treatment_score_fn, rows, split)
    if set(baseline_metrics) != set(treatment_metrics):
        raise ValueError(
            f"baseline_arm={baseline_arm!r} and treatment_arm={treatment_arm!r} scored "
            "different group sets on the same pool/split -- both arms must run over the "
            "identical candidate pool for a gate comparison to be meaningful"
        )

    target_to_groups: dict = {}
    for record in group_records:
        if record["group_id"] in labels and split_for_target(record["target_id"]) == split:
            target_to_groups.setdefault(record["target_id"], []).append(record["group_id"])

    bootstrap_result = paired_bootstrap(
        baseline_metrics, treatment_metrics, target_to_groups, n_resamples=n_resamples, seed=seed
    )
    return evaluate_offline_gate(bootstrap_result, coverage_identical=True, baseline_arm=baseline_arm, treatment_arm=treatment_arm)


def evaluate_offline_gate(
    bootstrap_result: dict, coverage_identical: bool, baseline_arm: str, treatment_arm: str
) -> dict:
    """Machine-judge the predefined offline gate against a `paired_bootstrap`
    result -- PASS requires ALL of:

      - `coverage_identical`: both arms scored the IDENTICAL group set (a
        structural assertion the caller establishes -- see
        `run_offline_gate` -- never a metric comparison, which would be
        trivially true whenever both arms simply share one candidate pool
        and would catch nothing).
      - end-to-end top-1 hit rate delta (treatment - baseline) >=
        `GATE_THRESHOLDS["top1_hit_rate_min_delta"]` (+1.0pp).
      - end-to-end mean reciprocal rank delta >=
        `GATE_THRESHOLDS["mean_reciprocal_rank_min_delta"]` (+0.01).
      - end-to-end top-10 hit rate delta >=
        `-GATE_THRESHOLDS["top10_hit_rate_max_regression"]` (regression
        capped at 0.2pp, not merely "not worse").
      - the top-1 hit rate delta's 95% CI lower bound > 0 (the improvement
        is not attributable to resampling noise alone).
    """
    top1 = bootstrap_result["deltas"]["top1_hit_rate"]
    mrr = bootstrap_result["deltas"]["mean_reciprocal_rank"]
    top10 = bootstrap_result["deltas"]["top10_hit_rate"]

    checks = {
        "coverage_unchanged": bool(coverage_identical),
        "top1_hit_rate_delta_meets_threshold": (
            top1["mean_delta"] is not None and top1["mean_delta"] >= GATE_THRESHOLDS["top1_hit_rate_min_delta"]
        ),
        "mean_reciprocal_rank_delta_meets_threshold": (
            mrr["mean_delta"] is not None
            and mrr["mean_delta"] >= GATE_THRESHOLDS["mean_reciprocal_rank_min_delta"]
        ),
        "top10_hit_rate_regression_within_threshold": (
            top10["mean_delta"] is not None
            and top10["mean_delta"] >= -GATE_THRESHOLDS["top10_hit_rate_max_regression"]
        ),
        "top1_hit_rate_ci_lower_bound_positive": (
            top1["ci_95"][0] is not None and top1["ci_95"][0] > 0
        ),
    }
    return {
        "baseline_arm": baseline_arm,
        "treatment_arm": treatment_arm,
        "result": "PASS" if all(checks.values()) else "FAIL",
        "checks": checks,
        "thresholds": dict(GATE_THRESHOLDS),
        "bootstrap": bootstrap_result,
    }


def self_test() -> None:
    """Fast (well under a second), dependency-free smoke check of the core
    deterministic logic against a tiny embedded fixture: split determinism,
    a minimal manifest/row schema round-trip, missing-feature-to-NaN
    labeling, evaluate()'s tie-break, and a tiny paired-bootstrap +
    offline-gate smoke. If lightgbm is importable, also runs a minimal
    end-to-end train+evaluate pass.

    Detailed regression coverage (schema/label/metrics/baseline-arm/
    bootstrap/training edge cases) lives in scripts/tests/ instead -- run
    via `python3 -m unittest discover -s scripts/tests -p "test_*.py"` --
    not duplicated here; this function is deliberately just a quick
    "is the core logic sane" signal for a developer without lightgbm/
    scikit-learn installed, not a substitute for that suite.
    """
    import math
    import os
    import tempfile

    # -- split determinism --
    ids = ["target_a", "target_b", "target_c"]
    assert {t: split_for_target(t) for t in ids} == {t: split_for_target(t) for t in ids}, (
        "split assignment must be deterministic"
    )

    # -- minimal manifest + candidate-row schema round-trip --
    with tempfile.TemporaryDirectory() as tmp:
        pool_path = os.path.join(tmp, "pool.jsonl")
        with open(pool_path, "w", encoding="utf-8") as f:
            f.write('{"candidate_id": "c1"}\n')
        groups_path = os.path.join(tmp, "groups.jsonl")
        with open(groups_path, "w", encoding="utf-8") as f:
            f.write('{"group_id": "g1"}\n')
        manifest = {
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
        validate_manifest(manifest, pool_path, groups_path)  # must not raise

        pool_row = {
            "group_id": "g1", "target_id": "t1", "target_smiles": "CCO", "candidate_id": "c1",
            "precursor_smiles": ["CCO"], "sources": [{"template_id": "rule:x"}],
            "feature_schema_version": FEATURE_SCHEMA_VERSION,
            "feature_values": [0.0] * len(FEATURE_NAMES_V1),
            "feature_missing": [True] * len(FEATURE_NAMES_V1),
        }
        group_records = [
            {"group_id": "g1", "target_id": "t1", "target_smiles": "CCO", "candidate_count": 1, "proposal_status": "ok"},
        ]
        validate_pool_rows([pool_row], group_records)  # must not raise

    print("self-test: minimal schema-loader fixture OK", flush=True)

    # -- labeling + missing-feature-to-NaN --
    labels = {"g1": GroupLabel(target_id="t1", correct_precursor_sets=frozenset({("CCO",)}))}
    labeled, unlabeled_count = label_and_split_rows([pool_row], labels, group_records)
    assert unlabeled_count == 0
    assert labeled[0].label == 1, "exact precursor-set match must be labeled positive"
    assert math.isnan(labeled[0].features[-1]), "a missing feature must become NaN, not 0"

    print("self-test: labeling and missing-feature-to-NaN OK", flush=True)

    # -- evaluate() ranking tie-break is deterministic, independent of lightgbm --
    def constant_score_fn(rows):
        return [0.0] * len(rows)

    tie_group_records = [
        {"group_id": "t", "target_id": "tt", "target_smiles": "tt", "candidate_count": 2, "proposal_status": "ok"},
    ]
    tie_labels = {"t": GroupLabel(target_id="tt", correct_precursor_sets=frozenset({("x",)}))}
    tied_a = [
        LabeledRow("t", "tt", "sha256:bbb", [1.0], 1, "test"),
        LabeledRow("t", "tt", "sha256:aaa", [1.0], 0, "test"),
    ]
    tied_b = list(reversed(tied_a))
    report_a = evaluate(constant_score_fn, tied_a, tie_group_records, tie_labels, "test")
    report_b = evaluate(constant_score_fn, tied_b, tie_group_records, tie_labels, "test")
    assert report_a == report_b, "tied scores must rank identically regardless of input row order"

    print("self-test: evaluate() tie-break is order-independent OK", flush=True)

    # -- tiny paired-bootstrap + offline-gate smoke --
    def metric(top1, mrr, top10):
        return {"has_positive": True, "top1_hit": top1, "top10_hit": top10, "reciprocal_rank": mrr, "ndcg10": 0.0, "best_positive_rank": 1}

    target_to_groups = {"ta": ["gg1"], "tb": ["gg2"]}
    baseline_metrics = {g: metric(0, 0.0, 0) for g in ("gg1", "gg2")}
    treatment_metrics = {g: metric(1, 1.0, 1) for g in ("gg1", "gg2")}
    bootstrap_result = paired_bootstrap(baseline_metrics, treatment_metrics, target_to_groups, n_resamples=20, seed=1)
    gate = evaluate_offline_gate(bootstrap_result, coverage_identical=True, baseline_arm="a", treatment_arm="b")
    assert gate["result"] == "PASS"

    print("self-test: paired bootstrap + offline gate smoke OK", flush=True)

    try:
        import lightgbm  # noqa: F401
    except ImportError:
        print(
            "self-test: lightgbm not installed -- skipping end-to-end train+evaluate "
            "(pip install lightgbm to exercise it)",
            flush=True,
        )
        return

    # -- minimal end-to-end train+evaluate smoke (a code-path check, not a
    # model-quality check -- a handful of synthetic groups is far too small
    # to mean anything about ranking quality). --
    smoke_rows = []
    smoke_group_records = []
    smoke_labels = {}
    for i in range(8):
        gid, tid = f"smoke-g{i}", f"smoke-t{i}"
        smoke_group_records.append(
            {"group_id": gid, "target_id": tid, "target_smiles": tid, "candidate_count": 2, "proposal_status": "ok"}
        )
        smoke_labels[gid] = GroupLabel(target_id=tid, correct_precursor_sets=frozenset({("pos",)}))
        for j, label in enumerate((1, 0)):
            smoke_rows.append({
                "group_id": gid, "target_id": tid, "target_smiles": tid, "candidate_id": f"c{i}-{j}",
                "precursor_smiles": ["pos"] if label else ["neg"],
                "sources": [{"template_id": "rule:x"}],
                "feature_schema_version": FEATURE_SCHEMA_VERSION,
                "feature_values": [float(j)] * len(FEATURE_NAMES_V1),
                "feature_missing": [False] * 13 + [True] * 5,
                "best_upstream_rank": j,
            })
    labeled_smoke, _ = label_and_split_rows(smoke_rows, smoke_labels, smoke_group_records)
    train_rows_smoke = [r for r in labeled_smoke if r.split == "train"]
    if not train_rows_smoke:
        print("self-test: no synthetic row landed in the train split this run -- skipping lightgbm smoke", flush=True)
        return

    train_result = train_ranker(train_rows_smoke)
    score_fn = lightgbm_score_fn(train_result["ranker"])
    report = evaluate(score_fn, labeled_smoke, smoke_group_records, smoke_labels, "train")
    assert "conditional" in report and "end_to_end" in report

    print("self-test: end-to-end train+evaluate smoke OK", flush=True)


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
    parser.add_argument(
        "--split-manifest",
        help="JSONL explicit target_id -> split assignment ({\"target_id\": ..., "
             "\"split\": \"train\"|\"val\"|\"test\"}), overriding the default SHA-256 hash "
             "bucket for every target_id in --groups. Must cover every target_id in --groups "
             "exactly once (hard error otherwise) -- see load_split_manifest. Omit to use the "
             "default hash-bucket split, unchanged from prior behavior.",
    )
    parser.add_argument("--model-out", help="Path to save the trained LightGBM booster (text format)")
    parser.add_argument("--eval-out", help="Path to save the evaluation report JSON")
    parser.add_argument(
        "--gate-baseline-arm",
        help="Arm name (see the printed report's 'arms' keys, e.g. 'original_rank') to "
             "compare against --gate-treatment-arm via the offline gate. Both must be given "
             "together. Explicitly NOT intended to be run against real/formal data yet -- "
             "see this script's module doc.",
    )
    parser.add_argument("--gate-treatment-arm", help="Arm name to compare against --gate-baseline-arm.")
    parser.add_argument(
        "--gate-split", default="test",
        help="Split the gate is computed on (default: test -- the held-out split).",
    )
    parser.add_argument("--gate-out", help="Path to save the offline-gate PASS/FAIL report JSON.")
    parser.add_argument(
        "--bootstrap-resamples", type=int, default=1000,
        help="Number of paired-bootstrap resamples for the offline gate (default: 1000).",
    )
    parser.add_argument(
        "--bootstrap-seed", type=int, default=1234,
        help="Fixed seed for the paired bootstrap (default: 1234) -- always recorded in the "
             "gate report, so a run is reproducible.",
    )
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

    if args.split_manifest:
        known_target_ids = {r["target_id"] for r in group_records}
        split_assignments = load_split_manifest(args.split_manifest, known_target_ids)
        configure_split_override(split_assignments)
        print(
            f"Loaded split manifest {args.split_manifest} "
            f"(sha256={sha256_file(args.split_manifest)}): "
            f"{len(split_assignments)} target_id assignments, overriding the default "
            "hash-bucket split for this run",
            flush=True,
        )
    else:
        configure_split_override(None)

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
    val_rows = [r for r in labeled if r.split == "val"]
    if not train_rows:
        print("ERROR: no rows in the train split.", file=sys.stderr)
        sys.exit(1)

    # Train-frozen frequency table (see fit_template_frequency): fit from
    # TRAIN rows only, used both as its own baseline arm (C) and to impute
    # the otherwise-always-missing frequency features for arm H alone (see
    # impute_frequency_features).
    freq_table = fit_template_frequency(train_rows)
    imputed_labeled = impute_frequency_features(labeled, freq_table)
    imputed_train_rows = [r for r in imputed_labeled if r.split == "train"]
    imputed_val_rows = [r for r in imputed_labeled if r.split == "val"]

    train_result = train_ranker(imputed_train_rows, val_rows=imputed_val_rows or None)
    ranker = train_result["ranker"]

    if args.model_out:
        Path(args.model_out).parent.mkdir(parents=True, exist_ok=True)
        ranker.booster_.save_model(args.model_out)
        print(f"Saved model to {args.model_out}", flush=True)

    full_model_score_fn = lightgbm_score_fn(ranker)

    # Every arm (A-H) evaluated on the SAME candidate pool, through the
    # same evaluate() code path -- see BASELINE_ARMS/run_baseline_arms and
    # evaluate()'s own doc.
    arm_reports: dict = {}
    for split in ("train", "val", "test"):
        for arm_name, report in run_baseline_arms(freq_table, labeled, group_records, labels, split).items():
            arm_reports.setdefault(arm_name, {})[split] = report
        arm_reports.setdefault("full_configured_model", {})[split] = evaluate(
            full_model_score_fn, imputed_labeled, group_records, labels, split
        )

    result = {
        "feature_schema_version": from_export_schema,
        "proposal_mode": manifest.get("proposal_mode"),
        "rules_content_hash": manifest.get("rules_content_hash"),
        "unlabeled_group_count": unlabeled_group_count,
        "template_frequency_table_sha256": template_frequency_table_sha256(freq_table),
        "lightgbm": {
            "hyperparameters": train_result["hyperparameters"],
            "package_versions": train_result["package_versions"],
            "best_iteration": train_result["best_iteration"],
        },
        "arms": arm_reports,
    }
    print(json.dumps(result, indent=2), flush=True)

    if args.eval_out:
        Path(args.eval_out).parent.mkdir(parents=True, exist_ok=True)
        with open(args.eval_out, "w", encoding="utf-8") as f:
            json.dump(result, f, indent=2)
        print(f"Saved evaluation report to {args.eval_out}", flush=True)

    if bool(args.gate_baseline_arm) != bool(args.gate_treatment_arm):
        parser.error("--gate-baseline-arm and --gate-treatment-arm must be given together")
    if args.gate_baseline_arm and args.gate_treatment_arm:
        # Every baseline arm's score_fn, plus arm H's (full_configured_model)
        # -- imputed_labeled is used uniformly for both arms regardless of
        # which is chosen: imputation only touches features 16/17
        # (max/mean_template_log_frequency), which no arm A-G reads (they
        # use source_template_ids or other feature indices directly), so
        # this never changes a baseline arm's own score_fn output.
        arm_score_fns = {arm["name"]: arm["score_fn"] for arm in build_baseline_arms(freq_table)}
        arm_score_fns["full_configured_model"] = full_model_score_fn
        for name in (args.gate_baseline_arm, args.gate_treatment_arm):
            if name not in arm_score_fns:
                parser.error(
                    f"unknown arm {name!r} -- must be one of {sorted(arm_score_fns)}"
                )
        gate_result = run_offline_gate(
            arm_score_fns[args.gate_baseline_arm],
            arm_score_fns[args.gate_treatment_arm],
            imputed_labeled,
            group_records,
            labels,
            args.gate_split,
            args.gate_baseline_arm,
            args.gate_treatment_arm,
            n_resamples=args.bootstrap_resamples,
            seed=args.bootstrap_seed,
        )
        print(json.dumps(gate_result, indent=2), flush=True)
        if args.gate_out:
            Path(args.gate_out).parent.mkdir(parents=True, exist_ok=True)
            with open(args.gate_out, "w", encoding="utf-8") as f:
                json.dump(gate_result, f, indent=2)
            print(f"Saved offline-gate report to {args.gate_out}", flush=True)


if __name__ == "__main__":
    main()
