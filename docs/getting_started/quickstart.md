# Quick Start

## Python

```python
--8<-- "examples/quickstart.py"
```

Output (re-run as part of CI, so this can't silently drift):
```
Routes found: 3
Route (depth 1):
  OC(=O)c1ccccc1OC(=O)C -> O=CC + c1cccc(c1O)C(O)=O
  via co_aliphatic_cleavage
Route (depth 1):
  OC(=O)c1ccccc1OC(=O)C -> OC(=O)C + c1cccc(c1O)C(O)=O
  via ester_cleavage
Route (depth 1):
  OC(=O)c1ccccc1OC(=O)C -> c1cccc(c1O)C(O)=O + OC(=O)C
  via aryl_ether_retro
```

`find_routes` returns a **JSON string** — always `json.loads()` it before
accessing fields; see [Python API](../api/python.md) for the full parameter
list (custom templates, evidence metadata, constraints, pricing, etc.).

## Custom Building Blocks

You can supply your own building block library:

```python
import renkin, json

my_stock = [
    "CC(=O)O",       # acetic acid
    "Oc1ccccc1",     # phenol
    "c1ccccc1",      # benzene
    "Brc1ccccc1",    # bromobenzene
    "OB(O)c1ccccc1", # phenylboronic acid
]

result = json.loads(renkin.find_routes(
    target="c1ccc(-c2ccccc2)cc1",  # biphenyl
    building_blocks=my_stock,
    depth=3,
))
print(f"Routes found: {result['routes_found']}")
# Routes found: 1 (bromobenzene + benzene via suzuki_retro, the only rule
# this small 5-compound stock can support)
```

## Rust

```rust
--8<-- "examples/quickstart.rs"
```

`find_routes` returns `Result<(Vec<Route>, SearchStats)>` — destructure the
tuple, and note `route.depth`/`route.steps` are plain fields, not methods. See
[Rust API](../api/rust.md) for the full signature and `SearchConfig` fields.

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
