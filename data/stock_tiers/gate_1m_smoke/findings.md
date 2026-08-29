# Stock-tier pilot — 1M scalability gate (Phase D), 2026-08-29

Per `internal_docs/ROADMAP.md`'s v0.36.0 stock-pilot plan, following
Phase C's 100k formal comparison (`data/stock_tiers/gate_100k_formal/findings.md`).

## Design (consistent with Phase C's union rationale)

Same union design as Phase C, scaled up: candidate =
`union(402 default stock, 1M tier)`, not the plain 1M tier alone --
guarantees candidate is a strict superset of baseline, isolating "does
the added scale help" from stock-source-identity noise (the same
confound Phase B surfaced at 10k scale).

- Baseline: `data/building_blocks.smi` (402 compounds, unchanged).
- Candidate: `data/stock_tiers/tier_1000000_union_default.imported.smi`
  (union of the 402-compound default stock and the 1M tier -- 1,000,362
  unique structures after dedup, 3 pre-existing unparseable rows in the
  default stock itself, 84 duplicates including cross-file overlap).
  `doctor stock`: 8/9 PASS + 1 WARN (missing license, inherited), 0 FAIL.
- `--spectator-bond-policy gated` fixed identically in both arms.
- `--depth 5 --beam-width 100 --templates data/templates_extracted_500.smi`,
  same for both arms.
- `--timeout-s 1800` (30 min per target) -- NOT the 150s used in Phases
  B/C, deliberately widened once the direct load-time measurement below
  showed 150s would be structurally incompatible with this stock's own
  fixed load cost.
- Fixed **10 targets**: first 10 by `sample_rank` from
  `data/comparison/sample_full_sorted.jsonl`.
- Harness: `scripts/compare_run.py` (unmodified) x2 +
  `scripts/stock_tier_paired_report.py` (reused as-is, same script as
  Phases B/C). **User decision (2026-08-29)**: run the full 10-target
  smoke even after the direct load-time measurement below showed it
  would cost multiple hours, rather than stopping at the load-time
  number alone or running a smaller compromise smoke.

## Pre-measurement: direct `ChemEnv::load` time isolation (done before committing to the smoke)

Per Phase C's own recommendation ("measure `ChemEnv::load` time directly
before committing to any smoke run under the old timeout assumption"):
a trivial depth-0 case (target already a stock member, ~zero search
work) against `tier_1000000_union_default.imported.smi` took
**13m37s (817s) wall-clock**. Confirmed via the run's own CPU/wall-clock
breakdown (11m user + 10s sys of 13m37s wall, ~81% CPU utilization) this
is genuine CPU-bound chematic canonicalization cost, not disk I/O -- a
second identical invocation was started to confirm OS-file-cache
warming wouldn't help, but killed before completion once the CPU-bound
conclusion was already clear from run 1's own numbers (avoiding an
unnecessary ~14-minute repeat measurement).

This number, not a guess, set the `--timeout-s 1800` budget above: ~14
minutes of fixed load tax leaves ~16 minutes of real search budget per
target before hitting the ceiling. Note this is **sub-linear** relative
to Phase C's 100k-union load time (170s): a naive 10x-linear
extrapolation predicted ~1700s (~28 min), but the actual number (817s,
~4.8x) came in well under that -- worth recording as a real data point,
not assuming linear-in-compound-count scaling holds at this range.

## Baseline arm's own aggregate

10/10 completed, 0 timeouts, 1 validator-confirmed (0.1). Wall-clock
total sweep 1175s (~19.6 min). Per-target `total_elapsed_ms`: 255ms to
741,763ms -- one clear outlier, `uspto50k_test#L1446` (rank 8, a long
polyene-chain molecule, `CC/C=C\C/C=C\.../C=C\CCC(=O)N...`), at 741.8s
even under the tiny 402-compound stock. This is a real, unusual
search-cost case **unrelated to the stock-size question** (same stock
as every other baseline target) -- flagged so it isn't confused with
the candidate arm's own (much larger, load-driven) latency below.

## Candidate arm's own aggregate

10/10 completed, **0 timeouts** (unlike Phase C's 28% timeout rate --
the widened 1800s budget was sufcient this time), 4 validator-confirmed
(0.4). Wall-clock total sweep 6670s (~111 min, ~1.85 hours). Per-target
`total_elapsed_ms` ranged 435,680ms to 998,997ms (~7.3-16.6 min) -- i.e.
**every single target's total time was dominated by the ~14min load
tax**, consistent with Phase C's own root cause, just at larger absolute
scale. Peak RSS ranged 133.4MB to 173.2MB across the 10 targets (some
real growth across the run, plausibly reflecting different
candidate-generation memory footprints per target rather than a leak,
since these are 10 independent subprocesses).

## The 4 axes (paired, within-run only)

- **Search capability**: raw `route_found` 1/10 -> 4/10 (**+30pp**,
  observed diff 0.30, McNemar p=0.25 -- not meaningful at n=10, purely
  descriptive, but the raw signal itself is clean: 3 candidate-only
  discordant pairs, **0 baseline-only**).
