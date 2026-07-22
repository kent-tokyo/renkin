#!/usr/bin/env python3
"""Aggregate renkin-bench chunk JSON files into headline USPTO-50k metrics.

Reads every *.json file produced by scripts/run_benchmark_chunks.sh (invoked
directly, or fanned out across shards by scripts/run_benchmark_parallel.sh)
and recomputes the census-wide numbers from the per-target `results[]`
records — the correct approach, since averaging 50 chunk-level percentages
would mis-weight uneven chunk sizes (see the Phase-31 corrected-baseline
run-doc for the methodology this mirrors).

`provenance_validated_solved_rate` is Phase 31's primary metric candidate:
solved AND atom_balance_ok AND route_validation_status == "validated"
(i.e. every step is atom-balanced AND confirmed by its own originating
rule's validator, not a coincidental cross-rule match), as a fraction of
ALL targets. `raw_solved_rate` alone is deliberately not treated as RENKIN's
representative performance number.

Usage:
    python3 scripts/aggregate_bench_results.py <out_dir> [--expected-total N]

<out_dir> is the directory passed as OUT_DIR to run_benchmark_parallel.sh /
run_benchmark_chunks.sh (e.g. data/bench_chunks_corrected_baseline). Chunk
files are discovered via <out_dir>/**/*.json (works for both the sharded
shard_N/chunk_*.json layout and a flat chunk_*.json layout).

Hard invariants — abort (nonzero exit), not warn-and-continue, on violation.
A silently-dropped or double-counted chunk would corrupt every metric below
it, so these are load-bearing, not optional:
  - every discovered .json file must parse and have an integer "total"
  - number of results[] records collected must equal sum(chunk["total"])
  - if --expected-total is given, the collected record count must match it

What this script CANNOT reconstruct exactly, and why (see full report):
  - validation_coverage / evaluable_validation_pass_rate: renkin-bench's
    BenchReport only serializes the per-chunk *ratio*, not the underlying
    step counts (steps_checked/steps_evaluable/steps_valid), so a properly
    weighted global ratio can't be derived from committed JSON output alone.
    This script reports the per-chunk range so a non-uniform (and therefore
    silently-misleading-if-averaged) result is visible rather than hidden.
  - rule-usage / per-step validation breakdown: results[] carries only
    route-level rollups (route_validation_status), never a step list.
"""
import argparse
import glob
import json
import os
import sys


def die(msg: str) -> None:
    print(f"FATAL: {msg}", file=sys.stderr)
    sys.exit(1)


