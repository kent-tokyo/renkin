# Phase 31.7 — Corrected-baseline USPTO-50k re-measurement (provenance)

Started 2026-07-20 20:42:16 JST. Full census, restarted from scratch per user
instruction (the earlier ~25%-complete attempt was discarded, no partial
output existed on disk to resume from).

## Commit / environment

| | |
|---|---|
| renkin commit (binary under test) | `35f26cb3db09204d0cffedfec362a3700f16f3b6` (master) — PR #25 + #26 + #27 merged, full CI/Docs/Security-Audit green on this exact SHA |
| harness scripts commit | `bench/corrected-baseline-runner` branch, commit `23e54bb` (PR #28, open) — `run_benchmark_chunks.sh` (`--plausibility` passthrough) + `run_benchmark_parallel.sh` (new) |
| Cargo.lock sha256 | `a80138e86c56293b0cb03dad11aaa9dc165b072b972c394bdde4a23ecdd80d96` |
| rustc | 1.97.0 (2d8144b78 2026-07-07) |
| chematic | 0.4.25 |
| renkin | 0.15.5 |
| CPU / OS | Apple M4, 10 cores / macOS 26.5.2 (Darwin 25.5.0) |
| build | `cargo build --release` (default opt-level=3) |

## Command / config

```
bash scripts/run_benchmark_parallel.sh \
    data/uspto50k_test.smi \
    data/templates_extracted_5000.smi \
    data/bench_chunks_corrected_baseline \
    5 100 5 2 \
    data/building_blocks.smi
```

| | |
|---|---|
| depth | 5 |
| beam-width | 100 |
| max-routes | 1 (per `run_benchmark_chunks.sh`) |
| templates | `data/templates_extracted_5000.smi` (sha256 `517f6a084921141b6080c3827c75e6c51ac148455218695dee6e9712e3731517`) |
| building blocks | `data/building_blocks.smi`, 475 non-comment entries (sha256 `6fb4550dbc29480427ef4331dc492f0f66a315776b32bf1a6ab7057c6f1521dd`) — **not** the ~160-entry embedded `DEFAULT_BUILDING_BLOCKS`; matches the historical 78% run's actual config per `phase0_baseline.json`, despite `docs/benchmark.md`'s reproduction snippet omitting the flag (known doc drift) |
| targets | `data/uspto50k_test.smi`, 4,907 targets (sha256 `ac246998c6e7b2c68904eaf030e4c6d8c72f0c35334f86e55100d504586fff3d`) |
| `--plausibility` | on (atom-balance + forward validation, needed for `strict_validated_solved_rate` / `validation_coverage` / `evaluable_validation_pass_rate` / `pct_atom_balanced`) |
| sharding | 5-way round-robin (`NR % 5`), `RAYON_NUM_THREADS=2` per shard |

## Known gaps in this pass (surfaced, not silently dropped)

- **p50/p95/p99 latency**: not an emitted JSON field on this binary — computed via post-processing over each target's `results[].time_ms` across all shard chunk files. No code change, no baseline taint.
- **Peak RSS**: not in the JSON either (the historical figure came from now-stashed uncommitted instrumentation). Captured per-shard (not per-target) via `/usr/bin/time -l` wrapping each shard's process in `run_benchmark_parallel.sh` (`data/bench_chunks_corrected_baseline/shard_*.rss.txt`) — coarser granularity than the original, and macOS `time -l`'s rusage accounting for a script that forks many short-lived `renkin-bench` children is not fully verified to aggregate correctly. Treat as best-effort, not authoritative.
- **Best-route rule-usage distribution / first-invalid-rule distribution**: NOT available from `renkin-bench`'s per-target JSON — it only carries route-level rollup flags, not the step list. Per user decision, this will be computed via a *separate, sampled* (n≈300) secondary pass using the single-target `renkin` CLI (`--format json`, which does emit `steps[].rule`) on a random subset of solved targets, run after this full census completes. Deterministic search means the reconstructed best route for a given target/config is identical to what `renkin-bench` found (spot-checked once during scoping).

## Corrections applied mid-setup (for the record)

1. Schema-verified on a 22-target smoke run *before* committing to the full run: confirmed `--plausibility` emits all the strict/coverage/atom-balance fields needed; confirmed `p50/p95/p99` are absent as fields but reconstructable from per-target `time_ms`.
2. `run_benchmark_parallel.sh` had a job-control bug (backgrounding inside a function called via command substitution forked a subshell, breaking `wait`) — fixed before the real run, verified via a 47-target dry run whose aggregate matched the input count exactly.
3. First full-run launch (started 20:22:53 JST) used the ~160-entry embedded default building-block set instead of `data/building_blocks.smi` (475 entries) — the actual historical config. Caught and killed ~90s in, before meaningful compute was wasted. Restarted 20:42:16 JST with the correct `--building-blocks` path.

## Status

Complete. Finished ~00:11–00:16 JST 2026-07-21 (per-shard `real` time in `shard_*.rss.txt`: 11740–12527s ≈ 3.26–3.48h). 50/50 chunks, 0 failed chunks, 0 repair passes needed. Peak RSS per shard (best-effort, see caveat above): 82–149 MB.

## Results (n=4,907, exact — see aggregation method below)

All six numbers below are **true measurements of commit `35f26cb`** — no hedging on the arithmetic. What's hedged is what they *mean* (see "Validator contamination" section). Aggregated directly from all 4,907 per-target `results[]` records (not by averaging 50 chunk-level percentages, which would mis-weight uneven chunk sizes) — script: see aggregation logic below, cross-checked to ~1e-15 reconstruction error.

| Metric | Value | Status |
|---|---|---|
| `raw_solved_rate` | **1,178 / 4,907 = 24.01%** | CONFIRMED, but provisional (see below) |
| `depth=0` direct stock hit | 2 / 4,907 = 0.04% | CONFIRMED |
| `pct_atom_balanced` (of 1,178 solved) | 58.32% (687/1,178) | CONFIRMED |
| `validation_coverage` (step-level) | 100.0000% (3,461/3,461 steps evaluable) | CONFIRMED — Finding A's blind spot (7 graph rules previously NotEvaluable) is fully closed |
| latency (ms) | p50=8,206 / p95=37,295 / p99=79,860 / max=279,002 / mean=12,416 | CONFIRMED |
| `strict_validated_solved_rate` | 49 / 4,907 = 0.9986% | VALIDATOR-STATE-ONLY — do not publish as a correctness rate |
| `evaluable_validation_pass_rate` | 701 / 3,461 = 20.25% | VALIDATOR-STATE-ONLY — do not publish as a correctness rate |
| `route_validation_status` (of 1,178 solved) | 1,129 invalid, 49 validated, 0 partially_validated, 0 not_evaluable | CONFIRMED (as validator output) |

**24.0% is the corrected baseline *of this commit*, not RENKIN's settled number** — see the halide-rule bug below. Fixing it would very likely drop raw_solved_rate further (same mechanism as the aryl_carboxylation_retro fix: 199/200→61/200). Publishing 24.0% now and finding it drops again next week is the exact churn Phase 31 exists to end. Recommend leaving public docs on the current "under re-evaluation" footing rather than committing this table publicly this session — flagged to the user to decide.

## Why `strict_validated_solved_rate` / `evaluable_validation_pass_rate` are not correctness rates

Investigated via `examples/inspect_validation.rs` (new, ad hoc, not part of any measured binary — reconstructs a target's best route with the same config and prints per-step `rule` + `StepValidationStatus` + `step_balanced`). Ran on a random n=300 sample of solved targets (seed=42) plus all 49 `route_validation_status=validated` targets.

**Finding 1 — three hand-crafted rules fabricate/destroy a halogen atom with no companion reagent** (same bug class as the already-fixed `aryl_carboxylation_retro`, structural pattern: `[c:1][X]>>[c:1]`, i.e. retro drops a heavy atom from the product side with nothing on the precursor side to account for it):

| Rule | SMIRKS | n in sample | Invalid | atom-imbalanced |
|---|---|---|---|---|
| `aryl_fluoride_snAr_retro` | `[c:1][F]>>[c:1]` | 36 | 36 (100%) | 36 (100%) |
| `aryl_iodide_retro` | `[c:1][I]>>[c:1]` | 8 | 8 (100%) | 8 (100%) |
| `aryl_chloride_retro` | `[c:1][Cl]>>[c:1]` | 37 | 27 (73%) | structurally the same bug — target (with halogen) always heavier than precursor (plain Ar-H); the 10 "Valid" instances are cross-corroboration false positives (see Finding 3), not evidence the rule is sound |

Phase 31.5's manual SMIRKS audit ("found none [beyond aryl_carboxylation_retro]") missed these — the mechanical pattern to scan for is "product-side MW cannot exceed reactant-side MW for a single-fragment retro step with no tracked reagent." **Not fixed this session — flagged in `tasks/todo.md` for a future PR, deliberately, to avoid re-invalidating this baseline the same day it was established.**

**Finding 2 — the bulk of `Invalid` verdicts (575/695 = 83% in the n=300 sample) are atom-*balanced*, real-vs-artifact status unresolved.** Cross-tab (n=300 sample, 886 steps): `{Invalid+balanced: 575, Invalid+imbalanced: 120, Valid+balanced: 164, Valid+imbalanced: 27}`. Rules like `cc_single_cleavage` (90.7% invalid, mostly balanced), `aryl_amine_retro` (74.2% invalid, mostly balanced), `cn_aliphatic_cleavage` (90% invalid, all balanced), and several `extracted_N` rdchiral templates (100% invalid, balanced) fail the exact reverse-SMIRKS-reproduction check despite the mass balancing — could be genuine regiochemistry/connectivity errors, or the reverse-SMIRKS canonical-SMILES string-equality check false-negativing on canonicalization/aromaticity/stereo/tautomer mismatches (a known failure mode of this validation method). By contrast `suzuki_retro` is 22/22 (100%) clean. **Deliberately not resolved further — this is exactly the scope of the planned `accuracy/rule-provenance-validation` PR, not this session's task.**

**Finding 3 — `Valid` verdicts are also contaminated, by design.** `forward::smirks_reproduces()` tries *every* rule's SMIRKS, not just the one the step actually used — a step is marked `Valid` if *any* unrelated rule's reverse-SMIRKS happens to reproduce the same target/precursor pair. Confirmed empirically: of the 49 `route_validation_status=validated` routes (71 steps total), **7 steps use `aryl_chloride_retro` and are atom-imbalanced** (i.e. structurally the same bug as Finding 1) yet are marked `Valid` — some other rule's SMIRKS coincidentally reproduced the target. This means the contamination reaches the headline 0.9986% "fully validated" figure directly: at least 7 of the 49 routes contain a demonstrably-impossible step that passed only by accident. This is precisely the "AlternateRuleCorroborated vs. actually-used-rule-validity" distinction the tri-state design doesn't yet separate — again the `accuracy/rule-provenance-validation` PR's scope, not this session's.

**Net effect**: `evaluable_validation_pass_rate` (20.25%) and `strict_validated_solved_rate` (1.00%) are neither an upper nor a lower bound on RENKIN's actual chemistry correctness — they are a property of a validator with known false-negatives (Finding 2) *and* known false-positives (Finding 3) simultaneously. Record and describe them only as "what the current validator confirms," never as a correctness rate.

## Rule-usage distribution (n=300 solved-target sample, 886 total steps)

Top rules by occurrence, with per-rule Valid/Invalid/NotEvaluable counts — see `examples/inspect_validation.rs` output, archived at `/private/tmp/claude-501/.../scratchpad/sample300_inspect2.tsv` (session-local scratch, not committed). Re-run command:
```
./target/release/examples/inspect_validation < <(sample of solved SMILES) > out.tsv
```
Full per-rule table not reproduced here for space — see session transcript. Headline: `suzuki_retro` 0% invalid (22/22); atom-dropping halide rules 100% invalid; most `extracted_N` templates and several hand-crafted rules (`cc_single_cleavage`, `aryl_amine_retro`, `cn_aliphatic_cleavage`) show 70-100% invalid despite atom balance holding.
