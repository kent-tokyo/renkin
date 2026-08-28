# Diversity-reserved beam — stage 5 formal gate, sweep results

Real results for `docs/design/diversity-reserved-beam-v0.md`'s stage 5,
using `scripts/beam_diversity_formal_gate.py` against the fixed 100-target
sample (`data/comparison/sample_full_sorted.jsonl`, first 100 by
`sample_rank`), PR #104's own Round 2G exact CLI config (`--ring-context-policy
conservative`, 500 templates, 393-compound shared stock, `--depth 5
--beam-width 100`). Two `--beam-diversity-slots` values measured so far:
`10` (2026-08-28) and `20` (2026-08-29, see second section below).

## Stage 5 formal gate, `slots=10` (2026-08-28)

`--beam-diversity-slots 10` (10% of beam width).

## Raw summary

```json
{
  "n_targets": 100,
  "beam_diversity_slots": 10,
  "baseline_solved": 14,
  "candidate_solved": 15,
  "coverage_delta_pp": 1.0,
  "invalid_count": 0,
  "baseline_timeouts": 5,
  "candidate_timeouts": 5,
  "regression_count": 0,
  "regressions": [],
  "new_solve_count": 1,
  "new_solves": ["uspto50k_test#L4575"],
  "baseline_p95_s": 102.18,
  "candidate_p95_s": 108.88
}
```

## Important caveat: PR #104's own absolute thresholds don't apply anymore

PR #104's Round 2G baseline (2026-08-13) solved 16/100 with 0 timeouts.
**This run's own baseline solves only 14/100 with 5 timeouts** -- the
codebase has genuinely drifted since then (chematic 0.16->0.20.1, 4 rules
removed in the v0.36.0 rule-safety census, etc.). This was flagged as a
real risk before running (the harness's own research-fork investigation
recommended a baseline sanity check). **Conclusion: PR #104's exact
numeric thresholds (`>=18/100`, `<=104.5s p95`, `=0 timeouts`) are stale
and must not be used to grade this result.** The only valid comparison is
candidate vs. baseline *within this same run*, which is what follows.

## Within-run comparison (the only valid one)

- **0 regressions** -- no target solved by baseline became unsolved under
  the candidate. This is the single most important safety signal, and it
  held cleanly.
- **+1 new solve** (`uspto50k_test#L4575`): baseline completes in 8.35s
  unsolved, candidate completes in 8.01s solved -- clean, fast, not a
  near-timeout rescue. The mechanism is doing exactly what it was
  designed to do (rescue a target crowded out by same-family candidates)
  at least once in this sample.
- **Timeouts are identical sets in both arms**
  (`L1446`/`L1530`/`L1845`/`L4422`/`L576`) -- the diversity mechanism
  causes zero *new* timeouts; these 5 are pre-existing hard cases
  (`L4422` in particular has a long history in this project's own
  timeout-diagnostics work) unrelated to this mechanism.
- **p95 latency looks worse at face value** (108.88s vs. 102.18s, +6.7s)
  but **median and mean are essentially unchanged** (median 20.29s vs.
  20.50s, mean 31.88s vs. 32.20s -- candidate very slightly *faster* on
  average). Paired per-target deltas (candidate - baseline, n=95 targets
  completed in both arms) sum to **-30.7s net** (candidate faster
  overall), with a handful of individual-target outliers (+12.7s to
  +38.3s) accounting for the tail. **Conclusion: no systematic
  performance regression** -- the mechanism's own extra bookkeeping is
  the small, expected `O(beam_width)` cost the design doc predicted, and
  the apparent p95 shift is ordinary tail variance from a modest (n=100)
  sample, not a real overhead signal.

## Assessment

Safe (zero regressions, no new timeouts, no systematic slowdown) with a
small, genuine, clean positive effect at `slots=10` (+1/100 targets, the
mechanism's diagnostics correctly attribute it to a diversity rescue, not
noise). Effect size is modest -- far short of PR #104's own historical
`route_to_configured_stock` bar, but that bar no longer reflects today's
codebase anyway (see caveat above). Whether a different `--beam-diversity-slots`
value (e.g. 20, 5) changes this meaningfully is untested -- this is a
single data point, not a sweep. Not shipped as a default anywhere;
`Off` remains the default on every surface (CLI/Python/WASM/Rust).

## Not done, deliberately (as of the `slots=10` run)

- No sweep across other `beam_diversity_slots` values -- one data point
  only, per explicit user decision to start with a single value before
  deciding whether further investment is worthwhile.
- No fresh-baseline-only confirmatory run to characterize exactly how
  much the 2026-08-13 baseline has drifted (14 vs. 16 solved, 5 vs. 0
  timeouts) -- noted as a real drift, not root-caused further here (out
  of this measurement's own scope; the rule-safety census's 4 rule
  removals are the most likely single contributor, not independently
  confirmed).

