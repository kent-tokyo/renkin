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
./target/release/renkin --target "CC(=O)Oc1ccccc1C(=O)O" --depth 5 \
    --templates data/templates_extracted.smi
```

---

## Key Features

| Feature | Detail |
|---|---|
| **Pure Rust** | Zero C/C++ dependencies — cross-platform with `cargo build` alone |
| **A\* / AND-OR Tree Search** | Retro\*-equivalent algorithm, proven more efficient than MCTS |
| **SA Score heuristic** | Admissible h = Σ(1 + 0.5·(sa−1)/9) guides toward accessible precursors |
| **Beam search** | `--beam-width N` for memory-bounded exploration |
| **314 reaction rules** | 31 hand-crafted + 283 auto-extracted from USPTO-50k via rdchiral |
| **Auto template extraction** | `scripts/extract_templates.py` — rdchiral + chematic-compatible simplification |
| **Graph-based biaryl cleavage** | Bridge-bond DFS for correct Suzuki disconnection |
| **Parallel rule application** | `rayon` on non-WASM; sequential fallback on wasm32 |
| **Python** | `pip install renkin` — pre-built wheels for Linux/macOS/Windows |
| **WASM** | ~500 KB bundle — runs in the browser at near-native speed |
| **463 building blocks** | Aryl halides, boronic acids, heterocycles, amines, acids, amino acids |

---

## Benchmark

USPTO-50k test set (4,907 molecules, full evaluation):

| Config | Solved | Rate | BBs | Rules | depth | beam |
|---|---|---|---|---|---|---|
| v0.1.0 initial | 366/4907 | 7.5% | 463 | 31 | 3 | 50 |
| + auto templates (top-300) | 1363/4907 | 27.8% | 463 | 222 | 3 | 50 |
| + depth=5, top-500 templates | 2315/4907 | 47.2% | 463 | 314 | 5 | 50 |
| + beam=100 | **~2550/4907** | **~52%** | 463 | 314 | 5 | 100 |

Surpasses AiZynthFinder lower bound (45%) and approaches upper bound (53%) with only 463 curated BBs and no neural networks.  
Competitor reference: AiZynthFinder 45–53% (depth≤5, 6M BBs, 50k templates).  
[Full benchmark details →](https://kent-tokyo.github.io/renkin/benchmark/)

---

## Competitive Landscape

| Tool | Language | License | WASM | Zero-dep | Algorithm | Template source | Stock |
|---|---|---|---|---|---|---|---|
| **ASKCOS** | Python | CC BY-NC | No | No (Docker, 64 GB) | MCTS + A\* | USPTO (ML) | ZINC |
| **AiZynthFinder** | Python | MIT | No | No (conda + model) | MCTS | USPTO (ML, ~50k) | eMolecules (~6M) |
| **SYNTHIA** | Closed | Proprietary | No | No | SMARTS + AND/OR | Manual curated | Sigma-Aldrich |
| **IBM RXN** | Closed | Cloud SaaS | No | No | Transformer | USPTO | — |
| **Retro\*** | Python | MIT | No | No (unmaintained) | A\* + AND/OR | USPTO (ML) | eMolecules |
| **★ RENKIN** | **Rust** | **MIT** | **Yes** | **Yes** | **A\* + AND/OR** | Hand-curated + rdchiral (314) | 463+ |

**RENKIN's goal**: match or exceed neural-network-based tools using only curated rules and auto-extracted SMIRKS templates — no GPU, no training data, no black boxes. At 47.2% USPTO-50k (surpassing ASKCOS and reaching AiZynthFinder's lower bound), RENKIN demonstrates that a transparent, rule-based engine in pure Rust can compete with deep-learning approaches. And it runs anywhere: browser, CLI, Python — single `cargo build`.

---

## Architecture

```
Target SMILES
     │
     ▼
┌─────────────────────────┐
│     chem_env.rs         │  ← chematic wrapper
│  - SMILES parse         │     canonical-SMILES HashSet BB lookup (O(1))
│  - 314 retro rules      │     fragment sanitization + ring-leak filter
│  - Building block check │     VF2 fallback for small sets
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
├── src/
│   ├── lib.rs               # public library
│   ├── main.rs              # CLI binary  (--templates flag)
│   ├── bin/benchmark.rs     # renkin-bench binary  (--templates flag)
│   ├── chem_env.rs          # 314 retro rules, BB check, template loader
│   ├── score.rs             # SA Score heuristic + step cost
│   ├── search.rs            # A* / AND-OR tree engine + beam pruning
│   ├── python.rs            # PyO3 bindings (--features python)
│   └── wasm.rs              # wasm-bindgen bindings (cfg = wasm32)
├── data/
│   ├── building_blocks.smi          # 463 curated commercial starting materials
│   ├── templates_extracted.smi      # 283 auto-extracted SMIRKS templates (top-500)
│   ├── benchmark_targets.smi        # internal benchmark set
│   └── bench_chunks/                # USPTO-50k per-chunk results
├── scripts/
│   ├── extract_templates.py         # rdchiral template extraction pipeline
│   └── run_benchmark_chunks.sh      # resumable chunked benchmark runner
├── docs/                # MkDocs source → kent-tokyo.github.io/renkin/
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
- [x] **Phase 8** — Unit tests · rules → 31 · building blocks → 463
- [x] **Phase 9** — WASM browser playground + i18n (EN/JA/ZH)
- [x] **Phase 10** — Graph-based biaryl cleavage · O(1) canonical-SMILES BB index
- [x] **Phase 11** — Published to crates.io / PyPI / npm · GitHub Actions CI/CD
- [x] **Phase 12** — MkDocs documentation site · GitHub Pages playground
- [x] **Phase 13** — Formal USPTO-50k benchmark: **7.5%** (depth=3, 31 rules)
- [x] **Phase 14** — Auto template extraction (rdchiral): **27.8%** (depth=3, 222 rules)
- [x] **Phase 17** — chematic 0.4.12: Bug #13 (BFS leakage) + Bug #14 (canonical SMILES) fixed
- [ ] **Phase 15** — Stereochemistry support (CIP SMIRKS)
- [ ] **Phase 16** — Large-scale building block DB integration

---

## License

MIT

---

*GitHub Topics: `retrosynthesis` `cheminformatics` `wasm` `rust` `drug-discovery` `casp` `synthesis-planning` `computational-chemistry`*
