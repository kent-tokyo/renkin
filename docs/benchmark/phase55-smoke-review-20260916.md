# Phase 55 smoke review — 2026-09-16

## Decision

Both 50-target arms completed and the existing prefix/manifest preflight
passed. This closes the legacy-runner execution check only. Phase 55.0's
formal comparison contract remains open; do not promote this run to the
planned independent TEST or use it to select a search candidate.

Measured revision: `5afd82952f00a534186b28e9835c5420f28b1140` (v1.0.7 plus
unreleased work). Both manifests recorded a clean worktree and unchanged
enumerated input files. Docker was usable during this run.

## Local evidence and time correction

Artifacts currently reside at `/private/tmp/renkin-phase55-smoke-20260915/`.
They are local temporary evidence, not a published reproducibility bundle.
Before cleanup, archive the rows, manifests, aggregates and their hashes to a
durable run directory. The original files must remain unmodified.

| Measure | RENKIN | AiZynthFinder |
|---|---:|---:|
| Completed rows | 50 | 50 |
| Native route found | 4 | 1 |
| Combined strict (route found + validator + stock) | 4 | 1 |
| Timeout / crash | 0 / 0 | 0 / 0 |
| Sum of all row `total_elapsed_ms`, seconds | 64.570 | 348.724 |
| Last invocation wall time, seconds | 64.672 | 210.521 |
| New rows in last invocation | 50 | 26 |

The earlier statement that AiZynthFinder's total time was 210.52 seconds was
incorrect: `compare_run.py` resets its invocation clock on resume. That value
describes the final 26 rows only. Summing the 50 row timings gives 348.724
seconds, which is not end-to-end wall time and excludes interruption downtime.
Each tool's timing includes a fresh process/container startup; this is not the
planned warm-worker speed comparison. RENKIN's manifest also records unrelated
Sekirei CPU activity, so matched host load has not been established.

Row-file SHA-256:

- `renkin.rows.jsonl`: `a38bdb5c7cf3f9bd52c2895316df01c53425c564d20a1a0fea1fe9a5c0f1e855`
- `aizynthfinder.rows.jsonl`: `e20120330f3d4494b3c218f2f4e747e9ff7308275fca31c8876736b5c01f4ecd`

## Contract gaps to close before formal execution

| Boundary | Observed smoke behavior | Required next work |
|---|---|---|
| Stock / search candidate | 393-line shared stock, RENKIN 500-template input, depth 5 / beam 100, standard mode, rank1 | Freeze a representative common stock identity set and the VAL-selected candidate; this smoke did not test recovery |
| Resources | RENKIN runs on macOS with `RLIMIT_AS_unavailable`; AiZynthFinder uses Docker CPU/memory limits | `phase55_preflight.py --require-effective-resource-enforcement` now rejects an arm without recorded effective CPU/RAM enforcement. The current native macOS RENKIN arm therefore remains development-only; run RENKIN under a cgroup/container or equivalent bounded environment before formal execution |
| Actual AiZynthFinder settings | The prior YAML relied on defaults | Tracked mode-specific templates now explicitly fix `max_transforms=5`, `iteration_limit=100`, `time_limit=120`, `return_first=false`, and top-5 extraction. Verify intended cross-tool budget semantics in the registered protocol |
| Input provenance | `manifest_input_files` hashes the supplied `.smi` and RENKIN templates for both arms | AiZynthFinder now hashes its mounted config plus every config-referenced HDF5/ONNX/template/filter asset, and rejects a config differing from the tracked template. Stock conversion identity still needs a dedicated equivalence check |
| Preflight scope | Checks prefix IDs, selected input hashes, revision, clean state and timeout/grace/max-routes fields | Add checks for effective configuration, resource enforcement and actual per-arm assets; templates/models may legitimately differ between tools |
| Timing / resume | Fresh process per target; final sweep time was last invocation only; run required manual continuation | `compare_run.py` now records each normally completed invocation in the manifest and reports its durable cumulative wall time separately from this invocation. An externally killed invocation remains an explicit unrecorded gap; startup/search/audit timing and supervised continuation still need protocol coverage |
| Independent TEST | Results for the first 50 of frozen 500 targets were viewed before final candidate selection | Preserve the original cohort and record exposure; register subsequent evaluation population and sample-size/power decision before measuring it |

Implementation references: `scripts/compare_run.py`,
`scripts/compare_renkin_adapter.py`, `scripts/compare_aizynthfinder_adapter.py`,
`scripts/four_tool_resources.py`, `scripts/phase55_preflight.py`.

## Next sequence

1. Validate the new invocation ledger and close the remaining measurement gaps on
   development targets. The local Phase 55 plan proposes a large common union,
   warm workers, 120 seconds and top-5; these are proposed values until an
   executable two-tool protocol is registered.
2. Evaluate the existing recovery candidate against baseline on VAL under one
   total budget. Retain only improvements with no lost baseline successes,
   non-worsening strict results and acceptable resource use. Freeze one arm.
3. Register the independent evaluation population, exposure exclusions and
   sample size using development-derived power assumptions. Preserve the old
   500-target artifact; any revised protocol/cohort gets a new identity. Do not
   silently replace it with 450 remaining targets or enlarge N after inspecting
   formal results. Complete paired analysis and a reproducible report even if
   superiority is not established.

O7 operational validation and v1.0.8 candidate gates can proceed independently.
This review updates planning only; it does not change the engine or start a run.
