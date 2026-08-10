"""Issue #101 Phase 3A Round 2, Sections B/C/D: reranker TRAIN/VALIDATION
labels, sourced from the USPTO-50k ORIGINAL train/val splits (never the
existing 4,903-target competitive benchmark corpus -- see
`generate_benchmark_quarantine_manifest.py` and
`data/phase3a_reranker_ground_truth_audit/round2_split_hygiene.md`).

Training a reranker on original-train reactions is NOT leakage -- it's
ordinary supervised learning, and `templates_extracted_500.smi` is already
extracted from this same train split (confirmed in the Phase 3A Round 1
audit), so this doesn't add any new information the templates didn't
already have. What must never happen is the reranker seeing anything whose
product matches a quarantined benchmark target.

Decontamination (Section C): every train/val product is canonicalized and
checked against the benchmark quarantine's target identities
(`generate_benchmark_quarantine_manifest.py`'s output). Any raw reaction
whose product matches is dropped entirely from train/val, with the exact
count reported.

Cross-split hygiene (also Section C): after per-split decontamination, no
`target_id` may appear in both train and val (USPTO-50k splits by reaction,
not by product, so the same product can legitimately have recorded
reactions in both). Deterministic rule, fixed here: **train wins** -- any
val target_id that also appears in train is dropped from val entirely (not
merged). This is a design choice, not the only valid one; recorded so a
future reviewer doesn't have to reverse-engineer it from the diff.

target_id / group_id (Section D): target_id **is** the RENKIN-canonical
product SMILES itself (same product -> same target_id, no matter how many
raw reactions produced it or which split they're read from) -- not an
opaque identifier of our own choosing. This is a hard constraint, not a
design preference: `propose_one_step` sets `CandidatePool.target_id` to
`to_canonical(&target_mol)` unconditionally (verified directly against the
real function, see round2_split_hygiene.md), so a labels file using any
other target_id convention would fail `train_reranker.py`'s group-index
cross-check on every single row. group_id is per raw reaction row
(`uspto50k_{split}#L{line}`) -- deliberately NOT collapsed to one group per
target_id the way the formal test corpus is: for train/val, multiple
literature routes to the same product are meant to be separate
training/validation examples (denser supervision), whereas the formal test
set collapses them so eval metrics don't double-count one target's
coverage.

Usage:
    cargo build --release --bin renkin-canonicalize
    python3 scripts/generate_train_val_labels.py \
        --raw-train-split data/uspto50k_raw_train_split.jsonl \
        --raw-val-split data/uspto50k_raw_val_split.jsonl \
        --quarantine-identities data/phase3a_reranker_ground_truth_audit/benchmark_quarantine_target_identities.txt \
        --canonicalize-bin target/release/renkin-canonicalize \
        --train-output data/reranker_labels_uspto50k_train.jsonl \
        --val-output data/reranker_labels_uspto50k_val.jsonl \
        --split-manifest-output data/reranker_split_manifest.jsonl \
        --summary-output data/reranker_labels_uspto50k_train_val.summary.json
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import defaultdict

from reranker_label_common import canonicalize_batch, sha256_of


def target_id_for(canonical_product: str) -> str:
    """target_id **is** the canonical product SMILES -- see the module
    docstring's Section D note for why this isn't a free choice. Kept as a
    named function (rather than using `canonical_product` inline
    everywhere) so the identity mapping has one place to change if that
    constraint is ever relaxed.
    """
    return canonical_product


def process_split(
    split_name: str, raw_rows: list, canonicalize_bin: str, quarantine_identities: set
) -> dict:
    n_raw = len(raw_rows)
    products_canon = canonicalize_batch([r["product"] for r in raw_rows], canonicalize_bin)

    frag_row_index: list[int] = []
    frags_mapped: list[str] = []
    for i, r in enumerate(raw_rows):
        for frag in r["reactants"].split("."):
            frag_row_index.append(i)
            frags_mapped.append(frag)
    frags_canon = canonicalize_batch(frags_mapped, canonicalize_bin)

    row_frags: dict[int, list] = defaultdict(list)
    for row_idx, canon in zip(frag_row_index, frags_canon):
        row_frags[row_idx].append(canon)

    n_product_parse_fail = sum(1 for c in products_canon if c is None)
    n_reactant_parse_fail = sum(1 for c in frags_canon if c is None)
    unique_products = {c for c in products_canon if c is not None}
    products_overlapping_benchmark = unique_products & quarantine_identities

    n_rows_removed_benchmark_overlap = 0
    n_rows_reactant_parse_fail = 0
    retained_rows = []  # (line_no, product_canon, precursor_set)
    for i, product_canon in enumerate(products_canon):
        line_no = i + 1
        if product_canon is None:
            continue
        if product_canon in quarantine_identities:
            n_rows_removed_benchmark_overlap += 1
            continue
        frags = row_frags.get(i, [])
        if any(c is None for c in frags):
            n_rows_reactant_parse_fail += 1
            continue
        retained_rows.append((line_no, product_canon, tuple(sorted(frags))))

    return {
        "split": split_name,
        "n_raw": n_raw,
        "n_product_parse_fail": n_product_parse_fail,
        "n_reactant_parse_fail": n_reactant_parse_fail,
        "n_rows_reactant_parse_fail": n_rows_reactant_parse_fail,
        "n_unique_products": len(unique_products),
        "n_products_overlapping_benchmark": len(products_overlapping_benchmark),
        "n_rows_removed_benchmark_overlap": n_rows_removed_benchmark_overlap,
        "retained_rows": retained_rows,
    }


def rows_to_labels(split_name: str, retained_rows: list) -> list[dict]:
    out = []
    for line_no, product_canon, precursor_set in retained_rows:
        out.append(
            {
                "schema_version": 1,
                "group_id": f"uspto50k_{split_name}#L{line_no}",
                "target_id": target_id_for(product_canon),
                "correct_precursor_sets": [list(precursor_set)],
            }
        )
    return out


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--raw-train-split", default="data/uspto50k_raw_train_split.jsonl")
    parser.add_argument("--raw-val-split", default="data/uspto50k_raw_val_split.jsonl")
    parser.add_argument(
        "--quarantine-identities",
        default="data/phase3a_reranker_ground_truth_audit/benchmark_quarantine_target_identities.txt",
    )
    parser.add_argument("--canonicalize-bin", default="target/release/renkin-canonicalize")
    parser.add_argument("--train-output", default="data/reranker_labels_uspto50k_train.jsonl")
    parser.add_argument("--val-output", default="data/reranker_labels_uspto50k_val.jsonl")
    parser.add_argument(
        "--train-targets-output",
        default="data/reranker_targets_uspto50k_train.jsonl",
        help="Deduplicated target list (target_id IS the canonical SMILES, but this is the "
             "flat list a pool-generation driver iterates over).",
    )
    parser.add_argument("--val-targets-output", default="data/reranker_targets_uspto50k_val.jsonl")
    parser.add_argument("--split-manifest-output", default="data/reranker_split_manifest.jsonl")
    parser.add_argument(
        "--summary-output", default="data/reranker_labels_uspto50k_train_val.summary.json"
    )
    parser.add_argument(
        "--test-identities",
        default="data/phase3a_reranker_ground_truth_audit/benchmark_quarantine_target_identities.txt",
        help="Quarantined test target identities, included in the split manifest as split=test.",
    )
    args = parser.parse_args(argv)

    with open(args.quarantine_identities, "r", encoding="utf-8") as f:
        quarantine_identities = {line.strip() for line in f if line.strip()}

    with open(args.raw_train_split, "r", encoding="utf-8") as f:
        raw_train = [json.loads(line) for line in f if line.strip()]
    with open(args.raw_val_split, "r", encoding="utf-8") as f:
        raw_val = [json.loads(line) for line in f if line.strip()]

    train_result = process_split("train", raw_train, args.canonicalize_bin, quarantine_identities)
    val_result = process_split("val", raw_val, args.canonicalize_bin, quarantine_identities)

    train_target_ids = {target_id_for(p) for _, p, _ in train_result["retained_rows"]}
    val_rows_before_dedup = val_result["retained_rows"]
    val_rows = [
        row for row in val_rows_before_dedup if target_id_for(row[1]) not in train_target_ids
    ]
    n_val_rows_dropped_train_overlap = len(val_rows_before_dedup) - len(val_rows)
    val_target_ids_dropped = {
        target_id_for(p) for _, p, _ in val_rows_before_dedup if target_id_for(p) in train_target_ids
    }

    train_labels = rows_to_labels("train", train_result["retained_rows"])
    val_labels = rows_to_labels("val", val_rows)

    with open(args.train_output, "w", encoding="utf-8") as f:
        for row in train_labels:
            f.write(json.dumps(row, sort_keys=True) + "\n")
    with open(args.val_output, "w", encoding="utf-8") as f:
        for row in val_labels:
            f.write(json.dumps(row, sort_keys=True) + "\n")

    def write_targets(path: str, retained_rows: list) -> None:
        seen = {}
        for _, product_canon, _ in retained_rows:
            seen.setdefault(target_id_for(product_canon), product_canon)
        with open(path, "w", encoding="utf-8") as f:
            for tid in sorted(seen):
                f.write(json.dumps({"target_id": tid, "canonical_smiles": seen[tid]}, sort_keys=True) + "\n")

    write_targets(args.train_targets_output, train_result["retained_rows"])
    write_targets(args.val_targets_output, val_rows)

    # Hard invariants (Phase 3B go/no-go gate items) -- assert, don't just report.
    val_target_ids_final = {r["target_id"] for r in val_labels}
    train_target_ids_final = {r["target_id"] for r in train_labels}
    assert not (train_target_ids_final & val_target_ids_final), (
        "train/val target_id overlap survived dedup -- this must be empty"
    )
    with open(args.test_identities, "r", encoding="utf-8") as f:
        test_identities = {line.strip() for line in f if line.strip()}
    train_products = {p for _, p, _ in train_result["retained_rows"]}
    val_products_final = {p for _, p, _ in val_rows}
    assert not (train_products & test_identities), "train vs benchmark overlap survived decontamination"
    assert not (val_products_final & test_identities), "val vs benchmark overlap survived decontamination"

    split_manifest_rows = []
    for tid in sorted(train_target_ids_final):
        split_manifest_rows.append({"target_id": tid, "split": "train"})
    for tid in sorted(val_target_ids_final):
        split_manifest_rows.append({"target_id": tid, "split": "val"})
    for tid in sorted(test_identities):
        split_manifest_rows.append({"target_id": tid, "split": "test"})
    with open(args.split_manifest_output, "w", encoding="utf-8") as f:
        for row in split_manifest_rows:
            f.write(json.dumps(row, sort_keys=True) + "\n")

    summary = {
        "quarantine_identities_path": args.quarantine_identities,
        "quarantine_identities_sha256": sha256_of(args.quarantine_identities),
        "n_quarantine_identities": len(quarantine_identities),
        "train": {
            k: v for k, v in train_result.items() if k != "retained_rows"
        }
        | {"n_labeled_groups": len(train_labels), "n_distinct_target_ids": len(train_target_ids_final)},
        "val": {
            k: v for k, v in val_result.items() if k != "retained_rows"
        }
        | {
            "n_rows_before_train_overlap_dedup": len(val_rows_before_dedup),
            "n_rows_dropped_train_overlap": n_val_rows_dropped_train_overlap,
            "n_target_ids_dropped_train_overlap": len(val_target_ids_dropped),
            "n_labeled_groups": len(val_labels),
            "n_distinct_target_ids": len(val_target_ids_final),
        },
        "cross_split_dedup_rule": "train wins: any val target_id also present in train is dropped from val entirely",
        "train_vs_benchmark_overlap_after_decontamination": len(train_products & test_identities),
        "val_vs_benchmark_overlap_after_decontamination": len(val_products_final & test_identities),
        "train_vs_val_target_id_overlap_after_dedup": len(train_target_ids_final & val_target_ids_final),
        "split_manifest_path": args.split_manifest_output,
        "split_manifest_sha256": sha256_of(args.split_manifest_output),
        "train_output": args.train_output,
        "val_output": args.val_output,
        "train_targets_output": args.train_targets_output,
        "val_targets_output": args.val_targets_output,
    }
    with open(args.summary_output, "w", encoding="utf-8") as f:
        json.dump(summary, f, indent=2, sort_keys=True)
        f.write("\n")

    print(json.dumps(summary, indent=2, sort_keys=True), file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
