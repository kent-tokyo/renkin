---
title: "Single-Reactant Forward Enumeration in RENKIN"
description: "How renkin-forward enumerate discovers forward products from one known reactant using unary templates and an explicit partner library, distinct from predict."
---

# Forward Enumeration

`renkin-forward enumerate` answers a different question from
[`predict`](forward-prediction.md): *"I only know one reactant — what can
it become?"* `predict` requires the caller to supply the complete reactant
set a template needs; it cannot discover a missing co-reactant. `enumerate`
is a bounded, **template-guided enumeration** foundation for exactly that
gap — it is not a generative predictor. It does not invent partner
molecules, reaction conditions, or yields.

This guide documents Phase 1 of `enumerate`: unary templates applied
directly, and binary (two-reactant) templates with **at most one** missing
partner, filled from an explicitly supplied SMILES library.

## Installation and build

Same binary as `predict`/`validate`, no separate build step:

```bash
cargo build --release -p renkin-forward
```

## Difference from `predict`

| | `predict` | `enumerate` |
|---|---|---|
| Input | every reactant a template needs | exactly one known reactant |
| Missing partner | not supported (caller must supply all reactants) | searched from an explicit `--partners` file |
| Reactant assignment | tries every ordering of the caller's own reactants | tries the known reactant in each compatible template slot |
| Multi-reactant templates | any arity, as long as the caller supplies that many reactants | unary applied directly; binary with exactly one missing slot; arity ≥3 always reported unsupported |
| Report schema | `ForwardPredictionReport` (`FORWARD_REPORT_SCHEMA_VERSION`) | `ForwardEnumerationReport` (`FORWARD_ENUMERATION_REPORT_SCHEMA_VERSION`) — separately versioned, `predict`'s schema is untouched |

## Unary enumeration

A unary forward template (its forward-direction SMIRKS has exactly one
reactant component) applies directly to the known reactant — no partner
needed:

```bash
renkin-forward enumerate --reactant "Brc1ccccc1"
```

Every binary template in the loaded rule set is skipped in this mode
(counted in `stats.templates_binary_skipped_no_partners`, with a paired
`binary_template_skipped_no_partners` warning per skipped template) — this
is not an error, it's an explicit, valid discovery mode for callers who
only want to know what the known reactant alone can become.

## Explicit partner enumeration

For binary templates, supply a partner library:

```bash
renkin-forward enumerate \
  --reactant "<SMILES>" \
  --partners <partners.smi> \
  [--templates <path>] \
  [--max-results N] \
  [--max-partners-per-template N] \
  [--max-combinations N]
```

For each binary template, `enumerate` tries the known reactant in **each**
compatible left-hand-side slot and searches every partner-file record for
the other slot, forward-applying every valid `(template, slot, partner)`
combination via `chematic::rxn::run_reactants` — the same engine
`predict` uses, no custom matching logic. RENKIN's own 402-compound retro
stock is **never** used as an implicit partner source; `--partners` is the
only input this mode ever reads from.

### Partner file format

Same line convention as `data/building_blocks.smi`: `#`-prefixed and blank
lines are skipped, the first whitespace-delimited token on a line is the
SMILES, and an optional second token is retained as a human-readable
`label`.

```text
# comment lines and blank lines are skipped
CCBr ethyl_bromide
CCCBr
```

Unlike `renkin::chem_env::ChemEnv::load` (used for the retro-search
building-block stock), this loader **never deduplicates**: two lines with
the same SMILES are two distinct partner records, each with its own
**row identity** — the 1-based physical line number in the file
(`row_index`), independent of the optional `label`. Duplicate SMILES rows
are both retained and both appear in candidate provenance if they both
contribute.

### Verified example

Run against the current release binary (`ethyl_bromide`/`CCCBr` as
partners, `CCCCCl` as the known reactant):

```bash
printf "CCBr ethyl_bromide\nCCCBr\n" > partners.smi
renkin-forward enumerate --reactant "CCCCCl" --partners partners.smi
```

`stats` from the actual output:

