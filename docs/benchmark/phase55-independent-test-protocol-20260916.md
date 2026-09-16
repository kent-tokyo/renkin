# Phase 55.6 independent TEST protocol — 2026-09-16

This protocol is registered before either planner runs on this cohort.

- Cohort: 690 targets, frozen as `phase55-independent-test-20260916-001` before
  either arm runs. The frozen sample-list SHA-256 is
  `598c13359649eec04f2f370546edc53dae82bf48914a6b19f5b3cefdb3fe5162` and
  target-list SHA-256 is `bb71d58ef3889042d6fee17b786608d313430b82a6e52e3add1f9099513c876d`.
- Candidate: `phase55-recovery-v2-20260916`; depth 5 / beam 100 followed only
  for unresolved native cases by depth 6 / beam 200 recovery, 30 s internal.
- Runtime: Docker `--network none --cpus 8 --memory 6g --memory-swap 6g`.
- Images: RENKIN `renkin-bench/renkin@sha256:5575b13dd261f5eafc36c683d0b01b85bbc2f49e3787fb3c4cbc5590c37a2fd0`
  with OCI revision `e4a5cf2b7d9394824654cf6833cce6c270f9e35f`;
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
