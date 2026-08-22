---
title: "RENKIN Historical USPTO-50k Stress Test (v0.15.5, Frozen)"
description: "A frozen, single-commit USPTO-50k route-to-stock stress test for RENKIN v0.15.5, with full methodology and known limitations. For current, matched-condition comparison data against other planners, see the Open-Source Retrosynthesis Comparison guide."
---

# Historical USPTO-50k Stress Test (Frozen, v0.15.5)

**This entire page is a frozen historical artifact, not a live or current benchmark.** Every number below — including the "Corrected Baseline" section — was measured once, against one specific commit (`e20dc8c`, RENKIN v0.15.5, 2026-07-22), and has not been re-run since. "Corrected" describes the rule set *at that commit*, not RENKIN's present state; treat every figure on this page as a snapshot of what RENKIN did on one day, not as its current performance.

> **Looking for current, matched-condition comparison data?** See the
> [Open-Source Retrosynthesis Comparison](guides/open-source-retrosynthesis-comparison.md#500-target-results)
> guide instead: a 500-target, paired-bootstrap, exact-McNemar-tested
> comparison against AiZynthFinder under both a shared stock and each
> tool's own native stock, kept current. This page is not kept current
> and should not be used for that purpose.

> ⚠️ **Notice (2026-07-22): historical 78.0% (single-pass) / 95.9% (cascade) / 81.8% (ChEMBL OOD) figures on this page are invalidated and have NOT been re-measured.** They were measured before fixing four retrosynthesis-rule/validator bugs that inflated solved counts with chemically-invalid or falsely-corroborated routes (full history below). **Only the "Corrected baseline" section below reflects the rule set as it stood at the frozen commit** — everywhere else on this page still shows the old, invalidated numbers for historical continuity, each marked accordingly. Do not cite any figure on this page, marked or unmarked, as current RENKIN performance.
>
> **Corrected baseline — USPTO-50k Stage 1 (single-pass), commit `e20dc8c`, 2026-07-22.** Search-to-stock rate (`raw_solved_rate`) **20.09%** (986/4,907) → atom-balance-filtered rate (`atom_balanced_solved_rate`) **15.41%** (756/4,907) → current-validator-confirmed rate (`provenance_validated_solved_rate`) **0.88%** (43/4,907). These three are a *nested* series over the same 4,907 targets (each a stricter subset of the previous), not independent measurements, and none is an experimentally-verified synthesis success rate or a human-chemist-reviewed route-accuracy figure.
>
> **What 0.88% actually is, precisely:** the fraction of targets with a complete stock route where every step passes the coarse atom-balance check AND is positively confirmed by its own originating rule's current validator. **This is not a measured chemical-accuracy rate, and it is not a proven lower bound on true correctness** — establishing a lower bound would require knowing the validator has no false positives, which hasn't been shown (1 of the 44 `validated` routes fails atom-balance; a diagnostic sample shows 14/864 steps are `Valid`-but-imbalanced). Separately, `Invalid` verdicts are not proven errors either: 72.2% of steps in a diagnostic n=300 sample are `Invalid` but atom-balanced, and an unknown fraction of those may be validator false negatives (canonicalization/tautomer/regiochemistry edge cases) rather than genuine rule or route errors — the split has not been measured (`suzuki_retro` steps are 0% invalid vs. `cn_aliphatic_cleavage` at 97.6%, a spread worth investigating further but not yet attributed to either cause). Full methodology, config hashes, and per-rule breakdown: [`tasks/phase31_final_remeasurement_run.md`](https://github.com/kent-tokyo/renkin/blob/master/tasks/phase31_final_remeasurement_run.md).
>
> **Fix history:** `aryl_carboxylation_retro` ester-overmatch, fixed (PR #26). Three more atom-loss rules — `aryl_chloride_retro`, `aryl_iodide_retro`, `aryl_fluoride_snAr_retro` — found and removed (PR #31, "31.11": each deleted a halogen atom with no tracked reagent). Forward validator, which had accepted a step as `Valid` if *any* rule's SMIRKS coincidentally reproduced the target rather than only the rule the step actually used, bound to each step's own originating rule (PR #33, "31.12"). All merged to `master` before this re-measurement, in that order, each individually CI-verified. **Cascade (95.9%) and ChEMBL OOD (81.8%) have not been re-run against the corrected rule set** and remain invalidated historical figures — see their own sections below. Phase 31 corrected-baseline publication is complete; validator fidelity analysis (separating real rule errors from validator false negatives) remains an explicit, open follow-up.

## USPTO-50k Test Set

USPTO-50k is primarily used as a **single-step** retrosynthesis benchmark (see "Comparison: Single-Step Top-1 Models" below for that use). This page repurposes a frozen, 4,907-row target corpus derived from [USPTO-50k](https://huggingface.co/datasets/bisectgroup/USPTO_50K) as a **route-to-stock stress test** for RENKIN's multi-step search — it is not a canonical multi-step benchmark like [PaRoutes](https://github.com/AstraZeneca/PaRoutes) (which RENKIN also supports directly, via `renkin-bench --input-format paroutes` — see the [README](https://github.com/kent-tokyo/renkin#paroutes-compatibility)).

There is also a known, disclosed provenance gap in this corpus: `data/uspto50k_test.smi`'s header claims "5007 reactions," but the file has 4,907 data rows, and no record in this repository traces the exact upstream Hugging Face revision this file was derived from. See the
[Open-Source Retrosynthesis Comparison guide's "Known gaps" section](guides/open-source-retrosynthesis-comparison.md#known-gaps-disclosed-not-fixed-in-this-round)
for the full disclosure — not repeated here to avoid two independently-drifting copies of the same caveat.

**What "solved" means:** A target is *solved* if at least one complete retrosynthetic route is found where every leaf precursor is in the building block set (402 unique compounds loaded from `data/building_blocks.smi` for the corrected-baseline run below — see that section for how this differs from the file's raw line count). This is **not** a check against ground-truth reactants from the USPTO dataset.

### Corrected Baseline (commit `e20dc8c`, 2026-07-22) — depth=5, beam=100, 5,000 extracted templates, 28 handcrafted rules

| Public label | Internal metric | Value | Denominator |
|--------|--------|-------|-------------|
| Search-to-stock rate | `raw_solved_rate` | **20.09%** (986/4,907) | all 4,907 targets |
| Atom-balance-filtered rate | `atom_balanced_solved_rate` | **15.41%** (756/4,907) | all 4,907 targets — subset of search-to-stock |
| Current-validator-confirmed rate | `provenance_validated_solved_rate` | **0.88%** (43/4,907) | all 4,907 targets — subset of atom-balance-filtered; see notice above for what this figure is and isn't |
| depth=0 direct stock hit | — | 0.04% (2/4,907) | all 4,907 targets |
| latency, all targets | — | p50 7.3s / p95 28.2s / p99 51.2s | includes unsolved targets run to the search budget |
| latency, solved only | — | p50 1.0s / p95 9.4s / p99 15.6s | |

These three rates are a nested series over the same 4,907 targets, not independent numbers, and none is a measured or bounded chemical-correctness rate on its own — read the notice above before citing any of them in isolation.

Building blocks: **402** unique compounds actually loaded by `ChemEnv::load("data/building_blocks.smi")` (`ChemEnv::bb_count()`) — i.e. after parsing, canonicalization, and de-duplication, not the file's raw line count (449 non-comment lines → 3 fail to parse → 44 duplicate after canonicalization → 402 unique). Full config/hashes/per-rule breakdown: `tasks/phase31_final_remeasurement_run.md`.

### Historical Results (v0.15.5, pre-fix) — depth=5, beam=100, ~5,000 extracted templates

| Config | Solved | Success Rate | Avg Time | Hardware |
|--------|--------|-------------|----------|----------|
| depth=5, beam=100, ~5,000 templates + Phase A | **3,826 / 4,907** | **78.0%** | **≈2,800 ms/mol** | Apple M-series, 8 threads |

*Status: invalidated historical measurement, pre-31.11/31.12 — see notice above. Kept for continuity only.*

### Progress History (Table A — RENKIN internal)

| Phase | Solved | Rate | Notes |
|-------|--------|------|-------|
| 31 rules only, depth=3 | 366 / 4,907 | 7.5% | handcrafted rules only |
| + 191 extracted templates, depth=3 | 1,363 / 4,907 | 27.8% | rdchiral top-300 |
| + depth=5 | 1,909 / 4,907 | 38.9% | depth increase |
| + top-500 templates, depth=5 | 2,315 / 4,907 | 47.2% | 314 rules total |
| + beam=100 | 2,688 / 4,907 | 54.8% | beam search |
| + Phase A frequency weighting | 3,540 / 4,907 | 72.1% | step_cost bonus for high-freq templates |
| **+ ~5,000 templates (v0.15.5)** | **3,826 / 4,907** | **78.0%** | pre-fix measurement, invalidated |
| **Cascade: Stage 2 (depth=7, beam=300, unsolved only)** | **4,705 / 4,907** | **95.9%** | 2026-06-29 ✅ (pre-fix, invalidated) |

*Status: invalidated historical measurement — see notice above.*

### Comparison: Single-Step Top-1 Models (different metric)

> **⚠️ Different metric.** These measure single-step top-1 prediction accuracy (does the model's top-1 prediction match the known reaction?), **not** multi-step planning success rate. Direct comparison with RENKIN's multi-step figures above, or with the literature citations in the previous section, is not valid.

| System | Single-Step Top-1 | Source |
|--------|------------------|--------|
| LocalRetro | 53.4% | Chen et al., ACS Cent. Sci. 2021 |

!!! note "Condition differences"
    RENKIN's 20.09% uses only **402 building blocks** and **~5,000 templates**, while systems like AiZynthFinder use 6M-compound databases and 50k templates. RENKIN's strength is **portability**: Pure Rust, zero C/C++ dependencies, WASM + Python + CLI from one binary.

### What RENKIN solves well

> ⚠️ **Not a measured accuracy claim.** The list below describes which
> transformation families RENKIN has an explicit hand-crafted or
> graph-based rule for — it is a statement about rule *coverage*, not a
> re-measured per-class accuracy figure (no such figure exists against
> the corrected rule set; the historical 78.0%/95.9% figures this section
> previously implied are invalidated, see the notice at the top of this
> page).

RENKIN contains explicit rules for these transformation families:

- Esters → carboxylic acid + alcohol
- Amides → acid + amine (graph-based cleavage)
- Biaryls → aryl halide + boronic acid (Suzuki, graph-based)
- Aryl amines → aryl halide + amine (Buchwald-Hartwig)
- Boc / Cbz protecting group removal (graph-based)
- Diaryl sulfones → arylsulfonyl chloride + arene (graph-based)
- Sulfonamides → sulfonyl chloride + amine

### Out-of-Distribution (OOD) Evaluation

> ⚠️ **Not re-measured against the corrected rule set (pre-31.11/31.12, invalidated).** ChEMBL OOD has not been re-run since the fix history in the notice above; treat both rows below as historical only.

To check whether RENKIN's accuracy is specific to the USPTO-50k domain, we evaluated on **500 FDA-approved drugs** from ChEMBL (Phase 4, MW 150–700, no salts, 2026-06-25).

| Dataset | Solved | Success Rate | Notes |
|---------|--------|-------------|-------|
| USPTO-50k test set (in-domain) | 3,826 / 4,907 | **78.0%** (pre-fix) | templates from USPTO train set |
| **ChEMBL approved drugs (OOD)** | **409 / 500** | **81.8%** (pre-fix) | real FDA-approved drugs |

The +3.8 pp difference on approved drugs is consistent with the hypothesis that the rule set covers common pharmaceutical transformations. However, this result should be interpreted cautiously: both datasets are small-molecule organic chemistry, so the OOD gap is limited. Unsolved molecules in both datasets share the same profile: nitrogen-rich heterocycles (+17 pp) and fluorinated compounds (+11 pp).

### Failure Taxonomy (2026-06-29, 500-mol sample)

> ⚠️ **Not re-measured against the corrected rule set (pre-31.11/31.12, invalidated).** Kept for methodology reference only.

`renkin-bench --failure-taxonomy` classifies unsolved targets by cause:

| Cause | Count | % of unsolved | Description |
|-------|-------|--------------|-------------|
| beam_limit_hit | 111 / 112 | 99.1% | beam pruned promising nodes |
| max_depth_reached | 111 / 112 | 99.1% | route depth > 5 required |
| stock_near_miss | 111 / 112 | 99.1% | BB found in frontier but no complete route |
| no_template_match | 1 / 112 | 0.9% | fewer than 3 templates matched |

**Key finding:** Template and building block coverage is not the bottleneck. Nearly all unsolved targets hit the search budget limit (beam/depth). Cascade search (Stage 2: depth=7, beam=300 on unsolved only) resolved 879/1,081 (81.3%) of previously unsolved targets, lifting the overall rate from 78.0% to **95.9%**.

### Improving the success rate

1. **Cascade search** — re-run unsolved targets with higher beam/depth (`--depth 7 --beam-width 300`). Failure taxonomy shows this is the primary lever.
2. **Expand the building block database** — supply eMolecules, ZINC, or your internal stock via `--building-blocks`
3. **Add more templates** — extract additional templates from the full USPTO training set (`--templates data/templates_extracted_5000.smi`)

### Running the benchmark

```bash
# Build
cargo build --release

# Full benchmark — 50 chunks × 100 mol, resumable
bash scripts/run_benchmark_chunks.sh \
    data/uspto50k_test.smi \
    data/templates_extracted_5000.smi \
    data/bench_chunks \
    5 100

# Failure taxonomy on unsolved
./target/release/renkin-bench \
    --input data/uspto50k_test.smi \
    --depth 5 --beam-width 100 \
    --templates data/templates_extracted_5000.smi \
    --failure-taxonomy \
    > bench_result.json

# Aggregate chunks
python3 -c "
import json, glob
files = sorted(glob.glob('data/bench_chunks/chunk_*.json'))
total = solved = 0
for f in files:
    d = json.load(open(f))
    total += d['total']; solved += d['solved']
print(f'{solved}/{total} = {solved/total:.1%}')
"
```
