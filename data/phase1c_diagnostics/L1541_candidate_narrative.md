# L1541 candidate-level crowd-out narrative (Comp-Phase1D evidence)

Target: `uspto50k_test#L1541` = `Cc1ccc(-c2ncc[nH]2)cc1NC(=O)c1ccc(OCc2cc(N3CCOCC3)ccn2)cc1`

**Methodology note (unrelated to this document's own data, which was
collected in isolation and is unaffected):** the Phase1C 100-target x 3-
beam-width *aggregate* sweep (`scripts/phase1c_diagnostic_sweep.py`) was
first run with all three beam-width arms in parallel as three concurrent
background processes. That inflated timeout counts -- renkin uses rayon
internally (~4.5 threads observed for one target in isolation), and three
concurrent arms oversubscribed the machine's 10 cores. Confirmed directly:
L1541 at beam=300 timed out at 150s under 3-arm contention but completed
in 58.6s (455% CPU) run alone. The parallel run's raw output is kept at
`data/phase1c_diagnostics/beam{100,200,300}_parallel_contended.jsonl` for
provenance; it was superseded by a sequential (one-arm-at-a-time) rerun
before Phase1D's taxonomy was finalized. This document's own 9 traces
(L1541/L4259/L1167 x beam 100/200/300) were always collected one at a time
via `/tmp/run_traces.sh` and are unaffected by this issue.

Conditions: Conservative ring-context policy, shared_stock
(`data/comparison/shared_stock/shared_stock.smi`, 393 entries -- **not**
`data/building_blocks.smi`, which an earlier ad hoc verification run in
this session mistakenly used before the discrepancy was caught), depth=5,
templates=`data/templates_extracted_500.smi`, binary
`renkin-crowdout-diag` @ `5d03554236a55cd16a2c5646a2d524386bc81b4f`
(`--candidate-trace-limit 20000`). At beam=100 (9355 candidates generated)
and beam=200 (trace length 18431) the cap was never reached, so those two
traces are complete. At beam=300 (18991 candidates generated per the
aggregate counter, but the `candidate_trace` array itself hit exactly
20000 records -- the cap) the trace **is truncated**: records are
appended in generation order and stop once the cap is reached, so absence
of a candidate late in a beam=300 search is not reliable evidence that it
was never generated, only that it wasn't generated within the traced
prefix. Anything below attributed to beam=300 is qualified accordingly.

Outcome, matching the original v0.21.0 beam-sensitivity gate's
non-monotonic pattern: `route_found` = False (beam=100), **True**
(beam=200), False (beam=300) -- the full non-monotonic 0/1/0 signature is
confirmed under the current binary and the correct shared_stock file.

## The winning route (found at beam=200)

1. `extracted_17`, depth 1: full target → **A** = `c1(N3CCOCC3)ccnc(COc2ccc(cc2)C(=O)O)c1`, **B** = `c1cc(c(cc1-c2[nH]ccn2)N)C`
2. `extracted_46`, depth 2: A → **C** = `c2c(ccnc2CBr)N1CCOCC1`, **D** = `c1(ccc(O)cc1)C(=O)O` (stock)
3. `aryl_amine_retro`, depth 3: B → **E** = `Cc2ccc(cc2)-c1ncc[nH]1`, **F** = `N` (stock)
4. `aryl_amine_retro`, depth 4: C → **G** = `c1cccnc1CBr`, **H** = `C1COCCN1` (stock)
5. `suzuki_retro`, depth 5: E → **I** = `c1c(ccc(C)c1)Br` (stock), **J** = `c1ncc[nH]1` (stock)

## What the candidate trace shows, beam=100 vs beam=200 vs beam=300

| step | template | rank_before_prune | survived_beam | later_reached_stock |
|---|---|---|---|---|
| 1 (depth 1, A+B) | extracted_17 | 0 (beam=100), 0 (beam=200), 0 (beam=300) | True at all three | False, True, False |
| 2 (depth 2, C+D) | extracted_46 | 0 at beam=100/200 | True at both | False → True |
| 5 (depth 5, I+J, the terminal step) | suzuki_retro | -- | -- | 0 matches at beam=100; 1 match at beam=200 (rank 0, survived, reaches stock); 0 matches at beam=300 (**unreliable -- trace truncated at cap, see above**) |

