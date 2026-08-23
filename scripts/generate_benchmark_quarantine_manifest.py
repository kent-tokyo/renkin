"""Issue #101 Phase 3A Round 2, Section A: freeze the existing 4,903-target
competitive benchmark corpus (`data/comparison/sample_full_sorted.jsonl`,
used by every Issue #101 Phase 1/2 gate and the future 500/4,903-target
route-search comparison against AiZynthFinder) as a FORMAL TEST QUARANTINE.

Why this exists: Phase 3A Round 1 derived reranker labels from this same
corpus and planned to re-split it 70/15/15 for reranker train/val/test.
That is fine for an offline-only evaluation, but if the trained reranker is
later integrated into search and re-run against this same benchmark to
compare against AiZynthFinder, ~70% of the "competitive" targets would have
been in the reranker's own training set -- invalidating any resulting
"beats AiZynthFinder" claim. See
`data/phase3a_reranker_ground_truth_audit/round2_split_hygiene.md` for the
full writeup.

This script records exactly which molecules are quarantined (a
canonical-identity digest, not just a line count) so any later train/val
generation step can mechanically check for overlap, and so a future
reviewer can verify the quarantine boundary was actually respected.

Usage:
    cargo build --release --bin renkin-canonicalize
    python3 scripts/generate_benchmark_quarantine_manifest.py \
        --corpus data/comparison/sample_full_sorted.jsonl \
        --canonicalize-bin target/release/renkin-canonicalize \
        --identities-output data/phase3a_reranker_ground_truth_audit/benchmark_quarantine_target_identities.txt \
        --manifest-output data/phase3a_reranker_ground_truth_audit/benchmark_quarantine_manifest.json
"""

from __future__ import annotations

import argparse
import json
import sys

from reranker_label_common import canonicalize_batch, sha256_of


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--corpus", default="data/comparison/sample_full_sorted.jsonl")
    parser.add_argument("--canonicalize-bin", default="target/release/renkin-canonicalize")
    parser.add_argument(
        "--identities-output",
        default="data/phase3a_reranker_ground_truth_audit/benchmark_quarantine_target_identities.txt",
    )
    parser.add_argument(
        "--manifest-output",
        default="data/phase3a_reranker_ground_truth_audit/benchmark_quarantine_manifest.json",
    )
    args = parser.parse_args(argv)

    with open(args.corpus, "r", encoding="utf-8") as f:
        rows = [json.loads(line) for line in f if line.strip()]

    smiles = [r["canonical_smiles"] for r in rows]
    canon = canonicalize_batch(smiles, args.canonicalize_bin)
    n_parse_fail = sum(1 for c in canon if c is None)
    if n_parse_fail:
        raise RuntimeError(
            f"{n_parse_fail} benchmark target(s) failed to canonicalize -- a "
            "quarantine that can't identify its own targets is useless. Fix "
            "the corpus or the canonicalizer before proceeding."
        )

    identities = sorted(set(canon))
    if len(identities) != len(canon):
        raise RuntimeError(
            f"benchmark corpus has {len(canon)} rows but only "
            f"{len(identities)} distinct canonical identities -- duplicate "
            "targets must be resolved before this corpus can be quarantined "
            "(a duplicate would otherwise silently count once for overlap "
            "checks but twice for n_targets)"
        )

    with open(args.identities_output, "w", encoding="utf-8") as f:
        for ident in identities:
            f.write(ident + "\n")

    manifest = {
        "corpus_path": args.corpus,
        "corpus_sha256": sha256_of(args.corpus),
        "n_targets": len(rows),
        "n_distinct_target_identities": len(identities),
        "identities_path": args.identities_output,
        "identities_sha256": sha256_of(args.identities_output),
        "source_dataset": "bisectgroup/USPTO_50K",
        "source_dataset_revision": "08a575f0546b2be57242997fd45f684d6814d5a9",
        "source_dataset_split": "test",
        "canonicalizer": "renkin-canonicalize --clear-atom-maps",
        "purpose": (
            "FORMAL TEST QUARANTINE: these target identities must never "
            "appear in reranker training or validation data. Any raw "
            "train/val reaction whose product canonicalizes to one of "
            "these identities must be excluded -- see "
            "generate_train_val_labels.py's decontamination step."
        ),
    }
    with open(args.manifest_output, "w", encoding="utf-8") as f:
        json.dump(manifest, f, indent=2, sort_keys=True)
        f.write("\n")

    print(json.dumps(manifest, indent=2, sort_keys=True), file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
