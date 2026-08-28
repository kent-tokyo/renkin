#!/usr/bin/env python3
"""
Paired formal gate for the diversity-reserved beam mechanism
(docs/design/diversity-reserved-beam-v0.md, ROADMAP Item 4 stage 5):
`beam_diversity_policy=off` (baseline, today's pure top-K `beam_prune`,
byte-identical to pre-existing behavior) vs `beam_diversity_policy=active
--beam-diversity-slots N` (candidate) on the fixed 100-target sample
(first 100 by `sample_rank` in data/comparison/sample_full_sorted.jsonl),
reusing PR #104's own Round 2G exact CLI config for direct comparability:
`--ring-context-policy conservative --ring-context-sidecar
data/ring_context_metadata_500.json --templates
data/templates_extracted_500.smi --building-blocks
data/comparison/shared_stock/shared_stock.smi --depth 5 --beam-width 100`.

PR #104's own harness script (scripts/phase2g_round2_paired_sweep.py) was
lost as untracked local state before this session could commit it --
rewritten from this repo's own committed findings-doc methodology
(data/l4422_timeout_diagnostics/, data/phase2g_round2_clean_gate/), not
from memory. Default --timeout-s reuses scripts/ring_context_gate.py's own
150.0 default (same depth/beam/template/stock shape, a real value already
proven to work for this exact config, not a fresh guess).

Since --building-blocks is always the fixed 393-compound shared stock and
`find_routes` only ever returns stock-terminal routes, `route_found`
(routes_found > 0) *is* PR #104's own `route_to_configured_stock` metric
here -- no separate stock-membership check needed.

Alternates run order by `sample_rank` parity (even ranks: baseline then
candidate; odd ranks: candidate then baseline), matching PR #104's own
stated reasoning for spreading thermal/order drift across the paired
comparison.

Usage:
    python3 scripts/beam_diversity_formal_gate.py \
        --renkin-binary target/release/renkin \
        --beam-diversity-slots 10 \
        --n-targets 100 \
        --output-rows data/beam_diversity_gate/rows.jsonl \
        --output-summary data/beam_diversity_gate/summary.json

    # Cheap validation run on a handful of targets before committing to
    # the full 100-target gate:
    python3 scripts/beam_diversity_formal_gate.py \
        --renkin-binary target/release/renkin --beam-diversity-slots 10 \
        --n-targets 5 --output-rows /tmp/validate_rows.jsonl \
        --output-summary /tmp/validate_summary.json
"""

import argparse
import json
import subprocess
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_SAMPLE = REPO_ROOT / "data/comparison/sample_full_sorted.jsonl"
DEFAULT_SIDECAR = REPO_ROOT / "data/ring_context_metadata_500.json"
DEFAULT_TEMPLATES = REPO_ROOT / "data/templates_extracted_500.smi"
DEFAULT_BUILDING_BLOCKS = REPO_ROOT / "data/comparison/shared_stock/shared_stock.smi"


def load_targets(sample_path, n):
    rows = []
    with open(sample_path) as f:
        for line in f:
            line = line.strip()
            if line:
                rows.append(json.loads(line))
    rows.sort(key=lambda r: r["sample_rank"])
    return rows[:n]


def run_one(
    binary,
    target_smiles,
    depth,
    beam_width,
    sidecar,
    templates,
    building_blocks,
    beam_diversity_policy,
    beam_diversity_slots,
    timeout_s,
):
    argv = [
        binary,
        "--target",
        target_smiles,
        "--depth",
        str(depth),
        "--beam-width",
        str(beam_width),
        "--max-routes",
        "1",
        "--ring-context-policy",
        "conservative",
        "--ring-context-sidecar",
        str(sidecar),
        "--templates",
        str(templates),
        "--building-blocks",
        str(building_blocks),
        "--format",
        "json",
    ]
    if beam_diversity_policy != "off":
        argv += [
            "--beam-diversity-policy",
            beam_diversity_policy,
            "--beam-diversity-slots",
            str(beam_diversity_slots),
        ]

    t0 = time.monotonic()
    try:
        proc = subprocess.run(argv, capture_output=True, timeout=timeout_s, text=True)
    except subprocess.TimeoutExpired:
        return {"status": "timeout", "elapsed_s": time.monotonic() - t0, "route_found": False}
    elapsed_s = time.monotonic() - t0

    if proc.returncode != 0:
        return {
            "status": "crashed",
            "elapsed_s": elapsed_s,
            "route_found": False,
            "stderr": proc.stderr[-2000:],
        }
    try:
        parsed = json.loads(proc.stdout)
    except json.JSONDecodeError:
        return {
            "status": "invalid_output",
            "elapsed_s": elapsed_s,
            "route_found": False,
            "stdout": proc.stdout[-2000:],
        }
    routes_found = parsed.get("routes_found", 0)
    return {
        "status": "completed",
        "elapsed_s": elapsed_s,
        "route_found": routes_found > 0,
    }


