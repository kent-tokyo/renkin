"""Fail-closed preflight for the v1.0.0 formal planner comparison.

This command validates the immutable input boundary before an expensive arm is
started. It intentionally does not run a planner and does not interpret
results; superiority is only assessed by the paired report after every arm has
the same complete target-id set.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import stat
import sys


FORMAL_TARGET_COUNT = 4903
SHA256_HEX_LENGTH = 64


def sha256_file(path: str) -> str:
    metadata = os.lstat(path)
    if stat.S_ISLNK(metadata.st_mode):
        raise ValueError(f"input must not be a symlink: {path!r}")
    if not stat.S_ISREG(metadata.st_mode):
        raise ValueError(f"input must be a regular file: {path!r}")
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(65536), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_target_list(path: str) -> list[dict]:
    rows: list[dict] = []
    with open(path, encoding="utf-8") as handle:
        for line_number, line in enumerate(handle, start=1):
            if not line.strip():
                raise ValueError(f"target list contains a blank line at {line_number}")
            try:
                row = json.loads(line)
            except json.JSONDecodeError as exc:
                raise ValueError(f"invalid JSON at target-list line {line_number}: {exc}") from exc
            if not isinstance(row, dict):
                raise ValueError(f"target-list line {line_number} is not an object")
            rows.append(row)
    return rows


def validate_target_list(path: str, expected_sha256: str | None) -> dict:
    rows = load_target_list(path)
    if len(rows) != FORMAL_TARGET_COUNT:
        raise ValueError(
            f"formal target list must contain exactly {FORMAL_TARGET_COUNT} rows; got {len(rows)}"
        )
    ids: set[str] = set()
    for expected_rank, row in enumerate(rows):
        required = ("sample_rank", "target_id", "canonical_smiles", "sample_key")
        missing = [key for key in required if key not in row]
        if missing:
            raise ValueError(f"target row {expected_rank} missing {', '.join(missing)}")
        if row["sample_rank"] != expected_rank:
            raise ValueError(f"target row {expected_rank} has non-contiguous sample_rank")
        target_id = row["target_id"]
        if not isinstance(target_id, str) or not target_id or target_id in ids:
            raise ValueError(f"target row {expected_rank} has a missing or duplicate target_id")
        ids.add(target_id)
        if not isinstance(row["canonical_smiles"], str) or not row["canonical_smiles"]:
            raise ValueError(f"target row {expected_rank} has an empty canonical_smiles")
        sample_key = row["sample_key"]
        if not isinstance(sample_key, str) or len(sample_key) != SHA256_HEX_LENGTH:
            raise ValueError(f"target row {expected_rank} has an invalid sample_key")
    actual_sha256 = sha256_file(path)
    if expected_sha256 and actual_sha256 != expected_sha256:
        raise ValueError(
            f"target list SHA-256 mismatch: expected {expected_sha256}, got {actual_sha256}"
        )
    return {"path": path, "rows": len(rows), "sha256": actual_sha256}


def validate_file(path: str, label: str, expected_sha256: str | None) -> dict:
    actual_sha256 = sha256_file(path)
    if expected_sha256 and actual_sha256 != expected_sha256:
        raise ValueError(f"{label} SHA-256 mismatch: expected {expected_sha256}, got {actual_sha256}")
    return {"path": path, "sha256": actual_sha256}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target-list", required=True)
    parser.add_argument("--stock", required=True)
    parser.add_argument("--templates", required=True)
    parser.add_argument("--target-list-sha256", required=True)
    parser.add_argument("--stock-sha256", required=True)
    parser.add_argument("--templates-sha256", required=True)
    args = parser.parse_args(argv)
    try:
        for label, digest in (("target-list", args.target_list_sha256), ("stock", args.stock_sha256), ("templates", args.templates_sha256)):
            if len(digest) != SHA256_HEX_LENGTH:
                raise ValueError(f"{label} expected SHA-256 must be 64 hex characters")
        result = {
            "protocol": "renkin-v1.0.0-formal-competitor-comparison-v1",
            "target_count": FORMAL_TARGET_COUNT,
            "target_list": validate_target_list(args.target_list, args.target_list_sha256),
            "stock": validate_file(args.stock, "stock", args.stock_sha256),
            "templates": validate_file(args.templates, "templates", args.templates_sha256),
            "complete_input_boundary": True,
        }
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        print(f"FORMAL PREFLIGHT: FAIL: {exc}", file=sys.stderr)
        return 2
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