```json
{
  "templates_binary_supported": 18, "templates_binary_skipped_no_partners": 0,
  "templates_unsupported_arity": 0, "slot_assignments_with_accepted_outcome": 2,
  "partners_scanned": 2, "partners_matched": 2,
  "partner_records_skipped_malformed": 0,
  "combinations_attempted": 75, "raw_outcomes": 40,
  "accepted_outcomes_before_merge": 40, "no_op_outcomes_rejected": 0,
  "duplicate_candidates_merged": 20,
  "candidates_before_limit": 20, "candidates_returned": 5,
  "results_capped": true, "truncated": true
}
```

`candidates_before_limit == 20` with the default `--max-results 5` cap
means only the top 5 of 20 distinct candidates are returned
(`results_capped`/`truncated: true`) — run with `--max-results 20` to see
the full set. The top-ranked candidate:

```json
{
  "candidate_id": "sha256:73890e5d76d436dc23e4c7ec71614e991521fe9f5e50d89529a506cd6717ed09",
  "products": ["BrC(CC)C(Cl)CCC"],
  "rank": 0,
  "proposal_score": 1.0,
  "sources": [
    {
      "template_id": "rule:cc_single_cleavage", "rule_name": "cc_single_cleavage",
      "template_weight": 1.0, "source_rank": 12, "slot_index": 0,
      "partner": { "row_index": 2, "label": null, "canonical_smiles": "C(CBr)C" }
    },
    {
      "template_id": "rule:cc_single_cleavage", "rule_name": "cc_single_cleavage",
      "template_weight": 1.0, "source_rank": 12, "slot_index": 1,
      "partner": { "row_index": 2, "label": null, "canonical_smiles": "C(CBr)C" }
    }
  ]
}
```

This candidate has two sources because `cc_single_cleavage` (a symmetric
C-C bond disconnection) matches the same partner in either slot position,
converging on the same product multiset — both slot assignments are
retained as distinct provenance entries, not collapsed or double-counted.

### Malformed partner-line policy

