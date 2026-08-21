# RENKIN Audit Policy Profiles — Design Doc

Status: **Design audit only, per explicit user instruction (2026-08-21):
no implementation, no version bump, no publish in this round.** This
document inventories the real finding/check taxonomy against the
`informational`/`standard`/`strict` policy semantics already promised in
`docs/guides/audit-reproducibility-contract.md` (published with v0.27.0,
never implemented beyond `standard`), and surfaces the concrete design
questions and scope gaps that block a clean v0.29.0 implementation round.
Nothing here is a commitment — §4, §5, and §6 are open questions the user
needs to resolve before implementation starts.

## 0. What this is, in one paragraph

`renkin audit-route` today only implements `policy: "standard"` — the
`AuditManifest.policy` field already exists and is already hardcoded to
that one value (`src/bridge/audit_route.rs`). The already-published
contract doc promises two more policies (`informational`, `strict`) with
a specific 2×3 behavior table, but that table has never been checked
against the actual, current finding taxonomy — it was written
speculatively alongside the manifest field, before this session's v0.28.0
work even existed. This document does that check: a full inventory of
every finding/check axis `bridge::audit` currently produces, confirmation
(or correction) of the existing table against that inventory, and the
non-status-table blockers (a real Python API gap, a WASM signature
question) that the user's own v0.29.0 pass criteria surface once actually
traced through the code.

## 1. Existing-code grounding (read before designing, not after)

- **`src/bridge/audit.rs`** is the whole of today's status-derivation
  logic — one function, `audit_document`, lines 441–548. It runs the
  reaction-steps-parseable check, stock validation, element accounting,
  and per-step forward validation, collects every `AuditFinding` from all
  four into one flat `Vec`, then derives `AuditStatus` from exactly two
  booleans:
  ```rust
  let any_fail = !steps_ok
      || findings.iter().any(|f| f.severity == AuditSeverity::Gating)
      || stock_validation.status == CheckStatus::Fail
      || steps.iter().any(|s| s.forward_validation.status == CheckStatus::Fail);
  let any_not_evaluable = stock_validation.status == CheckStatus::NotEvaluable
      || element_status == ElementAccountingStatus::NotEvaluable
      || steps.iter().any(|s| s.forward_validation.status == CheckStatus::NotEvaluable);
  let status = if any_fail { Fail } else if any_not_evaluable { Partial } else { Pass };
  ```
  This is `policy: "standard"`, fully unparameterized — there is no
  policy input anywhere in this function today.
- **Severity is already the per-finding differentiator**: every
  `AuditFindingCode` maps to exactly one of `AuditSeverity::Gating` or
  `AuditSeverity::Informational` via `AuditFindingCode::severity()` (lines
  75–90). `Informational` findings (`ChargeImbalance`,
  `StereoCenterCountMismatch`) are — by explicit design, and by a passing
  test (`informational_findings_never_gate_status_to_fail`) — never
  allowed to push `any_fail` true. This means the finding taxonomy
  **already carries a two-tier severity model**; policy work extends how
  that model maps to `AuditStatus`, it doesn't need to invent severity
  from scratch.
- **`AuditManifest.policy`** (`src/bridge/audit_route.rs`, `AuditManifest`
  struct) is `&'static str`, hardcoded `"standard"` in
  `build_audit_route_report`. This is the one piece of policy plumbing
  that already exists end-to-end (CLI + WASM both go through
  `build_audit_route_report`, so both already report it) — it just isn't
  wired to anything real yet.
- **No Python API surface exists for audit-route at all.** `grep -c
  audit src/python.rs` returns `0`. This is a real prerequisite gap, not
  a detail — see §5.
- **`docs/guides/audit-reproducibility-contract.md`'s existing table**
  (already public, already promised to users):

  | Policy | Route with only `not_evaluable` checks | Route with a gating finding present |
  |---|---|---|
  | `informational` | `partial` | `partial` (never `fail`) |
  | `standard` (only one shipped) | `partial` | `fail` |
  | `strict` | `fail` | `fail` |

  This table is written at the **aggregate** level (two boolean inputs,
  matching `any_fail`/`any_not_evaluable` above almost exactly — the only
  difference from today's code is that `informational` softens the
  gating-present row from `fail` to `partial`, and `strict` hardens the
  not_evaluable-only row from `partial` to `fail`). Section 3 below
  confirms this table is sufficient and derivable from a single pure
  function, with no additional per-finding-code branching needed.

## 2. Full finding/check inventory (the "棚卸し")

### 2a. `AuditFindingCode` (16-variant closed set, `src/bridge/audit.rs`)

