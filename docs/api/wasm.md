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
  import init, { find_routes, version } from './node_modules/renkin/renkin.js';
  
  await init();
  console.log('RENKIN version:', version());
  
  const raw = find_routes(
    "CC(=O)Oc1ccccc1C(=O)O",  // Aspirin
    5,   // max depth
    3,   // max routes
    0    // beam width (0 = unlimited)
  );
  const result = JSON.parse(raw);
  console.log('Routes found:', result.routes_found);
</script>
```

## Browser and bundler usage

The npm package is currently built with `wasm-pack build --target web`.

**Supported:**

- Native browser ES modules
- Vite
- Webpack
- Rollup and compatible bundlers

**Not currently supported:**

- Plain Node.js `require()`
- Direct Node.js execution without a bundler

To exercise the WASM API from a plain Node.js script (not through a
bundler), build a `--target nodejs` package from source instead — see
[Minimal Node.js Example](#minimal-nodejs-example-ci-verified) below,
which is verified this way, not against the published npm package.

## `find_routes`

```typescript
function find_routes(
  target: string,     // Target molecule SMILES
  depth: number,      // Maximum retrosynthetic depth
  max_routes: number, // Maximum routes to return
  beam_width: number  // A* beam width (0 = unlimited)
): string  // JSON-encoded result
```

WASM always uses the current compiled-in default rule set and
building blocks — there is no way to load an external templates file or
custom building blocks list from the WASM entry point (unlike the CLI/Python
bindings). See [Rust API](rust.md) or [Python API](python.md) for
`--templates`/`templates_path` support.

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

**Return value (JSON):**

```typescript
interface Result {
  routes_found: number;
  routes: Route[];
}

interface Route {
  depth: number;
  score: number;
  confidence: number;
  success_probability: number;
  convergency: number;
  route_cost: number;
  building_blocks: string[];
  steps: Step[];
}

interface Step {
  target: string;         // SMILES of target at this step
  rule: string;           // reaction rule name
  template_id: string;    // stable template identity (rule:<name> / smirks-sha256:<hex>)
  precursors: string[];   // SMILES of precursor molecules
  step_confidence: number;
  atom_economy_status: string; // "normal" / "above_expected_range" / "not_evaluable" (always present)
  // conditions / atom_economy / atom_economy_raw_percent / procedure_hint /
  // reaction_family / metadata_source / metadata_scope / evidence are present
  // when applicable and simply absent from the JSON otherwise
}
```

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

The browser counterpart to `renkin audit-route` (see
[Audit Reproducibility and Compatibility Contract](../guides/audit-reproducibility-contract.md)
for the full `AuditRouteReport`/`audit_manifest` shape, and what each
`policy` value means) — calls the identical
`bridge::build_audit_route_report_with_policy` pipeline the CLI uses, so a
route audited in the browser gets exactly the same verdict the CLI would
produce for the same input and policy. Unlike the CLI, `content` must
already be plain JSON text — there is no gzip support in the browser (a
paste or file upload never needs it). `policy` controls only how each
route's `status` is derived from findings already collected — never which
findings are detected or reported.

```js
import init, { audit_route_v2 } from './node_modules/renkin/renkin.js';

await init();
const routeJson = JSON.stringify({
  target: "CCOC(=O)c1ccccc1",
  routes: [{
    steps: [{ target: "CCOC(=O)c1ccccc1", precursors: ["CCO", "O=C(O)c1ccccc1"], template_id: "co_aliphatic_cleavage" }],
    building_blocks: ["CCO", "O=C(O)c1ccccc1"],
  }],
});
const report = JSON.parse(audit_route_v2(routeJson, "auto", "", "strict"));
console.log(report.routes[0].status); // "pass" | "fail" | "partial"
```

Also available from the [Live Playground](https://kent-tokyo.github.io/renkin/playground/){ target="_blank" }'s
`[ Audit a Route ]` tab — paste or upload a route (and optionally a stock
list) with a policy selector, entirely client-side.

### Limits, locations, and cancellation

`capabilities()` returns machine-readable limits and accepted audit formats for
the WASM module. Call it instead of copying a numeric limit into a browser
client; it also states that the module has no network access and no
cooperative-cancellation API.

The Node/WASM quickstart executed in CI derives over-limit search and stock-line
requests from this payload and requires the real exports to reject each with
`resource_exhausted`. Rust boundary tests separately cover the inclusive
maximum and `max + 1` for every published search limit, plus the route-text,
stock-text/line, nesting, and token-count audit boundaries.

Each node-specific finding can include an additive `occurrence_path` (zero-based
child indices from the route root) and, for a decomposing node, a preorder
`step_index` into the report's `steps` array. Route-wide parse failures have no
invented location. Existing consumers that ignore these optional fields remain
compatible.

Each serialized audited step has its own required `occurrence_path`. For a
`forward_validation_not_evaluable` finding, additive `reason` repeats the
step's forward-validation reason so a flat finding consumer can distinguish
missing evidence from unsupported input without joining against `steps`.

Each audited step also has an `atom_mapping` receipt. It reports only mapping
evidence as `valid`, `invalid`, or `not_evaluable`, including duplicate or
product-only map labels and, on non-root steps, an optional
`producer_consumer` check against the parent step. This receipt never changes
the pre-existing audit `pass`/`fail`/`partial` result.

The live playground provides cancellation by terminating and respawning its
Worker. That discards the interrupted WASM operation; it is not an in-module
cancel signal. See [Trusted Route Operations](../guides/trusted-route-operations.md)
for the cross-binding and browser-operation boundary.

## `audit_route`

```typescript
function audit_route(content: string, format: string, stockText: string): string
```

The original (v0.28.0) 3-argument export, kept unchanged for backward
compatibility — a thin `policy: "standard"` wrapper around
[`audit_route_v2`](#audit_route_v2). New code should call `audit_route_v2`
directly; `audit_route` exists only so a build predating v0.29.0's policy
parameter keeps working exactly as before.

## `version`

```typescript
function version(): string
```

Returns the RENKIN version string (for example, `"1.0.11"` for the current
release).

## Minimal Node.js Example (CI-verified)

This example runs against a package built locally with
`wasm-pack build --target nodejs` — a different build target from the
published npm package (`--target web`, browser/bundler only; see
[Browser and bundler usage](#browser-and-bundler-usage) above). It's the
from-source path for using RENKIN's WASM bindings in a plain Node.js
script; `npm install renkin` alone does not give you this.

`examples/quickstart.mjs` is run against a `wasm-pack build --target nodejs`
output as part of CI, so this call shape can't silently drift from the real API:

```javascript
--8<-- "examples/quickstart.mjs"
```

## Live Playground

An interactive playground is available at [RENKIN Playground](https://kent-tokyo.github.io/renkin/playground/){ target="_blank" }.

The playground runs entirely in WebAssembly in your browser — no network calls, no server.

## Example: React Integration

```jsx
import { useEffect, useState } from 'react';

function RetrosynthesisWidget({ smiles }) {
  const [routes, setRoutes] = useState(null);
  const [wasmReady, setWasmReady] = useState(false);
  
  useEffect(() => {
    import('renkin').then(async (mod) => {
      await mod.default();
      setWasmReady(true);
    });
  }, []);
  
  useEffect(() => {
    if (!wasmReady || !smiles) return;
    import('renkin').then((mod) => {
      const raw = mod.find_routes(smiles, 5, 3, 0);
      setRoutes(JSON.parse(raw));
    });
  }, [wasmReady, smiles]);
  
  if (!routes) return <div>Loading...</div>;
  return <div>Found {routes.routes_found} routes</div>;
}
```