def main():
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument("--renkin-binary", required=True)
    ap.add_argument("--sample", default=str(DEFAULT_SAMPLE))
    ap.add_argument("--sidecar", default=str(DEFAULT_SIDECAR))
    ap.add_argument("--templates", default=str(DEFAULT_TEMPLATES))
    ap.add_argument("--building-blocks", default=str(DEFAULT_BUILDING_BLOCKS))
    ap.add_argument("--depth", type=int, default=5)
    ap.add_argument("--beam-width", type=int, default=100)
    ap.add_argument("--beam-diversity-slots", type=int, required=True)
    ap.add_argument("--n-targets", type=int, default=100)
    ap.add_argument("--timeout-s", type=float, default=150.0)
    ap.add_argument("--output-rows", required=True)
    ap.add_argument("--output-summary", required=True)
    args = ap.parse_args()

    targets = load_targets(args.sample, args.n_targets)
    print(f"Loaded {len(targets)} targets from {args.sample}", flush=True)

    Path(args.output_rows).parent.mkdir(parents=True, exist_ok=True)
    rows = []
    with open(args.output_rows, "w") as out:
        for i, t in enumerate(targets):
            if i % 10 == 0:
                print(f"  {i}/{len(targets)}...", flush=True)
            arms = ["baseline", "candidate"]
            if t["sample_rank"] % 2 == 1:
                arms = list(reversed(arms))

            per_arm = {}
            for arm in arms:
                policy = "off" if arm == "baseline" else "active"
                result = run_one(
                    args.renkin_binary,
                    t["canonical_smiles"],
                    args.depth,
                    args.beam_width,
                    args.sidecar,
                    args.templates,
                    args.building_blocks,
                    policy,
                    args.beam_diversity_slots,
                    args.timeout_s,
                )
                per_arm[arm] = result

            row = {
                "target_id": t["target_id"],
                "sample_rank": t["sample_rank"],
                "run_order": arms,
                "baseline": per_arm["baseline"],
                "candidate": per_arm["candidate"],
            }
            rows.append(row)
            out.write(json.dumps(row) + "\n")
            out.flush()

    baseline_solved = sum(1 for r in rows if r["baseline"]["route_found"])
    candidate_solved = sum(1 for r in rows if r["candidate"]["route_found"])
    invalid = sum(
        1
        for r in rows
        for arm in ("baseline", "candidate")
        if r[arm]["status"] in ("crashed", "invalid_output")
    )
    baseline_timeouts = sum(1 for r in rows if r["baseline"]["status"] == "timeout")
    candidate_timeouts = sum(1 for r in rows if r["candidate"]["status"] == "timeout")
    regressions = [
        r["target_id"]
        for r in rows
        if r["baseline"]["route_found"] and not r["candidate"]["route_found"]
    ]
    new_solves = [
        r["target_id"]
        for r in rows
        if r["candidate"]["route_found"] and not r["baseline"]["route_found"]
    ]

    def percentile(values, p):
        if not values:
            return None
        s = sorted(values)
        idx = min(len(s) - 1, int(round(p * (len(s) - 1))))
        return s[idx]

    baseline_completed_times = [
        r["baseline"]["elapsed_s"] for r in rows if r["baseline"]["status"] == "completed"
    ]
    candidate_completed_times = [
        r["candidate"]["elapsed_s"] for r in rows if r["candidate"]["status"] == "completed"
    ]

    summary = {
        "n_targets": len(rows),
        "beam_diversity_slots": args.beam_diversity_slots,
        "beam_width": args.beam_width,
        "baseline_solved": baseline_solved,
        "candidate_solved": candidate_solved,
        "coverage_delta_pp": (
            (candidate_solved - baseline_solved) / len(rows) * 100 if rows else None
        ),
        "invalid_count": invalid,
        "baseline_timeouts": baseline_timeouts,
        "candidate_timeouts": candidate_timeouts,
        "regressions": regressions,
        "regression_count": len(regressions),
        "new_solves": new_solves,
        "new_solve_count": len(new_solves),
        "baseline_p95_s": percentile(baseline_completed_times, 0.95),
        "candidate_p95_s": percentile(candidate_completed_times, 0.95),
    }
    with open(args.output_summary, "w") as f:
        json.dump(summary, f, indent=2, sort_keys=True)
    print(json.dumps(summary, indent=2, sort_keys=True))
    print(
        f"Wrote {len(rows)} rows to {args.output_rows}, summary to {args.output_summary}",
        flush=True,
    )


if __name__ == "__main__":
    main()
