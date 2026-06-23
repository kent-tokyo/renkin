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
[![unsafe forbidden](https://img.shields.io/badge/unsafe-forbidden-success.svg)](https://github.com/rust-secure-code/safety-dance/)
[![Open In Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/kent-tokyo/renkin/blob/master/examples/renkin_quickstart.ipynb)

[日本語版 README](./README_ja.md) · [**Documentation**](https://kent-tokyo.github.io/renkin/) · [**Live Demo →**](https://kent-tokyo.github.io/renkin/playground/)

---

## What is RENKIN?

RENKIN is an open-source **retrosynthesis engine** for **computer-aided synthesis planning (CASP)** that automatically discovers optimal chemical reaction routes from a target molecule back to cheap, commercially available starting materials.

Built entirely in Rust with the [`chematic`](https://docs.rs/chematic/) cheminformatics crate. Zero C/C++ dependencies. All crates enforce `#![forbid(unsafe_code)]` — compiler-verified Pure Safe Rust throughout.

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
    --templates data/templates_extracted_5000.smi
```

---

## Key Features

| Feature | Detail |
|---|---|
| **Pure Safe Rust** | `#![forbid(unsafe_code)]` on all crates — compiler-enforced, zero C/C++ dependencies |
| **A\* / AND-OR Tree Search** | Retro\*-equivalent algorithm, proven more efficient than MCTS |
| **SA Score heuristic** | Admissible h = Σ(1 + 0.5·(sa−1)/9) guides toward accessible precursors |
| **SA Score memoization** | Per-search cache avoids redundant SA Score computation on repeated intermediates |
| **Beam search** | `--beam-width N` for memory-bounded exploration; `SmallVec<[FEntry; 6]>` stack-allocated frontier |
| **5,000 reaction templates** | Auto-extracted from USPTO-50k training set via rdchiral; frequency-weighted beam priority |
| **Template frequency weighting** | Phase A: `weight = ln(count+1)` from USPTO training set; high-frequency templates preferred in beam search (+19 pp) |
| **Element pre-screening** | `required_elements` bitset skips impossible rules before SMARTS matching |
| **apply_retro memoization** | SMARTS VF2 skip on repeated intermediates — per-search cache |
| **Arc<PathNode> path sharing** | Persistent linked-list; O(1) per child instead of O(depth) clone |
| **FxHashMap / FxHashSet** | rustc-hash replacing std collections throughout for faster hashing |
| **Auto template extraction** | `scripts/extract_templates.py` — rdchiral + chematic-compatible simplification |
| **Graph-based biaryl cleavage** | Bridge-bond DFS for correct Suzuki disconnection |
| **Parallel rule application** | `rayon` on non-WASM; sequential fallback on wasm32 |
| **tract-onnx NN scorer** | Pure Rust ONNX inference (no C++ dep) — optional `--scorer` flag for Phase B template relevance scoring |
| **Tetrahedral stereo @/@@** | Full stereochemistry support via chematic 0.4.16 |
| **Python** | `pip install renkin` — pre-built wheels for Linux/macOS/Windows |
| **WASM** | ~500 KB bundle — runs in the browser at near-native speed |
| **480 building blocks** | Aryl halides, boronic acids, heterocycles, amines, acids, amino acids |

---

## Benchmark

USPTO-50k test set (4,907 molecules, full evaluation):

> **Evaluation note**: All numbers use the standard USPTO-50k train/test split (same corpus). Templates are extracted from the training set and evaluated on the test set — the same methodology as AiZynthFinder and other published tools. Numbers reflect performance within the USPTO-50k domain; out-of-distribution generalization has not been separately evaluated.

| Config | Solved | Rate | BBs | Templates | depth | beam | ms/mol |
|---|---|---|---|---|---|---|---|
| v0.1.0 initial | 366/4907 | 7.5% | 463 | 31 | 3 | 50 | — |
| + auto templates (top-300) | 1363/4907 | 27.8% | 463 | 222 | 3 | 50 | — |
| + depth=5, top-500 templates | 2315/4907 | 47.2% | 463 | 314 | 5 | 50 | — |
| + beam=100 | 2688/4907 | 54.8%* | 463 | 314 | 5 | 100 | — |
| + Phase A (template freq. weighting) | 3540/4907 | 72.1%† | 463 | 314 | 5 | 100 | — |
| **+ 5,000 templates, 480 BBs** | **3826/4907** | **78.0%** | **480** | **5,000** | **5** | **100** | **2,775** |
| Phase A unlimited (beam=0) | 3832/4907 | 78.1% | 480 | 5,000 | 5 | 0 | — |
| Phase B (NN scorer, tract-onnx) | 3826/4907 | 78.0% | 480 | 5,000 | 5 | 100 | 3,394 |

\* 29/50 chunks, previous binary  
† 50/50 chunks — **72.1%** (3,540/4,907) confirmed

On the standard USPTO-50k benchmark (multi-step route-finding, same train/test split), RENKIN (**78.0%**) exceeds the published numbers for AiZynthFinder (45–53%), Retro\* (44.3%), and ASKCOS (41%) — though those are from 2019–2020 papers with different BB/template counts, so no matched-condition experiment exists yet.  
*Note: LocalRetro (53.4%) and GLG (58.0%) report single-step top-1 prediction accuracy — a different metric, not directly comparable.*  
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
| **★ RENKIN** | **Rust** | **MIT** | **Yes** | **Yes** | **A\* + AND/OR** | Hand-curated + rdchiral (5,000) | 480+ |

**RENKIN's goal**: match or exceed neural-network-based tools using only curated rules and auto-extracted SMIRKS templates — no GPU, no training data, no black boxes. On the standard USPTO-50k benchmark (same train/test split used by all published tools), RENKIN reaches **78.0%** (3,826/4,907 — full 4,907-molecule run confirmed). Template frequency weighting (Phase A) — the same principle as AiZynthFinder's neural template scoring — combined with 5,000 auto-extracted templates and 480 building blocks delivers this result. RENKIN runs anywhere: browser, CLI, Python — single `cargo build`.

---

## Architecture

```
Target SMILES
     │
     ▼
┌─────────────────────────┐
│     chem_env.rs         │  ← chematic wrapper
│  - SMILES parse         │     canonical-SMILES FxHashSet BB lookup (O(1))
│  - 5,000 retro rules    │     fragment sanitization + ring-leak filter
│  - Building block check │     apply_retro memoization cache
└────────────┬────────────┘
             │  par_iter (rayon / sequential on WASM)
             ▼
┌─────────────────────────┐
│      search.rs          │  ← A* / AND-OR Tree Search
│  - Priority queue       │     SA Score heuristic + memoization
│  - Closed list          │     beam search (SmallVec frontier)
│  - Arc<PathNode> paths  │     O(1) path sharing per child
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
┌─────────────────────────┐   (optional)
│      scorer.rs          │  ← Phase B: NN Template Scorer
│  - tract-onnx           │     Pure Rust ONNX inference
│  - --scorer flag        │     molecule-specific template ranking
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
│   ├── main.rs              # CLI binary  (--templates, --scorer flags)
│   ├── bin/benchmark.rs     # renkin-bench binary  (--templates flag)
│   ├── chem_env.rs          # 5,000 retro rules, BB check, template loader
│   ├── score.rs             # SA Score heuristic + step cost
│   ├── search.rs            # A* / AND-OR tree engine + beam pruning
│   ├── scorer.rs            # Phase B: tract-onnx NN template scorer
│   ├── python.rs            # PyO3 bindings (--features python)
│   └── wasm.rs              # wasm-bindgen bindings (cfg = wasm32)
├── data/
│   ├── building_blocks.smi              # 480 curated commercial starting materials
│   ├── templates_extracted_5000.smi     # 5,000 auto-extracted SMIRKS templates
│   ├── benchmark_targets.smi            # internal benchmark set
│   └── bench_chunks/                    # USPTO-50k per-chunk results
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
- [x] **Phase 15** — Tetrahedral stereo @/@@ support via chematic 0.4.16 ✅
- [x] **Phase 15a** — E/Z double-bond stereo filtering active via chematic-rxn 0.4.15 (issue #21)
- [x] **Phase 17** — chematic 0.4.12: Bug #13 (BFS leakage) + Bug #14 (canonical SMILES) fixed
- [x] **Phase 18** — Template frequency weighting (Phase A): **72.1%** USPTO-50k (3,540/4,907 — full run ✅)
- [x] **Phase 19** — Rust engine micro-optimizations (split_fragments, is_bb fast path, element pre-screening)
- [x] **Phase 20** — FxHashMap/FxHashSet (rustc-hash) replacing std collections throughout
- [x] **Phase 21** — SmallVec<[FEntry; 6]> beam frontier (stack allocation)
- [x] **Phase 22** — SA Score memoization cache per search
- [x] **Phase 23** — Arc<PathNode> persistent linked-list for path sharing (O(1) per child)
- [x] **Phase 24** — apply_retro memoization cache (SMARTS VF2 skip on repeated intermediates)
- [x] **Phase 25** — 5,000 extracted templates + 480 BBs: **78.0%** USPTO-50k (3,826/4,907 ✅, 2,775 ms/mol)
- [x] **Phase B** — NN template scorer via `--scorer` flag (tract-onnx, Pure Rust ONNX, no C++ dep) ✅
- [x] **`#![forbid(unsafe_code)]`** — compiler-enforced Pure Safe Rust on all crates
- [ ] **Phase 16** — Large-scale building block DB integration (500k BBs — in progress)

---

## Citation

If you use RENKIN in academic work, please cite:

```bibtex
@software{renkin2026,
  author    = {kent-tokyo},
  title     = {{RENKIN}: Retrosynthetic Exploration Network for Knowledge-Informed Navigation},
  year      = {2026},
  url       = {https://github.com/kent-tokyo/renkin},
  version   = {0.1.4},
  license   = {MIT}
}
```

---

## License

MIT

---

*GitHub Topics: `retrosynthesis` `cheminformatics` `wasm` `rust` `drug-discovery` `casp` `synthesis-planning` `computational-chemistry`*
