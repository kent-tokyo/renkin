# Phase 2A: open-state dominance diagnostics-only validation

Source: `renkin --open-state-diagnostics` (implies `--search-diagnostics`),
binary/commit recorded in `binary_sha256.txt` (worktree
`renkin-open-state-dominance`, branch `feat/open-state-dominance-101`, base
`8ecac2f` -- the just-merged PR #102). Conservative ring-context policy,
`data/comparison/shared_stock/shared_stock.smi`, depth=5, beam-width=100,
`data/templates_extracted_500.smi`. Raw JSON outputs and `run.log` (elapsed
wall-clock per target under mixed, mostly-idle machine load) alongside this
file.

This pass is **counting-only**: `SearchConfig::open_state_diagnostics`
computes what an open-state dominance decision would be for every
non-terminal candidate push (via a depth-scoped exact-frontier `StateKey`),
but never skips a push, never marks anything stale, and never changes
`nodes_expanded`/route output. Purpose: confirm, with real numbers instead
of one target's hand-inspected candidate trace, that cross-path duplicate
state generation is a real and *general* phenomenon among the beam-sensitive
targets identified in Phase 1D -- not an L1541 idiosyncrasy.

## Results

| target | routes_found | candidates_considered | unique_inserted | dominated_skipped | better_replacements | duplicate rate | peak_raw_heap | peak_duplicate (upper bound) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| L1541 | 0 | 13,396 | 9,310 | 3,046 | 1,040 | 30.5% | 174 | 23 |
| L984  | 0 | 13,400 | 9,829 | 2,778 |   793 | 26.6% | 170 | 11 |
| L1640 | 0 |  6,078 | 4,034 | 1,667 |   377 | 33.6% | 131 | 28 |
| L4092 | 0 |  8,389 | 6,085 | 1,885 |   419 | 27.5% | 165 | 22 |

"duplicate rate" = `(dominated_skipped + better_replacements) /
candidates_considered` -- the fraction of all candidate pushes that landed
on a (frontier, depth) state some other derivation path had already
generated.

`routes_found: 0` at beam=100 for all four matches the Phase 1D taxonomy
exactly: L1541 is `D_non_monotonic_crowd_out` (unsolved at beam=100), and
L984/L1640/L4092 are `beam_limited_monotone_gain` (unsolved at beam=100,
solved only at wider beams) -- this run does not change any of those
classifications, it only adds the new counters.

`stale_open_nodes_discarded_on_pop` and
`stale_open_nodes_removed_before_beam_prune` are `0` for all four targets,
exactly as documented (diagnostics-only mode never marks anything stale).

## Interpretation

Roughly **27-34% of every candidate push, across all four beam-sensitive
targets, lands on a state some other derivation path already generated** --
not just L1541. This generalizes the mechanism documented in
`data/phase1c_diagnostics/L1541_candidate_narrative.md` (candidate-trace-level
inspection of one target) with an aggregate, whole-search-tree measurement
across four.

`peak_duplicate_open_nodes` (11-28) is a smaller, more direct number: at the
single busiest moment observed, that many nodes in the *live, beam-limited*
heap (capped near beam_width=100 by `beam_prune` every round) were either
duplicates of an already-tracked state or otherwise untracked (terminal
nodes; this field is a deliberate upper bound, see its doc comment). Against
a beam of 100, 11-28 "wasted" slots at a single snapshot is a real, material
fraction of the search's total capacity being spent on redundant expansion
targets rather than genuinely distinct candidates -- consistent with the
crowd-out mechanism Phase 1D's candidate trace described for L1541
specifically.

This is the empirical basis for proceeding to Phase 2B/2C (the actual
dominance/replace/stale-removal mechanism, gated behind a new
`SearchConfig::open_state_dominance: bool`, default `false`): the
diagnostics-only pass confirms the target mechanism is real, general across
the four named beam-sensitive targets (not one-off), and large enough
(27-34% of pushes) that removing it plausibly matters for beam capacity,
before investing in the more invasive stale-node/heap-filtering plumbing.
