use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use crate::chem_env::{ChemEnv, default_rules, elem_symbols_to_mask};
use crate::search::{SearchConfig, find_routes};

/// Find retrosynthetic routes for a target molecule.
///
/// Args:
///     target (str): Target molecule as SMILES string.
///     depth (int): Maximum retrosynthesis depth. Default: 5.
///     max_routes (int): Maximum number of routes to return. Default: 5.
///     beam_width (int): Beam search width; 0 = unlimited A*. Default: 0.
///     building_blocks (list[str] | None): Custom list of commercial starting
///         materials as SMILES. If None, uses the built-in default set.
///     avoid_elements (str): Comma-separated element symbols to ban from building
///         blocks (e.g. ``"Br,I"``). Routes whose leaf BBs contain any forbidden
///         element are dropped. Default: ``""`` (no constraint).
///     require_elements (str): Comma-separated element symbols that must each appear
///         in at least one leaf BB (e.g. ``"B"`` for Suzuki-type routes).
///         Default: ``""`` (no constraint).
///     verbose (bool): Print search statistics (nodes expanded, elapsed time) to
///         stderr after the search completes. Default: ``False``.
///
/// Returns:
///     str: JSON string with retrosynthesis routes.
///
/// Example::
///
///     import renkin, json
///     routes = json.loads(renkin.find_routes("CC(=O)Oc1ccccc1C(=O)O", depth=3))
///     print(routes["routes_found"])
#[pyfunction]
#[pyo3(name = "find_routes", signature = (target, depth=5, max_routes=5, beam_width=0, building_blocks=None, avoid_elements="", require_elements="", verbose=false))]
pub fn find_routes_py(
    target: &str,
    depth: u32,
    max_routes: usize,
    beam_width: usize,
    building_blocks: Option<Vec<String>>,
    avoid_elements: &str,
    require_elements: &str,
    verbose: bool,
) -> PyResult<String> {
    let env = match building_blocks {
        Some(ref bbs) => {
            let refs: Vec<&str> = bbs.iter().map(|s| s.as_str()).collect();
            ChemEnv::in_memory(&refs)
        }
        None => ChemEnv::load("data/building_blocks.smi")
            .unwrap_or_else(|_| ChemEnv::in_memory(crate::DEFAULT_BUILDING_BLOCKS)),
    };

    let rules = default_rules();
    let config = SearchConfig {
        max_depth: depth,
        max_routes,
        beam_width,
        forbidden_elements: elem_symbols_to_mask(avoid_elements),
        required_element_present: elem_symbols_to_mask(require_elements),
        verbose,
        ..Default::default()
    };
    let (routes, stats) = find_routes(target, &env, &rules, &config)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;

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

    serde_json::to_string(&output).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// RENKIN Python module.
#[pymodule]
pub fn renkin(_py: Python, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(find_routes_py, m)?)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
