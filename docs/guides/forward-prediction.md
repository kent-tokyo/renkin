---
title: "Standalone Template-Based Forward Reaction Prediction in RENKIN"
description: "How renkin-forward predict enumerates and ranks forward reaction product candidates by applying reversed retrosynthetic SMIRKS templates, independent of route search."
---

# Forward Reaction Prediction

`renkin-forward` ships two independent subcommands: `predict` (standalone
forward prediction, given reactants) and `validate` (forward-verifies a
retrosynthetic route's steps). Most of RENKIN's documentation talks about
`validate`, since that's how `predict` is used internally during route
search — but `predict` is a complete, standalone capability on its own, and
this guide documents it as such.

## What RENKIN supports

RENKIN's forward mode is a **template-based forward reaction candidate
generator**: given reactant SMILES, it takes every reversible SMIRKS-backed
retrosynthetic template RENKIN knows about, reverses each one from its retro
direction (`product >> precursors`) to a forward direction
(`precursors >> product`), and forward-applies it to the reactants via
[`chematic::rxn::run_reactants`](https://docs.rs/chematic/). Candidates are
ranked by a simple, transparent signal — each contributing template's
training-frequency weight — never by a learned scoring model.

This is **not** a general learned forward-reaction predictor. It is not a
Molecular Transformer equivalent, it does not compute a calibrated reaction
probability, and it does not predict side products, yields, or reaction
conditions. See [Limitations](#limitations) for the full list.

## Installation and build

`renkin-forward` is a workspace member, built alongside the rest of RENKIN:

```bash
cargo build --release -p renkin-forward
```

The binary is at `target/release/renkin-forward`.

## Standalone prediction

```bash
renkin-forward predict --reactants "<SMILES>" "<SMILES>"... [--templates <path>] [--max-results N] [--report]
```

`--reactants` takes one or more reactant SMILES. Without `--report`, the
output is the legacy array of `{template, products, weight}` records; with
`--report`, the output is a full [`ForwardPredictionReport`](#detailed-report-output).

### Verified example: single, clean product

Salicylic acid (`Oc1ccccc1C(=O)O`) and ethanol (`CCO`), forward-applied
through `aryl_ether_retro` (a real, committed default rule), Williamson-ether
combine the phenolic `-OH` with the alcohol to give 2-ethoxybenzoic acid.
This was run twice against the release binary and the two runs were
byte-identical:

```bash
renkin-forward predict --reactants "Oc1ccccc1C(=O)O" "CCO" --report --max-results 5
```

```json
{
  "schema_version": 1,
  "reactants": [
    { "input_smiles": "Oc1ccccc1C(=O)O", "canonical_smiles": "c1cccc(c1O)C(O)=O", "input_index": 0 },
    { "input_smiles": "CCO", "canonical_smiles": "OCC", "input_index": 1 }
  ],
  "candidates": [
    {
      "candidate_id": "sha256:8322ea275031eb4aba247d469d105bc69c6204be6819792aac664dbba38af0c8",
      "products": ["OC(c1c(cccc1)OCC)=O"],
      "rank": 0,
      "proposal_score": 1.0,
      "sources": [
        { "template_id": "rule:aryl_ether_retro", "rule_name": "aryl_ether_retro", "template_weight": 1.0, "source_rank": 6 }
      ]
    }
  ],
  "stats": {
    "rules_loaded": 28, "smirks_rules": 21, "graph_rules_skipped": 7,
    "templates_attempted": 21, "templates_matched": 1, "template_application_errors": 3,
    "raw_outcomes": 1, "accepted_outcomes_before_merge": 1,
    "invalid_outcomes_rejected": 0, "no_op_outcomes_rejected": 0,
    "duplicate_candidates_merged": 0, "candidates_before_limit": 1,
    "candidates_returned": 1, "truncated": false
  },
  "warnings": [
    { "code": "template_application_failed", "template_id": "rule:aryl_chloride_to_bromide", "rule_name": "aryl_chloride_to_bromide", "message": "template \"rule:aryl_chloride_to_bromide\": run_reactants failed: ReactantCountMismatch { expected: 1, got: 2 }" },
    { "code": "template_application_failed", "template_id": "rule:alcohol_oxidation_retro", "rule_name": "alcohol_oxidation_retro", "message": "..." },
    { "code": "template_application_failed", "template_id": "rule:acyl_chloride_from_acid", "rule_name": "acyl_chloride_from_acid", "message": "..." }
  ]
}
```

The three warnings are expected and harmless: those three templates only
accept a single reactant, and two reactants were supplied. Non-strict mode
(the default) reports this and moves on instead of aborting the whole call.

This example is pinned as `validate_route_golden_fixture_verified_true` in
`crates/renkin-forward/src/lib.rs`'s test suite, so it can never silently
regress.

## Detailed report output

`--report` emits a `ForwardPredictionReport` (`FORWARD_REPORT_SCHEMA_VERSION
= 1`): canonicalized `reactants`, merged `candidates` (see
[Ranking and duplicate merging](#ranking-and-duplicate-merging)), a
`stats` object, and a `warnings` array. `stats` accounts for every outcome
independently at the pipeline stage it describes:

```
raw_outcomes == accepted_outcomes_before_merge + invalid_outcomes_rejected + no_op_outcomes_rejected
accepted_outcomes_before_merge - duplicate_candidates_merged == candidates_before_limit
```

Both are asserted directly in the test suite (`stats_accounting_invariants_hold`).

## Route verification

```bash
renkin-forward validate --route-json '{"steps":[...]}' [--templates <path>] [--max-results N]
```

Accepts a bare route object (`{"steps":[...]}`) or a full `find_routes`
output (`{"routes":[{"steps":[...]}]}`); omit `--route-json` to pipe JSON via
stdin instead — this is how `renkin ... --format json | renkin-forward
validate` works.

For each step, `verified` is `true` if the step's target's canonical SMILES
appears among **any** candidate's products — computed over the full,
untruncated candidate set, never limited by `--max-results` or the
`top_predictions` display cap, so an arbitrary display limit can never hide
a real match. `top_predictions` remains the same capped, legacy-shaped list
as before this change.

## How template inversion works

Every SMIRKS-backed retro rule is written `product_pattern >> precursor_pattern`.
Forward prediction reverses it — `precursor_pattern >> product_pattern` —
and validates the result before ever applying it:

1. Exactly one `>>` must be present.
2. Neither side may be empty after trimming.
3. The result is parsed with chematic's own reaction parser
   (`chematic::rxn::parse_reaction`) as a final syntactic check.

Graph-based rules (an empty `smirks` field — RENKIN has several, e.g. ester
and amide cleavage, which cut bonds directly in the target's molecular graph
rather than matching a SMIRKS pattern) have no forward direction to reverse
and are skipped, counted in `stats.graph_rules_skipped`, not treated as an
error.

This is a **syntactic** reversal, not a chemical reversibility guarantee —
see [Limitations](#limitations).

## Ranking and duplicate merging

`run_reactants` can return several independent outcomes for one template —
it may match the reactants in more than one way. Each outcome is treated as
one candidate; **outcomes are never flattened together**, so a two-product
outcome's products always stay paired with each other, never mixed with a
different outcome's products.

When two or more outcomes (from the same template or different templates)
canonicalize to the exact same product **multiset** (not set — `["CO",
"CO"]` and `["CO"]` are different candidates), they are merged into one
`ForwardCandidate`, retaining every contributing template as a `sources`
entry.

Ranking is fully deterministic:

- Candidates: `proposal_score` descending, then source count descending,
  then product multiset (lexicographic), then `candidate_id` (lexicographic).
- Sources within a candidate: `template_weight` descending, then
  `template_id`, then `rule_name`.

`proposal_score` is the maximum contributing source's template weight — **a
ranking signal only, not a calibrated probability**. Non-finite (NaN/±inf)
template weights are excluded from consideration entirely (reported as an
`invalid_template_weight` warning, or a hard error under
`strict_template_errors`) rather than ever falling through to an arbitrary
tie.

## Error and warning handling

By default (`strict_template_errors: false`), a single template's failure —
a malformed forward SMIRKS, a `run_reactants` application error, a
non-finite weight — is recorded as a `ForwardWarning` and processing
continues with the remaining rules. Setting `strict_template_errors: true`
(or, at the CLI, there is currently no flag for this — it is a
library-level config option) makes the very first such failure a hard
error instead.

An explicitly-supplied `--templates <path>` is validated strictly
regardless of this setting: a missing file, an unreadable file, or a file
containing zero valid templates is **always** a hard error, never a
silently-empty corpus.

Warning codes you may see: `invalid_forward_smirks`, `template_application_failed`,
`invalid_template_weight`, `empty_product_outcome`, `product_roundtrip_failed`,
`atom_balance_diagnostic` (informational only — see below, never rejects a
candidate).

## Limitations

- **SMIRKS-backed rules only.** Graph-based rules have no forward direction.
- **Template-based, not learned** — there is no neural forward-reaction model here.
- **Coverage is bounded by the loaded templates.** A real reaction whose
  transformation isn't expressed by any loaded template will not appear.
- **No reagents/conditions model.** Reagents, catalysts, and reaction
  conditions are not represented or required.
- **No yield prediction.**
- **No calibrated reaction-success probabilities** — `proposal_score` is a
  ranking signal derived from template training-frequency, nothing more.
- **No automatic side-product prediction.**
- **A reversed retro template may be chemically over-broad in the forward
  direction** — it was written to describe a disconnection, not validated
  as a general forward reaction rule. Not every retro template becomes a
  valid forward predictor just because its SMIRKS reverses cleanly.
- **Template frequency is a ranking score only**, not a probability of
  reaction success.
- **Stereochemistry** is only as well-preserved as the underlying template
  and chematic's reaction engine support it — no additional stereo model is
  applied.
- **Input reactants must structurally match a template's reactant pattern**;
  RENKIN does not suggest alternative reactants or protecting groups.

## Rust API

```rust
use renkin::chem_env::default_rules;
use renkin_forward::{predict_products, predict_products_detailed, ForwardPredictConfig};

let rules = default_rules();

// Legacy, backward-compatible API (unchanged signature):
let predictions = predict_products(&["CC(=O)O", "CCO"], &rules, 5)?;

// Recommended detailed API:
let report = predict_products_detailed(
    &["CC(=O)O", "CCO"],
    &rules,
    &ForwardPredictConfig::default(),
)?;
```

### Demonstrating outcome separation and no-op filtering

RENKIN's committed default rules all happen to have a single-fragment
product side, so every real default-rule outcome has exactly one product —
none of them can demonstrate a multi-product outcome on their own. The
mechanism itself is demonstrated with a hand-authored, synthetic SMIRKS rule
(not part of `default_rules()`) — this exact fixture is
`outcomes_are_never_flattened_together` in
`crates/renkin-forward/src/lib.rs`'s test suite, run on every commit:

```rust
use renkin::chem_env::RetroRule;
use renkin_forward::predict_products;

// A synthetic halide-metathesis rule, not shipped with RENKIN.
let rule = RetroRule {
    name: "synthetic_halide_metathesis".to_string(),
    template_id: "rule:synthetic_halide_metathesis".to_string(),
    smirks: "[C:1][Br:4].[C:3][Cl:2]>>[C:1][Cl:2].[C:3][Br:4]".to_string(),
    weight: 1.0,
    required_elements: 0,
};

let result = predict_products(&["ClCC(Cl)CBr", "BrCC(Br)CCl"], &[rule], 10)?;
// -> 3 candidates, not 4: chematic's run_reactants returns 4 raw outcomes
//    for this reactant pair, but one of them reassigns each molecule's
//    halogens back to its own starting arrangement (a genuine no-op) and
//    is correctly filtered. The other 3 outcomes' product pairs are all
//    distinct, even though individual products repeat across pairs --
//    exactly the information a naive flat_map would destroy.
```

## Reproducibility

Given the same reactants, the same rule set (same file contents, same
load order), and the same RENKIN version, `predict_products_detailed`'s
output is fully deterministic: candidate merge keys off content (sorted
canonical reactants + sorted canonical product multiset, SHA-256-hashed),
candidate and source ordering never falls back to an arbitrary tie, and no
`HashMap` is used on the candidate-construction path. Both examples in this
guide were run twice against the release binary and produced byte-identical
output.
