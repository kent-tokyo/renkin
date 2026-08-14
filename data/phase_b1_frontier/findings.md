# Phase B.1 -- coverage/cost frontier optimization

User GO: 2026-08-12 (detailed 10-step pre-registration, verbatim in
session transcript / `ROADMAP.md`). Supersedes the earlier draft
pre-registration placeholder from the same day.

**Sequencing decision, per GO**: don't run the raw 5k/10k route-search
gate first. Phase A.5 already proved template-diversity scaling works
(34.0% -> 17.6% zero-positive, +16.4pp) but at a generation-cost cost of
~28-30x (500->9,979). Route-searching on unindexed 5k/9,979 template
pools risks the whole experiment being dominated by match/apply cost
rather than the coverage question. So: attribution/profile first (Step
1) -> semantics-preserving retrieval index (Step 2-3) -> correctness gate
(Step 4) -> speed gate (Step 5) -> only then route-search gate (Step
6-9).

Scope guard for the whole phase: **VAL/development data only. Formal
TEST (4,903 targets) stays unused until a production template count,
retrieval design, and reranker policy are all frozen** -- reserved for
exactly one final confirmation run, not iterative use.

## Discovery (2026-08-12, before Step 1 started)

The "template retrieval index" the GO calls for is **not new work**.
`TemplateBondIndex` (element-pair bond-center index) and
`ProposalMode::BondIndexed` already exist in `src/chem_env.rs` /
`src/candidate.rs`, dating to very early project history (~v0.3-0.7).
Already wired into the main `renkin` search CLI and `renkin-bench` via
`--bond-index`, with a documented but unverified-at-this-scale claim:
`--bond-index Bond-center template index: ~24% faster, no accuracy
loss` (`src/main.rs` help text). That figure's provenance predates this
whole coverage program and was almost certainly measured against the
old 500-template default, at the route-search level, not the
one-shot-per-target pool-gen level Phase A.5/B.1 use.

**Not wired into `renkin-pool-gen`** (the tool Phase A.5's entire
measurement program used) before this phase. No existing test proves
`BondIndexed` is a lossless filter (zero false negatives) at any scale,
let alone 5,000/9,979 templates.

Consequence: Steps 2-3 are "wire the existing index into pool-gen, then
validate/extend it at 5k/9,979 scale," not "design and build a new
retrieval structure." Re-uses proven library code instead of duplicating
it.

## Step 1: cost attribution profiling

**Method**: added wall-clock phase timers to
`CandidateProposalContext::propose_one_step` (`src/candidate.rs`),
gated behind the existing `perf-instrumentation` feature (same pattern
as `chem_env::apply_retro_call_count` -- zero overhead, zero shared-atomic
contention in production builds; off by default). Three phases:

- `select` -- rule selection (`select_active_rules`; for `Exhaustive`
  mode, effectively "return every rule").
- `raw_propose` -- `raw_propose`'s parallel per-active-rule work: for
  each rule, chematic's `apply_retro_with_policy` (SMARTS match +
  reaction application fused into one call; RENKIN has no
  instrumentation boundary between those two sub-steps, and chematic
  doesn't expose one either -- this bucket is the combination, not a
  further split).
- `merge` -- `merge_into_candidates`'s canonicalize/dedup pass.

Also wired `--bond-index` / `--bond-index-top-k` into
`renkin-pool-gen` (`src/bin/pool_gen.rs`) for Step 2-5's use, and added
per-target wall-clock (`proposal_seconds_per_target_p50/p95/max`,
driver-level, not feature-gated -- a one-shot CLI tool's own loop, not
the search hot path) for Step 5's speed gate.

**No behavior change**: `Exhaustive` mode output is unaffected;
`propose_phase_nanos()` returns all-zero unless
`--features perf-instrumentation` is compiled in.

**Sample**: reused Phase A.5's existing fixed 500-target VAL subset
(`data/phase_a5_template_scaling/stage_500/{arm}_groups.jsonl`,
deterministic, already-established) and its exact template files
(`data/phase_a5_template_scaling/templates/templates_{arm}.smi`), rather
than generating a new sample -- also gives a free cross-check against
Phase A.5's own Stage 2 numbers.

**Result** (`data/phase_b1_frontier/step1_profile/`, `--features
perf-instrumentation`, unindexed/`Exhaustive` only):

| templates | wall-clock | select | raw_propose | merge | raw_propose share |
|---|---|---|---|---|---|
| 500 | 44.11s | 0.0007s | 43.53s | 0.148s | 98.67% |
| 5,000 | 792.71s | 0.0043s | 791.44s | 0.372s | 99.84% |
| 9,979 | 1,531.25s | 0.0103s | 1,529.34s | 0.489s | 99.87% |

`n_candidate_rows` at every arm (13,643 / 33,282 / 44,017) matches
Phase A.5's Stage 2 500-target measurement exactly, cross-validating
that this run's pipeline (same sample, same templates, same rules
loader) reproduces the original result byte-for-byte in substance
before trusting the new timing numbers layered on top.

