---
title: "RENKIN and AiZynthFinder: Registered Comparison"
description: "The current registered shared-stock comparison, its reproducibility contract, and its claim boundary."
---

# RENKIN and AiZynthFinder: registered comparison

This page is the short, current comparison record. It deliberately separates
coverage, latency, resources, and chemical validity. Detailed frozen
protocols and historical records stay in `docs/benchmark/` and
`data/comparison/`; they are not product claims.

## Current registered result

The Phase 55 r2 TEST compared RENKIN `1.0.7` at `b98a5c1` with
AiZynthFinder `4.4.1` on a frozen 690-target cohort. Both arms used the same
registered shared-stock endpoint, a 31-second cap plus 10-second grace, 8 CPU,
6 GiB memory, disabled network, and pre-pinned images and assets.

| Endpoint | RENKIN | AiZynthFinder | Paired difference |
| --- | ---: | ---: | ---: |
| Native route found | 481/690 (69.71%) | 32/690 (4.64%) | +65.07 pp, 95% CI +61.45 to +68.55 |
| Strict route to configured stock | 481/690 (69.71%) | 32/690 (4.64%) | +65.07 pp, 95% CI +61.45 to +68.55 |

All 690 rows in each arm completed; RENKIN had one timeout and no crash,
AiZynthFinder had neither. McNemar's exact two-sided p-value was
`1.38e-135` (RENKIN-only 449; AiZynthFinder-only 0).

The complete [Phase 55 r2 result record](../benchmark/phase55-r2-result-20260916.md)
names image digests, artifact locations, and the remaining performance gap.

## What this result does and does not establish

It supports a **coverage** conclusion for exactly the registered cohort,
stock, templates/models, images, budget, and validator. It does not prove:

- performance leadership across all targets: peak RSS and time-to-first-route
  receipts for RENKIN remain incomplete;
- superiority for another AiZynthFinder configuration, another stock, or any
  other CASP tool;
- synthetic feasibility, yield, safety, cost, or sustainability in a lab.

The conditional latency comparison is limited to the 32 targets both tools
solved: RENKIN minus AiZynthFinder was -4,990 ms (95% bootstrap CI -5,371 to
-4,662 ms). It is not a whole-cohort speed ranking.

## Endpoint contract

`native route found` is the tool's own completed route result. `strict route
to configured stock` additionally requires the normalized route tree to pass
the independent validator and all leaves to match the declared stock. The
stock is part of the benchmark identity, not a generic notion of commercial
availability.

Neither endpoint is a USPTO ground-truth reactant match or a chemistry-quality
judgment. The structural validator is deliberately conservative and records
unavailable evidence as `partial` or `not_evaluable`.

## Reproduce or inspect

The registered run is frozen; do not overwrite it. To inspect the contract and
artifacts, start with:

```bash
python3 scripts/phase55_preflight.py --help
python3 scripts/audit_phase55_cohort_provenance.py --help
python3 scripts/compare_paired_report.py --help
```

Use a new run identifier and preserve raw JSONL rows, manifests, tool/image
hashes, resource limits, and verifier outputs. The [r2 protocol](../benchmark/phase55-independent-test-protocol-20260916-r2.md)
is the authoritative command-and-artifact record.

## Earlier records

- The 200-target VAL result is a development cohort, not a replacement for
  this independent TEST.
- The [v1.0.1 4,903-target record](../benchmark/formal-v1.0-competitor-comparison.md)
  remains historical evidence with its own configuration and cannot be pooled
  with Phase 55.
- ASKCOS, Syntheseus, SynPlanner, and commercial tools are not represented by
  this registered result. Do not infer a ranking from unlike configurations.
