# Validator Accuracy Measurement — Design Doc (ROADMAP Item, P1)

Status: **Design only, not yet implemented.** Scopes
`internal_docs/ROADMAP.md`'s "Measure validator accuracy" item: *"no
fixed few-hundred/1000-step validation corpus exists (real AiZynthFinder
output + known USPTO reactions + RENKIN's own routes + deliberately-broken
mutations) to measure true/false accept/reject rates by reaction class.
Not started."* Follows `docs/design/spectator-bond-fail-closed-gating-v0.md`/
`docs/design/candidate-time-element-accounting-gate-v0.md`/
`docs/design/reaction-family-mislabel-regression-v0.md`'s structure and
rigor: map what already exists before proposing anything new.

## 0. What this is, in one paragraph

The roadmap item's own wording bundles four ingredients (AiZynthFinder
output, USPTO reactions, RENKIN's own routes, deliberately-broken
mutations) into one corpus, measuring "the validator" by reaction class.
This doc found that the four ingredients are not equally available (one
is fully ready, one exists but can't supply what the item wants from it,
one doesn't exist at all), and that **"the validator" is not one thing**
— two independent, deliberately-not-unified implementations exist. The
real, honest v1 slice is narrower than the item's wording implies: measure
one of the two validators' true-accept rate against already-existing real
ground truth (zero new data collection needed for that half), and its
true-reject rate against a small, hand-curated negative set (not a
generic mutation-injection engine, which is a materially bigger, separate
undertaking). Per-reaction-class breakdown is possible but only by
grouping on RENKIN's own asserted `reaction_family`, not an independent
corpus-native class label — an important caveat carried through the rest
of this doc.

## 1. Existing-code grounding

### 1.1 "The validator" is two independent implementations, by design

- **`src/validation/mod.rs::validate_step`** (`StepValidationStatus::{Valid,
  Invalid, NotEvaluable}`, rolled up per-route via `aggregate_route` into
  `RouteValidationStatus::{Validated, Invalid, PartiallyValidated,
  NotEvaluable}`). Strict and provenance-bound: it validates a step
  against *the rule it actually claims* (`step.rule`), looked up by name
  in the supplied rule set — SMIRKS-based rules go through
  `forward::rule_reproduces` (reverse the claimed rule's own SMIRKS only,
  no fallback to "does some other rule happen to connect these SMILES"),
  graph-based rules go through `graph_rules::validate_graph_step`. A rule
  name that isn't found (e.g. an extracted template RENKIN can't match
  back) is `NotEvaluable`, never silently `false` — this three-way split
  itself was a deliberate fix for an earlier bug where "chemically wrong"
  and "no validation method exists for this rule family" both collapsed
  to a bare `false` (see the module's own doc comment). Used by
  `renkin-bench`/`src/bin/benchmark.rs`.
- **`src/bridge/forward.rs::validate_step_forward`** (Bridge PR4):
  richer and reason-coded — `CheckStatus` (`src/bridge/audit.rs`,
  pass/fail/not_evaluable) plus `ForwardNotEvaluableReason` (6 variants:
  `MissingReactionRepresentation`, `MissingAtomMapping`,
  `UnsupportedReactionFormat`, `UnsupportedTemplateSyntax`,
  `ReactionApplicationError`, `AmbiguousExpectedProduct`) plus
  `EvidenceBasis` (3 variants: `DeclaredRuleTemplate`,
  `DerivedGraphRuleRoundtrip`, `SourceToolReaction`) tagging *which*
  evidentiary channel produced the verdict. Built for cross-tool routes
  (e.g. AiZynthFinder output) whose reaction metadata RENKIN didn't
  author itself, so it can't always assume a `step.rule` name that means
  anything in RENKIN's own rule table.
- **These are deliberately not unified.** `src/synthesizability/schema.rs:160`'s
  own comment: *"picking one of the two existing, behaviorally different
  forward-validation engines would couple the kernel to that engine's
  bugs."* Both feed the synthesizability kernel as an *external input*
  (`AssessmentContext::forward_validation: Option<&[StepValidationStatus]>`
  per route/step) — the kernel itself never recomputes a verdict, only
  rolls up whatever was supplied. **Consequence for this doc**: a
  validator-accuracy measurement has to pick which of the two it's
  measuring, or measure them separately and report separately — never
  average them into one "the validator" number, the same discipline the
  kernel itself already applies.

### 1.2 What ground truth already exists

- **`data/reranker_labels_uspto50k_test.jsonl`** (4903 rows): real,
  human/USPTO-derived ground truth. Each row —
  `{"group_id": "uspto50k_test#L3855", "target_id": "<SMILES>",
  "correct_precursor_sets": [[...]], "schema_version": 1}` — gives one or
  more known-*correct* precursor sets for a real target. Confirmed the
  `target_id` scheme (`uspto50k_test#L<n>`) is the exact same one
  `data/comparison/sample_full_sorted.jsonl` already uses (its own
  `target_id` field, e.g. `uspto50k_test#L3855`), so the already-existing
  100-target sample used by the beam-diversity/ring-context formal gates
  is directly joinable to this label file with no new data pull.
- **`data/uspto50k_raw_{train,val,test}_split.jsonl`** (5007 test rows):
  real, atom-mapped USPTO reactions (`{"class": "UNK", "id": "US...",
  "product": "<atom-mapped SMILES>", "reactants": "<atom-mapped SMILES>"}`).
  Confirmed **every row's `class` field is the literal string `"UNK"`**
  — this is real reaction data, but it carries no usable reaction-class
  label. **Any "by reaction class" grouping cannot come from this
  corpus's own labels.**
- **`data/comparison/results_100/` and `results_500/`**: real RENKIN vs.
  AiZynthFinder paired route output, already side-by-side
  (`renkin_native.jsonl`, `aizynthfinder_native.jsonl`,
  `*_aggregate.json`, `per_target_audit.md`). This is the "real
  AiZynthFinder output" ingredient the roadmap item names — it exists,
  but note it's *route*-level output (full precursor trees), not
  individually validator-scored steps; using it would mean re-deriving
  per-step verdicts from someone else's route metadata, which is exactly
  the `validate_step_forward`/cross-tool case in §1.1, not the
  `validate_step` case.
- **No deliberately-broken mutation corpus exists anywhere in the
  codebase.** Confirmed by search for mutation/corrupt/deliberately-broken
  patterns: the only precedent is a handful of hand-authored, single-target
  negative unit tests — `chem_env.rs`'s
  `aryl_ether_retro_skips_aryl_ester_oxygen`/
  `aryl_ether_retro_skips_aspirin_ester_oxygen` (the PR #171 ester-mislabel
  fix's own regression tests) and `search.rs`'s
  `route_integrity_tests::isoindolinone_ring_disconnection_is_rejected_not_returned`.
  Each is a real target with a real, confirmed-wrong disconnection — not
  output from a generic corruption/mutation engine. **This is the one
  ingredient in the roadmap item's own list that is genuinely missing,
  not just differently-shaped than expected.**
- **One documented historical validator false-positive**, `chem_env.rs:2088-2099`
  (the `aryl_chloride_retro`/`aryl_iodide_retro`/`aryl_fluoride_snAr_retro`
  removal comment, issue 31.11): *"confirmed 100% (F, I) and 73%+ (Cl; the
  remainder was a validator false-positive, tracked separately as 31.12)
  Invalid+imbalanced on sampled USPTO-50k targets."* No further resolution
  of 31.12 was found anywhere in the repo (code, `CHANGELOG.md`,
  `docs/design/*.md`) — it appears to be the only concretely-measured
  (if narrow, single-rule-family) prior validator-accuracy data point that
  exists today.

### 1.3 Reaction-class tagging on RENKIN's own output side

`ReactionStep` (`search.rs:113`) carries `rule`/`template_id`
(disconnection identity) and `reaction_family: Option<String>`
(`search.rs:150`, populated via `reaction_family_for_rule`,
`search.rs:885`, now directly test-covered as of PR #216 — 19 of 22
active rules mapped, 3 legitimately generic and `None`). This is the only
usable class-like label anywhere in this measurement's reach — and it's
RENKIN's own assertion about its own output, not an independent
third-party ground-truth label (see §1.2's `"class": "UNK"` finding).
**Any "true/false accept/reject by reaction class" breakdown this v1
slice produces is really "broken down by what RENKIN itself claims the
reaction is," not validated against an independent class taxonomy.** State
this caveat next to any such breakdown in the eventual report, the same
way `findings.md`'s existing measurements always caveat what a number
does and doesn't prove.

### 1.4 Harness/schema conventions to reuse

Confirmed concrete, reusable precedent for both shapes this measurement
would need:
- **Rate objects**: `data/comparison/results_500/renkin_conservative_native/aggregate.json`'s
  `{"denominator_kind": "all_sampled", "n_denominator": 500,
  "n_numerator": 12, "value": 0.024}` shape (and the sibling
  `{denominator_kind, n, p50, p95, max}` shape for distributions) —
  reuse directly, don't invent a new rate-reporting shape.
- **Manifest provenance**: `data/comparison/shared_stock/shared_stock_manifest.json`'s
  `source_file`/`source_file_sha256`, `excluded_count`/`excluded[]`,
  `duplicate_count`/`duplicates[]`, output-artifact-path + `_sha256`
  fields, numeric (never bare-boolean) round-trip counts.

## 2. The scoping gap the roadmap item's wording glosses over

`validate_step(step: &ReactionStep, rules: &[RetroRule])` requires a
**claimed rule name** (`step.rule`) — it looks that name up in `rules`
and checks the step against *that specific rule's* own SMIRKS or
graph-check. `reranker_labels_uspto50k_test.jsonl`'s
`correct_precursor_sets` gives only **target + correct precursor SMILES**,
with no rule attribution attached. So "run the ground truth through
`validate_step`" is not a zero-cost, call-the-function-directly operation
— connecting the two needs one more step. Two ways to close that gap,
with materially different cost and materially different meaning:

- **(a) Real search, then check.** Run RENKIN's actual route search on
  the labeled targets, and for any route found, check whether its
  precursors match a known-correct set *and* whether `validate_step`
  correctly marks that route's own already-attributed step `Valid`. This
  measures search-plus-attribution-plus-validation jointly — informative,
  but conflates three systems, and costs real search compute (like the
  beam-diversity formal gate's per-target subprocess runs) for something
  that doesn't have to be that expensive.
- **(b) Attribution-free validator probe, no search needed.** For each
  labeled `(target, correct_precursor_set)` pair, construct a candidate
  `ReactionStep` under *every* active rule name in turn (22 candidates)
  and call `validate_step` directly — a pure in-process function call,
  no subprocess, no search, no timeout budget. If **any** rule attribution
  makes `validate_step` return `Valid`, the validator successfully
  recognizes this genuine real disconnection as valid under *some*
  correct labeling; if **none** do, the validator would reject a real,
  correct synthesis step no matter how it's attributed — a genuine
  validator recall gap, isolated from search or attribution correctness.
  This is cheap (bounded to `4903 targets × 22 rules` pure function
  calls, no external process, likely seconds not hours) and measures the
  validator specifically, which is what this roadmap item actually asks
  for. **Recommended for v1** — (a) is a different, larger measurement
  (closer to "how good is RENKIN's search+attribution", already
  partially covered by the existing AiZynthFinder comparison harness)
  and should stay a distinct, separately-scoped effort if wanted later.

## 3. Scope boundary: v1 slice

**In scope:**
- Measure `validate_step`/`RouteValidationStatus` only (§1.1's first
  implementation). `validate_step_forward` (the bridge/cross-tool
  validator) is structurally different (reason-coded,
  evidence-basis-tagged, built for routes RENKIN didn't author) and
  deserves its own separate measurement design, not a shared number —
  consistent with `schema.rs`'s own "don't collapse these two" principle.
- **True-accept rate**: §2(b)'s attribution-free probe against
  `data/reranker_labels_uspto50k_test.jsonl`'s real correct-precursor-set
  labels. Zero new data collection.
- **True-reject rate**: a small, hand-curated negative corpus, reusing
  the already-existing confirmed-wrong cases (§1.2's ester-mislabel and
  isoindolinone cases) plus a small number of new, similarly
  hand-constructed cases (real target, a plausible-but-wrong rule
  attribution, manually verified wrong by the same reasoning PR #171's
  own fix used) — explicitly **not** a generic mutation-injection engine.
- Per-reaction-class breakdown by RENKIN's own `reaction_family`/`rule`,
  with §1.3's self-referential-labeling caveat stated in the report.

**Explicitly deferred, not v1:**
- A systematic, generic mutation/corruption-injection engine for broad
  negative-corpus coverage (atom deletion, valence violation, wrong
  stereo, etc.) — a materially bigger, separate undertaking; §2(b)'s
  attribution-free probe plus a small hand-curated negative set covers a
  real, useful first slice without it.
- Measuring `validate_step_forward` / the bridge validator.
- §2(a)'s real-search-based joint measurement (search+attribution+validator).
- Resolving the historical 31.12 false-positive tracked-but-unresolved
  item — noted as a real prior data point (§1.2), not re-investigated here.
- Scaling past the existing labeled corpus's own size (4903 targets is
  already the ceiling for the true-accept side without new data
  collection; no reason to draw a smaller sample for this specific
  measurement given it needs no per-target search compute).

## 4. Typed contract

No new production types needed for v1 — this is a measurement harness
(Python, matching `scripts/beam_diversity_formal_gate.py`/
`scripts/compare_shared_stock.py`'s own convention for this kind of
one-off formal measurement) plus, if the attribution-free probe needs a
capability not already exposed, a minimal Rust CLI surface exposing
`validate_step` for a given `(rule_name, target_smiles,
precursor_smiles[])` triple (check `src/main.rs`/`renkin-mcp` first for
whether something already exposes this before adding a new subcommand —
not confirmed either way in this pass).

Per-target/per-rule row shape (JSONL), following the existing
per-target-row convention (`data/comparison/results_500/*.jsonl`):

```json
{
  "target_id": "uspto50k_test#L3855",
  "rule_attribution_tried": "ester_cleavage",
  "validate_step_result": "valid",
  "matches_any_correct_set": true
}
```

Summary rate objects reuse `{denominator_kind, n_denominator,
n_numerator, value}` exactly as `results_500/aggregate.json` already
does (§1.4) — e.g. `true_accept_rate` with
`denominator_kind: "labeled_targets_with_any_valid_attribution"`.

## 5. Rollout stages (proposed)

1. Confirm coverage overlap between `reranker_labels_uspto50k_test.jsonl`'s
   4903 `target_id`s and `sample_full_sorted.jsonl`'s existing 100-target
   sample (should be near-total, same source split) — decide whether v1
   runs on the existing 100 (consistency with other harnesses) or the
   full 4903 (free to do, since this needs no search compute) — open
   question below.
2. Build the §2(b) attribution-free probe as a small script/tool, dry-run
   on a handful of targets, confirm it produces sane
   valid/invalid/not_evaluable distributions before running on the full
   chosen sample.
3. Curate the small hand-authored negative corpus (start from the
   already-existing 3 confirmed-wrong cases, add a handful more only
   where a *real*, hand-verified wrong attribution exists — not invented
   speculatively, matching this project's own established "confirmed
   defect, not pattern-matching" discipline).
4. Run both measurements, report true-accept/true-reject rates overall
   and by `reaction_family`, with §1.3's self-referential-labeling
   caveat stated explicitly next to the by-class breakdown.
5. Write findings, update `internal_docs/ROADMAP.md`. No production code
   changes expected in this slice unless the probe surfaces a real,
   confirmed validator defect (same "fix only on confirmed defect"
   discipline as every other measurement in this project) — out of scope
   to assume one exists.

## 6. Acceptance criteria

- True-accept rate measured against real, already-existing ground truth,
  zero new corpus collection for that half.
- True-reject rate measured against a small, hand-curated (not
  speculative, not auto-generated) negative corpus.
- Both reported for `validate_step` only, explicitly not blended with
  `validate_step_forward`.
- Per-reaction-class breakdown reported with the self-referential-labeling
  caveat stated in the same report, not left implicit.
- `cargo test --workspace` / `cargo fmt --check` / `cargo clippy
  --workspace --all-targets -- -D warnings` stay green if any Rust
  surface is touched (§4's possible minimal CLI addition).

## Open questions for sign-off before implementation starts

- **Answered 2026-08-29**: yes, §2(b)'s attribution-free probe is v1's
  true-accept method. §2(a)'s real-search-based joint measurement stays
  deferred to a separate later effort, not part of v0.37.0.
- **Answered 2026-08-29**: yes, defer a systematic mutation-generation
  engine. v1's true-reject side uses a small hand-curated negative
  corpus (existing confirmed-wrong cases plus new hand-authored examples,
  target size ~30-50 total, not a generic engine).
- **Answered 2026-08-29**: full 4,903-target label file, not the smaller
  100-target sample — the near-zero marginal search cost (this measures
  the validator directly via pure function calls, not real route search)
  makes the larger, free sample the clear choice here, unlike every other
  formal-gate measurement in this project that does pay real search cost
  per additional target.
- **Answered 2026-08-29**: yes, state the self-referential-labeling
  caveat prominently in the report, not left implicit.

This is v0.37.0 "Verified Candidate Integrity" scope (see
`internal_docs/ROADMAP.md`'s "Recommended roadmap") — not started until
v0.36.0's stock pilot is done and reviewed.
