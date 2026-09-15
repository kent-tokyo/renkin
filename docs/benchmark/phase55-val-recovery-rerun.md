# Phase 55.4 recovery candidate — registered VAL rerun

## Purpose

This is a development-only VAL selection run, not the independent Phase 55.6
comparison. It re-measures the already implemented non-displacing recovery
candidate against the baseline under one pre-registered per-target budget.

## Frozen conditions

- Cohort: `data/phase_b1_frontier/val_sample_disjoint_200.jsonl` (200 rows).
- Shared stock: `data/comparison/formal_v1.0.3_candidate_20260908/stock_union_eMolecules_renkin_canonical.rstock`.
- Templates: `data/templates_extracted_5000.smi`.
- Baseline: depth 5, beam 100, rank-1, standard search.
- Candidate: the same baseline, then native recovery depth 6 / beam 200.
- Total recovery deadline: **31 seconds** per target. This is registered
  before the rerun because the historical 30-second measurement had 38 small
  wrapper-boundary overruns; it is not retroactively relabelled as a 30-second
  pass.
- External deadline: 41 seconds (31 seconds search budget plus 10 seconds
  termination grace). No model, template, stock, or per-target hand tuning.

## Selection rule

Retain the candidate only if all are true:

1. no baseline success becomes a candidate failure;
2. native and common-strict counts do not decrease;
3. no timeout/crash and every candidate row has a recovery attempt record;
4. every observed candidate elapsed time is at most 31 seconds; and
5. the candidate adds at least one valid completed route.

The rows, manifests, command lines, input hashes, and verification JSON are
written to a new ignored artifact directory. The result does not alter the
frozen Phase 55 TEST cohort or its exposure record.