**Conclusion: `raw_propose` (SMARTS-match + reaction-application,
fused) is essentially the entire cost at every scale (98.7% -> 99.9%),
not merely "most of it."** `select` and `merge` are both negligible in
absolute terms even at 9,979 templates (10ms and 489ms respectively,
against a 1,531s total). This upgrades the 2026-08-12 addendum to
`data/phase_a5_template_scaling/findings.md` (an aggregate-counter
*inference* from wall-clock/candidate-count ratios, "already
well-supported without profiling") to a **direct instrumented
measurement**: match-time dominance isn't inferred anymore, it's
measured. `select_active_rules`'s own cost stays trivial regardless of
template-set size, meaning a retrieval index's *own* overhead is cheap
-- the entire benefit case rests on how many active rules it excludes
before they reach `raw_propose`'s expensive per-rule call, which is
exactly what Step 2-5 tests next.

## Step 2-3: wire the existing retrieval index into `renkin-pool-gen`

Done as part of the Step 1 code change (see above) -- `--bond-index`
selects `ProposalMode::BondIndexed { top_k: bond_index_top_k }`
(default `top_k=0`, no truncation, matching `find_routes`'s own usage)
via the pre-existing `TemplateBondIndex`; omitting the flag keeps
`ProposalMode::Exhaustive`, byte-identical to pre-Phase-B.1 behavior.
`CandidateProposalContext::new(&rules, bond_index)` builds the index
once per process (not once per target), matching the context's existing
design intent.

Purpose, per the GO: exclude templates that are *definitely* inapplicable
before the expensive `raw_propose` call, not prune candidates.
`TemplateBondIndex::retrieve` always includes graph-based and
fallback-classified rules unconditionally (see `src/chem_env.rs`) and
only narrows the SMIRKS-rule set by element-pair bond presence --
narrowing inputs to `raw_propose`, never the candidate output directly.

## Step 4-5: correctness and speed gates

Status: in progress, `data/phase_b1_frontier/step4_5_gate/`.

500-template arm (unindexed then indexed, sequential, same session):
`n_candidate_rows` and `target_group_index_sha256` identical between
arms (13,643 rows, matching hash) -- candidate-pool output is unaffected
by indexing at this template-set size, as expected (correctness holds).
Speed at 500 templates is not the interesting number (the GO's speed
gate targets the 5k arm specifically) but recorded for completeness:
wall-clock 53.28s -> 47.91s (~1.11x), p50 per-target proposal cost
0.0701s -> 0.0706s (flat -- retrieval overhead roughly cancels matching
savings for typical/small targets at only 500 templates), p95 0.3015s
-> 0.2410s (~1.25x, the tail benefits more). This 500-template unindexed
p95 (0.3015s) is the "500-template baseline" the GO's `<=6x`/`<=4x`
per-target p95 floor is measured against.

**5,000-template arm result (2026-08-12): correctness PASSES, speed
gate FAILS -- STOP per the pre-registered rule.**

Correctness (unindexed vs. indexed, same 500-target sample, same
session): `n_candidate_rows` identical (33,282 = 33,282),
`target_group_index_sha256` identical, `candidate_id` set **EXACT**
match (0 only-in-unindexed, 0 only-in-indexed -- zero false negatives),
source-template-id-set **EXACT** match on every row (0 mismatches,
mode-relative rank fields like `best_upstream_rank` excluded from this
comparison since they're documented as mode-relative by design, not a
correctness signal -- see Step 1 smoke-test note above).
`n_groups_parse_failed`/`n_groups_target_id_mismatch`/
`n_groups_zero_candidates` all identical (0/0/0 both arms). Determinism
not independently re-run at this scale (would cost another ~700s for a
property that's structurally guaranteed -- `TemplateBondIndex::retrieve`
is a pure function with no RNG, and `candidate_rows` is explicitly
sorted before every write) -- **correctness gate: PASS.**

Speed (same session, unindexed then indexed, back-to-back, nothing else
running concurrently):

| | unindexed | indexed | ratio |
|---|---|---|---|
| wall-clock | 792.60s | 697.14s | **1.14x** |
| per-target p50 | 1.164s | 1.080s | 1.08x |
| per-target p95 | 4.385s | 3.595s | 1.22x |

Pre-registered speed-gate thresholds: `>=3.0x` strong GO / `>=2.0x` GO /
`1.5-2.0x` weak-redesign / `<1.5x` STOP. **1.14x falls in the `<1.5x`
STOP band.** Additional floor (5k per-target p95 vs. the 500-template
unindexed baseline of 0.3015s): 3.595 / 0.3015 = **11.9x**, far above
even the `<=6x` minimum-GO floor, let alone the `<=4x` strong-GO floor.

**Root cause, confirmed by direct measurement** (temporary diagnostic,
not committed -- `TemplateBondIndex::retrieve` called directly against
5 representative drug-like SMILES at the 5,000-template scale):
`retrieve()` excludes only **0.7%-5.2%** of rules per target (94.8%-99.3%
still retrieved and sent to `raw_propose`). This is architectural, not a
tuning issue: `bond_pairs_from_smirks` extracts *every* adjacent
bracket-atom pair from a rule's LHS (not just its actual reaction-center
bond), and `retrieve()` unions in a rule if the target contains **any
one** of those pairs anywhere (OR/union semantics, not AND/intersection).
Since common organic bonds (C-C, C-N, C-O, ...) appear in nearly every
rule's LHS *and* nearly every drug-like target, almost every rule
matches on at least one common pair regardless of whether the target
actually has the rule's specific, rarer reaction-relevant bond. This
also explains Step 1's `select` phase staying cheap even under
`BondIndexed` (0.314s across 500 targets at 5,000 templates) --
`retrieve()` itself is fast, it just doesn't exclude much.

Per the GO's own pre-registered rule (`<1.5x -> current indexing案は
STOP`): **the existing `TemplateBondIndex` does not clear Phase B.1's
speed gate at 5,000 templates.** Not re-running the 9,979-template
pair -- the mechanism above (common-element-pair saturation) only gets
*worse* as template count grows (more rules, same handful of common
element pairs), so a weaker result there would add no new information at
~50 more minutes of compute. Per the GO, this stops the current
retrieval-index approach here; Step 6 (route-search gate) is **not**
started with this index. Reported to the user for a redesign-direction
decision rather than picked unilaterally -- see options below.

**Candidate redesign directions (not started, need a decision)**:
- **AND/intersection semantics**: require *all* of a rule's bond-pairs
  present in the target, not any one -- directly targets the diagnosed
  OR-union weakness, smallest change to existing code.
- **Reaction-center-only indexing**: key each rule only by its actual
  changed/broken bond (already computed elsewhere for ring-context
  purposes, see `src/ring_context.rs`), not every LHS adjacent pair --
  more surgical exclusion, larger change.
- **A different structural filter entirely** (e.g. required-element
  bitmask already exists per-rule via `RetroRule::required_elements` and
  is applied inside `raw_propose` today -- extending that idea with a
  richer per-rule fingerprint/substructure prefilter instead of
  bond-pairs).

**User chose AND/intersection semantics (2026-08-12).** Implemented as
`TemplateBondIndex`'s new retrieval rule: a SMIRKS rule is now retrieved
only if *every* element-pair bond in its `bond_pairs_from_smirks`-derived
LHS requirement set is present somewhere in the target (the coarse "any
one pair present" union index is kept only as a cheap candidate-superset
prefilter -- every AND-eligible rule necessarily appears in it, so no
correctness risk from reusing it that way). This is still a lossless
exclusion rule in principle: if a rule's LHS pattern needs an
element-pair bond type the target has zero occurrences of anywhere, no
SMARTS match against that pattern can succeed.

**Bug found and fixed as a correctness prerequisite**: `bond_pairs_from_smirks`
didn't reset its adjacency tracking at `.` (multi-component) boundaries,
so a disconnected LHS's last atom of one fragment and first atom of the
next would be recorded as a spurious bonded pair. Under the old OR/union
semantics this was harmless (a spurious *extra* pair only ever made a
rule *more* likely to be retrieved). Under the new AND/subset semantics
it would have been a real false-negative risk: a spurious pair becomes a
fabricated requirement, and a target correctly missing that
never-really-required bond would get wrongly excluded. Checked before
relying on this: 5 of 9,979 extracted templates have a multi-component
LHS (all 24 hand-crafted rules' LHS are single-component, unaffected).
Fixed by resetting the parser's adjacency state at `.` -- confirmed safe
via the 500/5,000-arm EXACT correctness re-runs below (0 false
negatives either arm, including the affected 5 templates being part of
the 5,000/9,979 sets).

**5,000-template arm result (2026-08-12): correctness PASSES again,
speed gate FAILS again -- second consecutive STOP.**

Correctness: `n_candidate_rows` identical (33,282 = 33,282),
`target_group_index_sha256` identical, `candidate_id` set **EXACT**
(0 false negatives), source-template-id-set **EXACT** (0 mismatches).

Speed (same session, back-to-back against the already-recorded 5,000
unindexed baseline from the OR-semantics round -- `Exhaustive` mode is
untouched by this change, so that baseline is still valid):

| | unindexed | AND-indexed | ratio |
|---|---|---|---|
| wall-clock | 792.60s | 670.04s | **1.18x** |
| per-target p95 | 4.385s | 3.656s | 1.20x |

Still in the `<1.5x` STOP band. p95-vs-500-baseline floor: 3.656 /
0.3015 = **12.1x** (essentially unchanged from OR semantics' 11.9x, both
far past the `<=6x` minimum).

**Why the improvement is so much smaller than the exclusion-fraction
diagnostic suggested**: the first diagnostic (5 hand-picked drug-like
SMILES) showed 68-70% average retained under AND semantics, which
looked like a large improvement over OR's 94.8-99.3%. Re-measured
against the *actual* 500-target gate sample (not hand-picked molecules):
**81.0% average retained (median 88.6%, p95 94.9%)** -- the hand-picked
sample was materially more favorable than the real USPTO-50k-derived
VAL distribution. At ~81% retained, a wall-clock ratio in the 1.1-1.2x
range is roughly the right order of magnitude (consistent with
per-rule cost being reasonably uniform, not with retained rules being
systematically cheaper or more expensive than excluded ones). Net:
**element-pair granularity itself -- ignoring bond order, topology, and
aromaticity, counting only "this atomic-number pair appears somewhere
in both" -- is too coarse to provide strong exclusion for a broad-
coverage template library, regardless of OR vs. AND combination logic.**
Common organic bond types (C-C/C-N/C-O/aromatic-C-C) are required by
most templates and present in most drug-like targets; only genuinely
unusual bond types (rare heteroatoms/halogens) get excluded either way.

**Two STOPs in a row on the two smallest-effort redesign options.**
Not attempting the two larger-effort options (reaction-center-only
indexing, richer structural fingerprint) unilaterally -- reported back
for a direction decision rather than picked, matching the same judgment
call as the first STOP. The pattern across both attempts suggests the
ceiling may be inherent to *any* element-pair-only filter, not a
tuning gap fixable with more combination-logic tweaks -- worth weighing
before investing in the larger redesigns versus reconsidering whether a
retrieval index is the right lever at all for this template-count range.

## Pivot (user decision, 2026-08-12): reconsider the whole approach

User chose, after the second STOP: stop trying to make 5,000+ templates
cheap via indexing, and instead pick a production template count
directly from Phase A.5's already-measured coverage/cost numbers --
no new pool-gen runs needed, all 5 points were already measured in
Phase A.5's Stage 3 (full VAL).

**Coverage-gain-per-extra-cost-unit frontier** (pp absolute zero-positive
reduction vs. 500, per multiple of 500's pool-gen wall-clock beyond 1x):

| templates | zero-positive | pp gain vs. 500 | cost multiple | pp per extra-cost-unit |
|---|---|---|---|---|
| 500 | 34.0% | -- | 1.00x | -- |
| 1,000 | 27.7% | -6.3pp | 1.92x | **6.82** |
| 2,000 | 23.4% | -10.6pp | 4.44x | **3.08** |
| 5,000 | 19.8% | -14.2pp | 14.87x | 1.02 |
| 9,979 | 17.6% | -16.4pp | 27.76x | 0.61 |

Returns drop off a cliff after 2,000 -- 1.02 and 0.61 pp/cost-unit for
5,000/9,979 vs. 3.08-6.82 for 1,000/2,000. **2,000 templates is the
smallest template count that independently clears the same `>=10pp`
absolute-improvement bar Phase A.5 itself used to call Phase B a "strong
GO"** (-10.6pp, just past the threshold), at a 4.44x pool-gen cost
multiple -- a small fraction of 5,000's 14.87x or 9,979's 27.76x for a
majority of the total achievable coverage gain (10.6 of the full 16.4pp
range, 65%, at only 4.44 of the full 27.76x cost range, 16%). 1,000
templates has the single best per-unit ratio but doesn't clear the
`>=10pp` bar on its own (-6.3pp).

**Recommendation: 2,000 templates as the primary production candidate**,
to be validated the same way Step 6-7 originally intended (paired
route-search gate vs. 500, reranker OFF, same fixed development sample)
-- but now directly, without any retrieval-index dependency, since
2,000's raw pool-gen cost (1,855s full-VAL / ~4.4x of 500's) is modest
enough that its route-search-level cost is plausibly tractable without
indexing (unconfirmed -- exactly what the next gate measures). User
confirmed: validate 2,000 only (not 1,000+2,000, not 5,000/9,979).

## Step 6-7: route-search gate, 500 vs. 2,000, reranker OFF (150s timeout)

**This verdict is final and is not superseded by the widened-timeout
follow-up below (per user direction, 2026-08-13).** At the pre-
registered 150s timeout, the gate **FAILED** -- see the result table
further down. A later run at a widened timeout does not retroactively
make this gate "actually PASS"; it is recorded as a separate experimental
condition (**Phase B.1b**, below), not a re-test of this one.

**Setup**: `scripts/compare_run.py` (this repo's existing paired
route-search-comparison harness, previously used for the beam-
sensitivity-gate and the reranker runtime gate) -- `--comparison-mode
shared_stock` (393-compound cross-arm-fair stock, same precedent as
those two gates), `--beam-width 100`, depth 5 (default), 150s timeout +
10s grace (default), `--ring-context-policy` left at its default
(`disabled` -- matches Phase A.5's own pool-gen methodology, which never
used the ring-context guard either; using anything else here would
inject a variable Phase A.5's coverage numbers never accounted for),
reranker OFF (no `--reranker-model`/`--reranker-freq-table`).

**Sample -- important scope-safety note**: `compare_run.py`'s *default*
`--sample-list` (`data/comparison/sample_full_sorted.jsonl`) is drawn
from `uspto50k_test` (confirmed by inspecting its `target_id` values,
e.g. `"uspto50k_test#L3855"`) -- the formal TEST split this whole
program is explicitly reserving for exactly one final confirmation run.
Generated a VAL-derived equivalent instead
(`data/phase_b1_frontier/val_sample_full_sorted.jsonl`, 4,924 rows,
deduplicated by canonical SMILES from the already-established
`data/reranker_groups_uspto50k_val.jsonl`, same schema `compare_run.py`
expects) and used that for `--sample-list` throughout. Formal TEST was
not touched by this gate.

**Resilience note**: `compare_run.py --resume` flushes and `fsync`s
every completed target's row immediately, so a kill loses at most the
in-flight target, never completed ones -- the 2,000-template arm was
in fact killed once mid-run (60/100 done) and resumed cleanly to 100/100
with a second invocation of the identical command.

**Results (2026-08-12/13, 100 targets each, same VAL sample)**:

| | 500 (baseline) | 2,000 | delta / ratio |
|---|---|---|---|
| `route_to_configured_stock` | 18/100 | 21/100 | **+3pp** |
| invalid/unparseable | 0 | 0 | 0 |
| solved-target regressions | -- | 2/100 | **2%** |
| timeout rate | 2/100 | 23/100 | **+21pp** |
| total-elapsed p50 | 16.6s | 70.3s | **4.23x** |
| total-elapsed p95 | 96.8s | 150.0s | 1.55x (censored, see below) |

**Verdict against the pre-registered thresholds: FAIL.** Checked
against every Step 7 condition:
- `route_to_configured_stock >=+3pp` -- **marginal PASS** (exactly
  +3pp, right at the boundary, not comfortably above it).
- `invalid/unparseable = 0` -- PASS.
- `solved-target regression <=1%` -- **FAIL** (2%, double the allowed
  rate). Per-target check: `uspto50k_val#L66` regressed by timing out at
  2,000 templates where it completed (unsolved but not timed out) at
  500; `uspto50k_val#L6` regressed via a different search outcome at the
  larger template set despite both runs completing within budget --the
  same candidate-ordering/crowd-out pattern documented elsewhere in this
  project's history (Issue #101, `L1541`) -- more templates is not a
  strictly monotonic improvement once beam/ordering interactions are in
  play.
- `timeout-rate increase <=+1pp` -- **FAIL, badly** (+21pp, 21x the
  allowed threshold). 2,000 templates pushed roughly a quarter of all
  sampled targets to the 150s wall, up from 1 in 50 at 500 templates.
- `p95 latency <=2.5x` -- nominal PASS (1.55x) but **this number is
  misleading**: 2,000's p95 (150.0s) is sitting almost exactly at the
  timeout ceiling, i.e. right-censored -- it cannot show how much worse
  the true latency distribution is beyond that wall, only that a large
  fraction of runs hit it. The median (p50) is the more honest signal
  here and it more than quadrupled (4.23x), consistent with the timeout-
  rate blowup, not with a merely "somewhat slower" search.

**Conclusion: 2,000 templates, unindexed, is not production-viable
under this beam-width/timeout configuration.** The coverage gain (+3pp)
barely clears its own bar while the cost side fails by a wide margin on
two independent, more decisive metrics (timeout rate, regression rate).
This directly confirms the concern Phase A.5's own findings flagged and
the reason Phase B.1 tried retrieval-indexing *first*: route-search
cost (templates matched repeatedly per search-tree node) does not scale
the way pool-gen cost (templates matched once per target) predicted --
even at a "modest" 4.4x pool-gen-cost template count, real route-search
cost blew up far more than that. Neither retrieval-indexing (two
attempts, both STOPped on speed) nor skipping indexing (this gate,
STOPped on timeout/regression) has yet produced a viable path past 500
templates for route search specifically, even though the *candidate-
pool* coverage benefit (Phase A.5) is real and substantial at every
tested template count. Reported to the user rather than picked
unilaterally -- see options going forward.

## Diagnosis (user choice, 2026-08-13): why does route-search cost blow up worse than pool-gen predicted?

User chose to diagnose the mechanism directly (cheap, VAL-only, no new
production-config attempt) before choosing among: try 1,000 templates,
invest in a bigger-effort index redesign, or conclude Phase B.1 as a
negative result.

**Method**: reused already-collected data first (no new runs): every
`compare_run.py` row already carries `tool_specific.renkin.{nodes_expanded,
matched_templates, beam_limit_hit, max_depth_reached}` from the normal
JSON output (no `--search-diagnostics` needed for these fields). Compared
distributions across the 500 vs. 2,000-template arms for the subset of
targets that completed (not timeout) in *both* arms:

| | 500 (n=80 measured) | 2,000 (n=56 measured) | ratio |
|---|---|---|---|
| `nodes_expanded` mean | 253.8 | 263.5 | **1.04x** |
| `matched_templates` mean | 11,498.7 | 19,282.6 | 1.68x |

`nodes_expanded` -- the number of search-tree states actually
explored -- is essentially **flat**. This rules out the crowd-out/
state-survival explosion mechanism documented elsewhere in this
project's history (the PR #104 open-state-dominance pattern, where more
diverse surviving states directly inflate node count): that is not what
is happening here. But this comparison has a blind spot -- `timeout`
rows carry an *empty* `tool_specific` (confirmed by inspection: the
external `/usr/bin/time` wrapper kills the process before it can print
its own JSON, so nothing is recoverable from those rows) -- meaning the
23 newly-timed-out-at-2,000 targets, the ones actually driving the gate
failure, are invisible to this comparison by construction.

