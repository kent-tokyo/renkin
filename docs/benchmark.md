# Benchmark

> ⚠️ **Notice (2026-07-22): historical 78.0% (single-pass) / 95.9% (cascade) / 81.8% (ChEMBL OOD) figures on this page are invalidated and have NOT been re-measured.** They were measured before fixing four retrosynthesis-rule/validator bugs that inflated solved counts with chemically-invalid or falsely-corroborated routes (full history below). **Only the "Corrected baseline" section below reflects the current rule set** — everywhere else on this page still shows the old, invalidated numbers for historical continuity, each marked accordingly. Do not cite the unmarked historical figures as current RENKIN performance.
>
> **Corrected baseline — USPTO-50k Stage 1 (single-pass), commit `e20dc8c`, 2026-07-22.** `raw_solved_rate` **20.09%** (986/4,907) → `atom_balanced_solved_rate` **15.41%** (756/4,907) → `provenance_validated_solved_rate` **0.88%** (43/4,907). These three are a *nested* series over the same 4,907 targets (each a stricter subset of the previous), not independent measurements. **`provenance_validated_solved_rate` is a floor, not RENKIN's actual correctness rate**: it only counts routes every step of which the current validator can positively confirm against its own originating rule, and a large, not-yet-quantified share of `Invalid` verdicts (72.2% of steps in a diagnostic n=300 sample) are atom-balanced but fail that check for reasons not yet separated into "real chemistry error" vs. "validator false negative" (canonicalization/tautomer/regiochemistry edge cases — `suzuki_retro` steps are 0% invalid, `cn_aliphatic_cleavage` steps are 97.6% invalid, too wide a spread to be one cause). Full methodology, config hashes, and per-rule breakdown: [`tasks/phase31_final_remeasurement_run.md`](https://github.com/kent-tokyo/renkin/blob/master/tasks/phase31_final_remeasurement_run.md).
>
> **Fix history:** `aryl_carboxylation_retro` ester-overmatch, fixed (PR #26). Three more atom-loss rules — `aryl_chloride_retro`, `aryl_iodide_retro`, `aryl_fluoride_snAr_retro` — found and removed (PR #31, "31.11": each deleted a halogen atom with no tracked reagent). Forward validator, which had accepted a step as `Valid` if *any* rule's SMIRKS coincidentally reproduced the target rather than only the rule the step actually used, bound to each step's own originating rule (PR #33, "31.12"). All merged to `master` before this re-measurement, in that order, each individually CI-verified. **Cascade (95.9%) and ChEMBL OOD (81.8%) have not been re-run against the corrected rule set** and remain invalidated historical figures — see their own sections below.

## USPTO-50k Test Set

RENKIN is evaluated on the full [USPTO-50k](https://huggingface.co/datasets/bisectgroup/USPTO_50K) test set (4,907 molecules) — the standard benchmark for multi-step retrosynthesis planning.

**What "solved" means:** A target is *solved* if at least one complete retrosynthetic route is found where every leaf precursor is in the 509-compound building block set. This is **not** a check against ground-truth reactants from the USPTO dataset.

### Corrected Baseline (commit `e20dc8c`, 2026-07-22) — depth=5, beam=100, 5,000 extracted templates, 28 handcrafted rules

| Metric | Value | Denominator |
|--------|-------|-------------|
| `raw_solved_rate` | **20.09%** (986/4,907) | all 4,907 targets |
| `atom_balanced_solved_rate` | **15.41%** (756/4,907) | all 4,907 targets — subset of raw |
| `provenance_validated_solved_rate` | **0.88%** (43/4,907) | all 4,907 targets — subset of atom-balanced; **floor, not ceiling, on correctness (see notice above)** |
| depth=0 direct stock hit | 0.04% (2/4,907) | all 4,907 targets |
| latency, all targets | p50 7.3s / p95 28.2s / p99 51.2s | includes unsolved targets run to the search budget |
| latency, solved only | p50 1.0s / p95 9.4s / p99 15.6s | |

Building blocks: 475 hand-curated commercial reagents (`data/building_blocks.smi`). Full config/hashes/per-rule breakdown: `tasks/phase31_final_remeasurement_run.md`.

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
| **Cascade: Stage 2 (depth=7, beam=300, unsolved only)** | **4,705 / 4,907** | **95.9%** | 2026-06-29 ✅ |

*Status: invalidated historical measurement — see notice above.*

### Comparison: Multi-Step Planners (Table B)

> **⚠️ Not a matched-condition comparison.** Building block counts, template counts, and evaluation setups differ significantly across systems. These numbers cannot be used to rank tools definitively. A matched-condition experiment (same BB set, same templates) has not been conducted.

| System | Multi-Step Rate | Stock | Templates | Source |
|--------|----------------|-------|-----------|--------|
| **RENKIN v0.15.5 (corrected, `raw_solved_rate`)** | **20.09%** | 475 BBs | ~5,000 | this work, 2026-07-22 |
| AiZynthFinder | 45–53% | ~6M (eMolecules) | ~50,000 | Genheden et al., J. Cheminform. 2020 |
| Retro\* | 44.3% | ~20,000 | ~17,000 | Chen et al., NeurIPS 2020 |
| ASKCOS | ~41% | ~20,000 | ~195,000 | Coley et al., Science 2019 |

RENKIN's row uses `raw_solved_rate` (found ≥1 route to stock) for the fairest like-for-like comparison — the other systems' published figures are also uncorroborated "found a route" rates, not independently mass-balance/step-validated. RENKIN additionally reports two stricter, nested figures (`atom_balanced_solved_rate` 15.41%, `provenance_validated_solved_rate` 0.88% — see the corrected-baseline notice above) that the other systems' papers don't provide a comparable number for.

### Comparison: Single-Step Top-1 Models (Table C — different metric)

> **⚠️ Different metric.** These measure single-step top-1 prediction accuracy (does the model's top-1 prediction match the known reaction?), **not** multi-step planning success rate. Direct comparison with Table B is not valid.

| System | Single-Step Top-1 | Source |
|--------|------------------|--------|
| LocalRetro | 53.4% | Chen et al., ACS Cent. Sci. 2021 |
| GLG | 58.0% | Yu et al., NeurIPS 2022 |

!!! note "Condition differences"
    RENKIN's 20.09% uses only **475 building blocks** and **~5,000 templates**, while systems like AiZynthFinder use 6M-compound databases and 50k templates. RENKIN's strength is **portability**: Pure Rust, zero C/C++ dependencies, WASM + Python + CLI from one binary.

### What RENKIN solves well

RENKIN achieves high accuracy on standard bond disconnections:

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
