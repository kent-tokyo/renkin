---
title: "Python Retrosynthesis with RENKIN: API Reference and Examples"
description: "Full Python API reference for RENKIN's find_routes, predict_forward, validate_forward, and audit_route functions, including parameters, return shapes, and error handling."
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
    reranker_model_path: str | None = None,
    reranker_freq_table_path: str | None = None,
    top_templates: int | None = None,
    search_mode: str = "standard",
    coverage_templates_path: str | None = None,
    coverage_timeout_seconds: int | None = None,
    search_diagnostics: bool = False,
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
| `reranker_model_path` | `str \| None` | `None` | Path to a frozen LightGBM `model.txt` for candidate reranking. Requires `reranker_freq_table_path` too. Ordering-only — never changes which candidates are considered, only their order |
| `reranker_freq_table_path` | `str \| None` | `None` | Path to the TRAIN-frozen template `frequency_table.json` the reranker needs alongside `reranker_model_path` |
| `top_templates` | `int \| None` | `None` | Keep only the top-N `templates_path` templates by frequency weight. Applies only to Stage 1 (`templates_path`) — coverage mode's Stage 2 (`coverage_templates_path`) always uses its full template set unfiltered |
| `search_mode` | `str` | `"standard"` | `"standard"` (unchanged behavior) or `"coverage"` — see [Coverage Mode](#coverage-mode) below |
| `coverage_templates_path` | `str \| None` | `None` | Stage 2's template set; required when `search_mode="coverage"`, validated before Stage 1 even runs |
| `coverage_timeout_seconds` | `int \| None` | `None` | Optional positive-integer wall-clock budget for Stage 2 only (cooperative cancellation, not a hard bound). `0` raises `ValueError` |
| `search_diagnostics` | `bool` | `False` | Add a `search_diagnostics` block (beam eviction, cross-template dedup, branching factor) to the JSON output — identical field names/shape to the `renkin` CLI's own `--search-diagnostics` flag |

**Returns:** a JSON string shaped like:

```python
{
    "target": str,
    "routes_found": int,
    # joint_success_probability is present only when routes_found > 0:
    # 1 - Π(1 - route.success_probability) across every returned route --
    # a frequency-derived score, not a calibrated experimental probability.
    "joint_success_probability": float,
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
                    # atom_economy_status: "normal" / "above_expected_range" / "not_evaluable" (always present)
                    # conditions / atom_economy / atom_economy_raw_percent / procedure_hint /
                    # reaction_family / metadata_source / metadata_scope / evidence are present
                    # when applicable and omitted from the JSON otherwise. evidence, when
                    # present, may itself include an "examples" array (schema_version
                    # 2 sidecars only), each entry carrying a "match_kind" of
                    # "exact_substrate" or "template_only" -- see Template Evidence Metadata
                }
            ]
        }
    ]
}
```

When `routes_found == 0`, `routes` is `[]` and `joint_success_probability` is
absent; instead there's a `diagnostics` object: `nodes_expanded` (int),
`max_depth_reached`/`beam_limit_hit` (bool), `matched_templates`/`stock_hits`
(int), `likely_causes` (`[str]`), `suggestions` (`[str]`) — identical shape to
the `renkin` CLI's own empty-route JSON output.

When `search_diagnostics=True`, a `search_diagnostics` object is added in
both cases above (beam-prune/crowd-out counters — see the `renkin` CLI's
`--search-diagnostics` flag for the full field list).

**Example** (also run in CI — see `examples/quickstart.py`):

```python
--8<-- "examples/quickstart.py"
```

## Coverage Mode

Opt-in Stage-1/Stage-2 escalation: only if the default `templates_path`
search finds nothing does Stage 2 run, against a separately loaded, larger
`coverage_templates_path` template set, cooperatively cancellable via
`coverage_timeout_seconds`. See
[the design doc](https://github.com/kent-tokyo/renkin/blob/master/docs/design/coverage-mode-v0.md)
for the full rationale and the formal-TEST confirmation numbers.

```python
--8<-- "examples/coverage_mode.py"
```

Standard-mode output (the default) is byte-for-byte unchanged: the extra
fields below are omitted entirely, not `null`, unless `search_mode="coverage"`
is actually passed.

**Returns**, in addition to everything above, when `search_mode="coverage"`:

```python
{
    "search_mode": "coverage",
    "selected_stage": "stage1" | "stage2",
    "stage2_invoked": bool,
    "stage1_timeout": bool,
    "stage2_timeout": bool,
    "stage1_elapsed_ms": float,
    "stage2_elapsed_ms": float | None,  # None iff Stage 2 never ran
    "total_elapsed_ms": float,
}
```

Identical field names and shapes to the `renkin` CLI's own coverage-mode
JSON output.

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

## `audit_route`

```python
renkin.audit_route(
    content: str,
    format: str = "auto",
    stock_text: str = "",
    policy: str = "standard",
) -> str
```

Audits an already-completed retrosynthesis route (a RENKIN `--format json`
export or an AiZynthFinder single-route/batch export) for structural
integrity, stock coverage, element accounting, and forward-reaction
reproducibility -- the Python binding for `renkin audit-route`, calling the
identical pipeline the CLI and the [WASM `audit_route_v2`](wasm.md#audit_route_v2)
export use, so the same input and policy get the same verdict from every
surface. See
[Audit Reproducibility and Compatibility Contract](../guides/audit-reproducibility-contract.md)
for the full `audit_manifest`/report shape and what each policy means.

A thin binding on purpose: `content` is JSON text you already have in hand
(read any file yourself, including a gzip-compressed AiZynthFinder batch
export -- decompress it before passing it in, this function never touches
the filesystem). `format` is `"auto"` (default) / `"renkin"` /
`"aizynthfinder"` / `"syntheseus"`. `stock_text` is an optional `.smi`-style listing (one
SMILES per line, `#`-comments allowed); omitted, stock validation reports
`not_evaluable`, never a silent pass. `policy` is `"informational"` /
`"standard"` (default) / `"strict"` -- controls only how each route's
`status` is derived from findings already collected, never which findings
are detected or reported.

```python
with open("trees.json", encoding="utf-8") as f:
    report = json.loads(
        renkin.audit_route(f.read(), format="aizynthfinder", policy="strict")
    )
print(report["summary"])
```

Returns a JSON string: the same `AuditRouteReport` shape
`renkin audit-route --output json` produces, including
`audit_manifest.policy` recording the policy actually used. Raises
`ValueError` on malformed JSON, an unrecognized route shape, or an invalid
`format`/`policy` value -- fail-loud, never a partial or guessed result.

## `audit_route_report`

```python
renkin.audit_route_report(
    content: str,
    format: str = "auto",
    stock_text: str = "",
    policy: str = "standard",
) -> AuditRouteReport
```

Same arguments, same validation, same `ValueError`s as `audit_route()` --
the only difference is the return type. `audit_route()` itself is
completely unchanged by this: it's still there, still returns a plain
`str`, for anyone who wants the raw JSON. `audit_route_report()` is a
pure-Python convenience layer on top (`python/renkin/audit_report.py`,
defined outside the compiled extension) that calls `audit_route()`,
`json.loads()`s it, and hands back attribute-accessible dataclasses
instead of a dict-of-dicts:

```python
report = renkin.audit_route_report(content, format="aizynthfinder", policy="strict")
print(report.audit_manifest.policy)
print(report.routes[0].status)
for finding in report.routes[0].findings:
    print(finding.code, finding.severity)
print(report.routes[0].steps[0].forward_validation.status)
```

Returns an `AuditRouteReport`:

| Field | Type |
|---|---|
| `schema_version` | `int` |
| `source_format` | `str` |
| `audit_manifest` | `AuditManifest` |
| `summary` | `AuditRouteSummary` |
| `routes` | `list[AuditReport]` |

`AuditManifest`: `renkin_version`, `report_schema_version` (`int`),
`source_format`, `input_sha256`, `policy` (all `str`), plus
`source_version: str | None` and `stock_sha256: str | None`.

`AuditRouteSummary`: `routes_total`, `passed`, `fail`, `partial` (all
`int`) -- note `passed`, not `pass`: the wire JSON's key really is
`"pass"`, renamed here since `pass` is a Python reserved word.

`AuditReport` (one per audited route): `source`, `status` (`str`),
`route_tree_parseable` (`bool`), `reaction_steps_parseable: bool | None`,
`stock_validation: StockValidationResult | None`,
`target_element_accounting_status: str | None`,
`normalized_route_sha256: str | None`, `steps: list[AuditedStep]`,
`findings: list[AuditFinding]`.

`AuditedStep`: `target: str`, `precursors: list[str]`,
`forward_validation: ForwardValidationResult`.

`ForwardValidationResult`: `status: str`, `method: str`,
`evidence_basis: str | None` (`"declared_rule_template"` |
`"derived_graph_rule_roundtrip"` | `"source_tool_reaction"` | `None` --
see [Audit Reproducibility and Compatibility Contract](../guides/audit-reproducibility-contract.md#forward-validation-evidence-basis)
for what each means), `reason: str | None`. `StockValidationResult`:
`status: str`, `reason: str | None`. `AuditFinding`: `code: str`,
`severity: str`, `node: str | None`.

**Every `str | None` field here collapses two different wire-level
states into one Python value.** In the raw JSON, some optional fields
are an explicit `null` and some are entirely *absent* keys (Rust's
`skip_serializing_if`) -- both mean "not applicable here", and both
become `None` on the typed side. This loses no information that matters
to a caller of this convenience API; anyone who genuinely needs to tell
"explicit null" apart from "key absent" should use `audit_route()`
(the string API) and inspect the parsed JSON directly instead.

**Status/code/severity fields stay plain `str`, not a Python `Enum`.**
A real `Enum` would raise the moment a future RENKIN version ships a new
variant value this stub doesn't know about yet; `str` degrades
gracefully. The current closed set of values for each is documented in
[Audit Reproducibility and Compatibility Contract](../guides/audit-reproducibility-contract.md).

## `__version__`

```python
>>> import renkin
>>> renkin.__version__
'0.54.0'
```

The version string is a module attribute, not a function.

## Typed Usage

RENKIN ships a type stub (`renkin.pyi` + `py.typed`) alongside the compiled
extension in every published wheel. Editors and type checkers (mypy,
pyright) pick it up automatically once `renkin` is installed — no extra
import or configuration needed:

```python
import renkin

result: str = renkin.find_routes("CC(=O)Oc1ccccc1C(=O)O", depth=3)  # type-checked
```

The stub only types the function signatures (arguments and the fact that
every function returns `str`); it doesn't type the JSON *contents* of that
string — parse with `json.loads()` and refer to the return-shape
documentation above for the actual fields.

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

`reranker_model_path`/`reranker_freq_table_path` are the one exception to
"bad input raises": a missing file, a malformed model, or only one of the
two paths given never raises — it prints a warning to stderr and falls
back to `find_routes`'s legacy candidate ordering for that call. This
matches the `renkin` CLI's `--reranker-model`/`--reranker-freq-table`
flags exactly, since a broken optional reranker file shouldn't be able to
take down an otherwise-working search.
