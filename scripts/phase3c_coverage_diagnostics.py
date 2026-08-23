"""Issue #101 Phase 3C-2/3C-3: real-loader validation + proposal-coverage/
rank diagnostics for a candidate pool, run through the actual
train_reranker.py machinery (not a reimplementation).

Split assignment always goes through --split-manifest (train_reranker.py's
explicit override), filtered down to exactly the target_ids present in
this run's --groups (load_split_manifest's own hard validation then
requires exact coverage -- no silent fallback to hash bucketing for
anything this run touches).

"Best positive rank" reuses train_reranker.py's own compute_arm_group_metrics
under the `original_rank` baseline arm's score_fn (ascending
best_upstream_rank -- the order propose_one_step's rules actually fired
in), so this is the same rank a human reading `original_rank`'s report
would see, not a bespoke recomputation.

Usage:
    python3 scripts/phase3c_coverage_diagnostics.py \
        --pool data/phase3c_500_target_feasibility/pool_train_500.jsonl \
        --groups data/phase3c_500_target_feasibility/groups_train_500.jsonl \
        --manifest data/phase3c_500_target_feasibility/manifest_train_500.json \
        --labels data/reranker_labels_uspto50k_train.jsonl \
        --split-manifest data/reranker_split_manifest.jsonl \
        --split train \
        --output data/phase3c_500_target_feasibility/coverage_train_500.json
"""

from __future__ import annotations

import argparse
import json
import sys

sys.path.insert(0, "scripts")
import train_reranker as tr  # noqa: E402


def original_rank_score_fn(rows):
    return [-float(r.best_upstream_rank) for r in rows]


def rank_bucket(rank: int) -> str:
    if rank == 1:
        return "rank_1"
    if rank <= 10:
        return "rank_2_10"
    if rank <= 50:
        return "rank_11_50"
    return "rank_over_50"


def percentile(sorted_vals: list, p: float):
    if not sorted_vals:
        return None
    idx = min(round((len(sorted_vals) - 1) * p), len(sorted_vals) - 1)
    return sorted_vals[idx]


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--pool", required=True)
    parser.add_argument("--groups", required=True)
    parser.add_argument("--manifest", required=True)
    parser.add_argument("--labels", required=True)
    parser.add_argument("--split-manifest", required=True)
    parser.add_argument("--split", required=True, choices=["train", "val", "test"])
    parser.add_argument("--output", required=True)
    args = parser.parse_args(argv)

    manifest = json.load(open(args.manifest, "r", encoding="utf-8"))
    tr.validate_manifest(manifest, args.pool, args.groups)

    pool_rows = tr.load_jsonl(args.pool)
    group_records = tr.load_jsonl(args.groups)
    tr.validate_pool_rows(pool_rows, group_records)

    known_target_ids = {r["target_id"] for r in group_records}
    full_manifest_rows = tr.load_jsonl(args.split_manifest)
    filtered = [r for r in full_manifest_rows if r["target_id"] in known_target_ids]
    filtered_path = args.output + ".split_manifest_subset.jsonl"
    with open(filtered_path, "w", encoding="utf-8") as f:
        for row in filtered:
            f.write(json.dumps(row, sort_keys=True) + "\n")
    assignments = tr.load_split_manifest(filtered_path, known_target_ids)

    # Confirm no target from this run is assigned to a DIFFERENT split than
    # requested (catches e.g. a test target accidentally leaking into a
    # train/val groups file -- Phase 3C-2's explicit requirement).
    wrong_split = {tid: s for tid, s in assignments.items() if s != args.split}
    tr.configure_split_override(assignments)

    labels = tr.load_labels(args.labels)
    labeled, unlabeled_count = tr.label_and_split_rows(pool_rows, labels, group_records)

    per_group_metrics = tr.compute_arm_group_metrics(original_rank_score_fn, labeled, args.split)
    coverage = tr.summarize_coverage(labeled, group_records, labels, args.split)

    n_groups_total = coverage.group_count
    n_with_positive = sum(1 for m in per_group_metrics.values() if m["has_positive"])
    n_zero_positive = n_groups_total - n_with_positive

    ranks = sorted(m["best_positive_rank"] for m in per_group_metrics.values() if m["has_positive"])
    buckets = {"rank_1": 0, "rank_2_10": 0, "rank_11_50": 0, "rank_over_50": 0}
    for r in ranks:
        buckets[rank_bucket(r)] += 1

    positive_counts_per_group = []
    for group_id, rows in _group_by(labeled).items():
        if tr.split_for_target(rows[0].target_id) != args.split:
            continue
        positive_counts_per_group.append(sum(1 for r in rows if r.label == 1))

    result = {
        "split": args.split,
        "pool_path": args.pool,
        "groups_path": args.groups,
        "labels_path": args.labels,
        "n_groups_total": n_groups_total,
        "n_pool_rows": len(pool_rows),
        "n_labeled_rows": len(labeled),
        "n_unlabeled_groups": unlabeled_count,
        "n_groups_wrong_split_assignment": len(wrong_split),
        "wrong_split_assignment_sample": dict(list(wrong_split.items())[:5]),
        "n_groups_with_positive": n_with_positive,
        "n_groups_zero_positive": n_zero_positive,
        "positive_coverage_rate": n_with_positive / n_groups_total if n_groups_total else None,
        "positive_count_per_group_mean": (
            sum(positive_counts_per_group) / len(positive_counts_per_group)
            if positive_counts_per_group
            else None
        ),
        "best_positive_rank_p50": percentile(ranks, 0.50),
        "best_positive_rank_p90": percentile(ranks, 0.90),
        "best_positive_rank_p95": percentile(ranks, 0.95),
        "best_positive_rank_max": ranks[-1] if ranks else None,
        "best_positive_rank_buckets": buckets,
    }
    with open(args.output, "w", encoding="utf-8") as f:
        json.dump(result, f, indent=2, sort_keys=True)
        f.write("\n")
    print(json.dumps(result, indent=2, sort_keys=True), file=sys.stderr)
    return 0


def _group_by(labeled_rows):
    out: dict = {}
    for r in labeled_rows:
        out.setdefault(r.group_id, []).append(r)
    return out


if __name__ == "__main__":
    raise SystemExit(main())
