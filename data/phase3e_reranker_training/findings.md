# Phase 3E: LightGBM Reranker Training, VAL Screening Gate, Model Freeze, Formal TEST (Issue #101 Task 35)

GO given after Phase 3D.5 (CASE A: existing TRAIN/VAL pools proceed unchanged).
Covers Steps 0-9 of the Task 35 specification. Yomitoki integration was not
touched. PR #104 untouched. The 4,903-target route-search benchmark was
**not** run -- Step 5 below is an offline candidate-*ranking* pool/evaluation
only.

## Step 1: protocol re-audit (no changes made)

Re-read `scripts/train_reranker.py` end to end (hyperparameters, gate
thresholds, baseline arms, leakage-safe split/bootstrap logic, CLI). No
correctness bug found -- `LIGHTGBM_HYPERPARAMETERS` and `GATE_THRESHOLDS`
match the pre-registered protocol exactly; nothing in this phase changed
them. One mechanical fact, not a bug: the CLI takes a single combined
`--pool`/`--groups`/`--labels`/`--split-manifest` set spanning all splits
(per-target split resolved via `--split-manifest`), so Phase 3D's separate
TRAIN/VAL full pools had to be concatenated before training could run at all
(Step 2, below).

## Step 2: combined TRAIN+VAL pool

`scripts/phase3e_build_combined_pool.py` -- pure byte concatenation
(TRAIN-then-VAL order) of Phase 3D's `pool_{train,val}_full.jsonl`,
`groups_{train,val}_full.jsonl`, the labels files, and Phase 3D.5's
`coverage_{train,val}_full.json.split_manifest_subset.jsonl`, plus a
recomputed manifest (`manifest_combined.json`, invariant fields copied
through after asserting they match between the two source manifests; only
the two SHA-256 fields and the three counts are recomputed). Hard asserts
(all passed): no `group_id` collision between train/val, no `target_id`
collision between the two split-manifest subsets.

| | train | val | combined |
|---|---|---|---|
| target_count | 39,668 | 4,924 | 44,592 |
| group_count | 39,927 | 4,931 | 44,858 |
| candidate rows | 1,095,467 | 135,641 | 1,231,108 |

Before running against the full combined pool, the same script + CLI
invocation was smoke-tested end-to-end against the Phase 3C 500-target
pools (mechanics only, not a decision-relevant result) -- confirmed
`validate_manifest`/`load_split_manifest` pass, and confirmed the CLI's
`for split in ("train","val","test")` loop does not crash when a split has
zero rows (the "test" split is legitimately empty at this stage; `evaluate`/
`aggregate_metrics`/`paired_bootstrap`/`evaluate_offline_gate` all handle an
empty group set via `None` propagation, not an exception).

## Step 3: VAL screening gate -- PASS

