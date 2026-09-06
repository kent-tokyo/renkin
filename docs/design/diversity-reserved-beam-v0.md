# Diversity-Reserved Beam — Design Doc (ROADMAP Item 4)

Status: **Implemented, remeasured, and opt-in; default remains `Off`.** The
corrected 20-slot implementation recovered five independently validated routes
on the fixed 58-target AiZynthFinder-only cohort, but always-on `Active` lost 10
of the old 591-success formal set. A causal `Off` replay recovered nine of
those ten under the same current code, so the confirmed Active-specific loss
alone remains above the <=1% gate. It therefore must not become the default.
The native CLI now offers the bounded
`retry-on-beam-exhaustion` orchestration: score-only search runs first and
diversity can only replace a completed unsuccessful run that actually reached
a beam cutoff. This document scopes the
"diversity-reserved beam" mechanism from `internal_docs/ROADMAP.md`'s
beam-crowd-out item
(P1, issue #101) — a *different* mechanism from PR #104
(`feat/open-state-dominance-101`), which attacked a related but distinct
problem (duplicate open-state re-pushes) and ended in a final,
**permanent negative result** (Round 2G: `route_to_configured_stock`
+3pp passed, but a 2% solved-target regression rate exceeded the <=1%
safety bar — do not resume or rescue that PR). This doc follows
`docs/design/spectator-bond-fail-closed-gating-v0.md`/
`docs/design/candidate-time-element-accounting-gate-v0.md`'s structure
and rigor.

## 0. What this is, in one paragraph

`beam_prune` (`src/search.rs:1125-1157`) selects survivors by pure
top-K-by-score (`Node::f() = g+h`) every time it's called — a stateless,
whole-heap drain/sort/truncate/rebuild, invoked globally once per search
node's expansion (`search.rs:2043-2046`). This means many high-scoring
candidates from the *same* rule/reaction type can fill the entire beam,
crowding out a lower-scoring-but-structurally-different candidate that
might lead to the only actual solution — the literal mechanism the
roadmap's own beam-crowd-out item describes, and the one PR #104's
duplicate-state pruning did **not** address (that mechanism targeted
identical-state re-pushes, not score-driven family homogeneity). This
doc proposes reserving a small fraction of beam slots for
family-diversity rather than pure score, so a fixed number of
underrepresented-family candidates always survive pruning regardless of
how many higher-scoring same-family siblings exist.

## 1. Existing-code grounding

> Implementation note (2026-09-05): this section records the historical
> baseline that motivated the design. The current implementation now carries
> `Node::family_key`, selects survivors in `select_beam_survivors`, and exposes
> the policy through Rust, CLI, Python, and WASM.

- `beam_prune`'s exact signature: `fn beam_prune(heap: &mut
  BinaryHeap<Node>, beam_width: usize) -> (Option<BeamEvictionStats>,
  Vec<TraceRank>)` (`search.rs:1125`). **No persistent state survives
  between calls** — every invocation drains the heap into a `Vec`, sorts
  by `f()`, truncates to `beam_width`, rebuilds the heap from scratch.
  This is the single most important fact for this design: PR #104's
  entire correctness saga (the ghost-record bug, a `HashMap` entry
  surviving past its holder's eviction and incorrectly dominating a later
  regeneration) was a consequence of *persistent cross-call state*.
  **A diversity reservation computed fresh from the just-drained `Vec`
  every call has no analogous staleness class to guard against** — there
  is nothing to go stale, since nothing persists.
- **Global, not per-parent-local scope, confirmed directly**: `beam_prune`
  is called once per node's expansion, on the *entire* open heap
  (`search.rs:2043-2046`), not scoped to just that node's own children.
  "Reserve N slots for family diversity" therefore means N slots of the
  *global* survivor set, not N slots per branching point — a materially
  simpler scope than a per-node-local reservation would be, and the only
  scope actually consistent with how `beam_prune` is invoked today.
- **No "template family"/"reaction center"/"changed-bond signature"
  concept exists anywhere in the codebase.** `dedup_counts`
  (`search.rs:1169-1195`) groups by `(template_id, sorted precursor
  SMILES)` for diagnostics only — the closest existing precedent for a
  grouping key, not a shipped mechanism. Nothing computes which bond
  changed between a candidate and its target. Any family notion this
  design uses must be defined here, not borrowed.
- **`Node` doesn't carry rule/template identity directly** (`search.rs:
  674-685`: `frontier`, `path: Option<Arc<PathNode>>`, `depth`, `g`, `h`,
  `trace_id`) — reaching it today means walking the `path` linked list,
  O(depth), not O(1). **But `entry.template_id`/`entry.rule_name`/
  `entry.precursor_signature` are already computed once, at push time**
  (`search.rs:2020-2023`, feeding `CandidateTraceRecord`) — they simply
  aren't retained on `Node` itself. `trace_id` is the exact existing
  precedent for exactly this shape of extension: an optional lightweight
  field, populated once from already-computed data at push time, at zero
  extra per-prune-call cost.
- **PR #104's measurement methodology is the established acceptance bar
  for any beam-mechanism change in this area** — re-derive nothing;
  reuse its exact 5 criteria and, ideally, its exact target sample, so a
  new result is directly comparable to the historical one rather than an
  apples-to-oranges new baseline.

## 2. Scope boundary: `template_id` as the family key (v1)

The biggest open design question from the research pass: is
`rule_name`/`template_id` alone (zero new computation, already available
at push time) a meaningful enough "family" to justify reserved slots, or
does the real crowd-out need a finer key (e.g. `template_id` +
precursor-structural-cluster) that doesn't exist yet? **v1 decision:
start with `template_id` alone.** It's the only zero-marginal-cost key
available, it directly matches the roadmap's own "template family" axis
(one of four listed: precursor signature, template family, reaction
center, changed-bond signature — the other three are all separate,
larger pieces of net-new work, deliberately deferred, same discipline as
scoping element-accounting-only for Item 1). If a measurement shows
`template_id` alone doesn't meaningfully change which routes get found,
that's real evidence for whether a finer key is worth building — not
something to assume up front.

## 3. Handling candidates without a clear family

A `Node` at depth 0 (the root) or a node reached through no rule
application at all has no `template_id` to key on. **These stay eligible
for the pure-score portion of the beam only, never for a diversity-slot
reservation** — they can't be under- or over-represented in a "family"
they don't belong to, so granting them diversity-slot preference would be
arbitrary. Mirrors the existing `not_evaluable` principle from the other
two design docs (never silently include something in a mechanism it
doesn't have the data to justify), adapted to a selection context instead
of an accept/reject one.

## 4. Typed contract

```rust
// src/search.rs — Node gains one new optional field, populated once at
// push time exactly like trace_id already is (search.rs:2020-2023 already
// computes this value; the only change is retaining it on the Node).
pub struct Node {
    // ...existing fields unchanged...
    pub family_key: Option<TemplateId>, // None for root / no-rule nodes (§3)
}
```

```rust
// New, alongside BeamEvictionStats
pub struct DiversityReservationStats {
    pub families_represented_by_score_alone: usize,
    pub families_rescued_by_reservation: usize,
    pub rescued_node_trace_ids: Vec<u64>, // for search_diagnostics, mirrors
                                           // spectator_bond_gated_out's own
                                           // "never a silent exclusion" rule,
                                           // applied to inclusion instead
}
```

## 5. Policy mechanism

Same 3-state shape as `SpectatorBondPolicy`/`ElementAccountingGatePolicy`,
its own independent `SearchConfig` field:

```rust
pub enum BeamDiversityPolicy {
    Off,             // default: today's exact pure-top-K behavior, zero cost
    DiagnosticsOnly, // computes what the diversity-reserved selection WOULD
                     // keep differently, records it, still returns the
                     // pure-top-K survivor set unchanged
    Active,          // actually applies the diversity reservation
}
```

(Named `Active` rather than `Gated` deliberately — this mechanism changes
*which candidates survive selection*, not whether an individual candidate
is accepted/rejected against a correctness check; "Gated" reads as
implying the latter, matching this codebase's own established naming
precision.)

## 6. Where it happens

Inside `beam_prune` itself, immediately after the existing sort-by-`f()`
step, before truncation:

1. Compute pure top-`(beam_width - diversity_slots)` by score, as today
   (the "score-selected" set).
2. Walk the remaining sorted candidates once; for each `family_key` not
   already represented in the score-selected set, take its best-scoring
   representative into a "diversity-selected" set, until
   `diversity_slots` is filled or candidates run out.
3. If fewer distinct families exist than reserved slots, fill unused slots
   with the next best score-ranked candidates. `Active` therefore never
   shrinks the effective beam merely because diversity is unavailable.
4. Apply the policy. `Off` skips steps 1-3 and returns pure
   top-`beam_width` — byte-for-byte the original behavior.
   `DiagnosticsOnly` computes steps 1-3 and populates
   `DiversityReservationStats`, but still returns the unmodified pure
   top-`beam_width` set. `Active` returns the score-selected,
   diversity-selected, and backfill union.

This keeps the mechanism entirely inside `beam_prune`'s own existing
call site — no change needed anywhere else in the search loop, since
`beam_prune` is already the sole place global survivor selection happens.

## 7. Acceptance criteria

**Reuse PR #104's own 5-criterion formal gate verbatim, not a new bar**:
`route_to_configured_stock` >= +3pp absolute vs. `Off`, invalid/
unparseable = 0, solved-target regression <= 1%, timeout-rate increase
<= +1pp, p95 route-search latency <= 2.5x. Run on the same 100-target VAL
sample PR #104's Round 2G used, if it's still available/reconstructable,
specifically so this result is comparable to that historical one rather
than a new, incomparable baseline.

**New criterion this mechanism needs that PR #104 didn't** (PR #104
pruned *duplicates* — nothing to lose by removing a true duplicate;
this reserves slots *away from* legitimately-different high-scoring
candidates, a real tradeoff): **diversity yield** —
`DiversityReservationStats::families_rescued_by_reservation` /
`diversity_slots` per target, to confirm the reserved slots are actually
buying something (a family that would otherwise have been fully excluded)
rather than sitting mostly unused because score-based selection already
covers most families most of the time.

**Sweep, not one fixed split**: measure at least 2-3 `diversity_slots`
fractions (e.g. 10%, 20% of `beam_width`) before picking a default, per
this codebase's own established practice of measuring a small parameter
sweep rather than shipping a single guessed value (mirrors Phase B.1's
own frontier-sweep discipline for template count).

## 8. Rust/CLI/Python/WASM parity

The base `off|diagnostics-only|active` policies and
`beam_diversity_slots` have Rust/CLI/Python/WASM parity. The native CLI and
formal-comparison adapter additionally expose
`retry-on-beam-exhaustion`; its reusable two-pass function is public Rust API.
The orchestration is deliberately not folded into `BeamDiversityPolicy`, which
continues to describe one search pass only.

## 9. Rollout stages

1. **Done.** Add `family_key: Option<TemplateId>` to `Node`, populated at push time
   from the already-computed value (§1) — pure plumbing, no behavior
   change, `Off`-equivalent by construction since nothing reads the field
   yet. Regression-test: identical search output before/after.
2. **Done.** Implement the selection (§6) as a new, separately unit-tested
   function taking a `Vec<Node>` + policy + slot count, returning the
   selected set + stats — unit-tested in isolation (synthetic `Node` sets
   with contrived family distributions) before wiring into `beam_prune`
   at all, same discipline as both prior docs' rollout stage 2.
3. **Done.** Wire into `beam_prune`/`SearchConfig`/`search_diagnostics` (§4-6).
   `Active` still off by default (`Off`).
4. **Done.** CLI/Python/WASM parity (§8).
5. **Corrected implementation measured; always-on default rejected.** On the
   fixed 58-target AiZynthFinder-only cohort, same-session `Off` found zero
   valid routes while `Active`/20 found five (`L3738`, `L87`, `L2531`,
   `L4863`, `L348`), with zero invalid routes and zero timeouts. Total wall
   time was 45.89s vs. 49.35s and median per-target time 570ms vs. 590ms.
   Against the old formal set of 591 RENKIN successes, however, always-on
   `Active`/20 retained only 581 and changed 103 selected route hashes. A
   same-code `Off` replay of those ten losses recovered nine; the remaining
   `L4699` loss came from the concurrently added #237 fragment-integrity
   correction changing frontier evolution and is recoverable at beam 200.
   The nine confirmed Active-specific losses alone exceed the <=1% gate, so
   `Active` remains opt-in.
6. **Done (Issue #238).** Add `retry-on-beam-exhaustion`: pass 1 forces `Off`;
   pass 2 uses `Active` only after no route, normal completion, and a recorded
   beam cutoff. Both passes reuse one prepared template set. Existing successes
   return directly from pass 1. On the same 58-target cohort it invoked 58
   retries, recovered the same five valid routes, produced no invalid route or
   timeout, and took 94.05s total. This is diagnostic recovery evidence, not a
   new full-corpus formal score.
7. **Pending stronger evidence.** Before reconsidering any one-pass default,
   revisit
   whether `template_id` alone was sufficient, or whether the sweep's own
   diversity-yield numbers make the case for a finer key
   (precursor-structural-cluster, reaction-center, changed-bond-signature) as
   a distinct, later phase.

## Resolved decisions and remaining gate

- `template_id` is implemented as the v1 family key (§2); finer structural
  keys remain deferred until measured diversity yield justifies them.
- PR #104's fixed VAL cohort and formal criteria were reused for direct
  comparability (§7).
- Corrected `Active`/20 failed the solved-target regression gate and must not be
  enabled by default. `retry-on-beam-exhaustion` is the bounded recovery path;
  it preserves score-only successes by construction, but its second-pass cost
  still requires explicit opt-in.
