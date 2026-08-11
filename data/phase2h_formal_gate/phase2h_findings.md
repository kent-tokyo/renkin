# Phase 2H: 100-target formal gate for open-state dominance

Source: `renkin --open-state-dominance` (commit `8259cf6`, worktree
`renkin-open-state-dominance`, branch `feat/open-state-dominance-101`).
100-target sample (`data/comparison/sample_full_sorted.jsonl`, first 100 by
`sample_rank` -- same sample as Phase 1C), Conservative ring-context policy,
`data/comparison/shared_stock/shared_stock.smi`, depth=5, beam=100,
sequential execution. Baseline = `data/phase1c_diagnostics/beam100.jsonl`
(flag off, binary `5d03554`) -- reused, not re-run, since the flag-off path
is verified byte-identical to unmodified master (unit test
`open_state_dominance_default_off_matches_legacy_behaviour_exactly` + a
real-binary repeat-run check this session).

## Raw sweep result

| metric | baseline | candidate (raw) |
|---|---|---|
| route_found | 16/100 | 18/100 |
| newly solved | -- | L1640, L4092 |
| regressed | -- | 0 |
| invalid/unparseable | 0 | 0 |
| timeouts | 0 | 6 (L1446, L1530, L3262, L4129, L4422, L506) |
| p95 latency (completed) | 83.6s | 116.6s |

## Methodology correction: CPU contention during the sweep

An unrelated external process (`pipeline_v2_vs_rdkit_dump`, not launched by
this session, running since before the sweep started) consumed ~80% CPU
throughout the ~100-minute sweep, with load average peaking around 15-21 on
a 10-core machine -- the same false-timeout risk documented in
`data/phase1c_diagnostics/L1541_candidate_narrative.md`'s methodology note
for Phase 1C. Rather than discard and re-run the full 100-target sweep, the
6 timeout targets were individually re-run in isolation (sequential,
one-at-a-time, no other renkin process, load ~3-9 by the time of the
recheck) with a generous 200s cap, and classified against the real protocol
threshold (150s SIGTERM / 160s SIGKILL):

| target | isolated wall-clock | classification |
|---|---|---|
| L1446 | 176s | **genuine** -- exceeds 150s even isolated |
| L4422 | >200s (killed) | **genuine** -- still times out at 2x the isolated cap |
| L3262 | 158s | **genuine** -- exceeds the 150s SIGTERM threshold (a well-behaved binary exits promptly on SIGTERM; the 10s grace is cleanup time, not extra compute budget) |
| L1530 | 104s | contention artifact -- well under 150s in isolation |
| L4129 | 91s | contention artifact |
| L506 | 100s | contention artifact |

**3 of the 6 raw timeouts were contention artifacts; 3 are genuine** --
`open_state_dominance` itself, not machine noise, makes these 3 targets
exceed the 150s budget at beam=100.

## Corrected (contention-adjusted) gate result

Substituting the 3 contention-artifact targets' isolated wall-clock into
the completed-run set (as if measured without contention) and excluding
the 3 genuine timeouts from the percentile calculation (matching the
pre-registered gate's "p95 latency (completed runs)" definition):

| gate criterion | threshold | raw result | corrected result | verdict |
|---|---|---|---|---|
| route_to_configured_stock | >= 18/100 | 18/100 | 18/100 | **PASS** |
| invalid/unparseable | = 0 | 0 | 0 | **PASS** |
| regressions among 16 baseline solves | <= 1 | 0 | 0 | **PASS** |
| timeouts | = 0 | 6 | 3 | **FAIL** |
| p95 latency (completed) | <= 104.5s | 116.6s | 114.9s | **FAIL** |

Deterministic-repeat check: not re-run as a full second 100-target pass
(would double an already ~100-minute run); already independently confirmed
at smaller scale (L1541 direct repeat, byte-identical `sha256`; unit test
`open_state_dominance_is_deterministic_across_repeated_runs`).

## Candidate-specific mechanism checks

- `peak_raw_heap_nodes` across all 100 targets: min 30, median 146, max 187
  -- bounded, tracks beam_width (100) plus in-flight expansion, not growing
  unboundedly.
- Every target with `open_state_better_replacements > 0` also shows
  `stale_open_nodes_removed_before_beam_prune > 0` or
  `stale_open_nodes_discarded_on_pop > 0` -- the mechanism is never silently
  inert when it should be acting.
- `stale_open_nodes_discarded_on_pop` totals **0 across all 100 targets** --
  matches the architectural prediction exactly: the pre-`beam_prune` filter
  (which runs every outer-loop iteration before any pop can observe a
  newly-stale node) catches staleness before the pop-time backstop is ever
  needed, in every one of 100 real searches, not just the synthetic unit
  test fixture.
- No invalid/unparseable routes at any point (raw or corrected) -- no
  chemistry-validation bypass introduced by dominance.

## Verdict: promising-but-gate-miss

The core efficacy signal -- the pre-registered primary metric,
`route_to_configured_stock` -- **passes exactly at the threshold** (18/100,
+2 over baseline, 0 regressions among the 16 already-solved targets), even
after controlling for CPU contention. This directly answers the question
Phase 2A's diagnostics posed: open-state dominance measurably converts some
of the cross-path duplicate-state crowd-out into additional solved targets,
not just fewer wasted beam slots.

However, the gate as pre-registered also fails on latency: **3 genuine new
timeouts** (not contention artifacts) and a **p95 that remains above the
1.25x threshold even after the correction** (114.9s vs 104.5s). This is a
real cost, not a measurement artifact -- freeing beam capacity from
duplicate-state crowd-out lets the search explore more distinct candidates
per target (`rules_attempted_total` rose 11-53% across the four
beam-sensitive targets in Phase 2G), which is exactly what drives the
extra solves, but the same effect pushes a handful of already-hard targets
past the timeout budget.

Per the Phase 2H spec's classification, this is **promising-but-gate-miss**,
not no-benefit and not a regression: the primary metric clears its bar, 0
correctness regressions, the mechanism behaves exactly as designed (stale
removal, bounded heap growth, deterministic tie-breaking) -- but the
pre-registered latency-tail criteria are not met. Per the program's own
decision rule ("Only proceed toward adaptive beam (Plan B) if the result is
no-benefit or a clear gate-miss"), this result does not trigger Plan B --
the candidate has real merit and should proceed to a **draft PR** (Phase
2I) disclosing this exact trade-off honestly, not a Ready-flip/merge, which
would require either accepting the latency-tail cost or following up with a
timeout-budget/latency mitigation this program's scope does not cover.
