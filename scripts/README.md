# Script map

`scripts/` contains reproducible data preparation, benchmark, and offline
model-training tools. It is intentionally not a second product API. Run the
CLI, Python package, MCP server, or WASM module for normal use.

## Start with these entry points

| Task | Entry point | Output contract |
| --- | --- | --- |
| Check public documentation facts | CI invokes `check_docs_facts.py` with live `doc_facts` values | Fails on stale version/rule/stock facts |
| Validate an O7 public-procedure receipt | `validate_o7_operational_procedure.py` | New output directory containing hashes and re-audit receipt |
| Preflight a Phase 55 run | `phase55_preflight.py` | Fail-closed configuration and input identity check |
| Freeze a Phase 55 cohort/candidate | `freeze_phase55_cohort.py`, `freeze_phase55_candidate.py` | Immutable manifest and hashes |
| Run a matched planner arm | `compare_run.py` | Per-target rows, aggregate, and manifest |
| Verify and compare completed arms | `compare_verify_arm.py`, `compare_paired_report.py` | Complete-row and paired statistics report |
| Train optional model artifacts | `train_template_scorer.py`, `train_reranker.py` | Offline artifacts; never required by the default planner |

Use `python3 scripts/<name>.py --help` before executing a tool. Benchmark
commands require a fresh run identifier and must preserve raw rows and
manifests; see `docs/benchmark/` for the corresponding protocol.

## Why some scripts remain separate

The similarly named comparison modules have different durability boundaries:
`compare_run.py` is the formal per-target runner, `compare_renkin_batch.py`
is a shared-process throughput arm, and `compare_renkin_parallel_batch.py`
measures multi-process throughput. They must not be merged because the latter
two cannot supply the isolated resource/latency evidence required by the
formal runner.

Historical migration and diagnostic scripts are kept where tracked artifacts
or tests still reference them. Do not delete a script merely because it is not
an everyday entry point: first remove or migrate its test, artifact, and
protocol references in the same change.

Generated files such as `__pycache__/` are ignored and should never be
committed.
