# Phase A.5 -- Template-Count Scaling Diagnosis (direct candidate-pool measurement)

Candidate-generation coverage program, direct follow-up to Phase A
(`data/chemical_space_coverage_diagnosis/findings.md`), authorized by the
user 2026-08-11 after Phase A's finding that whole-molecule nearest-TRAIN
similarity cannot discriminate a Phase B (template-diversity scaling)
rationale from a Phase C (higher-level templates) rationale.

**Question** (fixed before any results were seen): does increasing
TRAIN-derived template count 500->1,000->2,000->5,000->10,000 (nested by
construction) reduce the one-step candidate-pool zero-positive rate on
VAL?

## Scope, fixed in advance

- **One-step candidate-pool generation only** (`renkin-pool-gen`). No
  route search, no reranker, no beam, no stock, no Yomitoki integration.
  No ranking/top1 metrics -- this measures whether the *right candidate
  exists in the pool at all*, not how well it's ranked.
- **VAL corpus, not formal TEST.** Formal TEST has already been read three
  times (the reranker's own formal gate, Phase A, and Phase A review);
  further B/C exploration directly on it would erode its value as a
  held-out competitive benchmark. Sequence: TRAIN/VAL template-count
  scaling -> B/C direction freeze -> one final fixed-TEST confirmation,
  not iterative TEST probing.
- **Nested template sets.** All 5 template files extracted fresh from the
  same USPTO-50k TRAIN split with the SAME script invocation (see
  "Provenance" -- the originally-committed 500/1,000/5,000 files predate a
  header-comment wording change in `extract_templates.py`; re-extracted
  from scratch here rather than mixed with the old files, to guarantee
  strict nesting is not just assumed but verified). Confirmed strict
  subset relation 500 ⊂ 1,000 ⊂ 2,000 ⊂ 5,000 ⊂ 9,979 (10,000 requested;
  USPTO-50k TRAIN's simplified-template vocabulary has ~9,979 distinct
  entries at frequency >=1, so that is the natural ceiling, not a bug).
- **Pre-registered decision thresholds** (500->10,000 zero-positive
  absolute improvement, fixed before results were seen -- see
  `scripts/phase_a5_report.py`, not adjusted post-hoc):
  - >=10pp: Phase B strong GO
  - 5-10pp: Phase B GO (then examine efficiency/dedup for implementation design)
  - 3-5pp: ambiguous -- needs Phase C comparison
  - <3pp: simple Phase B template-count scaling rejected -- prioritize Phase C
  - Saturation-curve shape (still improving at 10k vs. plateaued earlier)
    matters as much as the endpoint delta -- read alongside the threshold,
    not instead of it.
- **Staged execution** (100-target smoke -> 500-target resource check ->
  full VAL), each stage checked for anomalies (candidate explosion,
  extreme runtime) before proceeding to the next.

## Provenance

- USPTO-50k TRAIN/VAL splits dumped from the local HF cache
  (`bisectgroup/USPTO_50K`, pinned revision
  `08a575f0546b2be57242997fd45f684d6814d5a9`) -- VAL arrow source verified
  byte-for-byte against `data/phase3a_reranker_ground_truth_audit/
  findings.md`'s documented SHA-256.
- `scripts/generate_benchmark_quarantine_manifest.py` (unmodified)
  regenerated the formal-TEST quarantine identity list (4,903 targets,
  matching the known corpus size exactly) so VAL/TRAIN decontamination
  against formal TEST leakage is enforced, matching the original reranker
  pipeline's own hygiene rule.
- `scripts/generate_train_val_labels.py` (unmodified) regenerated VAL
  labels/groups: 4,924 distinct target_ids, 4,931 labeled groups -- exact
  match to the historical `data/phase3d_full_pool/manifest_val_full.json`
  (`target_count`/`group_count`).
- Five nested template files re-extracted fresh via
  `scripts/extract_templates.py --split train --top {500,1000,2000,5000,10000}`
  (unmodified script; the pre-existing committed `templates_extracted_{500,1000,5000}.smi`
  were NOT reused, since they predate a header-comment change in the
  current script version and mixing old+new risked an unverified nesting
  assumption -- regenerating all 5 from the identical current invocation
  guarantees it, and the strict-subset check above confirms it held).
- **Independent full-pipeline validation**: the 500-template arm's full-VAL
  `renkin-pool-gen` run produced `candidate_jsonl_sha256`/
  `target_group_index_sha256` values that **exactly match** the
  already-committed `data/phase3d_full_pool/manifest_val_full.json`
  (`sha256:2770047e...`/`sha256:bb93a9b4...`) -- the historical pool used
  by the original reranker training pipeline. This confirms the entire
  regeneration chain (raw split dump -> quarantine -> labels/groups ->
  templates -> pool-gen) reproduces byte-for-byte, not approximately.

## Method notes

- **Metrics** (`scripts/phase_a5_pool_metrics.py`, reuses
  `scripts/train_reranker.py`'s functions unmodified for pool/label
  loading and group-level coverage -- no new labeling logic):
  - `zero_positive_rate` / `positive_present_rate` -- group-level (VAL's
    schema has one group per historical reaction, not collapsed per
    target the way formal TEST is).
  - `ground_truth_precursor_recall_target_level` -- target-level, handles
    the rare (~0.14%) multi-route targets properly: fraction of a
    target's own distinct correct precursor sets (pooled across all of
    that target's group_ids) found anywhere in that target's candidate
    pool. Differs from `positive_present_rate` only for those multi-route
    targets.
  - `dedup_rate` -- `1 - (unique merged candidates / sum of each row's
    source_template_count)`, i.e. how much raw per-template-application
    redundancy `candidate.rs`'s existing merge step already collapses.
    Computed from the pool JSONL's existing `source_template_count`
    field -- no new RENKIN-side instrumentation needed.
  - Candidate cardinality percentiles, generation latency, and
    parse/zero-candidate/target-id-mismatch counts come directly from
    `renkin-pool-gen`'s own stdout summary, unmodified.
  - **Known gap, not measured**: "rule/template attempts" (which
    templates were *tried*, success or not) has no existing
    instrumentation anywhere in the candidate-generation path -- only
    templates that produced a surviving candidate are visible (via each
    row's `sources`). Adding this would mean instrumenting
    `candidate.rs`/`propose_one_step` itself, which was deliberately
    avoided to keep this diagnostic's blast radius on RENKIN core at
    zero, matching Phase A's precedent. Not fatal to the primary
    question (`zero_positive_rate`), just not available as a secondary
    metric here.
- **Chunked execution for the 5k/10k full-VAL arms**: the unchunked
  single-shot run was killed by the environment twice in a row (once
  after ~1,000/4,931 groups, once after ~3,000/4,931) despite showing no
  application-level error -- likely an environment-level limit on very
  long-running background processes, not a bug in the pipeline. Switched
  to `scripts/phase_a5_run_arm_chunked.sh`: the VAL groups file split into
  ten 500-group chunks (a scale already proven reliable by Stage 2, which
  ran every arm including 10,000 templates as one 500-group shot), run
  independently, concatenated, then fed through the same metrics script.
  Percentiles are recomputed from the concatenated pool (not averaged
  across chunks' own percentiles, which wouldn't combine validly); counts
  and wall-clock seconds sum exactly.

## Results

### Stage 1: 100-target smoke test

| templates | zero-positive | dedup rate | recall | candidates | p50 / p95 | wall-clock |
|---|---|---|---|---|---|---|
| 500 | 36.0% | 18.7% | 63.6% | 2,590 | 25 / 49 | 9.6s |
| 1,000 | 26.0% | 20.0% | 73.7% | 4,107 | 40 / 71 | 22.4s |
| 2,000 | 24.0% | 25.6% | 75.8% | 4,937 | 48 / 83 | 48.1s |
| 5,000 | 20.0% | 29.4% | 79.8% | 6,447 | 66 / 110 | 159.4s |
| 10,000 | 20.0% | 33.0% | 79.8% | 8,627 | 89 / 147 | 311.2s |

No anomalies (0 zero-candidate groups, 0 target-id mismatches at any
arm). Apparent plateau 5,000->10,000 -- at n=100 this is not distinguishable
from sampling noise, and Stage 2 below shows it wasn't real.

### Stage 2: 500-target resource check

| templates | zero-positive | dedup rate | recall | candidates | p50 / p95 | wall-clock |
|---|---|---|---|---|---|---|
| 500 | 35.4% | 19.5% | 64.5% | 13,643 | 25 / 50 | 48.2s |
| 1,000 | 28.2% | 20.4% | 71.7% | 21,392 | 40 / 79 | 92.6s |
| 2,000 | 23.6% | 25.8% | 76.4% | 25,724 | 48 / 91 | 191.5s |
| 5,000 | 19.2% | 29.5% | 80.8% | 33,282 | 61 / 120 | 715.1s |
| 10,000 | 17.4% | 32.6% | 82.6% | 44,017 | 80 / 156 | 1,467.2s |

No anomalies. At this larger, less noisy sample, the improvement is
**monotonic and still meaningful at the 5,000->10,000 step** (19.2% ->
17.4%, -1.8pp) -- Stage 1's apparent plateau there was noise, not signal.
Candidate cardinality grows sub-linearly with template count (no
explosion); dedup rate climbs steadily (more templates -> more redundant
rediscovery of the same candidates, expected).

### Stage 3: full VAL (4,931 groups)

**[IN PROGRESS -- filled in once all 5 arms complete]**

<!-- PHASE_A5_STAGE3_RESULTS_PLACEHOLDER -->

## Interpretation

**[TODO once Stage 3 completes -- apply the pre-registered thresholds
above to the 500->10,000 absolute zero-positive improvement, read the
saturation-curve shape, and report the verdict mechanically. Do not adjust
the thresholds after seeing the result.]**

## Recommendation

**[TODO once Stage 3 completes.]**
