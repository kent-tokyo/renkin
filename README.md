# RENKIN — Retrosynthesis Engine for Knowledge-Informed Navigation

> **Computer-Aided Synthesis Planning (CASP) · Pure Rust · WebAssembly · Python**  
> Named after 錬金 (れんきん, *renkin*) — Japanese for alchemy: just as alchemists transformed base metals into gold, RENKIN transforms target molecules back into cheap starting materials.

<p>
  <a href="https://github.com/kent-tokyo/renkin/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/kent-tokyo/renkin/actions/workflows/ci.yml/badge.svg?branch=master"></a>
  <a href="https://github.com/kent-tokyo/renkin/actions/workflows/docs.yml"><img alt="Docs" src="https://github.com/kent-tokyo/renkin/actions/workflows/docs.yml/badge.svg?branch=master"></a>
</p>

<p>
  <a href="https://crates.io/crates/renkin"><img alt="Crates.io" src="https://img.shields.io/crates/v/renkin.svg"></a>
  <a href="https://docs.rs/renkin"><img alt="docs.rs" src="https://docs.rs/renkin/badge.svg"></a>
  <a href="https://pypi.org/project/renkin/"><img alt="PyPI" src="https://img.shields.io/pypi/v/renkin.svg"></a>
  <a href="https://pypi.org/project/renkin/"><img alt="Python" src="https://img.shields.io/pypi/pyversions/renkin.svg"></a>
  <a href="https://www.npmjs.com/package/renkin"><img alt="npm" src="https://img.shields.io/npm/v/renkin.svg"></a>
  <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-blue.svg"></a>
</p>

<p>
  <img alt="Pure Rust" src="https://img.shields.io/badge/Pure%20Rust-100%25-orange?logo=rust">
  <img alt="unsafe forbidden" src="https://img.shields.io/badge/unsafe-forbidden-success.svg">
  <img alt="WASM" src="https://img.shields.io/badge/WASM-ready-brightgreen">
  <img alt="PyO3" src="https://img.shields.io/badge/PyO3-Python%20bindings-blue">
  <img alt="MCP" src="https://img.shields.io/badge/MCP-ready-7f52ff">
  <img alt="templates" src="https://img.shields.io/badge/templates-up%20to%2050k-purple">
  <img alt="building blocks" src="https://img.shields.io/badge/building%20blocks-509-lightgrey">
  <img alt="USPTO-50k" src="https://img.shields.io/badge/USPTO--50k-78.0%25%20solved-brightgreen">
  <img alt="ChEMBL" src="https://img.shields.io/badge/ChEMBL-81.8%25%20solved-brightgreen">
</p>

