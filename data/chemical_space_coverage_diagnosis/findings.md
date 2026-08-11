# Chemical Space Coverage Diagnosis -- Phase A PoC

Issue #101-adjacent candidate-generation coverage program, Phase A (recorded
as a candidate in ROADMAP.md 2026-08-11, promoted to "started" the same day
after confirming chematic 0.11.0 already has everything the PoC needs --
see "Scope decisions" below).

**Question.** 33.0% (1,618/4,903) of the formal TEST corpus has zero
positive candidates in-pool -- a ceiling reranking cannot fix by
construction (see README Roadmap). Are these zero-positive targets
concentrated *near* USPTO-50k TRAIN chemical space (candidate generator
failing to reach structures it should be able to reach -- favors Phase B,
template-diversity scaling) or *far* from it (training/template knowledge
extrapolation limit -- favors Phase C, higher-level/consolidated
templates)?

**Success condition** (as scoped, not "improve solved rate"): decompose the
33.0% into >=2-4 chemical-space regions with per-region metrics, sufficient
to make a Phase B vs Phase C priority call.

## Scope decisions

- **No chematic 0.11 -> 0.13 upgrade.** Confirmed chematic-fp 0.11.0 already
  ships `ecfp4`, `tanimoto_ecfp4`, `tanimoto_matrix`, `top_k_similar` -- the
  facade's `fp` feature (`chematic::fp`) is publicly available and untouched
  between 0.11 and 0.13 for this API surface. No motivating requirement for
  0.13 was identified; deferred as a fully independent future task.
- **RENKIN core untouched.** All fingerprint/Tanimoto computation lives in
  `tools/chemical-space-eval/`, a standalone nested Cargo workspace (its own
  `[workspace]` table, not a member of the root `Cargo.toml` -- verified
  `git diff --stat Cargo.toml Cargo.lock` empty throughout this work). It
  depends on `chematic 0.11` with the `fp` feature directly. RENKIN's own
  `renkin-fp` binary (`src/bin/fp.rs`, gated behind the existing `nn-scoring`
  feature) was deliberately NOT reused, to keep this PoC's blast radius at
  zero for RENKIN core if it turns out to be a dead end.
