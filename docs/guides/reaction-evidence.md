---
title: "Reaction Conditions, Reported Yields, and DOI/Patent Evidence in RENKIN"
description: "How to attach curated reaction conditions, reported yields, literature/patent references, and known side-reaction warnings to specific retrosynthesis templates in RENKIN."
---

# Reaction Evidence Metadata

A retrosynthesis route tells you *what bond to break*. It doesn't tell you
*what conditions were actually used*, *what yield was reported*, *which paper
or patent this comes from*, or *what can go wrong*. RENKIN's evidence
metadata system exists to attach exactly that — as curated, cited data you
supply, never as something RENKIN invents.

## The Problem: Templates Don't Have Stable Identity by Default

An extracted SMIRKS template's only name is its position in a file
(`extracted_0`, `extracted_1`, ...). Re-sort the file, re-run the extraction
pipeline, or add a template in the middle, and every name after that point
shifts. You can't attach a DOI to "`extracted_412`" if `extracted_412` might
be a completely different reaction next week.

RENKIN gives every template a **stable `template_id`** instead:

- Hand-crafted rules: `rule:<rule_name>` (e.g. `rule:suzuki_retro`) — the name
  is a literal source-code identifier, so it's already stable.
- Extracted templates: `smirks-sha256:<hex>` — the SHA-256 hex digest of the
  *trimmed* SMIRKS string itself. Independent of file position, load order, or
  count. Purely syntactic: no SMIRKS canonicalization is performed, so an
  equivalent reaction written with different atom-map numbering gets a
  different ID.

Run `renkin template ids <file.smi>` to list every template's `template_id`,
current display name, SMIRKS, and weight — this is how you look up the ID you
need to reference in a sidecar file.

## Attaching Evidence

`--template-metadata sidecar.json` (CLI) or `template_metadata_path=...`
(Python `find_routes`) loads a JSON file keyed by `template_id`:

```json
{
  "schema_version": 1,
  "templates": {
    "smirks-sha256:ef8778a2888469d619c52cce7e74f6848e101049050dd1b765b78f32e3c94498": {
      "references": [
        { "id": "ref-1", "kind": "doi", "identifier": "10.xxxx/example" }
      ],
      "condition_candidates": [
        {
          "catalysts": ["Pd(PPh3)4"],
          "bases": ["K2CO3"],
          "solvents": ["EtOH", "water"],
          "temperature_c": { "min": 75.0, "max": 85.0 },
          "source": "literature",
          "scope": "template",
          "reference_ids": ["ref-1"]
        }
      ],
      "reported_yields": [
        {
          "percentage": { "min": 72.0, "max": 81.0 },
          "basis": "isolated",
          "source": "literature",
          "scope": "template",
          "reference_ids": ["ref-1"]
        }
      ],
      "warnings": [
        {
          "code": "possible_protodeboronation",
          "severity": "medium",
          "message": "Protodeboronation has been reported under prolonged aqueous heating.",
          "source": "literature",
          "scope": "template",
          "reference_ids": ["ref-1"]
        }
      ]
    }
  }
}
```

A step whose template matches this `template_id` gets an `evidence` field
with `condition_candidates`, `reported_yields`, `references`, and `warnings`.
A step whose template has no entry simply has no `evidence` key at all.

`kind` accepts `doi`, `patent`, `url`, or `dataset_record`. `basis` (for a
reported yield) accepts `isolated`, `assay`, `conversion`, or `unknown` — the
same word chemists already use to qualify a literature yield.

## Validation

The sidecar is loaded and validated **before search starts** — a malformed
file is a hard error, not a silent partial load:

- `schema_version` must be in the supported range.
- No duplicate `template_id` keys, no duplicate or dangling `reference_ids`.
- Yield percentages must be in `[0, 100]`; any `min`/`max` range must have
  `min <= max`.
- DOI/patent `identifier` must not be empty (a `url`/`dataset_record`
  identifier may be, since not every one is meaningfully citable).
- A `template_id` present in the sidecar but absent from the loaded rule set
  prints a warning (not a failure) — it's not silently ignored, but it also
  doesn't block a search over a different template set.

## Substrate-Specific Examples (`schema_version: 2`)

Everything above is *template-level* evidence: it applies to every step that
uses that template, regardless of the actual molecule. `schema_version: 2`
adds `examples` — a per-template array where each entry is one curated record
of *this exact reaction*, not the template in general:

```json
{
  "schema_version": 2,
  "templates": {
    "smirks-sha256:ef8778a2888469d619c52cce7e74f6848e101049050dd1b765b78f32e3c94498": {
      "references": [
        { "id": "ref-1", "kind": "doi", "identifier": "10.xxxx/example" }
      ],
      "examples": [
        {
          "id": "ex-1",
          "target_smiles": "c1ccc(-c2ccccc2)cc1",
          "precursor_smiles": ["Brc1ccccc1", "c1ccccc1"],
          "conditions": {
            "catalysts": ["Pd(PPh3)4"],
            "solvents": ["EtOH"],
            "source": "literature",
            "scope": "substrate_specific",
            "reference_ids": ["ref-1"]
          },
          "reported_yield": {
            "percentage": 78.0,
            "basis": "isolated",
            "source": "literature",
            "scope": "substrate_specific",
            "reference_ids": ["ref-1"]
          },
          "reference_ids": ["ref-1"]
        }
      ]
    }
  }
}
```

