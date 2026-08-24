---
title: "Rust Retrosynthesis Library: RENKIN API Reference"
description: "RENKIN's core Rust API: ChemEnv, SearchConfig, find_routes, RetroRule, and the feature flags for Python bindings and the NN template scorer."
---

# Rust API

## Core Types

### `ChemEnv`

The chemical environment holding the building block database.

```rust
use renkin::chem_env::{ChemEnv, mol_from_smiles};

// Load from SMILES file (402 unique compounds when this file is present --
// `?` propagates an error if it isn't; ChemEnv::load itself has no fallback).
let env = ChemEnv::load("data/building_blocks.smi")?;

// Load from in-memory list
let env = ChemEnv::in_memory(&["CC(=O)O", "c1ccccc1", "Brc1ccccc1"]);

// Check if a molecule is in the stock
let mol = mol_from_smiles("CC(=O)O")?;
assert!(env.is_building_block(&mol));
```

### `SearchConfig`

Configuration for the retrosynthesis search. Has a manual `Default` impl (not
`#[derive(Default)]`) — always use `..Default::default()` for fields you don't
set explicitly, since new fields are added over time.

```rust
use renkin::search::SearchConfig;

let config = SearchConfig {
    max_depth: 5,      // maximum retrosynthetic depth
    max_routes: 3,     // maximum number of routes to return
    beam_width: 50,    // A* beam width (0 = unlimited)
    ..Default::default()
};
```

### `find_routes`

Main search function. Returns `(Vec<Route>, SearchStats)`, not a bare `Vec<Route>`.

```rust
use renkin::chem_env::{ChemEnv, default_rules};
use renkin::search::{SearchConfig, find_routes};

let env = ChemEnv::load("data/building_blocks.smi")?;
let rules = default_rules();
let config = SearchConfig { max_depth: 5, ..Default::default() };

let (routes, stats) = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &config)?;
println!("Found {} routes ({} nodes expanded)", routes.len(), stats.nodes_expanded);
for route in &routes {
    println!("Route depth: {}", route.depth); // `depth` is a plain field, not a method
}
```

A full, CI-compiled-and-run version of this example lives at `examples/quickstart.rs`:

```rust
--8<-- "examples/quickstart.rs"
```

## Reaction Rules

```rust
use renkin::chem_env::{default_rules, RetroRule};

// Get the default rule set (26 hand-crafted rules)
let rules = default_rules();

// Each rule has a stable template_id, name, and SMIRKS pattern
// (SMIRKS is empty for graph-based rules dispatched by name in apply_retro,
// e.g. "ester_cleavage", "amide_cleavage", "suzuki_retro")
for rule in &rules {
    println!("{} ({}): {}", rule.name, rule.template_id, rule.smirks);
}

// Apply a single rule to a molecule
use renkin::chem_env::{apply_retro, mol_from_smiles};
let mol = mol_from_smiles("CC(=O)Oc1ccccc1C(=O)O")?;
let ester_rule = rules.iter().find(|r| r.name == "ester_cleavage").unwrap();
let precursor_sets = apply_retro(&mol, ester_rule); // Vec<Vec<PrecursorMol>>
```

`RetroRule` has 5 fields: `name`, `template_id` (`rule:<name>` for hand-crafted
rules, `smirks-sha256:<hex>` for extracted templates — see [Template Evidence
Metadata](https://github.com/kent-tokyo/renkin#template-evidence-metadata)),
`smirks`, `weight`, `required_elements`. A hand-written `RetroRule { .. }`
literal needs `..Default::default()` for any field you don't set explicitly.

## Molecule Utilities

```rust
use renkin::chem_env::{mol_from_smiles, to_canonical};

// Parse SMILES
let mol = mol_from_smiles("CC(=O)O")?;

// Get canonical SMILES
let canon = to_canonical(&mol);
println!("{}", canon);  // "CC(=O)O"
```

## Feature Flags

| Feature | Description |
|---------|-------------|
| `python` | Enable PyO3 Python bindings (for `maturin build`) |
| `nn-scoring` | ONNX-based template relevance scorer (`--scorer` CLI flag); not available on wasm32 |
| *(default: wasm32 target)* | WASM bindings via `wasm-bindgen`, gated on `target_arch = "wasm32"` rather than a Cargo feature |

## Error Types

RENKIN uses `anyhow::Error` for all fallible operations in the CLI and native
library surface (PyO3 wraps these as Python `ValueError`; WASM formats them
into the `{"error": "..."}` JSON shape). Common errors:
- SMILES parse failures from chematic
- Building block file I/O errors
- Template metadata sidecar validation failures (schema version, duplicate/dangling reference IDs, out-of-range yields, etc.)
