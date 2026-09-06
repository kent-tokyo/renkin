#!/usr/bin/env python3
"""Build a deterministic base + frequency-filtered additional template tier."""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import OrderedDict
from pathlib import Path


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load(path: Path) -> list[tuple[str, int]]:
    rows = []
    seen = set()
    for line_number, raw in enumerate(path.read_text().splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        fields = line.split("\t")
        if len(fields) != 2:
            raise ValueError(f"{path}:{line_number}: expected SMIRKS<TAB>count")
        smirks, count_text = fields
        count = int(count_text)
        if count <= 0 or smirks in seen:
            raise ValueError(f"{path}:{line_number}: duplicate SMIRKS or non-positive count")
        seen.add(smirks)
        rows.append((smirks, count))
    return rows


def build(
    base: list[tuple[str, int]],
    additional: list[tuple[str, int]],
    minimum_additional_count: int,
) -> tuple[list[tuple[str, int]], dict[str, int]]:
    if minimum_additional_count <= 0:
        raise ValueError("minimum_additional_count must be positive")
    merged = OrderedDict(base)
    selected = 0
    overlap = 0
    for smirks, count in additional:
        if count < minimum_additional_count:
            continue
        selected += 1
        if smirks in merged:
            overlap += 1
            merged[smirks] = max(merged[smirks], count)
        else:
            merged[smirks] = count
    accounting = {
        "base_count": len(base),
        "additional_input_count": len(additional),
        "additional_selected_count": selected,
        "additional_overlap_count": overlap,
        "additional_unique_added_count": selected - overlap,
        "output_count": len(merged),
    }
    return list(merged.items()), accounting


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", required=True, type=Path)
    parser.add_argument("--additional", required=True, type=Path)
    parser.add_argument("--additional-min-count", required=True, type=int)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--manifest", required=True, type=Path)
    args = parser.parse_args()

    rows, accounting = build(
        load(args.base), load(args.additional), args.additional_min_count
    )
    output_text = "\n".join(
        [
            "# RENKIN deterministic template union",
            f"# Additional minimum count: {args.additional_min_count}",
            "# Format: SMIRKS<TAB>count",
            *(f"{smirks}\t{count}" for smirks, count in rows),
            "",
        ]
    )
    args.output.write_text(output_text)
    manifest = {
        "schema_version": 1,
        "base_path": str(args.base),
        "base_sha256": sha256(args.base),
        "additional_path": str(args.additional),
        "additional_sha256": sha256(args.additional),
        "additional_min_count": args.additional_min_count,
        "output_path": str(args.output),
        "output_sha256": hashlib.sha256(output_text.encode()).hexdigest(),
        "builder_sha256": sha256(Path(__file__)),
        "accounting": accounting,
    }
    args.manifest.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    print(json.dumps(manifest, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
