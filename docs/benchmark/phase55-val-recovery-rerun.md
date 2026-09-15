# Phase 55.4 recovery candidate — registered VAL rerun

## Purpose

This is a development-only VAL selection run, not the independent Phase 55.6
comparison. It re-measures the already implemented non-displacing recovery
candidate against the baseline under one pre-registered per-target budget.

## Registered v2 conditions

- Cohort: `data/phase_b1_frontier/val_sample_disjoint_200.jsonl` (200 rows).
- Shared stock: `data/comparison/formal_v1.0.3_candidate_20260908/stock_union_eMolecules_renkin_canonical.rstock`.
- Templates: `data/templates_extracted_5000.smi`.
- Baseline: depth 5, beam 100, rank-1, standard search, **31-second external
  process cap**.
- Candidate: the same baseline, then native recovery depth 6 / beam 200; the
  cooperative recovery search is set to **30 seconds** and the full CLI
  process has the same **31-second external cap**.
- Both commands retain a 10-second termination grace only for cleanup after a
  cap breach. A wrapper-killed row is a timeout, never a pass.
- No model, template, stock, or per-target hand tuning.

The earlier v1 candidate used a 31-second cooperative recovery deadline and a
41-second external cap. It recovered five strict routes without regressions,
but 38/200 recovery totals exceeded 31 seconds (maximum 31,026.14 ms).
`SearchControl` is intentionally cooperative, so this is a boundary failure,
not a basis for rounding the budget upward. The v1 artifact is retained for
diagnosis and is excluded from candidate selection.

## Selection rule

Retain the candidate only if all are true:

1. no baseline success becomes a candidate failure;
2. native and common-strict counts do not decrease;
3. no timeout/crash and every candidate row has a recovery attempt record;
4. every recorded recovery total is at most 31 seconds, and every externally
   observed candidate process wall-clock is at most 31 seconds; and
5. the candidate adds at least one valid completed route.

The rows, manifests, command lines, input hashes, and verification JSON are
written to a new ignored artifact directory. Run
`verify_non_displacing_recovery.py` with both candidate rows and the
same-cohort baseline rows, `--require-zero-regression`,
`--require-zero-strict-regression`, `--require-complete-attempts`,
`--budget-ms 31000`, and `--process-budget-ms 31000`. This checks the shared-stock strict metric directly
instead of assuming that native-route preservation implies it. The result does
not alter the frozen Phase 55 TEST cohort or its exposure record.
