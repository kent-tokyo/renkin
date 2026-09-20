// RENKIN WASM/JavaScript quickstart. Run against a
// `wasm-pack build --target nodejs` output as part of CI (see
// .github/workflows/ci.yml) so this example can never silently drift from
// the real `find_routes`/`audit_route` API.
import assert from "node:assert/strict";
import {
  find_routes,
  audit_route,
  audit_route_v2,
  capabilities,
} from "../pkg/renkin.js";

// The CI quickstart also locks the machine-readable browser boundary to the
// functions that enforce it. These calls fail before chemistry/search work.
const capability = JSON.parse(capabilities());
assert.equal(capability.schema_version, 1);
assert.equal(capability.surface, "wasm");
assert.equal(capability.network, "never");
assert.equal(capability.search.cooperative_cancel, false);
assert.equal(capability.audit.cooperative_cancel, false);
assert.deepEqual(capability.audit.accepted_formats, [
  "auto",
  "renkin",
  "aizynthfinder",
  "syntheseus",
  "synplanner",
]);
assert.deepEqual(capability.audit.policies, [
  "informational",
  "standard",
  "strict",
]);

for (const rejected of [
  find_routes("CCO", capability.search.max_depth + 1, 1, 0),
  find_routes("CCO", 1, capability.search.max_routes + 1, 0),
  find_routes("CCO", 1, 1, capability.search.max_beam_width + 1),
]) {
  assert.match(JSON.parse(rejected).error, /^resource_exhausted:/);
}

const oversizedStockLine = "C".repeat(capability.audit.max_stock_line_bytes + 1);
assert.match(
  JSON.parse(audit_route_v2("{}", "auto", oversizedStockLine, "standard")).error,
  /^resource_exhausted:/,
);

const target = "CC(=O)Oc1ccccc1C(=O)O";
const result = JSON.parse(find_routes(target, 5, 3, 0));

console.log(`Routes found: ${result.routes_found}`);
for (const route of result.routes) {
  console.log(`Route (depth ${route.depth}):`);
  for (const step of route.steps) {
    console.log(`  ${step.target} -> ${step.precursors.join(" + ")}`);
    console.log(`  via ${step.rule}`);
  }
}

// Audit the first found route -- same "Plan a Route" -> "Audit a Route"
// flow the playground offers, via the identical pipeline `renkin
// audit-route` uses on the CLI.
if (result.routes.length > 0) {
  const route = result.routes[0];
  const routeInput = JSON.stringify({
    target,
    routes: [{
      steps: route.steps.map((s) => ({
        target: s.target,
        precursors: s.precursors,
        template_id: s.template_id,
      })),
      building_blocks: route.building_blocks,
    }],
  });
  const auditReport = JSON.parse(audit_route(routeInput, "renkin", ""));
  console.log(`Audit verdict: ${auditReport.routes[0].status}`);
}
