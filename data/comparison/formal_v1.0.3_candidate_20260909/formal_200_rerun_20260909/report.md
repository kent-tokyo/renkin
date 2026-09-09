# VAL-200 rerun — 2026-09-10

## Conditions

- VAL-200 disjoint sample
- shared-stock: eMolecules ∪ RENKIN canonical stock
- 5,000 extracted templates
- depth 5, beam 100, max-routes 1
- recovery: depth 6, beam 200, native stage policy
- RENKIN tool version: 1.0.4

## RENKIN result

- route_found: **134/200 (67.0%)**
- validator-confirmed route: 126/200 (63.0%)
- strict validated route to configured stock: 134/200 (67.0%)
- route tree parseable: 134/134
- timeout/crash: 0/200
- total elapsed p50/p95: 1.55s / 30.24s
- sweep wall time: 1,221.82s

## Context comparison

The latest stored AiZynthFinder arm reports route_found 134/200 (67.0%),
strict validated route 123/200 (61.5%), and total elapsed p50/p95 4.71s /
7.17s. These figures are useful context, but are not a new same-session paired
run; adapter output and validation semantics differ. Therefore this rerun
establishes parity in route_found count, not a proven universal win.

Raw rows, aggregate, and manifest are stored beside this report.