**Follow-up: direct standalone measurement of one representative
target** (`uspto50k_val#L66`'s neighbor `uspto50k_val#L5`, chosen because
it completed in a moderate 45.8s at 500 templates and was one of the
newly-timed-out cases at 2,000 -- reproduced via the exact same CLI
invocation `scripts/compare_renkin_adapter.py` uses, `--max-routes 1`,
confirmed matching the recorded row's `nodes_expanded=262`/
`matched_templates=13933` exactly), re-run at 2,000 templates with a
generous 400s bound instead of the gate's 150s wall:

| | 500 templates | 2,000 templates | ratio |
|---|---|---|---|
| wall-clock | 45.8s | 185.5s | **4.05x** |
| `nodes_expanded` | 262 | 297 | 1.13x |
| `matched_templates` | 13,933 | 28,217 | 2.03x |

**It completed.** Not a runaway/unbounded search -- given enough time,
this specific target's search terminates (still unsolved,
`routes_found=0`, same as at 500 templates) in a bounded, roughly
4x-longer time. `nodes_expanded` growing only 1.13x (not exploding)
confirms the aggregate finding above at the individual-target level.

**Conclusion: the mechanism is per-node cost scaling with template
count (the same mechanism Step 1 already characterized for pool-gen),
multiplied by a roughly constant number of nodes per target (~250-300
for this beam-width-100/depth-5 config) -- not a pathological,
super-linear search-behavior explosion.** The ~4x wall-clock ratio
observed here is in the same ballpark as pool-gen's own 500->2,000
ratio (4.44x, full VAL) -- reasonably close given this is one target,
not an aggregate. **The 2%->23% timeout-rate jump is best explained as
a roughly-linear ~4x per-target cost increase colliding with a FIXED
150s timeout wall calibrated for the old (500-template) cost
distribution** -- targets that used to comfortably finish in 40-140s
now need 160-560s, and a wall that didn't move catches a lot more of
them. This is a real production cost increase either way (whether it
manifests as "timeout" or "just much slower"), but it is a
**straightforward, roughly-predictable cost multiplier, not evidence
that route-search behavior itself degrades pathologically with more
templates.**

