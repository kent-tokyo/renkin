# Candidate-Time Element-Accounting Gate — Design Doc (v1 slice of ROADMAP Item 1)

Status: **Design only, not yet implemented.** Scopes the smallest real
first slice of `internal_docs/ROADMAP.md`'s "Universal chemical-structure
safety gate" (P0) item, per a 2026-08-28 research pass that mapped
existing validation machinery before proposing anything new. Full item's
ask is much bigger (general element-survival, valence/charge/aromaticity
validity, a `valid`/`valid_with_declared_omission`/`not_evaluable`/
`invalid_atom_loss`/`invalid_ring_topology`/`invalid_valence` taxonomy);
this doc scopes only the `invalid_atom_loss` axis, reusing already-tested
logic, following `SpectatorBondLoss`'s exact rollout shape
(`docs/design/spectator-bond-fail-closed-gating-v0.md`) rather than
inventing a new one.

## 0. What this is, in one paragraph

`synthesizability::compute_element_accounting` (`src/synthesizability/
element_accounting.rs:76`) already does a real, tested, directional
per-element heavy-atom check — but only once a **complete route** reaches
`search.rs`'s acceptance boundary (`route_integrity_defects()`,
`search.rs:1690-1713`), via `search.rs:365`. A single defective step deep
in an otherwise-fine route silently drops that whole route (search
continues elsewhere) rather than being caught at the moment it's
generated. This doc proposes lifting the exact same per-element check
one level earlier — to a single newly-generated candidate, at
`heap.push(Node {...})` time (`search.rs:1639`/`2032`) — as an opt-in,
default-off diagnostic first, matching how every other search-behavior-
changing gate in this codebase has shipped (`RingContextConfig`,
`SpectatorBondPolicy`).

## 1. Existing-code grounding

- `compute_element_accounting(route: &Route)` (`element_accounting.rs:76`)
  loops `route.steps`, and its **per-step body is already a
  self-contained, single-step check** — for one step: parse the step's
  own `target`, sum heavy-atom-per-element counts over that step's own
  `precursors`, and compare. The route-level wrapper only adds
  aggregation policy (`any_evaluated`/`failing_step_indices`/the
  three-way `ElementAccountingStatus`). **This means extracting the inner
  loop body into its own function is a pure refactor, not new logic** —
  the exact same code, called once per step instead of once per route.
- `heavy_atom_counts(smiles: &str) -> Option<HashMap<Element, usize>>`
  (`element_accounting.rs:34`) is already `pub(crate)`, already handles
  unparseable SMILES as `None` (never a panic), and is the single
  existing source of truth for this check's semantics — reuse directly,
  never re-derive.
- **This check is deliberately not mass conservation** (the module's own
  doc comment is explicit: never call it that). It only fails when the
  *target* needs more of an element than the *precursors* supply —
  precursors carrying extra atoms (leaving groups, reagents) is never a
  failure. This asymmetry matters for the gate design below: it can never
  reject a candidate for having "too many" atoms, only too few.
- `RawCandidate` (`src/candidate.rs:944`) already carries `precursors:
  Vec<PrecursorMol>` (each with its own SMILES) and the closure that
  builds it (`raw_propose`, `src/candidate.rs:1004`) already has the
  step's `target_smiles` in scope — everything the per-step check needs
  is already sitting there at generation time, for free, no re-derivation
  or extra chematic calls beyond what `heavy_atom_counts` itself does.
- **Confirmed no overlap with `SpectatorBondLoss`.** That mechanism
  detects a specific *topological* pattern (a real target bond spanning a
  ring-fusion/cross-product boundary that no RHS fragment declares) via
  bond-level graph analysis, independent of element counting. This gate
  detects a specific *arithmetic* pattern (an element genuinely
  disappearing from the accounted total) via atom counting, independent
  of ring topology. A candidate could fail one, the other, both, or
  neither — they are not redundant and not mutually exclusive.
