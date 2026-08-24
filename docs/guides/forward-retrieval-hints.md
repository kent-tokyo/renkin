---
title: "Partner-Free Forward Retrieval Hints in RENKIN"
description: "How renkin-forward hints extracts search/retrieval clues (matched template slots, missing-partner SMARTS, bond deltas) from a single known reactant, without inventing partner molecules or predicting a concrete product."
---

# Forward Retrieval Hints

`renkin-forward hints` answers a third question, distinct from both
[`predict`](forward-prediction.md) and [`enumerate`](forward-enumeration.md):
*"Given only this one molecule, what should I search a patent/reaction
database for?"* This is an **information-assisted retrieval** tool, not a
product predictor. It never invents a partner molecule and never guesses a
concrete product — it reports, for every compatible template, exactly which
part of the known molecule reacts, the exact SMARTS pattern the still-missing
partner would need to satisfy, and the bond-level change the template
represents, so a chemist can go search a real database (an internal ELN, a
patent corpus, Reaxys, SciFinder, ...) with concrete, mechanically-derived
search terms instead of guessing.

## `predict` / `enumerate` / `hints` at a glance

| | `predict` | `enumerate` | `hints` |
|---|---|---|---|
| Input | every reactant a template needs | one known reactant | one or more known reactants |
| Partner molecules | caller supplies all of them | filled from an explicit `--partners` library | **never invented or searched — no partner input exists** |
| Output | concrete product SMILES | concrete product SMILES | a query pattern for the product (`product_query_smarts`), never a concrete SMILES |
| Use case | "what does this exact reaction give?" | "what can this molecule become, given a candidate partner library?" | "what should I search for to find a real partner/reaction?" |
| Report schema | `ForwardPredictionReport` | `ForwardEnumerationReport` | `ForwardRetrievalHintReport` — separately versioned, the other two are untouched |

Putting a partner-free retrieval mode into `enumerate` itself was
deliberately rejected: `enumerate` without `--partners` already has a
well-defined, narrower meaning (skip binary templates, apply unary ones
directly), and mixing "a concrete product" and "an abstract query pattern"
into the same report schema would blur a distinction this guide's whole
purpose depends on keeping sharp.

## Installation and build

Same binary as `predict`/`enumerate`, no separate build step:

```bash
cargo build --release -p renkin-forward
```

## Basic usage

```bash
renkin-forward hints --reactants "Brc1ccccc1" [--templates <path>] \
  [--max-hints N] [--max-matches-per-slot N] [--max-assignments-per-template N]
```

- `--reactants <SMILES>...` — one or more known reactants (required).
- `--templates <path>` — an additional SMIRKS template file, on top of the
  embedded default rules (hard error if missing/unreadable/empty, same as
  `predict`/`enumerate`).
