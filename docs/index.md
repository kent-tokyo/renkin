---
title: "RENKIN: Open-Source Retrosynthesis Engine for Python, Rust and WebAssembly"
description: "Plan multi-step synthesis routes from SMILES with RENKIN, a pure-Rust computer-aided synthesis planning engine available for Python, CLI, Rust and WebAssembly."
---

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
    result = renkin.find_routes(target="CC(=O)Oc1ccccc1C(=O)O", depth=5)
    ```

## Key Features

| Feature | Details |
|---------|---------|
| **Pure Rust** | Zero C/C++ dependencies — safe, fast, cross-platform |
| **WebAssembly** | Runs in the browser at near-native speed |
| **Python bindings** | `pip install renkin` — no RDKit required |
| **26 hand-crafted rules + up to 50k extracted via `--templates`** | Ester, amide, Suzuki, Heck, Wittig, sulfonamide, and more; extended via rdchiral-extracted templates |
| **Building blocks** | 402 unique compounds in `data/building_blocks.smi` (used when found relative to the current working directory); otherwise CLI/Python fall back to a compiled-in 152-compound set, which WASM always uses. Pass `--building-blocks`/`building_blocks=` to specify explicitly |
| **A\* / beam search** | Frequency-weighted A* with beam-width control; `step_cost` reduced for high-frequency templates (Phase A) |
| **Route scoring** | Per-step `confidence`, `success_probability` (Retro-prob), `route_cost` with optional `--bb-prices CSV` |
| **Stable template IDs + evidence sidecar** | Every template has a stable `template_id`; attach curated conditions/yields/warnings via `--template-metadata` — see [Template Evidence](https://github.com/kent-tokyo/renkin#template-evidence-metadata) |
| **Constraint DSL** | `--avoid-elements Br,I --require-elements B` filters routes by element profile |
| **Forward validation** | `renkin-forward validate` verifies each retrosynthetic step by forward prediction; pipe-friendly (stdin support) |
| **Failure diagnostics** | `renkin-bench --failure-taxonomy` classifies unsolved targets by cause (beam limit, depth limit, template gap, stock near-miss) |
| **Cascade search** | Two-stage search: fast defaults → hard cases re-run at higher beam/depth |
| **Stability testing** | `--quietset-out` exports observations for [quietset](https://crates.io/crates/quietset-cli) cross-config stability analysis |
| **MCP server** | `renkin-mcp` exposes `find_routes`, `diagnose_failure`, `validate_route` to Claude Desktop |

## Quick Example

=== "Python"

    ```python
    --8<-- "examples/quickstart.py"
    ```

=== "Rust"

    ```rust
    --8<-- "examples/quickstart.rs"
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
  Retrosynthetic   ←── 27 built-in + up to 50k extracted (--templates)
  rule application
        │
        ▼
  Precursor set    ←── Check against building block stock (402 file / 152 fallback)
        │
        ▼
  A* / BFS search  ←── Beam width, depth limit
        │
        ▼
  Synthetic routes (depth, steps, precursors)
```

## Reaction Rules

RENKIN ships **26 hand-crafted rules** (a mix of graph-based dispatch and SMIRKS-based patterns) covering common pharmaceutical bond disconnections, plus supports up to 50k rdchiral-extracted templates via `--templates`:

- **Acyl disconnections**: ester hydrolysis, amide cleavage (graph-based), Friedel-Crafts acylation, acyl chloride formation
- **Aryl C-heteroatom**: Ullmann ether (C-O), sulfonamide formation, decarboxylation
- **Aryl C-halide**: chloride/bromide halogen exchange
- **Aryl C-C coupling**: Suzuki (graph-based), Heck, Negishi, Sonogashira
- **Sulfone disconnections**: diaryl sulfone cleavage (graph-based)
- **Protecting groups**: Boc, Cbz deprotection (graph-based)
- **Aliphatic**: reductive amination, N-benzylation, Wittig, Grignard addition, Claisen condensation, Michael addition
- **Oxidation**: alcohol → carbonyl

See [Benchmark](benchmark.md) for current USPTO-50k results and methodology — historical figures (78.0%/95.9%/81.8%) shown elsewhere on the web are invalidated and not representative of current performance; do not cite them.

## Installation

=== "pip"

    ```bash
    pip install renkin
    ```

=== "cargo"

    ```toml
    [dependencies]
    renkin = "0.21"
    ```

=== "npm"

    ```bash
    npm install renkin
    ```

See [Installation](getting_started/installation.md) for details.
