# Round 2G: clean, paired 100-target formal gate re-measurement

Source: `renkin` (post Round 2A-2E open-state lifecycle fix), single binary
sha256 `db16e5bdfdefeaf075ad9e723b23abe03f2a231fd51e5082a5fcc74326a61d68`
used for both arms (baseline = flag off, candidate = `--open-state-dominance`).
Conservative ring-context policy, `data/comparison/shared_stock/shared_stock.smi`,
depth=5, `data/templates_extracted_500.smi`, beam=100. Both arms measured
**fresh in this run** -- does NOT reuse `data/phase1c_diagnostics/beam100.jsonl`
or `data/phase2h_formal_gate/*.jsonl`, which were measured in a different
session, under confirmed CPU contention (`pipeline_v2_vs_rdkit_dump`), against
the pre-fix binary. Per-target run order alternates by `sample_rank` parity
(even: baseline->candidate, odd: candidate->baseline; confirmed 50/50 split)
to spread thermal/order drift evenly across both arms rather than
systematically favoring one. Script: `scripts/phase2g_round2_paired_sweep.py`.

## Pre-flight environment check

Full record in `preflight_environment.txt`. First check (11:21) found
`pipeline_v2_vs_rdkit_dump` running at 99.6% CPU -- the same unrelated
contention source documented in Phase 1C/2H, not launched by this session.
**Did not start the run** -- waited (polling every 30s) until it exited.
Re-verified clean at 12:01: load average 2.62/2.78/3.15 (vs. 10 cores),
no `renkin`/`pipeline_v2_vs_rdkit_dump`/`aizynthfinder` process running.
Swap usage was elevated (8.6G/9.2G used) in both checks -- a pre-existing
machine condition, not something this session caused or could clear;
noted for the record since it's a theoretically possible latency confound,
but load average and process list were clean, which is the primary signal
this protocol gates on.

Run started 12:01, completed 13:23:23 (~82 minutes for 100 targets x 2 arms
= 200 search invocations). No competing process re-appeared during the run
(confirmed via post-run environment check; `run.log` has no
crash/anomaly markers).

## Data integrity

100/100 unique `target_id`s in both `baseline.jsonl` and `candidate.jsonl`,
0 duplicates, single consistent `binary_sha256` across all 200 rows in each
file, `run_order.jsonl` confirms the even/odd pairing was applied exactly
as designed (50 baseline-first, 50 candidate-first).

## Gate result (pre-registered thresholds, unchanged per "goalpostは変更禁止")

| gate criterion | threshold | result | verdict |
|---|---|---|---|
| `route_to_configured_stock` | >= 18/100 | **17/100** | **FAIL** |
| invalid/unparseable | = 0 | 0 | PASS |
| regressions among baseline-solved | <= 1 | **0** | PASS |
| timeouts | = 0 | **1** (`L4422`) | **FAIL** |
| p95 latency (completed runs) | <= 104.5s | **98.2s** | **PASS** |
| deterministic repeat | identical `raw_output_sha256` | not re-run this round (already covered by `open_state_dominance_is_deterministic_across_repeated_runs` unit test + Phase 2H's L1541 repeat check) | -- |

**OVERALL: GATE MISS** (2 of 5 gating criteria fail).

## route_found: baseline 16/100 -> candidate 17/100

- **Newly solved (1): `L4092`.**
- **Regressed (0):** none.
- This exactly matches the corrected causal prediction from Round 2F/2A
  Round 2 (the fixed mechanism prunes *less* aggressively than the buggy
  one, so the honest prior was `17/100` or `16/100`, not the buggy
  candidate's `18/100`) -- **confirmed empirically**: the corrected,
  clean, full-corpus number is `17/100`, one solve short of both the old
  (bug-inflated, contamination-affected) `18/100` headline and the
  pre-registered `>=18` threshold.
- `L1640`, which the pre-fix buggy candidate solved (Phase 2G) but the
  fixed candidate did not retain at the 6-target scale (Round 2F), is
  **also unsolved here** at the full 100-target scale -- consistent
  between the two measurements, not a fluke of the smaller sample.

## Timeout: `L4422` (genuine, not a contention artifact)

| | baseline | candidate |
|---|---|---|
| status | completed | **timeout** |
| wall_clock_s | 96.2s | 150.0s (hit the cap) |

Baseline itself takes 96.2s on this target -- already close to the 150s
budget -- so a mechanism that explores more candidates (freed from the
ghost-record bug's incorrect suppression) pushing it over the timeout
threshold is unsurprising, and consistent with `L4422` having been
independently identified as one of the **3 genuine (non-contention-artifact)
timeouts** in the old, contamination-affected Phase 2H measurement
(alongside `L1446` and `L3262`, both of which complete within budget here).
This is a real cost of the mechanism on an already-hard target, not a
measurement artifact -- the whole point of this clean re-run was to
distinguish the two, and it does: 1 genuine timeout survives under a
verified-clean environment, down from Phase 2H's raw 6 (of which that
session's own isolated-recheck methodology had already argued 3 were
contention artifacts).

## p95 latency: 98.2s -- passes cleanly for the first time

Phase 2H's contamination-corrected estimate was 114.9s (FAIL, threshold
104.5s). Under this genuinely clean, paired measurement, candidate p95 is
**98.2s**, clearing the 104.5s (1.25x baseline) threshold with room to
spare (baseline p95: 68.3s). This is the direct payoff of Round 2G's
methodology: Phase 2H's isolated-recheck correction for 6 timeout targets
could not un-contaminate the other 94 "completed" targets that fed the
p95 calculation, which is exactly Blocker 2 from the user's review --
this run avoids that confound entirely by measuring the whole 100-target
population fresh, in a verified-clean environment, with both arms paired.

## Conclusion: PROMISING BUT GATE MISS

Per the pre-registered Round 2H decision rule:
- **Coverage gate**: FAIL by 1 (17/100 vs >=18/100 required).
- **Timeout gate**: FAIL by 1 (1 vs 0 genuine timeouts).
- **Latency gate**: PASS, and by a comfortable margin this time.
- **Regression gate**: PASS, cleanly (0 regressions, matching Phase 2G/2H's
  0-regression track record across every gate this candidate has been
  through).
- **Invalid/unparseable gate**: PASS (0, as always).

The mechanism is correctness-clean (0 regressions across every gate run
this program has done, at any beam width, on either the buggy or fixed
implementation) and now behaves exactly as intended after the Round 2A-2E
lifecycle fix (ghost records no longer linger, `peak_unique_open_states`
is trustworthy, ~11-16% live-state collision rate confirmed by diagnostics-only
re-measurement). But its net effect on the primary coverage metric is
smaller than the pre-fix, bug-inflated number suggested, and misses the
pre-registered `>=18/100` threshold by exactly 1 target, with 1 genuine
timeout also present. This is a **promising-but-gate-miss** result, not a
clean pass and not a no-benefit/regression result -- per the standing
program rule, this does **not** merge PR #104 and does **not** trigger
adaptive beam (Plan B) automatically; it calls for designing the next
candidate.
