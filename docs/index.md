# RENKIN

> **Computer-Aided Synthesis Planning (CASP) · Pure Rust · WebAssembly · Python**  
> Named after 錬金 (*renkin*) — Japanese for alchemy: just as alchemists transformed base metals into gold, RENKIN transforms target molecules back into cheap starting materials.

[![CI](https://github.com/kent-tokyo/renkin/actions/workflows/ci.yml/badge.svg)](https://github.com/kent-tokyo/renkin/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/renkin)](https://crates.io/crates/renkin)
[![PyPI](https://img.shields.io/pypi/v/renkin)](https://pypi.org/project/renkin/)
[![npm](https://img.shields.io/npm/v/renkin)](https://www.npmjs.com/package/renkin)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](https://github.com/kent-tokyo/renkin/blob/master/LICENSE)

## What is RENKIN?

RENKIN is a **retrosynthesis engine** that automatically plans multi-step chemical syntheses by working backwards from a target molecule to commercially available starting materials. Given a target SMILES, it searches for synthetic routes using a library of retrosynthetic reaction rules.

## Try It Now

=== "Browser (no install)"
    [**→ Open Playground**](playground/){ .md-button .md-button--primary }

    Runs entirely in WebAssembly — no server, no installation.

=== "Google Colab (Python)"
    [![Open In Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/kent-tokyo/renkin/blob/master/examples/renkin_quickstart.ipynb)

    One-click Python notebook — `pip install renkin` + aspirin example + RDKit visualization.

=== "Python"
    ```bash
    pip install renkin
    ```
    ```python
    import renkin
    result = renkin.find_routes("CC(=O)Oc1ccccc1C(=O)O", depth=5)
    ```

## Key Features

| Feature | Details |
|---------|---------|
| **Pure Rust** | Zero C/C++ dependencies — safe, fast, cross-platform |
| **WebAssembly** | Runs in the browser at near-native speed |
| **Python bindings** | `pip install renkin` — no RDKit required |
| **20 built-in rules + up to 50k via `--templates`** | Ester, amide, Suzuki, Buchwald-Hartwig, Wittig, and more; extended via rdchiral-extracted templates |
| **509 building blocks** | Common pharma starting materials pre-loaded |
| **A\* / AND-OR tree search** | Retro\*-equivalent algorithm with beam-width control and pluggable heuristics (`MoleculeValueEstimator`, `ReactionPrior`) |
| **Route scoring** | Per-step `confidence`, `success_probability` (Retro-prob), `route_cost` with optional `--bb-prices CSV` |
| **Forward validation** | `renkin-forward validate` verifies each retrosynthetic step by forward prediction; pipe-friendly (stdin support) |
| **PaRoutes benchmark** | `renkin-bench --input-format paroutes` for multi-step ground-truth evaluation with depth delta and route diversity metrics |
| **Atom balance check** | `renkin-bench` flags steps where `target_MW > Σ precursor_MW` (CompleteRXN-style) |
| **MCP server** | `renkin-mcp` exposes `find_routes`, `validate_route`, `estimate_diversity` to Claude Desktop |

## Quick Example

=== "Python"

    ```python
    import renkin

    routes = renkin.find_routes(
        smiles="CC(=O)Oc1ccccc1C(=O)O",  # Aspirin
        depth=5,
        max_routes=3,
    )

    for route in routes["routes"]:
        print(f"Route (depth {route['depth']}):")
        for step in route["steps"]:
            print(f"  {step['target']} → {' + '.join(step['precursors'])}")
            print(f"  via {step['rule']}")
    ```

=== "Rust"

    ```rust
    use renkin::{chem_env::ChemEnv, search::{SearchConfig, find_routes}};

    let env = ChemEnv::load("data/building_blocks.smi")?;
    let config = SearchConfig { max_depth: 5, ..Default::default() };
    let routes = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &config)?;

    for route in &routes {
        println!("Route depth: {}", route.depth());
    }
    ```

=== "JavaScript (WASM)"

    ```javascript
    import init, { find_routes } from './pkg/renkin.js';

    await init();
    const result = JSON.parse(find_routes("CC(=O)Oc1ccccc1C(=O)O", 5, 3, 0));
    console.log(`Found ${result.routes_found} routes`);
    ```

## How It Works

```
Target molecule (SMILES)
        │
        ▼
  Retrosynthetic   ←── 20 built-in rules (5,000 via --templates)
  rule application
        │
        ▼
  Precursor set    ←── Check against 509 building blocks
        │
        ▼
  A* / BFS search  ←── Beam width, depth limit
        │
        ▼
  Synthetic routes (depth, steps, precursors)
```

## Reaction Rules

RENKIN includes 20 retrosynthetic reaction templates covering the most common bond-forming reactions in pharmaceutical synthesis:

- **Acyl disconnections**: ester hydrolysis, amide cleavage, Friedel-Crafts acylation
- **Aryl C-heteroatom**: Buchwald-Hartwig (C-N), Ullmann ether (C-O), SNAr
- **Aryl C-halide**: C-Cl, C-I, C-F disconnections (Pd-activation / SNAr retro)
- **Aryl C-C coupling**: Suzuki (graph-based), Heck, Negishi
- **Aliphatic**: reductive amination, N-alkylation, O-alkylation, Wittig
- **Oxidation**: alcohol → carbonyl

## Installation

=== "pip"

    ```bash
    pip install renkin
    ```

=== "cargo"

    ```toml
    [dependencies]
    renkin = "0.1"
    ```

=== "npm"

    ```bash
    npm install renkin
    ```

See [Installation](getting_started/installation.md) for details.
