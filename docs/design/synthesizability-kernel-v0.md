# RENKIN Synthesizability Kernel v0 — Design Doc

Status: **draft, Phase 1 (PR 1) not yet implemented**. Base commit:
`8e3a7cd3ed6c13541d722be0274810f4192bc585` (`origin/master`).

This document is the synthesis of two independent read-only investigations
(API/type/serde-boundary review; chemistry-signal/PR #70-parity review) plus
the orchestrator's own audit of open PRs, worktrees, and file conflicts. It
exists to ground every design choice below in what the codebase actually
does today, not what a prior design assumed it does.

## 0. What this is, in one paragraph

A new module, `src/synthesizability/`, that takes an already-produced set of
`search::Route`s for a target (it does not run search itself) plus stock,
template, and evidence context, and returns a decisive, machine-readable,
auditable assessment of how well-supported a synthesis route is under the
*current* stock/templates/evidence/config — never a claim that a molecule
*cannot* be synthesized, and never an uncalibrated score.

## 1. Base state and open-PR conflict audit

- Base SHA: `8e3a7cd3ed6c13541d722be0274810f4192bc585` (`origin/master`,
  the commit PR #70 merged into).
- Open PRs at time of writing: **#68** (`feat/forward-benchmark-protocol`)
  and **#56** (`feat/mcp-2026-07-28-dual-era`).
- **#68's changed files**: `.github/workflows/ci.yml`, `CHANGELOG.md`,
  `crates/renkin-forward/**`, `docs/guides/forward-benchmark.md`,
  `mkdocs.yml`. **Zero overlap** with anything this design proposes
  touching.
- **#56's changed files**: `CHANGELOG.md`, `README.md`, `README_ja.md`,
  `docs/guides/mcp.md`, `mkdocs.yml`, `src/bin/mcp.rs`, `src/lib.rs`,
  `src/mcp/**`, `tests/fixtures/mcp/**`, `tests/mcp_transcript.rs`. **One
  real overlap**: `src/lib.rs`. This is expected and pre-scoped: the
  orchestrator's *only* edit to `src/lib.rs` for PR 1 is a single
  `pub mod synthesizability;` line, added last, after every agent branch is
  merged into the integration branch — a trivial one-line addition that
  will merge-conflict-resolve in seconds whichever PR lands second. No
  other file in PR 1 touches anything #56 or #68 already changed.
- **Issue #71 status** (important, load-bearing for §4 below): confirmed
  false-positive stock-membership bug in `ChemEnv::is_building_block`
  (VF2 subgraph fallback). Fix exists on branch
  `fix/vf2-stock-identity-false-positive`, opened as **PR #74**, CI green,
  **not yet merged**. This design is written so its correctness does not
  depend on #74's merge timing (see §4.2).
- `tests/` does not exist yet at the top level of `origin/master` — it's
  created by #56 (`tests/fixtures/mcp/**`, `tests/mcp_transcript.rs`) and,
  independently, by this design's Agent D (`tests/synthesizability_kernel.rs`,
  `tests/fixtures/synthesizability/**`). Different files under a
  not-yet-existing shared directory — no real conflict, git handles this
  fine regardless of merge order.
- `AGENTS.md` is a **gitignored, local-only file** (`.gitignore` line 24) —
  it was carrying stale guidance describing the VF2 bug as an intentional
  design decision. It has been corrected locally as an unrelated
  housekeeping item; it is not part of any PR and needs no further action
  here.

## 2. Existing-type reuse plan

Reuse without modification (Phase 1 never edits these):

| Type / fn | Where | Reused for |
|---|---|---|
| `search::Route`, `search::ReactionStep`, `search::SearchStats` | `src/search.rs` | `assess_routes()`'s primary input |
| `chem_env::standardize`, `chem_env::STANDARDIZE_OPTS` | `src/chem_env.rs` | The kernel's own stock-identity canonicalization (see §4.2 — deliberately **not** `ChemEnv::is_building_block`) |
| `chematic::smiles::canonical_smiles` | via chematic | Same, and for `canonical_target`/route-id hashing |
| `evidence::ExampleMatch`, `ReportedYield`, `MetadataSource`, `StepEvidence` | `src/evidence.rs` | Evidence-coverage signals (§4.4) — consumed as-is from `ReactionStep.evidence`, no reimplementation |
| `validation::StepValidationStatus` (`Valid`/`Invalid`/`NotEvaluable`) | `src/validation/mod.rs` | The forward-validation-per-step input shape (§4.3) — chosen over reimplementing either of the two existing forward-validation engines |

Explicitly **not** reused, with reasons:

- `ChemEnv::is_building_block` / `is_building_block_smiles` — see §4.2:
  reusing this couples kernel correctness to (a) whether #71/PR #74 has
  merged and (b) which canonicalization basis is linked, neither of which
  the kernel should have to know about.
- `atom_conservation::step_balanced`/`route_balanced` (MW-sum only) and
  `ReactionStep.atom_economy` (self-referential MW ratio, `.min(100.0)`
  clamped) — see §4.5: **not safe as a hard-failure signal**. Confirmed:
  Issue #72's `extracted_9` template self-reports `atom_economy: 100.0`
  while actually dropping most of the target's heavy atoms — the clamp
  hides exactly the case that should be the reddest flag.
- `graph_rules::element_counts`/`delta_matches` — closest existing Rust
  analog to the needed per-element accounting, but H-inclusive (Python's
  check is heavy-atom-only) and shaped for exact-delta equality, not a
  one-directional inequality. Reference for naming/structure only, not a
  drop-in (§4.5, new implementation required).
- `crates/renkin-forward`'s validator and `src/validation/forward.rs`'s
  validator — two independent, behaviorally different reimplementations of
  "does this reaction reproduce the target." Rather than pick one or write
  a third, Phase 1 takes forward-validation results as an **input**
  (§4.3) — a scope decision, not an oversight.
- `SearchStats.nodes_expanded`/`matched_templates`/`stock_hits` — real
  fields, but `nodes_expanded` is **always 0 on wasm32** (declared
  unconditionally, incremented only under
  `#[cfg(not(target_arch = "wasm32"))]`), and `stock_hits`/`matched_templates`
  are not distinct-entity counts (they double-count across repeated
  frontier visits and cache hits). Not reused for anything in the
  `AssessmentStatus` decision path; may appear in provenance as
  caller-supplied context only if the caller wants to attach them.

## 3. Why no existing "why did search stop" signal can be trusted — and what this means for `no_route_found_within_budget` vs `indeterminate`

This is the single most important finding from the review, because the
entire premise of the kernel (never say "unsynthesizable" when zero routes
come back) depends on knowing *why* zero routes came back.

`find_routes` (`src/search.rs:683-688`) returns `Result<(Vec<Route>,
SearchStats)>` with no stop-reason field. Tracing the main search loop:
it terminates when either (a) the frontier heap empties, or (b)
`routes.len() >= config.max_routes`. **`SearchConfig` has no time or node
budget today — only `max_depth`, `max_routes`, `beam_width`.** So if zero
routes are returned, condition (b) cannot be the reason (zero can't be
"enough"), which means the heap emptied — i.e. **the full search space
within the given depth/beam bounds was exhausted**. Today, depth and beam
width *are* the budget, and there is no other way to get zero routes.

**Design consequence**: with the *current* `find_routes`, a zero-route
result can always be classified `no_route_found_within_budget` — the
"budget" being the caller-supplied `SearchConfig` (depth/beam/max_routes),
which the kernel does not compute itself but requires the caller to pass
in alongside the routes (see `AssessmentContext` below). **`indeterminate`
is reserved for a future state** — if/when a wall-clock or node-count
budget is added to `SearchConfig` and a search can be cut off mid-run
without the heap being empty, that case genuinely cannot be told apart
from "space exhausted" without an explicit stop-reason field, and
`indeterminate` is where it goes until `SearchStats` grows one. Phase 1
does not add that field (it would be a `search.rs` change, out of scope —
see Non-Goals) — it just reserves the enum variant so PR 2 (`find_and_assess`)
can wire it up later without a schema break.

This also explains why `main.rs::diagnose()`, `wasm.rs`, `python.rs`, and
`mcp.rs::handle_diagnose_failure` are four independent, differently-shaped
reimplementations of "why didn't this work" today — there's no shared type
for them to converge on. The kernel is a natural place to unify this later
(PR 2+), not a goal of PR 1.

## 4. New types and status semantics (`src/synthesizability/`)

```
pub const SYNTHESIZABILITY_SCHEMA_VERSION: u32 = 1;
```

### 4.1 `AssessmentStatus`

```rust
pub enum AssessmentStatus {
    InvalidTarget,
    EvaluationError,
    NoRouteFoundWithinBudget,
    Indeterminate,          // reserved; unreachable with today's find_routes (see §3)
    RoutesFoundButRejected,
    RouteSupportedWithValidationGaps,
    RouteSupported,
}
```

Decision order (first match wins), matching the spec exactly:

1. Target SMILES doesn't parse → `InvalidTarget`.
2. The assessment itself couldn't complete (e.g. a stock entry's SMILES is
   unparseable and `SynthesizabilityConfig` doesn't tolerate that,
   internal invariant violation) → `EvaluationError`. This is a kernel
   integrity failure, never a chemistry judgment.
