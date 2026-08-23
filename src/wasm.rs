use wasm_bindgen::prelude::*;

use crate::DEFAULT_BUILDING_BLOCKS;
use crate::chem_env::{self, ChemEnv, default_rules};
use crate::search::{SearchConfig, find_routes as rs_find_routes};

/// Find retrosynthetic routes for a target molecule (WASM entry point).
///
/// Returns a JSON string with the retrosynthesis result.
///
/// # Arguments
/// * `target`      - Target molecule SMILES
/// * `depth`       - Maximum retrosynthesis depth
/// * `max_routes`  - Maximum number of routes
/// * `beam_width`  - Beam search width; 0 = unlimited A*
///
/// # Example (JavaScript)
/// ```js
/// import init, { find_routes } from '@renkin/wasm';
/// await init();
/// const result = JSON.parse(find_routes("CC(=O)Oc1ccccc1C(=O)O", 3, 5, 0));
/// console.log(result.routes_found);
/// ```
#[wasm_bindgen]
pub fn find_routes(target: &str, depth: u32, max_routes: usize, beam_width: usize) -> String {
    let env = ChemEnv::in_memory(DEFAULT_BUILDING_BLOCKS);
    let rules = default_rules();
    let config = SearchConfig {
        max_depth: depth,
        max_routes,
        beam_width,
        ..Default::default()
    };

    match rs_find_routes(target, &env, &rules, &config) {
        Ok((routes, stats)) => {
            let output = if routes.is_empty() {
                serde_json::json!({
                    "target": target,
                    "routes_found": 0,
                    "routes": [],
                    "diagnostics": {"nodes_expanded": stats.nodes_expanded}
                })
            } else {
                serde_json::json!({
                    "target": target,
                    "routes_found": routes.len(),
                    "routes": routes,
                })
            };
            serde_json::to_string(&output)
                .unwrap_or_else(|e| format!(r#"{{"error":"serialization: {e}"}}"#))
        }
        Err(e) => format!(r#"{{"error":"{e}"}}"#),
    }
}

/// Find retrosynthetic routes with element filtering (WASM entry point).
///
/// Same as [`find_routes`] plus `avoid_elements`/`require_elements`: comma-
/// separated element symbols, identical format and conversion
/// (`chem_env::elem_symbols_to_mask`) as the CLI's own `--avoid-elements`/
/// `--require-elements` flags, so browser filtering behaves exactly like
/// the CLI's, not a separately-maintained copy. A distinct function name
/// rather than optional/extra parameters on `find_routes`, so a caller on
/// an old build gets a real "no such export" `TypeError` instead of a
/// silently-ignored argument -- this playground's own JS previously relied
/// on `try { 6-arg call } catch { 4-arg call }` to bridge this gap, which
/// meant `avoid_elements`/`require_elements` silently never took effect
/// against any WASM build that predates this function.
#[wasm_bindgen]
pub fn find_routes_v2(
    target: &str,
    depth: u32,
    max_routes: usize,
    beam_width: usize,
    avoid_elements: &str,
    require_elements: &str,
) -> String {
    let env = ChemEnv::in_memory(DEFAULT_BUILDING_BLOCKS);
    let rules = default_rules();
    let config = SearchConfig {
        max_depth: depth,
        max_routes,
        beam_width,
        forbidden_elements: chem_env::elem_symbols_to_mask(avoid_elements),
        required_element_present: chem_env::elem_symbols_to_mask(require_elements),
        ..Default::default()
    };

    match rs_find_routes(target, &env, &rules, &config) {
        Ok((routes, stats)) => {
            let output = if routes.is_empty() {
                serde_json::json!({
                    "target": target,
                    "routes_found": 0,
                    "routes": [],
                    "diagnostics": {"nodes_expanded": stats.nodes_expanded}
                })
            } else {
                serde_json::json!({
                    "target": target,
                    "routes_found": routes.len(),
                    "routes": routes,
                })
            };
            serde_json::to_string(&output)
                .unwrap_or_else(|e| format!(r#"{{"error":"serialization: {e}"}}"#))
        }
        Err(e) => format!(r#"{{"error":"{e}"}}"#),
    }
}

/// Return the crate version string.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Static capabilities of this WASM build (browser edition), as a JSON
/// string -- real counts read from the same compiled-in data `find_routes`
/// itself searches against, not a hardcoded UI string that can drift from
/// what the engine actually has loaded.
#[wasm_bindgen]
pub fn capabilities() -> String {
    let env = ChemEnv::in_memory(DEFAULT_BUILDING_BLOCKS);
    serde_json::json!({
        "building_blocks": env.bb_count(),
        "reaction_rules": default_rules().len(),
    })
    .to_string()
}

/// Audit a pasted/uploaded route export (WASM entry point) -- the browser
/// counterpart to `renkin audit-route`, calling the identical
/// `bridge::build_audit_route_report` pipeline so a route audited in the
/// playground gets exactly the same pass/fail/partial verdict the CLI would
/// produce for the same input, not a separately-maintained copy.
///
/// # Arguments
/// * `content`    - Route export JSON text (RENKIN `--format json` output,
///                   or an AiZynthFinder single-route/batch export). Plain
///                   JSON only -- unlike the CLI, this has no gzip support;
///                   a browser paste/upload never needs it.
/// * `format`     - `"auto" | "renkin" | "aizynthfinder"`, same vocabulary
///                   as the CLI's `--format` flag.
/// * `stock_text` - Optional `.smi`-style stock listing (one SMILES per
///                   line, `#`-comments allowed), or `""` for "no stock to
///                   check against".
///
/// Returns a JSON string: either the `AuditRouteReport` shape (same as
/// `renkin audit-route --output json`) or `{"error": "..."}` on a bad
/// input (malformed JSON, unrecognized format, ...).
#[wasm_bindgen]
pub fn audit_route(content: &str, format: &str, stock_text: &str) -> String {
    audit_route_v2(content, format, stock_text, "standard")
}

/// Same as [`audit_route`], plus `policy` (v0.29.0 Audit Policy Profiles):
/// `"informational" | "standard" | "strict"`, same vocabulary as the CLI's
/// `--policy` flag -- controls only how each route's `status` is derived
/// from findings already collected, never which findings are detected or
/// reported. A distinct function name rather than a 4th parameter on
/// `audit_route`, so a caller on a pre-v0.29.0 build gets a real "no such
/// export" `TypeError` instead of a silently-ignored argument -- the same
/// reasoning `find_routes_v2` already established in this codebase.
/// `audit_route` itself is now a thin `"standard"` wrapper around this.
#[wasm_bindgen]
pub fn audit_route_v2(content: &str, format: &str, stock_text: &str, policy: &str) -> String {
    let policy = match policy.parse::<crate::bridge::AuditPolicy>() {
        Ok(p) => p,
        Err(e) => {
            return serde_json::to_string(&serde_json::json!({ "error": e }))
                .unwrap_or_else(|_| r#"{"error":"invalid policy"}"#.to_string());
        }
    };
    let stock =
        (!stock_text.trim().is_empty()).then(|| crate::bridge::parse_stock_text(stock_text));
    let rules = default_rules();
    match crate::bridge::build_audit_route_report_with_policy(
        content,
        format,
        stock.as_ref(),
        &rules,
        policy,
    ) {
        Ok(report) => serde_json::to_string(&report)
            .unwrap_or_else(|e| format!(r#"{{"error":"serialization: {e}"}}"#)),
        Err(e) => serde_json::to_string(&serde_json::json!({ "error": format!("{e:#}") }))
            .unwrap_or_else(|_| r#"{"error":"audit failed"}"#.to_string()),
    }
}
