# Quick Start

## Python

```python
import renkin

# Find retrosynthetic routes for Aspirin
result = renkin.find_routes(
    smiles="CC(=O)Oc1ccccc1C(=O)O",
    depth=5,
    max_routes=3,
)

print(f"Found {result['routes_found']} routes")

for i, route in enumerate(result["routes"]):
    print(f"\nRoute {i+1} (depth {route['depth']}):")
    for step in route["steps"]:
        precursors = " + ".join(step["precursors"])
        print(f"  {step['target']}")
        print(f"  → {precursors}  [{step['rule']}]")
```

Output:
```
Found 2 routes

Route 1 (depth 2):
  CC(=O)Oc1ccccc1C(=O)O
  → CC(=O)O + Oc1ccccc1C(=O)O  [ester_cleavage]

Route 2 (depth 2):
  CC(=O)Oc1ccccc1C(=O)O
  → CC(=O)Cl + Oc1ccccc1C(=O)O  [ester_acyl_chloride]
```

## Custom Building Blocks

You can supply your own building block library:

```python
import renkin

# List of SMILES strings
my_stock = [
    "CC(=O)O",      # acetic acid
    "Oc1ccccc1",    # phenol
    "c1ccccc1",     # benzene
    "Brc1ccccc1",   # bromobenzene
    "OB(O)c1ccccc1", # phenylboronic acid
]

result = renkin.find_routes(
    smiles="c1ccc(-c2ccccc2)cc1",  # biphenyl
    building_blocks=my_stock,
    depth=3,
)
```

## Rust

```rust
use renkin::{
    chem_env::{ChemEnv, default_rules},
    search::{SearchConfig, find_routes},
};

fn main() -> anyhow::Result<()> {
    // Load building blocks from a SMILES file
    let env = ChemEnv::load("data/building_blocks.smi")?;
    
    let config = SearchConfig {
        max_depth: 5,
        max_routes: 3,
        beam_width: 0,  // 0 = unlimited A*
    };
    
    let routes = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &config)?;
    
    println!("Found {} routes", routes.len());
    for (i, route) in routes.iter().enumerate() {
        println!("Route {} (depth {}):", i + 1, route.depth());
    }
    
    Ok(())
}
```

## CLI Benchmark

```bash
# Run retrosynthesis on a list of targets
renkin-bench \
    --input targets.smi \
    --building-blocks data/building_blocks.smi \
    --depth 3 \
    --beam-width 50 \
    > results.json
```

The input file should be a SMILES file (one SMILES per line, optional name after whitespace).

## SMILES File Format

Building blocks and target files use the standard `.smi` format:

```
CC(=O)O         acetic_acid
c1ccccc1        benzene
# Comments start with #
Brc1ccccc1      bromobenzene
```
