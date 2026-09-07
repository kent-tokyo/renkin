#!/usr/bin/env python3
"""Summarize per-target batch search cost and recovery-stage pressure."""

from __future__ import annotations

import argparse
import json
from collections import Counter
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--rows", required=True)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    rows = [json.loads(line) for line in open(args.rows, encoding="utf-8") if line.strip()]
    stage_counts: Counter[str] = Counter()
    termination_counts: Counter[str] = Counter()
    cost_buckets: Counter[str] = Counter()
    for row in rows:
        audit = row.get("tool_specific", {}).get("renkin", {}).get("recovery_audit") or {}
        stage_counts[audit.get("selected_stage", "not_recovery")] += 1
        for attempt in audit.get("attempts", []):
            termination_counts[attempt.get("termination", "unknown")] += 1
        elapsed = row.get("total_elapsed_ms") or 0
        bucket = "lt_1s" if elapsed < 1000 else "1_to_5s" if elapsed < 5000 else "ge_5s"
        cost_buckets[bucket] += 1
    report = {
        "schema_version": "renkin-batch-cost/1",
        "rows": len(rows),
        "selected_stage_counts": dict(sorted(stage_counts.items())),
        "attempt_termination_counts": dict(sorted(termination_counts.items())),
        "elapsed_buckets": dict(sorted(cost_buckets.items())),
        "interpretation": "Use stage and termination pressure to target bounded recovery changes; this report does not change success metrics.",
    }
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
