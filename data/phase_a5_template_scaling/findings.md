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

### Stage 3: full VAL (4,931 groups) -- final

| templates (actual) | zero-positive | positive-present | recall | dedup rate | candidates | p50 / p95 | wall-clock |
|---|---|---|---|---|---|---|---|
| 500 | 34.0% | 66.0% | 66.0% | 18.6% | 135,641 | 26 / 51 | 418s |
| 1,000 | 27.7% | 72.3% | 72.3% | 19.7% | 213,232 | 41 / 79 | 804s |
| 2,000 | 23.4% | 76.6% | 76.6% | 24.8% | 256,048 | 49 / 93 | 1,855s |
| 5,000 | 19.8% | 80.2% | 80.2% | 28.6% | 329,263 | 64 / 121 | 6,214s |
| 9,979 | 17.6% | 82.4% | 82.4% | 31.6% | 434,912 | 84 / 159 | 11,604s |

No anomalies at any arm: `n_groups_zero_candidates` 1/0/0/0/0,
`n_groups_target_id_mismatch` 14/14/14/14/14 (identical across every arm
-- confirms these are target-canonicalization edge cases independent of
template-set size, not something the scaling introduced),
`n_groups_parse_failed` 0 throughout. Full machine-readable output:
`data/phase_a5_template_scaling/full_val/summary.json`.

**Saturation curve** (successive-arm deltas): 500->1,000 **+6.35pp**,
1,000->2,000 **+4.30pp**, 2,000->5,000 **+3.57pp**, 5,000->9,979
**+2.23pp**. Decelerating (expected -- diminishing returns from an
ever-larger, ever-more-specific template pool) but **still clearly
positive at the last step**, not flattened out. No sign of the "saturates
by 2k, residual gap needs different template abstraction" pattern the
original proposal flagged as the concerning alternative.

**Primary result: 500->9,979 templates, zero-positive rate 34.0% ->
17.6%, an absolute improvement of 16.4 percentage points.**

## Interpretation

Applying the pre-registered thresholds (fixed before this run, not
adjusted after seeing the result):

> \>=10pp: **Phase B strong GO**

16.4pp clears this decisively -- it is not a borderline call sitting near
a threshold boundary. Template-diversity scaling is not a weak or
ambiguous lever here: on VAL, simply extending the same frequency-ranked
extraction from 500 to ~10,000 templates cuts the zero-positive rate by
essentially half (34.0% -> 17.6%), with candidate cardinality growing
sub-linearly (p50 26->84, a 3.2x increase for a 20x template increase)
and no zero-candidate or parse-failure pathologies introduced at any
scale.

This directly resolves the ambiguity Phase A left open (see
`data/chemical_space_coverage_diagnosis/findings.md`'s "What this does
and does not support"): Phase A's whole-molecule-Tanimoto metric could
rule out a *distance*-based Phase B rationale but could not distinguish
"more templates would find the right local pattern" from "no template
count helps without a different template shape." Phase A.5 measured the
actual mechanism directly and it works -- the local
reaction-center/disconnection-pattern-coverage mechanism Phase A's review
round identified as the more plausible path for Phase B is exactly what
this result is consistent with.

The saturation curve's still-positive final delta (+2.23pp at the
5,000->9,979 step, the smallest but not negligible) shows the
coverage-return curve had not flattened out by the time it hit 9,979 --
but 9,979 is not an arbitrarily-chosen stopping point on an open-ended
curve. It is USPTO-50k TRAIN's actual distinct-simplified-template
vocabulary ceiling under the current extraction method (see Provenance
above): `--top 10,000` returned every template that method can produce
from this corpus, no more exist to request. The correct reading is
**"the coverage-return curve was still improving at the point it hit
the current TRAIN x extraction-method's vocabulary ceiling, not that
returns had been exhausted before that ceiling was reached."** Whether
there is headroom *beyond* that ceiling is a different, unanswered
question -- it would require a different corpus (more/different TRAIN
reactions) or a different template-abstraction method (rdchiral's
current simplification is not the only way to derive templates), either
of which is a separate experiment, not a rerun of this one with a larger
`--top`.

dedup rate climbing steadily (18.6% -> 31.6%) confirms candidate.rs's
existing merge step is doing real, increasing work as the template pool
grows (more templates rediscovering the same candidate via different
disconnection routes) -- an efficiency signal for implementation design,
not a correctness concern.

## Recommendation

Two separate verdicts, deliberately not collapsed into one:

**Scientific verdict: Phase B strong GO.** Template-diversity scaling, as
a *mechanism*, is confirmed to work and to work strongly (16.4pp
absolute, clears the pre-registered >=10pp bar with room to spare). This
is the answer to the question Phase A.5 was designed to measure, and it
is settled -- don't re-run or re-litigate it.

