---
title: "Reaction Evidence Metadata in RENKIN"
description: "Attach cited conditions, reported yields, and warnings to stable retrosynthesis-template identifiers without turning evidence into a prediction."
---

# Reaction evidence metadata

Evidence sidecars attach curated, cited information to a route step. They do
not make conditions, yield, or reaction success predictions.

```bash
renkin template ids data/templates_extracted_5000.smi --format json
renkin --target "CC(=O)Oc1ccccc1C(=O)O" \
  --templates data/templates_extracted_5000.smi \
  --template-metadata evidence.json
```

Hand-crafted rules use `rule:<name>` identifiers. Extracted templates use
`smirks-sha256:<hex>`, derived from the trimmed SMIRKS text, so reordering a
template file does not move its evidence to another reaction.

## Sidecar shape

Use `schema_version: 1` for template-level evidence:

```json
{
  "schema_version": 1,
  "templates": {
    "smirks-sha256:<hex>": {
      "references": [{"id": "ref-1", "kind": "doi", "identifier": "10.xxxx/example"}],
      "condition_candidates": [{
        "catalysts": ["Pd(PPh3)4"], "bases": ["K2CO3"],
        "solvents": ["EtOH", "water"], "source": "literature",
        "scope": "template", "reference_ids": ["ref-1"]
      }],
      "reported_yields": [{
        "percentage": {"min": 72.0, "max": 81.0}, "basis": "isolated",
        "source": "literature", "scope": "template", "reference_ids": ["ref-1"]
      }],
      "warnings": [{
        "code": "possible_protodeboronation", "severity": "medium",
        "message": "Reported under prolonged aqueous heating.",
        "source": "literature", "scope": "template", "reference_ids": ["ref-1"]
      }]
    }
  }
}
```

`kind` is `doi`, `patent`, `url`, or `dataset_record`; yield `basis` is
`isolated`, `assay`, `conversion`, or `unknown`. A sidecar entry that does not
match a loaded template is warned about, not silently applied elsewhere.

`schema_version: 2` adds substrate-specific `examples`. Each example has an
ID, exact `target_smiles`, non-empty `precursor_smiles`, and cited
`conditions` and/or `reported_yield`. The nested records must use
`scope: "substrate_specific"`; template-level yields are rejected in v2 so a
single-substrate number cannot be applied to every use of a template.

## Validation and interpretation

RENKIN validates a sidecar before search. Unsupported schema versions, bad
SMILES, duplicate or dangling references, invalid yield ranges, and malformed
records are hard errors. Recheck an asset with:

```bash
renkin evidence validate-sidecar --metadata evidence.json
```

Resolved output distinguishes `exact_substrate` evidence from
`template_only` precedent. An exact match reports what the cited source
achieved; it is still not a prediction for an unobserved experiment.
`confidence` and `success_probability` remain separate template-ranking
signals. An empty warning list means no warning was curated, not that no risk
exists.

## Importing from ORD (Open Reaction Database)

For a locally downloaded Open Reaction Database corpus, run the offline
matcher and audit script:

```bash
python scripts/ord_evidence_audit.py \
  --ord-data /path/to/ord-data \
  --renkin-bin target/release/renkin \
  --templates data/templates_extracted_5000.smi \
  --output-sidecar artifacts/ord_candidates.json \
  --output-report artifacts/ord_audit.json \
  --output-manifest artifacts/ord_manifest.json
```

The script uses `renkin evidence match` for exact target/precursor matching.
It does not fetch data, fuzzy-match structures, invent warnings, or turn
reported yields into predictions. Ambiguous, unmatched, and out-of-scope
records remain in the audit report instead of entering the sidecar.

The importer code is MIT; ORD data and sidecars derived from it require the
applicable ORD attribution/share-alike handling. This repository does not
bundle a real ORD corpus. See [the ORD evidence script
guide](https://github.com/kent-tokyo/renkin/blob/main/scripts/README_ord_evidence.md)
for installation and field-level mapping details.
