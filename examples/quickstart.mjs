// RENKIN WASM/JavaScript quickstart. Run against a
// `wasm-pack build --target nodejs` output as part of CI (see
// .github/workflows/ci.yml) so this example can never silently drift from
// the real `find_routes`/`audit_route` API.
import { find_routes, audit_route } from "../pkg/renkin.js";

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
