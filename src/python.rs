use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use crate::bridge;
use crate::chem_env::{
    ChemEnv, default_rules, elem_symbols_to_mask, load_rules_from_file, mol_from_smiles,
};
use crate::search::{SearchConfig, diagnose, find_routes};

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
///     avoid_building_blocks (str): Comma-separated canonical SMILES of leaf BBs
///         to exclude from returned routes. Default: ``""`` (no constraint).
///     require_building_blocks (str): Comma-separated canonical SMILES of leaf BBs;
///         each returned route must contain at least one. Default: ``""`` (no constraint).
///     max_route_cost (float | None): Keep routes whose computed route cost is at
///         or below this inclusive limit. Default: ``None``.
///     min_confidence (float | None): Keep routes at or above this template-confidence
///         threshold. Default: ``None``.
///     min_success_probability (float | None): Keep routes at or above this
///         frequency-derived route score threshold. Default: ``None``.
///     require_reaction_families (str): Comma-separated reaction families; each
///         route must contain at least one. Default: ``""``.
///     avoid_reaction_families (str): Comma-separated reaction families; routes
///         containing any are excluded. Default: ``""``.
///     prefer_reaction_families (str): Comma-separated reaction families to rank
///         first without excluding other routes. Default: ``""``.
///     max_steps (int | None): Keep routes with at most this many reaction steps;
///         distinct from the search-depth limit. Default: ``None``.
///     candidate_trace_limit (int | None): Collect up to this many candidate
///         trace records for crowd-out diagnostics. Setting it also enables the
///         ``search_diagnostics`` output block. Default: ``None``.
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
///     search_diagnostics (bool): When ``True``, adds a
///         ``search_diagnostics`` block (beam eviction, cross-template
///         dedup, branching factor -- Issue #101) to the JSON output,
///         identical field names/shape to the ``renkin`` CLI's own
///         ``--search-diagnostics`` flag. Always accumulated internally
///         regardless of this flag (negligible bookkeeping cost); this
///         only controls whether it's serialized. Present on both the
///         route-found and empty-route branches when requested. Default:
///         ``False`` (omitted from the output, not present as ``null``).
///     spectator_bond_policy (str): ``"off"`` (default), ``"diagnostics_only"``,
///         or ``"gated"`` -- detects a real target bond a retro-rule's own
///         SMIRKS never declares broken but chematic silently drops from
///         precursors (``docs/design/spectator-bond-fail-closed-gating-v0.md``).
///         ``"diagnostics_only"`` records findings in the
///         ``search_diagnostics`` block's ``spectator_bond_loss_findings``
///         (requires ``search_diagnostics=True`` to see them) without
///         excluding any candidate. ``"gated"`` additionally excludes the
///         specific candidate a confident finding applies to, recording
///         each exclusion in ``spectator_bond_gated_out`` -- v1 scope only:
///         rules with no ``#`` in their SMIRKS; others stay
///         diagnostics-only regardless of this setting. Unrecognized
///         values raise ``ValueError``.
///     element_accounting_policy (str): ``"off"`` (default), ``"diagnostics_only"``,
///         or ``"gated"`` -- detects a candidate whose target needs more of
///         some heavy element than its precursors collectively supply
///         (``docs/design/candidate-time-element-accounting-gate-v0.md``),
///         an independent axis from ``spectator_bond_policy`` above.
///         ``"diagnostics_only"`` records the verdict in the
///         ``search_diagnostics`` block (requires ``search_diagnostics=True``
///         to see it) without excluding any candidate. ``"gated"``
///         additionally excludes the specific candidate, recording each
///         exclusion in ``element_accounting_gated_out``. Unrecognized
///         values raise ``ValueError``.
///     beam_diversity_policy (str): ``"off"`` (default), ``"diagnostics_only"``,
///         or ``"active"`` -- reserves ``beam_diversity_slots`` beam slots
///         for template-family diversity instead of pure score, so a
///         lower-scoring candidate from an underrepresented rule isn't
///         fully crowded out by many higher-scoring same-rule siblings
///         (``docs/design/diversity-reserved-beam-v0.md``).
///         ``"diagnostics_only"`` records what ``"active"`` would
///         additionally keep in the ``search_diagnostics`` block's
///         ``beam_diversity_stats`` (requires ``search_diagnostics=True``
///         to see them) without changing selection. ``"active"`` actually
///         reserves the slots. Unrecognized values raise ``ValueError``.
///     beam_diversity_slots (int): Beam slots reserved under
///         ``"diagnostics_only"``/``"active"`` above; ignored under
///         ``"off"``. Default: ``0`` (no reservation even if the policy is
///         opted into).
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
///     str: JSON string with retrosynthesis routes -- identical top-level
///     field set to the ``renkin`` CLI's own ``--format json`` output (the
///     two share the same underlying search code and JSON-assembly shape).
///     When ``routes_found > 0``: also has ``joint_success_probability``
///     (``1 - Π(1 - route.success_probability)`` across every returned
///     route -- a frequency-derived score, not a calibrated experimental
///     probability). When ``routes_found == 0``: ``diagnostics`` has
///     ``nodes_expanded``, ``max_depth_reached``, ``beam_limit_hit``,
///     ``matched_templates``, ``stock_hits``, ``likely_causes``,
///     ``suggestions``. In coverage mode, gains ``search_mode``,
///     ``selected_stage``, ``stage2_invoked``, ``stage1_timeout``,
///     ``stage2_timeout``, ``stage1_elapsed_ms``, ``stage2_elapsed_ms``,
///     ``total_elapsed_ms`` -- identical field names and shapes to the
///     ``renkin`` CLI's own coverage-mode JSON output. Absent (not
///     ``null``) in standard mode, byte-for-byte the same output as before
///     these fields existed.
///
/// Example::
///
///     import renkin, json
///     routes = json.loads(renkin.find_routes("CC(=O)Oc1ccccc1C(=O)O", depth=3))
///     print(routes["routes_found"])
#[pyfunction]
#[pyo3(name = "find_routes", signature = (target, depth=5, max_routes=5, beam_width=0, building_blocks=None, avoid_elements="", require_elements="", verbose=false, bb_prices_path=None, templates_path=None, template_metadata_path=None, reranker_model_path=None, reranker_freq_table_path=None, top_templates=None, search_mode="standard", coverage_templates_path=None, coverage_timeout_seconds=None, search_diagnostics=false, spectator_bond_policy="off", element_accounting_policy="off", beam_diversity_policy="off", beam_diversity_slots=0, avoid_building_blocks="", require_building_blocks="", max_route_cost=None, min_confidence=None, min_success_probability=None, require_reaction_families="", avoid_reaction_families="", prefer_reaction_families="", max_steps=None, candidate_trace_limit=None))]
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
    search_diagnostics: bool,
    spectator_bond_policy: &str,
    element_accounting_policy: &str,
    beam_diversity_policy: &str,
    beam_diversity_slots: usize,
    avoid_building_blocks: &str,
    require_building_blocks: &str,
    max_route_cost: Option<f64>,
    min_confidence: Option<f64>,
    min_success_probability: Option<f64>,
    require_reaction_families: &str,
    avoid_reaction_families: &str,
    prefer_reaction_families: &str,
    max_steps: Option<usize>,
    candidate_trace_limit: Option<usize>,
) -> PyResult<String> {
    crate::constraints::validate_route_thresholds(
        max_route_cost,
        min_confidence,
        min_success_probability,
    )
    .map_err(PyValueError::new_err)?;
    if search_mode != "standard" && search_mode != "coverage" {
        return Err(PyValueError::new_err(format!(
            "invalid search_mode {search_mode:?} (expected \"standard\" or \"coverage\")"
        )));
    }
    let spectator_bond_policy = match spectator_bond_policy {
        "off" => crate::spectator_bond::SpectatorBondPolicy::Off,
        "diagnostics_only" => crate::spectator_bond::SpectatorBondPolicy::DiagnosticsOnly,
        "gated" => crate::spectator_bond::SpectatorBondPolicy::Gated,
        other => {
            return Err(PyValueError::new_err(format!(
                "invalid spectator_bond_policy {other:?} (expected \"off\", \"diagnostics_only\", \
                 or \"gated\")"
            )));
        }
    };
    let element_accounting_policy = match element_accounting_policy {
        "off" => crate::search::ElementAccountingGatePolicy::Off,
        "diagnostics_only" => crate::search::ElementAccountingGatePolicy::DiagnosticsOnly,
        "gated" => crate::search::ElementAccountingGatePolicy::Gated,
        other => {
            return Err(PyValueError::new_err(format!(
                "invalid element_accounting_policy {other:?} (expected \"off\", \
                 \"diagnostics_only\", or \"gated\")"
            )));
        }
    };
    let beam_diversity_policy = match beam_diversity_policy {
        "off" => crate::search::BeamDiversityPolicy::Off,
        "diagnostics_only" => crate::search::BeamDiversityPolicy::DiagnosticsOnly,
        "active" => crate::search::BeamDiversityPolicy::Active,
        other => {
            return Err(PyValueError::new_err(format!(
                "invalid beam_diversity_policy {other:?} (expected \"off\", \"diagnostics_only\", \
                 or \"active\")"
            )));
        }
    };
    let search_diagnostics = search_diagnostics || candidate_trace_limit.is_some();
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
        spectator_bond_policy,
        element_accounting_policy,
        beam_diversity_policy,
        beam_diversity_slots,
        candidate_trace_cap: candidate_trace_limit,
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

    let (mut routes, stats, coverage_meta) = if search_mode == "coverage" {
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

    let avoided_building_blocks: Vec<&str> = avoid_building_blocks
        .split(',')
        .map(str::trim)
        .filter(|bb| !bb.is_empty())
        .collect();
    if !avoided_building_blocks.is_empty() {
        routes.retain(|route| {
            !route.building_blocks.iter().any(|bb| {
                avoided_building_blocks
                    .iter()
                    .any(|candidate| candidate == bb)
            })
        });
    }
    let required_building_blocks: Vec<&str> = require_building_blocks
        .split(',')
        .map(str::trim)
        .filter(|bb| !bb.is_empty())
        .collect();
    if !required_building_blocks.is_empty() {
        routes.retain(|route| {
            route.building_blocks.iter().any(|bb| {
                required_building_blocks
                    .iter()
                    .any(|candidate| candidate == bb)
            })
        });
    }
    if let Some(max_cost) = max_route_cost {
        routes.retain(|route| route.route_cost <= max_cost);
    }
    if let Some(limit) = max_steps {
        routes.retain(|route| route.steps.len() <= limit);
    }
    if let Some(minimum) = min_confidence {
        routes.retain(|route| route.confidence >= minimum);
    }
    if let Some(minimum) = min_success_probability {
        routes.retain(|route| route.success_probability >= minimum);
    }
    let required_families: Vec<&str> = require_reaction_families
        .split(',')
        .map(str::trim)
        .filter(|family| !family.is_empty())
        .collect();
    if !required_families.is_empty() {
        routes.retain(|route| {
            route.steps.iter().any(|step| {
                step.reaction_family
                    .as_deref()
                    .is_some_and(|family| required_families.contains(&family))
            })
        });
    }
    let avoided_families: Vec<&str> = avoid_reaction_families
        .split(',')
        .map(str::trim)
        .filter(|family| !family.is_empty())
        .collect();
    if !avoided_families.is_empty() {
        routes.retain(|route| {
            !route.steps.iter().any(|step| {
                step.reaction_family
                    .as_deref()
                    .is_some_and(|family| avoided_families.contains(&family))
            })
        });
    }
    let preferred_families: Vec<&str> = prefer_reaction_families
        .split(',')
        .map(str::trim)
        .filter(|family| !family.is_empty())
        .collect();
    if !preferred_families.is_empty() {
        routes.sort_by_key(|route| {
            let preferred = route.steps.iter().any(|step| {
                step.reaction_family
                    .as_deref()
                    .is_some_and(|family| preferred_families.contains(&family))
            });
            u8::from(!preferred)
        });
    }

    let reranker_failures_for_output = coverage_meta
        .as_ref()
        .map(|m| m.reranker_failures_summed)
        .unwrap_or(stats.reranker_failures);

    let mut output = if routes.is_empty() {
        let (causes, suggestions) = diagnose(&stats, depth);
        serde_json::json!({
            "target": target,
            "routes_found": 0,
            "routes": [],
            "diagnostics": {
                "nodes_expanded":    stats.nodes_expanded,
                "max_depth_reached": stats.max_depth_reached,
                "beam_limit_hit":    stats.beam_limit_hit,
                "matched_templates": stats.matched_templates,
                "stock_hits":        stats.stock_hits,
                "likely_causes":     causes,
                "suggestions":       suggestions,
            }
        })
    } else {
        let joint_success_probability = 1.0
            - routes
                .iter()
                .map(|r| 1.0 - r.success_probability)
                .product::<f64>();
        serde_json::json!({
            "target": target,
            "routes_found": routes.len(),
            "routes": routes,
            "joint_success_probability": joint_success_probability,
        })
    };
    if search_diagnostics {
        output["search_diagnostics"] = serde_json::to_value(&stats.crowd_out)
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
    }
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
                .flat_map(|ms| ms.iter().map(canon).collect::<Vec<_>>())
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
    let preds =
        py_predict_forward_core(&refs, &rules, max_results).map_err(PyValueError::new_err)?;
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
            .map_err(PyValueError::new_err)?;
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

/// Audit an already-completed retrosynthesis route (RENKIN or AiZynthFinder
/// export) for structural integrity, stock coverage, element accounting,
/// and forward-reaction reproducibility -- the first Python binding for
/// ``renkin audit-route`` (v0.29.0 Audit Policy Profiles), calling the
/// identical ``bridge::build_audit_route_report_with_policy`` pipeline the
/// CLI and WASM ``audit_route_v2`` use, so the same input+policy gets the
/// same verdict from every surface.
///
/// A thin binding on purpose: the caller reads any file (including a
/// gzip-compressed AiZynthFinder batch export) and passes the decoded
/// text in -- this function never touches the filesystem itself, matching
/// ``find_routes``'s own "pass data in, get a JSON string back" contract.
///
/// Args:
///     content (str): Route export JSON text -- a RENKIN ``--format json``
///         output, an AiZynthFinder single-route/batch export, a
///         ``renkin.syntheseus_exporter``-produced ``syntheseus-route-v1``
///         document, or a real SynPlanner ``write_routes_json`` export.
///     format (str): ``"auto"`` (default), ``"renkin"``, ``"aizynthfinder"``,
///         ``"syntheseus"``, or ``"synplanner"`` -- same vocabulary as the
///         CLI's ``--format`` flag.
///     stock_text (str): Optional ``.smi``-style stock listing (one SMILES
///         per line, ``#``-comments allowed). Default: ``""`` (no stock
///         configured -- stock validation reports ``not_evaluable``, never
///         a silent pass).
///     policy (str): ``"informational"``, ``"standard"`` (default), or
///         ``"strict"`` -- controls only how each route's ``status`` is
///         derived from findings already collected; never which findings
///         are detected or reported. See
///         ``docs/guides/audit-reproducibility-contract.md``.
///
/// Returns:
///     str: JSON string, the same ``AuditRouteReport`` shape
///     ``renkin audit-route --output json`` and the playground's
///     ``audit_route_v2`` WASM export both produce, including
///     ``audit_manifest.policy`` recording the policy actually used.
///
/// Raises:
///     ValueError: malformed JSON, an unrecognized route shape, or an
///         invalid ``format``/``policy`` value -- fail-loud, never a
///         partial or guessed result.
///
/// Example::
///
///     import json, renkin
///     with open("trees.json", encoding="utf-8") as f:
///         report = json.loads(
///             renkin.audit_route(f.read(), format="aizynthfinder", policy="strict")
///         )
///     print(report["summary"])
#[pyfunction]
#[pyo3(name = "audit_route", signature = (content, format="auto", stock_text="", policy="standard"))]
pub fn audit_route_py(
    content: &str,
    format: &str,
    stock_text: &str,
    policy: &str,
) -> PyResult<String> {
    let policy: bridge::AuditPolicy = policy.parse().map_err(PyValueError::new_err)?;
    let stock = (!stock_text.trim().is_empty()).then(|| bridge::parse_stock_text(stock_text));
    let rules = default_rules();
    let report = bridge::build_audit_route_report_with_policy(
        content,
        format,
        stock.as_ref(),
        &rules,
        policy,
    )
    .map_err(|e| PyValueError::new_err(format!("{e:#}")))?;
    serde_json::to_string(&report).map_err(|e| PyValueError::new_err(e.to_string()))
}

/// RENKIN Python module.
#[pymodule]
pub fn renkin(_py: Python, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(find_routes_py, m)?)?;
    m.add_function(wrap_pyfunction!(predict_forward_py, m)?)?;
    m.add_function(wrap_pyfunction!(validate_forward_py, m)?)?;
    m.add_function(wrap_pyfunction!(audit_route_py, m)?)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