3. Zero routes supplied → `NoRouteFoundWithinBudget` (see §3; today this is
   always reachable and always correct given `find_routes`'s actual
   termination conditions).
4. (Reserved, unreachable today) zero routes, stop reason unknown →
   `Indeterminate`.
5. One or more routes supplied, every one has ≥1 hard failure →
   `RoutesFoundButRejected`.
6. At least one route has zero hard failures, but none clears every
   `SynthesizabilityConfig`-configured *required* check →
   `RouteSupportedWithValidationGaps`.
7. At least one route has zero hard failures and clears every configured
   required check → `RouteSupported`.

`RouteSupported`'s doc comment (load-bearing, must ship verbatim in the
code): *"This means the route is supported under the configured search,
stock, templates, and validation policy. It is not a guarantee of
experimental synthesis success."*

### 4.2 Stock-termination signal — independent re-verification, not `is_building_block`

Seven states (post-review update: `StockNotSupplied`/"caller passed `None`"
and "caller passed `Some(&[])`" were originally conflated; split into two
distinct states, since only the latter implies the caller actually tried to
configure a stock):

```rust
pub enum StockTerminationStatus {
    AllLeavesVerifiedInConfiguredStock,
    OneOrMoreLeavesNotInStock,
    StockNotSupplied,           // AssessmentContext::stock was None
    StockSuppliedButEmpty,      // AssessmentContext::stock was Some(&[])
    StockIdentityUnavailable,   // a leaf SMILES fails to parse under the kernel's own pipeline
    StockCheckNotPerformed,     // config disabled this check
    StockCheckError,            // internal error distinct from a genuine mismatch
}
```

