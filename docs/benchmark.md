---
title: "RENKIN Benchmark Overview"
description: "A concise overview of RENKIN benchmark results, comparison conditions, and reproducibility boundaries."
---

# Benchmark overview

This page is a short index of benchmark results. It is not a complete lab
notebook. Historical runs, hashes, and failure-by-failure analysis belong in
the dedicated comparison guide and repository artifacts.

## Current comparison

The latest frozen VAL-200 shared-stock comparison found native routes for
134/200 targets (67.0%) for both RENKIN and AiZynthFinder 4.4.1.

Under the stricter common validator plus shared-stock endpoint:

| Tool | Success |
|---|---:|
| RENKIN | 134/200 (67.0%) |
| AiZynthFinder 4.4.1 | 123/200 (61.5%) |

This is a fixed-cohort result, not a universal CASP superiority claim. It does
not measure experimental yield or replace chemist review.

See the [formal comparison guide](guides/open-source-retrosynthesis-comparison.md)
for the protocol, validation policy, and claim limits.

## Historical USPTO-50k stress test

The former 4,907-target run used a small configured stock and was a
route-to-stock stress test, not the canonical single-step USPTO-50k task.
Its headline v0.15.5 figures (78.0%, 95.9%, and 81.8% OOD) were invalidated
after rule and validator fixes. They must not be used as current performance.

The corrected snapshot reported:

| Metric | Result |
|---|---:|
| Search-to-stock | 986/4,907 (20.09%) |
| Atom-balance filtered | 756/4,907 (15.41%) |
| Rule-validator confirmed | 43/4,907 (0.88%) |

These are historical internal diagnostics, not chemical accuracy rates.

## What “solved” means

Unless a benchmark says otherwise, `solved` means that the planner returned a
complete route whose leaves satisfy the configured stock policy. It is not a
match against USPTO ground-truth reactants and is not proof that a synthesis
will work in the laboratory.

## Reproduce a local run

```bash
cargo run --release --bin renkin-bench -- \
  --input data/uspto50k_test.smi \
  --depth 5 --beam-width 100 \
  --templates data/templates_extracted_5000.smi
```

For matched planner comparisons, use the [comparison guide](guides/open-source-retrosynthesis-comparison.md).
