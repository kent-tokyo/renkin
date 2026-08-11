#!/usr/bin/env python3
"""
Chemical Space Coverage Diagnosis (Phase A PoC) -- bin formal-TEST targets
by nearest-TRAIN ECFP4 Tanimoto similarity and report, per bin:

    N, zero-positive count/rate, positive-present count,
    baseline top1 (conditioned on positive-present), reranker top1 (same).

Joins data/chemical_space_coverage_diagnosis/test_target_labels.jsonl
(scripts/chemical_space_coverage_export_test_labels.py) with
nearest_train_tanimoto.jsonl (tools/chemical-space-eval's own binary) by
target_id. Pure glue -- no chemistry logic of its own.

Usage:
    python3 tools/chemical-space-eval/report.py
"""
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
DIAG = ROOT / "data" / "chemical_space_coverage_diagnosis"


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return f"sha256:{h.hexdigest()}"

BINS = [
    ("near (>=0.80)", 0.80, 1.01),
    ("medium (0.60-0.80)", 0.60, 0.80),
    ("far (0.40-0.60)", 0.40, 0.60),
    ("very-far/OOD-like (<0.40)", -0.01, 0.40),
]


def load_jsonl(path):
    with open(path, encoding="utf-8") as f:
        return [json.loads(line) for line in f if line.strip()]


def main():
    manifest = json.loads((DIAG / "nearest_train_tanimoto.jsonl.manifest.json").read_text())

    # Integrity check: the non-committed raw inputs this report is built
    # from must still match what tools/chemical-space-eval's binary hashed
    # when it wrote them -- catches silent drift between a fingerprint run
    # and a later report run (e.g. test_target_labels.jsonl regenerated
    # with different data in between).
    labels_path = DIAG / "test_target_labels.jsonl"
    sims_path = DIAG / "nearest_train_tanimoto.jsonl"
    actual_labels_sha = sha256_file(labels_path)
    actual_sims_sha = sha256_file(sims_path)
    assert actual_labels_sha == manifest["test_labels_sha256"], (
        f"{labels_path} has changed since the fingerprint run: "
        f"{actual_labels_sha} != manifest's {manifest['test_labels_sha256']}"
    )
    assert actual_sims_sha == manifest["output_sha256"], (
        f"{sims_path} has changed since the fingerprint run: "
        f"{actual_sims_sha} != manifest's {manifest['output_sha256']}"
    )

    labels = {r["target_id"]: r for r in load_jsonl(labels_path)}
    sims = {r["target_id"]: r for r in load_jsonl(sims_path)}
    assert set(labels) == set(sims), (
        f"target_id set mismatch between labels ({len(labels)}) and similarities ({len(sims)})"
    )

    joined = []
    for target_id, label_row in labels.items():
        sim_row = sims[target_id]
        joined.append({**label_row, "nearest_train_tanimoto": sim_row["nearest_train_tanimoto"]})

    report_bins = []
    for name, lo, hi in BINS:
        rows = [r for r in joined if lo <= r["nearest_train_tanimoto"] < hi]
        n = len(rows)
        zero_positive = [r for r in rows if r["zero_positive"]]
        positive_present = [r for r in rows if not r["zero_positive"]]

        def top1_rate(key):
            hits = [r[key] for r in positive_present if r[key] is not None]
            return (sum(hits) / len(hits)) if hits else None

        report_bins.append({
            "bin": name,
            "n": n,
            "zero_positive_count": len(zero_positive),
            "zero_positive_rate": (len(zero_positive) / n) if n else None,
            "positive_present_count": len(positive_present),
            "baseline_top1_hit_rate_conditional": top1_rate("baseline_top1_hit"),
            "reranker_top1_hit_rate_conditional": top1_rate("reranker_top1_hit"),
        })

    overall = {
        "n": len(joined),
        "zero_positive_count": sum(1 for r in joined if r["zero_positive"]),
        "zero_positive_rate": sum(1 for r in joined if r["zero_positive"]) / len(joined),
    }

    result = {
        "_purpose": (
            "Chemical Space Coverage Diagnosis Phase A PoC: decompose the 33.0% "
            "(1,618/4,903) zero-positive-candidate gap on the formal TEST corpus "
            "by nearest-TRAIN ECFP4 Tanimoto similarity, to prioritize between "
            "Phase B (template-diversity scaling) and Phase C (higher-level "
            "templates). Route-solved-rate intentionally omitted from this v1 -- "
            "the only per-target route data available is the 100-target paired "
            "gate (~25/bin, a different corpus, not usable here)."
        ),
        "reference_corpus": "USPTO-50k TRAIN split product SMILES (the same split "
                             "scripts/extract_templates.py extracts templates from by "
                             "default), 39,736 unique canonical structures",
        "fingerprint": "ECFP4 (chematic 0.11.0, radius=2, nbits=2048, no chirality)",
        "provenance": {
            "_note": (
                "SHA-256 chain from this committed report back to the non-committed "
                "raw inputs it was built from (see findings.md's 'Reproducing this' "
                "to regenerate them) -- copied from "
                "nearest_train_tanimoto.jsonl.manifest.json, verified to still match "
                "the actual files on disk at report-generation time (see asserts above)."
            ),
            "source_hf_revision": manifest["source_hf_revision"],
            "renkin_commit": manifest["renkin_commit"],
            "chematic_version": manifest["chematic_version"],
            "fingerprint_config": {
                "fingerprint": manifest["fingerprint"],
                "radius": manifest["radius"],
                "nbits": manifest["nbits"],
                "chirality": manifest["chirality"],
            },
            "train_reference_path": manifest["train_reference_path"],
            "train_reference_sha256": manifest["train_reference_sha256"],
            "test_labels_path": manifest["test_labels_path"],
            "test_labels_sha256": manifest["test_labels_sha256"],
            "nearest_train_tanimoto_sha256": manifest["output_sha256"],
        },
        "overall": overall,
        "bins": report_bins,
    }

    out_path = DIAG / "coverage_by_chemical_space_report.json"
    out_path.write_text(json.dumps(result, indent=2))
    print(json.dumps(result, indent=2))
    print(f"\nWrote {out_path}")


if __name__ == "__main__":
    main()
