use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use crate::chem_env::{
    ChemEnv, default_rules, elem_symbols_to_mask, load_rules_from_file, mol_from_smiles,
};
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
///     templates_path (str | None): Path to an extracted SMIRKS templates .smi
///         file (tab-separated). None = hand-crafted rules only. Default: ``None``.
///     template_metadata_path (str | None): Path to a JSON metadata sidecar
///         (curated conditions/yields/warnings/references keyed by
///         `template_id`, see ``renkin template ids``). Matching steps get an
///         ``evidence`` field; unmatched templates get none -- nothing is
///         fabricated. Default: ``None``.
///     reranker_model_path (str | None): Path to a frozen LightGBM
///         ``model.txt`` for candidate reranking (Issue #101 Task 35);
///         ordering-only, requires ``reranker_freq_table_path`` too. Default:
///         ``None``.
///     reranker_freq_table_path (str | None): Path to the TRAIN-frozen
///         template ``frequency_table.json`` for the reranker. Default:
///         ``None``.
///     top_templates (int | None): If given, keep only the top-N
///         ``templates_path`` templates by frequency weight. Applies only
///         to Stage 1 (``templates_path``) -- coverage mode's Stage 2
///         (``coverage_templates_path``) always uses its full template set
///         unfiltered, matching the ``renkin`` CLI's ``--top-templates``
///         exactly. Default: ``None`` (no filtering).
///     search_mode (str): ``"standard"`` (default, unchanged behavior) or
///         ``"coverage"``. In coverage mode, Stage 1 (``templates_path``)
///         runs first; only if it finds nothing does Stage 2 run against
///         ``coverage_templates_path`` (Phase 41.18B,
///         ``docs/design/coverage-mode-v0.md``). A Stage-1 valid route is
///         never overwritten.
///     coverage_templates_path (str | None): Stage 2's template set;
///         required when ``search_mode="coverage"``, validated (existence,
///         readability, non-empty) before Stage 1 even runs. Default:
///         ``None``.
///     coverage_timeout_seconds (int | None): Optional positive-integer
///         wall-clock budget for Stage 2 only (cooperative cancellation,
///         not a hard real-time bound -- see ``SearchTermination::
///         DeadlineExceeded``'s doc in ``src/search.rs``). ``0`` raises.
///         Default: ``None`` (unlimited).
///
///     Passing only one of the two reranker paths, or the model failing to
///     load, falls back to legacy ordering with a message printed to
///     stderr -- never a hard error, matching the ``renkin`` CLI's
///     ``--reranker-model``/``--reranker-freq-table`` flags exactly. When a
///     reranker is configured (either path given), the JSON output gains a
///     ``reranker_failures`` integer field -- ``0`` for a fully healthy
///     run, nonzero if inference degraded mid-search; the field is absent
///     entirely (not ``null``) when no reranker was configured. In coverage
///     mode this is the sum across every stage that actually ran.
///
///     Coverage mode does not support ``--bond-index``/an ONNX
///     ``--scorer``/an active ring-context policy in v0 -- none of these
///     are exposed as Python parameters today, so this restriction has no
///     practical effect from Python yet, but the same shared validation
///     the ``renkin`` CLI uses (``renkin::coverage_mode::
///     validate_coverage_mode_config``) still runs.
///
/// Returns:
///     str: JSON string with retrosynthesis routes. In coverage mode, gains
///     ``search_mode``, ``selected_stage``, ``stage2_invoked``,
///     ``stage1_timeout``, ``stage2_timeout``, ``stage1_elapsed_ms``,
///     ``stage2_elapsed_ms``, ``total_elapsed_ms`` -- identical field names
///     and shapes to the ``renkin`` CLI's own coverage-mode JSON output.
///     Absent (not ``null``) in standard mode, byte-for-byte the same
///     output as before these fields existed.
///
/// Example::
///
///     import renkin, json
///     routes = json.loads(renkin.find_routes("CC(=O)Oc1ccccc1C(=O)O", depth=3))
///     print(routes["routes_found"])
#[pyfunction]
#[pyo3(name = "find_routes", signature = (target, depth=5, max_routes=5, beam_width=0, building_blocks=None, avoid_elements="", require_elements="", verbose=false, bb_prices_path=None, templates_path=None, template_metadata_path=None, reranker_model_path=None, reranker_freq_table_path=None, top_templates=None, search_mode="standard", coverage_templates_path=None, coverage_timeout_seconds=None))]
#[allow(clippy::too_many_arguments)]
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
    templates_path: Option<&str>,
    template_metadata_path: Option<&str>,
    reranker_model_path: Option<&str>,
    reranker_freq_table_path: Option<&str>,
    top_templates: Option<usize>,
    search_mode: &str,
    coverage_templates_path: Option<&str>,
    coverage_timeout_seconds: Option<u64>,
) -> PyResult<String> {
    if search_mode != "standard" && search_mode != "coverage" {
        return Err(PyValueError::new_err(format!(
            "invalid search_mode {search_mode:?} (expected \"standard\" or \"coverage\")"
        )));
    }
    if search_mode == "standard" {
        if coverage_templates_path.is_some() {
            return Err(PyValueError::new_err(
                "coverage_templates_path requires search_mode=\"coverage\"",
            ));
        }
        if coverage_timeout_seconds.is_some() {
            return Err(PyValueError::new_err(
                "coverage_timeout_seconds requires search_mode=\"coverage\"",
            ));
        }
    }
    if search_mode == "coverage" && coverage_timeout_seconds == Some(0) {
        return Err(PyValueError::new_err(
            "coverage_timeout_seconds must be a positive integer (got 0)",
        ));
    }

    let env = match building_blocks {
        Some(ref bbs) => {
            let refs: Vec<&str> = bbs.iter().map(|s| s.as_str()).collect();
            ChemEnv::in_memory(&refs)
        }
        None => ChemEnv::load("data/building_blocks.smi")
            .unwrap_or_else(|_| ChemEnv::in_memory(crate::DEFAULT_BUILDING_BLOCKS)),
    };

    let mut rules = default_rules();
    if let Some(path) = templates_path {
        let mut extra = load_rules_from_file(path);
        if let Some(k) = top_templates {
            extra = crate::chem_env::top_templates_by_weight(extra, k);
        }
        rules.extend(extra);
    }

    // Malformed metadata must fail before any search runs.
    let template_metadata = template_metadata_path
        .map(crate::evidence::load_template_metadata)
        .transpose()
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    if let Some(ref tm) = template_metadata {
        let known_ids: std::collections::HashSet<&str> =
            rules.iter().map(|r| r.template_id.as_str()).collect();
        crate::evidence::warn_unknown_templates(tm, &known_ids);
    }

    let bb_price_map = bb_prices_path.map(|path| {
        std::fs::read_to_string(path)
            .ok()
            .map(|content| {
                content
                    .lines()
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

    // Issue #101 Task 35: ordering-only candidate reranker, mirroring the
    // `renkin` CLI's --reranker-model/--reranker-freq-table exactly -- a
    // missing/mismatched pair or a load failure degrades to this crate's
    // pre-existing ordering rather than raising, never a hard error.
    let reranker: Option<std::sync::Arc<dyn crate::candidate::CandidateReranker>> =
        match (reranker_model_path, reranker_freq_table_path) {
            (Some(model_path), Some(freq_path)) => {
                match crate::reranker::RuntimeReranker::from_paths(model_path, freq_path) {
                    Ok(r) => Some(std::sync::Arc::new(r)),
                    Err(e) => {
                        eprintln!(
                            "warning: failed to load reranker_model_path/reranker_freq_table_path \
                             ({e:#}); falling back to legacy ordering for this run"
                        );
                        None
                    }
                }
            }
            (None, None) => None,
            _ => {
                eprintln!(
                    "warning: reranker_model_path and reranker_freq_table_path must both be \
                     given; falling back to legacy ordering for this run"
                );
                None
            }
        };
    let config = SearchConfig {
        max_depth: depth,
        max_routes,
        beam_width,
        forbidden_elements: elem_symbols_to_mask(avoid_elements),
        required_element_present: elem_symbols_to_mask(require_elements),
        verbose,
        bb_price_map,
        template_metadata: template_metadata.map(|tm| tm.templates),
        reranker,
        ..Default::default()
    };

    struct CoverageModeMeta {
        selected_stage: &'static str,
        stage2_invoked: bool,
        stage1_timeout: bool,
        stage2_timeout: bool,
        stage1_elapsed_ms: f64,
        stage2_elapsed_ms: Option<f64>,
        total_elapsed_ms: f64,
        reranker_failures_summed: u64,
    }

    let (routes, stats, coverage_meta) = if search_mode == "coverage" {
        let coverage_path = coverage_templates_path.ok_or_else(|| {
            PyValueError::new_err("search_mode=\"coverage\" requires coverage_templates_path")
        })?;
        // Fail-loud validation before Stage 1 runs at all -- same contract
        // as the renkin CLI's --search-mode coverage.
        crate::coverage_mode::validate_coverage_mode_config(&config)
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        let coverage_rules = crate::coverage_mode::load_coverage_rules(coverage_path)
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        let coverage_timeout = coverage_timeout_seconds.map(std::time::Duration::from_secs);
        let result = crate::coverage_mode::run_coverage_mode(
            target,
            &env,
            &rules,
            &config,
            &coverage_rules,
            coverage_timeout,
        )
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
        let meta = CoverageModeMeta {
            selected_stage: match result.selected_stage {
                crate::coverage_mode::SelectedStage::Stage1 => "stage1",
                crate::coverage_mode::SelectedStage::Stage2 => "stage2",
            },
            stage2_invoked: result.stage2_invoked,
            stage1_timeout: result.stage1_timeout,
            stage2_timeout: result.stage2_timeout,
            stage1_elapsed_ms: result.stage1_elapsed_ms,
            stage2_elapsed_ms: result.stage2_elapsed_ms,
            total_elapsed_ms: result.total_elapsed_ms,
            reranker_failures_summed: result.reranker_failures,
        };
        (result.routes, result.stats, Some(meta))
    } else {
        let (routes, stats) = find_routes(target, &env, &rules, &config)
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        (routes, stats, None)
    };

    let reranker_failures_for_output = coverage_meta
        .as_ref()
        .map(|m| m.reranker_failures_summed)
        .unwrap_or(stats.reranker_failures);

    let mut output = if routes.is_empty() {
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
    // Mirrors the `renkin` CLI's exact contract (src/main.rs) -- surfaced
    // unconditionally whenever a reranker was configured, since a graceful
    // mid-run degrade (never a hard error) has no other way to be detected
    // by the caller. In coverage mode this is the sum across every stage
    // that actually ran, not just the selected one's.
    if config.reranker.is_some() {
        output["reranker_failures"] = serde_json::Value::from(reranker_failures_for_output);
    }
    if let Some(ref m) = coverage_meta {
        output["search_mode"] = serde_json::Value::from("coverage");
        output["selected_stage"] = serde_json::Value::from(m.selected_stage);
        output["stage2_invoked"] = serde_json::Value::from(m.stage2_invoked);
        output["stage1_timeout"] = serde_json::Value::from(m.stage1_timeout);
        output["stage2_timeout"] = serde_json::Value::from(m.stage2_timeout);
        output["stage1_elapsed_ms"] = serde_json::Value::from(m.stage1_elapsed_ms);
        output["stage2_elapsed_ms"] = serde_json::to_value(m.stage2_elapsed_ms)
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        output["total_elapsed_ms"] = serde_json::Value::from(m.total_elapsed_ms);
    }

    serde_json::to_string(&output).map_err(|e| PyValueError::new_err(e.to_string()))
}

// ── Forward prediction helpers (inlined to avoid circular dep with renkin-forward) ──────

fn py_reverse_smirks(s: &str) -> Option<String> {
    let (lhs, rhs) = s.split_once(">>")?;
    Some(format!("{rhs}>>{lhs}"))
}

fn py_is_valid_smiles(s: &str) -> bool {
    let has_aromatic = s
        .bytes()
        .any(|b| matches!(b, b'c' | b'n' | b'o' | b's' | b'p'));
    !has_aromatic || s.bytes().any(|b| b.is_ascii_digit())
}

fn py_predict_forward_core(
    reactants: &[&str],
    rules: &[crate::chem_env::RetroRule],
    max_results: usize,
) -> Result<Vec<serde_json::Value>, String> {
    use chematic::rxn::run_reactants;
    use chematic::smiles::canonical_smiles as canon;

    let mols: Vec<_> = reactants
        .iter()
        .filter_map(|s| mol_from_smiles(s).ok())
        .collect();
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
        b["weight"]
            .as_f64()
            .unwrap_or(0.0)
            .partial_cmp(&a["weight"].as_f64().unwrap_or(0.0))
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
    let steps = v["steps"]
        .as_array()
        .ok_or_else(|| PyValueError::new_err("route JSON must have a 'steps' array"))?;

    let mut rules = default_rules();
    if let Some(path) = templates_path {
        rules.extend(load_rules_from_file(path));
    }

    let mut results: Vec<serde_json::Value> = Vec::new();
    for (idx, step) in steps.iter().enumerate() {
        let target = step["target"].as_str().unwrap_or("");
        let prec_refs: Vec<&str> = step["precursors"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        let preds = py_predict_forward_core(&prec_refs, &rules, max_results)
            .map_err(|e| PyValueError::new_err(e))?;
        let target_canon = mol_from_smiles(target)
            .ok()
            .map(|m| canon(&m))
            .unwrap_or_else(|| target.to_string());
        let verified = preds.iter().any(|p| {
            p["products"]
                .as_array()
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
