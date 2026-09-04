# SpectatorBondLoss Fail-Closed Gating — Design Doc

Status: **Design only, not yet implemented.** Requested directly after
PR #186 (Case A/B detectors, merged `b1ded8a`) and PR #187's lightweight
smoke measurement (19,606 findings / 276 rules from 15 targets). This is
the last stage of the v0.35.0 plan before release — the highest-risk one,
since it's the first stage that can actually change which candidates a
search sees, not just report on them.

## 0. What this is, in one paragraph

Today `SearchConfig::spectator_bond_diagnostics` (`src/search.rs:1356`)
only *records* findings (`CrowdOutDiagnostics::spectator_bond_loss_findings`)
— `raw_propose` (`src/candidate.rs:977`) computes `detect_case_a`/
`detect_case_b` findings for a `(target, rule)` pair completely
independently of the `RawCandidate` list it generates for that same rule.
Nothing is ever excluded. Fail-closed gating needs to reject the specific
*candidate* a confident finding applies to — which requires answering a
question the current code never asks: **which of a rule's (possibly
several) matches against this target produced which specific candidate?**
That correlation, done wrong, is exactly how "don't reduce existing
routes for unclear reasons" gets violated — reject the wrong sibling
candidate and a legitimate route silently disappears with no traceable
cause.

## 1. Existing-code grounding

- `detect_case_a`/`detect_case_b` (`src/spectator_bond.rs:165`, `:252`)
  each call `find_reaction_matches(&rule.smirks, &[target])` and loop
  `for m in &matches`, aggregating every match's findings into one flat
  `Vec<SpectatorBondLossFinding>` with no record of *which* match a
  finding came from. This is fine for diagnostics; not enough for gating.
- `raw_propose` (`src/candidate.rs:1004-1032`) generates `candidates` via
  `apply_retro_with_policy` and `sbl_findings` via the two detectors, in
  the same per-rule closure, but never cross-references them.
- **The join is provably exact for `apply_retro`'s fast path.** Reading
  chematic-rxn 0.16.0's `transform.rs` directly:
  `run_reactants_impl` (line 73) is `find_matches_impl` (the same
  function `find_reaction_matches` calls) followed by
  `matches.iter().filter_map(|m| apply_match_impl(...))` — i.e.
  `find_reaction_matches` + `apply_reaction_match` **is** `run_reactants`,
  not a parallel reimplementation that could drift. Calling
  `apply_reaction_match(rule.smirks, &[target], m, true)` on a match
  `detect_case_a`/`detect_case_b` already produced reproduces that
  match's exact product set, by construction — no empirical fuzz-test
  needed to trust this for the `!rule.smirks.contains('#')` path
  `apply_retro` (`src/chem_env.rs:1098-1112`) uses directly.