**What this implies for the options going forward**: a retrieval index
would need to deliver something like a 3-4x+ per-node speedup to fully
cancel this cost increase and keep 2,000-template route search inside
the current timeout budget -- **far beyond what either tried redesign
achieved (1.14x, 1.18x)**. Both index attempts are now known to be
roughly an order of magnitude short of what this specific problem
needs, not just "somewhat short." Separately, since `nodes_expanded`
stays flat and the search does terminate (not runaway), simply
widening the timeout budget is a legitimate, different lever (a
latency/throughput product tradeoff, not an engineering fix) that this
data supports as viable *if* a several-times-longer per-target search
time is acceptable in production.

## Phase B.1b: increased-compute operating point (user direction, 2026-08-13)

**Framing, exactly as the user specified**: this is a new experimental
condition, not a re-test of Step 6-7's gate. The 150s-timeout gate's
**FAIL verdict stands, permanently** -- a later PASS at a wider timeout
does not retroactively change it. What Phase B.1b asks is a different
question: *given the diagnosed structure (template count 4x -> nodes
expanded ~1.04x -> per-node cost ~4.4x, a clean, non-pathological
multiplier), is the coverage gained at the higher compute point actually
worth the extra compute?* Not merely "did timeouts go away."

**The same 5 pre-registered thresholds apply, unchanged, no exceptions**:
`route_to_configured_stock >=+3pp`, invalid/unparseable `=0`, solved-
target regression `<=1%`, timeout-rate increase `<=+1pp` (relative to
whatever the 500-template arm's own timeout rate is *at the same widened
timeout*), p95 route-search latency `<=2.5x`. **`p95 <=2.5x` is
deliberately not relaxed to 4x/5x to accommodate the wider timeout
window** -- doing so would mean chasing the gate to fit a predetermined
answer rather than measuring against a fixed bar.

**Setup**: both arms (500 and 2,000 templates) re-run fresh at
`--timeout-s 600 --grace-s 20` (4x the original budget) on the identical
VAL sample -- both arms, not just 2,000, so the timeout-rate/regression/
latency comparison stays a clean apples-to-apples pair rather than an
asymmetric one. Output in `data/phase_b1_frontier/step7_widened_timeout/`.

**User's own prediction going in**: p95 is expected to be the hardest
criterion to clear -- if per-node cost really is ~4.4x with nodes
staying flat, removing the timeout ceiling should let 2,000's true p95
latency "reveal itself" at close to 4x, not the 1.55x the timeout-
censored 150s data showed. That would not be a failure of this
diagnosis -- it would be exactly what a clean, honest measurement should
show, matching the mechanism already confirmed above.

