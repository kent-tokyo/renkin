# L4422 timeout root-cause + corpus-wide overhead characterization (Issue #101 Phase 2, post Round 2G)

## Final disposition (user-confirmed, 2026-08-11)

Round 2G's **"promising-but-gate-miss" verdict is final.** No further
100-target paired gate re-runs; the `>=18/100`/`0 timeouts` thresholds
are not revised post-hoc to make PR #104 pass. PR #104 will **not**
merge. Phase 2 is stopped/parked, not iterated on further -- see
ROADMAP.md's "Parked: ... Phase 2" section for the full reasoning,
including the reranker comparison this finding motivated. The mechanism
insight below stands as the program's takeaway: this is a genuine,
useful negative result (it falsifies "duplicate-state removal is a cheap
crowd-out fix"), not a bug to keep chasing. Of the 3 candidate directions
below, only #1 (`Arc<str>` micro-optimization) is even loosely related to
PR #104's own code, and it is demoted to an optional future
performance-only PR, not a continuation of this program; #2 is a
separate future search-research candidate; #3 does not apply
retroactively to this gate's verdict.

Diagnostic follow-up to Round 2G's gate-miss (`data/phase2g_round2_clean_gate/`,
`route_to_configured_stock` 17/100 vs >=18 required, 1 genuine timeout
`L4422` vs 0 required). ROADMAP.md's Phase 2 section flags "design the
next candidate" as an open question for the user to prioritize -- this is
diagnostic input for that decision, not a proposal being executed. No
fresh 100-target paired gate was run (that is the ~80-minute Round 2G
protocol and is not authorized here); this is entirely (a) a mined
re-analysis of the existing Round 2G gate JSONL and (b) two fresh,
uncapped single-target reruns of `L4422` using the exact Round 2G binary
(`db16e5b...`) and CLI config, to see past the external 150s cap.

## Corpus-wide overhead (mined from `phase2g_round2_clean_gate/*.jsonl`, no new runs)

Of the 99 candidate-arm (`--open-state-dominance`) targets that completed
with diagnostics, wall-clock vs the paired baseline (dominance off), for
the 93 with baseline `wall_clock_s > 0.5s` (excludes near-instant targets
where a percent figure is dominated by noise):

| stat | value |
|---|---|
| n | 93 |
| median slowdown | **+45.5%** |
| mean slowdown | +47.5% |
| min / max | -8.0% / +88.4% |
| fraction with any slowdown | 98.9% |
| fraction with >50% slowdown | 47.3% |

Mean `open_state_dominated_skipped / open_state_candidates_considered`
across the same 99 rows is ~14% (1,726 / 12,015) -- consistent with Round
2A Round 2's corrected 10.6-16.1% live-state collision rate on its
original 4-target sample. **The mechanism prunes a modest, real fraction
of candidates but nearly every target still gets slower, not faster.**

## `L4422` specific (fresh uncapped reruns, `data/l4422_timeout_diagnostics/{baseline,candidate}.json`)

| | baseline (dominance off) | candidate (dominance on) | delta |
|---|---|---|---|
| wall_clock_s | 96.2s (matches Round 2G exactly) | **236.1s** (3:56 wall, uncapped -- Round 2G's 150s external cap had killed this run mid-search) | **+145%** |
| rules_attempted_total | 97,680 | 142,560 | **+46%** |
| candidates_generated_before_dedup | 9,962 | 14,961 | **+50%** |
| beam_prune_invocations | 267 | 332 | +24% |
| nodes_expanded, depth 2/3/4 | 76 / 87 / 78 | 109 / 100 / 98 | +43% / +15% / +26% |
| open_state_dominated_skipped / considered | n/a (flag off) | 2,605 / 17,520 = 14.9% | matches corpus rate |
| route_found | none (both arms) | none (both arms) | -- |

## Root cause: this is real extra search work, not (mainly) bookkeeping overhead

Initial hypothesis (before this data) was that `StateKey::new`'s
per-candidate `Vec<String>` clone-and-sort (`src/search.rs:630`, called
once per candidate whenever `open_state_diagnostics || open_state_dominance`
is set) was the dominant cost -- a pure implementation/allocation
overhead, fixable without touching search semantics. **The data does not
support that as the primary driver.** If the slowdown were dominated by
constant per-candidate bookkeeping, `rules_attempted_total` and
`candidates_generated_before_dedup` should stay roughly flat (same search
trajectory, extra constant cost per node) -- instead both increased by
46-50%, and `nodes_expanded` increased at every depth. That means the
search is doing **genuinely more expansion work**, not the same work
slower.

The mechanism is doing exactly what Phase 1/2 designed it to do: pruning
truly-dominated duplicate states (`continue`s past them in the hot loop,
`src/search.rs:1679-1683`) frees `beam_prune`'s beam-width-100 capacity
from crowd-out, so more **genuinely distinct** candidates survive and get
expanded on subsequent iterations. Each of those extra expansions costs
real rule-matching/candidate-generation work proportional to the
molecule/template corpus, not proportional to the ~15% pruned fraction.
**The coverage benefit (more diverse states explored, a precondition for
finding new routes -- this is where `L4092`'s new solve in Round 2G came
from) and the wall-clock cost are the same effect, not two separable
things.** A pure bookkeeping optimization cannot close a 46-50% increase
in genuine rule-application volume.

`StateKey`'s clone-and-sort cost is still real and orthogonal (present in
`open_state_candidates_considered` ~12,015-17,520 times per target
regardless of how much of that is pruned) but demoted from primary to
secondary suspect by this data -- see "candidate directions" below.

## Implication for "design the next candidate"

Round 2G's two gate misses -- coverage (17 vs >=18) and timeout (`L4422`)
-- are not independent problems to fix one at a time. They are two
expressions of the same mechanism property: the more effectively crowd-out
is fixed (more diverse states survive the beam), the more real exploration
happens, which is simultaneously the source of any future coverage gain
*and* the source of timeout risk on already-hard targets. Pushing further
in the same direction (loosening duplicate-state suppression more) would
plausibly worsen the timeout gate while chasing the coverage gate, not
help both.

## Candidate directions for the user to consider (none started, none implemented)

1. **Cheap, bounded, safe win regardless of which direction is chosen**:
   replace `FEntry.smiles: String` with `Arc<str>` (or otherwise make
   `StateKey::new`'s clone a refcount bump instead of an allocation +
   copy), keeping the existing full-`Eq`/`Hash` `StateKey` design exactly
   as documented at `src/search.rs:620` (do **not** switch to a
   hash-only key -- that doc comment explains that choice was
   deliberate). Secondary-cost fix only; will not by itself resolve
   `L4422`'s timeout given the root-cause finding above, but is a safe,
   semantics-preserving win worth doing regardless.
2. **Threshold/lazy-gated dominance**: only activate pruning once actual
   heap crowding is detected mid-search (e.g. a raw-heap-nodes /
   unique-open-states ratio crossing some threshold), rather than
   tracking every candidate from the first expansion. Would leave
   already-fast/easy targets untouched while still targeting genuinely
   crowded ones -- at the cost of a new tunable threshold, i.e. a real
   design decision requiring judgment, not a mechanical fix.
3. **Re-examine whether the coverage and timeout thresholds are jointly
   achievable for this mechanism at all** -- worth relaying back to
   whoever set `>=18/100` and `0 timeouts` as the pre-registered bar,
   given they may be structurally in tension for a mechanism whose
   entire value proposition is "explore more."

Not run: a fresh 100-target paired gate to validate any of the above --
that is Round 2G's own ~80-minute protocol and needs explicit
authorization before spending it again.
