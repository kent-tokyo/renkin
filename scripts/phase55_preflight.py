#!/usr/bin/env python3
"""Fail-closed preflight for the Phase 55 paired benchmark.

This checks comparability only; it does not compute a victory claim.  A pair
is eligible for Phase 55.6 only when its manifests and row files describe the
same target set, inputs, budget, and measured revision.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any


REQUIRED_INPUTS = ("sample_list", "stock", "templates")
COMMON_BUDGET_FIELDS = ("timeout_s", "grace_s", "max_routes")


def load_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{path}: expected JSON object")
    return value


def row_ids(path: Path) -> set[str]:
    ids: set[str] = set()
    with path.open(encoding="utf-8") as stream:
        for line_number, line in enumerate(stream, 1):
            if not line.strip():
                continue
            row = json.loads(line)
            target_id = row.get("target_id") if isinstance(row, dict) else None
            if not isinstance(target_id, str) or not target_id:
                raise ValueError(f"{path}:{line_number}: missing target_id")
            if target_id in ids:
                raise ValueError(f"{path}:{line_number}: duplicate target_id={target_id!r}")
            ids.add(target_id)
    return ids


def _file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(65536), b""):
            digest.update(chunk)
    return digest.hexdigest()


def frozen_cohort_preflight(frozen_manifest_path: Path) -> dict[str, Any]:
    """Verify that a frozen cohort and its runner sample list still agree."""
    frozen = load_json(frozen_manifest_path)
    blockers: list[str] = []
    if frozen.get("freeze_status") != "frozen":
        blockers.append("cohort_not_frozen")
    targets = frozen.get("targets")
    if not isinstance(targets, list) or not targets:
        blockers.append("cohort_targets_missing")
        targets = []
    frozen_ids = []
    for row in targets:
        if not isinstance(row, dict) or not isinstance(row.get("target_id"), str):
            blockers.append("cohort_target_malformed")
            continue
        frozen_ids.append(row["target_id"])
    if len(set(frozen_ids)) != len(frozen_ids):
        blockers.append("cohort_target_duplicate")
    sample_ref = frozen.get("sample_list")
    sample_path = None
    sample_ids: set[str] = set()
    if not isinstance(sample_ref, dict) or not isinstance(sample_ref.get("path"), str):
        blockers.append("cohort_sample_list_reference_missing")
    else:
        sample_path = Path(sample_ref["path"])
        if not sample_path.is_file():
            blockers.append("cohort_sample_list_missing")
        else:
            try:
                sample_ids = row_ids(sample_path)
                if _file_sha256(sample_path) != sample_ref.get("sha256"):
                    blockers.append("cohort_sample_list_hash_mismatch")
            except (OSError, ValueError, json.JSONDecodeError):
                blockers.append("cohort_sample_list_invalid")
    if set(frozen_ids) != sample_ids:
        blockers.append("cohort_target_id_set_mismatch")
    if sample_ref and sample_ref.get("rows") != len(sample_ids):
        blockers.append("cohort_sample_list_row_count_mismatch")
    return {
        "schema_version": "renkin-phase55-cohort-preflight/1",
        "eligible": not blockers,
        "freeze_id": frozen.get("freeze_id"),
        "target_count": len(frozen_ids),
        "sample_list": str(sample_path) if sample_path is not None else None,
        "blockers": blockers,
    }


def smoke_preflight(
    frozen_manifest_path: Path,
    left_manifest: dict[str, Any],
    right_manifest: dict[str, Any],
    left_ids: set[str],
    right_ids: set[str],
    *,
    smoke_size: int,
    require_clean: bool = True,
) -> dict[str, Any]:
    """Verify a paired smoke run is the deterministic prefix of a frozen cohort.

    A 50-target smoke is an environment/adapter gate, not an opportunity to
    choose easier targets.  Both runners keep the full frozen sample list as
    their hash-addressed input and select its first ``smoke_size`` rows.
    """
    cohort = load_json(frozen_manifest_path)
    cohort_result = frozen_cohort_preflight(frozen_manifest_path)
    result = preflight(
        left_manifest,
        right_manifest,
        left_ids,
        right_ids,
        require_clean=require_clean,
    )
    blockers = list(cohort_result["blockers"]) + list(result["blockers"])

    targets = cohort.get("targets")
    if not isinstance(smoke_size, int) or smoke_size <= 0:
        blockers.append("smoke_size_must_be_positive")
        targets = []
    if not isinstance(targets, list) or smoke_size > len(targets):
        blockers.append("smoke_size_exceeds_frozen_cohort")
        targets = []
    expected_ids: set[str] = set()
    if targets:
        for row in targets[:smoke_size]:
            target_id = row.get("target_id") if isinstance(row, dict) else None
            if not isinstance(target_id, str) or not target_id:
                blockers.append("smoke_target_malformed")
                continue
            expected_ids.add(target_id)
        if len(expected_ids) != smoke_size:
            blockers.append("smoke_target_duplicate")

    if left_ids != expected_ids or right_ids != expected_ids:
        blockers.append(
            "smoke_target_set_mismatch:"
            f"expected={len(expected_ids)}:left={len(left_ids)}:right={len(right_ids)}"
        )

    sample = cohort.get("sample_list")
    expected_hash = sample.get("sha256") if isinstance(sample, dict) else None
    for label, arm in (("left", left_manifest), ("right", right_manifest)):
        hashes = arm.get("input_file_sha256")
        actual_hash = hashes.get("sample_list") if isinstance(hashes, dict) else None
        if not expected_hash or actual_hash != expected_hash:
            blockers.append(f"{label}_smoke_sample_hash_mismatch")

    return {
        "schema_version": "renkin-phase55-smoke-preflight/1",
        "eligible": not blockers,
        "freeze_id": cohort.get("freeze_id"),
        "smoke_size": smoke_size,
        "expected_target_count": len(expected_ids),
        "blockers": blockers,
        "pair_preflight": result,
        "cohort_preflight": cohort_result,
    }


def preflight(
    left_manifest: dict[str, Any],
    right_manifest: dict[str, Any],
    left_ids: set[str],
    right_ids: set[str],
    *,
    require_clean: bool = True,
    require_effective_resource_enforcement: bool = False,
    require_effective_output_settings: bool = False,
) -> dict[str, Any]:
    blockers: list[str] = []
    if left_ids != right_ids:
        blockers.append(
            f"target_id_set_mismatch:left_only={len(left_ids - right_ids)}:"
            f"right_only={len(right_ids - left_ids)}"
        )

    for field in ("comparison_mode", "tool_version"):
        if field == "tool_version":
            if not left_manifest.get(field) or not right_manifest.get(field):
                blockers.append("missing_tool_version")
        elif left_manifest.get(field) != right_manifest.get(field):
            blockers.append(f"comparison_mode_mismatch:{left_manifest.get(field)!r}:{right_manifest.get(field)!r}")

    left_hashes = left_manifest.get("input_file_sha256") or {}
    right_hashes = right_manifest.get("input_file_sha256") or {}
    for field in REQUIRED_INPUTS:
        if not left_hashes.get(field) or not right_hashes.get(field):
            blockers.append(f"missing_input_hash:{field}")
        elif left_hashes[field] != right_hashes[field]:
            blockers.append(f"input_hash_mismatch:{field}")

    left_budget = left_manifest.get("resource_budget")
    right_budget = right_manifest.get("resource_budget")
    if not isinstance(left_budget, dict) or not isinstance(right_budget, dict):
        blockers.append("missing_resource_budget")
    else:
        for field in COMMON_BUDGET_FIELDS:
            if field not in left_budget or field not in right_budget:
                blockers.append(f"missing_common_budget:{field}")
            elif left_budget[field] != right_budget[field]:
                blockers.append(f"common_budget_mismatch:{field}")
        if require_effective_resource_enforcement:
            for label, budget in (("left", left_budget), ("right", right_budget)):
                enforcement = budget.get("execution_enforcement")
                if not isinstance(enforcement, dict):
                    blockers.append(f"{label}_resource_enforcement_missing")
                    continue
                if enforcement.get("cpu_enforced") is not True:
                    blockers.append(f"{label}_cpu_enforcement_not_effective")
                if enforcement.get("memory_enforced") is not True:
                    blockers.append(f"{label}_memory_enforcement_not_effective")
        if require_effective_output_settings:
            provenance = right_manifest.get("tool_asset_provenance")
            if not isinstance(provenance, dict):
                blockers.append("right_effective_config_missing")
            else:
                search = provenance.get("resolved_search")
                post_processing = provenance.get("resolved_post_processing")
                if not isinstance(search, dict) or not isinstance(post_processing, dict):
                    blockers.append("right_effective_config_incomplete")
                else:
                    if str(left_budget.get("depth")) != str(search.get("max_transforms")):
                        blockers.append("effective_depth_mismatch")
                    if str(left_budget.get("max_routes")) != str(post_processing.get("max_routes")):
                        blockers.append("effective_top_k_mismatch")

    left_revision = left_manifest.get("git_commit")
    right_revision = right_manifest.get("git_commit")
    if not left_revision or not right_revision:
        blockers.append("missing_git_revision")
    elif left_revision != right_revision:
        blockers.append("git_revision_mismatch")

    for label, manifest in (("left", left_manifest), ("right", right_manifest)):
        # compare_manifest's current schema uses ``git_worktree``. Retain
        # older spellings for historical artifacts, but do not reject a clean
        # current manifest merely because this checker lagged its producer.
        worktree = (
            manifest.get("git_worktree")
            or manifest.get("worktree")
            or manifest.get("worktree_state")
            or {}
        )
        if require_clean and worktree.get("clean") is not True:
            blockers.append(f"{label}_worktree_not_clean_or_unrecorded")
        if manifest.get("input_files_unchanged_during_run") is not True:
            blockers.append(f"{label}_inputs_changed_or_unrecorded")

    return {
        "schema_version": "renkin-phase55-preflight/1",
        "eligible": not blockers,
        "target_count": len(left_ids),
        "blockers": blockers,
        "common_budget_fields": list(COMMON_BUDGET_FIELDS),
        "left_tool_specific_budget": left_budget,
        "right_tool_specific_budget": right_budget,
        "effective_resource_enforcement_required": require_effective_resource_enforcement,
        "effective_output_settings_required": require_effective_output_settings,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--left-manifest", type=Path, required=True)
    parser.add_argument("--right-manifest", type=Path, required=True)
    parser.add_argument("--left-rows", type=Path, required=True)
    parser.add_argument("--right-rows", type=Path, required=True)
    parser.add_argument(
        "--frozen-cohort",
        type=Path,
        help="require rows to be the deterministic prefix of this frozen cohort",
    )
    parser.add_argument(
        "--require-effective-output-settings",
        action="store_true",
        help="Require effective AiZynthFinder depth/top-k YAML settings to match RENKIN's budget.",
    )
    parser.add_argument(
        "--smoke-size",
        type=int,
        help="number of frozen targets required when --frozen-cohort is used",
    )
    parser.add_argument("--allow-dirty", action="store_true")
    parser.add_argument(
        "--require-effective-resource-enforcement",
        action="store_true",
        help="Require each arm to record effective CPU and memory enforcement; use for a formal run.",
    )
    args = parser.parse_args()
    if (args.frozen_cohort is None) != (args.smoke_size is None):
        parser.error("--frozen-cohort and --smoke-size must be supplied together")
    left_manifest = load_json(args.left_manifest)
    right_manifest = load_json(args.right_manifest)
    left_ids = row_ids(args.left_rows)
    right_ids = row_ids(args.right_rows)
    if args.frozen_cohort:
        result = smoke_preflight(
            args.frozen_cohort,
            left_manifest,
            right_manifest,
            left_ids,
            right_ids,
            smoke_size=args.smoke_size,
            require_clean=not args.allow_dirty,
            require_effective_resource_enforcement=args.require_effective_resource_enforcement,
            require_effective_output_settings=args.require_effective_output_settings,
        )
    else:
        result = preflight(
            left_manifest,
            right_manifest,
            left_ids,
            right_ids,
            require_clean=not args.allow_dirty,
        )
    print(json.dumps(result, ensure_ascii=False, sort_keys=True))
    return 0 if result["eligible"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
