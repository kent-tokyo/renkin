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
| `ScorerConditioned { scores, top_k }` | an active NN template scorer | pool that matches scorer-gated runtime search, using caller-supplied scores |

**Different modes produce different candidate *sets*, not just different
orderings of the same set.** A `ScorerConditioned` or `BondIndexed` pool has
already had rules filtered out before a reranker ever sees it. Evaluating a
reranker on an `Exhaustive` pool answers "how good is the reranker at
selection, given everything to select from" — it does not by itself show
that hooking the reranker into a scorer-gated runtime search would reproduce
that improvement offline. That would need a separate `ScorerConditioned`
evaluation. See `src/candidate.rs`'s module doc for the full reasoning.

## Feature schema v1

`extract_features` computes a fixed-length, named feature vector
(`FEATURE_NAMES_V1`, `FEATURE_SCHEMA_VERSION = 1`) per candidate, split into
two groups:

- **Group 1** (`FEATURE_GROUP1_LEN = 14`, indices 0–13): structural
  (`num_precursors`, heavy-atom counts, `atom_economy`), chemistry-integrity
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
  now — `CandidateSource::template_log_frequency` is not yet train-split-frozen,
  and treating it as a feature before that recomputation exists would be a
  leakage risk, not a convenience.

`missing[i] == true` must be treated as missing, not zero, by every
consumer — `pool_export`'s JSONL writer and `scripts/train_reranker.py`'s
loader both convert a missing feature to `NaN` rather than `0.0`.

## Exporting a pool

```rust
use renkin::candidate::{ProposalConfig, ProposalMode, index_rules_by_template_id, propose_one_step};
use renkin::chem_env::{default_rules, mol_from_smiles};
use renkin::pool_export::{build_manifest, candidate_rows_for_pool, write_jsonl};

let rules = default_rules();
let target = "CC(=O)c1ccccc1";
let target_mol = mol_from_smiles(target)?;
let pool = propose_one_step(target, &rules, &ProposalConfig::default())?;
let templates_by_id = index_rules_by_template_id(&rules);

let rows = candidate_rows_for_pool(&pool, &target_mol, &templates_by_id, /* stock */ None);
write_jsonl(&rows, std::fs::File::create("pool.jsonl")?)?;

let manifest = build_manifest(&rows, /* target_count */ 1, &rules, &ProposalConfig::default().mode, None);
std::fs::write("pool.manifest.json", serde_json::to_string_pretty(&manifest)?)?;
```

Each JSONL line is one `CandidateRow`: `target_id`, `target_smiles`,
`candidate_id`, `precursor_smiles`, `source_template_count`,
`best_upstream_rank`, `feature_schema_version`, `feature_values`,
`feature_missing`. Rows are sorted by `candidate_id` before export, so two
runs over the same input produce byte-identical JSONL.

The manifest (`PoolManifest`, `MANIFEST_SCHEMA_VERSION = 1`) records
`feature_schema_version`, `feature_names`, `proposal_mode` (mode + `top_k`),
`rules_content_hash` (order-independent SHA-256 over the rule set),
`rules_count`, `stock_identity`/`stock_compound_count` (`None` if no stock
was supplied), `target_count`, and `candidate_count` — enough for a
downstream consumer to detect a mode/stock/rules mismatch instead of
silently training on the wrong assumption.

## Training and evaluating a reranker

`scripts/train_reranker.py` is a standalone dev script (not declared in
`pyproject.toml`, run directly with `python3`), mirroring
`scripts/train_template_scorer.py`'s convention. It requires `lightgbm`
(`pip install lightgbm`), which is not a RENKIN dependency.

```bash
python3 scripts/train_reranker.py --self-test
```

runs the deterministic split/label/group/tie-break logic against an
embedded synthetic fixture, with no real data or `lightgbm` required; if
`lightgbm` is importable it also runs a tiny end-to-end smoke pass. This is
a code-path check, not a model-quality check — the synthetic fixture is far
too small to mean anything about ranking quality.

```bash
python3 scripts/train_reranker.py \
  --pool pool.jsonl --manifest pool.manifest.json \
  --labels labels.jsonl \
  --model-out model.txt --eval-out eval.json
```

- `--labels` is JSONL of `{"target_id": ..., "correct_precursor_smiles": [...]}`.
  A candidate is labeled positive iff its sorted `precursor_smiles` exactly
  matches the labeled target's sorted correct set. A target absent from
  `--labels` gets label 0 for all its candidates (not skipped).
- Target-level train/val/test splitting is by a SHA-256 hash bucket of
  `target_id` (0–100), never by candidate — so no candidate from a target
  can leak across splits.
- The evaluation report separates **coverage** (`targets_with_zero_positive_in_pool`
  — a target with no positive candidate in its own pool, which no reranker
  can fix) from **ranking quality** (`top1_hit_rate`, `mean_reciprocal_rank`,
  computed only over targets that do have a positive candidate).
- If `manifest.proposal_mode.mode` isn't `"exhaustive"`, the script warns
  on stderr: training on a narrowed pool means the reranker never sees
  candidates outside that narrowing.

## Status

Implemented and unit/integration-tested: feature schema v1, JSONL
export + manifest, and this training/evaluation script. Not yet done:
real-scale pool generation, an actual training run, an offline-gate
decision, and runtime integration — see [What this is not (yet)](#what-this-is-not-yet).
