#!/usr/bin/env python3
"""Bucket unsolved-target failures into bottleneck categories using ONLY the
per-target fields already recorded by renkin-bench (no re-run needed).

This is Phase 32 merge-order step 4 ("bottleneck decomposition tooling"),
per tasks/phase32_matched_condition_goal.md. It must run before any
stock/template/NN/beam scaling decision — see that doc's "Forbidden" list
("skipping root-cause analysis in favor of only a full run").

Categories (best-effort from aggregate flags; see LIMITATIONS below):
  - template_limited: matched_templates == 0 (no rule fired on this target
    at all, at any depth reached)
  - stock_limited: templates fired but the search never touched a stock
    molecule (stock_hits == 0), and it wasn't merely cut short by budget
  - search_limited: beam_limit_hit or max_depth_reached, with templates
    AND stock both present in the frontier (plausible route existed,
    budget ran out first)
  - budget_cut_no_stock: budget cut before any stock hit was ever recorded
    (ambiguous: could still be genuinely stock-limited, just never got
    deep/wide enough to find out)
  - exhausted_no_route: search completed within budget (no beam/depth
    cut), had templates and stock hits, yet still produced no route —
    suggests an algorithmic/connectivity gap rather than a budget one

LIMITATIONS: retrieval-limited (right template exists but is ranked
outside the NN scorer's top-K), representation-limited, chemistry-limited,
and validator-limited failures cannot be distinguished from these
aggregate fields alone — results[] carries route-level rollups only, never
a step list (see scripts/aggregate_bench_results.py's docstring). Those
require per-step inspection (e.g. examples/inspect_validation.rs) on a
sample, which this script does not do.

Usage:
    python3 scripts/decompose_bottlenecks.py <out_dir>
"""
import argparse
import glob
import json
import os
import sys
from collections import Counter


def die(msg: str) -> None:
    print(f"FATAL: {msg}", file=sys.stderr)
    sys.exit(1)


def load_results(out_dir):
    files = sorted(glob.glob(os.path.join(out_dir, "**", "*.json"), recursive=True))
    if not files:
        die(f"no .json chunk files found under {out_dir}")
    all_results = []
    for f in files:
        with open(f) as fh:
            d = json.load(fh)
        all_results.extend(d["results"])
    return all_results


def bucket(r):
    mt = r.get("matched_templates") or 0
    sh = r.get("stock_hits") or 0
    cut = bool(r.get("beam_limit_hit")) or bool(r.get("max_depth_reached"))

    if mt == 0:
        return "template_limited"
    if sh == 0 and cut:
        return "budget_cut_no_stock"
    if sh == 0 and not cut:
        return "stock_limited"
    if sh > 0 and cut:
        return "search_limited"
    return "exhausted_no_route"


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("out_dir")
    args = ap.parse_args()

    all_results = load_results(args.out_dir)
    solved = [r for r in all_results if r.get("solved")]
    unsolved = [r for r in all_results if not r.get("solved")]

    buckets = Counter(bucket(r) for r in unsolved)

    # Discriminating-power sanity check: if beam/depth flags are ~always
    # true for SOLVED targets too, they carry no signal and this whole
    # decomposition is an artifact, not a finding.
    solved_cut_rate = sum(1 for r in solved if r.get("max_depth_reached")) / len(solved) if solved else None
    unsolved_cut_rate = sum(1 for r in unsolved if r.get("max_depth_reached")) / len(unsolved) if unsolved else None

    out = {
        "total": len(all_results),
        "solved": len(solved),
        "unsolved": len(unsolved),
        "unsolved_bucket_counts": dict(buckets),
        "unsolved_bucket_fractions": {k: v / len(unsolved) for k, v in buckets.items()} if unsolved else {},
        "max_depth_reached_rate_solved": solved_cut_rate,
        "max_depth_reached_rate_unsolved": unsolved_cut_rate,
        "discriminating": (
            unsolved_cut_rate is not None
            and solved_cut_rate is not None
            and unsolved_cut_rate - solved_cut_rate > 0.3
        ),
    }
    print(json.dumps(out, indent=2))


if __name__ == "__main__":
    main()