At beam=300 a **second**, distinct heap push of the identical depth-1
`extracted_17` disconnection appears (`rank_before_prune: 330`,
`survived_beam: false` -- evicted, since 330 ≥ 300) alongside the original
rank-0 copy that still survives. The winning top-level plan is still never
evicted outright at beam=300, but the duplicate-state proliferation that
drives the crowd-out (below) is visibly worse at the wider beam: the
`aryl_amine_retro` step-3 disconnection (B → E+F) is pushed 32 times at
beam=300 within the traced (possibly-truncated) prefix, of which only 1
survives -- compare 2/15 survived at beam=100 and 8/21 at beam=200. This
is consistent with route_found reverting to False at beam=300: more duplicate
copies compete for the same beam slots as the search space widens, and
this specific one-per-100-target crowd-out target's chemistry does not
reliably win that competition at any of the three beam widths tested,
even though it can win at beam=200 by chance.

The depth-1 and depth-2 disconnections of the winning route are **never
evicted at either beam width** -- they sit at rank 0 in both. The failure
is not "the correct top-level plan got pruned." The terminal depth-5
disconnection (`suzuki_retro` on E) is **never generated at all** in the
beam=100 run; at beam=200 it is generated exactly once and immediately
reaches stock.

## The actual mechanism: duplicate-state crowd-out, not top-level pruning

The intermediate step 3 (`aryl_amine_retro` on B → E+F) is independently
re-derived **many times** in the trace -- 15 distinct heap pushes at
beam=100, 21 at beam=200, all sharing the identical
`(template_id, sorted precursor_signature)` but differing in `f_score`
(5.69-6.87) because each copy accumulated a different path cost `g` from a
different partial-route derivation order. These are the same logical
state pushed from structurally distinct contexts, not genuine
alternatives.

At beam=100, only 2 of these 15 duplicate copies survive pruning (rank 0,
`survived_beam: true`); at beam=200, 8 of 21 survive. None of the
survivors at beam=100 ever get the chance to expand E via `suzuki_retro`
down to stock within the search budget -- the specific duplicate copy
whose *downstream* expansion would have reached the winning terminal step
is evicted before it is expanded, even though *other* copies of the same
logical state survive. Beam pruning is treating near-identical
partial-route states (same next-step chemistry, different accumulated
cost) as independent competitors for beam slots, rather than recognizing
them as duplicates of one logical search state.

This matches the general shape flagged during Phase 1B review: the
dominant crowd-out mode here is **duplicate-state proliferation across
distinct derivation paths to the same intermediate**, not
same-parent cross-template duplication at one expansion. A same-parent
cross-template dedup counter (Phase 1E candidate option 1, as literally
scoped) would not address this specific mechanism -- it dedups within one
expansion's proposals, not across expansions that arrive at the same
state via different routes. Any Phase 1E option targeting this needs to
dedup (or share cost/beam credit) on the **logical partial-route state**
itself (e.g. a frontier/state hash), not on same-parent siblings.

## Caveat

This is the full-fidelity mechanism for **this one target**. It is not
generalized to the other ~99 targets in the Phase1C sample without
per-target candidate traces (expensive: full traces run ~150s per target
under current machine load). Phase1D's taxonomy for the broader sample
relies on aggregate counters only (see scripts/phase1d_taxonomy.py) and
explicitly does not claim this same mechanism explains every unsolved
target -- most of the sample (73/100 in the original v0.21.0 gate) never
solves at any beam width tested (100/300), which is a structurally
different situation (no beam-crossing ground truth exists to compare
against).

## L4259 and L1167: status under the corrected stock file + current binary

These two were selected (per the original v0.21.0 beam-sensitivity gate)
to represent two other phenomena for contrast with L1541's crowd-out:
L4259 as "beam-limited but monotone" (`route_found` False, True, True
across beam 100/200/300 in the old gate) and L1167 as an always-solved
control (True, True, True).

Re-measured here on the current binary (`5d03554`, includes PR #91's
spectator-atom aromaticity fix, which landed between v0.21.0 and v0.21.1)
with the same corrected shared_stock file: **L4259 now solves at all
three beam widths** (`route_found: True` at beam=100/200/300), not just
beam≥200 as in the old gate. L1167 remains solved at all three beam
widths, as before.

L4259's shift is plausibly attributable to PR #91 (a real, intentional
behavior change to fix a correctness bug, landed and released in v0.21.1,
well before this diagnostics work) rather than to anything in this
diagnostics-only branch -- consistent with this branch's own verified
byte-identical-output guarantee relative to its v0.21.1 base. It was not
re-diagnosed further given the scope of this program (Phase 1 is
diagnostics + one candidate PR selection, not a re-litigation of already-
merged, already-released correctness fixes). Practically, this means only
L1541 exhibits genuine beam-width crowd-out in the current binary among
the three named targets; L4259 and L1167 now both serve as positive
controls showing what a "no crowd-out" trace looks like (their top-level
disconnections are generated, survive, and reach stock consistently
across all three beam widths -- available in
`data/phase1c_diagnostics/traces/{L4259,L1167}_beam{100,200,300}.json`
for anyone who wants to inspect them, but not narrated further here).
