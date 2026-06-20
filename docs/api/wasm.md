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

## Node.js

```javascript
import { createRequire } from 'module';
const require = createRequire(import.meta.url);

// Node.js usage (sync WASM load)
const renkin = require('renkin');
await renkin.default();  // initialize WASM

const raw = renkin.find_routes("c1ccc(-c2ccccc2)cc1", 5, 3, 0);
const result = JSON.parse(raw);
```

## `find_routes`

```typescript
function find_routes(
  smiles: string,    // Target molecule SMILES
  depth: number,     // Maximum retrosynthetic depth
  max_routes: number, // Maximum routes to return
  beam_width: number  // A* beam width (0 = unlimited)
): string  // JSON-encoded result
```

**Return value (JSON):**

```typescript
interface Result {
  routes_found: number;
  routes: Route[];
}

interface Route {
  depth: number;
  steps: Step[];
}

interface Step {
  target: string;      // SMILES of target at this step
  rule: string;        // reaction rule name
  precursors: string[]; // SMILES of precursor molecules
}
```

## `version`

```typescript
function version(): string
```

Returns the RENKIN version string (e.g., `"0.1.0"`).

## Live Playground

An interactive playground is available at [/playground/](../playground/){ target="_blank" }.

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