Trained on the full TRAIN split, evaluated on the full VAL split, gated
`original_rank` (primary baseline -- always computable under this project's
Exhaustive proposal mode; `upstream_score`/`availability` are both
`not_computable`, see `build_baseline_arms`'s own docstring) vs
`full_configured_model` (the trained LightGBM ranker) via
`--gate-split val`.

End-to-end (denominator = every labeled VAL group, 4,931; coverage miss
scores 0):

| metric | original_rank | full_configured_model | delta | threshold |
|---|---|---|---|---|
| top1_hit_rate | 17.22% | 28.90% | **+11.68pp** | ≥ +1.00pp |
| top10_hit_rate | 53.28% | 62.62% | **+9.35pp** | ≥ -0.20pp |
| mean_reciprocal_rank | 29.11% | 40.36% | **+11.25pp** | ≥ +1.00pp |

Paired bootstrap (cluster=target_id, n=1000, seed=1234): top1 delta 95% CI
`[0.1037, 0.1280]` -- lower bound positive. **Gate result: PASS** on all 5
checks (coverage_unchanged, top1 threshold, MRR threshold, top10 regression
threshold, CI lower bound positive). See `gate_val.json`, full report in
`eval_report.json`.

Coverage identical between arms (3,253/4,931 VAL groups have ≥1 positive
candidate in-pool -- 1,678 zero-positive groups, a candidate-generation
coverage gap neither arm can fix, unchanged from Phase 3D/3D.5's own
accounting).

## Step 4: model freeze

Full freeze manifest: `freeze_manifest.json`. Frozen the moment the VAL gate
PASSed, before any TEST-pool generation. Key facts: LightGBM 4.7.0,
`best_iteration_=199` (of `n_estimators=200` -- direct inspection of
`model.txt` shows exactly 199 `Tree=` blocks, i.e. early stopping DID fire:
round 200 was boosted but didn't improve VAL ndcg within the patience=20
window, so the sklearn API truncated the saved booster to the best 199
trees; corrected here from an earlier "never triggered" note -- does not
change any gate result, both gates evaluated this same 199-tree file
throughout), `random_state=42`, model SHA-256
`7e0b5a1ef1d119eb8451235cde734790f21ee7e1413b11a82cc6b3b521c3b85b`, feature
schema hash `756404c59bbee9a65e194f92df3530e1b801028f333e01c67214917977061df1`
(18 features, `max`/`mean_template_log_frequency` post-hoc imputed from a
TRAIN-frozen frequency table, SHA-256
`12c3023fbbc3f85811c8454760538dd0a3f93670ff011b0f2de1ad7685552ffa`).

## Step 5: formal TEST candidate pool generation (first time ever)

`renkin-pool-gen` against the quarantined 4,903-target TEST group file
(`data/reranker_groups_uspto50k_test.jsonl`), same Exhaustive proposal
configuration as Phase 3D (500 rules, no `--limit`). This is an offline
candidate-*ranking* pool -- **not** the prohibited 4,903-target route-search
benchmark.

| | count |
|---|---|
| groups requested | 4,903 |
| parse failures | 0 |
| target_id mismatches (mechanical exclusion, Phase 3D.5's confirmed policy, unchanged) | 13 |
| zero-candidate groups | 4 |
| candidate rows | 134,499 |
| wall clock | 651s |

The 13 target_id-mismatch count **exactly matches** Phase 3D.5 Step 2's
corpus-wide string-only prediction for the TEST corpus (13/4,903, 0.265%) --
independent confirmation that the earlier string-level audit correctly
predicted candidate-generation-time behavior, not just a canonicalization
curiosity.

## Step 6: formal TEST evaluation -- FORMAL GATE PASS

`scripts/phase3e_evaluate_formal_test.py` applied the **frozen** model
(loaded from `model.txt`, no retraining) to the formal TEST pool exactly
once, gated against `original_rank` on the identical pool, same
`GATE_THRESHOLDS` as Step 3. Full result: `formal_test_result.json`.

End-to-end (denominator = all 4,903 labeled TEST groups):

| metric | original_rank | full_configured_model | delta | threshold |
|---|---|---|---|---|
| top1_hit_rate | 16.40% | 29.13% | **+12.72pp** | ≥ +1.00pp |
| top10_hit_rate | 53.99% | 63.08% | **+9.08pp** | ≥ -0.20pp |
| mean_reciprocal_rank | 28.62% | 40.49% | **+11.87pp** | ≥ +1.00pp |

Paired bootstrap (cluster=target_id, n=1000, seed=1234): top1 delta 95% CI
`[0.1142, 0.1401]` -- lower bound positive. **Gate result: PASS** on all 5
checks. Magnitude closely matches the VAL screening gate (Step 3) -- no
sign of VAL-specific overfitting.

Conditional (only the 3,285 groups with ≥1 positive candidate in-pool):
top1 24.47% -> 43.47%, top10 80.58% -> 94.16%, MRR 42.71% -> 60.44%, mean
best-positive-rank 6.42 -> 3.32 (roughly halved). Coverage identical between
arms (1,618 zero-positive TEST groups, matching `n_groups_zero_candidates`
+ the 13 target_id-mismatch groups + genuine candidate-generation misses).

## Step 7: error taxonomy (4,903 TEST groups)

| class | definition | count |
|---|---|---|
| A | zero positive candidate in pool | 1,618 |
| B | positive present, baseline already rank 1 | 804 |
| C | positive present, reranker win (better rank than baseline) | **1,742** |
| D | positive present, reranker regression (worse rank) | **416** |
| E | positive present, both wrong, rank unchanged | 323 |

Net C-D = **+1,326** groups. Total rank-position improvement across C =
12,327 (mean 7.08 positions/group); total rank-position regression across D
= 1,473 (mean 3.54 positions/group). Win:loss ratio ~4.2:1 by count, ~8.4:1
by total rank-positions moved.

## Step 8: determinism / reproducibility

- **Inference (8a)**: frozen model's `booster.predict()` called twice on
  the identical formal-TEST feature matrix -- bit-for-bit identical both
  times (`formal_test_result.json.inference_determinism`).
- **Frequency-table refit integrity**: `fit_template_frequency` recomputed
  from the original TRAIN pool alone (a required step for TEST feature
  imputation) reproduced the exact frozen SHA-256
  (`12c3023f...`) -- confirms the frozen table was not accidentally
  order-dependent or corrupted.
- **Training reproducibility (8b)**: re-ran the identical
  `train_reranker.py` command (same pool/manifest/groups/labels/
  split-manifest/hyperparameters/seed) into a separate throwaway output
  directory (not used for anything else, deleted after comparison). Result:
  `model.txt`, `eval_report.json`, and `gate_val.json` were all
  **byte-for-byte identical** to the frozen artifacts -- no
  prediction-equivalence fallback needed (`deterministic=True`,
  `num_threads=1`, fixed `random_state=42` reproduce exactly, as designed).

## Step 9: decision

**FORMAL OFFLINE RERANKER GATE: PASS.** Every fixed threshold cleared on
both VAL (screening) and formal TEST (held-out, quarantined,
never-before-touched), with matching effect size and a large, favorably
skewed C/D taxonomy. Per Task 35's scope, **no runtime integration was done
in this task.** A future PR should propose runtime ordering-only integration
under these preconditions: candidate set unchanged, no top-K drop,
ordering-only (never filters candidates the search wouldn't otherwise
produce), fallback to legacy ordering on any model-load failure, model
identity/provenance surfaced explicitly (SHA-256 + this freeze manifest),
confirmed via a fixed route-search gate before merge.

## What was explicitly not done, per scope

- No Yomitoki integration.
- No changes to PR #104.
- No 4,903-target route-search benchmark (only the offline candidate-ranking
  pool/evaluation above).
- No runtime reranker integration.
- No Ready-flip or merge of PR #105 (still draft).
- No tag/release, no Issue #101 post.
- No hyperparameter/threshold/feature change at any point (Step 1's audited
  protocol ran unmodified start to finish).