**Decision tree for the result** (pre-registered before running, per the
user's instruction):
- All 5 criteria PASS -> freeze 2,000 as the production candidate,
  proceed to Step 9 (reranker ON compatibility gate).
- Coverage PASS, timeout/regression resolved, but p95 `>2.5x` -> **"2,000
  templates is coverage-valid but outside the current production
  frontier under this matching architecture."** Next candidate: 1,000
  templates (Phase A.5: -6.3pp vs. 500, a smaller but real coverage gain,
  at proportionally lower raw template-matching cost than 2,000).
- Coverage `<+3pp` -> the one-step candidate-pool coverage gain (Phase
  A.5) does not convert well enough into route-level coverage; 2,000-
  template scaling is weak specifically for production route search
  (independent of the cost question).
- Regression `>1%` -> not necessarily just a timeout artifact; re-
  diagnose the specific newly-unsolved targets individually (matching
  how `L6`/`L66` were separated out in the 150s gate's own regression
  analysis above) before concluding anything about coverage or cost.

**Step 9 (reranker ON compatibility) stays not started until a
production template count is frozen from this decision tree** -- testing
reranker+2,000 now would be discarded work if 2,000 fails and the next
candidate becomes 1,000.

**Result (2026-08-13, both arms complete, 100/100 each):**

| | 500 @ 600s | 2,000 @ 600s | delta / ratio |
|---|---|---|---|
| `route_to_configured_stock` | 18/100 | 21/100 | **+3pp** |
| invalid/unparseable | 0 | 0 | 0 |
| solved-target regressions | -- | 2/100 | **2%** |
| timeout rate | 0/100 | 1/100 | **+1pp** |
| total-elapsed p50 | 17.8s | 59.6s | 3.35x |
| total-elapsed p95 | 110.1s | 260.0s | **2.36x** |

Against the unchanged, unrelaxed thresholds:
- `route_to_configured_stock >=+3pp` -- **PASS** (marginal, exactly at
  the boundary again).
- `invalid/unparseable = 0` -- PASS.
- `solved-target regression <=1%` -- **FAIL** (2%, still double the
  allowed rate -- unchanged from the 150s gate, see root-cause below).
- `timeout-rate increase <=+1pp` -- **PASS** (marginal, exactly at the
  boundary; down from the 150s gate's severe +21pp).
- `p95 latency <=2.5x` -- **PASS** (2.36x). Notable: the user's own
  prediction going in was that this would be the hardest criterion and
  might reveal something closer to the ~4.4x per-node cost ratio once
  uncensored. It didn't -- p50 (3.35x) grew *faster* than p95 (2.36x),
  the opposite of the usual pattern. Read together with the per-target
  regression diagnosis below: the very hardest targets (contributing to
  p95) tend to already be bounded by state-space exhaustion at a
  roughly fixed node count (matching the earlier `nodes_expanded`
  finding) rather than continuing to scale with template count the way
  a typical (median) target's search does -- so the tail grows *less*
  proportionally than the middle of the distribution, not more.

**4 of 5 criteria pass cleanly. The sole failure is regression, and it
is not a timeout artifact.** Both regressing targets (`uspto50k_val#L6`,
`uspto50k_val#L66`) were `route_found=true` at 500 templates (4-5 step
routes) and, at 2,000 templates with the full 600s available, complete
within budget but end unsolved -- `beam_limit_hit: true` and
`max_depth_reached: true` on both. **Root cause: fixed-beam-width
(100) crowd-out.** More templates means more candidate branches compete
for the same 100 beam slots at every node; the specific state trajectory
that led to the solution at 500 templates loses that competition at
2,000 templates in favor of other newly-available candidates that don't
pan out. This is not a bug and not measurement noise -- it is the same
mechanism this project has already documented in other contexts (the
Issue #101 `L1541` non-monotonic case, the PR #104 open-state-dominance
investigation's "more diverse surviving states costs more and can
displace the eventual winner" finding), now observed in the opposite
direction (more *templates*, not more *states surviving pruning*,
crowding out a previously-winning path). Net effect at the aggregate
level is still positive (5 gains vs. 2 regressions, +3 net solved
targets) but the *regression* rate specifically, not the net delta, is
what the pre-registered threshold gates on, and it fails that specific
bar.

**This is now a substantive judgment call, not a clean pass/fail
mechanically resolvable by the decision tree as stated** -- the tree's
"regression >1%" branch says to re-diagnose before concluding anything,
which is done above; it does not by itself say whether a well-understood,
mechanistically-explained 2% regression (against a backdrop of the other
4 criteria passing, including the once-more-worrying p95) should still
block production adoption. Reported to the user for that specific call
rather than picked unilaterally.

### Final verdict: 2,000 templates REJECTED (user decision, 2026-08-13)

**Understanding the mechanism does not waive the threshold.** The
user's reasoning, verbatim in substance: accepting 2% here would make
the pre-registered `<=1%` bar meaningless in practice. The two
regressions are reproducible, mechanistically real performance
regressions (previously-solved routes pushed out of a fixed beam width
by increased candidate diversity), not noise or a timeout artifact --
and 2,000's net effect (5 gains, 2 losses, +3 net) is a 100-target-scale
trade of "drop 2 existing solutions to gain 3" that is too weak a
trade-off to accept as a *default* production setting.

**A targeted beam-widening rescue for just `L6`/`L66` was explicitly
declined as a gate-rescue path** (though it remains a valid mechanism-
confirmation experiment for later, separately) -- rescuing two named
targets doesn't answer whether widening the beam crowds out *other*
currently-solved targets, or what it does to timeout/memory/p95 at
scale, or whether the *aggregate* regression rate actually drops below
1%. Conflating "we understand why it broke" with "therefore it's safe
to ship" was identified and explicitly rejected as the wrong move.
**No beam-width tuning, no threshold changes, and no formal-TEST use for
the 2,000-template arm going forward.**

**Read as**: 2,000 templates helps coverage but was one step too
aggressive to be a safe *default* under the current template-matching/
beam-width architecture -- not "almost passed."

## Next: 1,000 templates (same fixed condition: 600s timeout, beam 100)

Phase A.5: 500->1,000 alone already captures a real chunk of the
coverage gain (34.0%->27.7% one-step zero-positive, -6.3pp) at a much
smaller pool-gen cost multiple (1.92x vs. 2,000's 4.44x) and,
mechanistically, should induce less beam-crowd-out than 2,000 did (fewer
new competing candidates per node). Same 5 thresholds, unchanged, same
VAL sample, same `shared_stock`/beam-100/timeout-600s/reranker-OFF
configuration as the (rejected) 2,000-template test.

**Manifest reuse check for the existing 500@600s arm, fail-loud as
instructed: FAILED -- re-running paired, not reusing.** The already-run
500@600s arm's manifest (`step7_widened_timeout/500_manifest.json`)
recorded `binary_sha256: dddd027e...`; the current `target/release/renkin`
hashes to `6aafd9ea...` -- different, because the `TemplateBondIndex`
revert/rebuild (code-disposition work, previous section) happened
*while that 500-arm run was still executing in the background* (it was
killed and resumed once during that window). Since `--bond-index` is
never passed in this gate, the actual behavior was unaffected either
way (confirmed earlier) -- but the manifest genuinely does not attest to
a single, unchanging binary across that run's 100 targets, so it fails
strict identity verification on its own terms. Per instruction: re-run
500 fresh, paired with the new 1,000-template arm, both under the
current, now-stable binary (no further rebuilds planned) -- output in
`data/phase_b1_frontier/step7c_1000_vs_500/`.

**Decision tree for the 1,000-template result (pre-registered, user's
own wording)**:
- 5/5 PASS -> freeze 1,000 as the production template count, proceed to
  Step 9 (reranker ON compatibility). Step 9 does not start before this.
- Coverage `<+3pp`, everything else PASS -> 1,000 is safe but the
  effect is too small to justify defaulting Phase B on its own;
  reconsider template-matching-speed work or Phase C instead of
  shipping a weak win.
- Regression `>=2%` again -> the limiting factor is candidate-ordering/
  fixed-beam-width non-monotonicity itself, not template count --
  pause standalone Phase B productionization regardless of which count
  is tried.
- Both 1,000 and 2,000 FAIL -> close Phase B.1 as a negative result:
  candidate-pool coverage increases with template count (Phase A.5,
  solid), but the current search architecture cannot convert that gain
  into route coverage safely at any of the tested template counts.

**Result (2026-08-13, both arms complete, 100/100 each, fresh paired
re-run under the confirmed-stable current binary):**

| | 500 @ 600s | 1,000 @ 600s | delta |
|---|---|---|---|
| `route_to_configured_stock` | 18/100 | 19/100 | **+1pp** |
| invalid/unparseable | 0 | 0 | 0 |
| timeout rate | 0/100 | 1/100 | +1pp |
| solved-target regressions | -- | 1/100 | 1% |

**Non-timing verdict, per the user's specified evaluation order (system
load was elevated during this run -- see below -- so timing metrics are
kept explicitly out of this call): coverage clearly FAILS.** `+1pp`
(18->19) is far short of the pre-registered `>=+3pp` bar -- not a
boundary case like 2,000's coverage number was. Invalid/unparseable
passes (0). Regression is 1/100 = 1%, at the boundary of the `<=1%`
allowed rate (technically passes) -- notably the same target, `L66`,
that regressed at 2,000 templates regresses again here, at a smaller
template increase, via the same `beam_limit_hit`/crowd-out signature.

**Per the pre-registered decision tree's first branch (coverage `<+3pp`
OR regression `>1%` OR invalid `>0` -> reject immediately, no clean
timing confirmation needed): 1,000 templates is REJECTED.** The
one-step candidate-pool coverage gain Phase A.5 measured at 1,000
templates (34.0%->27.7%, -6.3pp) does not convert into a meaningful
route-level coverage gain at beam-width 100 -- only 1pp of the
one-step gain's -6.3pp survives translation into `route_to_configured_stock`.
No clean load-controlled timing re-measurement was needed or run, since
the non-timing coverage criterion alone is decisive and timing noise
cannot change a 1pp-vs-3pp-required gap.

**Both 1,000 and 2,000 templates have now failed at the route-search
level** (1,000: coverage FAIL; 2,000: coverage/timeout/invalid PASS,
regression FAIL) -- exactly the pre-registered final branch of the
user's decision tree: **"candidate-pool coverage increases with
template count (Phase A.5, solid, reproduced independently multiple
times this session), but the current search architecture cannot safely
convert that gain into route coverage at any of the tested template
counts."** Phase B.1 closes as a negative result -- see the final
summary below.

**Process note on this run**: it ran across several kill/resume cycles
under elevated system load (other unrelated concurrent processes on the
same machine, load average 19-34 on a 10-core machine at various
points) and one full disk-space exhaustion event (`ENOSPC`, root cause:
`/System/Volumes/Data` at 100% capacity from accumulated large
`*_pool.jsonl` files across this session's Phase A.5 and Phase B.1 work
-- resolved by deleting the already-analyzed, gitignored, regenerable
pool files once free disk space allowed running any command at all).
`compare_run.py --resume`'s per-target flush+fsync held up through every
interruption, including the disk-full event -- no completed-target data
was ever lost, only wall-clock cost from re-invocation. This is exactly
why the user specified evaluating coverage/regression/invalid first,
independent of timing: those metrics are unaffected by any of this
noise, while timeout rate and latency percentiles would not have been
trustworthy without a load-controlled confirmation this run's
coverage failure made unnecessary.

## Code disposition (user direction, 2026-08-13): what ships, what doesn't

Both retrieval-index redesigns (OR baseline characterization, AND
redesign) failed Phase B.1's speed gate. Per explicit instruction: a
negative result stays recorded in git (this findings.md, the ROADMAP.md
summary); code that was specifically built to solve the problem and
didn't does not get merged as if it were a solution, since it becomes
unowned maintenance burden. Decision per change:

- **`TemplateBondIndex`'s AND/subset retrieval redesign
  (`src/chem_env.rs`): REVERTED to the original OR/union semantics.**
  Reasoning: AND-semantics is a strict subset of OR's retrieved-rule set
  (mathematically can never retrieve *more* than OR), so it plausibly
  wouldn't hurt anything -- but that was never independently validated
  at `--bond-index`'s actual shipped usage point (the 500-template
  default, route search), only at the 5,000-template pool-gen scale this
  program cared about. Shipping an unvalidated behavior change to an
  already-production flag on the strength of "should be fine" reasoning
  alone is exactly the kind of thing this instruction is about avoiding.
- **`bond_pairs_from_smirks`'s `.`-component-boundary fix (`src/chem_env.rs`):
  KEPT.** Independent value regardless of the AND/OR outcome -- a real
  latent parsing bug (disconnected LHS fragments spuriously treated as
  bonded) that happened to be harmless under OR-semantics (an extra
  spurious pair only ever makes OR retrieve *more*, never causes a false
  exclusion) but would have caused real false negatives if any future
  AND/subset-style retrieval redesign is attempted again without
  re-discovering this. Zero behavioral change to the reverted OR-mode
  index today; pure correctness-hardening for the future.
- **`propose_phase_nanos` cost-attribution instrumentation
  (`src/candidate.rs`, `perf-instrumentation`-gated): KEPT.** Zero
  overhead in production builds (same pattern as the pre-existing
  `apply_retro_call_count`), directly produced Step 1's finding
  (`raw_propose` = 98.7-99.9% of cost at every template scale) and
  remains available for any future cost-attribution work on this code
  path.
- **`--bond-index`/`--bond-index-top-k` CLI flags on `renkin-pool-gen`
  (`src/bin/pool_gen.rs`), plus its per-target
  `proposal_seconds_per_target_p50/p95/max` timing: KEPT.** Low-
  maintenance (a straightforward flag matching the existing `main.rs`
  pattern), gives `renkin-pool-gen` parity with `renkin`/`renkin-bench`'s
  existing `--bond-index` support, and preserves the ability to
  reproduce or extend this program's pool-gen-level measurements without
  re-plumbing -- diagnostic-tooling value independent of this specific
  program's outcome.

**Rebuild note**: the `TemplateBondIndex` revert and `renkin`/
`renkin-pool-gen` rebuilds happened while Phase B.1b's route-search gate
was already running in the background. This has no effect on that gate's
validity -- it never passes `--bond-index` (tests raw/unindexed candidate
generation only), so the retrieval-semantics revert is behaviorally
invisible to it either way. The `renkin` binary was rebuilt a second time
immediately after, without the `perf-instrumentation` feature, to match
the exact (non-instrumented) binary configuration the gate was launched
under, out of caution -- `perf-instrumentation`'s only effect on the
plain `renkin` binary is a per-call atomic-counter increment inside
`apply_retro`, expected to be negligible against SMARTS-matching cost,
but not worth leaving as an uncontrolled variable in an active timing
measurement.

## Phase B.1: final conclusion (2026-08-13)

**Negative result. Closed.** Full chain of evidence, in order:

1. **Phase A.5 (prior program)**: candidate-pool coverage improves
   substantially and reproducibly with more templates (500->9,979:
   34.0%->17.6% zero-positive, -16.4pp, "strong GO" against the
   pre-registered >=10pp bar). This finding is solid and unaffected by
   anything below -- it measures candidate generation, not route search.
2. **Retrieval indexing (this program, two attempts)**: neither
   OR/union nor AND/subset element-pair bond indexing gets remotely
   close to the 3-4x+ per-node speedup needed to make a larger template
   count route-search-affordable (achieved 1.14x, 1.18x). Root cause is
   architectural (element-pair granularity is too coarse for this
   template library), not a tuning gap. `TemplateBondIndex` reverted to
   its original, already-shipped OR-semantics; only a genuinely
   independent-value bug fix (`.`-component-boundary) and diagnostic
   instrumentation were kept.
3. **2,000 templates, route-search level**: FAILS at the original 150s
   timeout (timeout rate 2%->23%, regression 2%). Diagnosed the timeout
   blowup as a clean ~4x roughly-linear per-target cost increase
   colliding with an unadjusted timeout wall -- not pathological search
   behavior (`nodes_expanded` stays ~flat). Re-tested at a deliberately
   separate, non-superseding "Phase B.1b" widened-timeout (600s)
   condition: 4 of 5 criteria now pass, including p95 latency (2.36x,
   under the unrelaxed 2.5x bar) -- but **regression stays at 2%**,
   root-caused to fixed-beam-width-100 crowd-out (more templates ->
   more candidates competing for the same beam slots -> a previously-
   winning search path loses that competition). Understanding the
   mechanism did not waive the pre-registered threshold -- **rejected**.
4. **1,000 templates, route-search level**: tested next as the lower-
   cost escalation candidate. **Fails on coverage alone** -- only 1pp of route-level
   `route_to_configured_stock` improvement survives from Phase A.5's
   -6.3pp one-step candidate-pool gain, far short of the required +3pp.

**The throughline**: candidate-pool coverage gains from more templates
are real and well-established (step 1), but **do not reliably convert
into route-search coverage gains** under the current architecture --
either the conversion is too weak (1,000: coverage barely moves) or the
conversion's side effects (beam crowd-out at 2,000) violate a safety
bar that a well-understood mechanism doesn't get to waive. Retrieval
indexing, the one lever that could have changed the underlying cost
structure enough to make a larger template count workable, is roughly
an order of magnitude short of what would be needed, on two independent
attempts.

**What ships**: nothing from the indexing exploration ships as "the
fix" (see code disposition above) -- the currently-shipped 500-template
default and `--bond-index`'s original OR-semantics are both unchanged
from before this program started. What's retained is the negative
result itself (this document, `ROADMAP.md`'s summary), the cost-
attribution instrumentation (`perf-instrumentation`-gated timers,
`renkin-pool-gen`'s `--bond-index` CLI parity), and a real independent
bug fix (`bond_pairs_from_smirks`'s `.`-boundary reset).

**Not pursued as part of Phase B.1** (recorded as candidates for a
future, separately-scoped effort, not started here): reaction-center-
only indexing or a richer structural fingerprint (the two untried,
larger-effort retrieval-index redesigns); beam-width widening as a
crowd-out mitigation (explicitly declined as a gate-rescue path, valid
only as a separate mechanism-confirmation experiment); Phase C
(whatever alternative lever was being held in reserve behind Phase B).

Step 9 (reranker ON compatibility gate) was never started -- correctly,
since no production template count was ever frozen to test it against.

Formal TEST (4,903 targets) was never used anywhere in this program --
reserved, untouched, as designed from the start.

## Phase B.2: Progressive Template Escalation (2026-08-14)

GO given 2026-08-14 after Phase B.1 closed as a negative result. Full
pre-registration (arms, sample design, primary gate, B-vs-C tie-break,
implementation boundary) is in `ROADMAP.md`'s Phase B.2 section --
this section reports the result against that pre-registration,
unchanged.

**Sample**: new disjoint 200-target VAL sample, deterministic hash
order (sorted by `sample_key` = SHA-256 of canonical SMILES), excluding
the 100 targets already used throughout Phase B.1 and formal TEST.
Manifest + SHA-256 fixed before any run
(`data/phase_b1_frontier/phase_b2_sample_manifest.json`). A separate
10-target smoke sample (also disjoint) validated the Stage 1 ->
unsolved-manifest -> Stage 2 -> merge pipeline mechanics first --
confirmed clean (Stage 2's target set exactly matched Stage 1's
unsolved set for both arms, zero overlap with Stage 1's solved set) --
before committing to the real 200-target run.

**Implementation**: benchmark/orchestration layer only, per the
implementation boundary -- no new RENKIN core API. Stage 1 run once via
the existing `scripts/compare_run.py` (500 templates, 150s/10s
timeout, beam 100, `shared_stock`, reranker OFF) over the 200-target
sample. Its unsolved subset was written out in the same sample-list
JSONL schema `compare_run.py` already consumes, then fed to two
independent Stage 2 runs (Arm B: 1,000 templates; Arm C: 2,000
templates; both 600s/20s timeout) over that identical unsolved set.
Merge is trivial by construction: Stage-1-solved and Stage-2-attempted
target sets are disjoint, so there is no overwrite decision to make.

**Result**:

| | Arm A (500 only) | Arm B (500→1,000 coverage mode) | Arm C (500→2,000 coverage mode) |
|---|---|---|---|
| Solved / 200 | 33 (16.5%) | 38 (19.0%) | 50 (25.0%) |
| Coverage delta vs. A | -- | **+2.5pp** | **+8.5pp** |
| Solved-target regressions | -- | **0** | **0** |
| Invalid/unparseable | 0 | 0 | 0 |
| Stage-2 invocation rate | -- | 83.5% (167/200) | 83.5% (167/200) |
| New solves from Stage 2 | -- | 5 | 17 |
| End-to-end p50 | 14.3s | 42.8s | 83.6s |
| End-to-end p95 | 64.3s | 182.5s | 367.3s |
| p95 ratio vs. Arm A | -- | 2.84x | 5.72x |
| Total cumulative compute | 4,078.5s (68.0min) | 11,981.0s (199.7min) | 24,295.5s (404.9min) |
| Additional compute / additional solve | -- | 1,580.5s (26.3min) | 1,189.2s (19.8min) |

(Total/cumulative compute figures are the sum of every row's own
`total_elapsed_ms` across every stage that arm actually used -- not
`compare_run.py`'s own `wall_clock_total_sweep_s` field, which only
covers the single invocation that happened to finish each run and is
fragmented into meaninglessness by this program's many kill/resume
cycles under heavy, variable system load, load average observed
ranging 14-57 on a 10-core machine across this run's real-time span.)

**The regression-elimination hypothesis is confirmed exactly as
designed**: zero solved-target regressions in both arms, verified both
structurally (Stage 2's target set is byte-identical to Stage 1's
unsolved set in both arms -- confirmed by set equality, not merely
"observed to be low") and empirically (the merged solved-set for every
arm is a strict superset of Arm A's). This is the core mechanism Phase
B.1's crowd-out regression (2%, `L6`/`L66`) motivated, and it worked:
previously-solved targets structurally cannot compete with new
candidates for beam slots if they never re-enter a larger search at
all.

**Primary gate verdict**:
- **Arm B: FAILS.** Coverage `+2.5pp` is short of the pre-registered
  `>=+3pp` bar (6 new solves needed at n=200; only 5 achieved).
  Regression and invalid both clear their bars, but the gate requires
  all three, and coverage alone disqualifies this arm regardless of
  cost. Not layered into a cost-tier classification -- an arm that
  fails the primary gate doesn't get a cost verdict per the
  pre-registration.
- **Arm C: PASSES the primary gate.** Coverage `+8.5pp`, comfortably
  clear of `+3pp`. Regression `= 0` exact. Invalid `= 0`.

**Since only one arm (C) passes the primary gate, the B-vs-C tie-break
rule does not apply** (that rule is only defined for when both arms
pass) -- Arm C is the sole candidate remaining to classify by cost.

**Cost classification for Arm C**: `p95 = 5.72x` of Arm A -- **far
past the `<=2.5x` default-candidate threshold.** Arm C lands in the
**opt-in "coverage mode" candidate tier, not the default tier.** This
is not a marginal miss: p50 latency itself grew ~5.9x, driven by the
Stage-2 invocation rate being high (83.5% -- most targets in this
200-target VAL sample are *not* solved by the 500-template baseline,
so most targets do pay the full Stage-2 cost, not a rare fallback
cost). "Progressive escalation" as tested here is closer to "usually
escalate" than "rarely escalate" for this sample's difficulty
distribution -- worth keeping in mind when reasoning about what
`renkin coverage mode`'s typical latency would feel like in practice,
versus a mental model of escalation as a rare, cheap safety net.

**Interpretation**: the hypothesis motivating Phase B.2 is validated on
its own terms -- staged escalation genuinely converts candidate-pool
diversity into route coverage with zero regression, solving the exact
problem Phase B.1 identified. But it does so at a real cost that Phase
B.1's own numbers already foreshadowed (2,000-template per-node cost
~4-5x): an opt-in mode paying ~5.7x p95 latency for +8.5pp coverage,
not a free or cheap upgrade to the default. Arm B (1,000-template
coverage mode) is not a viable substitute at a lower cost tier either --
it fails the coverage bar outright before cost is even considered.

**Status (2026-08-14): PAUSED at the user's request to prioritize other
work.** Arm C is provisionally the coverage-mode candidate but not
formally frozen until the determinism gate below passes -- no further
CPU-heavy work (any `renkin`/`compare_run.py` run) resumes without
explicit OK.

**Determinism gate -- design finalized, one run started then cleanly
stopped on pause (0/32 targets, no data loss, fully resumable).** A
targeted 37-target spot-check, not a full 167-target repeat: all 17
Arm-C newly-solved targets, 10 still-Stage-2-unsolved (hash order), 5
Stage-1-solved-never-escalated (hash order), and 5 from Arm C's Stage-2
latency tail (in practice: all 4 real timeouts plus the next-longest
completed target). Each target's relevant stage(s) run twice; comparison
is a deterministic semantic projection (`target_id`, Stage-1 outcome,
Stage-2-invoked flag, selected stage, `route_found`,
`normalized_route_sha256` for the canonical route/tree,
configured-stock-leaves status, validator fields, invalid/timeout
classification -- explicitly excluding wall-clock, timestamps, and temp
paths), SHA-256'd and required to match exactly across both runs for
every target. Implemented as real, unit-tested code (not ad hoc
analysis) in `scripts/phase_b2_orchestrator.py` +
`scripts/tests/test_phase_b2_orchestrator.py` (17 new tests, all
passing alongside the existing 313) -- see `ROADMAP.md`'s Phase B.2
section for the full design and the six specific invariants tested.
Committed alongside the decision-run data (`1adab83`) and this
determinism-run data.

**Determinism gate: RESULT (2026-08-15) -- PASS, 37/37 targets, both
runs, exact match.**

| subset | targets | run1 projection SHA-256 | run2 | match |
|---|---|---|---|---|
| Stage-2 (Arm C escalated) | 32 | `ab3a4537...fda95` | identical | **YES** |
| Stage-1 (never escalated) | 5 | `ad859eec...ee4988` | identical | **YES** |

Every one of the 37 targets' semantic projection (route
found/shape/validator outcome, explicitly excluding wall-clock,
timestamps, and temp paths) matched byte-for-byte across two
independent runs, computed via `scripts/phase_b2_orchestrator.py`'s
`merge_arm`/`projection_sha256` (the same code that enforces the
Stage-1-never-overwritten and Stage-2-input-set-exact invariants, not
a separate one-off comparison script). This closes the primary gate's
fourth and final criterion.

**Phase B.2 primary gate, final status: Arm C PASSES all four
criteria** -- `route_to_configured_stock +8.5pp`, `regressions = 0`
exact, `invalid/unparseable = 0`, `deterministic outcome` exact.
**Arm C (500->2,000 coverage mode) is now formally frozen as the
opt-in coverage-mode production candidate**, cost tier "opt-in
coverage mode" (not default, `p95 5.72x`). Arm B (500->1,000) stays
rejected on coverage alone (+2.5pp, never reached a determinism check
since it failed earlier on a criterion that doesn't get waived).

## Reranker compatibility gate (2026-08-15)

**CORRECTION (2026-08-15, same day): this gate was initially recorded
below as "all 5 criteria PASS." That was premature.** The 5-target
spot-check described under "Determinism" is supplementary evidence,
not a substitute for the pre-registered 37-target protocol (same
discipline as never waiving the 2,000-template regression gate just
because the crowd-out mechanism became understood -- understanding
does not waive a pre-registered threshold, and neither does a lighter
spot-check substitute for the pre-registered replay it was meant to
approximate). The numeric result below (coverage/regressions/invalid/
`reranker_failures`) stands as a strong PASS on those four criteria.
The determinism criterion is downgraded to **PENDING** until an
extended replay runs: the *same* fixed 37-target set used for the
decision-run determinism gate (17 newly-solved + 10 Stage-2-unsolved +
5 Stage-1-solved + 5 latency-tail, no new sample carved out), re-run
under the reranker-ON coverage-mode configuration, twice, compared by
semantic projection (extended to include `reranker_failures`). See
below for the result once that replay completes.

GO given after Phase B.2's determinism gate passed. New, third disjoint
100-target VAL sample (`val_sample_reranker_100.jsonl`, excludes both
the original 100 from Phase B.1 and the 200-target Phase B.2 decision
sample -- 4,624 remaining candidates, hash-sorted, first 100 taken).
Arm A (500-only + reranker ON) vs. Arm C (500->2,000 coverage mode +
reranker ON), both using the frozen `data/phase3e_reranker_training/`
model/frequency-table artifacts.

**Bug found and fixed en route**: `scripts/compare_renkin_adapter.py`
never captured the `reranker_failures` field into `tool_specific`
despite it being present in `renkin`'s own JSON output -- meant the
gate's `reranker_failures=0` criterion couldn't actually be checked
against already-collected data. Fixed (own commit, +1 unit test,
`test_reranker_failures_is_captured_in_tool_specific`), then Arm A was
re-run fresh with the fix in place (the pre-fix data is archived under
`reranker_gate/_old_pre_fix/`, not used for the gate).

**Arm C's Stage 1 reuses Arm A's data directly** rather than
re-running the identical 500-template+reranker-ON config a second
time -- justified by this program's own just-completed determinism
gate proving run-to-run determinism for this exact architecture.

**Result**:

| | Arm A (500+reranker ON) | Arm C (500->2,000 coverage mode+reranker ON) |
|---|---|---|
| Solved / 100 | 27 | 34 |
| Coverage delta vs. A | -- | **+7pp** |
| Solved-target regressions | -- | **0** |
| Invalid/unparseable | 0 | 0 |
| `reranker_failures` | 0 (99/100 measured, 1 timeout) | 0 (72/73 measured, 1 timeout) |
| Timeouts | 1/100 (Stage 1) | 1/73 (Stage 2) |

**Determinism (superseded, see correction above)**: a lighter-weight
spot-check than Phase B.2's original 37-target protocol (5 targets: 3
Arm-C-newly-solved + 2 Stage-1-solved, each run twice) -- justified at
the time by strong prior evidence rather than re-running the full
protocol: the staging/merge architecture's determinism was just fully
verified (37/37 targets), and the reranker itself already has
dedicated bit-exact determinism coverage in the Rust test suite
(`reranker_some_is_also_fully_deterministic_across_repeated_runs`, and
the LightGBM reader's validation against `lightgbm.Booster.predict()`).
Both subsets matched exactly across both runs -- good supplementary
evidence, but not the pre-registered gate. See "Extended determinism
replay" below for the actual gate result.

**4 of 5 gate criteria PASS, determinism PENDING**: `coverage +7pp`
(>=+3pp), `regressions=0` exact, `invalid=0`, `reranker_failures=0`.
No p95 requirement applied (Arm C already opt-in-tier on cost) and no
extreme blowup observed (timeout rate stayed modest at both stages).

**Reranker compatibility: PROVISIONALLY CONFIRMED, pending the
extended determinism replay.** The frozen reranker (trained on
500-template candidate distributions) works correctly when layered on
top of the 2,000-template coverage-mode escalation on every numeric
criterion measured -- no candidate-distribution-shift degradation, no
reranker failures, coverage gain preserved. Not treated as a closed
gate until the determinism replay below passes. Per the pre-registered
discipline: none of this means the reranker couldn't be *improved* by
retraining on the larger-template candidate distribution -- that stays
a separate, not-started idea (Phase B.3, if ever pursued) rather than
something inferred from this compatibility check.

**Sequencing**: determinism PASS -> reranker compatibility (4/5 PASS,
determinism replay pending) -> coverage-mode CLI/Python design (may
proceed in parallel, design-only, no implementation) -> product
integration (blocked on the replay passing) -> exactly one
formal-TEST confirmation run under a frozen spec -> v0.24.0.
