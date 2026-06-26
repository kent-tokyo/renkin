use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use crate::chem_env::{ChemEnv, default_rules, elem_symbols_to_mask, load_rules_from_file, mol_from_smiles};
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
#[pyo3(name = "find_routes", signature = (target, depth=5, max_routes=5, beam_width=0, building_blocks=None, avoid_elements="", require_elements="", verbose=false, bb_prices_path=None))]
pub fn find_routes_py(
    target: &str,
    depth: u32,
    max_routes: usize,
    beam_width: usize,
    building_blocks: Option<Vec<String>>,
    avoid_elements: &str,
    require_elements: &str,
    verbose: bool,
    bb_prices_path: Option<&str>,
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
    let bb_price_map = bb_prices_path.map(|path| {
        std::fs::read_to_string(path)
            .ok()
            .map(|content| {
                content.lines()
                    .filter(|l| !l.is_empty() && !l.starts_with('#'))
                    .filter_map(|l| {
                        let (smiles, price) = l.split_once(',')?;
                        let price: f64 = price.trim().parse().ok()?;
                        Some((smiles.trim().to_string(), price))
                    })
                    .collect::<std::collections::HashMap<String, f64>>()
            })
            .unwrap_or_default()
    });
    let config = SearchConfig {
        max_depth: depth,
        max_routes,
        beam_width,
        forbidden_elements: elem_symbols_to_mask(avoid_elements),
        required_element_present: elem_symbols_to_mask(require_elements),
        verbose,
        bb_price_map,
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

// ── Forward prediction helpers (inlined to avoid circular dep with renkin-forward) ──────

fn py_reverse_smirks(s: &str) -> Option<String> {
    let (lhs, rhs) = s.split_once(">>")?;
    Some(format!("{rhs}>>{lhs}"))
}

fn py_is_valid_smiles(s: &str) -> bool {
    let has_aromatic = s.bytes().any(|b| matches!(b, b'c' | b'n' | b'o' | b's' | b'p'));
    !has_aromatic || s.bytes().any(|b| b.is_ascii_digit())
}

fn py_predict_forward_core(
    reactants: &[&str],
    rules: &[crate::chem_env::RetroRule],
    max_results: usize,
) -> Result<Vec<serde_json::Value>, String> {
    use chematic::rxn::run_reactants;
    use chematic::smiles::canonical_smiles as canon;

    let mols: Vec<_> = reactants.iter().filter_map(|s| mol_from_smiles(s).ok()).collect();
    if mols.len() != reactants.len() {
        return Err("one or more reactant SMILES failed to parse".into());
    }
    let mol_refs: Vec<_> = mols.iter().collect();

    let mut preds: Vec<serde_json::Value> = rules
        .iter()
        .filter(|r| !r.smirks.is_empty())
        .filter_map(|rule| {
            let fwd = py_reverse_smirks(&rule.smirks)?;
            let outcomes = run_reactants(&fwd, &mol_refs).ok()?;
            if outcomes.is_empty() { return None; }
            let products: Vec<String> = outcomes
                .into_iter()
                .flat_map(|ms| ms.iter().map(|m| canon(m)).collect::<Vec<_>>())
                .filter(|s| py_is_valid_smiles(s))
                .collect();
            if products.is_empty() { return None; }
            Some(serde_json::json!({ "template": rule.name, "products": products, "weight": rule.weight }))
        })
        .collect();

    preds.sort_unstable_by(|a, b| {
        b["weight"].as_f64().unwrap_or(0.0).partial_cmp(&a["weight"].as_f64().unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    preds.truncate(max_results);
    Ok(preds)
}

/// Predict forward reaction products from a list of reactant SMILES.
///
/// Uses reversed SMIRKS templates (retro templates applied in forward direction).
/// Graph-based rules are not supported and are silently skipped.
///
/// Args:
///     reactants (list[str]): Reactant SMILES strings.
///     templates_path (str | None): Path to a templates .smi file. None = hand-crafted rules only.
///     max_results (int): Maximum number of predictions to return. Default: 5.
///
/// Returns:
///     str: JSON list of ``{"template": str, "products": [str], "weight": float}``.
#[pyfunction]
#[pyo3(name = "predict_forward", signature = (reactants, templates_path=None, max_results=5))]
pub fn predict_forward_py(
    reactants: Vec<String>,
    templates_path: Option<&str>,
    max_results: usize,
) -> PyResult<String> {
    let mut rules = default_rules();
    if let Some(path) = templates_path {
        rules.extend(load_rules_from_file(path));
    }
    let refs: Vec<&str> = reactants.iter().map(|s| s.as_str()).collect();
    let preds = py_predict_forward_core(&refs, &rules, max_results)
        .map_err(|e| PyValueError::new_err(e))?;
    serde_json::to_string(&preds).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// Validate each step of a retrosynthetic route using forward reaction prediction.
///
/// Args:
///     route_json (str): A single route object (JSON) from ``find_routes()["routes"][0]``.
///     templates_path (str | None): Path to a templates .smi file. None = hand-crafted rules only.
///     max_results (int): Max forward predictions per step. Default: 5.
///
/// Returns:
///     str: JSON list of ``{"step_index": int, "target": str, "verified": bool, "top_predictions": [...]}``.
#[pyfunction]
#[pyo3(name = "validate_forward", signature = (route_json, templates_path=None, max_results=5))]
pub fn validate_forward_py(
    route_json: &str,
    templates_path: Option<&str>,
    max_results: usize,
) -> PyResult<String> {
    use chematic::smiles::canonical_smiles as canon;

    let v: serde_json::Value = serde_json::from_str(route_json)
        .map_err(|e| PyValueError::new_err(format!("invalid JSON: {e}")))?;
    let steps = v["steps"].as_array()
        .ok_or_else(|| PyValueError::new_err("route JSON must have a 'steps' array"))?;

    let mut rules = default_rules();
    if let Some(path) = templates_path {
        rules.extend(load_rules_from_file(path));
    }

    let mut results: Vec<serde_json::Value> = Vec::new();
    for (idx, step) in steps.iter().enumerate() {
        let target = step["target"].as_str().unwrap_or("");
        let prec_refs: Vec<&str> = step["precursors"].as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        let preds = py_predict_forward_core(&prec_refs, &rules, max_results)
            .map_err(|e| PyValueError::new_err(e))?;
        let target_canon = mol_from_smiles(target).ok()
            .map(|m| canon(&m))
            .unwrap_or_else(|| target.to_string());
        let verified = preds.iter().any(|p| {
            p["products"].as_array()
                .map(|a| a.iter().any(|v| v.as_str() == Some(&target_canon)))
                .unwrap_or(false)
        });
        results.push(serde_json::json!({
            "step_index": idx, "target": target, "verified": verified, "top_predictions": preds
        }));
    }
    serde_json::to_string(&results).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// RENKIN Python module.
#[pymodule]
pub fn renkin(_py: Python, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(find_routes_py, m)?)?;
    m.add_function(wrap_pyfunction!(predict_forward_py, m)?)?;
    m.add_function(wrap_pyfunction!(validate_forward_py, m)?)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
