# Rust API

## Core Types

### `ChemEnv`

The chemical environment holding the building block database.

```rust
use renkin::chem_env::ChemEnv;

// Load from SMILES file
let env = ChemEnv::load("data/building_blocks.smi")?;

// Load from in-memory list
let env = ChemEnv::in_memory(&["CC(=O)O", "c1ccccc1", "Brc1ccccc1"]);

// Check if a molecule is in the stock
let mol = mol_from_smiles("CC(=O)O")?;
assert!(env.is_building_block(&mol));
```

### `SearchConfig`

Configuration for the retrosynthesis search.

```rust
use renkin::search::SearchConfig;

let config = SearchConfig {
    max_depth: 5,      // maximum retrosynthetic depth
    max_routes: 3,     // maximum number of routes to return
    beam_width: 50,    // A* beam width (0 = unlimited)
};
```

### `find_routes`

Main search function.

```rust
use renkin::search::{SearchConfig, find_routes};
use renkin::chem_env::ChemEnv;

let env = ChemEnv::load("data/building_blocks.smi")?;
let config = SearchConfig { max_depth: 5, ..Default::default() };

let routes = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &config)?;
println!("Found {} routes", routes.len());
```

## Reaction Rules

```rust
use renkin::chem_env::{default_rules, RetroRule};

// Get the default rule set (20 rules)
let rules = default_rules();

// Each rule has a name and SMIRKS pattern
for rule in &rules {
    println!("{}: {}", rule.name, rule.smirks);
}

// Apply a single rule to a molecule
use renkin::chem_env::{apply_retro, mol_from_smiles};
let mol = mol_from_smiles("CC(=O)Oc1ccccc1C(=O)O")?;
let rule = RetroRule { name: "ester_cleavage", smirks: "[C:1](=[O:2])[O:3]>>[C:1](=[O:2])O.[O:3]" };
let precursor_sets = apply_retro(&mol, &rule);
```

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
| *(default: wasm32)* | WASM bindings via `wasm-bindgen` |

## Error Types

RENKIN uses `anyhow::Error` for all fallible operations. Common errors:
- SMILES parse failures from chematic
- Building block file I/O errors