A non-empty stock list where every single entry fails to parse is a
different, more severe case than either of the above — a data-quality
failure on the caller's own input, not "no stock." It is not a
`StockTerminationStatus` variant at all: `assess_routes()` short-circuits to
`AssessmentStatus::EvaluationError` before assessing any route in that case
(see `StockInputStatus::AllEntriesInvalid` below), so no route ever reaches
this per-route check with that condition.

`StockInputStatus` (new, computed once per `assess_routes()` call, echoed on
`AssessmentProvenance::stock_input_status`) makes the caller's raw stock
argument fully self-describing, since `stock_count`/`stock_hash` alone
cannot distinguish "no stock" from "an unusable stock":

```rust
pub enum StockInputStatus {
    NotSupplied,        // None
    SuppliedButEmpty,   // Some(&[])
    AllEntriesInvalid,  // Some(non_empty), 0 parsed -> escalates to EvaluationError
    PartiallyUsable,    // Some(non_empty), some parsed, some didn't
    FullyUsable,        // Some(non_empty), all parsed
}
```

**Decision: the kernel does its own stock-identity check. It does not call
`ChemEnv::is_building_block`.**

Reasoning (from the parity review):

- `ChemEnv::is_building_block` on `master` today is the confirmed-buggy
  VF2 path. Calling it means inheriting 156 confirmed false positives.
- Even post-#71, `is_building_block_smiles`'s correctness implicitly
  depends on whether the linked `chem_env.rs` standardizes stock entries
  at load time — a branch-dependent contract the kernel shouldn't have to
  track.
