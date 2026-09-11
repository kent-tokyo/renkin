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
        })
    return {
        "schema_version": "renkin-four-tool-report/1",
        "n_records": len(records),
        "groups": summaries,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("records")
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    payload = summarize(load_records(args.records))
    Path(args.output).write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(json.dumps(payload, indent=2, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
