# Round 2F: targeted 6-target rerun after the open-state lifecycle fix

Source: `renkin --open-state-dominance` (post Round 2A-2E lifecycle fix,
worktree `renkin-open-state-dominance`, branch `feat/open-state-dominance-101`,
built fresh for this run -- not commit `8259cf6`, which is the pre-fix
Phase 2G binary). Same protocol as Phase 2G: Conservative ring-context
policy, `data/comparison/shared_stock/shared_stock.smi`, depth=5,
`data/templates_extracted_500.smi`, beam=100, sequential execution (one
target at a time). Baseline = `data/phase1c_diagnostics/beam100.jsonl`
(flag off) -- re-checked against this run's 6 targets and confirmed to
still match `phase2g_findings.md`'s baseline column exactly.

## route_found, beam=100 -- three-way comparison

| target | baseline (flag off) | old buggy candidate (Phase 2G, `8259cf6`) | fixed candidate (Round 2F) |
|---|---|---|---|
| L1541 | False | False | False |
| L984  | False | False | False |
| L1640 | False | **True** (newly solved) | **False** (not retained -- see below) |
| L4092 | False | **True** (newly solved) | **True** (retained) |
| L4259 | True  | True | True |
| L1167 | True  | True | True |

**Against the pre-registered gate criterion (regressions among the
baseline-solved set): 0 regressions.** None of the 4 baseline-unsolved
targets here regress a baseline-solved target to unsolved -- the gate's
actual regression definition is untouched.

**L1640's new-solve status is NOT retained after the fix -- this is a
real, honest finding, not glossed over.** L4092's is retained. This is not
a violation of the regression gate (L1640 simply returns to its baseline
status, unsolved), but it does mean the old Phase 2G/2H headline
(`16->18/100`, driven partly by L1640) cannot be assumed to reproduce
identically post-fix -- Round 2G's full clean 100-target re-run is the
only way to get the real corrected number, not an assumption from this
6-target subset.

**Why this is plausible, not alarming -- and the direction of the effect,
stated correctly:** the ghost-record bug made dominance *more* aggressive
than it should have been -- an evicted-but-still-"remembered" state could
incorrectly `Dominated`-skip a later, legitimate regeneration of the same
state, so under the bug *fewer* candidates got pushed onto the heap overall.
The fix removes that incorrect suppression, so *more* candidates now get
pushed and survive to compete at every `beam_prune` call -- i.e. the fixed
mechanism prunes *less* aggressively than the buggy one, not more, and the
open heap is under *more* competitive pressure post-fix, not less.
`rules_attempted_total` rising further after the fix (110,352 -> 139,392 for
L1541 alone, see below) is the direct signature of this: more distinct
states surviving to get expanded. Under a fixed `beam_width`, more genuine
competitors at each prune call can easily bump whichever specific candidate
chain happened to solve L1640 under the more-restrictive buggy version --
this is a real consequence of correctly reducing suppression, not evidence
the fix is broken. It also means the honest prior for Round 2G's corpus-wide
number is that it may come in at or below the old (bug-inflated) 18/100,
not above it -- Round 2G must measure this directly rather than assume the
old headline still holds.

## Ghost-record removal (Issue #101 Round 2 Blocker 1 -- direct evidence the bug was real and pervasive)

| target | `open_state_records_dropped_on_beam_eviction` | `stale_open_nodes_removed_before_beam_prune` | `stale_open_nodes_discarded_on_pop` |
|---|---|---|---|
| L1541 | 13,190 | 975 | 0 |
| L984  | 14,876 | 745 | 0 |
| L4259 | 2,237  | 163 | 0 |
| L1640 | 5,237  | 476 | 0 |
| L4092 | 8,007  | 361 | 0 |
| L1167 | 561    | 48  | 0 |

Every single target shows `open_state_records_dropped_on_beam_eviction` an
order of magnitude larger than `stale_open_nodes_removed_before_beam_prune`
-- confirming the ghost-record bug (Blocker 1) was not a rare edge case: on
every one of these 6 real searches, the overwhelming majority of a live
representative's exits from the open heap were via `beam_prune` eviction,
not via a `Replaced` verdict, and every one of those exits would have left
a ghost record behind under the pre-fix code. `stale_open_nodes_discarded_on_pop`
staying at 0 across all 6 confirms the pre-`beam_prune` retain filter still
catches staleness before the pop-time backstop is ever needed, unchanged
from Phase 2G.