A malformed SMILES line is never a hard error by itself (only a missing
file, an unreadable file, or a file with zero valid records is — matching
`--templates`' own strictness). Each malformed line is counted in
`stats.partner_records_skipped_malformed` (the true, unbounded total) and,
up to 20 lines, recorded in `partner_load_warnings` with its row index,
input token, and parser error message:

```bash
printf "CCBr ethyl_bromide\nnot(a smiles\nCCCBr\n" > partners.smi
renkin-forward enumerate --reactant "CCCCCl" --partners partners.smi
```

```json
{
  "stats": {
    "partner_records_skipped_malformed": 1,
    "partner_diagnostics_returned": 1,
    "partner_diagnostics_truncated": false
  },
  "partner_load_warnings": [
    {
      "row_index": 2,
      "code": "invalid_partner_smiles",
      "input": "not(a",
      "message": "Failed to parse SMILES: not(a"
    }
  ]
}
```

If more than 20 lines are malformed, `partner_load_warnings` retains only
the first 20 (in file order) and `partner_diagnostics_truncated` becomes
`true` — `partner_records_skipped_malformed` always reports the true total
regardless.

## Report schema (`ForwardEnumerationReport`, schema v1)

- `known_reactant`: the single input reactant, canonicalized (`input_index`
  is always `0`).
- `candidates`: merged, ranked results — see
  [Candidate identity and source merging](#candidate-identity-and-source-merging).
- `stats`: structured pipeline accounting (see below).
- `warnings`: generic diagnostics (`ForwardWarning`, same shape `predict`
  uses — `binary_template_skipped_no_partners`, `template_arity_unsupported`,
  `invalid_forward_smirks`, `combination_application_failed`, etc.).
- `partner_load_warnings`: bounded per-line partner-file diagnostics (see
  above) — a distinct, more specific shape from `warnings`.

This is a **wholly separate schema** from `ForwardPredictionReport` —
bumping `FORWARD_ENUMERATION_REPORT_SCHEMA_VERSION` never implies anything
about `FORWARD_REPORT_SCHEMA_VERSION`, and vice versa.

## Candidate identity and source merging

Candidate identity hashes the known reactant's canonical SMILES and the
sorted canonical product multiset — **deliberately excluding the partner**,
so different partners (or the same template applied at different slots)
converging on the same products merge into one candidate. Provenance is
never lost: every contributing `(template, slot, partner)` combination is
retained as a distinct entry in that candidate's `sources` list, keyed on
`(template_id, rule_name, slot_index, partner.row_index)` — not just
`(template_id, rule_name)` as `predict` uses, since two different partners
reaching the same product via the same template must stay distinguishable.

Ranking is deterministic, same tie-break structure as `predict`:
`proposal_score` descending, source count descending, product multiset
lexicographic, `candidate_id` lexicographic. `proposal_score` is the
maximum contributing source's template weight — **a ranking signal only,
never a calibrated probability**.

## Spectator-slot detection

A left-hand-side template slot whose atom-map numbers share zero overlap
with any right-hand-side (product) atom-map number can never contribute an
atom to any outcome, for any molecule bound to it — this is a static fact
about the template, decided from its parsed structure alone (via
`chematic::rxn::parse_reaction`), not from a real reaction attempt. When
the known reactant would be bound to such a slot, that (template, slot)
assignment is skipped **before** `run_reactants` is ever called, counted
in `stats.spectator_slot_skips` (separate from `stats.raw_outcomes`).

## Arity ≥3 is not supported

Templates requiring two or more missing partners are always counted in
`stats.templates_unsupported_arity` and reported via a
`template_arity_unsupported` warning — **never silently skipped**. Phase 1
enumeration is bounded to at most one missing partner; there is no
`--partners`-omission fallback for arity ≥3.

## Limits and truncation

Three independent, deterministic limits, each paired with its own
`stats.*_capped` flag; `stats.truncated` is the logical OR of all three:

- `--max-partners-per-template` (default 50): caps how many partner
  records are tried per `(template, slot)` pair. `stats.partners_per_template_capped`.
- `--max-combinations` (default 2000): a global cap on total
  `(template, slot, partner)` combinations attempted across the whole run,
  independent of the per-template cap. `stats.combinations_capped`.
- `--max-results` (default 5): caps the final merged, ranked candidate
  list — applied **after** merging, never before (so a low `--max-results`
  never hides that two partners converged on one candidate).
  `stats.results_capped`.

## Non-goals

- No reaction conditions, catalyst recommendation, or yield prediction.
- No calibrated reaction-success probability — `proposal_score` is a
  ranking signal only, exactly as in `predict`.
- No side-product prediction.
- No implicit use of RENKIN's embedded 402-compound retro-search stock as a
  partner source — `--partners` is the only partner input.
- No integration into retrosynthetic route cost or A* search.
- No partner-side pre-filter/pre-screen in this phase: every attempted
  combination calls `run_reactants` directly, bounded by the two `--max-*`
  flags above (see [Performance](#performance)).

## Performance

There is currently no structural pre-filter that rules out an obviously
non-matching partner before calling `run_reactants` — every attempted
`(template, slot, partner)` combination pays the full application cost,
bounded only by `--max-partners-per-template`/`--max-combinations`. This
was a deliberate foundation-phase choice: a hand-built pre-filter risks
being *stricter* than the real matcher and silently dropping real
products for an unmeasured performance gain. For large partner libraries,
set `--max-partners-per-template`/`--max-combinations` deliberately rather
than relying on defaults, and expect runtime to scale roughly linearly
with partner-file size per contributing binary-template slot.

## Reproducibility

Given the same known reactant, the same partner file contents, the same
rule set, and the same RENKIN version, `enumerate_products_detailed`'s
output is fully deterministic — candidate merge keys off content (SHA-256
of the known reactant's canonical SMILES plus the sorted canonical product
multiset), source and candidate ordering never fall back to an arbitrary
tie, and no `HashMap` is used on the candidate-construction path. Both
examples in this guide were run twice against the release binary and
produced byte-identical output.

## Rust API

```rust
use renkin::chem_env::default_rules;
use renkin_forward::{enumerate_products_detailed, load_partners_strict, ForwardEnumerationConfig};

let rules = default_rules();
let partners = load_partners_strict("partners.smi")?;

let report = enumerate_products_detailed(
    "CCCCCl",
    Some(&partners.records),
    &rules,
    &ForwardEnumerationConfig::default(),
)?;
```
