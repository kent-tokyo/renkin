"""Post-arm integrity verification for the Issue #66 500-target comparison.

Checks the invariants a completed arm must satisfy before it's trusted as a
headline result: exact target coverage, no duplicates, terminal-status
accounting, route_found implies a route hash, zero schema/JSON errors, and
that the manifest's recorded input-file hashes still match what's on disk
now. Reports problems; does not decide whether they're fatal -- that's a
human (or a stop-condition rule) reading the output.

Usage:
    .venv-compare-66/bin/python scripts/compare_verify_arm.py \
        --rows data/comparison/results_500/renkin_conservative_shared_stock.jsonl \
        --manifest data/comparison/results_500/renkin_conservative_shared_stock.manifest.json \
        --sample-list data/comparison/sample_full_sorted.jsonl \
        --sample-size 500
"""

from __future__ import annotations

import argparse
import json
import sys

import compare_sampling as sampling
import compare_schema as schema


def verify_arm(rows_path: str, manifest_path: str | None, sample_list: str, sample_size: int) -> dict:
    problems: list[str] = []

    expected = sampling.load_sample(sample_list, sample_size)
    expected_ids = [row["target_id"] for row in expected]
    expected_id_set = set(expected_ids)
    if len(expected_id_set) != len(expected_ids):
        problems.append("sample list itself contains duplicate target_id values")

    malformed_json_lines = 0
    raw_lines = []
    with open(rows_path, "r", encoding="utf-8") as f:
        for line_no, line in enumerate(f, start=1):
            line = line.strip()
            if not line:
                continue
            try:
                json.loads(line)
                raw_lines.append(line)
            except json.JSONDecodeError:
                malformed_json_lines += 1
                problems.append(f"line {line_no}: malformed JSON")

    try:
        rows = schema.load_rows(rows_path)
    except Exception as e:
        problems.append(f"schema.load_rows failed: {e}")
        rows = []

    actual_ids = [r.target_id for r in rows]
    actual_id_set = set(actual_ids)
    duplicate_ids = [tid for tid in actual_id_set if actual_ids.count(tid) > 1]
    if duplicate_ids:
        problems.append(f"duplicate target_ids in output: {sorted(duplicate_ids)}")

    missing = expected_id_set - actual_id_set
    unexpected = actual_id_set - expected_id_set
    if missing:
        problems.append(f"{len(missing)} expected targets missing from output: {sorted(missing)[:20]}")
    if unexpected:
        problems.append(f"{len(unexpected)} unexpected targets in output not in sample: {sorted(unexpected)[:20]}")

    if len(rows) != sample_size:
        problems.append(f"expected exactly {sample_size} rows, found {len(rows)}")

    status_counts: dict[str, int] = {}
    for r in rows:
        status_counts[r.run_status] = status_counts.get(r.run_status, 0) + 1
    completed = status_counts.get("completed", 0)
    timeout = status_counts.get("timeout", 0)
    error = sum(status_counts.get(s, 0) for s in ("crashed", "invalid_input", "setup_error"))
    if completed + timeout + error != len(rows):
        problems.append(
            f"completed({completed}) + timeout({timeout}) + error({error}) "
            f"!= total rows ({len(rows)}); status_counts={status_counts}"
        )

    route_found_missing_hash = [
        r.target_id for r in rows if r.route_found is True and not r.normalized_route_sha256
    ]
    if route_found_missing_hash:
        problems.append(
            f"{len(route_found_missing_hash)} rows have route_found=true but no "
            f"normalized_route_sha256: {route_found_missing_hash[:20]}"
        )

    nullability_problems = []
    for r in rows:
        for p in schema.validate_row_nullability(r):
            nullability_problems.append(f"{r.target_id}: {p}")
    if nullability_problems:
        problems.extend(nullability_problems[:20])
        if len(nullability_problems) > 20:
            problems.append(f"... and {len(nullability_problems) - 20} more nullability problems")

    crash_or_adapter_failures = [
        {"target_id": r.target_id, "run_status": r.run_status, "adapter_warnings": r.adapter_warnings}
        for r in rows
        if r.run_status in ("crashed", "invalid_input", "setup_error") or r.adapter_warnings
    ]

    manifest_check = None
    if manifest_path:
        with open(manifest_path, "r", encoding="utf-8") as f:
            run_manifest = json.load(f)
        manifest_check = {
            "input_files_unchanged_during_run": run_manifest.get("input_files_unchanged_during_run"),
            "end_time_unix_set": run_manifest.get("end_time_unix") is not None,
        }
        if run_manifest.get("input_files_unchanged_during_run") is False:
            problems.append("manifest reports input files changed during the run")
        if run_manifest.get("end_time_unix") is None:
            problems.append("manifest has no end_time_unix -- arm may not have completed cleanly")

    return {
        "rows_path": rows_path,
        "expected_count": sample_size,
        "actual_count": len(rows),
        "status_counts": status_counts,
        "malformed_json_lines": malformed_json_lines,
        "route_found_missing_hash_count": len(route_found_missing_hash),
        "crash_or_adapter_failures": crash_or_adapter_failures,
        "manifest_check": manifest_check,
        "problems": problems,
        "PASS": len(problems) == 0,
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rows", required=True)
    parser.add_argument("--manifest", default=None)
    parser.add_argument("--sample-list", default="data/comparison/sample_full_sorted.jsonl")
    parser.add_argument("--sample-size", type=int, default=500)
    args = parser.parse_args(argv)

    result = verify_arm(args.rows, args.manifest, args.sample_list, args.sample_size)
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0 if result["PASS"] else 1


if __name__ == "__main__":
    sys.exit(main())
