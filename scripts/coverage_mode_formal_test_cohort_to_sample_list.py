#!/usr/bin/env python3
"""Converts `data/coverage_mode_formal_test/cohort_manifest.json`'s
`targets` array into the flat `{target_id, canonical_smiles, sample_rank}`
JSONL `scripts/compare_sampling.load_sample` (and therefore
`scripts/compare_run.py --sample-list`) expects. A one-time, auditable-by-
eye reshape -- `cohort_rank` becomes `sample_rank`, nothing else changes,
and no target selection happens here (that already happened when
cohort_manifest.json was generated and committed).

Usage:
    python3 scripts/coverage_mode_formal_test_cohort_to_sample_list.py \
        --cohort-manifest data/coverage_mode_formal_test/cohort_manifest.json \
        --output data/coverage_mode_formal_test/cohort_sample_list.jsonl
"""

from __future__ import annotations

import argparse
import json
import sys


def convert(cohort_manifest: dict) -> list[dict]:
    return [
        {
            "target_id": t["group_id"],
            "canonical_smiles": t["canonical_smiles"],
            "sample_rank": t["cohort_rank"],
        }
        for t in cohort_manifest["targets"]
    ]


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--cohort-manifest", default="data/coverage_mode_formal_test/cohort_manifest.json"
    )
    ap.add_argument(
        "--output", default="data/coverage_mode_formal_test/cohort_sample_list.jsonl"
    )
    args = ap.parse_args()

    with open(args.cohort_manifest, "r", encoding="utf-8") as f:
        manifest = json.load(f)
    rows = convert(manifest)

    with open(args.output, "w", encoding="utf-8") as f:
        for row in rows:
            f.write(json.dumps(row, sort_keys=True) + "\n")

    print(f"wrote {len(rows)} rows -> {args.output}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