- PR #70's own Python common-validator follows exactly this pattern:
  trust the tool's *classification* of which leaves are terminal
  (`route.building_blocks`), but independently re-verify *whether that
  classification is factually correct* against a separately-canonicalized
  stock set (`compare_validation.py`'s `validate_stock_leaves`, RDKit
  canonicalization, never trusting RENKIN's own verdict).
- The Rust kernel's analog: reuse `chem_env::standardize`/`STANDARDIZE_OPTS`
  (the identity *policy* is right and already documented) plus chematic's
  `canonical_smiles` (**not** RDKit — this is an in-tree Rust kernel, and
  CHANGELOG.md documents chematic `0.8.1`'s `canonical_smiles()` as now
  "genuinely construction-path invariant," which is the property the
  Python harness's original chematic-avoidance rationale was worried
  about). This makes kernel correctness independent of `is_building_block`'s
  *logic* (and therefore independent of whether #74 has merged), while
  still depending on chematic ≥0.8.1 as a pinned assumption — call this
  out in provenance.
- `StockIdentityUnavailable` is distinct from `StockCheckError`: the
  former is "this specific leaf's SMILES doesn't parse/standardize," a
  fact about the input; the latter is "something went wrong in the check
  logic itself," a kernel bug.

### 4.3 Forward-validation signal — taken as input, not (re)computed

Five states (per spec):

```rust
pub enum ForwardValidationStatus {
    AllEvaluatedStepsValid,
    OneOrMoreStepsInvalid,
    PartiallyEvaluated,
    NotEvaluated,
    ValidatorError,
}
```

**Decision: `assess_routes()` accepts an optional
`Option<Vec<StepValidationStatus>>` per route (reusing
`validation::StepValidationStatus` from `src/validation/mod.rs` verbatim)
instead of invoking any validator itself.**

Reasoning: two independently-maintained forward-validation engines already
exist (`crates/renkin-forward`'s binary `verified: bool` with
hard-abort-on-error; `src/validation/forward.rs`'s three-way
`StepValidationStatus` with a VF2-based structural fallback whose
docstring cites `ChemEnv::is_building_block`'s VF2 precedent — a
justification that goes stale the moment #74 lands). Picking one couples
the kernel to that engine's specific behavior and bugs; writing a third
duplicates effort for no clear gain. Taking validation as input keeps
Phase 1 a pure function of already-computed data, consistent with the
"does not run search" scope boundary, and lets a future PR decide (or let
the *caller* decide) which validator produced the input.

Mapping from per-step `StepValidationStatus` to the route-level status:
`None` supplied → `NotEvaluated`. All `Valid` → `AllEvaluatedStepsValid`.
Any `Invalid` → `OneOrMoreStepsInvalid`. Otherwise (some `Valid`, some
`NotEvaluable`, no `Invalid`) → `PartiallyEvaluated`. A structurally
inconsistent input (e.g. wrong step count for the route) → `ValidatorError`
— a kernel-side integrity error, not a chemistry judgment.

Post-review fix: under `ForwardValidationPolicy::RequireAllValid` specifically,
a per-route `ValidatorError` previously only produced a warning inside
`assess_one_route`, so a route with a structurally broken validator input
could still reach `RouteSupported`. `assess_routes()` now checks, before
assessing any route, whether any route's own per-step slice length matches
that route's own step count; a mismatch under `RequireAllValid` escalates
the whole assessment to `AssessmentStatus::EvaluationError` (mirroring the
existing across-routes length check, §4.1 #2, one level deeper) rather than
letting it slip through as a per-route warning. Under `Ignore`/
`RequireNoInvalid`, `ValidatorError` remains intentionally tolerated —
neither validator's reliability has been measured yet, so this fix is
scoped to `RequireAllValid` only, where the caller has explicitly asked
this kernel to enforce forward validation.

### 4.4 Evidence coverage — direct reuse of `src/evidence.rs`

```rust
pub struct EvidenceCoverage {
    pub exact_substrate_evidence_steps: usize,
    pub template_level_evidence_steps: usize,
    pub steps_without_evidence: usize,
    pub steps_with_reported_yield: usize,
    pub steps_with_conditions: usize,
    pub steps_with_warnings: usize,
}
```

Computed directly from each `ReactionStep.evidence: Option<StepEvidence>`
and its `examples[].match_kind` (`ExactSubstrate`/`TemplateOnly`) — no new
evidence logic. One deliberate guard, flagged by the parity review:
`MetadataSource` has a reserved-but-currently-unconstructed
`ModelPrediction` variant ("output of a trained yield/condition-prediction
model"). `steps_with_reported_yield` must only count a
`ReportedYield` whose `source != MetadataSource::ModelPrediction` — "yield
is never a prediction" is true today only because nothing builds that
variant yet, and the kernel must not bake in an assumption that stops
holding the moment it is.

### 4.5 Target-element accounting — new Rust implementation required, three failure classes

```rust
pub enum ElementAccountingStatus {
    Accounted,
    UnaccountedTargetElement,
    NotEvaluable,
}
```

**No existing Rust code implements this check's actual semantics** (see
§2 — `atom_conservation` is MW-sum-only, `graph_rules::element_counts` is
H-inclusive and delta-equality-shaped). Agent B implements it fresh in
`element_accounting.rs`: per step, per element, target's heavy-atom count
(H excluded, matching `compare_validation.py`'s `_heavy_atom_counts`) must
not exceed the sum over precursors — a one-directional inequality, never
described as mass conservation. A cross-language parity test against
`scripts/compare_validation.py`'s fixtures is required (Agent D); expect
and document, rather than chase as a bug, disagreement at the
`NotEvaluable` boundary caused by chematic vs. RDKit parsing differently
on some inputs — that's a parser-acceptance difference, not an accounting-logic
bug.

**Three distinct failure classes feed into `UnaccountedTargetElement`, and
the design doc records this explicitly so PR 1 doesn't collapse them:**

1. **Genuine template-extraction defect** (#72-style) — real correctness bug.
2. **Intentionally-unmodeled reagent** (#73-style — `boc_deprotection_retro`,
   `cbz_deprotection_retro`, possibly `aryl_amine_retro`) — an accepted
   search-rule limitation, not a defect. RENKIN already has partial
   precedent for encoding this as a calibrated exception rather than a
   failure: `graph_rules.rs`'s `BOC_DELTA`/`CBZ_DELTA` exact-formula
   allowlist. Note `aryl_amine_retro` is SMIRKS-based, not one of the
   graph-based rules `graph_rules.rs` covers, so it has **no existing
   carve-out** — per #73 (still unresolved), it is *not* assumed to be the
   same class as Boc/Cbz and must not be silently allowlisted alongside
   them.
3. **Fragment-filter artifact** — `split_fragments` silently drops any
   fragment with aromatic atoms but no ring-closure digit (BFS-leakage
   cleanup). This is invisible to the kernel: `assess_routes()` only sees
   the already-filtered `Route`, it cannot reach back into
   `split_fragments`'s internal decision to tell class 3 apart from class
   1. **This is a documented, accepted limitation of Phase 1**, not
   something this PR attempts to solve (would require new instrumentation
   inside `chem_env.rs` itself, out of scope).

**Phase 1 decision**: `SynthesizabilityConfig` carries a
`reagent_omission_template_allowlist: Vec<String>` (template IDs, default:
`["rule:boc_deprotection_retro", "rule:cbz_deprotection_retro"]` —
**not** `aryl_amine_retro`, pending #73). A step whose rule's
`template_id` is on this allowlist and whose only accounting problem is
`UnaccountedTargetElement` is recorded as a **validation gap**, not a hard
failure, regardless of `conservative()`/`diagnostic()` — because the
omission is by construction, not a search-quality question. Every other
`UnaccountedTargetElement` (classes 1 and 3, which the kernel cannot tell
apart) defaults to a **hard failure** under `conservative()` and a
**validation gap** under `diagnostic()`.

### 4.6 Top-level types

```rust
pub struct SynthesizabilityAssessment {
    pub schema_version: u32,
    pub target: String,               // as supplied
    pub canonical_target: Option<String>, // None only if InvalidTarget
    pub status: AssessmentStatus,
    pub selected_route: Option<RouteAssessment>,
    pub route_assessments: Vec<RouteAssessment>, // deterministic order, see §6 -- may be a truncated view, see below
    pub routes_supplied_count: usize,       // routes.len() as supplied; always the true input count, even on an early short-circuit
    pub routes_assessed_count: usize,       // routes actually assessed; 0 on an early short-circuit, else == routes_supplied_count today (a distinct field for a future partial-budget assessment)
    pub route_assessments_returned_count: usize, // route_assessments.len() after output-shaping
    pub route_assessments_truncated: bool,  // true iff returned_count < assessed_count
    pub provenance: AssessmentProvenance,
    pub config_used: SynthesizabilityConfigSummary, // echoes the config, for audit
    pub warnings: Vec<String>,
}

pub struct RouteAssessment {
    pub route_id: String,             // deterministic, see §6
    pub route_depth: u32,
    pub route_cost: f64,              // reused verbatim from Route.route_cost
    pub stock_termination_status: StockTerminationStatus,
    pub target_element_accounting_status: ElementAccountingStatus,
    pub forward_validation_status: ForwardValidationStatus,
    pub evidence_coverage: EvidenceCoverage,
    pub hard_failures: Vec<HardFailure>,
    pub validation_gaps: Vec<ValidationGap>,
    pub warnings: Vec<String>,
}

pub enum HardFailure {
    RouteStructureUnparseable,
    StockTerminalMismatch { leaf: String },
    UnaccountedTargetElement { step_index: usize },   // class 1 or 3, see §4.5
    ForwardValidationFailed { step_index: usize },     // only when configured as required
    RouteGraphInconsistent,
}

pub enum ValidationGap {
    ForwardValidationNotRun,
    NoEvidence,
    NoExactSubstrateEvidence,
    StepNotEvaluable { step_index: usize },
    StockNotSupplied,             // StockTerminationStatus::{StockNotSupplied, StockSuppliedButEmpty}
    StockProvenanceHashMissing,   // reserved, unreachable -- see below
    ReagentOmissionAccountingGap { step_index: usize, template_id: String }, // class 2, see §4.5
    UnaccountedTargetElementNotEnforced { step_index: usize }, // classes 1/3, diagnostic()-only downgrade
    BestEffortRouteOnly,  // reserved for when a real search timeout/budget concept exists
}
```

Post-review naming fix: `StockProvenanceHashMissing` was originally emitted
whenever no stock was supplied at all — indistinguishable from "no stock
argument was given." Split into two: `ValidationGap::StockNotSupplied`
covers that case (both `StockTerminationStatus::StockNotSupplied` and
`StockSuppliedButEmpty` land here — the per-route consequence is identical,
`StockInputStatus` on the top-level provenance is where the two are told
apart). `StockProvenanceHashMissing` is reserved for its originally-intended,
narrower meaning — "stock was supplied and used for this check, but its
provenance/hash metadata is incomplete" — and is currently **unreachable**:
the only candidate trigger (`AssessmentContext::stock_source` being `None`)
would demote nearly every assessment run without a source label out of
`RouteSupported`, a verdict-level behavior change beyond a naming fix. Kept
reserved-but-unused the same way `AssessmentStatus::Indeterminate` already
is in this schema, pending a maintainer decision on whether to wire it up.

### 4.7 `SynthesizabilityConfig`

```rust
pub struct SynthesizabilityConfig {
    pub require_verified_stock_terminal: bool,
    pub require_target_element_accounting: bool,
    pub forward_validation_policy: ForwardValidationPolicy, // Ignore | RequireAllValid | RequireNoInvalid
    pub evidence_policy: EvidencePolicy,                     // Ignore | RequireAnyEvidence | RequireExactSubstrate
    pub reagent_omission_template_allowlist: Vec<String>,
    pub max_route_diagnostics: usize,       // post-review rename from max_routes_to_assess -- it only ever bounded *output* size (see §4.1 #5-#7), never how many routes were actually assessed; the old name implied otherwise. Safe to rename: schema v1 was still unreleased.
    pub include_all_route_diagnostics: bool,
}

impl SynthesizabilityConfig {
    pub fn conservative() -> Self { /* require_verified_stock_terminal: true,
        require_target_element_accounting: true, forward_validation_policy:
        Ignore (not yet validated by default -- see §4.3), evidence_policy:
        Ignore -- absence of evidence is never a hard failure by default,
        per explicit spec instruction */ }
    pub fn diagnostic() -> Self { /* same hard-requirement toggles as
        conservative() except unaccounted-element outside the allowlist is
        a validation gap, not a hard failure -- for exploratory use */ }
}
```

Per the explicit spec instruction: **evidence absence is never a hard
failure by default**, and **forward validation is never a hard requirement
by default** unless the caller opts in — because neither this validator's
coverage nor its false-positive/negative rate has been measured yet.
"Hidden fallback or implicit stock substitution" is disallowed by
construction: `StockNotSupplied` is a distinct status, never silently
treated as `StockCheckNotPerformed` or, worse, "assume it's fine."

### 4.8 Route selection — lexicographic classification, not a single score

Per the explicit spec instruction, `selected_route` is **never** chosen by
a single uncalibrated score. `route_assessments` is sorted by this
lexicographic key (ascending unless noted), and `selected_route` is the
first entry with `status` in `{RouteSupported,
RouteSupportedWithValidationGaps}` (if none qualifies, `selected_route` is
`None` even though `route_assessments` is non-empty):

1. `hard_failures.len()` (fewer is better)
2. `validation_gaps.len()` (fewer is better)
3. `stock_termination_status == AllLeavesVerifiedInConfiguredStock` (true first)
4. `target_element_accounting_status == Accounted` (true first)
5. Forward-validation coverage rank (`AllEvaluatedStepsValid` >
   `PartiallyEvaluated` > `NotEvaluated`/`OneOrMoreStepsInvalid` — the
   latter two only reachable here if not already excluded as a hard
   failure)
6. Evidence coverage rank (more `exact_substrate_evidence_steps`, then
   more `template_level_evidence_steps`, wins)
7. `route_cost` (existing field, lower is better — this is the *existing*,
   already-disclaimed cost heuristic, used only as a tie-break, never
   promoted to a primary signal)
8. `route_depth` (fewer is better)
9. `route_id` (lexicographic — pure tie-break for full determinism)

Every field in this key is machine-readable directly off
`RouteAssessment` — "selection rationale" is not a separate opaque field,
it *is* this tuple, and `include_all_route_diagnostics` controls whether
callers see every route's full tuple or just the selected one's.

## 5. Provenance (`AssessmentProvenance`)

```rust
pub struct AssessmentProvenance {
    pub renkin_version: String,             // env!("CARGO_PKG_VERSION")
    pub assessment_schema_version: u32,
    pub canonical_target: String,
    pub rules_count: usize,
    pub rules_hash: String,                 // sha256 over sorted template_ids + smirks, same style as ChemEnv::content_sha256
    pub stock_count: usize,
    pub stock_hash: String,                 // sha256 over sorted kernel-canonicalized stock entries
    pub stock_input_status: StockInputStatus, // see §4.2 -- distinguishes "no stock" from "unusable stock", which stock_count/stock_hash alone cannot
    pub stock_source: Option<String>,       // caller-supplied label (file path, "embedded", etc.) -- never a silent default
    pub template_metadata_hash: Option<String>,
    pub search_config_summary: Option<String>, // caller-supplied echo of the SearchConfig used to produce the routes; the kernel does not re-run or infer this
    pub assessment_config_hash: String,
    pub git_commit: Option<String>,         // caller-supplied only -- see §5.1
    pub embedded_fallback_used: Option<bool>, // caller-supplied only -- see §5.1
    pub reproducibility_hash: String,
    pub reproducibility_exclusions: Vec<String>, // e.g. ["timing_ms", "wall_clock_timestamp"] -- documents what was deliberately left out, not just what was included
}
```

### 5.1 WASM constraint — provenance fields the kernel cannot compute itself

`src/lib.rs` declares one shared library crate compiled as both `rlib`
(native) and `cdylib` (wasm32, via wasm-bindgen) — there is no separate
wasm-only crate, and a new `pub mod synthesizability;` added the way every
other module is (`search`, `evidence`, etc.) compiles into the wasm32
target by default. Existing wasm-safety discipline is per-call-site, not
per-module (`src/search.rs`'s node-count/timing instrumentation is
`#[cfg(not(target_arch = "wasm32"))]`-gated at the exact line that needs
it, not at the module level).

`git_commit` and `embedded_fallback_used` are **caller-supplied inputs to
`assess_routes()`/`AssessmentContext`, never computed in-crate** via
`std::process::Command` or filesystem probing — this sidesteps the wasm
problem entirely rather than requiring cfg-gating inside
`src/synthesizability/`. `stock_hash`/`rules_hash` are computed from data
already in memory (the supplied stock list / rule list), so they need no
gating. No wall-clock timestamp is computed anywhere in this module.

## 6. Deterministic output design

- No `HashMap` iteration order ever reaches serialized output — sort
  before collecting into `Vec`, or use `BTreeMap`, matching
  `crates/renkin-forward`'s established convention.
- `route_id`: `sha256:<hex>` over a fixed domain-separator string (its own,
  e.g. `b"renkin-synthesizability-route-v1\0"` — never colliding with
  `renkin-forward`'s `candidate_id_for` or its own enumeration-candidate
  hash) plus the canonical target and the sorted, canonicalized
  `(template_id, target, precursors)` tuple for every step — deterministic
  regardless of route discovery order.
- `reproducibility_hash`: combines `rules_hash`, `stock_hash`,
  `assessment_config_hash`, `canonical_target`, and every route's
  `route_id` plus its `status`/`hard_failures`/`validation_gaps` — with
  `reproducibility_exclusions` documenting explicitly that timing and
  wall-clock fields are never part of the preimage. Two identical inputs
  must produce a byte-identical `SynthesizabilityAssessment` JSON except
  for whatever the caller stamps in outside this hash (there is nothing
  time-based generated inside this module at all, so full output is
  expected to be byte-identical, not just the hash).

## 7. Per-agent file ownership (unchanged from the original spec, confirmed no conflicts)

| Agent | Files | Depends on |
|---|---|---|
| **A** — schema | `src/synthesizability/mod.rs`, `src/synthesizability/schema.rs` | Nothing (first to land) |
| **B** — signals | `src/synthesizability/signals.rs`, `src/synthesizability/element_accounting.rs` | Agent A's types (branches from the integration branch *after* A merges, not from raw `origin/master`) |
| **C** — assessment/provenance | `src/synthesizability/assessment.rs`, `src/synthesizability/provenance.rs` | Agent A's types, same branching rule as B |
| **D** — tests/fixtures | `tests/synthesizability_kernel.rs`, `tests/fixtures/synthesizability/**` | A (always); fixture JSON authoring can proceed in parallel with B/C, but the test file itself needs B+C's real function signatures, so it lands last |
| **Orchestrator only** | `src/lib.rs` (`pub mod synthesizability;`, one line) | Added after A merges to the integration branch, once, not touched by any agent |

**Sequencing correction from the original plan**: A, B, C, D cannot all
branch from `origin/master` independently and merge cleanly, because B
and C both consume types A defines — if they branch before A lands, they
either can't compile against anything or invent incompatible local copies
of `AssessmentStatus`/`RouteAssessment` that won't reconcile. Actual
sequence: **A lands on the integration branch (`feat/synthesizability-kernel-v0`)
first**; B and C's worktrees are then created *from that updated
integration branch* (still using the branch names `agent/synth-b-signals`/
`agent/synth-c-assessment` the spec requested); D's fixture JSON can be
authored any time after A, but `tests/synthesizability_kernel.rs` itself
needs B and C's real signatures, so it's implemented last, after both
merge back to the integration branch.

## 8. PR split (unchanged from spec)

- **PR 1** (this doc's scope): `src/synthesizability/**`,
  `tests/synthesizability_kernel.rs`, `tests/fixtures/synthesizability/**`,
  this design doc, the one-line `src/lib.rs` module export. Draft only, not
  merged without explicit review.
- **PR 2** (later, after PR 1 merges): `find_and_assess(...)` — calls
  `find_routes` then `assess_routes`, wires real `SearchStats`/termination
  info into provenance, potentially adds the reserved `Indeterminate` path
  if a real time/node budget is added to `SearchConfig` at the same time.
- **PR 3** (later): `renkin assess --target ... --format json` CLI,
  versioned JSON, explicit human-readable disclaimer text as specified.
- **PR 4** (later, after PR 1-3's schema is stable): Python/WASM bindings.
- **MCP integration**: after PR #56 merges, as its own PR.

## 9. Test fixture plan (Agent D)

Every fixture in the original spec's list, plus:

- The three real Issue #71 false positives (1,4-pentadiene: `C=CCC=C`;
  glyoxylic acid: `O=CC(=O)O`; phenylacetaldehyde: `O=CCc1ccccc1`) as
  concrete `StockTerminationStatus` regression fixtures — asserting the
  kernel's *own* independent stock check correctly reports
  `OneOrMoreLeavesNotInStock` for these, regardless of which `chem_env.rs`
  (pre- or post-#74) happens to be linked. This is the fixture set that
  most directly proves §4.2's design decision was right.
- A parity fixture set mirrored from `scripts/compare_validation.py`'s own
  test fixtures for `check_target_element_accounting`, for the
  cross-language parity test required in §4.5.
- A `reagent_omission_template_allowlist` fixture pair: one route using
  `rule:boc_deprotection_retro` with an otherwise-clean accounting failure
  (expect `ValidationGap::ReagentOmissionAccountingGap`, not a hard
  failure) and one using `rule:aryl_amine_retro` with the same shape of
  accounting failure (expect a hard failure — #73 is unresolved, so this
  rule is deliberately *not* on the default allowlist).

## 10. Non-goals for PR 1

- Any change to `search.rs`'s search algorithm, route ranking, or template
  ranking.
- Any new stock file, template file, or trained model.
- Any 0–100 score, calibrated probability, or reuse of `success_probability`'s
  name/semantics for anything resembling a synthesizability score.
- CLI, Python, WASM, or MCP exposure (PR 2+).
- Resolving Issue #72 or #73 — this design only ensures PR 1 doesn't
  *assume* either is resolved, and encodes the current, explicitly
  unresolved state (allowlist excludes `aryl_amine_retro`) faithfully.
- Adding a real time/node search budget to `SearchConfig` (that's what
  would make `Indeterminate` reachable — reserved, not built, here).

## 11. Known limitations (ship these in doc comments, not just here)

1. The kernel cannot distinguish a genuine template-extraction defect
   (#72-class) from a `split_fragments` fragment-filter artifact — both
   surface as `UnaccountedTargetElement` with no further attribution.
   Fixing this needs new instrumentation inside `chem_env.rs`, out of scope.
2. `Indeterminate` is unreachable with today's `find_routes` — this is
   correct, not a bug, per §3, but worth a comment at the enum definition
   so nobody "fixes" it into reachability without also adding a real
   search budget.
3. Kernel correctness for stock-checking depends on chematic ≥0.8.1's
   `canonical_smiles()` invariance claim (CHANGELOG.md-documented, not
   independently re-verified by this design).
4. `success_probability`'s calibration disclaimer is already violated by
   two existing surfaces (`mcp.rs`'s `plan_with_constraints` tool
   description, `main.rs`'s `"most_reliable"` Pareto label) — this design
   does not fix those; flagged for a maintainer decision, independent of
   this PR.
5. Chematic's `canonical_smiles()` does not strip vestigial `/`/`\`
   bond-direction markers on a non-stereogenic double bond (confirmed:
   `C=C/C/C=C` and `C=CCC=C`, the same real molecule, 1,4-pentadiene,
   canonicalize to two different strings). This is a concrete instance of
   item 3's chematic-invariance dependency, found by the independent
   reviewer pass: the direction of the risk is a **false negative**
   (over-rejection) — a leaf genuinely in stock, written with such
   markers, can be wrongly reported `OneOrMoreLeavesNotInStock`. Not a
   kernel-introduced regression: `ChemEnv::is_building_block`'s primary
   path has the identical limitation (and the kernel is arguably more
   careful, since it standardizes both the stock side and the query side,
   where `is_building_block` only standardizes the stock side at load
   time).
6. Stoichiometric multiplicity is entirely unmodeled: if a route's
   `precursors` lists a building block once but the real reaction needs
   two-plus equivalents of it, nothing in `element_accounting.rs` or
   `signals.rs` catches that — heavy-atom sums only check element
   totals, not per-species quantities needed.
7. A zero-step route's element-accounting status is `NotEvaluable`
   (nothing to atom-balance with no reaction steps), which -- unlike the
   analogous "required check couldn't run" case for forward validation
   under `RequireAllValid` (`ValidationGap::ForwardValidationNotRun`) --
   contributes neither a hard failure nor a validation gap under
   `require_target_element_accounting: true`. Found by the independent
   reviewer pass during the PR #75 fix-up round: arguably correct (no
   steps means nothing to check), but it's an asymmetry with this same
   round's zero-step stock re-verification fix (§4.2), worth a deliberate
   maintainer decision rather than silently living with the inconsistency.
