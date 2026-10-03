---
title: "RENKIN Quick Start: Find a Retrosynthesis Route in a Few Lines"
description: "A minimal, CI-tested working example of RENKIN's find_routes API in Python, Rust, and the CLI, plus how to supply a custom building-block stock."
---

# Quick Start

## Python

```python
--8<-- "examples/quickstart.py"
```

The included example is executed in CI. `find_routes` returns a JSON string;
call `json.loads()` before accessing fields. Route counts can change with the
stock and rules. See the [Python API](../api/python.md) for options and output.

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
