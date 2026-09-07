"""Build a frozen comparison sample list from paired-atlas target IDs.

The output retains the canonical sample rows and reassigns contiguous ranks so
the normal comparison harness can run an explicitly selected failure cohort.
"""

from __future__ import annotations

import argparse
import json


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--atlas", required=True)
    parser.add_argument("--sample", required=True)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()

    with open(args.atlas, encoding="utf-8") as handle:
        atlas = json.load(handle)
    ids = {record["target_id"] for record in atlas["records"]}
    with open(args.sample, encoding="utf-8") as handle:
        sample_rows = [json.loads(line) for line in handle if line.strip()]
    selected = [row for row in sample_rows if row["target_id"] in ids]
    if len(selected) != len(ids):
        raise SystemExit(f"sample is missing {len(ids) - len(selected)} atlas target(s)")
    selected.sort(key=lambda row: row["target_id"])
    for rank, row in enumerate(selected):
        row["sample_rank"] = rank
    with open(args.output, "w", encoding="utf-8") as handle:
        for row in selected:
            handle.write(json.dumps(row, sort_keys=True) + "\n")
    print(json.dumps({"targets": len(selected), "output": args.output}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
