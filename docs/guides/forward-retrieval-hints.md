---
title: "Forward Retrieval Hints with RENKIN"
description: "Derive bounded SMARTS partner requirements and reaction-center hints without inventing a partner or product."
---

# Forward retrieval hints

`renkin-forward hints` analyzes reversible templates against reactants you
already know. It does not run a reaction, search a partner catalog, or invent a
product molecule.

```bash
renkin-forward hints \
  --reactants "Brc1ccccc1" \
  --max-hints 20 \
  --max-matches-per-slot 20 \
  --max-assignments-per-template 200
```

Supply multiple `--reactants` values when several known inputs may occupy
different template slots. Add `--templates templates.smi` to analyze extracted
templates alongside embedded rules.

## Read the report correctly

The versioned `ForwardRetrievalHintReport` records, for each compatible
template:

- known-reactant slot assignments and bounded match sites;
- literal `query_smarts` for each missing partner slot;
- mapped bond-forming, bond-breaking, and bond-order deltas;
- `product_query_smarts`, a query pattern rather than a concrete product;
- provenance and explicit truncation statistics.

`query_smarts` and `product_query_smarts` are the authoritative template
constraints. Human-readable `required_features`, reaction-family labels, and
search terms are best-effort aids; they may be incomplete and must not replace
the SMARTS when selecting a partner.

## Bounds and non-goals

`--max-hints`, `--max-matches-per-slot`, and
`--max-assignments-per-template` must be positive. A reported cap means that
the result is partial, not that no additional matching template exists.

Hints carry no claim about partner availability, conditions, yield, safety, or
experimental success. Use [enumeration](forward-enumeration.md) with an
explicit partner library for bounded products, then
[forward prediction](forward-prediction.md) or route audit for structural
checking.