Key rules:

- **`examples` requires `schema_version: 2`.** A `schema_version: 1` sidecar
  with an `examples` key is a hard error, not a silent no-op — v1 sidecars
  without `examples` load and behave exactly as before.
- **`schema_version: 2` requires reported yields under `examples[].reported_yield`,
  not the template-level `reported_yields` list.** A non-empty template-level
  `reported_yields` under `schema_version: 2` is a hard error — otherwise a
  substrate-specific number could be placed at the template level and get
  applied to every step using that template, defeating the point of
  substrate-specific evidence. (Template-level `condition_candidates`,
  `warnings`, and `references` remain allowed under `schema_version: 2` — only
  `reported_yields` moves to the example level.) `schema_version: 1` keeps
  allowing template-level `reported_yields`, unchanged, for backward
  compatibility.
- **Every nested `conditions`/`reported_yield`/`warnings` entry inside an
  example must be scoped `"substrate_specific"`.** This is enforced at load
  time (any other scope there is rejected), unlike template-level entries,
  which aren't scope-restricted.
- `target_smiles`/`precursor_smiles` must parse and `precursor_smiles` must
  be non-empty; `id` must be non-empty and unique within its template;
  `reference_ids` (the example's own, and each nested `conditions`/
  `reported_yield`/`warnings` entry's own) must each point at a reference
  declared in that template's `references` list.

**Matching an example to a route step.** This happens once per step, not just
in `--format explain` — a step's `evidence.examples` are *resolved*, not
merely copied from the sidecar. Each example is compared against that step's
actual `target`/`precursors` by canonical SMILES (target must match exactly;
the precursor set must match after canonicalizing, sorting, and
deduplicating both sides, so `precursor_smiles` order in the sidecar never
matters). Every exact-substrate match is kept; same-template-different-substrate
precedents are capped at 3. Each resolved entry in JSON carries a `match_kind`
(`"exact_substrate"` or `"template_only"`) right alongside the example's own
fields, and `evidence.template_examples_total` reports how many examples the
template declared in total — so a JSON/Python consumer can tell "evidence for
this exact reaction" from "literature precedent for a different substrate"
without re-implementing the canonical-SMILES comparison itself, and can tell
how many precedents were truncated. `evidence.references` is trimmed to only
the ids actually cited by what's kept (template-level entries plus the
retained examples), not the template's full reference list.

**In `--format explain`:** each step shows every exact-substrate example plus
up to 3 template-only ones, exact matches first, each labeled either `Exact
substrate example:` or `Template-level literature example (different
substrate; not a prediction):`. Any examples beyond what's shown are
summarized as `... and N more template examples` rather than silently
dropped. Under each example, `conditions`/`reported_yield`/`warnings` each
show their *own* cited references directly beneath them (not just the
example's top-level `reference_ids`) — a reference cited in more than one
place for the same example is shown once, not repeated.

## Reported vs. Predicted — Read This Before You Cite a Number

This is the single most important distinction on this page:

- **`reported_yields` is a citation, not a prediction.** It's exactly what
  the reference you supplied says was achieved, for that specific
  template/reaction — never a value RENKIN computed or estimated for your
  target molecule. The same is true of an example's `reported_yield`, even
  when the example is an *exact* substrate match: it's what was reported for
  that reaction, not a RENKIN forecast for your route.
- **A template-only example (different substrate) is reference context, not
  a forecast.** It shows the template has literature precedent, not that
  your specific target/precursors will behave the same way — that's exactly
  why `--format explain` always labels it "not a prediction".
- **`step_confidence` / `success_probability` are unrelated to yield.** They
  are template-frequency-derived search-ranking scores (how common a
  disconnection was in the training corpus) — not a measured yield, not a
  predicted yield, and not a probability that your specific synthesis will
  succeed. Nothing in this system changes what those two fields mean.
- **`warnings` reflects only what's in the sidecar you supplied.** RENKIN
  does not run automatic side-reaction detection, does not search the
  literature for you, and does not infer a warning from structure alone. An
  empty `warnings` list — whether at the template level or inside a specific
  example — means *no warning was curated* for that template or substrate.
  It does not mean *no side reaction is possible*.
- **Templates without a sidecar entry get nothing fabricated.** No made-up
  conditions, no invented yield, no synthesized-sounding warning.

## Building a Sidecar File

1. Run `renkin template ids data/templates_extracted_5000.smi --format json`
   to get every template's stable `template_id`.
2. For the templates you actually have literature/patent evidence for, add an
   entry keyed by that `template_id`.
3. Pass the file via `--template-metadata` (CLI) or `template_metadata_path`
   (Python) alongside your normal search.

There's no requirement to cover every template — the sidecar is additive, and
uncovered templates behave exactly as they did before you supplied one.

## Next Steps

- [Python retrosynthesis guide](python-retrosynthesis.md) / [Rust retrosynthesis guide](rust-retrosynthesis.md)
- [Python API](../api/python.md) / [Rust API](../api/rust.md) — full `template_metadata_path`/sidecar-loading signatures