- **Reference "TRAIN" corpus**: USPTO-50k TRAIN split product SMILES (the
  same split `scripts/extract_templates.py --split train` [default] draws
  from for template extraction), canonicalized + atom-maps-cleared via
  `renkin-canonicalize --clear-atom-maps`, deduplicated: 39,736 unique
  structures. Deliberately NOT the reranker's own 70/15/15 target-hash
  re-split (`train_reranker.py`'s `split_for_target`) -- that split governs
  reranker generalization, not candidate-generation template coverage,
  which is what this question is actually about.
- **Route-solved-rate omitted from v1** (pre-authorized: "利用可能なら"). The
  only per-target route-search data available is the 100-target paired gate
  (Issue #101 Task 35's `route_to_configured_stock` 16->20), which is a
  different, much smaller corpus (~25 targets/bin at this binning) and
  would not be informative here.

## Data provenance and verification

All regenerated from committed, hash-pinned sources -- no new ground truth,
no retraining, frozen model/frequency-table untouched.

1. `data/uspto50k_raw_test_split.jsonl` / `..._train_split.jsonl` dumped
   from the local HF cache (`bisectgroup/USPTO_50K`, pinned revision
   `08a575f0546b2be57242997fd45f684d6814d5a9`, already present locally --
   no network access used or needed). Test-split dump verified
   byte-for-byte against `data/phase3a_reranker_ground_truth_audit/
   findings.md`'s documented SHA-256
   (`c810404508bbf7a4a5154828c322596c09d0c8c999646616a161271487054550`) --
   exact match.
2. `scripts/generate_real_labels.py` (unmodified) regenerated
   `data/reranker_groups_uspto50k_test.jsonl` -- 4,903/4,903 targets
   matched, 0 unmatched.
3. `renkin-pool-gen` (unmodified) regenerated
   `data/phase3e_reranker_training/{pool,groups}_test_formal.jsonl` against
   `data/templates_extracted_500.smi` (already committed). Both
   `candidate_jsonl_sha256` and `target_group_index_sha256` in the
   regenerated manifest matched the committed
   `manifest_test_formal.json` exactly:
   `sha256:e16d811d128aa29653481be0a19589e643aa87e65785572550f5bdf7ce0bf94c`
   / `sha256:64f207eff20cf7d3fc34b0de5065ba21b9fa6a794591b2b1e4f4603ecb5e3036`
   -- this is the identical pool the published 33.0% figure comes from, not
   an approximation of it.
4. `scripts/chemical_space_coverage_export_test_labels.py` (new, reuses
   `scripts/train_reranker.py`'s functions unmodified -- same code path as
   `scripts/phase3e_evaluate_formal_test.py`) loaded the **committed**
   `frequency_table.json`'s `table` field directly (verified
   `template_frequency_table_sha256(...) == freeze_manifest.json`'s frozen
   SHA -- skips ~40 minutes of unnecessary full-TRAIN pool-gen needed only
   to *refit* that same table) and the frozen `model.txt` (SHA-256
   verified against `freeze_manifest.json`). Produced one row per target:
   `zero_positive`, `baseline_top1_hit`, `reranker_top1_hit` (the latter
   two `None` when `zero_positive` is true -- top1 is undefined without a
   positive to rank). **`groups_with_zero_positive_in_pool` reproduced
   exactly: 1,618/4,903 (33.00%)**, matching the published figure to the
   row -- confirms every downstream bin below is a decomposition of the
   *actual* 33.0%, not a different corpus.
5. `tools/chemical-space-eval`'s `nearest-train-tanimoto` binary computed
   ECFP4 (chematic 0.11.0, radius=2, nbits=2048, no chirality -- same
   config as `src/bin/fp.rs`) for all 4,903 TEST targets and all 39,736
   TRAIN reference structures (0 parse failures on either side) and found
   each TEST target's single nearest TRAIN neighbor via
   `chematic_fp::top_k_similar(..., k=1)`.
6. `tools/chemical-space-eval/report.py` joined (4) and (5) by `target_id`
   (set-equality asserted) and binned.

## Results

Bins: `>=0.80` near, `0.60-0.80` medium, `0.40-0.60` far, `<0.40`
very-far/OOD-like (nearest-TRAIN ECFP4 Tanimoto).

| bin | N | zero-positive | zero-positive rate | positive-present N | baseline top1 (cond.) | reranker top1 (cond.) |
|---|---|---|---|---|---|---|
| near (>=0.80) | 271 | 84 | 31.0% | 187 | 24.1% | 40.1% |
| medium (0.60-0.80) | 1,362 | 411 | 30.2% | 951 | 22.8% | 43.4% |
| far (0.40-0.60) | 2,474 | 787 | 31.8% | 1,687 | 25.1% | 44.5% |
| very-far/OOD (<0.40) | 796 | 336 | **42.2%** | 460 | 25.9% | 41.3% |
| **overall** | 4,903 | 1,618 | 33.0% | 3,285 | -- | -- |

Full machine-readable output:
`data/chemical_space_coverage_diagnosis/coverage_by_chemical_space_report.json`.

## Interpretation

**Neither of the two illustrative scenarios from the original proposal
holds.** The zero-positive rate is flat at ~30-32% across near/medium/far
(84% of the corpus, N=4,107) -- being structurally close to a TRAIN example
does *not* meaningfully protect a target from having zero usable
candidates. Only the very-far/OOD-like tail (16% of the corpus, N=796)
shows a real elevation, +10-12pp over the other three bins (42.2% vs
~30-32%) -- consistent with *some* training/template extrapolation-limit
effect at the extreme, but far too small a share of the corpus and too
mild an effect to be the dominant driver of the 33.0% overall gap.

This is itself the useful finding, not a null result: **whole-molecule
nearest-TRAIN similarity is a weak predictor of candidate-generation
coverage failure.** A target essentially as similar to some TRAIN product
as two TRAIN products are to each other (>=0.80 Tanimoto) still fails to
get any correct candidate in-pool 31% of the time. That argues *against*
"the generator just needs to reach further into the same kind of chemical
space it already covers" (Phase B's core premise) as the primary mechanism,
and *for* the failure being driven by something a whole-molecule fingerprint
doesn't see -- most plausibly the specific local reaction site/disconnection
pattern (which templates match on structurally, not on global similarity)
rather than overall molecular resemblance. That point toward Phase C
(higher-level/consolidated templates, which generalize across more
reaction-center variations per template) being the better-motivated next
experiment over blind Phase B template-count scaling, though this PoC alone
is not proof -- see limitations.

The reranker's conditional top1 lift over baseline (roughly +15-20pp) is
essentially uniform across all four bins, i.e. reranker quality is
independent of chemical-space distance to TRAIN -- consistent with the
reranker only ever operating on whatever candidates already exist and
having no bearing on whether the right one is in the pool at all, which is
the same "reranking is a ceiling-bound fix" point the coverage-gap README
entry already makes.

## Limitations

- **Whole-molecule ECFP4 Tanimoto is a coarse proxy** for "does the
  template set cover this target's actual disconnection." Two molecules can
  be globally dissimilar while sharing the exact local motif a template
  would match (or globally similar while differing exactly at the
  reaction center). A reaction-center/substructure-level similarity metric
  would be a sharper follow-up if Phase B/C prioritization stays
  ambiguous.
- **Single nearest neighbor, not local density.** A target with one
  near-TRAIN neighbor but otherwise-sparse local neighborhood is binned the
  same as one embedded in a dense TRAIN cluster; template *frequency*
  around a target's neighborhood (not just distance to the single closest
  point) may matter more and isn't captured here.
