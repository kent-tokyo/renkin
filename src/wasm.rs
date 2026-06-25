use wasm_bindgen::prelude::*;

use crate::DEFAULT_BUILDING_BLOCKS;
use crate::chem_env::{ChemEnv, default_rules};
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
        Ok(routes) => {
            let output = serde_json::json!({
                "target": target,
                "routes_found": routes.len(),
                "routes": routes,
            });
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
