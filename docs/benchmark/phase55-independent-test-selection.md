# Phase 55.6 independent TEST selection

更新: 2026-09-16

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
- Selection status: provenance audit passed; versioned cohort preserved; first 50 results now observed; formal eligibility pending

The candidate and audit were generated in a local temporary directory so they
are not accidentally treated as a formal result. A local freeze rehearsal
passed with freeze ID `phase55-test-candidate-20260912-003`. The versioned
frozen manifest is now at
`data/comparison/aizynthfinder_accuracy_phase55/phase55-test-candidate-20260912-003/frozen_manifest.json`
with SHA-256
`9b9c91ed85fe68b7122f18d41ab0ef1a81061b274656021bf5774d0fc34daf0a`.
The adjacent `sample_list.jsonl` has 500 rows and SHA-256
`2e0f53a3b3dbfd388341e4a220946e912ca16359deb9e7d0005c7650863e7067`.
Both artifacts pass saved-manifest verification. The cohort-level preflight in
`scripts/phase55_preflight.py` also passes (`eligible=true`) for the frozen
manifest: 500 IDs, matching sample-list hash, and matching ID set.

The comparison runner also locks each output ledger before reading or writing
it. A resumed run must reject an already-corrupted duplicate-target ledger or a
concurrent writer; such an artifact is never eligible for formal statistics.

On September 15–16, both tools ran the first 50 rows of this frozen list
with the legacy small-stock runner. `scripts/phase55_preflight.py` accepts
`--frozen-cohort` and `--smoke-size 50`; it rejects an arm pair unless both
ledgers are exactly that deterministic prefix and both manifests hash the full
frozen `sample_list.jsonl`. That check passed, but it does not verify all
formal requirements, notably effective CPU/RAM limits, actual tool assets and
resolved search settings. See the [smoke review](phase55-smoke-review-20260916.md)
for timing corrections and the contract gaps.

The first 50 results were viewed before final candidate selection. Preserve
this original manifest and sample list; do not describe all 500 as unobserved.
Before a new confirmatory run, register how this exposure is handled and
justify N with a power analysis. Any revised cohort/protocol must have a new
identity and explicit provenance; neither the remaining 450 nor a replacement
500 is automatically eligible. Further adapter smoke checks use development
targets. Candidate tuning uses VAL only.

`scripts/phase55_mcnemar_power.py` provides the required outcome-blind
sample-size calculation. It accepts only development-derived probabilities of
the two discordant outcomes and enumerates the exact two-sided McNemar test
used by the final report. It does not accept TEST rows. After one candidate
configuration is frozen, record its VAL-derived assumptions, alpha, target
power, N cap and selected N in the separate 55.6 execution protocol, then
create and freeze a new cohort excluding every exposed identity. Do not select
N from a TEST result or silently reuse the unobserved remainder of freeze-003.

Before that protocol is created, the selected development configuration must
be frozen with `scripts/freeze_phase55_candidate.py`. It accepts only finalized
baseline/candidate manifests, exact same-cohort row ledgers, and a passing
non-displacing verification (including both recovery and process budgets).
Its output is explicitly `development_only`; it is an input to TEST protocol
registration, not evidence from TEST.

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

The existing frozen artifact establishes cohort selection, not a complete
execution protocol. Before formal execution, close Phase 55.0 and register the
selected target rows, exposure decision, N, provenance, tool revisions, actual
input/config hashes, resource enforcement, output/time boundaries and execution
order in a separate protocol. Verify full expected coverage after completion;
matching two arm ID sets alone does not prove all planned targets were measured.
If any input changes, or if the
AiZynthFinder environment cannot be executed under the same contract, mark the
arm `not_measured`; do not substitute a historical arm.
