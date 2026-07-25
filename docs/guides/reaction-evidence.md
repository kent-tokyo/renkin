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

## Reported vs. Predicted — Read This Before You Cite a Number

This is the single most important distinction on this page:

- **`reported_yields` is a citation, not a prediction.** It's exactly what
  the reference you supplied says was achieved, for that specific
  template/reaction — never a value RENKIN computed or estimated for your
  target molecule.
- **`step_confidence` / `success_probability` are unrelated to yield.** They
  are template-frequency-derived search-ranking scores (how common a
  disconnection was in the training corpus) — not a measured yield, not a
  predicted yield, and not a probability that your specific synthesis will
  succeed. Nothing in this system changes what those two fields mean.
- **`warnings` reflects only what's in the sidecar you supplied.** RENKIN
  does not run automatic side-reaction detection, does not search the
  literature for you, and does not infer a warning from structure alone. An
  empty `warnings` list means *no warning was curated for this template* —
  it does not mean *no side reaction is possible*.
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
