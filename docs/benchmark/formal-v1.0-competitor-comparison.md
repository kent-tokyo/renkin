# v1.0.0 Formal Route-to-Stock Competitor Benchmark

Status: **comparison complete; statistical superiority PASS; formal publication
gate HOLD on the frozen v1.0.0 output**.

The completed run found 577/4,903 primary successes for RENKIN v1.0.0 and
200/4,903 for AiZynthFinder 4.4.1. The paired difference was +7.689 percentage
points with a fixed 10,000-resample paired-bootstrap 95% CI of [+6.812,
+8.566] percentage points. The statistical gate therefore passed. The formal
publication gate remains on hold because two RENKIN rows reported a route but
did not produce a parseable normalized route tree. The existing 50-target
current-master snapshot and historical 100/500-target artifacts remain
feasibility or diagnostic evidence only.

The complete report and per-target artifacts are under
`data/comparison/formal_v1.0/`. This result is specific to the declared shared-
stock endpoint; it is not a claim of universal CASP superiority, experimental
yield, or planner replacement.

### v1.0.1 remediation status

The two held targets were rerun with the route-normalization and explicit-H /
surviving-valence fixes included in the v1.0.1 candidate. Both returned
parseable, stock-terminated route trees with no crash or timeout. These
targeted results demonstrate that the observed failure modes are fixed, but
they do not rewrite the frozen v1.0.0 arm or promote the formal publication
gate to PASS. That requires a fresh, complete 4,903-target arm and verification
under the same protocol.

## Primary claim and endpoint

The primary comparison is a paired, same-target, shared-stock comparison of
RENKIN v1.0.0 against AiZynthFinder 4.4.1. A target is a primary success only
when the rank-1 route is found, its route tree is parseable, and every leaf is
in the exact configured shared stock after the independent RDKit-based
post-hoc check (`route_to_shared_stock`). A native tool-reported success that
fails this check is not a success for the primary endpoint.

Superiority is declared only if all of the following hold:

1. the paired RENKIN-minus-AiZynthFinder primary-rate 95% bootstrap confidence
   interval is strictly above zero;
2. the paired result has no target-set mismatch, duplicate, missing row, or
   unclassified timeout/crash; and
3. the route-validation and input-integrity gates pass, with all failures
   retained in the published per-target table.

The result is a route-to-configured-stock success rate, not an experimental
yield, a human-chemist quality score, or proof that RENKIN replaces a planner.

## Frozen population and inputs

- Population: all **4,903** rows in
  `data/comparison/sample_full_sorted.jsonl`, a frozen deterministic ordering
  derived from the USPTO-50k test corpus.
- Target-list SHA-256:
  `572f7796f3aec07c85f9487c99803cff491623a59b1258e1e77a774a8c838ce8`.
- Shared stock: `data/comparison/shared_stock/shared_stock.smi`.
- Shared-stock SHA-256:
  `9046b2e234efd32a44c9209f1f80c4162ca35c1629deaf3b69d4dfcc82de38b9`.
- RENKIN template bundle:
  `data/templates_extracted_500.smi`.
- Template SHA-256:
  `01cc64b636f4b4690b00dbb0b606c40b64bc94c511b86f336a30d795aba4bb82`.

Before every arm, run the network-free preflight:

```bash
python3 scripts/validate_formal_benchmark.py \
  --target-list data/comparison/sample_full_sorted.jsonl \
  --stock data/comparison/shared_stock/shared_stock.smi \
  --templates data/templates_extracted_500.smi \
  --target-list-sha256 572f7796f3aec07c85f9487c99803cff491623a59b1258e1e77a774a8c838ce8 \
  --stock-sha256 9046b2e234efd32a44c9209f1f80c4162ca35c1629deaf3b69d4dfcc82de38b9 \
  --templates-sha256 01cc64b636f4b4690b00dbb0b606c40b64bc94c511b86f336a30d795aba4bb82
```

## Fixed execution conditions

Each tool runs sequentially, one target at a time, with the same target order,
same wall-clock timeout (150 seconds plus a 30-second termination grace
period), and the same shared stock. RENKIN uses depth 5 and beam width 100;
these values are fixed before the run. The binary commit, binary hash, model or
template hashes, host conditions, timeout, and command line are recorded in a
per-arm manifest. A resumed run is acceptable only if the manifest and every
input hash remain unchanged.

The native-stock arm is secondary and must not be pooled with the shared-stock
arm. It answers what each tool does with its own public configuration, not
which search procedure is superior. Syntheseus, SynPlanner, and ASKCOS may be
reported as additional arms only when their exact version, model/data bundle,
stock semantics, budget, and complete per-target output can be independently
reproduced. If any of those conditions is unavailable, the cell is recorded as
not measured rather than filled with an incomparable published number.

## Required outputs

For every arm, retain JSONL rows for all 4,903 target IDs, an aggregate report,
the start/end manifest, and a post-hoc verification report. Produce a paired
report from the exact same target-id sets using the existing bootstrap and
McNemar implementation in `scripts/compare_paired_report.py`.

Report at minimum: raw route-found rate, strict route-to-shared-stock rate,
route-tree parseability, target-element-accounting status, timeout/crash/setup
counts, p50/p95 latency, and peak RSS with its measurement method. Publish
both absolute rates and paired differences with confidence intervals; never
publish only the favorable numerator.

## Interpretation boundary

The 4,903-target corpus is a route-to-stock stress test, not a ground-truth
multi-step synthesis-success dataset. It measures the declared search and
stock policy under a fixed bundle. Any headline must name the endpoint,
population, tool versions, stock arm, and date. A positive primary gate is
evidence of superiority on this protocol only; it is not evidence of universal
CASP superiority or laboratory success.