| Finding code | Severity (current) | Emitted today? | Trigger |
|---|---|---|---|
| `RawOutputNotDecodable` | Gating | Yes (via `ParseOutcome::defects`, adapter-specific) | Source JSON doesn't even parse into a route shape |
| `MultipleOrZeroRoots` | Gating | Yes | `route.steps` empty, or (AiZynthFinder) not exactly one root |
| `RootMismatch` | Gating | Yes | First step's target ≠ the route's declared/requested target |
| `CycleDetected` | Gating | Yes | A node appears as its own ancestor while building the tree |
| `DisconnectedReference` | Gating | **No — reserved, never emitted** (doc comment confirms: ported for schema completeness, no code path produces it, no test exercises it) |
| `UnparseableSmilesInRoute` | Gating | Yes | A SMILES in the route doesn't parse |
| `ChildlessNonLeaf` | Gating | Yes | A step's target has zero children after normalization |
| `AmbiguousLeafStatus` | Gating | Yes | A leaf is neither a declared building block nor another step's target |
| `DegenerateSelfReferentialStep` | Gating | Yes | A step's precursor canonicalizes to the same SMILES as its own target |
| `StepArityMismatch` | Gating | **No — reserved, never emitted** (same doc-comment guarantee as `DisconnectedReference`) |
| `LeafClaimedStockNotMatched` | Gating | Yes | A leaf claims `is_stock_leaf: true` but isn't in the configured `--stock` set |
| `LeafUnresolved` | Gating | Yes | A leaf's stock status is ambiguous/unresolved |
| `UnaccountedTargetElement` | Gating | Yes | A target has more of some element than its precursors combined |
| `ChargeImbalance` | **Informational** | Yes | Target/precursor net charge differs |
| `StereoCenterCountMismatch` | **Informational** | Yes | Target/precursor stereocenter count differs |
| `ForwardReactionNotReproduced` | Gating | Yes | Declared-reaction replay ran and produced a different product |
| `ForwardValidationNotEvaluable` | **Informational** | Yes | Declared-reaction replay couldn't reach pass/fail (see 2c) |

Two codes (`DisconnectedReference`, `StepArityMismatch`) need **no**
policy treatment at all — they cannot occur with the current normalizer
implementations, confirmed by the module's own doc comment, not just
absence of a test. Worth a one-line note in whatever policy design doc
ships with the feature, so a future reader doesn't wonder why they're
unreachable in coverage.

### 2b. Non-finding-code status axes

| Axis | Values | Not-evaluable reason(s) |
|---|---|---|
| `stock_validation.status` (`CheckStatus`) | Pass / Fail / NotEvaluable | `StockNotEvaluableReason::StockNotProvided` (only one variant) |
| `target_element_accounting_status` (`ElementAccountingStatus`) | Accounted / UnaccountedTargetElement / NotEvaluable | (no sub-reason type; `NotEvaluable` when no edge in the tree had countable heavy-atom data on both sides) |
| per-step `forward_validation.status` (`CheckStatus`) | Pass / Fail / NotEvaluable | `ForwardNotEvaluableReason`: `MissingReactionRepresentation`, `MissingAtomMapping`, `UnsupportedReactionFormat`, `UnsupportedTemplateSyntax`, `ReactionApplicationError`, `AmbiguousExpectedProduct` (`src/bridge/forward.rs`) |

Every `NotEvaluable` value across all three axes already contributes
identically to `any_not_evaluable` today — there is no existing
distinction between, say, `StockNotProvided` and
`AmbiguousExpectedProduct` in terms of status impact. `Fail` on any of
the three already contributes identically to `any_fail`. This confirms
§1's claim: the *entire* current status-derivation surface reduces to
two booleans, not sixteen-plus-six-plus-one independent signals.

## 3. Proposed policy semantics: formalize the existing table, don't invent new ones

Given §1 and §2, `policy` is fully captured by one pure function:

```rust
fn derive_status(any_gating_present: bool, any_not_evaluable: bool, policy: AuditPolicy) -> AuditStatus {
    match (policy, any_gating_present, any_not_evaluable) {
        (_, true, _) if policy != Informational => Fail,   // gating + standard/strict -> fail
        (Informational, true, _) => Partial,                // gating + informational -> softened
        (_, false, true) if policy == Strict => Fail,       // not_evaluable + strict -> hardened
        (_, false, true) => Partial,                        // not_evaluable + informational/standard
        (_, false, false) => Pass,
    }
}
```