- **N=4 bins, no confidence intervals reported** -- this is a PoC-level
  decomposition (the explicitly scoped success condition), not a
  publication-grade statistical claim. The near bin's N=271 is the
  smallest; treat that 31.0% cell as the least precise of the four.
- Route-solved-rate not included (see "Scope decisions").

## Reproducing this

```
cargo build --release --bin renkin-canonicalize --bin renkin-pool-gen
# (raw HF split dump -- see data/phase3a_reranker_ground_truth_audit/findings.md)
python3 scripts/generate_real_labels.py --canonicalize-bin target/release/renkin-canonicalize \
    --output data/reranker_labels_uspto50k_test.jsonl \
    --groups-output data/reranker_groups_uspto50k_test.jsonl \
    --summary-output data/reranker_labels_uspto50k_test.summary.json
./target/release/renkin-pool-gen --groups data/reranker_groups_uspto50k_test.jsonl \
    --templates data/templates_extracted_500.smi \
    --pool-output data/phase3e_reranker_training/pool_test_formal.jsonl \
    --groups-output data/phase3e_reranker_training/groups_test_formal.jsonl \
    --manifest-output /tmp/manifest_check.json  # diff against the committed one
python3 scripts/chemical_space_coverage_export_test_labels.py
cargo build --release --manifest-path tools/chemical-space-eval/Cargo.toml
./tools/chemical-space-eval/target/release/nearest-train-tanimoto \
    --train-reference data/chemical_space_coverage_diagnosis/train_reference_products.smi \
    --test-labels data/chemical_space_coverage_diagnosis/test_target_labels.jsonl \
    --output data/chemical_space_coverage_diagnosis/nearest_train_tanimoto.jsonl
python3 tools/chemical-space-eval/report.py
```

(`train_reference_products.smi` itself: canonicalize+dedupe
`data/uspto50k_raw_train_split.jsonl`'s `product` field via
`renkin-canonicalize --clear-atom-maps`.)

## Recommendation

Do not start Phase B (template-diversity 500->10,000) on the strength of
"near-TRAIN targets are failing" -- this PoC found the opposite: TRAIN
proximity barely matters. Before committing to Phase C either, the
sharper follow-up (reaction-center-level rather than whole-molecule
similarity) would firm up the mechanism; that is future work, not started
here, and not yet added to ROADMAP.md as anything more than a limitation
note above pending discussion.
