# RENKIN — **R**etrosynthetic **E**xploration **N**etwork for **K**nowledge-**I**nformed **N**avigation

> **Computer-Aided Synthesis Planning (CASP) · Pure Rust · WebAssembly · Python**  
> Named after 錬金 (れんきん, *renkin*) — Japanese for alchemy: just as alchemists transformed base metals into gold, RENKIN transforms target molecules back into cheap starting materials.

[![CI](https://github.com/kent-tokyo/renkin/actions/workflows/ci.yml/badge.svg)](https://github.com/kent-tokyo/renkin/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/renkin)](https://crates.io/crates/renkin)
[![PyPI](https://img.shields.io/pypi/v/renkin)](https://pypi.org/project/renkin/)
[![npm](https://img.shields.io/npm/v/renkin)](https://www.npmjs.com/package/renkin)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
[![WASM](https://img.shields.io/badge/WASM-ready-brightgreen)](https://kent-tokyo.github.io/renkin/playground/)
[![Pure Rust](https://img.shields.io/badge/Pure-Rust-orange?logo=rust)](https://www.rust-lang.org)

[日本語版 README](./README_ja.md) · [**Documentation**](https://kent-tokyo.github.io/renkin/) · [**Live Demo →**](https://kent-tokyo.github.io/renkin/playground/)

---

## What is RENKIN?

RENKIN is an open-source **retrosynthesis engine** for **computer-aided synthesis planning (CASP)** that automatically discovers optimal chemical reaction routes from a target molecule back to cheap, commercially available starting materials.

Built entirely in Rust with the [`chematic`](https://docs.rs/chematic/) cheminformatics crate. Zero C/C++ dependencies.

**[→ Try the Live Playground](https://kent-tokyo.github.io/renkin/playground/)** — runs entirely in WebAssembly, no installation needed.  
**[→ Full Documentation](https://kent-tokyo.github.io/renkin/)** — API reference, examples, benchmark.

---

## Installation

```bash
pip install renkin          # Python
cargo add renkin            # Rust
npm install renkin          # JavaScript / Node.js
```

---

## Quick Start

```python
import renkin

result = renkin.find_routes(
    "CC(=O)Oc1ccccc1C(=O)O",   # Aspirin
    depth=5,
    max_routes=3,
)

for route in result["routes"]:
    for step in route["steps"]:
        print(f"  {step['target']} → {' + '.join(step['precursors'])}  [{step['rule']}]")
```

```javascript
import init, { find_routes } from './pkg/renkin.js';
await init();
const result = JSON.parse(find_routes("CC(=O)Oc1ccccc1C(=O)O", 5, 3, 0));
```

```bash
./target/release/renkin --target "CC(=O)Oc1ccccc1C(=O)O" --depth 5
```

---

## Key Features

| Feature | Detail |
|---|---|
| **Pure Rust** | Zero C/C++ dependencies — cross-platform with `cargo build` alone |
| **A\* / AND-OR Tree Search** | Retro\*-equivalent algorithm, proven more efficient than MCTS |
| **SA Score heuristic** | Admissible h = Σ(1 + 0.5·(sa−1)/9) guides toward accessible precursors |
| **Beam search** | `--beam-width N` for memory-bounded exploration |
| **Graph-based biaryl cleavage** | Bridge-bond DFS for correct Suzuki disconnection (no SMIRKS BFS artifacts) |
| **Parallel rule application** | `rayon` on non-WASM; sequential fallback on wasm32 |
| **Python** | `pip install renkin` — pre-built wheels for Linux/macOS/Windows |
| **WASM** | 493 KB bundle — runs in the browser at near-native speed |
| **480+ building blocks** | Aryl halides, boronic acids, heterocycles, pharmaceutical amines, amino acids |
| **20 reaction rules** | See table below |

---

## Retro-Rules (20 total)

| Rule | Reaction type | Strategy |
|---|---|---|
| `ester_cleavage` | Ester → acid + alcohol | SMIRKS |
| `amide_cleavage` | Amide → acid + amine | SMIRKS |
| `friedel_crafts_acylation_retro` | Ar-C(=O)R → Ar-H + acyl chloride | SMIRKS |
| `aryl_carboxylation_retro` | Ar-COOH → Ar-H + CO₂ surrogate | SMIRKS |
| `aryl_amine_retro` | Ar-N → Ar-H + amine | SMIRKS |
| `buchwald_hartwig_retro` | Ar-N → Ar-Br + amine | SMIRKS |
| `aryl_ether_retro` | Ar-O → Ar-OH + fragment | SMIRKS |
| `aryl_chloride_retro` | Ar-Cl → Ar-H (retro-SNAr / Pd C-Cl) | SMIRKS |
| `aryl_iodide_retro` | Ar-I → Ar-H (retro-Pd/Cu C-I) | SMIRKS |
| `aryl_fluoride_snAr_retro` | Ar-F → Ar-H (retro-SNAr) | SMIRKS |
| `aryl_chloride_to_bromide` | Ar-Cl → Ar-Br (halogen exchange) | SMIRKS |
| `suzuki_retro` | Ar-Ar → Ar-Br + Ar-H | **Graph** (bridge-bond DFS) |
| `heck_retro` | Ar-CH=CH-R → Ar-Br + vinyl | SMIRKS |
| `negishi_retro` | Ar-CH₂ → Ar-Br + alkyl | SMIRKS |
| `cc_single_cleavage` | C–C → two fragments | SMIRKS |
| `wittig_retro` | C=C → C=O + C=O | SMIRKS |
| `reductive_amination_retro` | C–N → C=O + amine | SMIRKS |
| `cn_aliphatic_cleavage` | C–N → two fragments | SMIRKS |
| `co_aliphatic_cleavage` | C–O → two fragments | SMIRKS |
| `alcohol_oxidation_retro` | C–OH → C=O | SMIRKS |

---

## Benchmark

USPTO-50k test set (500-molecule random sample):

| Config | Solved | Rate | BBs | Rules |
|---|---|---|---|---|
| v0.1.0 (depth=2, beam=20) | 13/500 | 2.6% | 277 | 14 |
| current (depth=2, beam=20) | **25/500** | **5.0%** | **480+** | **20** |

**79 ms/molecule** on Apple M-series, single-threaded. [Full benchmark details →](https://kent-tokyo.github.io/renkin/benchmark/)

---

## Competitive Landscape

| Tool | Language | License | WASM | Zero-dep | Algorithm | Template source | Stock |
|---|---|---|---|---|---|---|---|
| **ASKCOS** | Python | CC BY-NC | No | No (Docker, 64 GB) | MCTS + A\* | USPTO (ML) | ZINC |
| **AiZynthFinder** | Python | MIT | No | No (conda + model) | MCTS | USPTO (ML, ~50k) | eMolecules (~6M) |
| **SYNTHIA** | Closed | Proprietary | No | No | SMARTS + AND/OR | Manual curated | Sigma-Aldrich |
| **IBM RXN** | Closed | Cloud SaaS | No | No | Transformer | USPTO | — |
| **Retro\*** | Python | MIT | No | No (unmaintained) | A\* + AND/OR | USPTO (ML) | eMolecules |
| **MEGAN** | Python | MIT | No | No (PyTorch) | Graph Transformer | USPTO | — |
| **★ RENKIN** | **Rust** | **MIT** | **Yes** | **Yes** | **A\* + AND/OR** | Hand-curated (20) | 480+ (extensible) |

**RENKIN's niche**: portable, embeddable, zero-dependency CASP engine for integration into pipelines that cannot afford Docker/conda environments or require browser/edge deployment. Not designed to maximize USPTO-50k recall — designed to maximize deployability.

---

## Architecture

```
Target SMILES
     │
     ▼
┌─────────────────────────┐
│     chem_env.rs         │  ← chematic wrapper
│  - SMILES parse         │     SMARTS VF2 building-block check
│  - 20 SMIRKS retro rules│     fragment sanitization
│  - Building block check │     HashMap O(1) pre-filter
└────────────┬────────────┘
             │  par_iter (rayon / sequential on WASM)
             ▼
┌─────────────────────────┐
│      search.rs          │  ← A* / AND-OR Tree Search
│  - Priority queue       │     SA Score heuristic
│  - Closed list          │     beam search pruning
│  - Degenerate filter    │
└────────────┬────────────┘
             │
             ▼
┌─────────────────────────┐
│      score.rs           │  ← Heuristic / Cost Function
│  - SA Score (chematic)  │     h = Σ(1 + 0.5·(sa−1)/9)
│  - MW step cost         │     g = Σ(1 + total_mw/2000)
└────────────┬────────────┘
             │
             ▼
  JSON  ←  CLI / Python / WASM
```

---

## Project Structure

```
renkin/
├── Cargo.toml
├── CHANGELOG.md
├── src/
│   ├── lib.rs           # public library
│   ├── main.rs          # CLI binary
│   ├── bin/benchmark.rs # renkin-bench binary
│   ├── chem_env.rs      # chematic wrapper — 20 retro rules, BB check
│   ├── score.rs         # SA Score heuristic + step cost
│   ├── search.rs        # A* / AND-OR tree engine + beam pruning
│   ├── python.rs        # PyO3 bindings (--features python)
│   └── wasm.rs          # wasm-bindgen bindings (cfg = wasm32)
├── data/
│   ├── building_blocks.smi      # 480+ commercial starting materials
│   ├── benchmark_targets.smi   # 42-molecule internal benchmark set
│   └── uspto50k_benchmark_result.json
├── demo/index.html      # Local WASM demo (serve with python3 -m http.server)
├── docs/                # MkDocs source → kent-tokyo.github.io/renkin/
│   ├── index.md
│   ├── getting_started/
│   ├── api/
│   ├── examples/
│   ├── benchmark.md
│   └── playground/index.html   # → /playground/
└── mkdocs.yml
```

---

## Roadmap

- [x] **Phase 1** — SMIRKS retro-reaction rules + fragment sanitization
- [x] **Phase 2** — A\* / AND-OR tree search, closed list, degenerate-route filter
- [x] **Phase 3** — SA Score heuristic + beam search
- [x] **Phase 4** — Parallel rule application (rayon; sequential fallback on WASM)
- [x] **Phase 5** — Python bindings (PyO3 + maturin) · `pip install renkin`
- [x] **Phase 6** — WASM build · `npm install renkin`
- [x] **Phase 7** — Benchmark CLI (`renkin-bench`) + initial USPTO-50k evaluation
- [x] **Phase 8** — 23 unit tests · rules 5 → 20 · building blocks 30 → 480+
- [x] **Phase 9** — WASM browser playground + benchmark target set (42 mol)
- [x] **Phase 10** — Graph-based biaryl cleavage · O(1) BB HashMap index
- [x] **Phase 11** — Published to crates.io / PyPI / npm · GitHub Actions CI/CD
- [x] **Phase 12** — MkDocs documentation site · GitHub Pages playground
- [ ] **Phase 13** — Formal USPTO-50k benchmark vs. AiZynthFinder / Retro\*
- [ ] **Phase 14** — Automatic template extraction from USPTO-50k train set (rdchiral)
- [ ] **Phase 15** — Stereochemistry support (CIP SMIRKS)
- [ ] **Phase 16** — Large-scale building block DB (eMolecules / ZINC integration)
- [ ] **Phase 17** — chematic upstream fixes (#13 BFS leakage, #14 canonical SMILES)

---

## License

MIT

---

*GitHub Topics: `retrosynthesis` `cheminformatics` `wasm` `rust` `drug-discovery` `casp` `synthesis-planning` `computational-chemistry`*
