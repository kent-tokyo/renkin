---
title: "Offline Candidate-Pool Export and Reranker Training (RETROSPECT-Inspired)"
description: "How to export one-step retrosynthesis candidate pools with features for offline LambdaMART reranker training and evaluation."
---

# Candidate Pools and Reranker Training

RENKIN's route search always couples candidate *generation* (which rules
fire) with candidate *selection* (which result the search keeps). The
`candidate` and `pool_export` modules split those apart so a reranker can be
trained and evaluated offline, against a pool of candidates the runtime
search never had to narrow down first.

This is inspired by Pappala et al. (2026), "RETROSPECT: RETROsynthesis via
Sequential Prediction, and Chemically Transformed-ranking" (arXiv:2606.07181)
— see [`CITATION.cff`](https://github.com/kent-tokyo/renkin/blob/master/CITATION.cff)
for the citation. This is an independent RENKIN implementation; no upstream
source was copied, and no benchmark from that paper is reproduced here.

## What this is for

- Generating a **candidate pool**: for a target molecule, every one-step
  retrosynthetic candidate a chosen rule-selection mode would produce,
  each with a fixed-schema feature vector attached.
- Exporting that pool as JSONL plus a sidecar manifest, so a pool is
  self-describing (which rule-selection mode, which rules, which stock)
  and can't be silently trained on under one assumption and evaluated
  under another.
- Training and evaluating a LambdaMART (`LGBMRanker`) reranker against an
  exported pool, with leakage-safe, deterministic target-level splitting.

## What this is not (yet)

- **No candidate pool at any real scale has been generated.** Pool
  generation at 100/500/full-corpus scale is deliberately staged and is
  not part of this module — see the repo's own performance-gate history
  for why (heavy `apply_retro`/`run_reactants` computation is held back
  until it's fast enough to run at scale without silently degrading
  another benchmark).
- **No reranker has been formally trained or evaluated.** The training/
  evaluation script below is implemented and self-tested, but has not
  been run against a real corpus, and no offline-gate decision (whether a
  trained reranker actually improves route-search quality) has been made.
- **No runtime integration.** Nothing here wires a trained reranker into
  `find_routes`. `CandidateReranker` (the trait a future runtime reranker
  would implement) exists in `src/candidate.rs`, but nothing implements or
  calls it yet.

## Rule-selection modes (`ProposalMode`)

`propose_one_step` takes a `ProposalMode` that mirrors how the runtime
search itself would have narrowed the active rule set:

| Mode | Mirrors | Use |
|---|---|---|
| `Exhaustive` | nothing — every rule is tried | offline-only, maximum-coverage pool for evaluating a reranker's own selection ability |
| `BondIndexed { top_k }` | `--bond-index` retrieval | pool that matches bond-index-gated runtime search |
| `ScorerConditioned { input, top_k }` | an active NN template scorer | pool that matches scorer-gated runtime search, using a caller-supplied `ScorerConditionedInput` |

**Different modes produce different candidate *sets*, not just different
orderings of the same set.** A `ScorerConditioned` or `BondIndexed` pool has
already had rules filtered out before a reranker ever sees it. Evaluating a
reranker on an `Exhaustive` pool answers "how good is the reranker at
selection, given everything to select from" — it does not by itself show
that hooking the reranker into a scorer-gated runtime search would reproduce
that improvement offline. That would need a separate `ScorerConditioned`
evaluation. See `src/candidate.rs`'s module doc for the full reasoning.

`ScorerConditionedInput` (deliberately not gated behind the `nn-scoring`
feature -- this module never owns a `TemplateScorer`, so it only needs the
*shape* of a scorer's output) carries `scores` (`(rule_index, raw_logit,
rank)` per scored file template), `status`, `rules_offset` (hand-crafted
rules are `[0, rules_offset)` by *position*, never by a rule-name prefix),
`scorer_identity`, and `scorer_model_sha256`. `propose_one_step` fails
closed (`Err`) when `status != Available`, and validates every scored entry
(`rule_index` in bounds and non-duplicate, `rank` non-duplicate, `raw_logit`
finite) before using it -- a scorer failure or a corrupted scores payload
must never look identical to "the scorer succeeded and found nothing
relevant".

`propose_one_step` is a single-call convenience wrapper. For proposing
candidates across **many** targets against the same `rules` set (e.g. a
pool-generation run), build one `candidate::CandidateProposalContext`
instead: `CandidateProposalContext::new(&rules, prepare_bond_index)` builds
`BondIndexed`'s `TemplateBondIndex` once (it's a pure function of `rules`,
never of the target), then `ctx.propose_one_step(group_id, target_smiles,
&config)` reuses it per target instead of rebuilding it for every one.
`prepare_bond_index` must be `true` for any call that uses
`ProposalMode::BondIndexed` -- a context built with `false` that is then
asked to run `BondIndexed` proposal returns `Err`, never a silent fallback
to `Exhaustive`.

## Feature schema v1

`extract_features` computes a fixed-length, named feature vector
(`FEATURE_NAMES_V1`, `FEATURE_SCHEMA_VERSION = 1`) per candidate, split into
two groups:

- **Group 1** (`FEATURE_GROUP1_LEN = 14`, indices 0–13): structural
  (`num_precursors`, heavy-atom counts, `heavy_atom_retention_ratio` — a
  heavy-atom-count ratio, not the MW-based chemistry "atom economy" reported
  per route step elsewhere in RENKIN), chemistry-integrity
  (`net_charge_balanced`, `no_heavy_atom_gain`), and reaction-center /
  template-transformation features. Always attempted. `best_upstream_score`
  (index 13) is still group 1 but is legitimately `missing` under
  `Exhaustive`/`BondIndexed` mode, since no scorer is involved at all — that
  absence is mode-dependent, not a leakage concern.
- **Group 2**: stock-dependent availability (`fraction_precursors_in_stock`,
  `all_precursors_in_stock`) and template-frequency features
  (`max_template_log_frequency`, `mean_template_log_frequency`). Availability
  features are `missing` unless a `ChemEnv` stock is supplied to
  `extract_features`. The frequency features are **always** `missing` for
  now — `CandidateSource::template_log_frequency_raw` is not yet
  train-split-frozen, and treating it as a feature before that recomputation
  exists would be a leakage risk, not a convenience.

`missing[i] == true` must be treated as missing, not zero, by every
consumer — `pool_export`'s JSONL writer and `scripts/train_reranker.py`'s
loader both convert a missing feature to `NaN` rather than `0.0`.

## Exporting a pool

```rust
use renkin::candidate::{ProposalConfig, ProposalMode, index_rules_by_template_id, propose_one_step};
use renkin::chem_env::{default_rules, mol_from_smiles};
use renkin::pool_export::{
    PoolProvenance, build_manifest, candidate_rows_for_pool, target_pool_record_for_pool,
    write_jsonl, write_target_pool_jsonl,
};

let rules = default_rules();
let target = "CC(=O)c1ccccc1";
let target_mol = mol_from_smiles(target)?;
// `group_id` is the caller's dataset reaction/example id -- distinct from
// the canonical `target_id` the pool derives internally (see below).
let pool = propose_one_step("rxn-example-1", target, &rules, &ProposalConfig::default())?;
let templates_by_id = index_rules_by_template_id(&rules)?;

let rows = candidate_rows_for_pool(&pool, &target_mol, &templates_by_id, /* stock */ None);
let candidate_jsonl_sha256 = write_jsonl(&rows, std::fs::File::create("pool.jsonl")?)?;

let records = vec![target_pool_record_for_pool(&pool)];
let target_group_index_sha256 =
    write_target_pool_jsonl(&records, std::fs::File::create("pool.groups.jsonl")?)?;

let manifest = build_manifest(
    &rows,
    &candidate_jsonl_sha256,
    &records,
    &target_group_index_sha256,
    &rules,
    &ProposalConfig::default().mode,
    None,
    PoolProvenance {
        renkin_git_commit: "...".to_string(), // e.g. `git rev-parse HEAD` output
        cargo_lock_sha256: "...".to_string(),
        chematic_version: "...".to_string(),
        target_input_sha256: "...".to_string(), // hash of the driver's own target-list input
        stock_source: None,
        embedded_fallback_used: false,
        export_config: serde_json::json!({}),
    },
)?;
std::fs::write("pool.manifest.json", serde_json::to_string_pretty(&manifest)?)?;
```

Each JSONL line is one `CandidateRow`: `group_id` (the caller-supplied
dataset reaction/example id -- one LightGBM ranking group), `target_id` (the
canonical target structure -- the leakage-safe split key; two rows can share
`target_id` while having different `group_id`s), `target_smiles`,
`candidate_id`, `precursor_smiles`, `source_template_count`,
`best_upstream_rank`, `sources` (full per-rule provenance: `template_id`,
`rule_name`, `original_rank`, `upstream_score`, `upstream_score_status`,
`template_log_frequency_raw`, `base_step_cost` -- one entry per distinct
contributing rule, duplicates of the same rule already merged),
`feature_schema_version`, `feature_values`, `feature_missing`. Rows are
sorted by `candidate_id` before export, so two runs over the same input
produce byte-identical JSONL. `write_jsonl` hard-validates every row before
writing anything (matching feature-vector lengths, no non-finite non-missing
values, no duplicate `candidate_id` within one `group_id`, non-empty
`precursor_smiles`/`sources`) and returns the SHA-256 digest of exactly the
bytes it wrote.

Alongside the candidate JSONL, `pool_export::target_pool_record_for_pool`
(or `target_pool_record_for_failure` if `propose_one_step` returned `Err`)
builds one `TargetPoolRecord` per (`group_id`, target) attempt --
`group_id`, `target_id`, `target_smiles`, `candidate_count`,
`proposal_status` (`Ok` or `TargetParseFailed`) -- written with
`write_target_pool_jsonl`, which (like `write_jsonl`) rejects a duplicate
`group_id` and returns the digest of what it wrote. This group index exists
even for a target with zero candidates, so a consumer's coverage denominator
can be built from it plus labels, never by counting which `group_id`s happen
to appear in the candidate rows (a zero-candidate group would otherwise
silently vanish).

The manifest (`PoolManifest`, `MANIFEST_SCHEMA_VERSION = 2`) records
`feature_schema_version`, `feature_names`, `feature_schema_hash` (SHA-256
over the version + names, so a same-length rename/reorder is still
detectable), `proposal_mode` (mode + `top_k`, plus -- for
`ScorerConditioned` -- `rules_offset`/`scorer_identity`/
`scorer_model_sha256`/`scorer_status`), `rules_content_hash`
(order-independent SHA-256 over the rule set, including each rule's `name`
so a rename alone changes the hash), `rules_count`,
`stock_identity`/`stock_compound_count`/`stock_content_sha256` (`None` if no
stock was supplied -- `stock_content_sha256` hashes the stock's actual
compound content, so a swap under an unchanged `stock_identity` label is
still detectable), `target_count`/`group_count` (derived from, and
cross-validated against, the target/group index -- never taken unchecked
from caller input), `candidate_count`, `candidate_jsonl_sha256`/
`target_group_index_sha256` (must be the digests `write_jsonl`/
`write_target_pool_jsonl` actually returned, never independently
recomputed), and `provenance` (`PoolProvenance`: `renkin_git_commit`,
`cargo_lock_sha256`, `chematic_version`, `target_input_sha256`,
`stock_source`, `embedded_fallback_used`, `export_config` -- all
caller-supplied, since this crate has no way to derive git/build state or
its caller's own driver input itself; `PoolProvenance::default()` produces
obviously-placeholder values for local smoke tests, never anything that
could pass for real provenance). `build_manifest` itself now returns
`anyhow::Result<PoolManifest>`: it hard-validates that every `group_id` in
`rows` has a consistent entry in the target/group index before building
anything, so a mismatch between the two files is caught at manifest-build
time, not discovered by a downstream loader -- including that each group
index record's `candidate_count` matches the number of candidate rows
actually observed for that `group_id`, not just that the `group_id` exists.
(`MANIFEST_SCHEMA_VERSION` moved 1 -> 2 for the 5 new required fields above
plus the `rules_content_hash` algorithm change; both `pool_export.rs` and
`train_reranker.py` reject a manifest declaring the old version.)

## Training and evaluating a reranker

`scripts/train_reranker.py` is a standalone dev script (not declared in
`pyproject.toml`, run directly with `python3`), mirroring
`scripts/train_template_scorer.py`'s convention. It requires `lightgbm`
(`pip install lightgbm`), which is not a RENKIN dependency.

```bash
python3 scripts/train_reranker.py --self-test
```

is a fast (~1-2s), dependency-minimal smoke test — split determinism,
minimal manifest/row schema round-trip, labeling and missing-to-NaN,
`evaluate()`'s tie-break, and a tiny paired-bootstrap + gate PASS smoke, all
against an embedded synthetic fixture with no real data required; if
`lightgbm` is importable it also runs a minimal end-to-end train+evaluate
smoke. This is a code-path check, not a model-quality check — the synthetic
fixture is far too small to mean anything about ranking quality. It
deliberately does **not** carry detailed regression coverage; that lives in
`scripts/tests/` (below).

```bash
python3 scripts/train_reranker.py \
  --pool pool.jsonl --manifest pool.manifest.json \
  --groups pool.groups.jsonl --labels labels.jsonl \
  --model-out model.txt --eval-out eval.json
```

- `--manifest` is hard-validated before anything else runs (`validate_manifest`):
  `manifest_schema_version`/`feature_schema_version` must match this
  script's own constants, `feature_names` must exactly equal this script's
  `FEATURE_NAMES_V1` mirror, `feature_schema_hash` must match this script's
  recomputed hash (catching a Rust/Python schema drift a plain name/length
  comparison could miss), and `candidate_jsonl_sha256`/
  `target_group_index_sha256` must match the actual on-disk hashes of
  `--pool`/`--groups` -- a manifest paired with the wrong file is a hard
  error, not a warning.
- `--groups` is the JSONL group index (`write_target_pool_jsonl` output,
  above) -- one record per (`group_id`, target) attempt, including
  zero-candidate and parse-failure groups. The set of groups to consider
  always comes from this file, never from which `group_id`s happen to
  appear in `--pool`. Every `--pool` row is hard-validated against it
  (`validate_pool_rows`): matching feature-vector lengths (never a silently
  length-truncating `zip()`), non-finite values only where marked missing,
  non-empty `precursor_smiles`/`sources`, no duplicate `candidate_id` within
  one `group_id`, and `target_id`/`target_smiles` consistency with the
  group index entry for that `group_id`.
- `--labels` is JSONL, schema v1: `{"schema_version": 1, "group_id": ...,
  "target_id": ..., "correct_precursor_sets": [["...", "..."], ...]}` --
  multiple accepted correct precursor multisets per group are allowed, each
  supplied pre-sorted (matching the exporter's own convention; the script
  hard-errors on an unsorted entry, an empty `correct_precursor_sets` list,
  a non-v1 `schema_version`, or a duplicate `group_id` with conflicting
  data). A candidate is labeled positive iff its sorted `precursor_smiles`
  exactly matches any of its group's accepted sets.
- A group present in `--groups` but absent from `--labels` is a **hard
  error by default** -- never silently treated as "every candidate
  negative". Pass `--allow-unlabeled` to exclude such groups from
  training/evaluation instead; the excluded count is printed and reported
  as `unlabeled_group_count`, kept separate from the zero-positive coverage
  gap below.
- Splitting is by `target_id` (SHA-256 hash bucket, 0–100), never by
  `group_id` and never by candidate -- two groups sharing a `target_id`
  (e.g. two literature reactions producing the same product) always land in
  the same split. LightGBM's ranking "group" is `group_id`, so those two
  groups still form separate ranking groups.
- Every group's metrics are reported two ways: **conditional** (denominator
  is only groups with a positive candidate in their own pool — "given the
  answer is somewhere in the pool, did the ranker surface it") and
  **end-to-end** (denominator is every labeled group for the split; a
  coverage-miss group contributes 0 to every metric instead of being
  excluded, so a reranker cannot look better by ignoring groups it can't
  win). Both report `top1_hit_rate`, `top10_hit_rate`, `mean_reciprocal_rank`,
  `mean_ndcg10`, and `mean_best_positive_rank`. Coverage counts
  (`target_count`, `group_count`) are built from `--groups` + `--labels`,
  not inferred from `--pool`.
- If `manifest.proposal_mode.mode` isn't `"exhaustive"`, the script warns
  on stderr: training on a narrowed pool means the reranker never sees
  candidates outside that narrowing.

### `score_fn`: one scoring interface for the trained model and every baseline

Every arm — the trained LightGBM ranker and every deterministic baseline —
is scored through the same `score_fn(rows: list[LabeledRow]) -> list[float]`
interface and the same `evaluate()`/metrics code, so no arm can get a
different tie-break rule or a different metric definition than any other.
`evaluate()` hard-rejects a `score_fn` returning the wrong length or a
non-finite value; ties break deterministically on `(-score, candidate_id)`.

A row missing an arm's relevant feature scores `_MISSING_SENTINEL`
(a large-but-finite negative value), so it always ranks last rather than
producing NaN/Inf — `not_computable` status is reported separately, per
arm, when the relevant feature is absent for every row in a group.

Seven deterministic baseline arms (A–G) are always computable without
`lightgbm`: `original_rank` (upstream proposal order), `upstream_score`,
`template_frequency`, `upstream_plus_frequency` (rank fusion via
Borda-style summed rank, not raw score averaging — the two scales aren't
commensurable), `structural`, `reaction_center`, and `availability`. The
trained LightGBM ranker is arm H (`full_configured_model`), scored through
`lightgbm_score_fn()`.

`template_frequency` (and arm H's frequency features) come from
`fit_template_frequency()`, fit on **train-split rows only**: it counts how
often each `source_template_ids` entry is *proposed* across train rows,
regardless of that row's own label — this is deliberately "how often is
this template proposed", not "how often is it correct" (the latter would
leak label information into a supposedly-unsupervised feature).
`impute_frequency_features()` is a local, training-script-only step: it
returns new `LabeledRow` copies and never mutates the frozen exported
feature schema (features 16/17 stay `missing` in every exported row; see
`FEATURE_NAMES_V1`'s own doc).

LightGBM hyperparameters (`LIGHTGBM_HYPERPARAMETERS`, `lambdarank`
objective, fixed seed/threads/`deterministic=True`) and
`EARLY_STOPPING_ROUNDS` are pinned constants, not left at library defaults,
so a training run is reproducible run-to-run.

### Offline gate: paired bootstrap + PASS/FAIL

`--gate-baseline-arm`/`--gate-treatment-arm` (arm names from the list
above) run a paired bootstrap (`paired_bootstrap`, `--bootstrap-resamples`,
default 1000, `--bootstrap-seed`, default 1234) comparing two arms on
`--gate-split` (default `test`), writing the result to `--gate-out`.
Resampling is clustered at `target_id`, **never** at `group_id` alone: two
groups sharing a `target_id` always move together in a resample, matching
the same leakage-safe grouping the train/val/test split itself uses.

`run_offline_gate` judges **PASS** only when *all* of the following hold:
identical group coverage between the two arms (a structural assertion, not
a metric comparison — if this fails, the two arms weren't compared on the
same problem), top-1 hit-rate delta ≥ +1.0pp, MRR delta ≥ +0.01, top-10
regression capped at 0.2pp, and the top-1 delta's 95% CI lower bound > 0
(guards against an improvement that's just resampling noise). Any failing
check is named individually in the result, not just a bare FAIL.

### Test suite

`scripts/tests/` is a `unittest`-based suite (`__init__.py` +
`test_reranker_schema.py`, `test_reranker_labels.py`,
`test_reranker_metrics.py`, `test_reranker_baselines.py`,
`test_reranker_bootstrap.py`, `test_reranker_training.py`) that carries all
the detailed regression coverage `--self-test` intentionally does not (see
`--self-test`'s own doc below). LightGBM-dependent tests are isolated into
their own `@unittest.skipUnless(LIGHTGBM_AVAILABLE, ...)`-gated classes so
the rest of each file runs with no dependency and asserts training
code-path/artifact-field correctness only, never a model-quality claim (a
handful of synthetic groups mean nothing about ranking quality). Run with:

```bash
python3 -m unittest discover -s scripts/tests -p "test_*.py"
```

Wired into CI as the `reranker-tests` job in `.github/workflows/ci.yml`
alongside `--self-test`.

## Status

Implemented and unit/integration-tested: feature schema v1, JSONL
export + manifest, the training/evaluation script (conditional/end-to-end
metrics, baseline arms A–H, paired bootstrap, offline gate), and the
`scripts/tests/` suite. Not yet done: real-scale pool generation, an actual
training run, a real offline-gate decision, and runtime integration — see
[What this is not (yet)](#what-this-is-not-yet).
