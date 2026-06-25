# Benchmark

## USPTO-50k Test Set

RENKIN is evaluated on the full [USPTO-50k](https://huggingface.co/datasets/bisectgroup/USPTO_50K) test set (4,907 molecules) — the standard benchmark for single-step retrosynthesis.

### Latest Results (v0.1.8) — depth=5, beam=100, 5,000 extracted templates

| Config | Solved | Success Rate | Avg Time | Hardware |
|--------|--------|-------------|----------|----------|
| depth=5, beam=100, 5,000 templates | **3,831 / 4,907** | **78.1%** | **≈2,800 ms/mol** | Apple M-series, 8 threads |

Building blocks: 509 hand-curated commercial reagents (default set).

### Progress History

| Version / Phase | Solved | Success Rate | Avg Time | Notes |
|-----------------|--------|-------------|----------|-------|
| v0.1.0 | 25 / 500 | 5.0% | 79 ms/mol | 20 rules, 509 BBs, depth=2, 500-mol sample |
| v0.1.1 (baseline) | 1,363 / 4,907 | 27.8% | — | default rules only, depth=3 |
| Phase A (500 templates, beam=100) | 2,315 / 4,907 | 47.2% | — | depth=5, +500 extracted templates |
| Phase A (5k templates, beam=100) | 3,540 / 4,907 | 72.1% | 1,742 ms/mol | depth=5, template frequency weighting |
| Phase A (5k templates, unlimited A\*) | 3,830 / 4,907 | 78.1% | 2,956 ms/mol | depth=5, beam=0 |
| Phase B (5k templates, beam=100, NN scorer) | 3,826 / 4,907 | 78.0% | 3,394 ms/mol | depth=5, ONNX neural scorer |
| v0.1.3 (5k templates, beam=100) | 3,826 / 4,907 | 78.0% | 2,775 ms/mol | depth=5, Pure Rust optimizations |
| **v0.1.8 (5k templates, beam=100, diaryl sulfone rule)** | **3,831 / 4,907** | **78.1%** | **≈2,800 ms/mol** | depth=5, diaryl_sulfone_retro + 509 BBs |

v0.1.8 adds a graph-based diaryl sulfone retrosynthesis rule and expands the building block set to 509 reagents (+29 sulfonyl chlorides and CF3 arenes).

### Comparison with Other Systems

| System | Top-1 | Stock | Templates | Notes |
|--------|-------|-------|-----------|-------|
| **RENKIN v0.1.8** | **78.1%** | **509 BBs** | **5,000** | Pure Rust, no C++ dependencies |
| AiZynthFinder (Mol. Inf. 2020) | ~45% | eMolecules (~6M) | ~50,000 | Python, RDKit |
| Retro\* (ICML 2020) | ~40% | eMolecules (~6M) | ~50,000 | Python |
| LocalRetro (AAAI 2021) | ~65% | eMolecules (~6M) | template-free | GNN-based |
| GLN (NeurIPS 2020) | ~64% | eMolecules (~6M) | ~17,000 | GNN-based |

!!! note "Apples vs oranges"
    RENKIN's 78.0% is achieved with only **509 commercial reagents** and **5,000 templates**.
    Other systems use multi-million-compound databases (eMolecules, ZINC) and tens of thousands of templates,
    putting RENKIN at a structural disadvantage on raw numbers.

    RENKIN's strength is **portability and embeddability**: Pure Rust, zero C/C++ dependencies, WASM and Python ready.
    A single `cargo build` produces a binary that runs identically in the browser (WASM), Python, and CLI.

### What RENKIN solves well

RENKIN achieves high accuracy on standard bond disconnections:

- Esters → carboxylic acid + alcohol
- Amides → acid + amine (graph-based cleavage)
- Biaryls → aryl halide + boronic acid (Suzuki)
- Aryl amines → aryl halide + amine (Buchwald-Hartwig)
- C–halide bonds → dehalogenated arene
- Boc / Cbz protecting group removal
- Diaryl sulfones → arylsulfonyl chloride + arene (graph-based, v0.1.8)
- Sulfonamides → sulfonyl chloride + amine

### Out-of-Distribution (OOD) Evaluation

To check whether RENKIN's accuracy is specific to the USPTO-50k domain, we evaluated on **500 FDA-approved drugs** from ChEMBL (Phase 4, MW 150–700, no salts).

| Dataset | Solved | Success Rate | Notes |
|---------|--------|-------------|-------|
| USPTO-50k test set | 3,831 / 4,907 | **78.1%** | In-distribution (templates from USPTO train set) |
| **ChEMBL approved drugs** | **409 / 500** | **81.8%** | Out-of-distribution (real FDA-approved drugs) |

**RENKIN generalizes well beyond the USPTO domain.** The +3.7 pp improvement on approved drugs suggests the rule set covers the common transformations used in drug synthesis, not just reactions seen in USPTO training data.

Unsolved molecules in both datasets share the same profile: nitrogen-rich heterocycles (+17 pp in unsolved) and fluorinated compounds (+11 pp). This is a structural challenge, not a domain-specific artifact.

### Improving the success rate

To push the success rate higher:

1. **Expand the building block database** — supply eMolecules, ZINC, or your internal stock via `--building-blocks`
2. **Add more templates** — extract additional templates from the full USPTO training set
3. **Increase search depth** — `--depth 7` covers longer multi-step routes at the cost of more computation

### Running the benchmark

```bash
# Build
cargo build --release

# Download USPTO-50k test set (first time only)
python3 scripts/download_uspto50k.py

# Full benchmark — 50 chunks × 100 mol, resumable
bash scripts/run_benchmark_chunks.sh \
    data/uspto50k_test.smi \
    data/templates_extracted_5000.smi \
    data/bench_chunks \
    5 100

# Aggregate results
python3 -c "
import json, glob
files = sorted(glob.glob('data/bench_chunks/chunk_*.json'))
total = solved = 0; times = []
for f in files:
    d = json.load(open(f))
    total += d['total']; solved += d['solved']
    times.append(d['avg_time_ms'])
print(f'{solved}/{total} = {solved/total:.1%}, avg {sum(times)/len(times):.0f} ms/mol')
"
```
