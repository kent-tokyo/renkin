#!/usr/bin/env python3
"""Convert legacy RENKIN/AiZynthFinder rows to the four-tool row schema."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

try:
    from .four_tool_record import FourToolRecord
except ImportError:  # direct script execution
    from four_tool_record import FourToolRecord


def convert_row(row: dict[str, Any]) -> FourToolRecord:
    tool = row["tool"]
    if tool not in {"renkin", "aizynthfinder"}:
        raise ValueError(f"legacy converter only accepts RENKIN/AiZynthFinder, got {tool!r}")
    route_found = row.get("route_found")
    parseable = row.get("reaction_steps_parseable")
    strict = row.get("strict_validated_route_to_configured_stock")
    audit_status = None
    if parseable is True:
        audit_status = "pass" if strict is True else "fail"
    warnings = row.get("adapter_warnings", []) + row.get("common_validation_warnings", [])
    failure_reason = None
    if row.get("run_status") != "completed":
        failure_reason = (warnings[0].get("detail") if warnings else None) or "legacy run did not complete"
    return FourToolRecord(
        target_id=row["target_id"], target_smiles=row["target_smiles"],
        sample_rank=row["sample_rank"], tool=tool,
        arm_id=row["configuration_id"], run_status=row["run_status"],
        route_found=route_found,
        tool_reported_route_count=row.get("tool_reported_route_count"),
        common_route_parseable=parseable,
        strict_route_to_shared_stock=strict,
        common_audit_status=audit_status,
        total_elapsed_ms=row.get("total_elapsed_ms"),
        planning_elapsed_ms=row.get("total_elapsed_ms"),
        peak_rss_bytes=row.get("peak_rss_bytes"),
        rss_measurement_method=row.get("rss_measurement_method"),
        raw_output_sha256=row.get("raw_output_sha256"),
        route_artifact_sha256=row.get("normalized_route_sha256"),
        failure_reason=failure_reason,
        warnings=warnings,
        tool_specific={tool: row.get("tool_specific", {})},
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("legacy_rows")
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    output = []
    with open(args.legacy_rows, encoding="utf-8") as handle:
        for line_number, line in enumerate(handle, 1):
            if not line.strip():
                continue
            try:
                output.append(convert_row(json.loads(line)).to_json_line())
            except (KeyError, TypeError, ValueError, json.JSONDecodeError) as exc:
                raise ValueError(f"{args.legacy_rows}:{line_number}: {exc}") from exc
    Path(args.output).write_text("\n".join(output) + ("\n" if output else ""), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
