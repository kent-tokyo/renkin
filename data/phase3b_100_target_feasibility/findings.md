# Phase 3B: 100-target candidate-pool feasibility

Pipeline/resource feasibility only, per the program's explicit rule --
**the top-1/MRR numbers below are not a formal accuracy claim** (that's
Phase 3D-3G, on the full quarantine-respecting corpus, only after 500-target
feasibility (Phase 3C) is also GO).

## Driver

No CLI/binary existed for candidate-pool generation before this round --
`src/pool_export.rs`'s own module doc says that's deliberately a driver's
job, kept out of the crate. New: `src/bin/pool_gen.rs`
(`renkin-pool-gen`), registered in `Cargo.toml`. Reads a `{group_id,
target_id}` group list (never a labels file -- proposal/label separation
holds at the file level, not just in-process), runs
`propose_one_step(Exhaustive)` per group via one shared
`CandidateProposalContext`, and writes exactly what `pool_export.rs`
defines: candidate JSONL, group/target-index JSONL, `PoolManifest`.

## A real bug this staged feasibility check caught before it could contaminate anything

`propose_one_step` sets `CandidatePool.target_id` to
`to_canonical(&target_mol)` **unconditionally** -- verified directly
against the function, not assumed. Every labels row generated in Phase
3A/3A-Round-2 had used a human-readable identifier
(`uspto50k_test#L3855`, or a content hash for train/val) as `target_id`
instead. This is exactly the kind of thing "confirm export -> label ->
train_reranker.py runs end-to-end" exists to catch: it would have failed
`train_reranker.py::label_and_split_rows`'s group-index cross-check on
**every single row**, the first time a real pool was ever exported. Fixed
in a separate commit (`375d6a1`) before this round's pool-generation run:
`target_id` is now the RENKIN-canonical target SMILES itself in all three
label corpora; `group_id` keeps the human-readable identifier. Both
`generate_real_labels.py` and `generate_train_val_labels.py` gained a
`--groups-output` (test) / `--{train,val}-groups-output` companion file --
`{group_id, target_id}` pairs only, no `correct_precursor_sets` -- as the
driver's actual input.

## 100-target run (formal test quarantine corpus)

First 100 groups of `data/reranker_groups_uspto50k_test.jsonl` (input-file
order, so this is deterministic and reproducible -- not a random sample of
the 4,903).

| metric | value |
|---|---|
| n targets / n groups | 100 / 100 (1:1 for this corpus -- see Round 2's Section D note on why train/val differ) |
| groups with zero candidates | 0 |
| groups with zero positive candidate | 33 (67/100 have >=1 positive) |
| candidates/group p50 / p90 / p95 / max | 28 / 47 / 52 / 66 |
| total candidate rows | 2,913 |
| pool file size | 2.8M |
| wall clock | 9.2s (100 targets) |
| peak RSS | 12.7 MB (`/usr/bin/time -l`, `maximum resident set size`) |

Provenance (`data/phase3b_100_target_feasibility/manifest_test_100.json`):
`renkin_git_commit=375d6a1`, `cargo_lock_sha256`, `chematic_version=0.11.0`
(parsed out of `Cargo.lock`, not renkin's own crate version --
`PoolProvenance` exists specifically to prevent that kind of
mislabeling), `rules_content_hash` (500 rules), `target_input_sha256`
(hash of the groups file itself), `candidate_jsonl_sha256`,
`target_group_index_sha256`. Both hashes are of the exact bytes written
(`write_jsonl`/`write_target_pool_jsonl`'s own return values), not
independently recomputed.

## Confirmed (all required checks)

- **export -> label -> train_reranker.py end-to-end**: `validate_manifest`,
  `validate_pool_rows`, and `label_and_split_rows` all ran against the real
  script on this real output. 2,913/2,913 pool rows labeled, **0 unlabeled
  groups** (every group_id in the pool found its match in the labels file
  -- this is the check the target_id bug above would have failed).
- **schema validation**: `PoolManifest` schema v2, `feature_schema_hash`
  matches (18 features), all cross-checks in `validate_manifest` pass.
- **no target leakage**: this 100-target sample is drawn from the already
  fully-quarantined `reranker_groups_uspto50k_test.jsonl` (Round 2's
  quarantine + decontamination already guarantees zero overlap with
  train/val at the source; nothing in pool generation itself can
  reintroduce it, since it only reads target_id/produces candidates).
- **no duplicate group corruption**: `write_target_pool_jsonl` hard-checks
  duplicate `group_id`; `write_jsonl` hard-checks duplicate `candidate_id`
  within a group. Both passed (0 errors).
- **zero-positive groups not silently dropped**: 33/100 groups have zero
  positive candidates in their pool and all 33 are still present in the
  group index and in `label_and_split_rows`'s output (`groups_with_zero_
  positive`-style accounting, confirmed via `summarize_coverage`) -- a
  real, disclosed coverage gap, not lost data.
- **deterministic repeat**: re-ran the identical 100-target command; both
  `candidate_jsonl_sha256` and `target_group_index_sha256` identical, `diff`
  of both output files empty.

## Cross-corpus smoke check (train/val, 30 groups each)

Not the formal test corpus, but confirms the driver and the
train/val-specific group-id design (Round 2 Section D: multiple groups can
share one target_id) also work end-to-end:

| split | groups | pool rows | unlabeled groups | positive rows |
|---|---|---|---|---|
| train | 30 | 870 | 0 | 18 |
| val | 30 | 683 | 0 | 21 |

## Not done this round (per Phase 3B's explicit scope)

500-target feasibility (Phase 3C) and full-corpus generation (Phase 3D) --
separate go/no-go checkpoints the user controls, not implied by this
100-target result. No baseline arms, no LightGBM training, no formal gate.
PR #104 untouched, still draft.
