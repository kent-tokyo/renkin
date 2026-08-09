# 100-target repeatability characterization — Issue #66 "item D"

Answers the comparison guide's outstanding "500/full run gate" requirement:
"AiZynthFinder's repeat-run variance... characterized on at least 3
independent repetitions of the 100-target sample, in both native and
shared-stock modes" and "RENKIN's own repeatability... at least 2 RENKIN
runs per mode."

**Run counts**: RENKIN 2 total runs per arm (the existing checked-in
`data/comparison/results_100/renkin_{native,shared_stock}.jsonl` plus one
new run each, `renkin_{native,shared_stock}_run2.jsonl` in this directory).
AiZynthFinder 4 total runs per arm (the existing checked-in
`aizynthfinder_{native,shared_stock}.jsonl` plus three new runs each,
`aizynthfinder_{native,shared_stock}_run{2,3,4}.jsonl`).

All new runs post-date PR #74 (Issue #71's VF2-fallback removal) and
post-date this repository's post-#78 re-measurement — same binary/config as
the currently-checked-in `data/comparison/results_100/` files. **Labeled
post-#74 / pre-#72-fix**: these runs predate Issue #72's ring-topology gap
being addressed (still tracked separately, not yet fixed).

## RENKIN: deterministic, both arms

| Arm | Diffs (route_found or route hash) between the 2 runs |
|---|---|
| native | 1/100 (`uspto50k_test#L3345` — a boundary timeout in one run, completed-unsolved in the other) |
| shared_stock | 1/100 (`uspto50k_test#L4422` — same boundary-timeout pattern) |

Both disclosed already in `per_target_audit.md`'s addendum — neither is new.
Excluding these two known timeout-boundary cases, **RENKIN is
byte-for-byte deterministic across both arms**: identical `route_found` and
identical `normalized_route_sha256` for every other target. This matches
the project's own expectation (no RNG/seed anywhere in the search).

## AiZynthFinder: solve-state fully stable, route selection has minor variance

| Arm | Solve-state unanimous across all 4 runs | Union solved | Intersection solved | Per-run solved counts |
|---|---|---|---|---|
| native | 100/100 (zero flips) | 66/100 | 66/100 | 66, 66, 66, 66 |
| shared_stock | 100/100 (zero flips) | 4/100 | 4/100 | 4, 4, 4, 4 |

**Whether AiZynthFinder solves a given target is completely stable across
4 independent runs in this sample**, for both arms — despite the project
having no documented seed-fixing mechanism. This is a real, disclosed
empirical finding about this specific 100-target sample and configuration;
it is not a guarantee that holds at 500/4,903 targets or under different
search budgets.

**Route selection (which specific route is reported as best) has some
variance**, even among consistently-solved targets:

| Arm | Always-solved targets | Same route hash all 4 runs | Route-hash variance |
|---|---|---|---|
| native | 66 | 60 (90.9%) | 6 (`L2263`, `L3217`, `L68`, `L1486`, `L338`, `L1845`) |
| shared_stock | 4 | 3 (75%) | 1 (`L2263`) |

So AiZynthFinder's MCTS-based search has some run-to-run variance in which
specific route it reports as best for a small fraction of targets, even
though whether it solves the target at all is stable. `L2263` shows this
variance in both arms.

## What this does and doesn't establish

- Both tools' `route_found`/solve-state numbers checked into
  `data/comparison/results_100/` (the paired-stats headline numbers) are
  well-supported by this repeatability check — neither tool's solve rate is
  an artifact of a lucky/unlucky single run at n=100.
- This does **not** re-run or change any paired statistics — `paired_stats_*.json`
  in `data/comparison/results_100/` are unaffected; this is a repeatability
  characterization, not a new measurement round.
- This does **not** by itself clear the "500/full run gate" — see the
  guide's own remaining preconditions (Issue #72 disposition, corpus
  provenance gaps, compute budget confirmation for a much larger sweep).
- Reproduction: `docs/guides/open-source-retrosynthesis-comparison.md`'s
  "Reproduction" section, running `compare_run.py` additional times with
  the same flags into this directory's `_run2`/`_run3`/`_run4` file names.
