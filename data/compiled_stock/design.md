# Compiled stock v1

`renkin stock compile` turns a line-oriented `.smi` stock into a deterministic
`.rstock` snapshot. It is intended for large, repeatedly loaded private or
vendor stocks. The source `.smi` remains the interoperable source of truth.

```bash
renkin stock compile \
  --input stock.canonical.smi \
  --output stock.rstock \
  --fail-on-rejection

renkin --target 'CCO' --building-blocks stock.rstock
```

`ChemEnv::load` detects the compiled magic automatically, so no new search flag
is required. Loading a compiled stock does not parse or canonicalize individual
molecules. Before using any entry, it verifies schema and normalization,
source/payload/content hashes, molecule count, sorting and uniqueness, and
line/row resource limits. Policy mismatch, corruption, and truncation fail
closed. Private compiled stocks should be regenerated rather than committed.

See [findings.md](findings.md) for the exact performance method, hashes, memory
figures, and comparison limits.
