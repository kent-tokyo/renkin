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

The built-in rule set is 24 hand-crafted, human-readable disconnections
(ester cleavage, Suzuki, Heck, and so on). For broader reaction
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

## Reading the result

Each route has `steps` and `building_blocks`; each step identifies its
`target`, `precursors`, and `template_id`. See the [Python API](../api/python.md)
for the current return contract instead of relying on a copied result snapshot.
`step_confidence` and `success_probability` rank search results; neither is a
measured yield. Hand-crafted `conditions` and `procedure_hint` are defaults,
not citations. Attach literature evidence through a validated sidecar as
described in [Reaction Evidence Metadata](reaction-evidence.md).

## Current Limitations

- Default stock is 402 compounds from the repository file when found, or 152
  compiled-in compounds otherwise. The 24 hand-crafted rules do not cover
  every reaction; supply explicit stock and templates for your use case.
- No literature/patent auto-search, no automatic side-reaction prediction, no
  yield prediction — see [Reaction Evidence Metadata](reaction-evidence.md)
  for exactly what curated evidence is and isn't.
- Benchmark claims depend on the exact cohort, stock, assets, and budget. Check
  the [benchmark overview](../benchmark.md) before citing a success rate.

## Next Steps

- [Python API reference](../api/python.md) — full parameter list, `predict_forward`, `validate_forward`
- [Reaction Evidence Metadata](reaction-evidence.md) — conditions, yields, references, warnings
- [Rust API](../api/rust.md) / [WASM API](../api/wasm.md) — if you need the engine outside Python
