# Phase 2A Round 2: corrected diagnostics-only re-measurement

Source: `renkin --open-state-diagnostics` (post Round 2A-2E lifecycle fix,
worktree `renkin-open-state-dominance`, branch `feat/open-state-dominance-101`).
Same 4 targets, same protocol as the original Phase 2A measurement
(Conservative ring-context policy, `data/comparison/shared_stock/shared_stock.smi`,
depth=5, `data/templates_extracted_500.smi`, beam=100). Diagnostics-only
mode never skips a push or removes a heap node, so the search trajectory is
byte-identical to legacy/flag-off in both the original and this
re-measurement -- only the bookkeeping differs.

## Why this needed re-measuring

Phase 2A's original 27-34% figure was computed as
`open_state_dominated_skipped / open_state_candidates_considered` against
`open_state_best.len()`-derived peak counters, but under the pre-fix code
`open_state_best` accumulated ghost records forever (Blocker 1) --
`open_state_best.len()` measured "states ever recorded", not "states
currently live". This inflates the apparent collision opportunity: a
candidate could get `Dominated`-skipped against a long-dead ghost from a
state that had already left the heap, which a correctly-implemented
mechanism would never do. The original figure is formally withdrawn (see
`CHANGELOG.md`); this is its replacement.

## Corrected collision rate (live-state semantics)

| target | candidates considered | dominated (skipped) | **corrected rate** | inserted | replaced | ghost records dropped on eviction |
|---|---|---|---|---|---|---|
| L1541 | 13,396 | 1,976 | **14.8%** | 10,696 | 724 | 10,267 |
| L984  | 13,400 | 1,414 | **10.6%** | 11,396 | 590 | 11,046 |
| L1640 | 6,078  | 938   | **15.4%** | 4,752  | 388 | 4,399  |
| L4092 | 8,389  | 1,351 | **16.1%** | 6,758  | 280 | 6,422  |

**Corrected range: 10.6%-16.1%**, roughly half the original 27-34% claim --
but still real, nonzero, and consistent across all 4 targets, not a
statistical artifact of one outlier. This is the number that should have
been reported the first time: it answers "of every candidate push that
reaches an already-live open state, how often is it correctly recognized as
dominated" -- not "how often does a candidate coincide with a SMILES string
some earlier, possibly long-evicted, node also once held."

`open_state_records_dropped_on_beam_eviction` being ~5-11x larger than
`open_state_better_replacements` for every target confirms the same
pattern found in Round 2F's dominance-mode run: under real search dynamics,
the overwhelming majority of a live representative's exits from the open
heap are via `beam_prune` eviction, not via being superseded by a
strictly-better arrival -- exactly the exit path the pre-fix code never
reconciled with `open_state_best` (Blocker 1).

## Conclusion

The open-state duplicate-state crowd-out mechanism still has a real,
non-trivial target after the fix -- roughly 11-16% of candidate pushes
genuinely hit an already-live state across all 4 originally-diagnosed
beam-sensitive targets, not near-zero. This justifies proceeding to Round
2G's full clean 100-target formal gate rather than treating the candidate
as moot; it also means the corpus-wide efficacy number should be expected
to be smaller than the original (bug-inflated) headline, consistent with
Round 2F's L1640 finding.
