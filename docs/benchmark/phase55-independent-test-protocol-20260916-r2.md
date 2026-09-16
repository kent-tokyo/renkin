# Phase 55.6 independent TEST protocol — 2026-09-16 r2

This protocol is registered before either planner runs on this cohort. It
supersedes neither the historical protocol nor its artifacts: the earlier
690-target execution is a **rehearsal**, because the effective AiZynthFinder
YAML hash was not frozen before its RENKIN arm began. Its target identities
are explicitly excluded below.

## Cohort and analysis

- Cohort: 690 targets, frozen as `phase55-independent-test-20260916-002`
  before either arm runs. The frozen sample-list SHA-256 is
  `31816728a4f77c0f0047e4334548003b27f7e30a451e801a3f1e89acfc916950`;
  target-list SHA-256 is
  `df96b97e9f1a0acffbd15b918bfdb4899bf916cc375d0d116faca005c078e1fe`.
- Exclusions: the earlier 500-target candidate, VAL-200, smoke-10, and all
  690 identities from `phase55-independent-test-20260916-001`. Exclusion is
  by both target ID and canonical SMILES.
- Candidate: `phase55-recovery-v2-20260916`; baseline depth 5 / beam 100,
  with native-only unresolved cases receiving depth 6 / beam 200 recovery
  under a 30-second internal budget. `configuration_id` and manifest budget
  include recovery depth, beam, timeout, and stage policy.
- Primary outcomes: native `route_found` and common strict
  route-to-shared-stock. Both require a paired 95% CI lower bound above zero
  for a superiority claim.
- Analysis: two-sided exact McNemar, alpha 0.05. N=690 retains the
  development-only assumptions used before the rehearsal: RENKIN-only
  31/200 and AiZynthFinder-only 20/200 (registered calculated power 0.80022).
  No rehearsal or r2 TEST outcome is an input to sample-size selection.

## Execution contract

- Runtime for both arms: Docker `--network none --cpus 8 --memory 6g
  --memory-swap 6g`.
- Boundary: 31-second external timeout plus 10-second cleanup grace.
  Wrapper-killed rows are timeouts. The rank-1 native output is retained and
  common strict is evaluated separately.
- RENKIN image: `renkin-bench/renkin@sha256:7baf2bad28d2599bbcaf975b2c7eaf6c2f3225e65f296d5777f036e1bc572b37`,
  OCI revision `b98a5c1e9867530efd3f1f1d80905eb1e7e425d4`.
- AiZynthFinder image:
  `renkin-compare-66/aizynthfinder@sha256:e1cad6a92772917095fb73a10ae2a30ff893fd19afd488754e2b0341ee5e7ac8`.
- AiZynthFinder YAML:
  `config_phase55_31s_rank1_shared_stock.yml`, SHA-256
  `9252db8d95852c72c70d9058a13fe1928530eadb7876dede801cf12e95d934a9`.
  It explicitly sets max transforms 5, iteration limit 100, time limit 31,
  `return_first: false`, and min/max routes 1. The mounted HDF5, ONNX, and
  template assets are hash-recorded by `public_data_provenance` in the arm
  manifest before the first row.

## Admission and reporting

Before reporting a result, `phase55_preflight.py` must verify the frozen r2
prefix/size, both manifests' input and image provenance, effective CPU/RAM
enforcement, timing receipts, and effective AiZ depth/top-k settings. A
failure creates `not_measured` or a new protocol; it never permits a
single-arm rerun or a post-hoc configuration change.

No r2 TEST row may change the cohort, configuration, image, mounted YAML,
asset hashes, budget, or analysis rule.
