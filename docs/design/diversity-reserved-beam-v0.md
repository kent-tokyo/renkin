# Diversity-Reserved Beam — Design Doc (ROADMAP Item 4)

Status: **Design only, not yet implemented.** Scopes the "diversity-reserved
beam" mechanism from `internal_docs/ROADMAP.md`'s beam-crowd-out item
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
3. `Off`: return step 1's set at full `beam_width` (i.e. skip steps 2-3
   entirely — byte-for-byte today's behavior, not just equivalent).
   `DiagnosticsOnly`: compute steps 1-2, populate
   `DiversityReservationStats`, but still return step 1's unmodified
   full-`beam_width` set. `Active`: return the union of both sets.

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

Same shape as the other two docs' §8: CLI flag (`--beam-diversity-policy
off|diagnostics-only|active`, plus a `--beam-diversity-slots <N>` or
`--beam-diversity-fraction <0.0-1.0>` parameter), Python `SearchConfig`
field, WASM config struct field, same `snake_case` serde convention.

## 9. Rollout stages

1. Add `family_key: Option<TemplateId>` to `Node`, populated at push time
   from the already-computed value (§1) — pure plumbing, no behavior
   change, `Off`-equivalent by construction since nothing reads the field
   yet. Regression-test: identical search output before/after.
2. Implement the 3-step selection (§6) as a new, separately unit-tested
   function taking a `Vec<Node>` + policy + slot count, returning the
   selected set + stats — unit-tested in isolation (synthetic `Node` sets
   with contrived family distributions) before wiring into `beam_prune`
   at all, same discipline as both prior docs' rollout stage 2.
3. Wire into `beam_prune`/`SearchConfig`/`search_diagnostics` (§4-6).
   `Active` still off by default (`Off`).
4. CLI/Python/WASM parity (§8).
5. The parameter sweep + formal gate (§7), reusing PR #104's own sample
   and criteria — before any default change or release.
6. Only after a `Active`-policy PASS: revisit whether `template_id` alone
   was sufficient, or whether the sweep's own diversity-yield numbers
   make the case for a finer key (precursor-structural-cluster,
   reaction-center, changed-bond-signature) as a distinct, later phase.

## Open questions for sign-off before implementation starts

- OK starting with `template_id` alone as the family key (§2), deferring
  the roadmap's other three candidate axes (precursor signature,
  reaction center, changed-bond signature) to a later phase gated on
  this one's own measured diversity yield?
- OK reusing PR #104's exact formal-gate criteria and (if reconstructable)
  its exact 100-target sample for direct comparability (§7), rather than
  a fresh sample that might be more representative of current
  `default_rules()`/template-corpus composition but loses that
  comparability?
- Is a 2-3 point parameter sweep on `diversity_slots` (§7) sufficient, or
  does this warrant the same kind of staged frontier search Phase B.1
  used for template count (multiple sweep rounds, explicit STOP/GO gates
  per round) given how directly this touches the same crowd-out territory
  PR #104 already spent significant effort on without a clean win?
- Given PR #104's own history (one plausible mechanism, extensively
  measured, ultimately rejected on a real safety bar) — is there
  appetite to invest another full measurement cycle in this general
  problem area right now, or does the roadmap's own P1 priority order
  suggest a different item first? (Not this doc's call to make; flagging
  it explicitly since it's the most consequential open question of all.)