(Written as a match for clarity here, not a proposed literal
implementation — the real function would replace today's inline `if
any_fail { Fail } else if any_not_evaluable { Partial } else { Pass }`
with a call to this, parameterized by policy.)

This reproduces the published table exactly, requires no new
`AuditStatus`/`CheckStatus` variants, and needs no per-finding-code
branching beyond the `Gating`/`Informational` split that already exists.
**This is the recommended design** — smallest diff, fully consistent with
what's already publicly documented, and the invariant ("policy never
hides a finding, only changes verdict derivation") holds trivially
because `derive_status` never touches the `findings` vector at all.

## 4. Open question: does the aggregate model in §3 satisfy the intent, or is finer per-finding granularity actually wanted?

The user's own illustrative sketch for this design round used a
per-finding-code table, e.g. (paraphrased):

| Finding | informational | standard | strict |
|---|---|---|---|
| reaction evidence insufficient | partial | partial | fail |
| stock not specified | **pass/notice** | partial | fail |
| element mismatch | fail | fail | fail |
| unsupported metadata | **pass/notice** | partial | partial |

Three of these four rows match §3's aggregate model exactly (treating
"reaction evidence insufficient" and "unsupported metadata" as
`NotEvaluable`-class, "element mismatch" as `Gating`-class). **One does
not**: "stock not specified" under `informational` is given as
`pass/notice`, but §3's aggregate model (and the *already-published*
contract table) gives `partial` for every `NotEvaluable`-class check
under `informational`, uniformly.

`pass/notice` isn't a value `AuditStatus` has today (`Pass` / `Fail` /
`Partial` only) — introducing it would mean either a fourth status
variant, or a new "Pass, but see findings" flag alongside status. That's
a materially bigger change than "add a policy parameter to an existing
three-way match," and it would special-case exactly one `NotEvaluable`
reason (`StockNotEvaluableReason::StockNotProvided`) out of the seven
that exist across all three axes (§2b) — the codebase currently has no
per-reason distinction to hang that on either.

**Recommendation: treat the illustrative example as informal shorthand,
not a literal spec, and ship §3's aggregate model** — it's what's already
promised in the published contract doc, it needs no new vocabulary, and
"a route with no configured stock is `partial`, not `pass`" is already an
explicitly tested invariant today
(`no_configured_stock_is_partial_not_a_silent_pass`) that a
`pass`-under-`informational` carve-out would contradict for one specific
reason code while leaving the other six untouched — an inconsistency,
not a refinement. **But this needs the user's explicit confirmation
before implementation starts**, since it's a real interpretation choice,
not a fact derivable from the code alone.

## 5. Prerequisite gap: Python has no audit-route surface at all

The user's stated v0.29.0 pass condition is "CLI, Rust API, Python, WASM:
same policy." Tracing the actual code (§1) shows Python isn't at parity
today to extend — there is no `renkin.audit_route`/equivalent in
`src/python.rs` whatsoever. "Add `--policy` to the existing Python
surface" is not a real task, because the surface doesn't exist; the real
task would be "add a first Python audit-route binding, *and* give it a
policy parameter" — meaningfully more scope than the other three
surfaces, which all already have `audit-route`/`audit_route` and just
need a fourth parameter/flag threaded in.

**Two paths, need a decision, not silently picked here:**

- **(a) Descope v0.29.0 to the three surfaces that already exist** (CLI,
  Rust `bridge::audit`, WASM `audit_route`) and track a first Python
  audit-route binding as its own separate, explicitly-scoped future item
  — recommended, since it keeps v0.29.0's actual new surface area to "one
  new enum + a parameter threaded through three existing call sites,"
  matching how narrowly-scoped this session's other rounds (PR1/PR2/PR3)
  stayed.
- **(b) Fold a new Python audit-route binding into v0.29.0** — real
  option, but changes the round's actual size/risk profile substantially
  (a first PyO3 binding for this surface, its own tests, its own docs
  section) and should be sized as such if chosen, not treated as "just
  add `--policy` everywhere."

## 6. Open question: does `audit_route`'s WASM signature get a 4th parameter in place, or a versioned `_v2`?

This codebase already has a live precedent for exactly this situation:
`find_routes` → `find_routes_v2` (`src/wasm.rs`) added a distinct
function name for new parameters specifically "so a caller on an old
build gets a real 'no such export' `TypeError` instead of a
silently-ignored argument" (that function's own doc comment) — because
`find_routes` had real, presumably-existing consumers by the time
`find_routes_v2` shipped.

`audit_route` shipped in v0.28.0 within the last few hours of this same
session, with no known external consumers yet (the playground itself is
the only caller, and it's part of this same repo/release). Adding a
fourth `policy: &str` parameter to `audit_route` directly, rather than
minting `audit_route_v2`, is the smaller-footprint option and matches
"no premature versioning for an API with zero known external callers" —
but this is a judgment call about adoption risk the user is better
positioned to make than an inference from the code, so it's listed here
as a question, not decided.

## 7. Threading sketch (signatures only — not a proposed diff)

```rust
// src/bridge/audit.rs — new, alongside AuditStatus/CheckStatus
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditPolicy { Informational, Standard, Strict }
impl Default for AuditPolicy { fn default() -> Self { Self::Standard } }

// audit_document gains one parameter; every call site (audit(), tests)
// needs updating, but the finding-collection loop is untouched -- only
// the final `let status = ...` block changes, per §3.
pub fn audit_document(document: &RouteDocument, configured_stock: Option<&HashSet<String>>, rules: Option<&[RetroRule]>, policy: AuditPolicy) -> AuditReport

// src/bridge/audit_route.rs
pub fn build_audit_route_report(content: &str, format: &str, stock: Option<&HashSet<String>>, rules: &[RetroRule], policy: AuditPolicy) -> anyhow::Result<AuditRouteReport>
// AuditManifest.policy becomes policy.as_str() instead of the hardcoded "standard"

// src/main.rs — new flag, same hand-rolled match-arm style as every
// other audit-route flag (`--format`, `--stock`, `--output`)
// --policy informational|standard|strict, default: standard (absent -> zero behavior change, same "additive, opt-in" shape as coverage mode's own --search-mode)

// src/wasm.rs — see §6 for the naming question
pub fn audit_route(content: &str, format: &str, stock_text: &str, policy: &str) -> String
```

## 8. Backward compatibility

`--policy` absent (CLI) / a 4th argument of `"standard"` or omitted
(WASM, if backward-compatible signature chosen) → byte-identical output
to today, since `Standard` reproduces the exact current `any_fail`/
`any_not_evaluable` derivation. This mirrors coverage mode's own
"standard is the default, nothing changes unless you opt in" contract
(`docs/design/coverage-mode-v0.md` §2) — consistent with this project's
established pattern for additive flags.

## 9. Test plan sketch (for the future implementation round, not run here)

- **Unit**: `derive_status` (or wherever the match ends up) tested
  table-driven against all `2 (gating) × 2 (not_evaluable) × 3 (policy)
  = 12` combinations — small, fast, exhaustive by construction.
- **`tests/audit_route_cli.rs`**: extend with `--policy` coverage against
  the fixtures already generated there (a gating-finding fixture and a
  not_evaluable-only fixture, each run under all 3 policies = 6 new
  assertions, reusing existing fixture-generation helpers).
- **`tests/cross_tool_audit.rs`**: parametrize existing fixtures across
  all 3 policies. The core invariant to assert directly, not just
  imply: **the `findings` array is byte-identical across all 3 policies
  for the same input** — only `status` differs. This is the single most
  important regression test for "policy never hides a finding."
- **Determinism**: extend `auditing_the_same_input_twice_is_byte_identical`
  to also hold per-policy (same input + same policy → byte-identical,
  already true structurally once policy is just another parameter, but
  worth a direct assertion).
- **Playground**: a policy `<select>` alongside the existing
  format/stock inputs; manual/Playwright check that switching it changes
  the displayed verdict badge without changing the findings list for the
  same already-audited input — matches §9's core invariant, made visible
  in the UI, not just in the JSON.
- Ordinary fixtures only, no heavy/formal-scale measurement — this is a
  correctness feature with a closed, enumerable state space (12 cases),
  not a search-quality change requiring statistical confirmation.

## 10. Explicitly out of scope for v0.29.0 per this design

- A first Python audit-route binding, unless §5's option (b) is chosen.
- WASM `_v2`-style versioning, unless §6 resolves toward it.
- Any new `AuditStatus`/`CheckStatus` vocabulary (a `notice` state, a 4th
  status value), unless §4 resolves toward the finer-grained model.
- Evidence Package (proposed v0.30.0) — unrelated, separate round.
- Any change to `bridge::audit`'s existing finding taxonomy, severity
  classification, or check logic — this design only adds a policy
  parameter to status *derivation*; it does not touch what gets detected
  or reported.

## 11. Sequencing / next step

This document is for review. §4, §5, and §6 are the three concrete
decisions needed before an implementation round can be scoped correctly;
everything else here (§3's aggregate model, §7's threading sketch, §8's
backward-compatibility contract, §9's test plan) is ready to execute
once those three are resolved. No code changes, version bump, or publish
happen as part of this document, per this round's explicit scope.
