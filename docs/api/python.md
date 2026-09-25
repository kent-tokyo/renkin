---
title: "Python Retrosynthesis with RENKIN"
description: "Compact reference for RENKIN's local Python planning, audit, and forward-validation APIs."
---

# Python API

The extension returns JSON strings so its Rust, CLI, Python, and WASM surfaces
can share stable report shapes. Parse the result before using it. It makes no
network request; caller-provided local paths are the only filesystem inputs.

```python
import json
import renkin

result = json.loads(renkin.find_routes("CC(=O)Oc1ccccc1C(=O)O", depth=5))
print(result["routes_found"])
print(renkin.__version__)
```

## Capabilities first

```python
capabilities = json.loads(renkin.capabilities())
```

Use this machine-readable contract for the installed version's limits,
accepted audit formats and policies, filesystem/network stance, and
cancellation boundary. Calls are synchronous; ordinary search and audit do
not expose cooperative cancellation.

## Plan routes

```python
renkin.find_routes(
    target,
    depth=5,
    max_routes=5,
    beam_width=0,
    building_blocks=None,
    templates_path=None,
    search_mode="standard",
    # further keyword arguments are optional
)
```

The return value has `target`, `routes_found`, `routes`, and search metadata.
`routes` may contain `route_cost`, `confidence`, and `success_probability`.
The latter two are ranking signals, not calibrated experimental probabilities
or yield predictions.

| Group | Key arguments | Meaning |
| --- | --- | --- |
| Search | `depth`, `max_routes`, `beam_width` | Bound retrosynthetic exploration; `beam_width=0` leaves it unbounded by beam pruning. |
| Stock | `building_blocks`, `avoid_elements`, `require_elements`, `bb_prices_path` | Supply explicit stock or filter/rank leaves. With no `building_blocks`, the extension loads `data/building_blocks.smi` when available, otherwise the compiled fallback. |
| Templates | `templates_path`, `template_metadata_path`, `top_templates` | Add extracted templates and optional evidence; metadata is validated before search. |
| Ordering | `reranker_model_path`, `reranker_freq_table_path` | Reorder candidates only. The candidate set is not silently replaced. |
| Constraints | `avoid_building_blocks`, `require_building_blocks`, `max_route_cost`, `min_confidence`, `min_success_probability`, reaction-family filters, `max_steps` | Filter returned routes explicitly. |
| Diagnostics | `search_diagnostics`, `candidate_trace_limit` | Add bounded search diagnostics; a trace limit enables diagnostics. |

`search_mode` is either `"standard"` or `"coverage"`. Coverage requires
`coverage_templates_path`; optional `coverage_timeout_seconds` and
`coverage_beam_width` apply only to its second stage. Stage-specific fields
are absent in standard-mode output rather than guessed as null values.

The opt-in `spectator_bond_policy` and `element_accounting_policy` values are
`"off"`, `"diagnostics_only"`, or `"gated"`. `beam_diversity_policy` is
`"off"`, `"diagnostics_only"`, or `"active"`; use `beam_diversity_slots`
only with that policy. Invalid combinations raise `ValueError` before search.

## Building blocks

Pass `building_blocks` for an in-memory stock that is independent of the
current working directory. Otherwise the extension uses the repository stock
file when it can find it and the compiled fallback when it cannot. Exact
standardized canonical-SMILES identity decides stock membership; relaxed
substructure matching is not a stock check.

## Forward checks

```python
predictions = json.loads(
    renkin.predict_forward(["Oc1ccccc1C(=O)O", "CCO"], max_results=5)
)

checks = json.loads(
    renkin.validate_forward(json.dumps(result["routes"][0]), max_results=5)
)
```

Both accept an optional `templates_path`. `predict_forward` returns template
applications and candidate products; `validate_forward` reports every route
step and whether its target appears among the bounded forward predictions.
They do not predict conditions, yield, or laboratory success.

## Audit an existing route

```python
with open("route.json", encoding="utf-8") as handle:
    report = json.loads(
        renkin.audit_route(handle.read(), format="auto", policy="standard")
    )
```

`format` accepts `auto`, `renkin`, `aizynthfinder`, `syntheseus`, or
`synplanner`. Pass newline-delimited SMILES with `stock_text` to enable stock
checks. `policy` is `informational`, `standard`, or `strict`; it changes only
the derived `pass`/`fail`/`partial` status, never the findings. See the
[audit contract](../guides/audit-reproducibility-contract.md) for report
semantics and the [private-stock guide](../guides/private-stock-policy.md) for
vendor policy.

## Errors and compatibility

Invalid SMILES, invalid policy values, unreadable or malformed assets,
resource limits, and unsupported route shapes raise `ValueError`. Do not
interpret an exception as an empty result. Pin `renkin.__version__`, asset
hashes, stock, template files, and budgets when a result must be reproduced.
