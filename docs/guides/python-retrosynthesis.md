---
title: "Python Retrosynthesis Package: RENKIN Without RDKit"
description: "How to plan multi-step retrosynthesis routes in Python with RENKIN -- a pip-installable, RDKit-free, pure-Rust engine with custom stock, templates, and evidence metadata."
---

# Python Retrosynthesis with RENKIN

If you're looking for an open-source Python library for computer-aided
synthesis planning (CASP) that doesn't require RDKit or a C/C++ toolchain,
RENKIN ships as a pip-installable wheel with the search engine, template set,
and building-block database compiled in.

## Install

```bash
pip install renkin
```

No RDKit, no Boost, no C/C++ compiler needed at install time — RENKIN's
chemistry layer ([`chematic`](https://docs.rs/chematic/)) and search engine
are both pure Rust, compiled ahead of time into the wheel.

## A Working Example

```python
--8<-- "examples/quickstart.py"
```

`find_routes` always returns a **JSON string**, not a `dict` — call
`json.loads()` on it. Full parameter list and return shape:
[Python API reference](../api/python.md).

## Custom Building Blocks

With no `building_blocks` argument, RENKIN searches against `data/building_blocks.smi`
(402 unique compounds) *if that path resolves relative to your current working
directory* — in practice, only when running from a checkout of this repo. A
`pip install renkin` wheel does not bundle that file, so a plain `pip install`
run from anywhere else silently falls back to a smaller, compiled-in
152-compound set instead. Don't rely on either default having a specific
compound — supply your own stock explicitly:

```python
import renkin, json

my_stock = ["CC(=O)O", "Oc1ccccc1", "c1ccccc1", "Brc1ccccc1", "OB(O)c1ccccc1"]
result = json.loads(renkin.find_routes(
    target="c1ccc(-c2ccccc2)cc1",
    building_blocks=my_stock,
    depth=3,
))
```

Any SMILES that fails to parse is silently skipped, not an error — it just
can't match as a leaf building block.

## Extracted Templates

The built-in rule set is 27 hand-crafted, human-readable disconnections
(ester cleavage, Suzuki, Buchwald-Hartwig, and so on). For broader reaction
coverage, load additional SMIRKS templates auto-extracted from USPTO-50k/MIT
via rdchiral:

```python
result = json.loads(renkin.find_routes(
    target="CC(=O)Oc1ccccc1C(=O)O",
    templates_path="data/templates_extracted_5000.smi",
    depth=5,
))
```

Each extracted template gets a stable `template_id`
(`smirks-sha256:<hex>`) derived from the SMIRKS itself, independent of file
order or position — unlike the display name (`extracted_0`, `extracted_1`, ...),
which shifts if the file is re-sorted or re-extracted.

## Evidence Metadata (Conditions, Yields, References)

You can attach curated external evidence — reported conditions, yields, DOIs,
patents, known side-reaction warnings — to a specific template, keyed by its
`template_id`:

```python
result = json.loads(renkin.find_routes(
    target="CC(=O)Oc1ccccc1C(=O)O",
    templates_path="data/templates_extracted_5000.smi",
    template_metadata_path="sidecar.json",
    depth=5,
))
for route in result["routes"]:
    for step in route["steps"]:
        if "evidence" in step:
            print(step["template_id"], step["evidence"])
```

Steps whose template has no matching sidecar entry simply have no `evidence`
key — nothing is fabricated. See the [Reaction Evidence Metadata
guide](reaction-evidence.md) for the sidecar format and what `evidence` is
(and isn't).

## Reading the Result

Real output for aspirin at `depth=1` (hand-crafted rules only, one route shown):

```python
{
  "depth": 1,
  "score": 1.099087,
  "confidence": 1.0,
  "success_probability": 1.0,
  "route_cost": 8.298266666666667,
  "building_blocks": ["OC(=O)C", "c1cccc(c1O)C(O)=O"],
  "steps": [
    {
      "target": "OC(=O)c1ccccc1OC(=O)C",
      "rule": "ester_cleavage",
      "template_id": "rule:ester_cleavage",
      "precursors": ["OC(=O)C", "c1cccc(c1O)C(O)=O"],
      "step_confidence": 1.0,
      "atom_economy": 90.90950376941474,
      "atom_economy_raw_percent": 90.90950376941474,
      "atom_economy_status": "normal",
      "reaction_family": "esterification",
      "conditions": {"catalyst": "NaOH or LiOH (2 eq)", "solvent": "THF/H₂O (2:1)", "temperature": "rt → 60 °C"},
      "procedure_hint": "Dissolve in THF/H₂O, add NaOH (2 eq), stir at 60 °C, acidify to pH 2.",
      "metadata_source": "handcrafted_default",
      "metadata_scope": "reaction_family"
      # evidence appears only when a --template-metadata sidecar matches this template_id
    }
  ]
}
```

`step_confidence`/`success_probability` are template-frequency-derived
search-ranking scores (here 1.0 because, with only hand-crafted rules loaded,
every rule has equal weight) — not a measured or predicted experimental
yield. `conditions`/`procedure_hint` are rule-author-supplied defaults for
hand-crafted rules (`metadata_source: "handcrafted_default"`), not a literature
citation — see [Reaction Evidence Metadata](reaction-evidence.md) for the
distinction and how to attach real cited evidence.

## Current Limitations

- The default stock (402 compounds when running from a repo checkout, 152
  otherwise — see [Building Blocks](../api/python.md#building-blocks)) and 27
  hand-crafted rules cover common pharmaceutical disconnections well, but broader reaction space needs
  the larger extracted-template files or your own stock.
- No literature/patent auto-search, no automatic side-reaction prediction, no
  yield prediction — see [Reaction Evidence Metadata](reaction-evidence.md)
  for exactly what curated evidence is and isn't.
- Current benchmark numbers are still being re-measured after a validator fix
  — see the [Benchmark page](../benchmark.md) before citing a success rate.

## Next Steps

- [Python API reference](../api/python.md) — full parameter list, `predict_forward`, `validate_forward`
- [Reaction Evidence Metadata](reaction-evidence.md) — conditions, yields, references, warnings
- [Rust API](../api/rust.md) / [WASM API](../api/wasm.md) — if you need the engine outside Python
