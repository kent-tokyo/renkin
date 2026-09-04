#!/usr/bin/env python3
"""
Chemical Space Coverage Diagnosis (Phase A PoC): export per-target
baseline/reranker outcomes on the formal TEST corpus.

Reuses scripts/train_reranker.py's own functions (imported as a module,
never reimplemented) for pool/label loading and per-group scoring -- the
same code path scripts/phase3e_evaluate_formal_test.py used to produce the
committed data/phase3e_reranker_training/formal_test_result.json aggregate
report. This script's only new logic is flattening per-group metrics down
to one row per target_id (target_count == group_count == 4,903 here, so
this is a 1:1 relabeling, not an aggregation) and writing them out
individually instead of only as an aggregate.

Unlike phase3e_evaluate_formal_test.py, this does NOT refit the frozen
frequency table from the full TRAIN pool -- data/phase3e_reranker_training/
frequency_table.json's "table" field is the exact frozen table (verified
byte-identical to the frozen model's own SHA-256 in freeze_manifest.json;
see the run notes in data/chemical_space_coverage_diagnosis/findings.md),
so loading it directly skips ~40 minutes of unnecessary full-TRAIN
pool-gen. Nothing here retrains or changes the frozen model.

Usage:
    python3 scripts/chemical_space_coverage_export_test_labels.py
"""
import hashlib
import importlib.util
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PHASE3E = ROOT / "data" / "phase3e_reranker_training"
OUT_DIR = ROOT / "data" / "chemical_space_coverage_diagnosis"

FROZEN = json.loads((PHASE3E / "freeze_manifest.json").read_text())
FROZEN_FREQ_TABLE_SHA = FROZEN["feature_schema"]["template_frequency_table_sha256"]
FROZEN_MODEL_SHA = FROZEN["model_artifact"]["sha256"]

spec = importlib.util.spec_from_file_location("train_reranker", ROOT / "scripts" / "train_reranker.py")
tr = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tr)


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return f"sha256:{h.hexdigest()}"


def main() -> None:
    model_sha = sha256_file(PHASE3E / "model.txt")
    assert model_sha == FROZEN_MODEL_SHA, f"model.txt has changed since freeze: {model_sha} != {FROZEN_MODEL_SHA}"

    freq_doc = json.loads((PHASE3E / "frequency_table.json").read_text())
    freq_table = freq_doc["table"]
    freq_sha = tr.template_frequency_table_sha256(freq_table)
    assert freq_sha == FROZEN_FREQ_TABLE_SHA, (
        f"committed frequency_table.json does not match frozen table: {freq_sha} != {FROZEN_FREQ_TABLE_SHA}"
    )

    print("Loading formal TEST pool...", flush=True)
    test_group_records = tr.load_jsonl(PHASE3E / "groups_test_formal.jsonl")
    tr.configure_split_override({r["target_id"]: "test" for r in test_group_records})

    test_pool_rows = tr.load_jsonl(PHASE3E / "pool_test_formal.jsonl")
    test_labels = tr.load_labels(ROOT / "data" / "reranker_labels_uspto50k_test.jsonl")
    tr.validate_pool_rows(test_pool_rows, test_group_records)
    test_labeled, test_unlabeled = tr.label_and_split_rows(
        test_pool_rows, test_labels, test_group_records, allow_unlabeled=False
    )
    assert test_unlabeled == 0
    test_rows = [r for r in test_labeled if r.split == "test"]
    imputed_test_rows = tr.impute_frequency_features(test_rows, freq_table)

    import lightgbm as lgb
    booster = lgb.Booster(model_file=str(PHASE3E / "model.txt"))

    def full_model_score_fn(rows):
        return list(booster.predict([r.features for r in rows]))

    arms = tr.build_baseline_arms(freq_table)
    baseline_score_fn = next(a for a in arms if a["name"] == "original_rank")["score_fn"]

    baseline_report = tr.evaluate(baseline_score_fn, imputed_test_rows, test_group_records, test_labels, "test")
    print(
        f"baseline_report: groups_with_zero_positive_in_pool="
        f"{baseline_report['groups_with_zero_positive_in_pool']}, "
        f"scored_groups={baseline_report['scored_groups']}, "
        f"target_count={baseline_report['target_count']}",
        flush=True,
    )
    assert baseline_report["target_count"] == 4903
    assert baseline_report["groups_with_zero_positive_in_pool"] == 1618, (
        "regenerated pool's zero-positive count does not match the published 33.0% figure -- "
        f"got {baseline_report['groups_with_zero_positive_in_pool']}, expected 1618"
    )

    baseline_metrics = tr.compute_arm_group_metrics(baseline_score_fn, imputed_test_rows, "test")
    model_metrics = tr.compute_arm_group_metrics(full_model_score_fn, imputed_test_rows, "test")
    assert set(baseline_metrics) == set(model_metrics)

    # Denominator matches summarize_coverage()/baseline_report exactly: every
    # group_record present in --labels and in the "test" split, not just the
    # subset with >=1 scored candidate row. A group absent from
    # baseline_metrics/model_metrics has zero candidate rows at all (see
    # pool-gen's n_groups_zero_candidates/n_groups_target_id_mismatch) and so
    # is zero_positive by construction, with no rank to report.
    rows_out = []
    for record in test_group_records:
        gid = record["group_id"]
        if gid not in test_labels or tr.split_for_target(record["target_id"]) != "test":
            continue
        bm = baseline_metrics.get(gid)
        mm = model_metrics.get(gid)
        has_positive = bool(bm and bm["has_positive"])
        rows_out.append({
            "target_id": record["target_id"],
            "group_id": gid,
            "target_smiles": record["target_smiles"],
            "zero_positive": not has_positive,
            "baseline_top1_hit": (bm["best_positive_rank"] == 1) if has_positive else None,
            "reranker_top1_hit": (mm["best_positive_rank"] == 1) if has_positive else None,
        })
    assert len(rows_out) == baseline_report["target_count"] == 4903

    OUT_DIR.mkdir(parents=True, exist_ok=True)
    out_path = OUT_DIR / "test_target_labels.jsonl"
    with open(out_path, "w", encoding="utf-8") as f:
        for row in rows_out:
            f.write(json.dumps(row, sort_keys=True) + "\n")

    n_zero_positive = sum(1 for r in rows_out if r["zero_positive"])
    print(json.dumps({
        "out_path": str(out_path),
        "n_targets": len(rows_out),
        "n_zero_positive": n_zero_positive,
        "zero_positive_rate": n_zero_positive / len(rows_out),
    }, indent=2))


if __name__ == "__main__":
    main()