- **But `raw_propose` doesn't call `apply_retro` directly — it calls
  `apply_retro_with_policy`** (`src/ring_context.rs:712`), which for an
  extracted-template rule under `RingContextConfig::Guarded` runs its
  *own* match-level pipeline and can filter or reorder outcomes relative
  to plain `apply_retro`. So a match-index-based join (find_reaction_matches's
  Nth match ↔ raw_propose's Nth candidate) does **not** hold in general —
  only a content-based join does.
- **The content-based join already exists as a precedent**:
  `declared_forward_smirks` (`src/chem_env.rs:1218`) solves the identical
  "which of a rule's several outcomes produced this candidate" problem for
  graph-based rules, by comparing canonical, atom-map-cleared precursor
  SMILES sets. Gating reuses the same idea, simpler (no atom-map dance
  needed — production targets aren't atom-mapped): a match's finding
  attaches to a `RawCandidate` iff their sorted canonical precursor-SMILES
  multisets are equal.
- **`cc_single_cleavage` cannot be a gating test case** — checked directly,
  not assumed. Its rule (`"[C:1][C:2]>>[C:1].[C:2]"`, `chem_env.rs:2114`)
  has a 2-atom LHS and two single-atom RHS fragments. `detect_case_a` only
  ever has one map-number pair to check (1,2), and that pair is exactly
  the declared LHS bond, so it's always skipped. `detect_case_b` only
  ever has one cross-fragment pair to check, and it's always the direct
  cut bond (Case A's territory), so it always `continue`s before reaching
  `unmatched_only_path`. Both are empty *for every match on every target*,
  structurally — there's no third atom in the match for a spectator bond
  to hide behind. The multi-match mixed-outcome risk is real for any rule
  whose match spans **3+ atoms** (all four Finding #4 positive controls
  qualify); it needs a purpose-built fixture (§6), not this rule.

## 2. Scope boundary: gate only the `[#N]`-free fast path (v1)

`apply_retro` has three paths (`src/chem_env.rs:1082`): graph-based
(empty `smirks`, already out of scope — `detect_case_a`/`b` return early),
the fast path (no `#N]`, ~57% of the corpus), and the `[#N]` path, which
applies `application_smirks_variants`' *concrete-element* strings, not
the literal `rule.smirks` the detectors match against. The join proven in
§1 only holds for the literal `rule.smirks` string. Extending it to the
variant-expansion path is a real but separable problem (matching would
need to run per-variant and merge, same dedup-by-signature logic
`apply_retro` itself already uses).

**v1 decision: gating is only ever `Rejected` for rules with no `#` in
`rule.smirks`.** For `[#N]`-bearing rules, detection keeps running exactly
as it does today (diagnostics unaffected), but the gate always resolves
to `NotEvaluable` — never blocks a candidate it can't correlate with
proof. This is the conservative default the user's "確実なものだけ除外"
requirement calls for, not a hidden capability gap.

## 3. Confident vs. `not_evaluable`

A finding may reject its candidate only when **all** of the following
hold; otherwise the candidate's gate status is `NotEvaluable(reason)`,
never silently accepted or rejected:

1. `rule.smirks` contains no `#` (§2).
2. `declared_map_pairs` successfully parsed the LHS **and** every RHS
   fragment (`src/spectator_bond.rs:144` returns an empty set on any parse
   failure today, indistinguishable from "genuinely declares nothing" —
   this is a live false-positive path for gating specifically, even
   though it's harmless for diagnostics-only use. Fix: change
   `declared_map_pairs` to return `Option<HashSet<...>>`, `None` on parse
   failure, and treat `None` as `not_evaluable` rather than "empty.")
3. The finding's originating match, replayed via `apply_reaction_match` +
   the same fragment-splitting `apply_retro`'s fast path uses, resolves to
   **exactly one** `RawCandidate` by canonical precursor-SMILES-multiset
   equality (§1). Zero matches → the join failed unexpectedly (defensive
   fallback, should not happen given §1's proof, but never trust silently)
   → `not_evaluable`. More than one `RawCandidate` sharing that signature
   with **inconsistent** finding status among the matches that produced it
   → ambiguous → `not_evaluable` for all of them, not a coin flip.

Everything else — a confidently-identified, single-candidate match with a
lost bond — is `Rejected`.

## 4. Typed contract

```rust
pub enum SpectatorBondGateVerdict {
    Accepted,
    Rejected(Vec<SpectatorBondLossFinding>),
    NotEvaluable(&'static str), // e.g. "hash_atom_wildcard_rule", "unparseable_declared_bonds", "match_correlation_failed", "ambiguous_signature"
}
```

Attaches as a new field on `RawCandidate` (`src/candidate.rs:944`):
`pub spectator_bond_gate: SpectatorBondGateVerdict` (or an `Option<...>`
if `Accepted` should stay implicit — `Option::None` reads ambiguously
against `NotEvaluable`, so prefer the explicit three-way enum, always
populated once the policy is anything but `Off`). Serializes for CLI/JSON
consumers as the user's own contract shape for the `Rejected` case:

```json
{
  "status": "rejected",
  "reason": "spectator_bond_loss",
  "rule": "extracted_824",
  "lost_bonds": [
    {"source_atom_a": 4, "source_atom_b": 5, "bond_order": "single"}
  ]
}
```

Findings are recorded identically regardless of policy — policy decides
only whether `Rejected` actually removes the candidate from
`raw_propose`'s returned list. Mirrors `bridge::audit::AuditPolicy`'s
"policy changes the verdict, never the finding set" principle, adapted
from post-hoc route auditing to pre-route candidate filtering.

## 5. Policy mechanism

Replace the `bool` `SearchConfig::spectator_bond_diagnostics` with a
3-state enum before either ever ships in a release (both are
merged-to-master but unpublished — v0.34.0 is the latest published
version, confirmed via crates.io/PyPI/npm/docs.rs — so this is a free
rename, not a breaking change):

```rust
pub enum SpectatorBondPolicy {
    Off,             // default: zero cost, matches today's absent behavior
    DiagnosticsOnly, // today's shipped spectator_bond_diagnostics: true
    Gated,           // detect + record + reject confident findings
}
```

`raw_propose` gains this enum in place of the current bool. `Gated`
implies detection runs (no separate "on but not gating" combination to
reason about); `DiagnosticsOnly` behaves exactly as today, byte-for-byte
— existing tests (`spectator_bond_diagnostics_opt_in_runs_without_error_on_default_rules`,
the `raw_propose_spectator_bond_diagnostics_*` trio) get renamed to the
enum but assert the same behavior.

## 6. Where rejection happens, and its own diagnostic trail

In `raw_propose`'s per-rule closure (`src/candidate.rs:1004`), after
`candidates` and `sbl_findings` are both computed: for `Gated`, correlate
per §1/§3, drop `Rejected` candidates from the closure's `candidates`
before they're merged into the returned `Vec<RawCandidate>`, and push a
record for every dropped one into a new
`CrowdOutDiagnostics::spectator_bond_gated_out: Vec<GatedCandidateRecord>`
(rule name, template id, the finding(s), the dropped precursor SMILES) —
so an exclusion is never invisible even though it's no longer in the
route search. `NotEvaluable` candidates are never dropped, `Gated` or not.

## 7. Acceptance criteria

**Positive controls** (must reject under `Gated`): `extracted_824`,
`extracted_109`, `extracted_112`, `extracted_4255` (Finding #4's four),
plus `extracted_288` (this smoke test's hand-verified β-lactone case) —
five now, not four.

**Negative controls** (must never reject, must stay `Accepted`):
`co_aliphatic_cleavage`, `cc_single_cleavage` (§1 — structurally can't
even produce a finding, confirm the gate leaves it alone too), plus the
existing ~10 already-covered ordinary-disconnection fixtures in
`spectator_bond.rs`'s test module (intermolecular amide formation,
reductive amination, legitimate ring opening, Suzuki-style 2-fragment
cut, unrelated distant ring).

**New required fixture — mixed-outcome, same rule**: a target with two
separate instances of the same 3+-atom match pattern, one where the extra
bond is really present (must reject) and one where it isn't (must stay
`Accepted`), proving gating rejects only the specific defective candidate
and leaves the clean sibling from the same rule untouched. Candidate
construction: reuse `extracted_824`'s oxazolidinone LHS/RHS against a
target containing both a closed oxazolidinone ring (the known-defective
instance) and, on a separate part of the same molecule, an acyclic
5-atom chain matching the identical LHS pattern with no ring-closing bond
(the clean instance) — two matches, one rule, one target, opposite
verdicts.

**Not-evaluable coverage**: one fixture per §3 reason (`#N`-bearing rule,
an LHS/RHS fragment engineered to fail `mol_from_smiles`, and — if
reachable at all given §1's proof — a forced correlation failure).

## 8. Rust/CLI/Python/WASM parity

Untouched by any work so far — `spectator_bond_diagnostics` today is an
internal `SearchConfig` field only. Needed for v0.35.0:

- CLI: a flag mirroring how ring-context enforcement is already exposed
  (find and match that flag's exact naming pattern before adding a new
  one — don't invent a second convention).
- Python: a `SearchConfig`-equivalent field/enum in `src/python.rs`,
  matching how `spectator_bond_diagnostics` would have needed the same
  treatment.
- WASM: same field on the WASM-facing config struct; the enum must
  serialize deterministically (`#[serde(rename_all = "snake_case")]`,
  matching `SpectatorBondLossCase`'s existing convention).

None of this exists yet; scoping it now so v0.35.0's acceptance gate
doesn't quietly ship Rust-only.

## 9. Rollout stages

1. `declared_map_pairs` → `Option<HashSet<...>>` fix (§3.2) — small,
   independent, lands first.
2. Per-match correlation (§1/§3) as a new function, e.g.
   `correlate_candidate(target, rule, candidate, matches) -> SpectatorBondGateVerdict`,
   plus the mixed-outcome fixture (§7). Unit-tested in isolation before
   touching `raw_propose` at all.
3. Wire `SpectatorBondPolicy` into `SearchConfig`/`raw_propose`/
   `CrowdOutDiagnostics` (§5/§6). `Gated` still off by default.
4. CLI/Python/WASM parity (§8).
5. Re-run the lightweight smoke measurement (10-20 targets, same sample)
   under `Gated`, recording excluded-candidate counts and route-count
   deltas per the user's own staged plan — before any default changes or
   release.
6. v0.35.0 release, after separate explicit approval (no version bump/
   tag/publish before then, per standing session policy).

## Open questions for sign-off before implementation starts

- OK scoping gating to `#`-free rules only in v1 (§2) — ~43% of the
  corpus stays diagnostics-only, never gated, until a later round extends
  the join to `application_smirks_variants`?
- OK replacing `spectator_bond_diagnostics: bool` with the 3-state
  `SpectatorBondPolicy` enum (§5) — free now, not once anything ships?
- Does the mixed-outcome fixture in §7 look like the right acceptance
  bar, or is there a real corpus example (rather than a constructed one)
  worth using instead — e.g. does anything in the smoke test's 276
  flagged rules already exhibit this on a real target?
