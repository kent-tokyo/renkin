# Phase 31 final corrected-baseline USPTO-50k re-measurement (provenance)

Run against `master` **after both 31.11 (halide atom-loss rule removal, PR #31)
and 31.12 (rule-provenance validator fix, PR #33) merged** — per Phase 31's
explicit rule not to re-measure between the two fixes. This supersedes the
provisional `tasks/phase31_corrected_baseline_run.md` (24.01% raw,
commit `35f26cb`, deliberately never published as the corrected baseline
because it predates both fixes).

## Commit / environment

| | |
|---|---|
| renkin commit (binary under test) | `e20dc8ccca6cac10484dd20b3191a2cbfafefb93` (master) — PR #30 (record integrity) + PR #31 (31.11 halide fix) + PR #32 (aggregation script) + PR #33 (31.12 validator fix) merged |
| working tree | clean (`git status --porcelain` empty at build time) |
| rustc | 1.97.0 (2d8144b78 2026-07-07) |
| chematic | 0.4.25 |
| renkin | 0.15.5 |
| OS | macOS 26.5.2 (Darwin 25.5.0, arm64) |
| CPU | Apple M4, 10 cores |
| build | `cargo build --release` (default opt-level=3) |
| handcrafted rule count | 28 (was 31 pre-31.11; 3 atom-loss halide rules removed) |

## Corpus / config

| | |
|---|---|
| depth | 5 |
| beam-width | 100 |
| max-routes | 1 (`run_benchmark_chunks.sh` default) |
| templates | `data/templates_extracted_5000.smi` — sha256 `517f6a084921141b6080c3827c75e6c51ac148455218695dee6e9712e3731517`, 5,000 entries |
| building blocks | `data/building_blocks.smi` — sha256 `6fb4550dbc29480427ef4331dc492f0f66a315776b32bf1a6ab7057c6f1521dd`, 475 entries |
| targets | `data/uspto50k_test.smi` — sha256 `ac246998c6e7b2c68904eaf030e4c6d8c72f0c35334f86e55100d504586fff3d`, 4,907 targets |
| `--plausibility` | on (hardcoded by `run_benchmark_parallel.sh`, atom-balance + rule-provenance-bound forward validation) |
| sharding | 5-way round-robin (`NR % 5`), `RAYON_NUM_THREADS=2` per shard |
| random seed | N/A — search is deterministic (no RNG in `src/search.rs`/`src/chem_env.rs`) |
| aggregation script | `scripts/aggregate_bench_results.py` (added PR #32) at commit `e20dc8c` |

## Command

```
bash scripts/run_benchmark_parallel.sh \
    data/uspto50k_test.smi \
    data/templates_extracted_5000.smi \
    data/bench_chunks_phase31_final_e20dc8c \
    5 100 5 2 \
    data/building_blocks.smi
```

## Status

Started 2026-07-21 23:24:12Z. Pre-run dry-run (50 targets, depth=3 for
speed) on this exact commit completed successfully end-to-end (0 failed
chunks, `aggregate_bench_results.py --expected-total 50` passed) before
launching the full run.

<!-- Results, retried chunks, failed targets, and end timestamp appended after completion. -->