def percentile(sorted_vals, p):
    """Linear-interpolation percentile (p in [0, 100]) over an already-sorted
    list — matches numpy.percentile's default 'linear' method, which is the
    method that reproduces tasks/phase31_corrected_baseline_run.md's reported
    p95/p99 exactly (nearest-rank does not, verified empirically)."""
    if not sorted_vals:
        return None
    idx = p / 100.0 * (len(sorted_vals) - 1)
    lo = int(idx)
    hi = min(lo + 1, len(sorted_vals) - 1)
    frac = idx - lo
    return sorted_vals[lo] + (sorted_vals[hi] - sorted_vals[lo]) * frac


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("out_dir")
    ap.add_argument("--expected-total", type=int, default=None)
    args = ap.parse_args()

    files = sorted(glob.glob(os.path.join(args.out_dir, "**", "*.json"), recursive=True))
    if not files:
        die(f"no .json chunk files found under {args.out_dir}")

    all_results = []
    sum_reported_total = 0
    chunk_coverage = []
    chunk_eval_pass = []

    for f in files:
        try:
            with open(f) as fh:
                d = json.load(fh)
        except (json.JSONDecodeError, OSError) as e:
            die(f"{f}: unreadable/invalid JSON ({e}) — a corrupted chunk must be "
                f"rerun, not silently excluded from the aggregate")
        if not isinstance(d.get("total"), int):
            die(f"{f}: missing/non-integer 'total' field")
        results = d.get("results")
        if not isinstance(results, list) or len(results) != d["total"]:
            die(f"{f}: results[] length ({len(results) if isinstance(results, list) else 'missing'}) "
                f"!= declared total ({d['total']})")
        sum_reported_total += d["total"]
        all_results.extend(results)
        if d.get("validation_coverage") is not None:
            chunk_coverage.append(d["validation_coverage"])
        if d.get("evaluable_validation_pass_rate") is not None:
            chunk_eval_pass.append(d["evaluable_validation_pass_rate"])

    n = len(all_results)
    if n != sum_reported_total:
        die(f"collected {n} results[] records but chunk 'total' fields sum to {sum_reported_total}")
    if args.expected_total is not None and n != args.expected_total:
        die(f"collected {n} records, expected {args.expected_total} (--expected-total)")

    solved = [r for r in all_results if r.get("solved")]
    n_solved = len(solved)

    depth0 = [r for r in solved if r.get("best_depth") == 0]
    balanced = [r for r in solved if r.get("atom_balance_ok") is True]
    provenance_validated = [
        r
        for r in solved
        if r.get("atom_balance_ok") is True
        and r.get("route_validation_status") == "validated"
    ]

    status_counts = {}
    for r in solved:
        s = r.get("route_validation_status")
        status_counts[s] = status_counts.get(s, 0) + 1

    depth_dist = {}
    for r in solved:
        d = r.get("best_depth")
        depth_dist[d] = depth_dist.get(d, 0) + 1

    times_all = sorted(r["time_ms"] for r in all_results if "time_ms" in r)
    times_solved = sorted(r["time_ms"] for r in solved if "time_ms" in r)

    def latency_block(vals):
        if not vals:
            return None
        return {
            "n": len(vals),
            "p50": percentile(vals, 50),
            "p95": percentile(vals, 95),
            "p99": percentile(vals, 99),
            "max": vals[-1],
            "mean": sum(vals) / len(vals),
        }

    out = {
        "files_aggregated": len(files),
        "total": n,
        "solved": n_solved,
        "raw_solved_rate": n_solved / n if n else None,
        "depth0_direct_stock_hit_rate": len(depth0) / n if n else None,
        # Nested series, all three over the SAME denominator (total targets),
        # so they're directly comparable: raw >= atom_balanced >= provenance_validated.
        "atom_balanced_solved_rate": len(balanced) / n if n else None,
        "provenance_validated_solved_rate": len(provenance_validated) / n if n else None,
        # Diagnostic only (different denominator: of solved, not of total) -- do not
        # compare this directly to the two _solved_rate fields above.
        "pct_atom_balanced_of_solved": (len(balanced) / n_solved) if n_solved else None,
        "route_validation_status_of_solved": status_counts,
        "depth_distribution_of_solved": {str(k): v for k, v in sorted(depth_dist.items(), key=lambda kv: (kv[0] is None, kv[0]))},
        "latency_ms_all_targets": latency_block(times_all),
        "latency_ms_solved_only": latency_block(times_solved),
        "validation_coverage_per_chunk_range": [min(chunk_coverage), max(chunk_coverage)] if chunk_coverage else None,
        "evaluable_validation_pass_rate_per_chunk_range": [min(chunk_eval_pass), max(chunk_eval_pass)] if chunk_eval_pass else None,
        "note": (
            "validation_coverage / evaluable_validation_pass_rate ranges above are the "
            "min/max of each chunk's own ratio — NOT a properly weighted global figure. "
            "Exact reconstruction needs steps_checked/steps_evaluable/steps_valid, which "
            "BenchReport does not currently serialize. If the range collapses to a single "
            "value, any weighting gives that value; otherwise treat as reported-not-derived."
        ),
    }
    print(json.dumps(out, indent=2, sort_keys=False))


if __name__ == "__main__":
    main()
