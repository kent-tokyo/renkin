#!/usr/bin/env python3
"""
Issue #101 Phase 3E step 1: mechanically concatenate the separate TRAIN/VAL
full pools (Phase 3D) into the single combined pool/groups/labels/manifest
`scripts/train_reranker.py` expects (it takes one `--pool`/`--groups`/
`--labels`/`--split-manifest` set spanning all splits, with per-target split
resolved via `--split-manifest`, not two separate invocations).

Pure concatenation, in TRAIN-then-VAL order (recorded below and in the
combined manifest's `combined_from` field, since the recomputed SHA-256s are
order-dependent) -- no row is added, removed, or modified. Every source file
already ends in a trailing newline (verified via `xxd` before writing this
script), so byte concatenation is a valid JSONL join.

Safety asserts (not re-validated downstream by train_reranker.py, which
trusts --groups/--pool once loaded):
  - no group_id appears in both the train and val group-index files
    (byte-disjoint prefixes -- "uspto50k_train#L*" vs "uspto50k_val#L*" --
    by construction, but asserted rather than assumed);
  - no target_id appears in both split-manifest subsets (Phase 3D.5 Step 6
    measured this as 0 via a wider check; re-asserted here on the exact
    files being merged).

Usage:
    python3 scripts/phase3e_build_combined_pool.py
"""
import hashlib
import json
from pathlib import Path

SRC = Path("data/phase3d_full_pool")
OUT = Path("data/phase3e_reranker_training")

TRAIN_POOL, VAL_POOL = SRC / "pool_train_full.jsonl", SRC / "pool_val_full.jsonl"
TRAIN_GROUPS, VAL_GROUPS = SRC / "groups_train_full.jsonl", SRC / "groups_val_full.jsonl"
TRAIN_LABELS = Path("data/reranker_labels_uspto50k_train.jsonl")
VAL_LABELS = Path("data/reranker_labels_uspto50k_val.jsonl")
TRAIN_SPLIT_SUBSET = SRC / "coverage_train_full.json.split_manifest_subset.jsonl"
VAL_SPLIT_SUBSET = SRC / "coverage_val_full.json.split_manifest_subset.jsonl"
TRAIN_MANIFEST, VAL_MANIFEST = SRC / "manifest_train_full.json", SRC / "manifest_val_full.json"

OUT_POOL = OUT / "pool_combined.jsonl"
OUT_GROUPS = OUT / "groups_combined.jsonl"
OUT_LABELS = OUT / "labels_combined.jsonl"
OUT_SPLIT_MANIFEST = OUT / "split_manifest_combined.jsonl"
OUT_MANIFEST = OUT / "manifest_combined.json"

# Manifest fields that must be byte-identical between the train/val source
# manifests before being copied through unchanged into the combined one --
# a mismatch here means the two pools were not exported under the same
# schema/config and must NOT be silently merged.
INVARIANT_FIELDS = [
    "manifest_schema_version", "feature_schema_version", "feature_names",
    "feature_schema_hash", "proposal_mode", "rules_content_hash",
    "rules_count", "stock_identity", "stock_compound_count", "stock_content_sha256",
]


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return f"sha256:{h.hexdigest()}"


def concat(out_path: Path, *sources: Path) -> None:
    with open(out_path, "wb") as out:
        for src in sources:
            with open(src, "rb") as f:
                out.write(f.read())


def load_jsonl(path: Path) -> list:
    with open(path, "r", encoding="utf-8") as f:
        return [json.loads(line) for line in f if line.strip()]


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)

    train_groups = load_jsonl(TRAIN_GROUPS)
    val_groups = load_jsonl(VAL_GROUPS)
    train_group_ids = {r["group_id"] for r in train_groups}
    val_group_ids = {r["group_id"] for r in val_groups}
    collision = train_group_ids & val_group_ids
    assert not collision, f"{len(collision)} group_id(s) in both train and val: {sorted(collision)[:5]}"

    train_split = load_jsonl(TRAIN_SPLIT_SUBSET)
    val_split = load_jsonl(VAL_SPLIT_SUBSET)
    train_target_ids = {r["target_id"] for r in train_split}
    val_target_ids = {r["target_id"] for r in val_split}
    target_collision = train_target_ids & val_target_ids
    assert not target_collision, (
        f"{len(target_collision)} target_id(s) in both train and val split subsets: "
        f"{sorted(target_collision)[:5]}"
    )
    assert all(r["split"] == "train" for r in train_split)
    assert all(r["split"] == "val" for r in val_split)

    concat(OUT_POOL, TRAIN_POOL, VAL_POOL)
    concat(OUT_GROUPS, TRAIN_GROUPS, VAL_GROUPS)
    concat(OUT_LABELS, TRAIN_LABELS, VAL_LABELS)
    concat(OUT_SPLIT_MANIFEST, TRAIN_SPLIT_SUBSET, VAL_SPLIT_SUBSET)

    train_manifest = json.loads(TRAIN_MANIFEST.read_text())
    val_manifest = json.loads(VAL_MANIFEST.read_text())
    for field in INVARIANT_FIELDS:
        assert train_manifest.get(field) == val_manifest.get(field), (
            f"manifest field {field!r} differs between train/val source manifests: "
            f"{train_manifest.get(field)!r} vs {val_manifest.get(field)!r}"
        )

    combined_manifest = {
        field: train_manifest[field] for field in INVARIANT_FIELDS if field in train_manifest
    }
    combined_manifest.update({
        "target_count": train_manifest["target_count"] + val_manifest["target_count"],
        "group_count": train_manifest["group_count"] + val_manifest["group_count"],
        "candidate_count": train_manifest["candidate_count"] + val_manifest["candidate_count"],
        "candidate_jsonl_sha256": sha256_file(OUT_POOL),
        "target_group_index_sha256": sha256_file(OUT_GROUPS),
        "combined_from": {
            "order": "train_then_val",
            "train_source_candidate_jsonl_sha256": train_manifest["candidate_jsonl_sha256"],
            "train_source_target_group_index_sha256": train_manifest["target_group_index_sha256"],
            "val_source_candidate_jsonl_sha256": val_manifest["candidate_jsonl_sha256"],
            "val_source_target_group_index_sha256": val_manifest["target_group_index_sha256"],
        },
    })
    OUT_MANIFEST.write_text(json.dumps(combined_manifest, indent=2))

    print(json.dumps({
        "pool_rows": sum(1 for _ in open(OUT_POOL)),
        "group_records": len(train_groups) + len(val_groups),
        "split_manifest_target_ids": len(train_target_ids) + len(val_target_ids),
        "candidate_jsonl_sha256": combined_manifest["candidate_jsonl_sha256"],
        "target_group_index_sha256": combined_manifest["target_group_index_sha256"],
        "out_dir": str(OUT),
    }, indent=2))


if __name__ == "__main__":
    main()
