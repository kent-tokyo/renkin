#!/usr/bin/env python3
"""
Issue #101 Task 35 Steps 6-8: apply the FROZEN model (see
data/phase3e_reranker_training/freeze_manifest.json) to the formal TEST
candidate pool exactly once, compared against the same primary baseline
(original_rank) on the identical pool, judged against the same pre-fixed
GATE_THRESHOLDS used for the VAL screening gate. Also builds the C/D/E error
taxonomy (Step 7) and an inference-determinism check (Step 8a -- re-running
training for Step 8b's reproducibility check is a separate, explicit re-run,
not part of this script).

Reuses scripts/train_reranker.py's own functions (imported as a module,
never reimplemented) for every metric/gate/labeling computation -- this
script's only new logic is: (a) refitting the frozen train-only frequency
table from the original TRAIN pool as an integrity/determinism check, (b)
loading the frozen booster instead of training a new one, (c) the C/D/E
taxonomy classification, which has no equivalent in train_reranker.py.

Nothing here retrains, changes hyperparameters, or changes gate thresholds.
"""
import hashlib
import importlib.util
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "data" / "phase3e_reranker_training"

FROZEN = json.loads((OUT / "freeze_manifest.json").read_text())
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
    model_sha = sha256_file(OUT / "model.txt")
    assert model_sha == FROZEN_MODEL_SHA, f"model.txt has changed since freeze: {model_sha} != {FROZEN_MODEL_SHA}"

    # --- Split override covering train (for freq-table refit) + test (for evaluation). ---
    split_map = {}
    for row in tr.load_jsonl(OUT / "split_manifest_combined.jsonl"):
        split_map[row["target_id"]] = row["split"]
    test_group_records = tr.load_jsonl(OUT / "groups_test_formal.jsonl")
    for r in test_group_records:
        split_map.setdefault(r["target_id"], "test")
    tr.configure_split_override(split_map)

    # --- Refit the TRAIN-frozen frequency table from the original TRAIN pool alone
    #     (fit_template_frequency's output is order-independent -- see its own
    #     dict-accumulation logic -- so this reproduces the frozen table exactly
    #     regardless of which combined file it's sourced from). Doubles as part
    #     of Step 8's determinism check.
    print("Loading TRAIN pool to refit the frozen frequency table...", flush=True)
    train_pool_dir = ROOT / "data" / "phase3d_full_pool"
    train_pool_rows = tr.load_jsonl(train_pool_dir / "pool_train_full.jsonl")
    train_group_records = tr.load_jsonl(train_pool_dir / "groups_train_full.jsonl")
    train_labels = tr.load_labels(ROOT / "data" / "reranker_labels_uspto50k_train.jsonl")
    tr.validate_pool_rows(train_pool_rows, train_group_records)
    train_labeled, train_unlabeled = tr.label_and_split_rows(
        train_pool_rows, train_labels, train_group_records, allow_unlabeled=False
    )
    assert train_unlabeled == 0
    train_rows = [r for r in train_labeled if r.split == "train"]
    freq_table = tr.fit_template_frequency(train_rows)
    freq_sha = tr.template_frequency_table_sha256(freq_table)
    assert freq_sha == FROZEN_FREQ_TABLE_SHA, (
        f"refit frequency table does not match frozen table: {freq_sha} != {FROZEN_FREQ_TABLE_SHA}"
    )
    print(f"Frequency table refit confirmed identical to frozen: {freq_sha}", flush=True)
    del train_pool_rows, train_labeled, train_rows  # free ~1.1GB before loading TEST

    # --- Load formal TEST pool. ---
    print("Loading formal TEST pool...", flush=True)
    test_pool_rows = tr.load_jsonl(OUT / "pool_test_formal.jsonl")
    test_labels = tr.load_labels(ROOT / "data" / "reranker_labels_uspto50k_test.jsonl")
    tr.validate_pool_rows(test_pool_rows, test_group_records)
    test_labeled, test_unlabeled = tr.label_and_split_rows(
        test_pool_rows, test_labels, test_group_records, allow_unlabeled=False
    )
    assert test_unlabeled == 0
    test_rows = [r for r in test_labeled if r.split == "test"]
    imputed_test_rows = tr.impute_frequency_features(test_rows, freq_table)

    # --- Load the FROZEN model (no retraining). ---
    import lightgbm as lgb
    booster = lgb.Booster(model_file=str(OUT / "model.txt"))

    def full_model_score_fn(rows):
        return list(booster.predict([r.features for r in rows]))

    arms = tr.build_baseline_arms(freq_table)
    baseline_arm = next(a for a in arms if a["name"] == "original_rank")
    baseline_score_fn = baseline_arm["score_fn"]

    # --- Step 6: full evaluate() report for both arms on TEST. ---
    baseline_report = tr.evaluate(baseline_score_fn, imputed_test_rows, test_group_records, test_labels, "test")
    model_report = tr.evaluate(full_model_score_fn, imputed_test_rows, test_group_records, test_labels, "test")

    # --- Step 8a: inference determinism -- predict twice, compare bit-for-bit. ---
    X = [r.features for r in imputed_test_rows]
    scores_a = booster.predict(X)
    scores_b = booster.predict(X)
    inference_deterministic = list(scores_a) == list(scores_b)

    # --- Gate: same computation run_offline_gate does, but keeping per-group
    #     metrics around for the Step 7 taxonomy. ---
    baseline_metrics = tr.compute_arm_group_metrics(baseline_score_fn, imputed_test_rows, "test")
    model_metrics = tr.compute_arm_group_metrics(full_model_score_fn, imputed_test_rows, "test")
    assert set(baseline_metrics) == set(model_metrics), "baseline/model scored different group sets on TEST"

    target_to_groups = {}
    for record in test_group_records:
        if record["group_id"] in test_labels and tr.split_for_target(record["target_id"]) == "test":
            target_to_groups.setdefault(record["target_id"], []).append(record["group_id"])

    bootstrap_result = tr.paired_bootstrap(
        baseline_metrics, model_metrics, target_to_groups, n_resamples=1000, seed=1234
    )
    gate_result = tr.evaluate_offline_gate(
        bootstrap_result, coverage_identical=True,
        baseline_arm="original_rank", treatment_arm="full_configured_model",
    )

    # --- Step 7: error taxonomy A-E. ---
    taxonomy = {"A": 0, "B": 0, "C": 0, "D": 0, "E": 0}
    rank_deltas_c, rank_deltas_d = [], []
    for record in test_group_records:
        gid = record["group_id"]
        if gid not in test_labels or tr.split_for_target(record["target_id"]) != "test":
            continue
        bm = baseline_metrics.get(gid)
        if bm is None or not bm["has_positive"]:
            taxonomy["A"] += 1
            continue
        mm = model_metrics[gid]
        b_rank, m_rank = bm["best_positive_rank"], mm["best_positive_rank"]
        if b_rank == 1:
            taxonomy["B"] += 1
        elif m_rank < b_rank:
            taxonomy["C"] += 1
            rank_deltas_c.append(b_rank - m_rank)
        elif m_rank > b_rank:
            taxonomy["D"] += 1
            rank_deltas_d.append(m_rank - b_rank)
        else:
            taxonomy["E"] += 1

    result = {
        "frozen_model_sha256": model_sha,
        "frequency_table_refit_matches_frozen": True,
        "test_unlabeled_group_count": test_unlabeled,
        "baseline_report": baseline_report,
        "model_report": model_report,
        "gate": gate_result,
        "error_taxonomy": {
            "counts": taxonomy,
            "net_C_minus_D": taxonomy["C"] - taxonomy["D"],
            "total_rank_improvement_C": sum(rank_deltas_c),
            "total_rank_regression_D": sum(rank_deltas_d),
            "mean_rank_improvement_per_C_group": (sum(rank_deltas_c) / len(rank_deltas_c)) if rank_deltas_c else None,
            "mean_rank_regression_per_D_group": (sum(rank_deltas_d) / len(rank_deltas_d)) if rank_deltas_d else None,
        },
        "inference_determinism": {
            "predict_twice_bit_identical": inference_deterministic,
        },
    }

    out_path = OUT / "formal_test_result.json"
    out_path.write_text(json.dumps(result, indent=2))
    print(json.dumps({
        "gate_result": gate_result["result"],
        "error_taxonomy": result["error_taxonomy"],
        "inference_deterministic": inference_deterministic,
        "out_path": str(out_path),
    }, indent=2))


if __name__ == "__main__":
    main()
