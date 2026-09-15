#!/usr/bin/env python3
"""Freeze one development-selected Phase 55 candidate before independent TEST.

This creates an immutable selection receipt; it neither runs a planner nor
looks at any independent TEST outcome.  The receipt binds baseline/candidate
manifests, their exact row ledgers, and the non-displacing verification that
justified selecting the candidate.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import stat
from pathlib import Path
from typing import Any


SCHEMA_VERSION = "renkin-phase55-candidate-freeze/1"
MAX_FILE_BYTES = 256 * 1024 * 1024


def _regular_file(path: str | Path) -> Path:
    value = Path(path)
    metadata = os.lstat(value)
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise ValueError(f"input must be a regular non-symlink file: {value}")
    if metadata.st_size > MAX_FILE_BYTES:
        raise ValueError(f"input exceeds {MAX_FILE_BYTES} bytes: {value}")
    return value


def sha256_file(path: str | Path) -> str:
    value = _regular_file(path)
    digest = hashlib.sha256()
    with value.open("rb") as handle:
        for chunk in iter(lambda: handle.read(65536), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _load_json(path: str | Path) -> dict[str, Any]:
    value = json.loads(_regular_file(path).read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"expected JSON object: {path}")
    return value


def _row_ids(path: str | Path) -> set[str]:
    values: set[str] = set()
    with _regular_file(path).open(encoding="utf-8") as handle:
        for line_number, line in enumerate(handle, start=1):
            if not line.strip():
                continue
            row = json.loads(line)
            target_id = row.get("target_id") if isinstance(row, dict) else None
            if not isinstance(target_id, str) or not target_id:
                raise ValueError(f"{path}:{line_number}: target_id is required")
            if target_id in values:
                raise ValueError(f"{path}:{line_number}: duplicate target_id {target_id!r}")
            values.add(target_id)
    if not values:
        raise ValueError(f"{path}: no rows")
    return values


def _require_final_manifest(manifest: dict[str, Any], label: str) -> None:
    if manifest.get("end_time_unix") is None:
        raise ValueError(f"{label} manifest is not finalized")
    if manifest.get("input_files_unchanged_during_run") is not True:
        raise ValueError(f"{label} manifest did not preserve input identity")
    if not manifest.get("configuration_id"):
        raise ValueError(f"{label} manifest lacks configuration_id")


def freeze_candidate(
    *,
    baseline_manifest_path: str | Path,
    candidate_manifest_path: str | Path,
    baseline_rows_path: str | Path,
    candidate_rows_path: str | Path,
    verification_path: str | Path,
    output_path: str | Path,
    selection_id: str,
) -> dict[str, Any]:
    if not selection_id or any(character.isspace() for character in selection_id):
        raise ValueError("selection_id must be non-empty and contain no whitespace")
    output = Path(output_path)
    if output.exists():
        raise FileExistsError(f"refusing to overwrite candidate selection: {output}")

    baseline_manifest = _load_json(baseline_manifest_path)
    candidate_manifest = _load_json(candidate_manifest_path)
    verification = _load_json(verification_path)
    _require_final_manifest(baseline_manifest, "baseline")
    _require_final_manifest(candidate_manifest, "candidate")
    baseline_ids = _row_ids(baseline_rows_path)
    candidate_ids = _row_ids(candidate_rows_path)
    if baseline_ids != candidate_ids:
        raise ValueError("baseline/candidate row target sets differ")
    if verification.get("row_count") != len(candidate_ids):
        raise ValueError("verification row_count does not match candidate rows")
    required_true = (
        "zero_regression",
        "zero_strict_regression",
        "within_budget",
        "within_process_budget",
    )
    for field in required_true:
        if verification.get(field) is not True:
            raise ValueError(f"verification does not pass {field}")
    if verification.get("timeout_count") != 0 or verification.get("crash_count") != 0:
        raise ValueError("verification contains timeout or crash")
    if verification.get("missing_attempts_count") != 0:
        raise ValueError("verification has missing recovery attempts")
    if not isinstance(verification.get("recovered_count"), int) or verification["recovered_count"] <= 0:
        raise ValueError("verification does not demonstrate a recovered route")

    baseline_inputs = baseline_manifest.get("input_file_sha256")
    candidate_inputs = candidate_manifest.get("input_file_sha256")
    if not isinstance(baseline_inputs, dict) or baseline_inputs != candidate_inputs:
        raise ValueError("baseline/candidate input identity differs")

    result = {
        "schema_version": SCHEMA_VERSION,
        "selection_status": "frozen",
        "selection_id": selection_id,
        "development_only": True,
        "target_count": len(candidate_ids),
        "baseline": {
            "manifest_sha256": sha256_file(baseline_manifest_path),
            "rows_sha256": sha256_file(baseline_rows_path),
            "configuration_id": baseline_manifest["configuration_id"],
            "tool_version": baseline_manifest.get("tool_version"),
        },
        "candidate": {
            "manifest_sha256": sha256_file(candidate_manifest_path),
            "rows_sha256": sha256_file(candidate_rows_path),
            "configuration_id": candidate_manifest["configuration_id"],
            "tool_version": candidate_manifest.get("tool_version"),
        },
        "verification": {
            "sha256": sha256_file(verification_path),
            "recovered_count": verification["recovered_count"],
            "baseline_strict_success_count": verification.get("baseline_strict_success_count"),
            "final_strict_success_count": verification.get("final_strict_success_count"),
            "budget_ms": verification.get("budget_ms"),
            "process_budget_ms": verification.get("process_budget_ms"),
        },
        "input_file_sha256": baseline_inputs,
        "execution_requirement": (
            "derive independent TEST power and cohort protocol from this frozen development-only "
            "selection; do not use independent TEST outcomes to modify it"
        ),
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline-manifest", required=True)
    parser.add_argument("--candidate-manifest", required=True)
    parser.add_argument("--baseline-rows", required=True)
    parser.add_argument("--candidate-rows", required=True)
    parser.add_argument("--verification", required=True)
    parser.add_argument("--selection-id", required=True)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    result = freeze_candidate(
        baseline_manifest_path=args.baseline_manifest,
        candidate_manifest_path=args.candidate_manifest,
        baseline_rows_path=args.baseline_rows,
        candidate_rows_path=args.candidate_rows,
        verification_path=args.verification,
        output_path=args.output,
        selection_id=args.selection_id,
    )
    print(json.dumps({"selection_status": result["selection_status"], "selection_id": result["selection_id"]}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
