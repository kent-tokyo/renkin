---
title: "Forward Reaction Prediction with RENKIN"
description: "Use reversible template application to enumerate bounded forward candidates and check a proposed retrosynthetic route."
---

# Forward reaction prediction

`renkin-forward` applies the reversible template set to known reactants. It
is a deterministic, template-based check: it does not infer catalysts,
conditions, yield, or experimental success.

```bash
cargo run --release -p renkin-forward -- \
  predict --reactants "Oc1ccccc1C(=O)O" "CCO" --report --max-results 5
```

Add `--templates templates.smi` to load extracted SMIRKS alongside the
embedded rules. Missing, unreadable, or empty template files fail before
prediction.

## Choose an output

Without `--report`, `predict` returns a compact JSON array of
`{template, products, weight}`. Several templates can legitimately emit the
same products; `weight` only ranks template output and is not a probability.

With `--report`, it returns a versioned `ForwardPredictionReport` with merged
candidates, source template provenance, deterministic rank, bounds, and
warnings. Prefer this format when saving results or comparing runs.

## Validate a route

```bash
renkin-forward validate \
  --route-json '{"steps":[...]}' \
  --templates templates.smi \
  --max-results 5
```

Each supplied retrosynthetic step is replayed forward against the bounded
template set. A matching product is a structural replay result, not a claim
about reaction conditions or laboratory viability. A template with no usable
forward representation remains explicitly not evaluable rather than passing.

## Reproducible use

Pin the binary version, template file hash, input reactant order, and
`--max-results`. Preserve the report form for provenance. For route-level
audit, stock policy, and evidence receipts, use
[`renkin audit-route`](audit-reproducibility-contract.md) after the forward
check.

Related tools: [bounded partner enumeration](forward-enumeration.md) and
[partner-free retrieval hints](forward-retrieval-hints.md).
