---
title: "Python Retrosynthesis with RENKIN: API Reference and Examples"
description: "Full Python API reference for RENKIN's find_routes, predict_forward, and validate_forward functions, including parameters, return shapes, and error handling."
---

# Python API

## `find_routes`

```python
renkin.find_routes(
    target: str,
    depth: int = 5,
    max_routes: int = 5,
    beam_width: int = 0,
    building_blocks: list[str] | None = None,
    avoid_elements: str = "",
    require_elements: str = "",
    verbose: bool = False,
    bb_prices_path: str | None = None,
    templates_path: str | None = None,
    template_metadata_path: str | None = None,
) -> str
```

Find retrosynthetic routes for a target molecule. **Returns a JSON string**, not
a `dict` — parse it with `json.loads()` before accessing fields.

**Parameters:**

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `target` | `str` | required | Target molecule as SMILES string |
| `depth` | `int` | `5` | Maximum number of retrosynthetic steps |
| `max_routes` | `int` | `5` | Maximum number of routes to return |
| `beam_width` | `int` | `0` | A\* beam width (0 = unlimited BFS/A\*) |
| `building_blocks` | `list[str] \| None` | `None` | Custom building block SMILES list. If `None`, uses `data/building_blocks.smi` (402 unique compounds) when that path resolves relative to the current working directory, otherwise falls back to a compiled-in 152-compound set — see [Building Blocks](#building-blocks) below |
| `avoid_elements` | `str` | `""` | Comma-separated element symbols to ban from building blocks (e.g. `"Br,I"`) |
| `require_elements` | `str` | `""` | Comma-separated element symbols that must each appear in at least one leaf building block (e.g. `"B"` for Suzuki-type routes) |
| `verbose` | `bool` | `False` | Print search statistics (nodes expanded, elapsed time) to stderr |
| `bb_prices_path` | `str \| None` | `None` | CSV (`SMILES,price_per_gram`) for route cost scoring |
| `templates_path` | `str \| None` | `None` | Path to an extracted SMIRKS templates `.smi` file (tab-separated). `None` = hand-crafted rules only |
| `template_metadata_path` | `str \| None` | `None` | Path to a JSON evidence sidecar keyed by `template_id` (see [Template Evidence Metadata](https://github.com/kent-tokyo/renkin#template-evidence-metadata)). Matching steps get an `evidence` field; nothing is fabricated for unmatched templates |

**Returns:** a JSON string shaped like:

```python
{
    "target": str,
    "routes_found": int,
    "routes": [
        {
            "depth": int,
            "score": float,
            "confidence": float,
            "success_probability": float,
            "convergency": float,
            "route_cost": float,
            "building_blocks": [str],
            "steps": [
                {
                    "target": str,           # SMILES of molecule being disconnected
                    "rule": str,             # reaction rule name
                    "template_id": str,      # stable template identity, see Template Evidence Metadata
                    "precursors": [str],     # SMILES of precursor molecules
                    "step_confidence": float,
                    # conditions / atom_economy / procedure_hint / reaction_family /
                    # metadata_source / metadata_scope / evidence are present when
                    # applicable and omitted from the JSON otherwise. evidence, when
                    # present, may itself include an "examples" array (schema_version
                    # 2 sidecars only), each entry carrying a "match_kind" of
                    # "exact_substrate" or "template_only" -- see Template Evidence Metadata
                }
            ]
        }
    ]
}
```

**Example** (also run in CI — see `examples/quickstart.py`):

```python
--8<-- "examples/quickstart.py"
```

## `predict_forward`

```python
renkin.predict_forward(
    reactants: list[str],
    templates_path: str | None = None,
    max_results: int = 5,
) -> str
```

Predicts forward reaction products from a list of reactant SMILES, by running
retrosynthetic SMIRKS templates in reverse. Graph-based rules (e.g.
`ester_cleavage`, `amide_cleavage`) are not reversible this way and are
silently skipped. Returns a JSON string:
`[{"template": str, "products": [str], "weight": float}, ...]`.

## `validate_forward`

```python
renkin.validate_forward(
    route_json: str,
    templates_path: str | None = None,
    max_results: int = 5,
) -> str
```

Validates each step of a retrosynthetic route by checking whether forward
template application reproduces the claimed target from its precursors.
`route_json` must be a **single route object** with a top-level `steps` array
— i.e. one entry of `find_routes()`'s `routes` list, not the full
`find_routes()` output itself (which has no top-level `steps` key and raises
`ValueError: route JSON must have a 'steps' array` if passed directly):

```python
result = json.loads(renkin.find_routes(target="CC(=O)Oc1ccccc1C(=O)O", depth=1, max_routes=1))
route_json = json.dumps(result["routes"][0])
validation = json.loads(renkin.validate_forward(route_json))
```

Returns a JSON string:
`[{"step_index": int, "target": str, "verified": bool, "top_predictions": [...]}, ...]`.

## `__version__`

```python
>>> import renkin
>>> renkin.__version__
'0.20.0'
```

The version string is a module attribute, not a function.

## Building Blocks

There are **two different building-block sets**, and which one you get by
default depends on where you run Python from:

- **`data/building_blocks.smi`** — the full curated library, 402 unique
  compounds (by canonical SMILES). Loaded automatically only when that
  relative path resolves from your current working directory — in practice,
  when you're running from a checkout of the
  [renkin repository](https://github.com/kent-tokyo/renkin) itself. A wheel
  installed from PyPI (`pip install renkin`) does **not** bundle this file.
- **Compiled-in fallback (`DEFAULT_BUILDING_BLOCKS`)** — 152 unique compounds,
  built into the extension module itself. Used automatically whenever the
  402-compound file above isn't found — which, for a typical
  `pip install renkin` used outside a repo checkout, is every time.

Both cover similar ground (simple aliphatics, aryl/heteroaryl halides,
boronic acids, common heterocycles and pharmaceutical amines, protecting-group
reagents, amino acids), but they are **not the same list** — don't assume a
specific compound is present in one because it's present in the other.

**To get a specific, known set reliably, pass it explicitly** rather than
relying on either default:

```python
result = renkin.find_routes(
    target="...",
    building_blocks=["CC(=O)O", "Oc1ccccc1", ...],  # or read your own data/building_blocks.smi
)
```

Entries that fail to parse as SMILES are silently skipped (not an error) —
they simply can't match as a leaf building block during search.

## Error Handling

```python
import renkin

try:
    result = renkin.find_routes("not_a_valid_smiles!!!")
except ValueError as e:
    print(f"Error: {e}")
    # Error: Failed to parse SMILES: not_a_valid_smiles!!!
```

`find_routes`/`predict_forward`/`validate_forward` raise `ValueError` (via
PyO3) when the target SMILES fails to parse, when `template_metadata_path`
points to malformed or invalid metadata (validated before search starts), or
when `route_json` isn't valid JSON.