[日本語版 README](./README_ja.md) · [**Documentation**](https://kent-tokyo.github.io/renkin/) · [**Live Demo →**](https://kent-tokyo.github.io/renkin/playground/)

---

## What is RENKIN?

RENKIN is an open-source **retrosynthesis engine** for **computer-aided synthesis planning (CASP)** that automatically discovers optimal chemical reaction routes from a target molecule back to cheap, commercially available starting materials.

Built entirely in Rust with the [`chematic`](https://docs.rs/chematic/) cheminformatics crate. Zero C/C++ dependencies. All crates enforce `#![forbid(unsafe_code)]` — compiler-verified Pure Safe Rust throughout.

**[→ Try the Live Playground](https://kent-tokyo.github.io/renkin/playground/)** — runs entirely in WebAssembly, no installation needed.  
**[→ Full Documentation](https://kent-tokyo.github.io/renkin/)** — API reference, examples, benchmark.

---

## Why RENKIN?

RENKIN is designed as a Rust-native synthesis planning stack:

| | |
|---|---|
| **Fast** | A\* / AND-OR tree search with beam search and template frequency weighting |
| **Portable** | Native CLI · Python wheels · npm/WASM · browser playground — one codebase |
| **Explainable** | Per-step `confidence`, `atom_economy`, `route_cost`, and `procedure_hint` |
| **Verifiable** | `renkin-forward` validates each retrosynthetic step by forward-applying templates |
| **Benchmarkable** | USPTO-50k, PaRoutes-style evaluation, route diversity, and atom balance checks |
| **Agent-ready** | MCP server exposes routes and validation to Claude Desktop and AI agents |

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

[![Open In Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/kent-tokyo/renkin/blob/master/examples/renkin_quickstart.ipynb)

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
| **A\* / AND-OR Tree Search** | Retro\*-equivalent algorithm with pluggable heuristics (`MoleculeValueEstimator`, `ReactionPrior`) |
| **Up to 50k reaction templates** | Auto-extracted from USPTO-50k/MIT via rdchiral; frequency-weighted priority; `--templates` for custom sets |
| **Route scoring** | `confidence`, `step_confidence`, `success_probability` (Retro-prob style), `convergency`, `atom_economy` per step |
| **Route cost scoring** | `route_cost = Σ(BB cost) + steps×0.5`; actual prices via `--bb-prices CSV` or `--stock stock.csv` |
| **Pareto multi-objective search** | `--format pareto` returns a Pareto front across `route_cost`, `success_probability`, `steps`, etc.; objectives configurable via `--objectives cost:min,success_probability:max,steps:min` |
| **Constraint DSL** | `--constraints constraints.json` — JSON-driven synthesis planning: element filters, step limits, confidence thresholds, preferred reaction families; enables LLM → RENKIN pipeline |
| **Output formats** | `--format json` · `tree` · `mermaid` · `explain` (human-readable per-route analysis) · `compare` (side-by-side table) · `compare-json` · `pareto` |
| **Failure diagnostics** | Zero-route JSON output includes `diagnostics` block with `likely_causes` and `suggestions` |
| **Forward validation** | `renkin-forward validate` verifies each step by applying templates forward; accepts `--route-json` or stdin |
| **Plausibility report** | `renkin-bench --plausibility` — forward-validates best routes and reports composite plausibility score |
| **PaRoutes benchmark** | `renkin-bench --input-format paroutes` for multi-step ground-truth evaluation with `depth_delta` and `route_diversity` |
| **Atom balance check** | `renkin-bench` flags steps where `target_MW > Σ precursor_MW` (CompleteRXN reference) |
| **Stock CSV management** | `renkin stock stats\|validate\|coverage` — inspect and validate stock CSV files with SMILES, name, vendor, price, hazard fields |
| **Template quality tools** | `renkin template stats\|validate\|dedup\|explain\|coverage` — inspect SMIRKS template sets: frequency distribution, validity, duplicates, per-template lookup, coverage rate |
| **MCP server** | `renkin-mcp` exposes 6 tools: `find_routes`, `validate_route`, `explain_route`, `find_pareto_routes`, `plan_with_constraints`, `estimate_diversity` |
| **`renkin-doctor`** | Environment diagnostic binary — checks templates, building blocks, Python import, tool versions, and data integrity |
| **`renkin-kg`** | Reaction knowledge graph builder — constructs bipartite mol↔reaction graphs from routes; exports to GraphML or Cypher |
| **Beam search** | `--beam-width N` for memory-bounded exploration; `SmallVec<[FEntry; 6]>` stack-allocated frontier |
| **Parallel rule application** | `rayon` on non-WASM; sequential fallback on wasm32 |
| **tract-onnx NN scorer** | Pure Rust ONNX inference (no C++ dep) — optional `--scorer` flag for Phase B template relevance scoring |
| **`building_blocks` in JSON** | Each route includes the leaf starting-material SMILES — no manual step parsing needed |
| **Tetrahedral stereo @/@@** | Full stereochemistry support via chematic 0.4.16 |
| **Python** | `pip install renkin` — pre-built wheels for Linux/macOS/Windows |
| **WASM** | ~500 KB bundle — runs in the browser at near-native speed |
| **509 building blocks** | Aryl halides, boronic acids, heterocycles, amines, acids, amino acids |

---

## Pipeline Examples

```bash
# Route cost scoring with commercial prices
renkin -t "Cc1ccc(-c2ccccc2)cc1" --bb-prices data/prices.csv --format json

# Forward validation — pipe find_routes output directly
renkin -t "CC(=O)Oc1ccccc1C(=O)O" --format json | renkin-forward validate

# Faster template retrieval with bond-center index (~24% speedup)
renkin -t "c1ccc(NC(=O)c2ccccc2)cc1" --templates data/templates_extracted_5000.smi --bond-index
```

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
| **+ diaryl sulfone rule, 509 BBs** | **3826/4907** | **78.0%** | **509** | **5,000** | **5** | **100** | **≈2,800** |
| Cascade (stage2: depth=7, beam=300 on unsolved) | 4705/4907 | **95.9%** | 509 | 5,000 | 7 | 300 | — |

\* 29/50 chunks, previous binary  
† 50/50 chunks — **72.1%** (3,540/4,907) confirmed

Under RENKIN's evaluation setting (see definition above), RENKIN reaches **78.0%** single-pass on USPTO-50k, and **95.9%** with cascade search (re-running unsolved targets at depth=7, beam=300). Published numbers for AiZynthFinder (45–53%), Retro\* (44.3%), and ASKCOS (41%) use different stock databases, template counts, and evaluation years — **this is not a matched-condition comparison**.  
*Note: LocalRetro (53.4%) and GLG (58.0%) report single-step top-1 prediction accuracy — a different metric, not directly comparable.*  
[Full benchmark details →](https://kent-tokyo.github.io/renkin/benchmark/)

> **Benchmark scope note**: USPTO-50k is used here as a *standardized sanity benchmark*, not as proof of broad real-world synthesis performance. The corpus covers a narrow slice of reaction space (primarily C–C and C–N bond formations common in pharmaceutical synthesis), and reaction types with sparse USPTO representation are systematically underserved. Out-of-distribution performance on ChEMBL approved drugs (**81.8%**, 409/500) suggests the rule set generalizes beyond the test corpus, but neither number should be interpreted as a guarantee of route quality on arbitrary targets.

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
| **★ RENKIN** | **Rust** | **MIT** | **Yes** | **Yes** | **A\* + AND/OR** | Hand-curated + rdchiral (5k default; 50k via `--templates`) | 509+ |

**RENKIN's goal**: match state-of-the-art accuracy using only curated rules and auto-extracted SMIRKS templates — no GPU, no training data, no black boxes. Under RENKIN's benchmark setting, it reaches **78.0%** (3,826/4,907 — full run confirmed) single-pass, and **95.9%** (4,705/4,907) with cascade search. Template frequency weighting (Phase A) combined with 5,000 auto-extracted templates and 509 building blocks delivers this result. RENKIN runs anywhere: browser, CLI, Python — single `cargo build`.

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

**Tools** (6):

| Tool | Description |
|---|---|
| `find_routes` | Retrosynthesis: SMILES → routes with scoring |
| `validate_route` | Forward-validate a retrosynthetic route |
| `explain_route` | Human-readable strengths/weaknesses per route |
| `find_pareto_routes` | Pareto-front multi-objective route search |
| `plan_with_constraints` | Constraint-DSL planning (element filters, step limits, confidence thresholds) |
| `estimate_diversity` | Route diversity and coverage metrics |

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
│  renkin  (retrosynthesis)         renkin-forward                  │
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
│  - 20 built-in + up to 50k via --templates  │     fragment sanitization + ring-leak filter
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
renkin/                          ← Cargo workspace root
├── Cargo.toml
├── src/                         ← renkin crate (retrosynthesis)
│   ├── lib.rs                   # public library
│   ├── main.rs                  # CLI binary (--templates, --scorer, --constraints, --objectives flags)
│   ├── bin/benchmark.rs         # renkin-bench binary (--plausibility flag)
│   ├── bin/doctor.rs            # renkin-doctor diagnostic binary
│   ├── bin/fp.rs                # renkin-fp ECFP4 fingerprint (nn-scoring feature)
│   ├── bin/mcp.rs               # renkin-mcp MCP server (6 tools)
│   ├── chem_env.rs              # retro rules + BB lookup + template loader
│   ├── score.rs                 # SA Score heuristic + step cost
│   ├── search.rs                # A* / AND-OR tree engine + beam pruning
│   ├── scorer.rs                # Phase B: tract-onnx NN template scorer
│   ├── python.rs                # PyO3 bindings (--features python)
│   └── wasm.rs                  # wasm-bindgen bindings (cfg = wasm32)
├── crates/                      ← sibling crates
│   ├── renkin-forward/          # forward reaction prediction (reactants → products)
│   └── renkin-kg/               # reaction knowledge graph builder (GraphML / Cypher export)
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

### Recently shipped

- [x] `renkin-bench cascade` — multi-stage search (fast defaults → hard cases re-run deeper); only unsolved targets propagate to later stages. **78.0% → 95.9%** on USPTO-50k
- [x] `renkin-bench --failure-taxonomy` — classify unsolved targets by cause (beam limit / depth limit / template gap / stock near-miss)
- [x] Graph-based ester cleavage — BFS-leakage-free `R-C(=O)-O-R' → RCOOH + R'OH`
- [x] `--top-templates N` — frequency-rank filter: use the top-N most frequent templates for speed / less noise
- [x] `raw / validated / practical` solved-rate metrics (`--plausibility --practical-max-steps N`)
- [x] Retro cache hit-rate in `SearchStats` + `--verbose`

### In progress

- [ ] Template retrieval index (element bitmask + bond-center prefilter) for the 50k template set
- [ ] Calibrated route confidence (map `success_probability` to empirical solve rate)

### Next

- [ ] Graph rule expansion — sulfonamide / carbamate / urea cleavage (one PR per family, each with benchmark delta)
- [ ] Stock-aware planning (price / hazard / availability re-ranking)

<details>
<summary>Earlier milestones</summary>

- [x] Route cost scoring — `route_cost` field + `--bb-prices path.csv` / `--stock stock.csv`
- [x] Cargo workspace — `crates/renkin-forward/` + `crates/renkin-kg/`
- [x] `renkin-forward predict` / `validate` — forward prediction + route validation (stdin-pipe friendly)
- [x] `renkin-doctor` — environment diagnostic binary (templates, BBs, Python, binaries)
- [x] Failure diagnostics — zero-route output includes `likely_causes` + `suggestions` JSON block
- [x] `--format explain|compare|compare-json` — human-readable and tabular route output
- [x] `renkin stock stats|validate|coverage` — stock CSV management subcommand
- [x] Pareto multi-objective search — `--format pareto`, `--objectives`, `find_pareto_routes` MCP
- [x] Constraint DSL — `--constraints JSON`, `plan_with_constraints` MCP tool
- [x] `renkin template stats|validate|dedup|explain|coverage` — template quality tools
- [x] `renkin-kg` — reaction knowledge graph (bipartite mol↔reaction, GraphML/Cypher export)
- [x] MCP server expanded to 6 tools (`explain_route`, `find_pareto_routes`, `plan_with_constraints`)
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
- [x] Auto template extraction (rdchiral): **27.8%** → **78.0%** USPTO-50k
- [x] Tetrahedral stereo @/@@ + E/Z double-bond stereo
- [x] Template frequency weighting (Phase A): **72.1%** USPTO-50k
- [x] FxHashMap · SmallVec beam frontier · SA Score memoization · Arc<PathNode> path sharing
- [x] 5,000 extracted templates + 509 BBs: **78.0%** USPTO-50k (3,826/4,907 ✅)
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
  title     = {{RENKIN}: Retrosynthesis Engine for Knowledge-Informed Navigation},
  year      = {2026},
  url       = {https://github.com/kent-tokyo/renkin/releases/tag/v0.15.5},
  version   = {0.15.5},
  license   = {MIT}
}
```

---

## Security

Report vulnerabilities via [GitHub Private vulnerability reporting](https://github.com/kent-tokyo/renkin/security/advisories/new). See [SECURITY.md](SECURITY.md).

---

## License

MIT

---

*GitHub Topics: `retrosynthesis` `cheminformatics` `wasm` `rust` `drug-discovery` `casp` `synthesis-planning` `computational-chemistry`*

---

If RENKIN saves you time, a GitHub star helps others discover it.
