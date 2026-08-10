#!/usr/bin/env python3
"""
Issue #101 Task 35 / runtime-integration prerequisite: export the frozen
TRAIN-only template-frequency table (see `fit_template_frequency`,
frozen SHA-256 recorded in `freeze_manifest.json`) as a standalone JSON
artifact, so a runtime (Rust) consumer can impute `max`/`mean_template_log_
frequency` (feature indices 16/17) identically to how
`impute_frequency_features` does it offline -- these two features are
*always* NaN in exported candidate-pool rows and only ever get a real value
via this post-hoc, TRAIN-frozen table (see `impute_frequency_features`'s
own doc). Without this artifact, a runtime scorer would feed the model a
feature distribution it was never trained on for indices 16/17.

Refits from the original TRAIN pool (deterministic, order-independent --
see `fit_template_frequency`'s own accumulation logic) and asserts the
result's SHA-256 matches the frozen value, exactly like
`phase3e_evaluate_formal_test.py`'s own integrity check.

Usage:
    python3 scripts/phase3e_export_frequency_table.py
"""
import importlib.util
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "data" / "phase3e_reranker_training"
FROZEN = json.loads((OUT / "freeze_manifest.json").read_text())
FROZEN_SHA = FROZEN["feature_schema"]["template_frequency_table_sha256"]

spec = importlib.util.spec_from_file_location("train_reranker", ROOT / "scripts" / "train_reranker.py")
tr = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tr)


def main() -> None:
    train_pool_dir = ROOT / "data" / "phase3d_full_pool"
    train_pool_rows = tr.load_jsonl(train_pool_dir / "pool_train_full.jsonl")
    train_group_records = tr.load_jsonl(train_pool_dir / "groups_train_full.jsonl")
    train_labels = tr.load_labels(ROOT / "data" / "reranker_labels_uspto50k_train.jsonl")
    tr.validate_pool_rows(train_pool_rows, train_group_records)

    split_map = {r["target_id"]: "train" for r in train_group_records}
    tr.configure_split_override(split_map)

    train_labeled, unlabeled = tr.label_and_split_rows(
        train_pool_rows, train_labels, train_group_records, allow_unlabeled=False
    )
    assert unlabeled == 0
    train_rows = [r for r in train_labeled if r.split == "train"]
    freq_table = tr.fit_template_frequency(train_rows)
    sha = tr.template_frequency_table_sha256(freq_table)
    assert sha == FROZEN_SHA, f"refit table does not match frozen SHA: {sha} != {FROZEN_SHA}"

    out_path = OUT / "frequency_table.json"
    out_path.write_text(json.dumps({
        "_purpose": (
            "TRAIN-frozen template_id -> log(count+1) frequency table used to "
            "impute FEATURE_NAMES_V1 indices 16/17 (max/mean_template_log_frequency) "
            "-- see scripts/train_reranker.py::impute_frequency_features. Required "
            "second artifact alongside model.txt for a runtime scorer to reproduce "
            "the offline feature distribution exactly."
        ),
        "sha256": sha,
        "entries": len(freq_table),
        "table": freq_table,
    }, indent=2, sort_keys=True))
    print(json.dumps({"sha256": sha, "entries": len(freq_table), "out_path": str(out_path)}, indent=2))


if __name__ == "__main__":
    main()
