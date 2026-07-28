#!/usr/bin/env python3
"""
Train and evaluate a LambdaMART candidate reranker over an offline
candidate pool exported by `renkin::pool_export` (see src/pool_export.rs).

This script does NOT decide what pool to run it against, and it does not
gate anything -- it is a mechanism, not a decision. Whether/when to run it
against a real 100/500/full-target pool, and what to conclude from the
result, is a separate call (see this repo's staged candidate-pool gate).

Pipeline:
  1. Load a JSONL candidate pool (one row per candidate, schema per
     PoolManifest.feature_schema_version) and its sidecar manifest.
  2. Load ground-truth labels: one line per target, giving the known
     correct precursor set. A candidate row is positive (label=1) iff its
     precursor_smiles (as a sorted list of strings) exactly matches.
  3. Split by target_id (SHA-256(target_id) mod 100 -> bucket), NEVER by
     candidate -- every candidate for one target lands in exactly one of
     train/val/test. This is the leakage-prevention property; there is no
     other place in this script leakage could otherwise creep in.
  4. Train a LightGBM LGBMRanker (objective="lambdarank") on the train
     split, with one "group" per target.
  5. Evaluate on val/test: top-1 hit rate (does the highest-scored
     candidate for a target carry the positive label) and mean reciprocal
     rank, plus a separate "zero-positive-in-pool" count -- a target with
     no positive candidate at all is a *candidate-generation* coverage gap,
     not something any reranker could fix, and conflating the two would
     make ranking quality and pool coverage indistinguishable.

Requires (not declared in pyproject.toml -- this is a standalone dev
script, like the other scripts/*.py in this repo, e.g.
train_template_scorer.py's torch/datasets/rdchiral): `pip install lightgbm`.
Missing lightgbm is a hard error for --train/--evaluate; `--self-test`
still exercises everything that doesn't need it and reports what it
skipped.

Usage:
    python3 scripts/train_reranker.py \
        --pool data/pool.jsonl --manifest data/pool.manifest.json \
        --labels data/labels.jsonl \
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


def load_labels(path: str) -> dict:
    """target_id -> sorted tuple of correct precursor SMILES."""
    labels = {}
    for row in load_jsonl(path):
        precursors = tuple(sorted(row["correct_precursor_smiles"]))
        labels[row["target_id"]] = precursors
    return labels


@dataclass
class LabeledRow:
    target_id: str
    candidate_id: str
    features: list  # float, NaN where feature_missing[i] is True
    label: int
    split: str


def label_and_split_rows(pool_rows: list, labels: dict) -> list:
    """Attach a leakage-safe split and a binary label to every pool row.

    A row's label is 1 iff its precursor_smiles, sorted, exactly matches
    the labeled target's known-correct precursor set. A target absent from
    `labels` gets label 0 for all its candidates (not skipped) -- callers
    computing coverage should look at how many targets have zero positives
    among their OWN candidates, using `summarize_coverage`, not assume an
    unlabeled target silently disappears.
    """
    out = []
    for row in pool_rows:
        precursors = tuple(sorted(row["precursor_smiles"]))
        correct = labels.get(row["target_id"])
        label = 1 if correct is not None and precursors == correct else 0
        features = [
            float("nan") if m else v
            for v, m in zip(row["feature_values"], row["feature_missing"])
        ]
        out.append(
            LabeledRow(
                target_id=row["target_id"],
                candidate_id=row["candidate_id"],
                features=features,
                label=label,
                split=split_for_target(row["target_id"]),
            )
        )
    return out


@dataclass
class CoverageSummary:
    target_count: int = 0
    targets_with_zero_positive: int = 0
    positive_candidate_count: int = 0
    total_candidate_count: int = 0


def summarize_coverage(labeled_rows: list, split: str) -> CoverageSummary:
    by_target: dict = {}
    for r in labeled_rows:
        if r.split != split:
            continue
        by_target.setdefault(r.target_id, []).append(r)

    summary = CoverageSummary(target_count=len(by_target))
    for rows in by_target.values():
        summary.total_candidate_count += len(rows)
        positives = sum(1 for r in rows if r.label == 1)
        summary.positive_candidate_count += positives
        if positives == 0:
            summary.targets_with_zero_positive += 1
    return summary


def group_sizes(rows: list) -> list:
    """LightGBM group sizes: consecutive run-lengths per target_id. Callers
    must sort `rows` by target_id first (see `train_ranker`) -- LightGBM's
    `group` parameter is defined as consecutive counts, not a labeled
    grouping key.
    """
    sizes = []
    current = None
    count = 0
    for r in rows:
        if r.target_id != current:
            if count:
                sizes.append(count)
            current = r.target_id
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

    rows = sorted(train_rows, key=lambda r: r.target_id)
    X = [r.features for r in rows]
    y = [r.label for r in rows]
    groups = group_sizes(rows)

    ranker = lgb.LGBMRanker(objective="lambdarank", verbosity=-1)
    ranker.fit(X, y, group=groups)
    return ranker


def evaluate(ranker, rows: list, split: str) -> dict:
    """Top-1 hit rate and mean reciprocal rank on `split`, plus the
    zero-positive-in-pool coverage count (a target with no positive
    candidate in its pool can't be salvaged by any reranker -- reporting it
    alongside hit rate keeps the two failure modes distinguishable).
    """
    by_target: dict = {}
    for r in rows:
        if r.split == split:
            by_target.setdefault(r.target_id, []).append(r)

    top1_hits = 0
    reciprocal_ranks = []
    scored_targets = 0
    for target_rows in by_target.values():
        if not any(r.label == 1 for r in target_rows):
            continue  # coverage gap, not a ranking failure -- see summarize_coverage
        scored_targets += 1
        scores = ranker.predict([r.features for r in target_rows])
        # Explicit candidate_id secondary key: LightGBM produces exact score
        # ties on this feature set (many candidates share identical group-1
        # values), and a tie broken by whatever order the rows arrived in
        # would make top1_hit_rate/mean_reciprocal_rank depend on JSONL line
        # order rather than on the model. Score is negated so ascending sort
        # puts the highest score first -- `reverse=True` would also reverse
        # the candidate_id tie-break, which is not what we want.
        ranked = sorted(
            zip(scores, target_rows), key=lambda p: (-p[0], p[1].candidate_id)
        )
        if ranked[0][1].label == 1:
            top1_hits += 1
        rank = next(
            i + 1 for i, (_, r) in enumerate(ranked) if r.label == 1
        )
        reciprocal_ranks.append(1.0 / rank)

    coverage = summarize_coverage(rows, split)
    return {
        "split": split,
        "target_count": coverage.target_count,
        "scored_targets": scored_targets,
        "targets_with_zero_positive_in_pool": coverage.targets_with_zero_positive,
        "top1_hit_rate": (top1_hits / scored_targets) if scored_targets else None,
        "mean_reciprocal_rank": (
            sum(reciprocal_ranks) / len(reciprocal_ranks) if reciprocal_ranks else None
        ),
    }


def self_test() -> None:
    """Assert-based self-check of every piece of logic that doesn't need
    lightgbm; if lightgbm IS importable, also runs a tiny end-to-end
    train+evaluate pass and asserts it produces a well-formed report (not
    that any particular metric value is achieved -- 2-3 synthetic targets
    is far too small to mean anything, and this is a code-path check, not a
    model-quality check).
    """
    # -- split determinism and no target crosses splits --
    ids = ["target_a", "target_b", "target_c", "target_d", "target_e"]
    first = {t: split_for_target(t) for t in ids}
    second = {t: split_for_target(t) for t in ids}
    assert first == second, "split assignment must be deterministic"

    # -- label assignment --
    pool_rows = [
        {
            "target_id": "target_a",
            "target_smiles": "CC(=O)OCC",
            "candidate_id": "sha256:aaa",
            "precursor_smiles": ["CC(=O)O", "CCO"],
            "source_template_count": 1,
            "best_upstream_rank": 0,
            "feature_schema_version": 1,
            "feature_values": [2.0, 6.0, 6.0, 4.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.5, 0.0],
            "feature_missing": [False] * 12 + [True],
        },
        {
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
    labels = {"target_a": ("CC(=O)O", "CCO")}  # target_b has no label -> zero positives
    labeled = label_and_split_rows(pool_rows, labels)

    by_id = {r.candidate_id: r for r in labeled}
    assert by_id["sha256:aaa"].label == 1, "exact precursor-set match must be labeled positive"
    assert by_id["sha256:bbb"].label == 0, "non-matching precursor set must be labeled negative"
    assert by_id["sha256:ccc"].label == 0, "an unlabeled target must default every candidate to 0"

    # -- group_sizes requires target-sorted input --
    sorted_rows = sorted(labeled, key=lambda r: r.target_id)
    sizes = group_sizes(sorted_rows)
    assert sum(sizes) == len(sorted_rows)
    assert len(sizes) == len({r.target_id for r in labeled})

    # -- feature_missing -> NaN, not silently 0 --
    import math

    assert math.isnan(by_id["sha256:aaa"].features[-1]), "a missing feature must become NaN, not 0"

    print("self-test: deterministic split, labeling, and grouping logic OK", flush=True)

    # -- evaluate() ranking tie-break is deterministic, independent of lightgbm --
    class _ConstantRanker:
        def predict(self, X):
            return [0.0] * len(X)

    tied_a = [
        LabeledRow("t", "sha256:bbb", [1.0], 1, "test"),
        LabeledRow("t", "sha256:aaa", [1.0], 0, "test"),
    ]
    tied_b = list(reversed(tied_a))
    report_a = evaluate(_ConstantRanker(), tied_a, "test")
    report_b = evaluate(_ConstantRanker(), tied_b, "test")
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

    # Force everything into one split for this tiny fixture so there's
    # enough data in one bucket to fit a model at all -- overriding
    # `split_for_target`'s real hash-bucket assignment is deliberate here,
    # this is a code-path smoke test, not a leakage-prevention test (that's
    # covered above by the assertions on `split_for_target` itself).
    for r in labeled:
        r.split = "train"
    ranker = train_ranker(labeled)
    for r in labeled:
        r.split = "test"
    report = evaluate(ranker, labeled, "test")
    assert report["split"] == "test"
    assert report["target_count"] == 2
    assert report["scored_targets"] == 1, "only target_a has a positive candidate"
    assert report["targets_with_zero_positive_in_pool"] == 1, "target_b has none"
    assert report["top1_hit_rate"] in (0.0, 1.0)
    print(f"self-test: end-to-end train+evaluate OK, report={report}", flush=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--pool", help="JSONL candidate pool (renkin::pool_export::write_jsonl output)")
    parser.add_argument("--manifest", help="Sidecar PoolManifest JSON (for feature_schema_version validation)")
    parser.add_argument("--labels", help="JSONL ground-truth labels: {target_id, correct_precursor_smiles}")
    parser.add_argument("--model-out", help="Path to save the trained LightGBM booster (text format)")
    parser.add_argument("--eval-out", help="Path to save the evaluation report JSON")
    parser.add_argument(
        "--self-test", action="store_true",
        help="Run the deterministic-logic self-check (and, if lightgbm is installed, "
             "a tiny end-to-end smoke pass) against an embedded synthetic fixture, "
             "then exit. Does not read --pool/--labels.",
    )
    args = parser.parse_args()

    if args.self_test:
        self_test()
        return

    if not (args.pool and args.manifest and args.labels):
        parser.error("--pool, --manifest, and --labels are all required unless --self-test is given")

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
    labels = load_labels(args.labels)
    labeled = label_and_split_rows(pool_rows, labels)

    train_rows = [r for r in labeled if r.split == "train"]
    if not train_rows:
        print("ERROR: no rows in the train split.", file=sys.stderr)
        sys.exit(1)

    ranker = train_ranker(train_rows)

    if args.model_out:
        Path(args.model_out).parent.mkdir(parents=True, exist_ok=True)
        ranker.booster_.save_model(args.model_out)
        print(f"Saved model to {args.model_out}", flush=True)

    reports = {split: evaluate(ranker, labeled, split) for split in ("train", "val", "test")}
    result = {
        "feature_schema_version": from_export_schema,
        "proposal_mode": manifest.get("proposal_mode"),
        "rules_content_hash": manifest.get("rules_content_hash"),
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
