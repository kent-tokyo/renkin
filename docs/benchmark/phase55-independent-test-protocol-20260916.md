# Phase 55.6 independent TEST protocol — 2026-09-16

This protocol is registered before either planner runs on this cohort.

- Cohort: 690 targets, `bb71d58ef3889042d6fee17b786608d313430b82a6e52e3add1f9099513c876d`.
- Candidate: `phase55-recovery-v2-20260916`; depth 5 / beam 100 followed only
  for unresolved native cases by depth 6 / beam 200 recovery, 30 s internal.
- Runtime: Docker `--network none --cpus 8 --memory 6g --memory-swap 6g`.
- Images: RENKIN `renkin-bench/renkin@sha256:c9af17e50975f74629ba17468117778e0f0292d72fdbe44bb4cd5eeec0c2c49f`;
  AiZynthFinder `renkin-compare-66/aizynthfinder@sha256:e1cad6a92772917095fb73a10ae2a30ff893fd19afd488754e2b0341ee5e7ac8`.
- Boundary: 31 s external timeout plus 10 s cleanup grace; wrapper-killed rows
  are timeouts. Top-1 native output is retained and common strict is evaluated
  separately.
- Primary outcomes: native `route_found` and common strict route-to-shared-stock.
  Both require a paired 95% CI lower bound above zero for superiority.
- Analysis: two-sided exact McNemar, alpha 0.05. N=690 was fixed before TEST
  from development-only discordant assumptions 31/200 RENKIN-only and 20/200
  AiZynthFinder-only (computed power 0.80022).

No TEST row may change this configuration, N, target list, or analysis rule.
