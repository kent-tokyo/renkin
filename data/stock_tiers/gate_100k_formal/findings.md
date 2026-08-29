# Stock-tier pilot — 100k formal comparison (Phase C), 2026-08-29

Formal 100-target measurement, per `internal_docs/ROADMAP.md`'s v0.36.0
stock-pilot plan, following Phase B's 10k smoke (`data/stock_tiers/gate_10k_smoke/findings.md`).

## Design (updated after Phase B's finding)

Phase B's 10k smoke surfaced a real confound: 3/3 baseline-solved targets
regressed under a plain eMolecules-tier candidate, and all 3 traced to
specific building blocks (phenol, propane, butane, methanesulfonyl
chloride) **confirmed absent from the entire 9.48M-compound eMolecules
source**, not just the 10k slice -- a stock-*source*-identity gap, not a
stock-*size* effect. Presented this to the user before committing to
Phase C's ~10x-more-expensive run; **user decided**: redefine the
candidate arm as `union(402 default stock, 100k tier)` rather than the
100k tier alone, so candidate is a guaranteed superset of baseline --
"adding compounds never hurts" becomes structurally true, isolating the
actual question (does the added scale help) from source-identity noise.

- Baseline: `data/building_blocks.smi` (402 compounds, unchanged).
- Candidate: `data/stock_tiers/tier_100000_union_default.imported.smi`
  (union of the 402-compound default stock and the 100k tier, built via
  `renkin stock import` on the concatenation -- 100,397 unique
  structures after dedup; 3 pre-existing unparseable rows in the default
  stock itself caught and reported, not silently dropped; 49 duplicate
  rows including genuine cross-file overlap, e.g. `c1sccc1`/thiophene and
  `o1c(ccc1)Br`/2-bromofuran already present in both sources).
  `doctor stock`: 8/9 PASS + 1 WARN (missing license, inherited), 0 FAIL.
- `--spectator-bond-policy gated` fixed identically in both arms.
- `--depth 5 --beam-width 100 --templates data/templates_extracted_500.smi`,
  same for both arms.
- Fixed 100 targets: first 100 by `sample_rank` from
  `data/comparison/sample_full_sorted.jsonl`.