- **Confirmed no overlap with the v0.36.0 rule-safety census**
  (`examples/rule_safety_census.rs`). That's a static, design-time SMIRKS
  pattern screen (flags rule *shapes* that historically caused defects,
  like `aryl_amine_retro`'s). This gate is a runtime, per-candidate
  numeric check, independent of which rule produced the candidate or what
  its SMIRKS looks like.

## 2. Scope boundary: element accounting only (v1)

This doc does **not** propose valence/charge/aromaticity checks (the
research pass confirmed no such check exists anywhere in this codebase,
at any stage — that's a materially larger, separate piece of work
requiring new chemistry-validity logic, not a lift of existing code) or
the full `valid`/`valid_with_declared_omission`/`not_evaluable`/
`invalid_atom_loss`/`invalid_ring_topology`/`invalid_valence`
classification taxonomy (no existing code produces this anywhere; it's a
new type regardless of which checks eventually feed it). v1 ships exactly
one binary-ish signal — element-accounted or not, or not-evaluable on
parse failure — as its own opt-in policy, not a taxonomy.

## 3. Confident vs. `not_evaluable`

Directly inherits the existing route-level semantics, applied per-step
instead of per-route: a candidate's target or **any** of its precursors
failing to parse under `heavy_atom_counts` makes that candidate
`NotEvaluable`, never silently accepted or rejected. This is the same
choice `compute_element_accounting` already makes (a step that can't
parse is skipped, not counted as failing) — no new judgment call
introduced, just the same one applied at a finer grain.

## 4. Typed contract

```rust
// src/synthesizability/element_accounting.rs — new, alongside the
// existing route-level function; both call the same extracted per-step
// helper so they can never silently disagree.
pub(crate) fn step_element_accounting(
    target: &str,
    precursors: &[String],
) -> Option<bool> // Some(true) = accounted, Some(false) = fails, None = not_evaluable (parse failure)
```

```rust
// src/candidate.rs — mirrors SpectatorBondGateVerdict's shape exactly
pub enum ElementAccountingGateVerdict {
    Accepted,
    Rejected, // no findings payload needed here (unlike SpectatorBondLoss,
              // there's no bond-level detail to attach — the element-count
              // mismatch itself, computed on demand, is the whole story)
    NotEvaluable,
}
```

Attaches as a new field on `RawCandidate`, `pub element_accounting_gate:
ElementAccountingGateVerdict`, always populated once policy is anything
but `Off` (matching `SpectatorBondGateVerdict`'s own "explicit three-way
enum, never implicit" rationale).

## 5. Policy mechanism

New enum, same shape as `SpectatorBondPolicy` (`src/search.rs`), added as
its own `SearchConfig` field — **not folded into `SpectatorBondPolicy`**,
since these are two independent, separately-toggleable mechanisms (a
caller may want one gated and not the other, same "never one label for
two orthogonal axes" rule this codebase already applies to
`ring_context_policy` vs. `spectator_bond_policy`):

```rust
pub enum ElementAccountingGatePolicy {
    Off,             // default: zero cost, matches today's absent behavior
    DiagnosticsOnly, // computes and records the verdict, never excludes
    Gated,           // additionally excludes Rejected candidates
}
```

## 6. Where rejection happens, and its own diagnostic trail

In `raw_propose`'s per-rule closure (`src/candidate.rs:1004`), right where
each `RawCandidate` is constructed (after `target_smiles`/`precursor
list` are both known, no route context needed): call
`step_element_accounting(target_smiles, &precursor_smiles)`, attach the
verdict, and — under `Gated` only — drop `Rejected` candidates before
they reach the closure's returned list, pushing a record into a new
`CrowdOutDiagnostics::element_accounting_gated_out: Vec<GatedCandidateRecord>`
(rule name, target, precursor SMILES, missing element(s) if cheap to
recompute for the record) — mirroring `spectator_bond_gated_out`'s own
shape exactly, so an exclusion is never invisible.

## 7. Acceptance criteria

**Positive controls (must reject under `Gated`)**: any of the existing
route-level `UnaccountedTargetElement` fixtures already covering
`compute_element_accounting`'s route-level tests, reproduced as a single
step rather than a full route — these already exist and already pass at
the route level, so this is a re-target of existing fixtures onto the new
function, not new fixture authoring from scratch.

**Negative controls (must never reject)**: any of this codebase's many
already-passing ordinary single-step disconnections (aspirin's
ester-cleavage step is a convenient, already-used-elsewhere example) —
confirm the lifted check doesn't reject anything the route-level check
wouldn't have flagged for that exact step.

**Consistency check (new, not present in the SpectatorBondLoss doc since
it had no earlier-stage counterpart to compare against)**: for a batch of
real routes, per-step `Gated` verdicts must agree with
`route_integrity_defects()`'s own `UnaccountedTargetElement` determination
for the same steps — if a route-level `Fail` exists for a step that the
candidate-time gate called `Accepted`, or vice versa, that's a bug in one
of the two call sites, not an acceptable discrepancy, since both now call
the identical extracted function.

## 8. Rust/CLI/Python/WASM parity

Same shape as `SpectatorBondPolicy`'s own §8 (untouched by any work so
far, needed before release): CLI flag mirroring `--spectator-bond-policy`
exactly (`--element-accounting-policy off|diagnostics-only|gated`),
Python `SearchConfig` field, WASM config struct field with the same
`#[serde(rename_all = "snake_case")]` convention.

## 9. Rollout stages

1. Extract `step_element_accounting` from `compute_element_accounting`'s
   existing loop body (§1) — pure refactor, route-level behavior must be
   byte-identical before/after (regression-test this explicitly: same
   inputs, same `ElementAccountingStatus`/`failing_step_indices` output).
2. New `ElementAccountingGateVerdict`/`ElementAccountingGatePolicy` types
   + the consistency-check test (§7) — unit-tested in isolation before
   touching `raw_propose` at all, same discipline as the SpectatorBondLoss
   rollout.
3. Wire into `raw_propose`/`RawCandidate`/`CrowdOutDiagnostics` (§6).
   `Gated` still off by default.
4. CLI/Python/WASM parity (§8).
5. A lightweight smoke measurement (same shape as
   `examples/spectator_bond_smoke.rs`/its 15-target run) under `Gated`,
   recording excluded-candidate counts and route-count deltas, before any
   default change or release — per this codebase's own standing rule that
   a new fail-closed gate never ships assumed-safe.
6. Only after 1-5: revisit whether `invalid_ring_topology`
   (already substantially covered by `SpectatorBondLoss`, see §1) and
   `invalid_valence` (genuinely new work, no existing logic to lift) are
   worth pursuing as separate, later phases of the same ROADMAP item, or
   whether v1's element-accounting slice alone already covers the
   highest-value share of real defects. Not decided here.

## Open questions for sign-off before implementation starts

- **Answered 2026-08-29**: yes, a second, independent
  `ElementAccountingGatePolicy` enum, not a widened `SpectatorBondPolicy`
  umbrella (§5). Keeping them orthogonal costs one more CLI flag and one
  more `SearchConfig` field, but avoids ever conflating two detection
  mechanisms (topological vs. arithmetic) under one label, matching this
  codebase's existing ring-context/spectator-bond precedent.
- **Partially answered 2026-08-29, still open in full**: `Off` stays the
  default through all of v0.37.0's rollout regardless — so "is a
  lightweight smoke measurement sufficient before ever defaulting to
  non-`Off`" doesn't block v0.37.0 itself, but remains genuinely
  undecided for whenever a future default-change proposal comes up.
  Don't treat v0.37.0's rollout as having answered this.
- **Answered 2026-08-29**: this slice (`invalid_atom_loss`) plus the
  already-shipped `SpectatorBondLoss` (ring topology) are considered
  sufficient coverage of ROADMAP Item 1 for now. `invalid_ring_topology`
  does not need its own separate design doc. `invalid_valence`/charge/
  aromaticity remains a genuinely open, explicitly later, not-yet-scoped
  question — not bundled into v0.37.0.
