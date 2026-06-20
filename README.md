# RENKIN — Retrosynthesis Engine

> **Computer-Aided Synthesis Planning (CASP) · Pure Rust · WebAssembly · Python**  
> Named after 錬金 (れんきん, *renkin*) — Japanese for alchemy: just as alchemists transformed base metals into gold, RENKIN transforms target molecules back into cheap starting materials.

[![Crates.io](https://img.shields.io/crates/v/renkin)](https://crates.io/crates/renkin)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
[![WASM](https://img.shields.io/badge/WASM-ready-brightgreen)](https://github.com/kent-tokyo/renkin/tree/master/demo)
[![Pure Rust](https://img.shields.io/badge/Pure-Rust-orange?logo=rust)](https://www.rust-lang.org)

[日本語版 README](./README_ja.md)

---

## What is RENKIN?

RENKIN is an open-source **retrosynthesis engine** for **computer-aided synthesis planning (CASP)** that automatically discovers optimal chemical reaction routes from a target molecule back to cheap, commercially available starting materials — a core problem in **drug discovery** and **medicinal chemistry**.

Built entirely in Rust with the [`chematic`](https://docs.rs/chematic/) cheminformatics crate, RENKIN solves the fundamental speed and dependency problems of existing Python-based CASP tools (AiZynthFinder, ASKCOS, Retro\*, etc.). It ships as:

- **CLI** — single binary, `cargo build --release`
- **Python package** — `import renkin` via PyO3 + maturin
- **WASM module** — 493 KB bundle, runs in the browser with no server

All from a single pure-Rust codebase with zero C/C++ dependencies.

---

## Key Features

| Feature | Detail |
|---|---|
| **Pure Rust** | Zero C/C++ dependencies. Cross-platform with `cargo build` alone |
| **A\* / AND-OR Tree Search** | Retro\*-equivalent algorithm proven more efficient than MCTS for retrosynthesis |
| **SA Score heuristic** | `chematic::chem::sa_score` guides search toward synthetically accessible precursors |
| **Beam search** | `--beam-width N` limits heap size for memory-bounded exploration |
| **Graph-based Ar–Ar cleavage** | Bridge-bond detection via DFS — correctly handles biaryl (Suzuki) disconnections |
| **Parallel rule application** | `rayon` parallelises SMIRKS rule evaluation; sequential fallback on WASM |
| **Python bindings** | `maturin` extension — `import renkin; renkin.find_routes(...)` |
| **WASM-ready** | 493 KB bundle via `wasm-pack`; browser demo with 2D structure rendering |
| **~400 building blocks** | Curated commercial starting materials covering esters, amines, halides, heterocycles, amino acids, sulfonyl chlorides, boronic acids and more |
| **Benchmark CLI** | `renkin-bench --input targets.smi` produces a JSON success/timing report |

---

## Architecture

```
Target SMILES
     │
     ▼
┌─────────────────────────┐
│     chem_env.rs         │  ← chematic wrapper
│  - SMILES parse         │     SMARTS VF2 building-block check
│  - SMIRKS retro rules   │     fragment sanitization
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

## Technology Stack

- **Language**: Rust (Edition 2024)
- **Cheminformatics**: [`chematic`](https://crates.io/crates/chematic) v0.4.9+
  - `chematic-smiles` — SMILES parsing & canonical SMILES
  - `chematic-smarts` — VF2 substructure matching (building block identity)
  - `chematic-rxn` — SMIRKS reaction application (`run_reactants`)
  - `chematic-chem` — SA Score, molecular weight, aromaticity descriptors
- **Search**: A\* + AND/OR Tree (Retro\* equivalent)
- **Parallelism**: [`rayon`](https://crates.io/crates/rayon) — parallel SMIRKS rule application
- **Python**: [`PyO3`](https://pyo3.rs) + [`maturin`](https://www.maturin.rs)
- **WASM**: [`wasm-bindgen`](https://rustwasm.github.io/wasm-bindgen/) + [`wasm-pack`](https://rustwasm.github.io/wasm-pack/)

---

## Installation

### As a library

```toml
# Cargo.toml
[dependencies]
renkin = "0.1"
```

### CLI (from source)

```bash
git clone https://github.com/kent-tokyo/renkin
cd renkin
cargo build --release
```

### Python

```bash
pip install maturin
git clone https://github.com/kent-tokyo/renkin && cd renkin
python -m venv .venv && source .venv/bin/activate
maturin develop --features python
```

---

## Getting Started

### CLI

```bash
# Retrosynthesis (Aspirin, depth 3)
./target/release/renkin --target "CC(=O)Oc1ccccc1C(=O)O" --depth 3

# With beam search (top-50 nodes)
./target/release/renkin --target "CC(=O)Oc1ccccc1C(=O)O" --depth 5 --beam-width 50
```

```
--target / -t      Target molecule SMILES
--depth  / -d      Max retrosynthesis depth (default: 5)
--max-routes / -n  Max routes to return (default: 5)
--beam-width / -w  Beam search width, 0 = unlimited A* (default: 0)
--building-blocks  Path to .smi file of commercial starting materials
```

### Python

```python
import renkin, json

routes = json.loads(renkin.find_routes(
    "CC(=O)Oc1ccccc1C(=O)O",   # Aspirin
    depth=3,
    max_routes=5,
))
print(routes["routes_found"])   # number of routes found
for r in routes["routes"]:
    print(r["depth"], [s["rule"] for s in r["steps"]])
```

### WASM

```bash
wasm-pack build --target web --no-default-features
# Output: pkg/  (npm-ready package)
# Browser demo: python3 -m http.server 8080 → http://localhost:8080/demo/
```

```javascript
import init, { find_routes } from './pkg/renkin.js';
await init();

const result = JSON.parse(find_routes(
  "CC(=O)Oc1ccccc1C(=O)O",  // target SMILES
  3,   // depth
  5,   // max_routes
  0,   // beam_width (0 = unlimited A*)
));
console.log(result.routes_found);
```

### Benchmark

```bash
# Input: one SMILES per line, optional name after whitespace
./scripts/run_benchmark.sh --input data/benchmark_targets.smi --depth 5
```

```json
{
  "total": 42, "solved": 37, "success_rate": 0.88,
  "avg_depth": 1.05, "avg_time_ms": 2.5,
  "results": [...]
}
```

---

## CLI Output Example

```json
{
  "target": "CC(=O)Oc1ccccc1C(=O)O",
  "routes_found": 2,
  "routes": [
    {
      "steps": [
        {
          "rule": "ester_cleavage",
          "target": "CC(=O)Oc1ccccc1C(=O)O",
          "precursors": ["CC(=O)O", "Oc1ccccc1C(=O)O"]
        }
      ],
      "depth": 1
    }
  ]
}
```

**depth: 0** means the target itself is a commercially available starting material (buy directly).

---

## Retro-Rules (14 total)

| Rule | Reaction type | Strategy |
|---|---|---|
| `ester_cleavage` | Ester → acid + alcohol | SMIRKS |
| `amide_cleavage` | Amide → acid + amine | SMIRKS |
| `friedel_crafts_acylation_retro` | Ar-C(=O)R → Ar-H + acyl chloride | SMIRKS |
| `aryl_carboxylation_retro` | Ar-COOH → Ar-H + CO₂ surrogate | SMIRKS |
| `aryl_amine_retro` | Ar-N → Ar-H + amine | SMIRKS |
| `buchwald_hartwig_retro` | Ar-N → Ar-Br + amine | SMIRKS |
| `aryl_ether_retro` | Ar-O → Ar-OH + fragment | SMIRKS |
| `suzuki_retro` | Ar-Ar → Ar-Br + Ar-H | Graph (bridge-bond DFS) |
| `cc_single_cleavage` | C–C → two fragments | SMIRKS |
| `wittig_retro` | C=C → C=O + C=O | SMIRKS |
| `reductive_amination_retro` | C–N → C=O + amine | SMIRKS |
| `cn_aliphatic_cleavage` | C–N → two fragments | SMIRKS |
| `co_aliphatic_cleavage` | C–O → two fragments | SMIRKS |
| `alcohol_oxidation_retro` | C–OH → C=O | SMIRKS |

`suzuki_retro` uses a graph-based bridge-bond algorithm instead of SMIRKS to correctly handle symmetric biaryls (biphenyl, 4-fluorobiphenyl, etc.) without the BFS leakage artifacts that affect SMIRKS-based approaches.

---

## Project Structure

```
renkin/
├── Cargo.toml
├── src/
│   ├── lib.rs           # public library (DEFAULT_BUILDING_BLOCKS, re-exports)
│   ├── main.rs          # CLI binary
│   ├── bin/
│   │   └── benchmark.rs # renkin-bench binary
│   ├── chem_env.rs      # chematic wrapper — parse, retro rules, BB check
│   ├── score.rs         # SA Score heuristic + step cost
│   ├── search.rs        # A* / AND-OR tree engine + beam pruning
│   ├── python.rs        # PyO3 bindings (--features python)
│   └── wasm.rs          # wasm-bindgen bindings (cfg = wasm32)
├── data/
│   ├── building_blocks.smi      # Commercial starting materials (~400 entries)
│   └── benchmark_targets.smi   # 42-molecule benchmark set
├── demo/
│   └── index.html       # Browser WASM demo with 2D structure rendering
└── scripts/
    └── run_benchmark.sh # Benchmark runner with human-readable summary
```

---

## Roadmap

- [x] **Phase 1** — SMIRKS retro-reaction rules + fragment sanitization
- [x] **Phase 2** — A\* / AND-OR tree search, closed list, degenerate-route filter
- [x] **Phase 3** — SA Score heuristic + beam search (`--beam-width`)
- [x] **Phase 4** — Parallel rule application (`rayon`; sequential fallback on WASM)
- [x] **Phase 5** — Python bindings (PyO3 + maturin)
- [x] **Phase 6** — WASM build (493 KB, `pkg/` npm-ready)
- [x] **Phase 7** — Benchmark CLI (`renkin-bench`)
- [x] **Phase 8** — 21 unit tests, SMIRKS rules 5→14, building blocks ~30→~400
- [x] **Phase 9** — Browser WASM demo (SmilesDrawer 2D rendering), benchmark target set
- [x] **Phase 10** — Graph-based biaryl cleavage (suzuki_retro), O(1) BB HashMap index
- [ ] **Phase 11** — Formal benchmark vs. AiZynthFinder / Retro\* on USPTO-50k
- [ ] **Phase 12** — PyPI / npm publish

---

## Competitive Landscape

| Tool | Language | Algorithm | WASM | Zero-dep build |
|---|---|---|---|---|
| **ASKCOS** | Python | MCTS / A\* | No | No (Docker, 64 GB RAM) |
| **AiZynthFinder** | Python | MCTS primary | No | No (conda, model download) |
| **IBM RXN** | Closed | Transformer | No | No (cloud only) |
| **SYNTHIA** | Closed | SMARTS + AND/OR | No | No (proprietary) |
| **Retro\*** | Python | A\* + AND/OR | No | No (unmaintained) |
| **★ RENKIN** | **Rust** | **A\* + AND/OR** | **Yes** | **Yes (`cargo build`)** |

All existing open CASP tools are Python-based. RENKIN fills the vacant niche: Rust-native, WASM-deployable, zero-dependency, A\* search.

---

## License

MIT

---

*GitHub Topics: `retrosynthesis` `cheminformatics` `wasm` `rust` `drug-discovery` `casp` `synthesis-planning` `computational-chemistry`*
