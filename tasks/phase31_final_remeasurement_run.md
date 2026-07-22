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

Started 2026-07-21T23:24:12Z, finished 2026-07-22T02:19:36Z (wall clock
≈2h55m; per-shard `real` time 9,500–10,363s ≈ 2.64–2.88h per
`shard_*.rss.txt`). Pre-run dry-run (50 targets, depth=3 for speed) on
this exact commit completed successfully end-to-end (0 failed chunks,
`aggregate_bench_results.py --expected-total 50` passed) before launching
the full run. 50/50 chunks, **0 failed chunks, 0 repair passes needed**
(no `WARN: renkin-bench failed` in any `shard_*.log`). Peak RSS per shard:
98–164 MB.

## Results (n=4,907, exact — `scripts/aggregate_bench_results.py`)

Reconciliation: `sum(chunk.total) == len(results[]) == 4,907` (script
hard-asserts this; ran with `--expected-total 4907`, exit 0).

| Metric | Value |
|---|---|
| `raw_solved_rate` | 986 / 4,907 = **20.09%** |
| `depth=0` direct stock hit | 2 / 4,907 = 0.04% |
| `pct_atom_balanced` (of 986 solved) | 76.67% (756/986) |
| **`provenance_validated_solved_rate`** (primary metric — solved AND atom-balanced AND every step `Valid` via its own originating rule) | 43 / 4,907 = **0.88%** |
| `route_validation_status` (of 986 solved) | 942 invalid, 44 validated, 0 partially_validated, 0 not_evaluable |
| `validation_coverage` (step-level) | 100.0% uniformly across all 50 chunks |
| `evaluable_validation_pass_rate` | non-uniform across chunks (range 8.0%–38.9%) — **not exactly reconstructable as a single weighted figure** from the current JSON schema (known gap, see `scripts/aggregate_bench_results.py`'s docstring); do not average the range into a point estimate |
| latency, all targets (ms) | p50=7,338 / p95=28,228 / p99=51,165 / max=181,053 / mean=10,175 |
| latency, solved only (ms) | p50=977 / p95=9,362 / p99=15,596 / max=37,889 / mean=2,309 |
| solved-route depth distribution | 0:2, 1:102, 2:324, 3:277, 4:179, 5:102 |

Note: 44 routes are `route_validation_status=validated` but only 43 are
also `atom_balance_ok` — one `validated` route fails the coarse MW-based
atom-balance check despite every step passing its own rule's structural/
SMIRKS-reversal validator. Not investigated further (single-instance edge
case, doesn't change any published figure meaningfully); flagged here
rather than silently dropped.

## Comparison to the provisional pre-fix measurement

| | Provisional (`35f26cb`, pre-31.11/31.12) | This run (`e20dc8c`, post-31.11/31.12) |
|---|---|---|
| `raw_solved_rate` | 24.01% (1,178/4,907) | **20.09%** (986/4,907) |
| `pct_atom_balanced` of solved | 58.32% | 76.67% |
| `route_validation_status=validated` of solved | 49/1,178 (4.2%) | 44/986 (4.5%) — but now provenance-bound, not cross-rule-contaminated |
| `evaluable_validation_pass_rate` | 20.25% (single figure — later found to itself be a schema-averaging artifact) | non-uniform per-chunk range 8.0–38.9%, reported as a range rather than a misleading point estimate |

The further raw-rate drop (24.01% → 20.09%) is the expected, predicted
consequence of removing the three atom-loss halide rules (31.11) — the
same pattern as the earlier `aryl_carboxylation_retro` fix (199/200 →
61/200 in that isolated n=200 sample). It is **not** a regression in
search capability; it is fake "solved" routes (chlorobenzene → benzene
with the Cl silently discarded, etc.) no longer being counted. The rise
in `pct_atom_balanced` (58.32% → 76.67%) corroborates this directly.

## Rule-usage / per-rule validation breakdown (n=300 solved-target sample, seed=42)

Sampled 300 of the 986 solved targets (Python `random.seed(42)`, same
methodology as the provisional run) and ran
`examples/inspect_validation` (same search config as the benchmark:
depth=5, beam=100, max-routes=1) — reconstructs each target's best route
deterministically and reports per-step `rule` + `StepValidationStatus` +
atom-balance. 300 routes, 864 steps, 167 distinct rules used.

Route-level (this sample): 290 Invalid, 10 Validated, 0 PartiallyValidated,
0 NotEvaluable (10/300 = 3.3%, in the same ballpark as the full-population
44/986 = 4.5% — sampling variance on a small validated-route count).

**Confirms 31.11 took effect**: zero steps use `aryl_chloride_retro`,
`aryl_iodide_retro`, or `aryl_fluoride_snAr_retro` (all three fully absent
from `default_rules()`, as expected).

**Confirms 31.12 took effect and did not introduce new `NotEvaluable`
noise**: `NotEvaluable` count is 0 for every one of the 167 rules used —
every step's `step.rule` name resolves to an actual rule in the combined
default+extracted rule set, so provenance-binding never degrades to "can't
evaluate" in practice at this rule-pool size.

Step-level atom-balance × validation cross-tab (864 steps): Invalid+balanced
624 (72.2%), Invalid+imbalanced 60 (6.9%), Valid+balanced 166 (19.2%),
Valid+imbalanced 14 (1.6%). The Valid+imbalanced 14 are expected, not a
regression: several graph-based rules (e.g. `boc_deprotection_retro`,
discarding a volatile isobutylene/CO2 byproduct) are intentionally
single-fragment and fail the coarse MW-only `step_balanced()` check by
design — documented in `src/validation/atom_conservation.rs`'s module
doc comment, unrelated to 31.11/31.12.

Top rules by usage (≥10 occurrences), Valid/Invalid/NotEvaluable:

| Rule | Uses | Valid | Invalid | Invalid % |
|---|---|---|---|---|
| `cc_single_cleavage` | 117 | 9 | 108 | 92.3% |
| `aryl_amine_retro` | 52 | 11 | 41 | 78.8% |
| `cn_aliphatic_cleavage` | 41 | 1 | 40 | 97.6% |
| `co_aliphatic_cleavage` | 37 | 8 | 29 | 78.4% |
| `suzuki_retro` | 28 | 28 | 0 | **0.0%** |
| `aryl_carboxylation_retro` | 25 | 7 | 18 | 72.0% |
| `extracted_51` | 22 | 0 | 22 | 100.0% |
| `negishi_retro` | 21 | 14 | 7 | 33.3% |
| `extracted_64` | 21 | 19 | 2 | 9.5% |
| `boc_deprotection_retro` | 20 | 6 | 14 | 70.0% |
| `extracted_30` | 18 | 0 | 18 | 100.0% |
| `extracted_12` | 16 | 1 | 15 | 93.8% |

**Not resolved by this PR or this measurement** (explicitly out of scope,
matches `tasks/todo.md` 31.12's still-open second half): whether the bulk
of `Invalid`-but-atom-balanced verdicts (624/864 = 72.2% of all steps in
this sample) reflect real chemical errors in those rules/templates, or
false negatives in the reverse-SMIRKS-reproduction check itself (e.g. a
`chematic` canonicalization quirk already found and documented in PR #33's
description: a freshly-installed bracket atom gets `hydrogen_count =
Some(0)` while a parsed target's bare atom gets `hydrogen_count = None`,
so `canonical_smiles` never matches even for a chemically-correct pair —
confirmed for `aryl_chloride_to_bromide`'s own reversal, not investigated
beyond that one rule). `suzuki_retro` at 0% invalid vs. e.g.
`cn_aliphatic_cleavage` at 97.6% invalid is a wide enough spread that both
explanations (some rules genuinely wrong, some false-negative from the
validator) are plausible and likely coexist. Recommended as the next
Phase-31-adjacent investigation, not undertaken here.
