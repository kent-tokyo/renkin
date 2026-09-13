"""Prepare a fail-closed, outcome-blind candidate cohort for Phase 55.6.

This utility selects from an already frozen canonical target list.  It removes
target IDs and canonical structures present in explicitly supplied historical
artifacts, then hashes the remaining rows into a deterministic order.  It does
not claim statistical independence from training or template-extraction data;
that provenance audit is recorded as a separate required gate.

The output is a *candidate* manifest and must be frozen before either planner
is run against it.  Search outcomes are never read by this script as a
selection criterion.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import stat
from pathlib import Path

PROTOCOL_VERSION = "renkin-phase55-independent-cohort-v1"
MAX_FILE_BYTES = 128 * 1024 * 1024
MAX_LINE_BYTES = 256 * 1024


def _regular_file(path: str | Path) -> Path:
    candidate = Path(path)
    metadata = os.lstat(candidate)
    if stat.S_ISLNK(metadata.st_mode):
        raise ValueError(f"input must not be a symlink: {candidate}")
    if not stat.S_ISREG(metadata.st_mode):
        raise ValueError(f"input must be a regular file: {candidate}")
    if metadata.st_size > MAX_FILE_BYTES:
        raise ValueError(f"input exceeds {MAX_FILE_BYTES} bytes: {candidate}")
    return candidate


def sha256_file(path: str | Path) -> str:
    candidate = _regular_file(path)
    digest = hashlib.sha256()
    total = 0
    with candidate.open("rb") as handle:
        for chunk in iter(lambda: handle.read(65536), b""):
            total += len(chunk)
            if total > MAX_FILE_BYTES:
                raise ValueError(f"input exceeds {MAX_FILE_BYTES} bytes: {candidate}")
            digest.update(chunk)
    return digest.hexdigest()


def _records(path: str | Path) -> list[dict]:
    """Read JSON or JSONL artifacts without silently accepting malformed rows."""
    candidate = _regular_file(path)
    with candidate.open("rb") as handle:
        raw = handle.read(MAX_FILE_BYTES + 1)
    if len(raw) > MAX_FILE_BYTES:
        raise ValueError(f"input exceeds {MAX_FILE_BYTES} bytes: {candidate}")
    text = raw.decode("utf-8")
    stripped = text.strip()
    if not stripped:
        return []
    try:
        value = json.loads(stripped)
    except json.JSONDecodeError:
        rows = []
        for line_number, line in enumerate(text.splitlines(), start=1):
            if len(line.encode("utf-8")) > MAX_LINE_BYTES:
                raise ValueError(f"line {line_number} exceeds {MAX_LINE_BYTES} bytes: {candidate}")
            if not line.strip():
                continue
            row = json.loads(line)
            if not isinstance(row, dict):
                raise ValueError(f"JSONL row {line_number} is not an object: {candidate}")
            rows.append(row)
        return rows
    if isinstance(value, list):
        if not all(isinstance(row, dict) for row in value):
            raise ValueError(f"JSON array contains a non-object row: {candidate}")
        return value
    if isinstance(value, dict):
        return [value]
    raise ValueError(f"JSON artifact must contain objects: {candidate}")


def _identity_sets(paths: list[str | Path]) -> tuple[set[str], set[str], list[dict]]:
    target_ids: set[str] = set()
    canonical_smiles: set[str] = set()
    details: list[dict] = []
    for path in paths:
        rows = _records(path)
        ids_before = len(target_ids)
        smiles_before = len(canonical_smiles)
        for row in rows:
            for key in ("target_id", "sample_id"):
                value = row.get(key)
                if isinstance(value, str) and value:
                    target_ids.add(value)
            value = row.get("canonical_smiles")
            if isinstance(value, str) and value:
                canonical_smiles.add(value)
        details.append(
            {
                "path": str(path),
                "sha256": sha256_file(path),
                "rows": len(rows),
                "new_target_ids": len(target_ids) - ids_before,
                "new_canonical_smiles": len(canonical_smiles) - smiles_before,
            }
        )
    return target_ids, canonical_smiles, details


def _sample_key(canonical_smiles: str) -> str:
    return hashlib.sha256(
        f"{PROTOCOL_VERSION}|{canonical_smiles}".encode("utf-8")
    ).hexdigest()


def _load_source(path: str | Path) -> list[dict]:
    rows = _records(path)
    seen_ids: set[str] = set()
    seen_smiles: set[str] = set()
    for index, row in enumerate(rows, start=1):
        target_id = row.get("target_id")
        canonical = row.get("canonical_smiles")
        if not isinstance(target_id, str) or not target_id:
            raise ValueError(f"source row {index} has no target_id")
        if not isinstance(canonical, str) or not canonical:
            raise ValueError(f"source row {index} has no canonical_smiles")
        source_line = row.get("source_line_number")
        if source_line is not None and (
            isinstance(source_line, bool) or not isinstance(source_line, int) or source_line < 1
        ):
            raise ValueError(f"source row {index} has an invalid source_line_number")
        if target_id in seen_ids:
            raise ValueError(f"duplicate target_id in source: {target_id}")
        if canonical in seen_smiles:
            raise ValueError(f"duplicate canonical_smiles in source: {canonical}")
        seen_ids.add(target_id)
        seen_smiles.add(canonical)
    return rows


def build_cohort(
    source: str | Path,
    exclusions: list[str | Path],
    cohort_size: int,
) -> dict:
    if isinstance(cohort_size, bool) or cohort_size <= 0:
        raise ValueError("cohort_size must be a positive integer")
    source_rows = _load_source(source)
    excluded_ids, excluded_smiles, exclusion_details = _identity_sets(exclusions)
    eligible = [
        row
        for row in source_rows
        if row["target_id"] not in excluded_ids
        and row["canonical_smiles"] not in excluded_smiles
    ]
    if len(eligible) < cohort_size:
        raise ValueError(
            f"only {len(eligible)} eligible targets remain; need {cohort_size}"
        )
    keyed = sorted(
        ((_sample_key(row["canonical_smiles"]), row) for row in eligible),
        key=lambda item: (item[0], item[1]["canonical_smiles"]),
    )
    selected = []
    for rank, (sample_key, row) in enumerate(keyed[:cohort_size]):
        selected_row = {
                "sample_rank": rank,
                "target_id": row["target_id"],
                "canonical_smiles": row["canonical_smiles"],
                "sample_key": sample_key,
        }
        if "source_line_number" in row:
            selected_row["source_line_number"] = row["source_line_number"]
        selected.append(selected_row)
    selected_text = "\n".join(
        json.dumps(row, sort_keys=True, separators=(",", ":")) for row in selected
    ) + "\n"
    return {
        "protocol_version": PROTOCOL_VERSION,
        "selection_rule": (
            "SHA256(protocol_version + '|' + canonical_smiles), sorted by "
            "(sample_key, canonical_smiles), first N eligible rows"
        ),
        "source": {"path": str(source), "sha256": sha256_file(source), "rows": len(source_rows)},
        "exclusions": exclusion_details,
        "excluded_identity_counts": {
            "target_ids": len(excluded_ids),
            "canonical_smiles": len(excluded_smiles),
        },
        "eligible_rows": len(eligible),
        "cohort_size": cohort_size,
        "cohort_targets_sha256": hashlib.sha256(selected_text.encode("utf-8")).hexdigest(),
        "independence_status": "pending_provenance_audit",
        "freeze_requirement": "freeze this manifest before running either planner",
        "targets": selected,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", required=True, help="frozen canonical target JSONL")
    parser.add_argument("--exclude", action="append", default=[], help="historical artifact; repeatable")
    parser.add_argument("--cohort-size", type=int, required=True)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    manifest = build_cohort(args.source, args.exclude, args.cohort_size)
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"cohort_size": manifest["cohort_size"], "output": str(output)}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
