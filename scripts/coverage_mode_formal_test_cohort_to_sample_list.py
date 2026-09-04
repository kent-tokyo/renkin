#!/usr/bin/env python3
"""Converts `data/coverage_mode_formal_test/cohort_manifest.json`'s
`targets` array into the flat `{target_id, canonical_smiles, sample_rank,
source_line_number, sample_key}`
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
import re
import sys


def source_line_number(group_id: str) -> int:
    if not isinstance(group_id, str):
        raise ValueError("cohort target group_id must be a string")
    match = re.search(r"#L([1-9][0-9]*)$", group_id)
    if match is None:
        raise ValueError(f"cohort target has an invalid group_id: {group_id!r}")
    return int(match.group(1))


def convert(cohort_manifest: dict) -> list[dict]:
    return [
        {
            "target_id": t["group_id"],
            "canonical_smiles": t["canonical_smiles"],
            "sample_rank": t["cohort_rank"],
            "source_line_number": source_line_number(t["group_id"]),
            "sample_key": t["sample_key"],
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
