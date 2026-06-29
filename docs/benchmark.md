# Benchmark

## USPTO-50k Test Set

RENKIN is evaluated on the full [USPTO-50k](https://huggingface.co/datasets/bisectgroup/USPTO_50K) test set (4,907 molecules) — the standard benchmark for multi-step retrosynthesis planning.

**What "solved" means:** A target is *solved* if at least one complete retrosynthetic route is found where every leaf precursor is in the 509-compound building block set. This is **not** a check against ground-truth reactants from the USPTO dataset.

### Latest Results (v0.15.5) — depth=5, beam=100, ~5,000 extracted templates

| Config | Solved | Success Rate | Avg Time | Hardware |
|--------|--------|-------------|----------|----------|
| depth=5, beam=100, ~5,000 templates + Phase A | **3,826 / 4,907** | **78.0%** | **≈2,800 ms/mol** | Apple M-series, 8 threads |

Building blocks: 509 hand-curated commercial reagents (default set).

### Progress History (Table A — RENKIN internal)

| Phase | Solved | Rate | Notes |
|-------|--------|------|-------|
| 31 rules only, depth=3 | 366 / 4,907 | 7.5% | handcrafted rules only |
| + 191 extracted templates, depth=3 | 1,363 / 4,907 | 27.8% | rdchiral top-300 |
| + depth=5 | 1,909 / 4,907 | 38.9% | depth increase |
| + top-500 templates, depth=5 | 2,315 / 4,907 | 47.2% | 314 rules total |
| + beam=100 | 2,688 / 4,907 | 54.8% | beam search |
| + Phase A frequency weighting | 3,540 / 4,907 | 72.1% | step_cost bonus for high-freq templates |
| **+ ~5,000 templates (v0.15.5)** | **3,826 / 4,907** | **78.0%** | current default ✅ |

### Comparison: Multi-Step Planners (Table B)

> **⚠️ Not a matched-condition comparison.** Building block counts, template counts, and evaluation setups differ significantly across systems. These numbers cannot be used to rank tools definitively. A matched-condition experiment (same BB set, same templates) has not been conducted.

| System | Multi-Step Rate | Stock | Templates | Source |
|--------|----------------|-------|-----------|--------|
| **RENKIN v0.15.5** | **78.0%** | 509 BBs | ~5,000 | this work, 2026 |
| AiZynthFinder | 45–53% | ~6M (eMolecules) | ~50,000 | Genheden et al., J. Cheminform. 2020 |
| Retro\* | 44.3% | ~20,000 | ~17,000 | Chen et al., NeurIPS 2020 |
| ASKCOS | ~41% | ~20,000 | ~195,000 | Coley et al., Science 2019 |

### Comparison: Single-Step Top-1 Models (Table C — different metric)

> **⚠️ Different metric.** These measure single-step top-1 prediction accuracy (does the model's top-1 prediction match the known reaction?), **not** multi-step planning success rate. Direct comparison with Table B is not valid.

| System | Single-Step Top-1 | Source |
|--------|------------------|--------|
| LocalRetro | 53.4% | Chen et al., ACS Cent. Sci. 2021 |
| GLG | 58.0% | Yu et al., NeurIPS 2022 |

!!! note "Condition differences"
    RENKIN's 78.0% uses only **509 building blocks** and **~5,000 templates**, while systems like AiZynthFinder use 6M-compound databases and 50k templates. RENKIN's strength is **portability**: Pure Rust, zero C/C++ dependencies, WASM + Python + CLI from one binary.

### What RENKIN solves well

RENKIN achieves high accuracy on standard bond disconnections:

- Esters → carboxylic acid + alcohol
- Amides → acid + amine (graph-based cleavage)
- Biaryls → aryl halide + boronic acid (Suzuki, graph-based)
- Aryl amines → aryl halide + amine (Buchwald-Hartwig)
- C–halide bonds → dehalogenated arene
- Boc / Cbz protecting group removal (graph-based)
- Diaryl sulfones → arylsulfonyl chloride + arene (graph-based)
- Sulfonamides → sulfonyl chloride + amine

### Out-of-Distribution (OOD) Evaluation

To check whether RENKIN's accuracy is specific to the USPTO-50k domain, we evaluated on **500 FDA-approved drugs** from ChEMBL (Phase 4, MW 150–700, no salts, 2026-06-25).

| Dataset | Solved | Success Rate | Notes |
|---------|--------|-------------|-------|
| USPTO-50k test set (in-domain) | 3,826 / 4,907 | **78.0%** | templates from USPTO train set |
| **ChEMBL approved drugs (OOD)** | **409 / 500** | **81.8%** | real FDA-approved drugs |

The +3.8 pp difference on approved drugs is consistent with the hypothesis that the rule set covers common pharmaceutical transformations. However, this result should be interpreted cautiously: both datasets are small-molecule organic chemistry, so the OOD gap is limited. Unsolved molecules in both datasets share the same profile: nitrogen-rich heterocycles (+17 pp) and fluorinated compounds (+11 pp).

### Failure Taxonomy (2026-06-29, 500-mol sample)

`renkin-bench --failure-taxonomy` classifies unsolved targets by cause:

| Cause | Count | % of unsolved | Description |
|-------|-------|--------------|-------------|
| beam_limit_hit | 111 / 112 | 99.1% | beam pruned promising nodes |
| max_depth_reached | 111 / 112 | 99.1% | route depth > 5 required |
| stock_near_miss | 111 / 112 | 99.1% | BB found in frontier but no complete route |
| no_template_match | 1 / 112 | 0.9% | fewer than 3 templates matched |

**Key finding:** Template and building block coverage is not the bottleneck. Nearly all unsolved targets hit the search budget limit (beam/depth). Cascade search (Stage 2: depth=7, beam=300 on unsolved only) is the primary lever for further improvement.

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