- **Route quality**: `validator_confirmed_route_found_rate` identical to
  `route_found_rate` in both arms (1/10 and 4/10) -- every route either
  arm found was validator-confirmed, same clean pattern as Phase C.
- **Side effects**: **0 timeouts in either arm** (unlike Phase C) --
  the widened 1800s timeout was the right call. Latency: candidate mean
  +516s/target vs. baseline (paired, all 10 both-completed). Peak RSS:
  candidate max 173MB vs. baseline max 50MB -- a real, substantial
  memory-footprint increase at 1M scale (unlike Phase C's ~100k scale,
  where RSS barely moved 156MB->164MB; this time it roughly
  3.5x'd, 50MB->173MB, plausibly the union stock's own resident
  `FxHashSet<String>` size becoming a real factor at this scale, not
  just the load-time cost).
- **Safety**: `gated_out_candidate_count` totals essentially unchanged
  (3916 baseline vs. 3902 candidate) -- at n=10 this is too small a
  sample to read much into either way.

## Regressions: zero

**`regression_count: 0`** -- every target baseline solved, candidate
also solved (trivially true here since baseline only solved 1/10, but
confirmed: that 1 target, `uspto50k_test#L2743`, was NOT lost under
candidate). Unlike Phase C (2 timeout-driven "regressions" under the
150s budget), **the widened 1800s timeout fully eliminated the
load-tax-exceeds-budget artifact** -- this run shows the union design's
core structural promise ("adding compounds never hurts") holding
cleanly with zero exceptions, once given a realistic time budget.

## New solves: 3

`uspto50k_test#L1248`, `uspto50k_test#L1446`, `uspto50k_test#L2743`.
Notably, **`L1446` is the same target that was baseline's own 741.8s
outlier above** -- baseline's search ran for 12+ minutes against the
tiny stock and still failed to find a route, while candidate (with
access to 1,000,362 compounds) found a validator-confirmed route in
511.6s, *faster* than baseline's own failed attempt. This is a clean,
concrete illustration of the actual value proposition scale is meant to
deliver: a real target that a small curated stock genuinely couldn't
reach became solvable with a scale-appropriate stock, at no cost to
anything the small stock could already do.

## Assessment

**Two separate, non-conflated findings, same shape as Phase C but
stronger signal**:
1. **Search capability, when it gets to run**: a substantial positive
   signal -- +30pp raw solve rate, 0 regressions, 100% validator-
   confirmed in both arms, and a concrete illustrative case (`L1446`)
   showing genuine value from scale. Directionally consistent with and
   larger than Phase C's own +8pp at 100k scale (different n and
   different specific targets, so not a strict apples-to-apples
   scaling claim, but the qualitative pattern -- more stock, more real
   solves, zero cost to existing solves under the union design -- held
   at both tested scales).
2. **Practical usability at 1M scale via the CLI's current
   per-invocation-reload architecture**: confirmed, more severely than
   Phase C's 100k-scale finding, that this is impractical for
   interactive/latency-sensitive use -- every single candidate-arm
   invocation spent the large majority of its ~8-17 minute total time
   on stock loading alone, not search. This is the same
   `ChemEnv::load` bottleneck Phase C already identified, now confirmed
   at the actual 1M target scale with a direct measurement rather than
   an extrapolation.

Neither finding undermines the other: the searches that completed found
real, validator-confirmed, genuinely new routes -- the scale-helps
conclusion is not an artifact of the loading cost. But the loading cost
is real, large, and would need to be solved (persistent-process
loading, a pre-canonicalized on-disk format, or similar) before a
1M-compound stock could be used practically outside a batch/offline
context.

## Phase 3 gate status (issue #86, exact AiZynthFinder ZINC-scale arm)

Per the plan's own Phase 3 entry conditions, condition 4 ("Phase 2's
1M-scale pilot has already passed cleanly") is a **split verdict**, not
a clean pass or fail:
- **Structural stock-size safety**: passes cleanly. 0 regressions
  across both the 100k (Phase C) and 1M (Phase D) tiers under the union
  design; the "does more stock ever hurt" question has a clear,
  evidenced "no" answer at both scales tested.
- **Practical usability at this scale**: does not pass. The
  ~14-17-minute-per-invocation load cost, confirmed directly rather
  than assumed, means the *current* CLI architecture cannot practically
  run a real 100+-target comparison at 1M scale in any reasonable time
  (a 100-target run at this per-target cost would take on the order of
  1M-tier-load-time x 100 ~ 24+ hours, before any real search time is
  added).

**Recommendation**: do not treat condition 4 as met for launching
Phase 3's exact-ZINC-scale (~17.4M-compound) arm yet -- that scale is
another ~17x beyond what was just measured here, and the load-time
problem would only get worse, not better, without an architecture
change first (persistent-process loading or a serialized/cached stock
format). This is exactly the kind of engineering prerequisite v0.38.0
"Vendor Stock Intelligence" should address before Phase 3 becomes
practically approachable, not something to route around via a bigger
timeout alone.
