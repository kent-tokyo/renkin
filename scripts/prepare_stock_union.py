#!/usr/bin/env python3
"""Create a deterministic local SMILES union from stock source files."""

from __future__ import annotations

import argparse
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", required=True)
    parser.add_argument("sources", nargs="+")
    args = parser.parse_args()

    records: dict[str, tuple[str, int]] = {}
    for source_index, source_name in enumerate(args.sources):
        for line_number, raw in enumerate(
            Path(source_name).read_text(encoding="utf-8").splitlines(), start=1
        ):
            line = raw.strip()
            if not line or line.startswith("#"):
                continue
            smiles = line.split()[0]
            records.setdefault(smiles, (source_name, line_number))

    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(
        "# Local benchmark stock union; source provenance is recorded by the caller.\n"
        + "\n".join(sorted(records))
        + "\n",
        encoding="utf-8",
    )
    print(f"unique_raw_smiles={len(records)}")
    for source_name in args.sources:
        print(
            f"{source_name}: "
            f"{sum(1 for source, _ in records.values() if source == source_name)}"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
