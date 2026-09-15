# Phase 55.6 independent TEST selection

更新: 2026-09-12

This is the selection and provenance record for the Phase 55.6 candidate
cohort. It is not a benchmark result. The cohort must be frozen before either
planner is executed and must not be changed after inspecting outcomes.

## Candidate

- Source: `data/comparison/sample_full_sorted.jsonl`
- Source provenance: `data/comparison/sample_manifest.json`
- Historical exclusions: results 100/500 and the saved VAL-200 paired artifact
- Eligible population after exclusions: 4,403 targets
- Candidate size: 500 targets
- Candidate cohort hash: recorded in the frozen manifest below
- Selection status: provenance audit passed; versioned cohort frozen; arm preflight pending

The candidate and audit were generated in a local temporary directory so they
are not accidentally treated as a formal result. A local freeze rehearsal
passed with freeze ID `phase55-test-candidate-20260912-003`. The versioned
frozen manifest is now at
`data/comparison/aizynthfinder_accuracy_phase55/phase55-test-candidate-20260912-003/frozen_manifest.json`
with SHA-256
`9b9c91ed85fe68b7122f18d41ab0ef1a81061b274656021bf5774d0fc34daf0a`.
The adjacent `sample_list.jsonl` has 500 rows and SHA-256
`2e0f53a3b3dbfd388341e4a220946e912ca16359deb9e7d0005c7650863e7067`.
Both artifacts pass saved-manifest verification; clean-checkout preflight is
still required before either planner is run. The cohort-level preflight in
`scripts/phase55_preflight.py` also passes (`eligible=true`) for the frozen
manifest: 500 IDs, matching sample-list hash, and matching ID set.

The comparison runner also locks each output ledger before reading or writing
it. A resumed run must reject an already-corrupted duplicate-target ledger or a
concurrent writer; such an artifact is never eligible for formal statistics.

Before the 500-target run, execute the first 50 rows of this same frozen list
as the clean-checkout smoke gate. `scripts/phase55_preflight.py` accepts
`--frozen-cohort` and `--smoke-size 50`; it rejects an arm pair unless both
ledgers are exactly that deterministic prefix and both manifests hash the full
frozen `sample_list.jsonl`. A passing smoke proves only that the paired
environment and adapters are runnable. It is neither a pilot result nor a
substitute for the 500-target TEST.

## Provenance audit

The fail-closed audit is implemented by
`scripts/audit_phase55_cohort_provenance.py`.

| Check | Result |
|---|---:|
| Exact target-ID overlap with raw train/val | 0 |
| Exact canonical-structure overlap with raw train/val | 0 |
| Unparseable train/val products | 0 |
| Template provenance | TRAIN split |
| Frozen reranker provenance | TRAIN + VAL only |
| Source split | TEST |
| Audit eligibility under supplied evidence | `true` |

Scaffold overlap is reported as a distribution diagnostic, not an exclusion
rule: 166/424 unique cohort scaffolds overlap TRAIN and 95/424 overlap VAL.
This is expected to be possible for a chemically related benchmark and must be
reported with the final comparison rather than hidden.

## Remaining freeze gate

Before execution, verify that the frozen manifest contains the selected target
rows, all input hashes, the provenance-audit hash, tool revisions, common
resource budget, stock/template/model hashes, and the fixed execution order.
For the 50-target smoke, run `scripts/phase55_preflight.py` with the two
arm manifests and ledgers, plus this `frozen_manifest.json`, `--frozen-cohort`,
and `--smoke-size 50`. For the 500-target gate, run the ordinary paired
preflight after all rows are present. If any input changes, or if the
AiZynthFinder environment cannot be executed under the same contract, mark the
arm `not_measured`; do not substitute a historical arm.