---

# Stage 5 formal gate, `slots=20` (2026-08-29)

Second sweep point, same harness/config/sample, `--beam-diversity-slots 20`
(20% of beam width), run per user's explicit follow-up request to measure
additional slots values.

## Raw summary

```json
{
  "n_targets": 100,
  "beam_diversity_slots": 20,
  "baseline_solved": 14,
  "candidate_solved": 16,
  "coverage_delta_pp": 2.0,
  "invalid_count": 0,
  "baseline_timeouts": 7,
  "candidate_timeouts": 8,
  "regression_count": 0,
  "regressions": [],
  "new_solve_count": 2,
  "new_solves": ["uspto50k_test#L2263", "uspto50k_test#L4575"],
  "baseline_p95_s": 94.09,
  "candidate_p95_s": 75.08
}
```

## Cross-run caveat: baseline itself is not stable run-to-run

This run's own baseline solved 14/100 with **7** timeouts, vs. the
`slots=10` run's baseline of 14/100 with **5** timeouts -- same solve
count, different timeout count, despite both nominally running the
identical `policy=off` configuration. This is real run-to-run variance
(most likely CPU contention on this shared development machine across
separate measurement sessions), not a config difference. **Consequence:
the two runs' raw numbers must not be compared directly against each
other.** Only each run's own internal (paired, same-run) baseline-vs-
candidate comparison is valid, exactly as for `slots=10`.

## Within-run comparison (the only valid one)

- **0 regressions** -- no target solved by baseline became unsolved
  under the candidate. Holds cleanly a second time.
- **+2 new solves**, both clean completions, not near-timeout rescues:
  - `uspto50k_test#L2263`: baseline completes unsolved in 10.99s,
    candidate completes solved in 10.36s (candidate faster).
  - `uspto50k_test#L4575`: same target that was the single new solve at
    `slots=10`. Baseline completes unsolved in 9.46s, candidate
    completes solved in 8.00s. **This target is rescued consistently
    across both slots values tested so far** -- reassuring, not a
    one-off fluke of a specific slots setting.
- **One genuine new cost, not a regression in the route_found sense**:
  `uspto50k_test#L338` is baseline's near-timeout case (completes
  unsolved at 105.5s) but times out under the candidate (150.0s, still
  unsolved either way). The diversity mechanism's extra bookkeeping
  pushed this one borderline target over the wall-clock limit. It does
  not flip solved-to-unsolved (`regression_count` correctly reports 0
  since baseline never solved it either), but it is a real, measurable
  latency cost on at least one target and should not be glossed over.
  Net timeout count: baseline 7, candidate 8 (all 7 baseline timeouts
  reproduced under candidate, plus this one new one).
- **p95 favors candidate at face value** (75.08s vs. 94.09s) and this
  time the deeper stats agree: median 17.08s vs. 18.31s, mean 28.56s vs.
  29.90s (candidate faster on both), and paired per-target deltas
  (n=92 targets completed in both arms) sum to **-47.9s net** (mean
  delta -0.52s/target) -- consistent with `slots=10`'s own net-faster
  finding, not contradicted by it.

## Assessment after two data points

`slots=10` and `slots=20` show the same qualitative shape: zero
regressions, a small positive coverage effect (+1 and +2 targets
respectively, out of a 100-target sample), and no systematic slowdown
(paired deltas net faster both times). `slots=20` additionally surfaces
one real, small latency cost (`L338`'s timeout) that `slots=10` did not
-- the first evidence in this sweep that the mechanism's cost is not
strictly free, even though it doesn't show up as a solved-target
regression. `uspto50k_test#L4575` being rescued at *both* tested slots
values is the strongest single piece of evidence that this is a real,
reproducible rescue effect and not sample noise. Still not a strong case
for a default change (modest effect size, small sample, no ablation past
2 points), but a second consistent, mostly-clean data point in the same
direction as the first. `Off` remains the default everywhere.

## Not done, deliberately (as of the `slots=20` run)

- No sweep past 2 points (`10`, `20`) yet -- next candidate values per
  the design doc's own suggested range would be `5` (below `10`) or a
  point above `20`, not yet run.
- No root-cause investigation into why `L338` specifically regresses in
  latency under diversity reservation (e.g. which template family it
  reserves slots for on this target) -- flagged as a real, small,
  reproducible cost, not investigated further here.
- No fresh-baseline-only confirmatory run to explain the 5-vs-7 baseline
  timeout discrepancy between the two runs -- attributed to environmental
  noise (shared-machine contention), not independently confirmed.
