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
    --templates data/templates_extracted_5000.smi --format tree
```

```text
Target: CC(=O)Oc1ccccc1C(=O)O
Routes found: 3

Route 1  [score=1.02, depth=1]
OC(=O)c1ccccc1OC(=O)C
└── [extracted_169]
    ├── OC(=O)C  ✓ BB
    └── [OH]c1ccccc1C(=O)O  ✓ BB

Route 2  [score=1.02, depth=1]
OC(=O)c1ccccc1OC(=O)C
└── [extracted_145]
    ├── CC(=O)Cl  ✓ BB
    └── [OH]c1ccccc1C(=O)O  ✓ BB

Route 3  [score=1.03, depth=1]
OC(=O)c1ccccc1OC(=O)C
└── [extracted_238]
    ├── c1cccc(c1O)C(O)=O  ✓ BB
    └── C([OH])(=O)C  ✓ BB
```

Use `--format mermaid` for GitHub/Notion-compatible flowcharts.

---

## Constraint-based Search

Restrict routes by the element composition of their building blocks.

**Default search** — all 5 routes for biphenyl:

```bash
renkin --target "c1ccc(-c2ccccc2)cc1" --templates data/templates_extracted_5000.smi --format tree
```

```text
Routes found: 5
Route 1  [score=1.00, depth=1]  c1ccccc1Br + c1c(B(O)O)cccc1
Route 2  [score=1.03, depth=1]  c1ccccc1Br + c1c(B(O)O)cccc1
Route 3  [score=1.06, depth=1]  c1cc(Cl)ccc1 + c1c(B(O)O)cccc1
Route 4  [score=1.08, depth=1]  c1(I)ccccc1  + c1c(B(O)O)cccc1
Route 5  [score=1.08, depth=1]  c1ccccc1Br  + c1(B2OC(C(C)(C)O2)(C)C)ccccc1
```

**Constrained search** — boronic-acid coupling, no Br or I starting materials:

```bash
renkin --target "c1ccc(-c2ccccc2)cc1" --templates data/templates_extracted_5000.smi \
    --require-elements "B" --avoid-elements "Br,I" --format tree
```

```text
Routes found: 1

Route 1  [score=1.06, depth=1]
c1ccccc1-c2ccccc2
└── [extracted_398]
    ├── c1cc(Cl)ccc1  ✓ BB
    └── c1c(B(O)O)cccc1  ✓ BB
```

Constraints compose freely. Applied as a post-filter on completed routes — the A\* search itself is unchanged.

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
| **Route visualization** | `--format tree` ASCII tree · `--format mermaid` GitHub/Notion flowchart |
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
| + 5,000 templates, 480 BBs | 3826/4907 | 78.0% | 480 | 5,000 | 5 | 100 | 2,775 |
| Phase A unlimited (beam=0) | 3832/4907 | 78.1% | 480 | 5,000 | 5 | 0 | — |
| Phase B (NN scorer, tract-onnx) | 3826/4907 | 78.0% | 480 | 5,000 | 5 | 100 | 3,394 |
| **+ diaryl sulfone rule, 509 BBs** | **3831/4907** | **78.1%** | **509** | **5,000** | **5** | **100** | **≈2,800** |

\* 29/50 chunks, previous binary  
† 50/50 chunks — **72.1%** (3,540/4,907) confirmed

On the standard USPTO-50k benchmark (multi-step route-finding, same train/test split), RENKIN (**78.1%**) exceeds the published numbers for AiZynthFinder (45–53%), Retro\* (44.3%), and ASKCOS (41%) — though those are from 2019–2020 papers with different BB/template counts, so no matched-condition experiment exists yet.  
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
| **★ RENKIN** | **Rust** | **MIT** | **Yes** | **Yes** | **A\* + AND/OR** | Hand-curated + rdchiral (5,000) | 509+ |

**RENKIN's goal**: match or exceed neural-network-based tools using only curated rules and auto-extracted SMIRKS templates — no GPU, no training data, no black boxes. On the standard USPTO-50k benchmark (same train/test split used by all published tools), RENKIN reaches **78.1%** (3,831/4,907 — full 4,907-molecule run confirmed). Template frequency weighting (Phase A) — the same principle as AiZynthFinder's neural template scoring — combined with 5,000 auto-extracted templates and 509 building blocks delivers this result. RENKIN runs anywhere: browser, CLI, Python — single `cargo build`.

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

- [ ] MCP server — AI agents call retrosynthesis directly
- [ ] Route cost scoring (commercial reagent price integration)

<details>
<summary>Completed milestones</summary>

- [x] SMIRKS retro-reaction rules + fragment sanitization
- [x] A\* / AND-OR tree search, closed list, degenerate-route filter
- [x] SA Score heuristic + beam search
- [x] Parallel rule application (rayon; sequential fallback on WASM)
- [x] Python bindings (PyO3 + maturin) · `pip install renkin`
- [x] WASM build · `npm install renkin`
- [x] Benchmark CLI (`renkin-bench`) + USPTO-50k evaluation
- [x] WASM browser playground + i18n (EN/JA/ZH)
- [x] Graph-based biaryl cleavage · O(1) canonical-SMILES BB index
- [x] Published to crates.io / PyPI / npm · GitHub Actions CI/CD
- [x] MkDocs documentation site · GitHub Pages playground
- [x] Auto template extraction (rdchiral): **27.8%** → **78.1%** USPTO-50k
- [x] Tetrahedral stereo @/@@ + E/Z double-bond stereo
- [x] Template frequency weighting (Phase A): **72.1%** USPTO-50k
- [x] FxHashMap · SmallVec beam frontier · SA Score memoization · Arc<PathNode> path sharing
- [x] 5,000 extracted templates + 509 BBs: **78.1%** USPTO-50k (3,831/4,907 ✅)
- [x] NN template scorer via `--scorer` flag (tract-onnx, Pure Rust ONNX)
- [x] `--format tree|mermaid` route visualization
- [x] Constraint-based search: `--avoid-elements`, `--require-elements`
- [x] `--verbose` search statistics to stderr
- [x] `#![forbid(unsafe_code)]` — compiler-enforced Pure Safe Rust

</details>

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
