# Benchmark

## USPTO-50k Test Set

We evaluate RENKIN on a 500-molecule random sample from the [USPTO-50k](https://huggingface.co/datasets/bisectgroup/USPTO_50K) test set — the standard benchmark for single-step retrosynthesis.

### Results (v0.1.0)

| Config | Solved | Success Rate |
|--------|--------|-------------|
| depth=2, beam=20, 480 BBs | 25 / 500 | **5.0%** |
| depth=2, beam=20, 277 BBs | 13 / 500 | 2.6% |

Average time: **79 ms/molecule** on Apple M-series (single-threaded).

### Context

| System | Success Rate | Stock | Templates |
|--------|-------------|-------|-----------|
| **RENKIN v0.1.0** | **5.0%** | **480 BBs** | **20 rules** |
| AiZynthFinder (Mol. Inf. 2020) | ~45% | eMolecules (~6M) | ~50,000 |
| Retro\* (ICML 2020) | ~40% | eMolecules (~6M) | ~50,000 |

!!! note "Apples vs oranges"
    The gap reflects fundamentally different approaches:
    
    - AiZynthFinder / Retro\* use **millions of stock compounds** and **thousands of ML-extracted reaction templates**
    - RENKIN uses **480 hand-curated building blocks** and **20 expert-written reaction rules**
    
    RENKIN's goal is a **pure Rust, zero-dependency, embeddable engine** that users can extend with their own rules and stock databases. Speed and portability, not maximum recall.

### What RENKIN solves

RENKIN excels at molecules where standard disconnections apply cleanly:

- Esters → carboxylic acid + alcohol
- Amides → acid + amine  
- Biaryl systems → aryl halide + boronic acid (Suzuki)
- Aryl amines → aryl halide + amine (Buchwald-Hartwig)
- Simple C-halide bonds → dehalogenated arene

### Improving the success rate

To push the success rate higher:

1. **Expand the building block database** — use eMolecules, ZINC, or your organization's internal stock
2. **Add reaction rules** — extract templates from USPTO training data using atom-mapped reactions and `rdchiral`
3. **Increase depth** — depth=3+ covers multi-step routes at the cost of more computation

### Running the benchmark

```bash
# Download USPTO-50k test set
python3 scripts/download_uspto50k.py  # requires internet

# Run benchmark
cargo build --bin renkin-bench --release
./target/release/renkin-bench \
    --input data/uspto50k_test.smi \
    --building-blocks data/building_blocks.smi \
    --depth 2 \
    --beam-width 20 \
    > results.json

# Analyze results
python3 scripts/analyze_benchmark.py results.json
```
