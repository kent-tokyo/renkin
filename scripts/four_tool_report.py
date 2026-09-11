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
import random
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


def percentile(values: list[float], quantile: float) -> float | None:
    """Return the nearest-rank percentile with a documented deterministic rule."""
    if not values:
        return None
    if not 0 <= quantile <= 1:
        raise ValueError("quantile must be between 0 and 1")
    ordered = sorted(values)
    rank = max(1, math.ceil(quantile * len(ordered)))
    return ordered[rank - 1]


def _paired_bootstrap_ci(values: list[int], seed: int = 0, draws: int = 10_000) -> list[float] | None:
    """Return a deterministic percentile CI for a paired difference sample."""
    if not values:
        return None
    rng = random.Random(seed)
    n = len(values)
    estimates = [sum(values[rng.randrange(n)] for _ in range(n)) / n for _ in range(draws)]
    estimates.sort()
    return [estimates[int(0.025 * (draws - 1))], estimates[int(0.975 * (draws - 1))]]


def _exact_mcnemar_p(discordant_a: int, discordant_b: int) -> float:
    """Return a two-sided exact McNemar p-value."""
    discordant = discordant_a + discordant_b
    if discordant == 0:
        return 1.0
    smaller = min(discordant_a, discordant_b)
    tail = sum(math.comb(discordant, k) for k in range(smaller + 1)) / (2 ** discordant)
    return min(1.0, 2 * tail)


def _paired_comparison(left: list[FourToolRecord], right: list[FourToolRecord]) -> dict[str, object]:
    left_by_target = {row.target_id: row for row in left}
    right_by_target = {row.target_id: row for row in right}
    paired = [
        (left_by_target[target_id].strict_route_to_shared_stock,
         right_by_target[target_id].strict_route_to_shared_stock)
        for target_id in sorted(left_by_target.keys() & right_by_target.keys())
        if left_by_target[target_id].strict_route_to_shared_stock is not None
        and right_by_target[target_id].strict_route_to_shared_stock is not None
    ]
    left_wins = sum(a and not b for a, b in paired)
    right_wins = sum(not a and b for a, b in paired)
    differences = [int(a) - int(b) for a, b in paired]
    return {
        "paired_trials": len(paired),
        "left_wins": left_wins,
        "right_wins": right_wins,
        "ties": len(paired) - left_wins - right_wins,
        "difference_left_minus_right": sum(differences) / len(differences) if differences else None,
        "paired_bootstrap_95": _paired_bootstrap_ci(differences),
        "mcnemar_exact_two_sided_p": _exact_mcnemar_p(left_wins, right_wins),
    }


def summarize(records: list[FourToolRecord]) -> dict[str, object]:
    groups: dict[tuple[str, str], list[FourToolRecord]] = defaultdict(list)
    for record in records:
        groups[(record.tool, record.arm_id)].append(record)
    summaries = []
    for (tool, arm_id), rows in sorted(groups.items()):
        planning = [row.planning_elapsed_ms for row in rows if row.planning_elapsed_ms is not None]
        rss = [row.peak_rss_bytes for row in rows if row.peak_rss_bytes is not None]
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
                "p95": percentile(planning, 0.95),
            },
            "peak_rss_bytes": {
                "n": len(rss),
                "max": max(rss) if rss else None,
                "measurement_methods": sorted({
                    row.rss_measurement_method for row in rows
                    if row.peak_rss_bytes is not None and row.rss_measurement_method
                }),
            },
            "resource_enforcement": sorted({
                str(row.tool_specific.get(tool, {}).get("resource_enforcement"))
                for row in rows
                if row.tool_specific.get(tool, {}).get("resource_enforcement") is not None
            }),
        })
    comparisons = {}
    for index, left in enumerate(summaries):
        left_rows = groups[(left["tool"], left["arm_id"])]
        for right in summaries[index + 1:]:
            right_rows = groups[(right["tool"], right["arm_id"])]
            key = f"{left['tool']}:{left['arm_id']}__vs__{right['tool']}:{right['arm_id']}"
            comparisons[key] = {
                "left": {"tool": left["tool"], "arm_id": left["arm_id"]},
                "right": {"tool": right["tool"], "arm_id": right["arm_id"]},
                **_paired_comparison(left_rows, right_rows),
            }
    return {
        "schema_version": "renkin-four-tool-report/1",
        "n_records": len(records),
        "groups": summaries,
        "paired_comparisons": comparisons,
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
        "| Tool / arm | Rows | Native route found | Strict shared-stock pass | Planning p50 / p95 (ms) | Peak RSS (bytes; method) | Resource enforcement |",
        "|---|---:|---|---|---:|---|",
    ]
    for group in payload["groups"]:
        enforcement = ", ".join(group["resource_enforcement"]) or "not_measured"
        planning = group["planning_elapsed_ms"]
        latency = ("not_measured" if planning["median"] is None else
                   f"{planning['median']} / {planning['p95']}")
        rss = group["peak_rss_bytes"]
        rss_text = "not_measured" if rss["max"] is None else str(rss["max"])
        if rss["measurement_methods"]:
            rss_text += "; " + ", ".join(rss["measurement_methods"])
        lines.append(
            f"| `{group['tool']}` / `{group['arm_id']}` | {group['n_rows']} | "
            f"{_rate_text(group['native_route_found'])} | "
            f"{_rate_text(group['strict_route_to_shared_stock'])} | "
            f"{latency} | {rss_text} | {enforcement} |"
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
        "## Paired strict-endpoint comparisons",
        "",
        "Each comparison uses targets evaluable by both arms. `difference` is left minus right; p-values are two-sided exact McNemar tests.",
        "",
        "| Pair | Paired n | Left wins | Right wins | Difference | Bootstrap 95% CI | McNemar p |",
        "|---|---:|---:|---:|---:|---|---:|",
    ]
    for key, comparison in payload["paired_comparisons"].items():
        ci = comparison["paired_bootstrap_95"]
        ci_text = "not_measured" if ci is None else f"{ci[0]:.3f} to {ci[1]:.3f}"
        difference = comparison["difference_left_minus_right"]
        lines.append(
            f"| `{key}` | {comparison['paired_trials']} | {comparison['left_wins']} | "
            f"{comparison['right_wins']} | "
            f"{difference if difference is not None else 'not_measured'} | {ci_text} | "
            f"{comparison['mcnemar_exact_two_sided_p']:.4g} |"
        )
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