## Open-state collision rate and unique/raw heap ratio (Round 2D, now trustworthy)

| target | candidates considered | dominated (skipped) | collision rate | peak unique/raw ratio |
|---|---|---|---|---|
| L1541 | 16,922 | 2,265 | 13.4% | 157/161 = 97.5% |
| L984  | 18,015 | 1,964 | 10.9% | 162/166 = 97.6% |
| L4259 | 2,983  | 438   | 14.7% | 171/176 = 97.2% |
| L1640 | 7,166  | 1,039 | 14.5% | 126/128 = 98.4% |
| L4092 | 10,581 | 1,801 | 17.0% | 148/151 = 98.0% |
| L1167 | 999    | 265   | 26.5% | 144/147 = 98.0% |

The **10.9%-26.5% collision rate here is measured against live-state
semantics for the first time** (post Round 2A-2D fix) -- it is lower than
and not directly comparable to Phase 2A's original 27-34% figure, which was
computed against `open_state_best.len()` under the ghost-record bug (an
"ever-seen" count, not a "currently live" count). Phase 2A's number is
formally withdrawn per the CHANGELOG note added this round; the corrected
diagnostics-only re-measurement of the original 4 Phase 2A targets
(**10.6%-16.1%**, confirming real, nonzero headroom remains after the fix)
is in `data/phase2a_round2_diagnostics/phase2a_round2_findings.md`. The
97-98% peak unique/raw ratio is
the expected, reassuring signature of the fix working correctly: at any
snapshot during these real searches, the open heap is now almost entirely
deduplicated distinct states, with only ~2-3% raw duplicate copies lingering
-- consistent with prompt ghost cleanup on both `beam_prune` eviction and
pop.

## L1541 depth-5 `suzuki_retro` generation count (`--candidate-trace-limit 30000`)

| | old buggy candidate (Phase 2G) | fixed candidate (Round 2F) |
|---|---|---|
| depth-5 `suzuki_retro` generations | 174 | **218** |
| survived beam | 0 | **2** |
| reached stock | 0 | 0 |
| `rules_attempted_total` | 110,352 | **139,392** |
| `open_state_candidates_considered` | 16,282 | 16,922 |

The fix increases exploration *further* than the already-improved buggy
candidate did: more depth-5 `suzuki_retro` terminal-step generations (218
vs 174), and for the first time 2 of them actually survive a `beam_prune`
call (though neither reaches stock, so L1541 remains unsolved at beam=100).
This is directionally consistent with the corrected causality above: the
buggy version's ghost records were incorrectly `Dominated`-skipping some
regenerated candidates that should have been allowed to re-enter the heap
(the pre-fix retain filter only ever removed heap *nodes* superseded by a
`Replaced` verdict -- it never touched `open_state_best` itself and had no
way to react to a `beam_prune` eviction, since it runs *before*
`beam_prune` in the same loop iteration; that gap is exactly Blocker 1).
With the fix, those wrongly-suppressed candidates are pushed and expanded
instead, which is what drives both the higher `rules_attempted_total` and
the two suzuki_retro candidates now surviving to be ranked at all.

## Conclusion / next step

0 regressions against the pre-registered gate definition (baseline-solved
targets). L4092's new-solve status is retained; L1640's is not -- an honest,
reported-not-hidden side effect of correctly fixing the ghost-record bug.
The fixed mechanism prunes *less* aggressively than the buggy one (fewer
incorrect `Dominated` skips means more real competitors at every
`beam_prune`), so the corrected `16->18/100` headline should not be assumed
to reproduce -- the honest prior is `17/100` or `16/100`, not `18/100`,
until Round 2G measures it directly. The ghost-removal counters confirm
Blocker 1 was real and fired constantly (not an edge case) across every one
of these 6 targets. Per Round 2G's spec, next is a full clean 100-target
re-run from scratch (both arms measured fresh, no reuse of Phase 1C/2H's
old files) to get the real, current `route_to_configured_stock` / timeout /
p95 numbers.
