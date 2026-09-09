#!/usr/bin/env python3
"""Build a deterministic sample list for the native route-found gap.

Only targets where AiZynthFinder reports ``route_found`` and RENKIN does not
are selected.  The original sample rows are retained and receive contiguous
sample ranks so the normal benchmark adapters can consume the result.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path


def load_jsonl(path: str) -> list[dict]:
    with open(path, encoding="utf-8") as handle:
        return [json.loads(line) for line in handle if line.strip()]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--paired-table", required=True)
    parser.add_argument("--sample", required=True)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()

    with open(args.paired_table, encoding="utf-8") as handle:
        paired = json.load(handle)
    gap_ids = {
        row["target_id"]
        for row in paired
        if row.get("aizynthfinder_route_found") and not row.get("renkin_route_found")
    }
    sample_rows = {row["target_id"]: row for row in load_jsonl(args.sample)}
    missing = gap_ids - sample_rows.keys()
    if missing:
        raise SystemExit(f"sample is missing {len(missing)} native-gap target(s)")

    selected = [sample_rows[target_id] for target_id in sorted(gap_ids)]
    for rank, row in enumerate(selected):
        row["sample_rank"] = rank

    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(
        "".join(json.dumps(row, sort_keys=True) + "\n" for row in selected),
        encoding="utf-8",
    )
    print(json.dumps({"targets": len(selected), "output": str(output)}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