- `--max-hints N` (default 50) — cap on merged hints returned, applied
  **after** cross-template merging (see [Merge semantics](#merge-semantics)).
- `--max-matches-per-slot N` (default 20) — cap on reported match sites per
  (template, slot); a capped slot sets `match_sites_truncated: true` on
  that assignment rather than silently reporting a partial list as complete.
- `--max-assignments-per-template N` (default 200) — cap on the injective
  known-reactant/slot permutations enumerated per template; hitting this
  cap increments `stats.templates_with_assignments_truncated` and is never
  silently treated as "no assignment found".

## One known reactant

```bash
renkin-forward hints --reactants "Brc1ccccc1" --max-hints 1
```

Real captured output (embedded default rules, `renkin-forward` release
binary):

```json
{
  "schema_version": 1,
  "known_reactants": [
    { "input_index": 0, "canonical_smiles": "c1cc(Br)ccc1" }
  ],
  "hints": [
    {
      "hint_id": "sha256:04780ac95230798f61c42fd8910c1bad74e82dde7c1d0c48f40429b53e2febe0",
      "rank": 0,
      "reaction_family": { "label": "C-C bond formation", "basis": "derived_bond_delta" },
      "known_assignments": [
        {
          "input_index": 0,
          "slot_index": 0,
          "match_sites": [
            { "target_atom_indices": [1], "mapped_atoms": [{ "template_map": 1, "target_atom_index": 1 }] }
          ],
          "match_sites_truncated": false
        }
      ],
      "missing_partners": [
        {
          "slot_index": 1,
          "query_smarts": "[C:2](=O)O",
          "required_features": {
            "required_elements": ["C", "O"],
            "excluded_elements": [],
            "aromatic": false,
            "hydrogen_constraints": [],
            "summary_complete": true
          }
        }
      ],
      "transformation": {
        "bonds_formed": [{ "left_map": 1, "right_map": 2, "order": "any" }],
        "bonds_broken": [],
        "bonds_order_changed": []
      },
      "product_query_smarts": ["[c:1][C:2](=O)[OH]"],
      "search_terms": ["C partner", "C-C bond formation", "O partner", "aryl carboxylation retro"],
      "proposal_score": 1.0,
      "sources": [
        { "template_id": "rule:aryl_carboxylation_retro", "rule_name": "aryl_carboxylation_retro", "template_weight": 1.0 }
      ]
    }
  ],
  "stats": { "hints_returned": 1, "hints_capped": true }
}
```

(One `match_site`/`mapped_atoms` entry shown above for brevity; a bromobenzene
molecule with its symmetric ring positions typically produces several.)

With `--max-hints 5` on the same reactant, the same run also surfaces
aryl carboxylation, Friedel-Crafts acylation, Sonogashira, Heck, and aryl
chloride-to-bromide retro-templates — each with its own
`missing_partners[].query_smarts` and `search_terms` — a realistic spread
of "what to search for" starting from one aryl bromide, none of it
invented. (Verified against the current 22-rule `default_rules()` set,
2026-08-24; the exact top-5 shifts if the rule set changes, since
ranking depends on which templates are loaded.)

## Multiple known reactants

Supply more than one `--reactants` value when you know several starting
materials and want to see which templates connect them directly (both slots
satisfied, zero missing partners) versus which still need an external
partner:

```bash
renkin-forward hints --reactants "Brc1ccccc1" "NCC"
```

For a template where both known reactants match distinct slots,
`known_assignments` has two entries (`input_index` 0 and 1, one per slot)
and `missing_partners` is empty. `hints` finds every injective assignment
of known reactants to distinct slots — an assignment never places the same
reactant on two slots, and two rows of the *same* SMILES supplied as two
separate `--reactants` arguments are still two distinct inputs, each
eligible for its own slot.

## Exact vs. heuristic fields

`query_smarts` (on each missing partner) and `product_query_smarts` are
always the literal SMARTS text `hints` parsed from the reversed
template — authoritative, never summarized or approximated.

`required_features`, `reaction_family`, and `search_terms` are best-effort,
auxiliary summaries derived *from* that authoritative SMARTS — read them as
"probably useful search hints," not as a guaranteed-complete restatement of
the constraint. Two safeguards keep this honest:

- **`summary_complete`**: `false` whenever the SMARTS contains a recursive
  `$(...)` sub-pattern, a `NOT` over anything more complex than a single
  element, or an `OR` whose branches disagree on some property (e.g.
  `[c,C]` — aromatic vs. aliphatic carbon: `aromatic` is left `null` rather
  than arbitrarily picking one branch's value). An `OR` across values of
  the *same* property (`[N,O]`, `[N;H1,H2]`) is still fully summarized —
  `required_elements: ["N", "O"]` / `hydrogen_constraints: ["H1 or H2"]`
  already mean "any of these." It is *also* `false` whenever a missing
  partner's query spans more than one atom or has any bond at all: every
  field above is a flat union/merge across the whole slot, so it cannot
  represent which atom carries which constraint or how the atoms connect
  — `[N:1][O:2]` (two bonded atoms) and `[N,O:1]` (one N-or-O atom) would
  otherwise look identical whenever their per-field values happen to
  agree. `query_smarts` is unaffected either way and remains authoritative.
- **`reaction_family.basis`**: `"derived_bond_delta"` (computed purely from
  which mapped bonds appear/disappear between the reactant and product
  sides), `"rule_name"` (a readable fallback derived from the template's
  own name), or `"ambiguous_across_sources"` (`label` = `"mixed"`) — never
  an inferred named reaction like "Buchwald-Hartwig" unless the template's
  own metadata said so (RENKIN's `RetroRule` doesn't currently carry that
  kind of curated metadata at all). `"ambiguous_across_sources"` shows up
  when two differently-named templates merge into the same hint (same
  retrieval signature) but disagree on a `"rule_name"`-basis label; rather
  than silently keeping whichever template happened to be processed first,
  both are represented as unresolved. `search_terms` are the union of
  every merged source's own search terms (not just the first source's),
  derived from `reaction_family`/`missing_partners` plus each source's
  `rule_name`, and are explicitly auxiliary — don't search a patent
  database on `search_terms` alone without also checking `query_smarts`.
- **`transformation.bonds_*[].order`**: `"directional_unspecified"` for an
  `up`/`down` (`/`/`\`, E/Z-style) bond whose orientation could not be
  trusted — internally, bonds are keyed by atom-map number sorted
  low-to-high, which can swap which atom was originally "first" for a
  directional bond; rather than guess, or worse, report a direction that
  may be backwards, this is surfaced explicitly rather than silently
  emitting a possibly-wrong `"up"`/`"down"`.

## Match sites and attachment points

Each `known_assignments[].match_sites` entry is one structural embedding of
the matched template slot into the known reactant: `target_atom_indices` is
every atom index chematic assigned in that embedding, and `mapped_atoms`
gives the subset carrying a template atom-map number (`:N`) — the actual
reaction-center/attachment atoms, cross-referenced against
`transformation.bonds_formed`/`bonds_broken` by that same map number. A
molecule with several chemically-equivalent reactive sites (e.g. 1,4-
dibromobenzene against an aryl-bromide template) reports one `match_sites`
entry per site, capped by `--max-matches-per-slot` with
`match_sites_truncated` set when the cap was hit.

## Bond delta

`transformation.bonds_formed`/`bonds_broken`/`bonds_order_changed` are
computed by comparing the reactant-side and product-side mapped bonds by
atom-map number alone — never by calling `chematic::rxn::run_reactants`.
Each entry's `order` is one of `single`/`double`/`triple`/`aromatic`/
`any`/`ring`/`up`/`down`, or `complex` for a compound bond query this
describer doesn't attempt to flatten (never silently narrowed to a single
misleading type).

## `product_query_smarts` is not a concrete product

`product_query_smarts` is a list of query patterns (usually one, plural
because a template's product side can itself have multiple disconnected
components) — a pattern any real product of this transformation would need
to match, not a specific molecule `hints` claims will form. Never treat it
as `predict`'s or `enumerate`'s concrete `products` field.

## Merge semantics

Two templates converge into one hint when their retrieval signature
matches: the same known-reactant slot roles, the same missing-partner
`query_smarts` set, the same bond-delta signature, and the same
`product_query_smarts` set — independent of the templates' own names or
IDs. A merged hint's `sources` lists every contributing template; nothing
is dropped. `--max-hints` truncates the **post-merge** hint list, so two
templates merging into one hint under `--max-hints 1` is never reported as
capped.

## Caps, truncation, and "unknown" vs. "no match"

Every cap (`--max-matches-per-slot`, `--max-assignments-per-template`,
`--max-hints`) is reported explicitly when hit — `match_sites_truncated`
per assignment, `stats.templates_with_assignments_truncated` per report,
`stats.hints_capped` for the final list. A capped result still returns
whatever was found before the cap; it is never silently reported as "no
match," which would misrepresent a truncated search as a confirmed
negative.

## Malformed templates and graph-based rules

A retro SMIRKS that fails to reverse or fails per-component SMARTS parsing
is counted in `stats.template_parse_failed` and the template is simply
skipped — never a hard error for the whole run. A rule with an empty
`smirks` field (a graph-based/hard-coded transformation such as Boc
deprotection, which has no retro-SMIRKS to reverse and analyze statically)
is counted separately in `stats.graph_rules_skipped`, the same convention
`predict`/`enumerate` already use — never miscounted as a parse failure.

`hints` validates every template component with
`chematic::smarts::parse_smarts` (the correct SMARTS grammar), not
`predict`/`enumerate`'s `parse_reaction`-based check (which parses with
`parse_smiles` and rejects legitimate multi-condition SMARTS like
`[N;H1,H2:2]`) — so `hints` accepts a strict superset of what
`predict`/`enumerate` can run concretely. Verified against the real
extracted-template corpus: `predict`/`enumerate` accept 283/500 templates,
`hints` accepts all 500.

## No implicit partner corpus, no product invention

`hints` never reads RENKIN's embedded 402-compound retro stock and never
searches, generates, or guesses a partner molecule — there is no
`--partners` flag at all for this subcommand, by design. It also never
predicts reaction conditions, catalysts, yields, or a calibrated
reaction-feasibility probability; `proposal_score` is a ranking signal only
(the maximum contributing template's frequency-derived weight), matching
`predict`/`enumerate`'s own `proposal_score` semantics.

## Performance

See `cargo run --release -p renkin-forward --example hints_benchmark` for a
reproducible timing + stats report across the embedded 28 default rules,
the first 100 extracted templates, and the full 500-template extracted
corpus, at small/medium/large reactant sizes. Measured on this development
machine: from under 1ms (28 rules, a single small aromatic ring) up to
roughly 570ms (the full 500-template corpus, a moderately complex
multi-ring molecule) — well within interactive/offline-batch use for this
tool's retrieval use case. No partner-side prefilter or caching was added
in this round; the existing per-call caps already bound worst-case cost,
and no measurement crossed into blocking-latency territory that would
justify the added complexity.

## Non-goals

- No conditions, catalyst, solvent, temperature, or yield prediction.
- No claim that a hint corresponds to a real, literature-verified reaction
  — `hints` reports what a *template* would require, not that a specific
  partner satisfying it is known to exist or to react successfully.
- No partner generation, search, or ranking — that remains entirely the
  user's own database/search step.
- No arity ≥3 concrete application (same as `enumerate`) — but unlike
  `enumerate`, `hints` *does* report the structural shape of an arity-3
  template (which known-reactant slots match, which remain missing), since
  that's still useful retrieval information even though no concrete
  product can ever be generated for it.

## Rust API

```rust
use renkin_forward::hints::{generate_retrieval_hints, HintGenerationConfig};
use renkin::chem_env::default_rules;

let report = generate_retrieval_hints(
    &["Brc1ccccc1"],
    &default_rules(),
    &HintGenerationConfig::default(),
)?;
for hint in &report.hints {
    println!("{}: {:?}", hint.reaction_family.label, hint.search_terms);
}
```
