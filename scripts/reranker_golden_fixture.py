#!/usr/bin/env python3
"""
Issue #101 runtime-integration golden test, Python side. Loads the formal
TEST pool (same imputed rows `phase3e_evaluate_formal_test.py` scored),
takes a deterministic sample, and dumps `{"features": [...]}` (null =
missing, matching CandidateFeatures.missing) alongside the FROZEN model's
own `booster.predict()` score for that exact row -- ground truth for the
Rust `LightGbmModel` reader (`src/reranker.rs`) to be checked against via
`renkin-reranker-predict`.

Usage:
    python3 scripts/reranker_golden_fixture.py [--n 2000] [--seed 7]
"""
import argparse
import importlib.util
import json
import random
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "data" / "phase3e_reranker_training"

spec = importlib.util.spec_from_file_location("train_reranker", ROOT / "scripts" / "train_reranker.py")
tr = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tr)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--n", type=int, default=2000)
    ap.add_argument("--seed", type=int, default=7)
    ap.add_argument("--out", default=str(OUT / "reranker_golden_fixture.jsonl"))
    args = ap.parse_args()

    freq_table = json.loads((OUT / "frequency_table.json").read_text())["table"]

    split_map = {}
    for row in tr.load_jsonl(OUT / "split_manifest_combined.jsonl"):
        split_map[row["target_id"]] = row["split"]
    test_group_records = tr.load_jsonl(OUT / "groups_test_formal.jsonl")
    for r in test_group_records:
        split_map.setdefault(r["target_id"], "test")
    tr.configure_split_override(split_map)

    test_pool_rows = tr.load_jsonl(OUT / "pool_test_formal.jsonl")
    test_labels = tr.load_labels(ROOT / "data" / "reranker_labels_uspto50k_test.jsonl")
    tr.validate_pool_rows(test_pool_rows, test_group_records)
    test_labeled, unlabeled = tr.label_and_split_rows(
        test_pool_rows, test_labels, test_group_records, allow_unlabeled=False
    )
    assert unlabeled == 0
    test_rows = [r for r in test_labeled if r.split == "test"]
    imputed = tr.impute_frequency_features(test_rows, freq_table)

    rng = random.Random(args.seed)
    sample = imputed if len(imputed) <= args.n else rng.sample(imputed, args.n)

    import lightgbm as lgb
    booster = lgb.Booster(model_file=str(OUT / "model.txt"))
    X = [r.features for r in sample]
    python_scores = booster.predict(X)

    with open(args.out, "w") as f:
        for row, score in zip(sample, python_scores):
            features = [None if (v != v) else v for v in row.features]  # NaN -> null
            f.write(json.dumps({"features": features, "python_score": float(score)}) + "\n")

    print(json.dumps({"n_rows": len(sample), "out": args.out}))


if __name__ == "__main__":
    main()
