# Reaction-Family Mislabel Regression Suite — Design Doc (ROADMAP Item 5, v1 slice)

Status: **Design only, not yet implemented.** Scopes the smallest real
first slice of `internal_docs/ROADMAP.md`'s "Disconnection vs.
named-reaction-family separation" item (P1). Full item's ask is a 3-layer
schema redesign (`disconnection` / `reaction_family_candidates` with
confidence / `conditions`); this doc found that two of those three layers
already exist in reasonable form, and scopes only the concretely missing
piece: a regression suite that catches the *defect class* PR #171 already
fixed one instance of, before it recurs on a different rule.

## 0. What this is, in one paragraph

`aryl_ether_retro` used to match any aromatic-C-O bond, including an
ester's own oxygen, mislabeling an ester cleavage as an Ullmann-ether
disconnection with fabricated Cs₂CO₃/DMF/110°C conditions (fixed, PR
#171). The underlying mechanism that produced this bug is still present
for **every other rule** in `reaction_family_for_rule` (`search.rs:885`):
a static, unconditional `rule_name -> single asserted family name`
lookup, applied every time that rule fires, with no per-match validation
that the *actual* substrate matches the *named* reaction's real scope.
This doc proposes a regression suite — real target fixtures per rule,
each asserting the correct family name and a list of specifically-banned
wrong ones — so a future over-broad SMIRKS pattern on `suzuki_retro`,
`wittig_retro`, etc. gets caught the same way `aryl_ether_retro`'s
eventually was, rather than shipping silently.

## 1. Existing-code grounding — the schema is more separated than the
## roadmap item's own wording suggested

- `ReactionStep` (`search.rs:113`) already has **distinct fields** for
  what the roadmap calls three layers: `rule`/`template_id` (disconnection
  identity — which bond broke, unambiguous, derived from the actual
  match), `conditions: Option<ReactionConditions>` (already gated: `None`
  for extracted templates, populated only for hand-crafted rules with a
  `metadata_source` provenance tag — "no specific conditions without real
  evidence" is already the shipped behavior, not a gap), and
  `reaction_family: Option<String>` (the named-reaction claim).
- **The actual gap is narrower than "build three layers from scratch":**
  `reaction_family` is a single asserted `String`, not a confidence-scored
  candidate list, and `reaction_family_for_rule`'s static match arms
  (`search.rs:885-918`) have no per-case validation that the SMIRKS match
  that fired actually stayed within the named reaction's real scope —
  exactly the shape of defect `aryl_ether_retro` had. Confirmed by
  reading the full 18-arm match table: every other graph-based/broad rule
  (`suzuki_retro`, `heck_retro`, `wittig_retro`, `sonogashira_retro`,
  `diaryl_sulfone_retro`, ...) has the identical structural risk —
  untested, not confirmed defective, but architecturally capable of the
  same mistake.
- **No existing regression suite pins `(target, rule) -> expected
  reaction_family` today.** `aryl_ether_retro`'s own fix (PR #171) added
  a positive-case test confirming the ester no longer matches, but there
  is no repo-wide fixture set asserting "this rule may claim family X,
  must never claim family Y."

## 2. Scope boundary: a regression suite, not a confidence-scoring engine (v1)

This doc does **not** propose `reaction_family_candidates: Vec<...>` with
genuine per-match confidence scoring (the roadmap's fuller ask) — that
requires new evidence-derivation logic (what would even feed a confidence
score today? no such signal exists), a materially larger and more
speculative undertaking than lifting already-tested logic the way the
other two ROADMAP-item design docs did. v1 ships exactly one thing: a
fixture-based regression suite pinning correct-family and
banned-mislabel assertions for the highest-risk existing rules, reusing
the exact pattern `aryl_ether_retro`'s own PR #171 fix already
established (a positive-case test on a real target).

## 3. Which rules first

Not all 18 arms are equally risky. Prioritize by the same shape that made
`aryl_ether_retro` dangerous: a **broad, topology-only SMIRKS match**
covering more chemistry than the named reaction implies (as opposed to a
narrow match with little room for a different real reaction to fire
through it). Candidates for the first fixture round, ranked by apparent
match breadth reading their SMIRKS directly in `chem_env.rs`:

1. `suzuki_retro` — aryl-aryl bond cleavage; risk of matching a
   biaryl formed by a different real coupling (Negishi-class or a
   pre-existing biaryl in a starting material, not a Suzuki product).
2. `heck_retro`/`heck_retro_terminal` — alkene-from-aryl-halide pattern;
   risk of matching a naturally-occurring styrene-like alkene with no
   real halide precursor history.
3. `diaryl_sulfone_retro` — labeled `friedel_crafts_sulfonylation`;
   same aryl-X-aryl shape class as `aryl_ether_retro` was.
4. `sonogashira_retro`/`wittig_retro` — lower apparent risk (more
   structurally distinctive patterns) but included for coverage breadth
   in the same pass, cheap given the fixture-authoring machinery is
   already being built for 1-3.

Not proposing a campaign to fixture-test all 18 arms in one round —
matches this project's own established discipline (see the v0.36.0
Phase 1 rule-safety census's own explicit scope cap) of starting with
the highest-risk subset, not exhaustively re-auditing everything at once.

## 4. Typed contract

No new production types — this is test-only. A small fixture table,
mirroring PR #171's own regression test shape:

```rust
// tests/reaction_family_mislabel_regression.rs (new file) or a module in
// the existing chem_env.rs test block, whichever this repo's own
// convention prefers for cross-rule regression fixtures -- check
// `chem_env.rs`'s existing test module structure before deciding, don't
// guess.
struct FamilyFixture {
    target: &'static str,
    expected_rule: &'static str,
    expected_family: Option<&'static str>,
    /// Family names this exact (target, rule) pair must NEVER produce --
    /// the aryl_ether_retro/esterification pair is the canonical entry
    /// once this rule's own equivalent fixture exists.
    banned_families: &'static [&'static str],
}
```

Each fixture calls `apply_retro` (or the existing test helper
`raw_propose`/whatever `aryl_ether_retro`'s own PR #171 test used —
reuse that exact call shape, don't re-derive) directly against a real
target, asserts the resulting step's `rule` matches `expected_rule`,
`reaction_family` matches `expected_family`, and — the actual new
assertion this doc adds — that none of `banned_families` ever appears
for that same `(target, rule)` pair.

## 5. Where this lives, and why no `SearchConfig`/policy needed

Unlike the other two ROADMAP Item design docs (Items 1 and 4), this is
**pure test code, not a runtime mechanism** — there's no `SearchConfig`
field, no CLI/Python/WASM surface, no `Off`/`DiagnosticsOnly`/`Active`
policy needed, because nothing here changes search behavior at runtime.
The "fix" this regression suite delivers is catching a *future* mislabel
at CI time (a fixture starts failing when someone widens a SMIRKS pattern
without checking what else it now matches), not gating anything live.

## 6. Acceptance criteria

- Each of the 3-5 prioritized rules (§3) gets at least one real-target
  positive-case fixture (confirms today's correct behavior, matching PR
  #171's own pattern) and, where a plausible over-broad-match scenario
  exists (checked by hand against the rule's own SMIRKS, not
  speculatively invented), a banned-mislabel fixture analogous to the
  original ester/`aryl_ether_retro` case.
- All fixtures pass against current `master` before this ships — if one
  doesn't, that's either a **newly confirmed real defect** (root-cause
  and fix it the same way PR #171 did, following this repo's own
  established fixture-first discipline) or a mistaken fixture assumption
  (fix the fixture, document why).
- `cargo test --workspace` stays green; no production code changes
  expected in this slice unless a fixture surfaces a real, confirmed
  defect (out of scope to assume one exists — this is a coverage
  addition, not a bug-fix PR, unless testing finds otherwise).

## 7. Rollout stages

1. Locate and confirm the exact existing test pattern/location
   `aryl_ether_retro`'s own PR #171 regression test uses (file, helper
   function, assertion style) — reuse it exactly, don't invent a second
   convention.
2. Write positive-case + banned-mislabel fixtures for `suzuki_retro`
   (§3's #1 priority) first, in isolation — confirms the fixture shape
   works and either passes cleanly or surfaces a real finding, before
   scaling to the other 2-4 rules.
3. Extend to the remaining prioritized rules (§3), one small commit per
   rule or a single batched PR if all pass cleanly on the first attempt
   (a single mixed PR is fine here since these are independent,
   read-only test additions with no shared state — a different
   situation from Items 1/4's hot-path changes, which needed careful
   staged isolation).
4. If any fixture surfaces a real, confirmed mislabel defect: root-cause
   and fix it as its own PR, following the `aryl_amine_retro`/
   `aryl_ether_retro`/rule-safety-census precedent (frozen fixture,
   direct `apply_retro` reproduction, fix or disable with a removal
   comment, CHANGELOG/rule-count cascade) — not bundled into the
   regression-suite PR itself.

## Open questions for sign-off before implementation starts

- OK prioritizing `suzuki_retro`/`heck_retro`(`_terminal`)/
  `diaryl_sulfone_retro` first (§3), deferring the other ~14 arms to a
  later round only if this first batch finds something, rather than a
  full 18-rule audit up front?
- Is a hand-picked "plausible over-broad-match scenario" (§3, §6)
  sufficient fixture-authoring rigor, or does this need the same kind of
  systematic SMIRKS-breadth static screen `examples/rule_safety_census.rs`
  already does for the atom-loss defect class — i.e., should this v1
  slice include a *second* static census tool (SMIRKS pattern breadth
  vs. named-reaction specificity) rather than manually-authored fixtures
  alone?
- Confirm scope: this doc deliberately does not build
  `reaction_family_candidates`/confidence scoring. Should that remain a
  separate, much-later, explicitly-approved-before-starting phase, or is
  the regression suite alone considered "sufficiently addressing" this
  ROADMAP item for now (matching how Item 1's element-accounting slice
  plus the already-shipped `SpectatorBondLoss` were judged to
  substantially cover that item without building the full taxonomy)?
