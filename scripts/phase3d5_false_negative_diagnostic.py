"""Phase 3D.5 Step 4: false-negative diagnostic on currently zero-positive
groups (diagnostic only -- does not change any label/pool file or the real
labeling policy in train_reranker.py).

Baseline positive/negative labeling comes from the REAL
`train_reranker.py::label_and_split_rows` (exact-string-multiset match,
unmodified). For groups the real loader marks zero-positive, this script
asks: would pushing both the candidate's precursor_smiles and the label's
correct_precursor_sets through one more `renkin-canonicalize` pass (i.e.
normalizing both sides onto the SAME identity, since Phase 3D.5 Step 2/3
confirmed a ~0.2-0.28% one-shot-drift-then-stable non-idempotence) make a
positive match appear that the raw string comparison currently misses?

Usage:
    python3 scripts/phase3d5_false_negative_diagnostic.py --split train
    python3 scripts/phase3d5_false_negative_diagnostic.py --split val
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))
import train_reranker as tr  # noqa: E402

AUDIT_DIR = ROOT / "data" / "phase3d5_canonical_identity_audit"


def load_canon_map(unique_path: Path, canon_path: Path) -> dict:
    unique = unique_path.read_text(encoding="utf-8").splitlines()
    canon = canon_path.read_text(encoding="utf-8").splitlines()
    assert len(unique) == len(canon)
    return {u: c for u, c in zip(unique, canon) if c != "ERR"}


def normalize_tuple(precursors, canon_map):
    return tuple(sorted(canon_map.get(p, p) for p in precursors))


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--split", required=True, choices=["train", "val"])
    args = parser.parse_args(argv)
    split = args.split

    pool_path = ROOT / "data" / "phase3d_full_pool" / f"pool_{split}_full.jsonl"
    groups_path = ROOT / "data" / "phase3d_full_pool" / f"groups_{split}_full.jsonl"
    manifest_path = ROOT / "data" / "phase3d_full_pool" / f"manifest_{split}_full.json"
    labels_path = ROOT / "data" / f"reranker_labels_uspto50k_{split}.jsonl"
    split_manifest_path = ROOT / "data" / "reranker_split_manifest.jsonl"

    manifest = json.load(open(manifest_path, "r", encoding="utf-8"))
    tr.validate_manifest(manifest, str(pool_path), str(groups_path))
    pool_rows = tr.load_jsonl(str(pool_path))
    group_records = tr.load_jsonl(str(groups_path))
    tr.validate_pool_rows(pool_rows, group_records)

    known_target_ids = {r["target_id"] for r in group_records}
    full_manifest_rows = tr.load_jsonl(str(split_manifest_path))
    filtered = [r for r in full_manifest_rows if r["target_id"] in known_target_ids]
    filtered_path = AUDIT_DIR / f"fn_diag_{split}.split_manifest_subset.jsonl"
    with open(filtered_path, "w", encoding="utf-8") as f:
        for row in filtered:
            f.write(json.dumps(row, sort_keys=True) + "\n")
    assignments = tr.load_split_manifest(str(filtered_path), known_target_ids)
    tr.configure_split_override(assignments)

    labels = tr.load_labels(str(labels_path))
    labeled, unlabeled_count = tr.label_and_split_rows(pool_rows, labels, group_records)

    # LabeledRow drops precursor_smiles (it's not a feature); recover it from
    # the raw pool rows by (group_id, candidate_id), which is unique.
    precursor_lookup = {
        (row["group_id"], row["candidate_id"]): row["precursor_smiles"] for row in pool_rows
    }

    # Baseline: real labeling, unmodified.
    by_group = {}
    for row in labeled:
        if tr.split_for_target(row.target_id) != split:
            continue
        by_group.setdefault(row.group_id, []).append(row)

    zero_positive_groups = [
        gid for gid, rows in by_group.items() if not any(r.label == 1 for r in rows)
    ]
    n_groups_total = len(by_group)
    n_zero_positive_before = len(zero_positive_groups)
    n_with_positive_before = n_groups_total - n_zero_positive_before

    candidate_canon = load_canon_map(
        AUDIT_DIR / f"candidate_precursors_{split}_unique.txt",
        AUDIT_DIR / f"candidate_precursors_{split}_canon.txt",
    )
    label_canon = load_canon_map(
        AUDIT_DIR / f"label_precursors_{split}_unique.txt",
        AUDIT_DIR / f"label_precursors_{split}_canon.txt",
    )

    flipped = []
    for gid in zero_positive_groups:
        rows = by_group[gid]
        label_entry = labels.get(gid)
        if label_entry is None:
            continue
        normalized_label_sets = {
            normalize_tuple(pset, label_canon) for pset in label_entry.correct_precursor_sets
        }
        for row in rows:
            precursor_smiles = precursor_lookup[(row.group_id, row.candidate_id)]
            normalized_candidate = normalize_tuple(precursor_smiles, candidate_canon)
            if normalized_candidate in normalized_label_sets:
                flipped.append(
                    {
                        "group_id": gid,
                        "candidate_id": row.candidate_id,
                        "best_upstream_rank": row.best_upstream_rank,
                        "raw_precursor_smiles": precursor_smiles,
                        "raw_correct_precursor_sets": [list(s) for s in label_entry.correct_precursor_sets],
                    }
                )
                break  # one flip is enough to make this group non-zero-positive

    n_flipped = len(flipped)
    n_zero_positive_after = n_zero_positive_before - n_flipped
    n_with_positive_after = n_with_positive_before + n_flipped

    result = {
        "split": split,
        "n_groups_total": n_groups_total,
        "n_unlabeled_groups": unlabeled_count,
        "coverage_before": {
            "n_with_positive": n_with_positive_before,
            "n_zero_positive": n_zero_positive_before,
            "rate": n_with_positive_before / n_groups_total if n_groups_total else None,
        },
        "coverage_after_normalized_diagnostic": {
            "n_with_positive": n_with_positive_after,
            "n_zero_positive": n_zero_positive_after,
            "rate": n_with_positive_after / n_groups_total if n_groups_total else None,
        },
        "n_groups_flipped_zero_to_positive": n_flipped,
        "flipped_groups_sample": flipped[:20],
        "flipped_best_upstream_rank_distribution": sorted(f["best_upstream_rank"] for f in flipped),
    }

    out_path = AUDIT_DIR / f"false_negative_diagnostic_{split}.json"
    with open(out_path, "w", encoding="utf-8") as f:
        json.dump(result, f, indent=2, sort_keys=False)

    print(json.dumps({k: v for k, v in result.items() if k not in ("flipped_groups_sample",)}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
