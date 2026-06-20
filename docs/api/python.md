# Python API

## `find_routes`

```python
renkin.find_routes(
    smiles: str,
    depth: int = 5,
    max_routes: int = 5,
    beam_width: int = 0,
    building_blocks: list[str] | None = None,
) -> dict
```

Find retrosynthetic routes for a target molecule.

**Parameters:**

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `smiles` | `str` | required | Target molecule as SMILES string |
| `depth` | `int` | `5` | Maximum number of retrosynthetic steps |
| `max_routes` | `int` | `5` | Maximum number of routes to return |
| `beam_width` | `int` | `0` | A\* beam width (0 = unlimited BFS/A\*) |
| `building_blocks` | `list[str]` | `None` | Custom building block SMILES list. If `None`, uses the built-in library (~480 compounds) |

**Returns:**

```python
{
    "routes_found": int,   # number of routes found
    "routes": [
        {
            "depth": int,   # number of steps in this route
            "steps": [
                {
                    "target": str,           # SMILES of molecule being disconnected
                    "rule": str,             # reaction rule name
                    "precursors": list[str], # SMILES of precursor molecules
                }
            ]
        }
    ]
}
```

**Example:**

```python
import renkin

result = renkin.find_routes("CC(=O)Oc1ccccc1C(=O)O", depth=5, max_routes=3)

print(f"Routes: {result['routes_found']}")
for route in result['routes']:
    print(f"  depth={route['depth']}: {len(route['steps'])} step(s)")
```

## `version`

```python
renkin.version() -> str
```

Returns the RENKIN version string.

```python
>>> import renkin
>>> renkin.version()
'0.1.0'
```

## Building Blocks

The default building block library includes ~480 commercially available compounds:

- Simple aliphatics (C1–C6 chains, alcohols, acids)
- Aryl and heteroaryl halides (Br, Cl, I)
- Boronic acids (Suzuki coupling acceptors)
- Pyridines, pyrimidines, pyrazoles, imidazoles, furans, thiophenes
- Common pharmaceutical amines (piperidine, morpholine, piperazine, etc.)
- Aldehydes and ketones for reductive amination
- Protecting group reagents (Boc, Cbz)
- Amino acids (Gly, Ala, Asp, Glu, Ser, Phe, Tyr, Lys, Cys, Val)

To use a custom library, pass a list of SMILES strings to the `building_blocks` parameter.

## Error Handling

```python
import renkin

try:
    result = renkin.find_routes("invalid_smiles")
except Exception as e:
    print(f"Error: {e}")
    # "Failed to parse SMILES: invalid_smiles"
```

Common errors:
- `Failed to parse SMILES: ...` — invalid SMILES string
- `Building block parse error: ...` — invalid SMILES in custom building blocks list