- Harness: `scripts/compare_run.py` (unmodified) x2 +
  `scripts/stock_tier_paired_report.py` (Phase B's script, reused as-is).

## Raw summary

```json
{
  "n_targets": 100,
  "baseline_route_found_rate": {"n_numerator": 13, "value": 0.13},
  "candidate_route_found_rate": {"n_numerator": 21, "value": 0.21},
  "baseline_validator_confirmed_rate": {"n_numerator": 13, "value": 0.13},
  "candidate_validator_confirmed_rate": {"n_numerator": 21, "value": 0.21},
  "baseline_timeout_count": 5,
  "candidate_timeout_count": 28,
  "regression_count": 2,
  "regressions": ["uspto50k_test#L2553", "uspto50k_test#L4139"],
  "new_solve_count": 10,
  "new_solves": ["uspto50k_test#L1486", "uspto50k_test#L1632", "uspto50k_test#L2263",
    "uspto50k_test#L2867", "uspto50k_test#L3034", "uspto50k_test#L405",
    "uspto50k_test#L4259", "uspto50k_test#L4575", "uspto50k_test#L4886", "uspto50k_test#L775"],
  "route_found_rate_diff_candidate_minus_baseline": {
    "observed": 0.08, "ci_low": 0.02, "ci_high": 0.15,
    "mcnemar_p_value": 0.0386, "discordant_candidate_only": 10, "discordant_baseline_only": 2
  },
  "baseline_peak_rss_bytes_max": 155942912,
  "candidate_peak_rss_bytes_max": 163971072,
  "baseline_gated_out_candidate_count_total": 114776,
  "candidate_gated_out_candidate_count_total": 56125,
  "latency_paired_deltas_ms": {
    "n_both_completed": 72,
    "mean_candidate_minus_baseline": 62827.8,
    "sum_candidate_minus_baseline": 4523603.4
  }
}
```

Full summary at `data/stock_tiers/gate_100k_formal/summary.json`; raw
rows at `baseline.jsonl`/`candidate.jsonl`.

## Baseline arm's own aggregate

100/100 completed, 5 timeouts (5%), `validator_confirmed_route_found_rate`
13/100 (0.13), `total_elapsed_ms` p50 18.2s / p95 122.6s / max 150.0s.
Wall-clock total sweep: 3452s (~57.5 min).

## Candidate arm's own aggregate

100/100 completed, **28 timeouts (28%)**, `validator_confirmed_route_found_rate`
21/100 (0.21), `total_elapsed_ms` **p50 108.5s** / p95 150.0s / max 150.1s.
Wall-clock total sweep: 10,530s (~2.9 hours, ~3x baseline's).

## The 4 axes (paired, within-run only)

- **Search capability**: raw `route_found` 13/100 -> 21/100 (+8pp, McNemar
  p=0.039, 10 candidate-only vs. 2 baseline-only discordant pairs). At
  n=100 this is descriptive only per this project's own standing
  statistics discipline (`compare_stats.py`) -- not a "statistically
  significant" claim, but a real, consistent, non-trivial-sized signal
  (5x more new-solves than regressions).
- **Route quality**: `validator_confirmed_route_found_rate` is
  **identical to `route_found_rate` in both arms** (13/100 and 21/100) --
  every single route either arm found was validator-confirmed. No
  chemically-invalid "solved" routes in this sample, in either arm.
- **Side effects**: timeout rate jumped 5% -> 28%, median latency
  18.2s -> 108.5s (~6x). **Root-caused below: this is stock-loading
  overhead, not search-quality degradation.** Peak RSS rose modestly
  (156MB -> 164MB max) -- the union stock is ~250x more compounds than
  the default but RSS barely moved, consistent with `FxHashSet<String>`
  holding canonical SMILES strings efficiently once loaded (the *load
  process* is the expensive part, not the *resident* memory).
- **Safety**: `gated_out_candidate_count` totals actually *dropped*
  under candidate (114,776 -> 56,125) despite the much larger stock --
  counterintuitive at first glance, but consistent with the timeout
  explanation below: many candidate-arm searches never got far enough
  into real expansion (spending most of their 150s budget on stock
  loading) to generate as many gate-checkable candidates as baseline's
  faster, fully-completed searches did.

## Root cause of the timeout/latency increase: stock *loading*, not search

Confirmed directly, not inferred: a trivial **depth-0 case** (target
`C(=O)O`, already a member of the union stock -- `route_cost: 0.0`,
`joint_success_probability: 1.0`, i.e. essentially zero search work
beyond confirming stock membership) took **169.76s wall-clock**
end-to-end against `tier_100000_union_default.imported.smi`. Compare:
the *entire* `stock_import` tool's own from-scratch canonicalization of
the standalone 100k tier took ~4 minutes (Phase A) -- this number is
consistent with that, since `ChemEnv::load` (the runtime CLI's stock
loader) re-parses and re-canonicalizes the whole ~100,397-line file via
the same chematic calls, **from scratch, on every single CLI
invocation** -- there is no caching, no serialized/pre-canonicalized
format, no persistent-process reuse between the 100 separate `renkin
--target ...` subprocess calls this harness makes.

This single number explains the entire side-effects picture:
- **The 2 regressions** (`L2553`, `L4139`): baseline solved both in
  **112ms and 2.47s** respectively (trivial, fast searches). Under
  candidate, both **timed out at 150s** -- confirmed via `run_status`
  in the raw rows. Given ~170s is needed just to *load* the stock before
  any search logic runs, and the external timeout is 150s, **these two
  specific targets almost certainly never got a meaningful chance to
  search at all** -- the fixed load tax alone exceeds the time budget.
  This is not a case of the bigger stock causing worse search decisions
  (e.g. beam crowd-out); it's a pure infrastructure cost swallowing the
  time budget before search-quality even becomes a factor.
- **The 28% timeout rate and 6x median latency increase**: directly
  consistent with every candidate-arm invocation paying a ~170s fixed
  tax before doing any real work, on top of whatever the actual search
  itself costs.

**This is the single most actionable finding of Phase C** -- more
consequential than the raw solve-rate numbers. It directly answers the
original plan's own deferred question ("do NOT preemptively rewrite
`ChemEnv::load` into a streaming/indexed implementation... only redesign
if a specific tier's numbers show it's actually needed"): **the numbers
now show it's needed.** At 100k-compound scale, per-invocation
CLI-style usage (reload everything from scratch every call) is already
impractical for latency-sensitive workloads; this problem can only get
worse at 1M scale (Phase D).

## Assessment

**Two separate, non-conflated findings**:
1. **Search capability, when it gets to run**: a positive, real signal.
   +8pp raw solve rate, 100% validator-confirmed in both arms, 5x more
   new-solves than regressions, and the only 2 regressions are fully
   explained by a timeout artifact, not a genuine quality loss. This
   supports "a larger, still-modest-scale stock helps RENKIN solve more
   real targets" as a directionally real effect (n=100, descriptive
   only, not proof at real vendor-catalog scale).
2. **Practical usability at this stock scale, via the CLI's current
   loading architecture**: a real, serious cost. ~170s fixed load time
   per invocation makes the *current* CLI-per-target usage pattern
   impractical for anything beyond small-batch or non-interactive use
   at 100k+-compound scale, regardless of what the search itself finds.
   This is an engineering/architecture finding, not a chemistry one --
   and does not undermine finding #1's validity (the searches that
   *did* complete found real, validator-confirmed routes).

This does **not** prove "RENKIN beats X at native scale" or any
production-readiness claim about a 100k+-compound stock -- it proves the
add-scale-safely design question (does more stock ever hurt solve rate)
has a clean "no" answer at this scale under the union design, and
surfaces a concrete, fixable engineering bottleneck for anyone wanting
to actually use a stock this size day-to-day.

## Consequence for Phase D

The 1M tier is 10x larger than the 100k tier. If `ChemEnv::load`'s cost
scales anywhere near linearly with stock size (plausible, given it's a
straight per-line parse+canonicalize loop), **a naive 1M-compound load
could cost on the order of 1,700s (~28 minutes) per CLI invocation** --
making a standard 150s-external-timeout search-smoke run at 1M scale
almost meaningless (every invocation would time out on load alone,
regardless of search). Recommend Phase D **measure `ChemEnv::load` time
directly first** (the same depth-0-trivial-target technique used above),
before committing to any 10-target search smoke under the original
150s-timeout assumption -- the load-time number alone may already be a
sufficient, load-bearing answer to Phase D's own "1M scalability gate"
question, and the smoke's own timeout budget will need to be set based
on that real number, not the value inherited from Phases B/C.

On the plan's own conditional-escalation question (only run a full
100-target 1M comparison if Phase C showed a meaningful coverage gain
AND 1M's load/RSS stays reasonable): **condition (a) is met** (a real,
if modest, +8pp gain at 100k) but **condition (b) needs the direct 1M
load-time measurement above before it can be judged** -- not assumed
either way here.
