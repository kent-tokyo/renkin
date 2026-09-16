# Phase 55 r2 independent comparison result

Date: 2026-09-16  
Protocol: `phase55-independent-test-20260916-002`  
Scope: RENKIN `1.0.7` at `b98a5c1` versus AiZynthFinder `4.4.1`

## Conclusion

The pre-registered **coverage** gate passes for this exact shared-stock,
shared-budget configuration.  On the frozen 690-target TEST cohort, RENKIN
found a native route and a strict validated route to configured stock for
481 targets (69.71%); AiZynthFinder did so for 32 (4.64%).  The paired
difference is +65.07 percentage points, with a 95% bootstrap CI of
**[+61.45, +68.55] pp** for both primary measures.  McNemar's exact
two-sided p-value is `1.38e-135` (RENKIN-only 449, AiZynthFinder-only 0).

This proves a result only for the registered tools, images, assets, stock,
budget, and cohort below.  It is not evidence of universal superiority over
other AiZynthFinder configurations or other CASP tools, and it is not a claim
of experimental synthesis success.

## Registered execution and validation

Both arms completed 690/690 rows without crash or adapter failure.  The arm
verifiers passed; the paired preflight passed with frozen-cohort membership,
identical sample/stock/template hashes, clean worktrees, input immutability,
effective 8 CPU / 6 GiB Docker enforcement, fixed output settings, and
planner timing receipts.

| Item | RENKIN | AiZynthFinder |
|---|---:|---:|
| Image digest | `renkin-bench/renkin@sha256:7baf…72b37` | `renkin-compare-66/aizynthfinder@sha256:e1ca…7ac8` |
| Search budget | depth 5, beam 100; native-only recovery depth 6 / beam 200 | max transforms 5, rank 1 |
| Wall-clock cap | 31 s + 10 s grace | 31 s + 10 s grace |
| Resources | 8 CPU, 6 GiB, network disabled | 8 CPU, 6 GiB, network disabled |
| Native `route_found` | 481/690 (69.71%) | 32/690 (4.64%) |
| Strict validated route to configured stock | 481/690 (69.71%) | 32/690 (4.64%) |
| Timeout / crash | 1 / 0 | 0 / 0 |

The paired native latency comparison is restricted to the 32 targets solved
by both tools: RENKIN minus AiZynthFinder was -4,990 ms (95% bootstrap CI
[-5,371, -4,662] ms).  This is a conditional latency observation, not a
whole-cohort speed ranking.  RENKIN's peak-RSS and time-to-first-route
receipts are not available in this arm, so the performance portion of the
broader Phase 55 exit gate remains incomplete.

## Reproduction artifacts

- `formal-renkin.manifest.json` and `formal-aizynthfinder.manifest.json`
- `formal-*.rows.jsonl` and `formal-*.aggregate.json`
- `paired_stats_native.json`, `paired_stats_shared_stock.json`, and paired
  tables
- `frozen-cohort.json` and `frozen-sample-list.jsonl`

All paths are under `artifacts/phase55-independent-test-20260916-r2/`.
