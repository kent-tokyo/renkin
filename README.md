# RENKIN

RENKIN = **R**etrosynthetic **E**xploration **N**etwork for **K**nowledge-**I**nformed **N**avigation

> Ultra-comfortable, lightweight, and blazingly fast retrosynthesis engine.  
> Named after 錬金 (れんきん, *renkin*) — Japanese for alchemy: just as alchemists transformed base metals into gold, RENKIN transforms target molecules back into cheap starting materials.

[日本語版 README](./README_ja.md)

---

## What is RENKIN?

RENKIN is a **retrosynthesis engine** that automatically discovers optimal chemical reaction routes from a target molecule back to cheap, commercially available starting materials.

Built entirely in Rust with the [`chematic`](https://docs.rs/chematic/) crate, RENKIN addresses the fundamental speed and dependency problems of existing Python-based CASP tools (AiZynthFinder, ASKCOS, etc.). It ships as a CLI, a Python package, and a WASM module — all from the same Rust codebase.

---

## Key Features

| Feature | Detail |
|---|---|
| **Pure Rust** | Zero C/C++ dependencies. Cross-platform with `cargo build` alone |
| **A\* / AND-OR Tree Search** | Retro\*-equivalent algorithm proven more efficient than MCTS |
| **SA Score heuristic** | `chematic::chem::sa_score` guides search toward synthetically accessible precursors |
| **Beam search** | `--beam-width N` limits heap size for memory-bounded exploration |
| **Parallel rule application** | `rayon` parallelises SMIRKS rule evaluation; falls back to sequential on WASM |
| **Python bindings** | `maturin` extension — `import renkin; renkin.find_routes(...)` |
| **WASM-ready** | 493 KB bundle via `wasm-pack`; runs in the browser with no server |
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
│  - Building block check │
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
  - `chematic-smarts` — VF2 substructure matching (building block identity check)
  - `chematic-rxn` — SMIRKS reaction application (`run_reactants`)
  - `chematic-chem` — SA Score, molecular weight, aromaticity descriptors
- **Search**: A\* + AND/OR Tree (Retro\* equivalent)
- **Parallelism**: [`rayon`](https://crates.io/crates/rayon) — parallel SMIRKS rule application
- **Python**: [`PyO3`](https://pyo3.rs) + [`maturin`](https://www.maturin.rs)
- **WASM**: [`wasm-bindgen`](https://rustwasm.github.io/wasm-bindgen/) + [`wasm-pack`](https://rustwasm.github.io/wasm-pack/)

---

## Getting Started

### CLI

```bash
# Build
cargo build --release

# Retrosynthesis (Aspirin, depth 3)
./target/release/renkin --target "CC(=O)Oc1ccccc1C(=O)O" --depth 3

# With beam search (top-50 nodes)
./target/release/renkin --target "CC(=O)Oc1ccccc1C(=O)O" --depth 5 --beam-width 50

# Options
./target/release/renkin --help
```

```
--target / -t      Target molecule SMILES
--depth  / -d      Max retrosynthesis depth (default: 5)
--max-routes / -n  Max routes to return (default: 5)
--beam-width / -w  Beam search width, 0 = unlimited A* (default: 0)
--building-blocks  Path to .smi file of commercial starting materials
```

### Python

```bash
# Install (requires Python ≥ 3.8 and maturin)
python -m venv .venv && source .venv/bin/activate
maturin develop --features python
```

```python
import renkin, json

routes = json.loads(renkin.find_routes(
    "CC(=O)Oc1ccccc1C(=O)O",   # Aspirin
    depth=3,
    max_routes=5,
))
print(routes["routes_found"])   # 2
for r in routes["routes"]:
    print(r["depth"], [s["rule"] for s in r["steps"]])
```

### WASM

```bash
wasm-pack build --target web --no-default-features
# Output: pkg/  (npm-ready package)
```

```javascript
import init, { find_routes } from './pkg/renkin.js';
await init();

const result = JSON.parse(find_routes(
  "CC(=O)Oc1ccccc1C(=O)O",  // target SMILES
  3,   // depth
  5,   // max_routes
  0,   // beam_width (0 = unlimited)
));
console.log(result.routes_found);
```

### Benchmark

```bash
# Input: one SMILES per line, optional name after whitespace
cargo run --bin renkin-bench -- --input targets.smi --depth 3
```

```json
{
  "total": 7, "solved": 7, "success_rate": 1.0,
  "avg_depth": 0.57, "avg_time_ms": 2.9,
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
          "target": "c1cccc(c1OC(=O)C)C(=O)O",
          "precursors": ["c1c(C(=O)O)cccc1", "OC(C)=O", "C", "Oc1c(cccc1)C(O)=O"]
        }
      ],
      "depth": 1
    }
  ]
}
```

**depth: 0** means the target itself is a commercially available starting material.

---

## Retro-Rules

| Rule | Reaction type | SMIRKS |
|---|---|---|
| `ester_cleavage` | Ester → acid + alcohol | `[C:1](=[O:2])[O:3]>>[C:1](=[O:2])O.[O:3]` |
| `amide_cleavage` | Amide → acid + amine | `[C:1](=[O:2])[N:3]>>[C:1](=[O:2])O.[N:3]` |
| `aryl_carboxylation_retro` | Ar-COOH → Ar + CO₂ | `[c:1][C:2](=O)O>>[c:1].[C:2](=O)O` |
| `aryl_amine_retro` | Ar-NH₂ → Ar + NH₃ | `[c:1][N:2]>>[c:1].[N:2]` |
| `cc_single_cleavage` | C–C cleavage | `[C:1][C:2]>>[C:1].[C:2]` |

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
│   └── building_blocks.smi  # Commercial starting materials (~30 entries)
└── pkg/                 # WASM npm package (generated by wasm-pack)
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
- [ ] **Phase 7+** — Formal benchmark vs. AiZynthFinder / Retro\* on USPTO-50k

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
