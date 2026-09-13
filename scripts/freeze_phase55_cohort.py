"""Freeze an audited Phase 55.6 cohort before either planner is run.

The command is intentionally one-way: it accepts only an eligible provenance
audit whose manifest hash matches the candidate, requires an explicit freeze
ID, and refuses to overwrite an existing output.  It does not execute a
benchmark or inspect route-search outcomes.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import stat
from pathlib import Path

MAX_FILE_BYTES = 256 * 1024 * 1024
FREEZE_SCHEMA_VERSION = 1


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
    with candidate.open("rb") as handle:
        for chunk in iter(lambda: handle.read(65536), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _load_object(path: str | Path) -> dict:
    candidate = _regular_file(path)
    value = json.loads(candidate.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"artifact must be a JSON object: {candidate}")
    return value


def freeze_candidate(
    candidate_path: str | Path,
    audit_path: str | Path,
    output_path: str | Path,
    freeze_id: str,
    sample_list_output: str | Path | None = None,
) -> dict:
    if not freeze_id or any(character.isspace() for character in freeze_id):
        raise ValueError("freeze_id must be non-empty and contain no whitespace")
    candidate = _load_object(candidate_path)
    audit = _load_object(audit_path)
    targets = candidate.get("targets")
    if not isinstance(targets, list) or not targets:
        raise ValueError("candidate must contain a non-empty targets array")
    if audit.get("eligible") is not True or audit.get("blockers"):
        raise ValueError("candidate provenance audit is not eligible")
    candidate_hash = sha256_file(candidate_path)
    if audit.get("manifest", {}).get("sha256") != candidate_hash:
        raise ValueError("audit was not produced from this candidate manifest")
    output = Path(output_path)
    if output.exists():
        raise FileExistsError(f"refusing to overwrite frozen manifest: {output}")
    sample_output = Path(sample_list_output) if sample_list_output is not None else None
    if sample_output is not None and sample_output.exists():
        raise FileExistsError(f"refusing to overwrite frozen sample list: {sample_output}")
    frozen = {
        "freeze_schema_version": FREEZE_SCHEMA_VERSION,
        "freeze_status": "frozen",
        "freeze_id": freeze_id,
        "candidate_manifest": {"path": str(candidate_path), "sha256": candidate_hash},
        "provenance_audit": {
            "path": str(audit_path),
            "sha256": sha256_file(audit_path),
            "eligible": True,
        },
        "selection": {
            "protocol_version": candidate.get("protocol_version"),
            "source": candidate.get("source"),
            "exclusions": candidate.get("exclusions", []),
            "cohort_size": candidate.get("cohort_size"),
            "cohort_targets_sha256": candidate.get("cohort_targets_sha256"),
        },
        "targets": targets,
        "execution_requirement": (
            "freeze this artifact before running RENKIN or AiZynthFinder; "
            "do not modify targets after observing outcomes"
        ),
    }
    if sample_output is not None:
        sample_text = "\n".join(
            json.dumps(row, sort_keys=True) for row in targets
        ) + "\n"
        sample_output.parent.mkdir(parents=True, exist_ok=True)
        sample_output.write_text(sample_text, encoding="utf-8")
        frozen["sample_list"] = {
            "path": str(sample_output),
            "sha256": hashlib.sha256(sample_text.encode("utf-8")).hexdigest(),
            "rows": len(targets),
        }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(frozen, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return frozen


def verify_frozen_manifest(path: str | Path) -> dict:
    frozen = _load_object(path)
    blockers = []
    if frozen.get("freeze_schema_version") != FREEZE_SCHEMA_VERSION:
        blockers.append("freeze_schema_version_mismatch")
    if frozen.get("freeze_status") != "frozen":
        blockers.append("not_frozen")
    if not isinstance(frozen.get("freeze_id"), str) or not frozen["freeze_id"]:
        blockers.append("missing_freeze_id")
    targets = frozen.get("targets")
    if not isinstance(targets, list) or not targets:
        blockers.append("missing_targets")
        targets = []
    target_ids = []
    for row in targets:
        if not isinstance(row, dict):
            blockers.append("malformed_target")
            continue
        target_ids.append(row.get("target_id"))
    if len(set(target_ids)) != len(target_ids):
        blockers.append("duplicate_target_id")
    if [row.get("sample_rank") for row in targets if isinstance(row, dict)] != list(range(len(targets))):
        blockers.append("non_contiguous_sample_rank")
    target_text = "\n".join(
        json.dumps(row, sort_keys=True, separators=(",", ":")) for row in targets
    ) + "\n"
    expected_hash = hashlib.sha256(target_text.encode("utf-8")).hexdigest()
    if frozen.get("selection", {}).get("cohort_targets_sha256") != expected_hash:
        blockers.append("cohort_hash_mismatch")
    if frozen.get("provenance_audit", {}).get("eligible") is not True:
        blockers.append("provenance_audit_not_eligible")
    sample_list = frozen.get("sample_list")
    if sample_list is not None:
        if not isinstance(sample_list, dict) or not sample_list.get("path"):
            blockers.append("malformed_sample_list_reference")
        else:
            try:
                sample_path = _regular_file(sample_list["path"])
                if sha256_file(sample_path) != sample_list.get("sha256"):
                    blockers.append("sample_list_hash_mismatch")
                if sample_list.get("rows") != len(targets):
                    blockers.append("sample_list_row_count_mismatch")
            except (OSError, ValueError):
                blockers.append("sample_list_missing_or_invalid")
    return {
        "path": str(path),
        "sha256": sha256_file(path),
        "eligible": not blockers,
        "target_count": len(targets),
        "freeze_id": frozen.get("freeze_id"),
        "blockers": blockers,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--candidate", required=True)
    parser.add_argument("--audit", required=True)
    parser.add_argument("--freeze-id", required=True)
    parser.add_argument("--sample-list-output")
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    frozen = freeze_candidate(
        args.candidate,
        args.audit,
        args.output,
        args.freeze_id,
        sample_list_output=args.sample_list_output,
    )
    print(json.dumps({"freeze_status": frozen["freeze_status"], "freeze_id": frozen["freeze_id"]}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
