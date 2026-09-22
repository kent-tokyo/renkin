---
title: "RENKIN Benchmark Overview"
description: "Current registered comparison results, historical records, and the limits of each claim."
---

# Benchmark overview

This page separates the current registered result from older diagnostic runs.
Every benchmark claim is tied to a tool revision, cohort, stock definition,
assets, endpoint, and resource budget. A route found by a planner is not proof
of experimental success.

## Current registered result: Phase 55 r2

The 2026-09-16 independent TEST used a frozen 690-target cohort, shared stock,
pinned tool assets, and declared budgets. It compared RENKIN `1.0.7` at
`b98a5c1` with AiZynthFinder `4.4.1`.

| Endpoint | RENKIN | AiZynthFinder | Paired difference |
| --- | ---: | ---: | ---: |
| Strict route to the declared shared stock | 481/690 (69.71%) | 32/690 (4.64%) | +65.07 pp, 95% CI +61.45 to +68.55 |

This is a **coverage result for that registered configuration**. It is not a
measurement of v1.0.9, universal CASP superiority, experimental viability,
peak RSS, time-to-first-route, or whole-cohort latency. Those last three
performance receipts were deliberately recorded as `not_measured`.

Read the [formal Phase 55 result record](benchmark/phase55-r2-result-20260916.md)
and [metric truth table](benchmark/phase55-metric-truth-table.md) before
quoting the result.

## Older records

| Record | Why it remains | How to interpret it |
| --- | --- | --- |
| [VAL-200 comparison](guides/open-source-retrosynthesis-comparison.md) | Diagnosis of search/stock differences | The saved 134/200 native tie is not a new-release measurement or a superiority claim |
| [v1.0.1 4,903-target arm](benchmark/formal-v1.0-competitor-comparison.md) | Corrected historical route-to-stock record | A separate, older configuration; do not combine it with Phase 55 |
| `data/comparison/` | Raw rows, manifests, and audits | Evidence artifacts, not product headline copy |

## Reproduce a local smoke run

```bash
cargo run --release --bin renkin-bench -- \
  --input data/uspto50k_test.smi \
  --depth 5 --beam-width 100 \
  --templates data/templates_extracted_5000.smi
```

This command is a local planner smoke test. It does not reproduce a formal
cross-tool comparison. For a registered comparison, start with the protocol,
pin every input hash and image identity, preserve raw rows, and run the
verification scripts described in the formal result record.
