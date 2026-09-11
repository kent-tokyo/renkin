#!/usr/bin/env python3
"""Summarize ``renkin-four-tool-row/1`` records for a benchmark report.

No popularity or repository metadata is included.  The report is deliberately
about measured outcomes and denominators, with unavailable measurements kept
out of the corresponding rate rather than treated as failures.
"""

from __future__ import annotations

import argparse
import json
import math
from collections import Counter, defaultdict
from pathlib import Path
from statistics import median

from four_tool_record import FourToolRecord, load_records


def wilson_interval(successes: int, trials: int, z: float = 1.959963984540054) -> list[float] | None:
    if trials == 0:
        return None
    p = successes / trials
    denominator = 1 + z * z / trials
    centre = (p + z * z / (2 * trials)) / denominator
    margin = z * math.sqrt(p * (1 - p) / trials + z * z / (4 * trials * trials)) / denominator
    return [max(0.0, centre - margin), min(1.0, centre + margin)]


def _rate(values: list[bool | None]) -> dict[str, object]:
    measured = [value for value in values if value is not None]
    successes = sum(measured)
    return {
        "successes": successes,
        "trials": len(measured),
        "rate": successes / len(measured) if measured else None,
        "wilson_95": wilson_interval(successes, len(measured)),
    }


def summarize(records: list[FourToolRecord]) -> dict[str, object]:
    groups: dict[tuple[str, str], list[FourToolRecord]] = defaultdict(list)
    for record in records:
        groups[(record.tool, record.arm_id)].append(record)
    summaries = []
    for (tool, arm_id), rows in sorted(groups.items()):
        planning = [row.planning_elapsed_ms for row in rows if row.planning_elapsed_ms is not None]
        summaries.append({
            "tool": tool,
            "arm_id": arm_id,
            "n_rows": len(rows),
            "run_status": dict(sorted(Counter(row.run_status for row in rows).items())),
            "native_route_found": _rate([row.route_found for row in rows]),
            "strict_route_to_shared_stock": _rate(
                [row.strict_route_to_shared_stock for row in rows]
            ),
            "common_audit_status": dict(
                sorted(Counter(row.common_audit_status or "not_measured" for row in rows).items())
            ),
            "planning_elapsed_ms": {
                "n": len(planning),
                "median": median(planning) if planning else None,
            },
            "resource_enforcement": sorted({
                str(row.tool_specific.get(tool, {}).get("resource_enforcement"))
                for row in rows
                if row.tool_specific.get(tool, {}).get("resource_enforcement") is not None
            }),
        })
    return {
        "schema_version": "renkin-four-tool-report/1",
        "n_records": len(records),
        "groups": summaries,
    }


def _rate_text(value: dict[str, object]) -> str:
    rate = value["rate"]
    if rate is None:
        return "not_measured"
    interval = value["wilson_95"]
    return f"{value['successes']}/{value['trials']} ({float(rate):.3f}; 95% CI {interval[0]:.3f}-{interval[1]:.3f})"


def render_markdown(payload: dict[str, object]) -> str:
    lines = [
        "# Four-tool benchmark report",
        "",
        "Generated from `renkin-four-tool-row/1` records. This report contains measured outcomes only; `not_measured` is not a failure.",
        "",
        f"- Records: {payload['n_records']}",
        "- Primary endpoint: rank-1 `strict_route_to_shared_stock`",
        "- Popularity metrics: not included",
        "",
        "## Summary",
        "",
        "| Tool / arm | Rows | Native route found | Strict shared-stock pass | Planning median (ms) | Resource enforcement |",
        "|---|---:|---|---|---:|---|",
    ]
    for group in payload["groups"]:
        enforcement = ", ".join(group["resource_enforcement"]) or "not_measured"
        median_ms = group["planning_elapsed_ms"]["median"]
        lines.append(
            f"| `{group['tool']}` / `{group['arm_id']}` | {group['n_rows']} | "
            f"{_rate_text(group['native_route_found'])} | "
            f"{_rate_text(group['strict_route_to_shared_stock'])} | "
            f"{median_ms if median_ms is not None else 'not_measured'} | {enforcement} |"
        )
    lines += [
        "",
        "## Run-status accounting",
        "",
        "| Tool / arm | Status counts | Common audit statuses |",
        "|---|---|---|",
    ]
    for group in payload["groups"]:
        statuses = ", ".join(f"{key}={value}" for key, value in group["run_status"].items())
        audits = ", ".join(f"{key}={value}" for key, value in group["common_audit_status"].items())
        lines.append(f"| `{group['tool']}` / `{group['arm_id']}` | {statuses} | {audits} |")
    lines += [
        "",
        "## Interpretation boundary",
        "",
        "These rates are descriptive for the supplied target cohort. They do not establish experimental yield, universal chemical correctness, or superiority outside the declared protocol.",
        "",
    ]
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("records")
    parser.add_argument("--output", required=True)
    parser.add_argument("--markdown-output")
    args = parser.parse_args()
    payload = summarize(load_records(args.records))
    Path(args.output).write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    if args.markdown_output:
        Path(args.markdown_output).write_text(render_markdown(payload), encoding="utf-8")
    print(json.dumps(payload, indent=2, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
