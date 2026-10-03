---
title: "Browser-Based Retrosynthesis with RENKIN WebAssembly"
description: "Run RENKIN's retrosynthesis search entirely in the browser via WebAssembly -- no server, no installation. API reference, bundler support, and examples."
---

# WASM / JavaScript API

## Installation

```bash
npm install renkin
```

## Browser (ES Module)

```html
<script type="module">
  import init, { find_routes } from './node_modules/renkin/renkin.js';
  await init();
  const result = JSON.parse(find_routes("CC(=O)Oc1ccccc1C(=O)O", 5, 3, 0));
  console.log(result.routes_found);
</script>
```

## Browser and bundler usage

The published npm package targets browser ES modules and bundlers such as Vite,
Webpack, and Rollup. It does not support plain Node.js `require()` or direct
Node execution. For Node, build from source with `wasm-pack --target nodejs`;
see the [CI-tested example](#minimal-nodejs-example-ci-verified).

## `find_routes`

```typescript
function find_routes(
  target: string,     // Target molecule SMILES
  depth: number,      // Maximum retrosynthetic depth
  max_routes: number, // Maximum routes to return
  beam_width: number  // A* beam width (0 = unlimited)
): string  // JSON-encoded result
```

WASM uses compiled-in rules and stock; this entry point cannot load external
templates or stock. Use [Rust](rust.md) or [Python](python.md) for that.

### Input limits and validation

Public WASM search exports validate inputs before starting search. The current
limits are:

| Input | Maximum |
| --- | ---: |
| Search depth | 16 |
| Returned routes | 100 |
| Beam width and diversity slots | 10,000 |
| Candidate trace records | 50,000 |
| Target SMILES | 64 KiB |
| Element-filter text | 256 bytes |

Element filters accept the supported symbols (`H`, `B`, `C`, `N`, `O`, `F`,
`Si`, `P`, `S`, `Cl`, `Br`, `I`) as a comma-separated list. Empty tokens,
unknown symbols, and oversized values return an error. These bounds protect
the browser boundary and do not change native CLI or Python limits.

The JSON includes `routes_found` and `routes`. Each route contains `steps` and
`building_blocks`; each step identifies its `target`, `precursors`, and
`template_id`. Evidence and condition fields appear only when available.
Confidence values rank candidates; they are not measured yields.

## `find_routes_v6`

`find_routes_v6` keeps the policy arguments from `find_routes_v5` and adds a
final `candidate_trace_limit` argument. It always returns a
`search_diagnostics` object; the candidate trace is capped at the requested
limit and does not change route selection. Existing versioned exports remain
available for compatibility.

## `audit_route_v2`

```typescript
function audit_route_v2(
  content: string,    // Route export JSON text (RENKIN, AiZynthFinder, Syntheseus, or SynPlanner)
  format: string,      // "auto" | "renkin" | "aizynthfinder" | "syntheseus" | "synplanner"
  stockText: string,    // "" for no stock, else one SMILES per line (.smi-style)
  policy: string         // "informational" | "standard" | "strict"
): string  // JSON-encoded AuditRouteReport, or {"error": "..."}
```

This shares the CLI audit pipeline. `content` must be plain JSON (no gzip).
The policy changes the derived `pass`/`fail`/`partial` verdict, not the
findings. See the [audit contract](../guides/audit-reproducibility-contract.md)
for the report shape and policy semantics, or use the
[Playground](https://kent-tokyo.github.io/renkin/playground/) to try it locally.

### Limits, locations, and cancellation

`capabilities()` returns machine-readable limits and accepted audit formats for
the WASM module. Call it instead of copying a numeric limit into a browser
client; it also states that the module has no network access and no
cooperative-cancellation API.

Boundary tests reject over-limit requests with `resource_exhausted`. Audited
steps carry `occurrence_path` and an `atom_mapping` receipt (`valid`, `invalid`,
or `not_evaluable`); findings may add a `step_index` and reason. Mapping
receipts do not change the audit verdict. See the
[trusted-route guide](../guides/trusted-route-operations.md) for details.

The playground cancels by terminating and respawning its Worker; the WASM
module has no cooperative cancellation API.

## `audit_route`

```typescript
function audit_route(content: string, format: string, stockText: string): string
```

The older three-argument export uses `policy: "standard"`. New code should
use [`audit_route_v2`](#audit_route_v2).

## `version`

```typescript
function version(): string
```

Returns the RENKIN version string.

## Minimal Node.js Example (CI-verified)

This CI-run example uses a locally built `wasm-pack --target nodejs` package,
not the published browser-targeted npm package:

```javascript
--8<-- "examples/quickstart.mjs"
```
