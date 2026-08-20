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
