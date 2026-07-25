// RENKIN WASM/JavaScript quickstart. Run against a
// `wasm-pack build --target nodejs` output as part of CI (see
// .github/workflows/ci.yml) so this example can never silently drift from
// the real `find_routes` API.
import { find_routes } from "../pkg/renkin.js";

const result = JSON.parse(find_routes("CC(=O)Oc1ccccc1C(=O)O", 5, 3, 0));

console.log(`Routes found: ${result.routes_found}`);
for (const route of result.routes) {
  console.log(`Route (depth ${route.depth}):`);
  for (const step of route.steps) {
    console.log(`  ${step.target} -> ${step.precursors.join(" + ")}`);
    console.log(`  via ${step.rule}`);
  }
}