**Production verdict: NOT decided.** Coverage improving does not by
itself mean shipping 9,979 templates as the new default is the right
call. Generation cost scales far faster than coverage benefit:

| | 500 templates | 9,979 templates | ratio |
|---|---|---|---|
| candidates/group (p50) | 26 | 84 | **3.2x** (sub-linear) |
| generation wall-clock, full VAL | 418s | 11,604s | ~27.8x |
| generation wall-clock, 500-target (single-shot, directly comparable) | 48.2s | 1,467.2s | ~30.4x |

(The full-VAL ratio is from summed per-chunk wall-clock time across the
9,979-template arm's 17 chunks, not one continuous single-shot run --
see "Chunked execution" above -- so the 500-target row, where both sides
are single, uninterrupted `renkin-pool-gen` invocations, is the cleaner
apples-to-apples comparison; both land in the same ~28-30x range
regardless.)

Candidate cardinality staying sub-linear means "no candidate explosion"
holds, but **that is a correctness/memory observation, not a cost
one** -- generation latency growing ~9x faster than candidate count
means most of that 500->9,979 wall-clock increase is match-time (more
templates attempted per target, most producing nothing), not
downstream merge/write volume. Whether that cost is acceptable in
production route search (called per search-tree node, not once per
target the way this diagnosis ran it) is unmeasured and cannot be waved
away by the coverage result alone.

**Next: Phase B.1 -- coverage/cost frontier optimization (recorded as a
candidate, not started).** Not a move to Phase C -- Phase B's mechanism
is confirmed. The open question is *which* template count to actually
ship, chosen by coverage-gain-per-compute-unit rather than "biggest
tested size wins":

- Compute coverage gain per unit of generation cost at each already-
  measured point (500/1,000/2,000/5,000/9,979) using this run's own
  numbers -- no new full-VAL runs needed for this part.
- **5,000 is a plausible sweet-spot candidate worth a dedicated
  comparison against 9,979**: the marginal cost/benefit shape changes
  sharply at this point. 500->5,000 buys -14.2pp coverage for a ~14.8x
  generation-cost increase (both Stage 2 single-shot and full-VAL
  agree); 5,000->9,979 buys only another -2.2pp for a further ~1.9-2.1x
  cost increase on top of that. The last third of the tested template
  range delivers a small fraction of the total coverage gain at a
  proportionally larger cost step than what came before it -- exactly
  the pattern worth checking whether it holds up as a real production
  trade-off, not assumed from these two ratios alone. Do not default to
  5,000 without running this comparison end-to-end -- it is a candidate
  to test, not a conclusion.
- If match-time (not merge/write) is confirmed as the dominant cost via
  profiling, that points at template-retrieval/prefiltering optimization
  (the existing "template retrieval index (element bitmask + bond-center
  prefilter)" in-progress ROADMAP item) as directly relevant to making a
  larger template count production-viable, independent of which count is
  ultimately chosen.
  - **Addendum (2026-08-12, no new run): existing full-VAL arm data
    already points the same way, without needing a profiling run to
    start with.** Two derived ratios from this run's own
    `full_val/*_metrics.json` (wall-clock, `n_candidate_rows`,
    template count): wall-clock per *template* is nearly flat across the
    20x template-count range (500->9,979: 0.84 -> 0.80 -> 0.93 -> 1.24 ->
    1.16 s/template, a ~1.5x spread), while wall-clock per *merged
    candidate row* (a proxy for merge/write volume) climbs steeply over
    the same range (0.0031 -> 0.0038 -> 0.0072 -> 0.0189 -> 0.0267
    s/row, a ~8.7x spread). If merge/write dominated, the per-row ratio
    should stay roughly flat as template count grows; instead nearly all
    of the added cost tracks template count directly and none of it
    tracks the (sub-linearly growing) output volume. This is an
    aggregate-counter inference, not a true instrumented profile (no
    per-phase timers inside `renkin-pool-gen` splitting match vs.
    merge/write time), and the 5,000/10,000 arms' chunked execution adds
    minor per-chunk startup overhead (10 and 17 chunks respectively)
    that a true single-shot run wouldn't have -- but that overhead is a
    few seconds per chunk against totals of 6,214s/11,604s, too small to
    explain an 8.7x spread. Net: match-time-dominated cost is already
    well-supported by data in hand; a dedicated profiling run would only
    be needed to go from "well-supported" to "precisely quantified," not
    to establish the direction.
- Only once a production template count is frozen from this analysis:
  one final formal-TEST route-search gate run to confirm the VAL-measured
  coverage gain survives into actual route-search outcomes (not just
  candidate-pool presence) -- per Scope above, formal TEST stays reserved
  for exactly this one confirmation, not iterative use.
