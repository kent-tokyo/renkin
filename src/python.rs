use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use crate::chem_env::{ChemEnv, default_rules};
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
#[pyo3(name = "find_routes", signature = (target, depth=5, max_routes=5, beam_width=0, building_blocks=None))]
pub fn find_routes_py(
    target: &str,
    depth: u32,
    max_routes: usize,
    beam_width: usize,
    building_blocks: Option<Vec<String>>,
) -> PyResult<String> {
    let env = match building_blocks {
        Some(ref bbs) => {
            let refs: Vec<&str> = bbs.iter().map(|s| s.as_str()).collect();
            ChemEnv::in_memory(&refs)
        }
        None => ChemEnv::load("data/building_blocks.smi").unwrap_or_else(|_| {
            ChemEnv::in_memory(crate::DEFAULT_BUILDING_BLOCKS)
        }),
    };

    let rules = default_rules();
    let config = SearchConfig { max_depth: depth, max_routes, beam_width };
    let routes = find_routes(target, &env, &rules, &config)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;

    let output = serde_json::json!({
        "target": target,
        "routes_found": routes.len(),
        "routes": routes,
    });

    serde_json::to_string(&output).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// RENKIN Python module.
#[pymodule]
pub fn renkin(_py: Python, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(find_routes_py, m)?)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
