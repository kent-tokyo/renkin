#!/usr/bin/env python3
"""Validate the declarative four-tool benchmark configuration registry.

This checks protocol/configuration shape only. It does not claim that an arm
is runnable; use ``--check-artifacts`` for a local availability check.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

EXPECTED_SCHEMA = "renkin-four-tool-benchmark-config/1"
EXPECTED_TOOLS = {"renkin", "aizynthfinder", "syntheseus", "synplanner"}
VALID_STATUSES = {"candidate", "verified", "not_measured", "excluded"}
REQUIRED_COMMON = {
    "target_source",
    "pilot_target_count",
    "formal_target_count",
    "external_timeout_s",
    "termination_grace_s",
    "cpus",
    "memory_limit",
    "comparison_modes",
    "shared_stock",
    "pairing_key",
    "common_audit",
    "statistics",
}
REQUIRED_ARM = {
    "arm_id",
    "tool",
    "status",
    "version",
    "source_ref",
    "search_algorithm",
    "model_or_templates",
    "stock",
    "parameters",
    "runner",
    "runtime",
    "artifact_paths",
}


def validate_registry(
    payload: object, root: Path, check_artifacts: bool = False, formal: bool = False
) -> list[str]:
    problems: list[str] = []
    if not isinstance(payload, dict):
        return ["registry must be a JSON object"]
    if payload.get("schema_version") != EXPECTED_SCHEMA:
        problems.append(f"schema_version must be {EXPECTED_SCHEMA!r}")
    common = payload.get("common")
    if not isinstance(common, dict):
        problems.append("common must be an object")
    else:
        problems.extend(f"common missing {key!r}" for key in sorted(REQUIRED_COMMON - common.keys()))
        if common.get("pairing_key") != "target_id":
            problems.append("common.pairing_key must be 'target_id'")
        if common.get("comparison_modes") != ["native", "shared_stock"]:
            problems.append("common.comparison_modes must be ['native', 'shared_stock']")

    arms = payload.get("arms")
    if not isinstance(arms, list):
        return problems + ["arms must be an array"]
    seen_ids: set[str] = set()
    seen_tools: set[str] = set()
    for index, arm in enumerate(arms):
        prefix = f"arms[{index}]"
        if not isinstance(arm, dict):
            problems.append(f"{prefix} must be an object")
            continue
        problems.extend(f"{prefix} missing {key!r}" for key in sorted(REQUIRED_ARM - arm.keys()))
        arm_id = arm.get("arm_id")
        if not isinstance(arm_id, str) or not arm_id:
            problems.append(f"{prefix}.arm_id must be a non-empty string")
        elif arm_id in seen_ids:
            problems.append(f"duplicate arm_id {arm_id!r}")
        else:
            seen_ids.add(arm_id)
        tool = arm.get("tool")
        if tool not in EXPECTED_TOOLS:
            problems.append(f"{prefix}.tool must be one of {sorted(EXPECTED_TOOLS)}")
        elif tool in seen_tools:
            problems.append(f"duplicate tool arm {tool!r}")
        else:
            seen_tools.add(tool)
        if arm.get("status") not in VALID_STATUSES:
            problems.append(f"{prefix}.status must be one of {sorted(VALID_STATUSES)}")
        if not isinstance(arm.get("version"), str) or not arm.get("version"):
            problems.append(f"{prefix}.version must be a non-empty string")
        if not isinstance(arm.get("parameters"), dict):
            problems.append(f"{prefix}.parameters must be an object")
        paths = arm.get("artifact_paths")
        if not isinstance(paths, list) or not paths or not all(isinstance(path, str) and path for path in paths):
            problems.append(f"{prefix}.artifact_paths must be a non-empty string array")
        elif check_artifacts:
            for path in paths:
                if not (root / path).exists():
                    problems.append(f"{prefix} missing artifact: {path}")
        if arm.get("status") in {"not_measured", "excluded"} and not arm.get("reason"):
            problems.append(f"{prefix} requires reason when status is {arm.get('status')!r}")
        if formal:
            if arm.get("status") != "verified":
                problems.append(f"{prefix} must be 'verified' for a formal run")
            runtime = arm.get("runtime")
            enforcement = runtime.get("resource_enforcement", "") if isinstance(runtime, dict) else ""
            if not isinstance(enforcement, str) or any(
                marker in enforcement.lower()
                for marker in ("pending", "unverified", "unavailable")
            ):
                problems.append(f"{prefix} has no verified formal resource enforcement")

    if seen_tools != EXPECTED_TOOLS:
        problems.append(f"arm tool set must equal {sorted(EXPECTED_TOOLS)}, got {sorted(seen_tools)}")
    return problems


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("registry", type=Path)
    parser.add_argument("--repo-root", type=Path, default=Path("."))
    parser.add_argument("--check-artifacts", action="store_true")
    parser.add_argument("--formal", action="store_true",
                        help="fail unless every arm is verified with a bounded resource runtime")
    args = parser.parse_args()
    payload = json.loads(args.registry.read_text(encoding="utf-8"))
    problems = validate_registry(payload, args.repo_root.resolve(), args.check_artifacts, args.formal)
    if problems:
        for problem in problems:
            print(f"ERROR: {problem}")
        return 1
    print(f"PASS: four-tool registry is structurally valid ({len(payload['arms'])} arms)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
