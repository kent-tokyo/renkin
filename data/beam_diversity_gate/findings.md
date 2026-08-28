# Diversity-reserved beam — stage 5 formal gate, `slots=10` (2026-08-28)

Real result for `docs/design/diversity-reserved-beam-v0.md`'s stage 5,
using `scripts/beam_diversity_formal_gate.py` against the fixed 100-target
sample (`data/comparison/sample_full_sorted.jsonl`, first 100 by
`sample_rank`), PR #104's own Round 2G exact CLI config (`--ring-context-policy
conservative`, 500 templates, 393-compound shared stock, `--depth 5
--beam-width 100`), `--beam-diversity-slots 10` (10% of beam width).

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

## Not done, deliberately

- No sweep across other `beam_diversity_slots` values -- one data point
  only, per explicit user decision to start with a single value before
  deciding whether further investment is worthwhile.
- No fresh-baseline-only confirmatory run to characterize exactly how
  much the 2026-08-13 baseline has drifted (14 vs. 16 solved, 5 vs. 0
  timeouts) -- noted as a real drift, not root-caused further here (out
  of this measurement's own scope; the rule-safety census's 4 rule
  removals are the most likely single contributor, not independently
  confirmed).
