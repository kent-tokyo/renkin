---
title: "Forward Enumeration with RENKIN"
description: "Enumerate bounded, template-guided forward candidates from one known reactant and an explicit partner library."
---

# Forward enumeration

`renkin-forward enumerate` explores what an explicit template set can make
from one known reactant. It is bounded partner enumeration, not a generative
reaction model.

```bash
# Unary templates only
renkin-forward enumerate --reactant "Brc1ccccc1" --max-results 5

# Include binary-template partners from a local SMILES file
renkin-forward enumerate \
  --reactant "Brc1ccccc1" \
  --partners partners.smi \
  --max-results 10 \
  --max-partners-per-template 50 \
  --max-combinations 2000
```

`partners.smi` contains one SMILES per line; blank lines and `#` comments are
ignored. A missing, unreadable, empty, or entirely malformed partner file is a
hard error.

## Bounds and output

The command always emits a versioned `ForwardEnumerationReport`. It records
merged candidates, template provenance, warnings, and truncation statistics.
The three explicit limits are:

| Limit | Meaning |
| --- | --- |
| `--max-results` | Maximum merged candidates returned. |
| `--max-partners-per-template` | Maximum library partners tried for each compatible template slot. |
| `--max-combinations` | Total template/slot/partner attempts across the run. |

All must be positive. A cap is reported; it is never silently interpreted as
an exhaustive search. Templates with two or more missing partners are
reported as unsupported rather than guessed.

## What it does not establish

An enumerated product is a template-compatible structural proposal. It does
not provide conditions, catalyst choice, yield, price, availability, or a
reaction-success probability. Apply a route audit and explicit stock policy
before treating a proposal as a route candidate.

For a direct list of reactants and predicted products, use
[`predict`](forward-prediction.md). For partner-free search terms and SMARTS
requirements, use [retrieval hints](forward-retrieval-hints.md).
