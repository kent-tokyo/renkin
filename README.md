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

Constraints compose freely and are enforced in two layers:
- `--avoid-elements` **prunes expansions during search** when a BB precursor contains a forbidden element (no dead-end nodes added to the heap).
- A final route-level post-filter is still applied for correctness.
- `--require-elements` is a route-level post-filter only.

Add `--verbose` to print search statistics (nodes expanded, elapsed time) to stderr. Performance counters are available in native builds only; disabled in WASM.

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
| **`building_blocks` in JSON** | Each route includes the leaf starting-material SMILES — no manual step parsing needed |
| **MCP server** | `renkin-mcp` binary — AI agents (Claude, etc.) call retrosynthesis over JSON-RPC stdio |
| **Tetrahedral stereo @/@@** | Full stereochemistry support via chematic 0.4.16 |
| **Python** | `pip install renkin` — pre-built wheels for Linux/macOS/Windows |
| **WASM** | ~500 KB bundle — runs in the browser at near-native speed |
| **509 building blocks** | Aryl halides, boronic acids, heterocycles, amines, acids, amino acids |

---

## Benchmark

USPTO-50k test set (4,907 molecules, full evaluation):

> **Evaluation definition**: A molecule is *solved* if `find_routes` returns at least one route whose leaf precursors are all in the 509-reagent building block set, within depth=5 and beam=100. Ground-truth reactants from USPTO-50k are **not** checked — any commercially accessible route counts.

> **Evaluation note**: All numbers use the standard USPTO-50k train/test split (same corpus). Templates are extracted from the training set and evaluated on the test set. Numbers reflect performance within the USPTO-50k domain; out-of-distribution generalization is separately evaluated via ChEMBL approved drugs (**81.8%**, 409/500).

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

Under RENKIN's evaluation setting (see definition above), RENKIN reaches **78.1%** on USPTO-50k. Published numbers for AiZynthFinder (45–53%), Retro\* (44.3%), and ASKCOS (41%) use different stock databases, template counts, and evaluation years — **this is not a matched-condition comparison**.  
*Note: LocalRetro (53.4%) and GLG (58.0%) report single-step top-1 prediction accuracy — a different metric, not directly comparable.*  
[Full benchmark details →](https://kent-tokyo.github.io/renkin/benchmark/)

### PaRoutes compatibility

RENKIN is compatible with the [PaRoutes](https://github.com/AstraZeneca/PaRoutes) multi-step benchmark. Download their stock compounds and target molecules, then pass them directly:

```bash
renkin-bench \
  --input paroutes_n1_targets.smi \
  --building-blocks paroutes_stock.smi \
  --templates data/templates_extracted_5000.smi \
  --depth 5 --beam-width 100
```

The JSON output includes `avg_nodes_expanded`, `avg_confidence`, `avg_convergency`, and `avg_success_prob` (Retro-prob style) alongside the standard solved/success_rate metrics.

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

**RENKIN's goal**: match state-of-the-art accuracy using only curated rules and auto-extracted SMIRKS templates — no GPU, no training data, no black boxes. Under RENKIN's benchmark setting, it reaches **78.1%** (3,831/4,907 — full run confirmed). Template frequency weighting (Phase A) combined with 5,000 auto-extracted templates and 509 building blocks delivers this result. RENKIN runs anywhere: browser, CLI, Python — single `cargo build`.

> ⚠️ The table above lists tools under different evaluation conditions. No matched-condition experiment against other tools has been performed.

---

## MCP Server

`renkin-mcp` exposes retrosynthesis as an MCP tool so AI agents (Claude, etc.) can call it directly.

**Setup** — add to `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "renkin": { "command": "/path/to/renkin-mcp" }
  }
}
```

**Tool**: `find_routes(smiles, depth?, max_routes?, avoid_elements?, require_elements?)`

The server auto-detects `data/building_blocks.smi` and `data/templates_extracted_5000.smi` in the working directory. Falls back to the embedded 509-BB / 20-rule defaults if not found.

```bash
cargo build --release
# binary: target/release/renkin-mcp
```

---

## Architecture

### Workspace scope

```
┌──────────────────────────────────────────────────────────────────┐
│ renkin workspace (this repository)                               │
│                                                                  │
│  renkin  (retrosynthesis)         renkin-forward  (planned)      │
│  ──────────────────────           ─────────────────────────────  │
│  target → precursors              reactants → products           │
│  A* / AND-OR search               template-based forward         │
│  route scoring & constraints      (validates retro routes)       │
│        │                                    │                    │
│        └──────────────────┬─────────────────┘                    │
│                           ▼                                      │
│               chematic  (molecular representation,               │
│               SMILES, substructure matching, reaction SMARTS)    │
└──────────────────────────────────────────────────────────────────┘
```

### Internal data flow (renkin crate)

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
renkin/                          ← Cargo workspace root (planned)
├── Cargo.toml
├── src/                         ← renkin crate (retrosynthesis)
│   ├── lib.rs                   # public library
│   ├── main.rs                  # CLI binary  (--templates, --scorer flags)
│   ├── bin/benchmark.rs         # renkin-bench binary  (--templates flag)
│   ├── chem_env.rs              # 5,000 retro rules, BB check, template loader
│   ├── score.rs                 # SA Score heuristic + step cost
│   ├── search.rs                # A* / AND-OR tree engine + beam pruning
│   ├── scorer.rs                # Phase B: tract-onnx NN template scorer
│   ├── python.rs                # PyO3 bindings (--features python)
│   └── wasm.rs                  # wasm-bindgen bindings (cfg = wasm32)
├── crates/                      ← sibling crates (in development)
│   └── renkin-forward/          # forward reaction prediction (reactants → products)
├── data/
│   ├── building_blocks.smi              # 509 curated commercial starting materials
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

- [x] Route cost scoring — `route_cost` field + `--bb-prices path.csv` flag (SA Score proxy or real prices)
- [ ] Cargo workspace restructure — `crates/renkin-forward/` sibling crate
- [ ] `renkin-forward`: template-based forward reaction prediction (reactants → products)
- [ ] Optional forward validation of retrosynthetic routes via `renkin-forward`

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
- [x] MCP server (`renkin-mcp`) — AI agents call retrosynthesis directly
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
  version   = {0.2.0},
  license   = {MIT}
}
```

---

## License

MIT

---

*GitHub Topics: `retrosynthesis` `cheminformatics` `wasm` `rust` `drug-discovery` `casp` `synthesis-planning` `computational-chemistry`*
