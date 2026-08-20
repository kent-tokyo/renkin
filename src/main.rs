#![forbid(unsafe_code)]

use renkin::DEFAULT_BUILDING_BLOCKS;
use renkin::bridge;
use renkin::chem_env;
use renkin::display;
use renkin::evidence_match;
use renkin::ring_context;
use renkin::search::{self, SearchConfig};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct Output {
    target: String,
    routes_found: usize,
    routes: Vec<search::Route>,
    /// Combines each route's frequency-derived route_score across all
    /// returned routes: 1 − Π(1 − route.success_probability). Not a
    /// calibrated probability that any route succeeds -- route.success_probability
    /// itself is a template-frequency ranking score, not a measured or
    /// predicted experimental success rate.
    joint_success_probability: f64,
    /// Beam/crowd-out diagnostics (Issue #101). Only present with
    /// `--search-diagnostics`; omitted (not `null`) by default so existing
    /// consumers see byte-identical output.
    #[serde(skip_serializing_if = "Option::is_none")]
    search_diagnostics: Option<search::CrowdOutDiagnostics>,
    /// Issue #101 Task 35: only present when `--reranker-model`/
    /// `--reranker-freq-table` were both given and loaded successfully.
    /// A nonzero value means the reranker degraded to legacy ordering
    /// partway through this search (see `SearchStats::reranker_failures`'
    /// doc) -- surfaced unconditionally whenever the reranker was active,
    /// not gated behind `--search-diagnostics`, since a paired ON-vs-OFF
    /// comparison has no other way to tell "reranker ON and healthy for
    /// this whole run" apart from "reranker ON but silently degraded"
    /// (a run that exits 0 either way).
    #[serde(skip_serializing_if = "Option::is_none")]
    reranker_failures: Option<u64>,
    /// Phase 41.18B (coverage mode): present only when `--search-mode
    /// coverage` was used -- omitted (not `null`) for standard mode, so
    /// existing consumers see byte-identical output. When a reranker is
    /// also configured, `reranker_failures` above is the sum across every
    /// stage that actually ran, not just this one's.
    #[serde(skip_serializing_if = "Option::is_none")]
    search_mode: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    selected_stage: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stage2_invoked: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stage1_timeout: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stage2_timeout: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stage1_elapsed_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stage2_elapsed_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_elapsed_ms: Option<f64>,
}

// ..Default::default() is needed when nn-scoring feature is enabled (adds nn_scorer field).
// When the feature is off, all fields are explicit, making the spread redundant — suppress lint.
#[allow(clippy::needless_update)]
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // Subcommand dispatch
    if args.get(1).map(|s| s.as_str()) == Some("stock") {
        return run_stock(&args[2..]);
    }
    if args.get(1).map(|s| s.as_str()) == Some("template") {
        return run_template(&args[2..]);
    }
    if args.get(1).map(|s| s.as_str()) == Some("evidence") {
        return run_evidence(&args[2..]);
    }
    if args.get(1).map(|s| s.as_str()) == Some("audit-route") {
        return run_audit_route(&args[2..]);
    }

    let mut target: Option<String> = None;
    let mut max_depth: u32 = 5;
    let mut bb_path: Option<String> = None;
    let mut templates_path: Option<String> = None;
    let mut template_metadata_path: Option<String> = None;
    let mut top_templates: Option<usize> = None;
    let mut max_routes: usize = 5;
    let mut beam_width: usize = 0;
    let mut format: String = "json".to_string();
    let mut avoid_elements: String = String::new();
    let mut require_elements: String = String::new();
    let mut verbose = false;
    let mut search_diagnostics = false;
    let mut candidate_trace_limit: Option<usize> = None;
    let mut bond_index = false;
    let mut bb_prices_path: Option<String> = None;
    let mut stock_path: Option<String> = None;
    let mut objectives_spec: String = "cost:min,success_probability:max,steps:min".to_string();
    let mut constraints_path: Option<String> = None;
    #[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
    let mut scorer_path: Option<String> = None;
    let mut ring_context_policy_arg: Option<String> = None;
    let mut ring_context_sidecar_path: Option<String> = None;
    let mut reranker_model_path: Option<String> = None;
    let mut reranker_freq_table_path: Option<String> = None;
    let mut search_mode_arg: Option<String> = None;
    let mut coverage_templates_path: Option<String> = None;
    let mut coverage_timeout_secs_arg: Option<String> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--target" | "-t" => {
                i += 1;
                if i < args.len() {
                    target = Some(args[i].clone());
                }
            }
            "--depth" | "-d" => {
                i += 1;
                if i < args.len() {
                    max_depth = args[i].parse().unwrap_or(5);
                }
            }
            "--building-blocks" | "-b" => {
                i += 1;
                if i < args.len() {
                    bb_path = Some(args[i].clone());
                }
            }
            "--templates" => {
                i += 1;
                if i < args.len() {
                    templates_path = Some(args[i].clone());
                }
            }
            "--template-metadata" => {
                i += 1;
                if i < args.len() {
                    template_metadata_path = Some(args[i].clone());
                }
            }
            "--top-templates" => {
                i += 1;
                if i < args.len() {
                    top_templates = args[i].parse().ok();
                }
            }
            "--max-routes" | "-n" => {
                i += 1;
                if i < args.len() {
                    max_routes = args[i].parse().unwrap_or(5);
                }
            }
            "--beam-width" | "-w" => {
                i += 1;
                if i < args.len() {
                    beam_width = args[i].parse().unwrap_or(0);
                }
            }
            "--format" | "-f" => {
                i += 1;
                if i < args.len() {
                    format = args[i].clone();
                }
            }
            "--avoid-elements" | "-e" => {
                i += 1;
                if i < args.len() {
                    avoid_elements = args[i].clone();
                }
            }
            "--require-elements" | "-r" => {
                i += 1;
                if i < args.len() {
                    require_elements = args[i].clone();
                }
            }
            "--verbose" | "-v" => {
                verbose = true;
            }
            "--search-diagnostics" => {
                search_diagnostics = true;
            }
            "--candidate-trace-limit" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    bail!("--candidate-trace-limit requires a <N> value");
                };
                let n: usize = v.parse().map_err(|_| {
                    anyhow::anyhow!(
                        "--candidate-trace-limit value must be a non-negative integer, got {v:?}"
                    )
                })?;
                candidate_trace_limit = Some(n);
                // Self-sufficient: requesting a trace implies wanting to see
                // it, without also having to remember --search-diagnostics.
                search_diagnostics = true;
            }
            "--bond-index" => {
                bond_index = true;
            }
            "--ring-context-policy" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    bail!(
                        "--ring-context-policy requires a value \
                         (disabled|audit-only|conservative|ring-only|element-only)"
                    );
                };
                ring_context_policy_arg = Some(v.clone());
            }
            "--ring-context-sidecar" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    bail!("--ring-context-sidecar requires a <path> value");
                };
                ring_context_sidecar_path = Some(v.clone());
            }
            "--reranker-model" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    bail!("--reranker-model requires a <path> value");
                };
                reranker_model_path = Some(v.clone());
            }
            "--reranker-freq-table" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    bail!("--reranker-freq-table requires a <path> value");
                };
                reranker_freq_table_path = Some(v.clone());
            }
            "--search-mode" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    bail!("--search-mode requires a <standard|coverage> value");
                };
                search_mode_arg = Some(v.clone());
            }
            "--coverage-templates" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    bail!("--coverage-templates requires a <path> value");
                };
                coverage_templates_path = Some(v.clone());
            }
            "--coverage-timeout-secs" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    bail!("--coverage-timeout-secs requires an <N> value");
                };
                coverage_timeout_secs_arg = Some(v.clone());
            }
            "--bb-prices" => {
                i += 1;
                if i < args.len() {
                    bb_prices_path = Some(args[i].clone());
                }
            }
            "--stock" => {
                i += 1;
                if i < args.len() {
                    stock_path = Some(args[i].clone());
                }
            }
            "--objectives" => {
                i += 1;
                if i < args.len() {
                    objectives_spec = args[i].clone();
                }
            }
            "--constraints" => {
                i += 1;
                if i < args.len() {
                    constraints_path = Some(args[i].clone());
                }
            }
            #[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
            "--scorer" => {
                i += 1;
                if i < args.len() {
                    scorer_path = Some(args[i].clone());
                }
            }
            _ => {}
        }
        i += 1;
    }

    let Some(target_smiles) = target else {
        bail!(
            "Usage: renkin --target <SMILES> [--depth <N>] [--max-routes <N>] \
             [--beam-width <N>] [--building-blocks <path>] [--templates <path>] \
             [--format json|tree|mermaid]\n\
             \n\
             Options:\n  \
             --target / -t      Target molecule SMILES\n  \
             --depth  / -d      Max retrosynthesis depth (default: 5)\n  \
             --max-routes / -n  Max routes to return (default: 5)\n  \
             --beam-width / -w  Beam search width, 0 = unlimited A* (default: 0)\n  \
             --building-blocks  Path to .smi file of commercial starting materials\n  \
             --templates        Path to extracted SMIRKS templates file (tab-separated)\n  \
             --template-metadata <path>  JSON sidecar of curated evidence keyed by template_id\n  \
             --format / -f      Output format: json (default), tree, mermaid\n  \
             --avoid-elements / -e  Comma-separated elements to ban from BBs (e.g. \"Br,I\")\n  \
             --require-elements / -r  Comma-separated elements each route must supply (e.g. \"B\")\n  \
             --verbose / -v         Print search statistics to stderr\n  \
             --search-diagnostics   Add a \"search_diagnostics\" block (beam eviction, \
             cross-template dedup, branching factor -- Issue #101) to JSON output\n  \
             --candidate-trace-limit <N>  Also collect up to N per-candidate trace records \
             (implies --search-diagnostics; offline diagnostic use, competitive program Phase 1B)\n  \
             --bond-index           Bond-center template index: ~24%% faster, no accuracy loss\n  \
             --bb-prices <path>     CSV (SMILES,price_per_gram) for route cost scoring\n  \
             --ring-context-policy <policy>  disabled (default) | audit-only | conservative | \
             ring-only | element-only\n  \
             --ring-context-sidecar <path>   Ring-context metadata JSON, required unless policy \
             is disabled\n  \
             --reranker-model <path>       Frozen LightGBM model.txt for candidate reranking \
             (Issue #101 Task 35); ordering-only, requires --reranker-freq-table too\n  \
             --reranker-freq-table <path>  TRAIN-frozen template frequency_table.json for the \
             reranker\n  \
             (either flag missing, or the model fails to load, falls back to legacy ordering \
             with a stderr warning -- never a hard error)\n  \
             --search-mode standard|coverage  standard (default): unchanged behavior. \
             coverage: Stage 1 (--templates) runs first; only if it finds nothing does Stage 2 \
             run against --coverage-templates (Phase 41.18B, docs/design/coverage-mode-v0.md)\n  \
             --coverage-templates <path>   Stage 2's template set; required with \
             --search-mode coverage, validated before Stage 1 runs\n  \
             --coverage-timeout-secs <N>   Optional positive-integer wall-clock budget for \
             Stage 2 only (cooperative cancellation, not a hard bound); default: unlimited\n  \
             coverage mode does not support --bond-index, --scorer, or an active \
             --ring-context-policy in v0 (fails loud before Stage 1 runs)"
        );
    };

    // Phase 41.18B: coverage mode. `search_mode_arg` absent or "standard"
    // is byte-for-byte the pre-existing path below -- this whole block is
    // additive. Resolved and validated here, before any of the env/rules/
    // scorer/ring-context loading below -- specifically so an unsupported
    // option combination (--bond-index / --scorer / an active
    // --ring-context-policy) is rejected by flag presence alone, before
    // this process attempts to load a real ONNX model or ring-context
    // sidecar file for a combination that's going to be rejected anyway.
    enum SearchMode {
        Standard,
        Coverage,
    }
    let search_mode = match search_mode_arg.as_deref() {
        None | Some("standard") => SearchMode::Standard,
        Some("coverage") => SearchMode::Coverage,
        Some(other) => bail!("invalid --search-mode '{other}' (expected standard|coverage)"),
    };
    match search_mode {
        SearchMode::Standard => {
            if coverage_templates_path.is_some() {
                bail!("--coverage-templates requires --search-mode coverage");
            }
            if coverage_timeout_secs_arg.is_some() {
                bail!("--coverage-timeout-secs requires --search-mode coverage");
            }
        }
        SearchMode::Coverage => {
            if coverage_templates_path.is_none() {
                bail!("--search-mode coverage requires --coverage-templates <path>");
            }
            let ring_context_policy_active = ring_context_policy_arg
                .as_deref()
                .is_some_and(|p| p != "disabled");
            #[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
            let onnx_scorer_active = scorer_path.is_some();
            #[cfg(not(all(not(target_arch = "wasm32"), feature = "nn-scoring")))]
            let onnx_scorer_active = false;
            renkin::coverage_mode::validate_coverage_mode_flags(
                bond_index,
                ring_context_policy_active,
                onnx_scorer_active,
            )?;
        }
    }
    let coverage_timeout: Option<std::time::Duration> = match coverage_timeout_secs_arg {
        None => None,
        Some(ref s) => {
            let n: u64 = s.parse().map_err(|_| {
                anyhow::anyhow!("--coverage-timeout-secs must be a positive integer, got {s:?}")
            })?;
            if n == 0 {
                bail!("--coverage-timeout-secs must be a positive integer (got 0)");
            }
            Some(std::time::Duration::from_secs(n))
        }
    };

    // --stock overrides --building-blocks and --bb-prices
    let (env, bb_price_map) = if let Some(ref path) = stock_path {
        let entries = load_stock_csv(path);
        let smiles_owned: Vec<String> = entries.iter().map(|e| e.smiles.clone()).collect();
        let smiles_refs: Vec<&str> = smiles_owned.iter().map(|s| s.as_str()).collect();
        let stock_env = chem_env::ChemEnv::in_memory(&smiles_refs);
        let prices: std::collections::HashMap<String, f64> = entries
            .into_iter()
            .filter_map(|e| e.price_jpy.map(|p| (e.smiles, p)))
            .collect();
        (stock_env, Some(prices))
    } else {
        let env = match bb_path {
            Some(ref path) => chem_env::ChemEnv::load(path)?,
            None => chem_env::ChemEnv::load("data/building_blocks.smi")
                .unwrap_or_else(|_| chem_env::ChemEnv::in_memory(DEFAULT_BUILDING_BLOCKS)),
        };
        let prices = bb_prices_path.as_deref().map(load_prices);
        (env, prices)
    };

    let mut rules = chem_env::default_rules();
    if let Some(ref path) = templates_path {
        let mut extra = chem_env::load_rules_from_file(path);
        if let Some(k) = top_templates {
            extra = chem_env::top_templates_by_weight(extra, k);
        }
        eprintln!("Loaded {} templates from {path}", extra.len());
        rules.extend(extra);
    }

    // Malformed metadata must fail before any search runs.
    let template_metadata = template_metadata_path
        .as_deref()
        .map(renkin::evidence::load_template_metadata)
        .transpose()?;
    if let Some(ref tm) = template_metadata {
        let known_ids: std::collections::HashSet<&str> =
            rules.iter().map(|r| r.template_id.as_str()).collect();
        renkin::evidence::warn_unknown_templates(tm, &known_ids);
    }
    #[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
    let nn_scorer: Option<std::sync::Arc<renkin::scorer::nn::TemplateScorer>> =
        scorer_path.as_deref().map(|p| {
            let top_k = rules.len();
            let rules_offset = renkin::chem_env::default_rules().len();
            renkin::scorer::nn::TemplateScorer::from_path(p, top_k, rules_offset)
                .map(std::sync::Arc::new)
                .unwrap_or_else(|e| {
                    eprintln!("scorer load error: {e}");
                    std::process::exit(1)
                })
        });

    // Issue #101 Task 35: ordering-only candidate reranker. Unlike --scorer
    // above, a problem here never aborts the run -- both flags are opt-in,
    // and the whole point of a staged rollout is that a bad model file or a
    // missing sibling flag degrades to this crate's pre-existing ordering
    // rather than blocking prediction.
    let reranker: Option<std::sync::Arc<dyn renkin::candidate::CandidateReranker>> = match (
        reranker_model_path.as_deref(),
        reranker_freq_table_path.as_deref(),
    ) {
        (Some(model_path), Some(freq_path)) => {
            match renkin::reranker::RuntimeReranker::from_paths(model_path, freq_path) {
                Ok(r) => Some(std::sync::Arc::new(r)),
                Err(e) => {
                    eprintln!(
                        "warning: failed to load --reranker-model/--reranker-freq-table \
                             ({e:#}); falling back to legacy ordering for this run"
                    );
                    None
                }
            }
        }
        (None, None) => None,
        _ => {
            eprintln!(
                "warning: --reranker-model and --reranker-freq-table must both be given; \
                     falling back to legacy ordering for this run"
            );
            None
        }
    };

    let ring_context_safety_policy = match ring_context_policy_arg.as_deref() {
        None | Some("disabled") => None,
        Some("audit-only") => Some(ring_context::ExtractedTemplateSafetyPolicy::AUDIT_ONLY),
        Some("conservative") => Some(ring_context::ExtractedTemplateSafetyPolicy::CONSERVATIVE),
        Some("ring-only") => Some(ring_context::ExtractedTemplateSafetyPolicy::RING_ONLY),
        Some("element-only") => Some(ring_context::ExtractedTemplateSafetyPolicy::ELEMENT_ONLY),
        Some(other) => {
            eprintln!(
                "error: invalid --ring-context-policy '{other}' \
                 (expected disabled|audit-only|conservative|ring-only|element-only)"
            );
            std::process::exit(1);
        }
    };
    let ring_context_config = match ring_context_safety_policy {
        None => ring_context::RingContextConfig::Disabled,
        Some(policy) => {
            let Some(sidecar_path) = ring_context_sidecar_path.as_deref() else {
                eprintln!("error: --ring-context-policy requires --ring-context-sidecar <path>");
                std::process::exit(1);
            };
            let Some(templates_path) = templates_path.as_deref() else {
                eprintln!(
                    "error: --ring-context-policy requires --templates <path> (the sidecar is validated against the loaded template file's exact content)"
                );
                std::process::exit(1);
            };
            let templates_content = std::fs::read_to_string(templates_path).unwrap_or_else(|e| {
                eprintln!("error: could not read --templates file {templates_path}: {e}");
                std::process::exit(1);
            });
            match ring_context::RingContextGuard::load(sidecar_path, &templates_content) {
                Ok(g) => ring_context::RingContextConfig::Guarded {
                    guard: std::sync::Arc::new(g),
                    policy,
                },
                Err(e) => {
                    eprintln!("error: failed to load ring-context sidecar: {e}");
                    std::process::exit(1);
                }
            }
        }
    };

    let constraints: ConstraintSpec = constraints_path
        .as_deref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    // constraints override CLI flags when present
    let eff_depth = constraints.max_depth.unwrap_or(max_depth);
    let avoid_mask = chem_env::elem_symbols_to_mask(&avoid_elements)
        | chem_env::elem_symbols_to_mask(
            &constraints
                .avoid_elements
                .as_deref()
                .unwrap_or(&[])
                .join(","),
        );
    let require_mask = chem_env::elem_symbols_to_mask(&require_elements)
        | chem_env::elem_symbols_to_mask(
            &constraints
                .require_elements
                .as_deref()
                .unwrap_or(&[])
                .join(","),
        );
    if let Some(ref obj) = constraints.objectives {
        objectives_spec = obj.clone();
    }

    let config = SearchConfig {
        max_depth: eff_depth,
        max_routes,
        beam_width,
        forbidden_elements: avoid_mask,
        required_element_present: require_mask,
        verbose,
        bond_index,
        bb_price_map,
        template_metadata: template_metadata.map(|tm| tm.templates),
        #[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
        nn_scorer,
        ring_context: ring_context_config,
        candidate_trace_cap: candidate_trace_limit,
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

    let (mut routes, stats, coverage_meta): (
        Vec<search::Route>,
        search::SearchStats,
        Option<CoverageModeMeta>,
    ) = match search_mode {
        SearchMode::Standard => {
            let (routes, stats) = search::find_routes(&target_smiles, &env, &rules, &config)?;
            (routes, stats, None)
        }
        SearchMode::Coverage => {
            // Unsupported-combination and --coverage-templates-presence
            // validation already happened above, before this process did
            // any env/rules/scorer/ring-context loading -- only the
            // asset load itself (fail-loud on a bad path) remains here.
            let coverage_path = coverage_templates_path
                .as_deref()
                .expect("already validated above: SearchMode::Coverage implies Some");
            let coverage_rules = renkin::coverage_mode::load_coverage_rules(coverage_path)?;
            let result = renkin::coverage_mode::run_coverage_mode(
                &target_smiles,
                &env,
                &rules,
                &config,
                &coverage_rules,
                coverage_timeout,
            )?;
            let meta = CoverageModeMeta {
                selected_stage: match result.selected_stage {
                    renkin::coverage_mode::SelectedStage::Stage1 => "stage1",
                    renkin::coverage_mode::SelectedStage::Stage2 => "stage2",
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
        }
    };
    apply_constraints(&mut routes, &constraints);

    match format.as_str() {
        "tree" => {
            println!("Target: {target_smiles}");
            println!("Routes found: {}\n", routes.len());
            for (i, route) in routes.iter().enumerate() {
                print!(
                    "{}",
                    display::format_route_tree(route, &target_smiles, i + 1)
                );
                println!();
            }
        }
        "mermaid" => {
            for (i, route) in routes.iter().enumerate() {
                println!(
                    "{}",
                    display::format_route_mermaid(route, &target_smiles, i + 1)
                );
            }
        }
        "explain" => {
            for (i, route) in routes.iter().enumerate() {
                print!("{}", display::explain_route(route, &target_smiles, i + 1));
            }
        }
        "compare" | "table" => {
            println!("{}", display::format_route_table(&routes));
        }
        "compare-json" => {
            #[derive(serde::Serialize)]
            struct RouteCompare {
                route_num: usize,
                steps: usize,
                depth: u32,
                confidence: f64,
                success_probability: f64,
                route_cost: f64,
                convergency: f64,
                families: Vec<String>,
            }
            let rows: Vec<RouteCompare> = routes
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    let mut families: Vec<String> = Vec::new();
                    for step in &r.steps {
                        if let Some(f) = step.reaction_family.as_deref()
                            && !families.iter().any(|x| x == f)
                        {
                            families.push(f.to_string());
                        }
                    }
                    RouteCompare {
                        route_num: i + 1,
                        steps: r.steps.len(),
                        depth: r.depth,
                        confidence: r.confidence,
                        success_probability: r.success_probability,
                        route_cost: r.route_cost,
                        convergency: r.convergency,
                        families,
                    }
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&rows)?);
        }
        "pareto" => {
            let objs = parse_objectives(&objectives_spec);
            let front = pareto_front_indices(&routes, &objs);
            let obj_labels: Vec<String> = objs
                .iter()
                .map(|(f, d)| format!("{}:{}", f.as_str(), d.as_str()))
                .collect();
            #[derive(serde::Serialize)]
            struct ParetoRoute {
                route_num: usize,
                route_cost: f64,
                success_probability: f64,
                steps: usize,
                depth: u32,
                confidence: f64,
                convergency: f64,
                #[serde(skip_serializing_if = "Option::is_none")]
                tradeoff: Option<String>,
            }
            let front_routes: Vec<ParetoRoute> = front
                .iter()
                .map(|&idx| ParetoRoute {
                    route_num: idx + 1,
                    route_cost: routes[idx].route_cost,
                    success_probability: routes[idx].success_probability,
                    steps: routes[idx].steps.len(),
                    depth: routes[idx].depth,
                    confidence: routes[idx].confidence,
                    convergency: routes[idx].convergency,
                    tradeoff: tradeoff_label(idx, &front, &routes, &objs),
                })
                .collect();
            let out = serde_json::json!({
                "target": target_smiles,
                "routes_searched": routes.len(),
                "objectives": obj_labels,
                "pareto_front_size": front.len(),
                "pareto_front": front_routes,
                "dominated_count": routes.len() - front.len(),
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        _ => {
            let reranker_failures_for_output = coverage_meta
                .as_ref()
                .map(|m| m.reranker_failures_summed)
                .unwrap_or(stats.reranker_failures);
            if routes.is_empty() {
                let (causes, suggestions) = search::diagnose(&stats, max_depth);
                let mut out = serde_json::json!({
                    "target": target_smiles,
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
                });
                if search_diagnostics {
                    out["search_diagnostics"] = serde_json::to_value(&stats.crowd_out)?;
                }
                if config.reranker.is_some() {
                    out["reranker_failures"] = serde_json::to_value(reranker_failures_for_output)?;
                }
                if let Some(ref m) = coverage_meta {
                    out["search_mode"] = serde_json::Value::from("coverage");
                    out["selected_stage"] = serde_json::Value::from(m.selected_stage);
                    out["stage2_invoked"] = serde_json::Value::from(m.stage2_invoked);
                    out["stage1_timeout"] = serde_json::Value::from(m.stage1_timeout);
                    out["stage2_timeout"] = serde_json::Value::from(m.stage2_timeout);
                    out["stage1_elapsed_ms"] = serde_json::Value::from(m.stage1_elapsed_ms);
                    out["stage2_elapsed_ms"] = serde_json::to_value(m.stage2_elapsed_ms)?;
                    out["total_elapsed_ms"] = serde_json::Value::from(m.total_elapsed_ms);
                }
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                let joint_success_probability = 1.0
                    - routes
                        .iter()
                        .map(|r| 1.0 - r.success_probability)
                        .product::<f64>();
                let output = Output {
                    target: target_smiles,
                    routes_found: routes.len(),
                    joint_success_probability,
                    search_diagnostics: search_diagnostics.then_some(stats.crowd_out),
                    reranker_failures: config
                        .reranker
                        .is_some()
                        .then_some(reranker_failures_for_output),
                    search_mode: coverage_meta.as_ref().map(|_| "coverage"),
                    selected_stage: coverage_meta.as_ref().map(|m| m.selected_stage),
                    stage2_invoked: coverage_meta.as_ref().map(|m| m.stage2_invoked),
                    stage1_timeout: coverage_meta.as_ref().map(|m| m.stage1_timeout),
                    stage2_timeout: coverage_meta.as_ref().map(|m| m.stage2_timeout),
                    stage1_elapsed_ms: coverage_meta.as_ref().map(|m| m.stage1_elapsed_ms),
                    stage2_elapsed_ms: coverage_meta.as_ref().and_then(|m| m.stage2_elapsed_ms),
                    total_elapsed_ms: coverage_meta.as_ref().map(|m| m.total_elapsed_ms),
                    routes,
                };
                println!("{}", serde_json::to_string_pretty(&output)?);
            }
        }
    }
    Ok(())
}

// ── Constraint DSL ────────────────────────────────────────────────────────

#[derive(serde::Deserialize, Default)]
struct ConstraintSpec {
    avoid_elements: Option<Vec<String>>,
    require_elements: Option<Vec<String>>,
    max_steps: Option<usize>,
    max_depth: Option<u32>,
    min_confidence: Option<f64>,
    min_success_probability: Option<f64>,
    prefer_reaction_families: Option<Vec<String>>,
    objectives: Option<String>,
}

fn apply_constraints(routes: &mut Vec<search::Route>, c: &ConstraintSpec) {
    if let Some(n) = c.max_steps {
        routes.retain(|r| r.steps.len() <= n);
    }
    if let Some(v) = c.min_confidence {
        routes.retain(|r| r.confidence >= v);
    }
    if let Some(v) = c.min_success_probability {
        routes.retain(|r| r.success_probability >= v);
    }
    if let Some(ref fams) = c.prefer_reaction_families {
        routes.sort_by_key(|r| {
            let has = r.steps.iter().any(|s| {
                s.reaction_family
                    .as_deref()
                    .is_some_and(|f| fams.iter().any(|p| p == f))
            });
            u8::from(!has) // preferred first (0), others after (1)
        });
    }
}

// ── Template quality tools ────────────────────────────────────────────────

fn run_template(args: &[String]) -> Result<()> {
    let cmd = args.first().map(|s| s.as_str()).unwrap_or("help");
    let rest = if args.len() > 1 {
        &args[1..]
    } else {
        &[] as &[String]
    };
    match cmd {
        "stats" => template_stats(rest),
        "validate" => template_validate(rest),
        "dedup" => template_dedup(rest),
        "explain" => template_explain(rest),
        "coverage" => template_coverage(rest),
        "ids" => template_ids(rest),
        _ => {
            println!("Usage: renkin template <cmd> [args]");
            println!("  stats    <file.smi>                   — count, frequency distribution");
            println!("  validate <file.smi>                   — check SMIRKS validity");
            println!("  dedup    <file.smi>                   — find duplicate SMIRKS");
            println!("  explain  <name> [--templates <path>]  — show one template by name");
            println!("  coverage <targets.smi> [--templates <path>] [--depth N]");
            println!(
                "  ids      <file.smi> [--format tsv|json]  — stable template_id per template"
            );
            Ok(())
        }
    }
}

/// Returns the value following the first occurrence of `--name` in `args`,
/// or `None` if the flag isn't present (or has no following value).
fn flag_value<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|w| w[0] == name)
        .map(|w| w[1].as_str())
}

fn run_evidence(args: &[String]) -> Result<()> {
    let cmd = args.first().map(|s| s.as_str()).unwrap_or("help");
    let rest = if args.len() > 1 {
        &args[1..]
    } else {
        &[] as &[String]
    };
    match cmd {
        "match" => evidence_match(rest),
        "validate-sidecar" => evidence_validate_sidecar(rest),
        _ => {
            println!("Usage: renkin evidence <cmd> [args]");
            println!(
                "  match             --input <reactions.jsonl> [--templates <file.smi>] --output <matches.jsonl>"
            );
            println!(
                "  validate-sidecar  --metadata <sidecar.json>  — revalidate an evidence sidecar via RENKIN's own loader"
            );
            Ok(())
        }
    }
}

/// One line of `renkin evidence match --input`: an external reaction record
/// to batch-match against RENKIN's stable `template_id`s (see
/// `renkin::evidence_match::match_reaction_to_templates`).
#[derive(Deserialize)]
struct EvidenceMatchInputRow {
    record_id: String,
    target_smiles: String,
    #[serde(default)]
    precursor_smiles: Vec<String>,
}

/// One line of `renkin evidence match --output`.
#[derive(Serialize)]
struct EvidenceMatchOutputRow {
    record_id: String,
    canonical_target: String,
    canonical_precursors: Vec<String>,
    matching_template_ids: Vec<String>,
    status: evidence_match::TemplateMatchStatus,
}

/// Batch-matches every record in a JSONL file against RENKIN's stable
/// `template_id`s. Input order is preserved in the output. A malformed JSON
/// line is a hard error (whole process aborts, line number reported); a
/// malformed SMILES within an otherwise-valid record instead yields
/// `invalid_input` for that one record only. No network access. No progress
/// output on stdout -- only the JSONL result goes to `--output`; diagnostics
/// go to stderr via the returned `Result`'s error path.
fn evidence_match(args: &[String]) -> Result<()> {
    let input_path = flag_value(args, "--input")
        .context("renkin evidence match: --input <reactions.jsonl> is required")?;
    let output_path = flag_value(args, "--output")
        .context("renkin evidence match: --output <matches.jsonl> is required")?;

    let mut rules = chem_env::default_rules();
    if let Some(path) = flag_value(args, "--templates") {
        rules.extend(chem_env::load_rules_from_file(path));
    }

    let content = std::fs::read_to_string(input_path)
        .with_context(|| format!("failed to read --input file {input_path}"))?;

    let mut rows: Vec<EvidenceMatchInputRow> = Vec::new();
    for (i, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let row: EvidenceMatchInputRow = serde_json::from_str(line)
            .with_context(|| format!("{input_path}:{}: malformed JSONL", i + 1))?;
        rows.push(row);
    }

    let mut out = String::new();
    for row in rows {
        let result = evidence_match::match_reaction_to_templates(
            &row.target_smiles,
            &row.precursor_smiles,
            &rules,
        );
        let output_row = EvidenceMatchOutputRow {
            record_id: row.record_id,
            canonical_target: result.target_smiles,
            canonical_precursors: result.precursor_smiles,
            matching_template_ids: result.matching_template_ids,
            status: result.status,
        };
        out.push_str(&serde_json::to_string(&output_row)?);
        out.push('\n');
    }

    std::fs::write(output_path, out)
        .with_context(|| format!("failed to write --output file {output_path}"))?;

    Ok(())
}

/// Revalidates a metadata sidecar file via RENKIN's own loader
/// (`evidence::load_template_metadata`, which validates as part of loading).
/// Exits with an error (non-zero status) if validation fails -- a sidecar
/// that fails validation is never reported as success.
fn evidence_validate_sidecar(args: &[String]) -> Result<()> {
    let path = flag_value(args, "--metadata")
        .context("renkin evidence validate-sidecar: --metadata <sidecar.json> is required")?;
    renkin::evidence::load_template_metadata(path)
        .with_context(|| format!("sidecar {path} failed validation"))?;
    println!("OK: {path}");
    Ok(())
}

/// Loads a plain `.smi` stock file into the canonical-SMILES set
/// `bridge::audit_route::build_audit_route_report`'s `stock` expects --
/// thin file-reading wrapper around `bridge::parse_stock_text`, which owns
/// the actual line-parsing so the CLI's `--stock <PATH>` and the
/// playground's pasted/uploaded stock text share identical parsing.
fn load_audit_stock(path: &str) -> Result<std::collections::HashSet<String>> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("failed to read --stock {path}"))?;
    Ok(bridge::parse_stock_text(&content))
}

/// `renkin audit-route <PATH> [--format auto|renkin] [--stock <PATH>]
/// [--output human|json]` -- audits every route in a RENKIN `--format json`
/// output file via `bridge::route_graph::normalize_renkin_route` +
/// `bridge::audit::audit`. RENKIN-native input only: no AiZynthFinder
/// adapter, no HTML/DOI/condition/yield output, no alternative-
/// disconnection suggestions -- see `bridge` module docs for why those stay
/// out of scope here. stdout carries only the report (human text or JSON,
/// per `--output`); nothing else is printed to stdout. Exit code matches
/// the rest of this CLI's own convention (`main.rs`'s JSON route-search
/// path): 0 whenever the program ran to completion and produced a report,
/// including a `fail`/`partial` verdict -- that's a completed audit, not a
/// program error -- and non-zero only for usage/input errors (bad flags,
/// unreadable/malformed input).
/// Reads `path`, transparently gzip-decompressing if the content starts
/// with the gzip magic bytes (`1f 8b`) regardless of file extension --
/// `aizynthcli`'s own batch output is `.json.gz` by convention, but nothing
/// enforces the extension actually matches the content, so this sniffs the
/// real bytes rather than trusting the filename.
fn read_maybe_gzip(path: &str) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("failed to read {path}"))?;
    if bytes.starts_with(&[0x1f, 0x8b]) {
        use std::io::Read;
        let mut decoder = flate2::read::GzDecoder::new(&bytes[..]);
        let mut out = String::new();
        decoder
            .read_to_string(&mut out)
            .with_context(|| format!("{path}: failed to gzip-decompress"))?;
        Ok(out)
    } else {
        String::from_utf8(bytes).with_context(|| format!("{path}: not valid UTF-8"))
    }
}

fn run_audit_route(args: &[String]) -> Result<()> {
    let path = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .cloned()
        .context("renkin audit-route: <PATH> is required (usage: renkin audit-route <PATH> [--format auto|renkin|aizynthfinder] [--stock <PATH>] [--output human|json])")?;
    let format = flag_value(args, "--format").unwrap_or("auto");
    if !["auto", "renkin", "aizynthfinder"].contains(&format) {
        bail!(
            "renkin audit-route: unsupported --format {format:?} (only auto|renkin|aizynthfinder supported)"
        );
    }
    let output_format = flag_value(args, "--output").unwrap_or("human");
    if output_format != "human" && output_format != "json" {
        bail!(
            "renkin audit-route: unsupported --output {output_format:?} (only human|json supported)"
        );
    }

    let content = read_maybe_gzip(&path)?;
    let stock = flag_value(args, "--stock")
        .map(load_audit_stock)
        .transpose()?;
    let rules = chem_env::default_rules();

    let out = bridge::build_audit_route_report(&content, format, stock.as_ref(), &rules)
        .with_context(|| format!("{path}: audit input rejected"))?;

    if output_format == "json" {
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!(
            "{} routes audited — {} pass, {} fail, {} partial",
            out.summary.routes_total, out.summary.pass, out.summary.fail, out.summary.partial
        );
        for (i, report) in out.routes.iter().enumerate() {
            let status = match report.status {
                bridge::AuditStatus::Pass => "PASS",
                bridge::AuditStatus::Fail => "FAIL",
                bridge::AuditStatus::Partial => "PARTIAL",
            };
            println!("route {}/{}: {status}", i + 1, out.summary.routes_total);
            for finding in &report.findings {
                println!("  - {:?}", finding.code);
            }
            if let Some(stock_validation) = &report.stock_validation
                && let Some(reason) = &stock_validation.reason
            {
                println!("  - stock: {reason:?}");
            }
            for step in &report.steps {
                if let Some(reason) = &step.forward_validation.reason {
                    println!("  - forward: {reason:?}");
                }
            }
        }
    }

    Ok(())
}

/// Print `template_id`, current display name, SMIRKS, and weight for every
/// template in `path`, so users can author a `--template-metadata` sidecar.
/// Uses `chem_env::load_rules_from_file` (not `read_template_lines`) so the
/// reported `template_id`/name match exactly what real search runs assign --
/// that reader validates and filters lines the same way the search engine does.
fn template_ids(args: &[String]) -> Result<()> {
    // Skip `--format <value>` when picking the positional path argument, so
    // `renkin template ids --format json <file>` (or the flag before the
    // path at all) doesn't mistake "--format" itself for the file path.
    let path = args
        .iter()
        .enumerate()
        .find(|(i, a)| !a.starts_with("--") && !(*i > 0 && args[*i - 1] == "--format"))
        .map(|(_, a)| a.as_str())
        .unwrap_or("data/templates_extracted_5000.smi");
    // load_rules_from_file warns-and-returns-empty on a read error (matching its
    // use in the main search path); this subcommand must fail loudly instead.
    let meta =
        std::fs::metadata(path).with_context(|| format!("cannot read template file {path}"))?;
    if !meta.is_file() {
        bail!("template file {path} is not a file");
    }
    let format = args
        .windows(2)
        .find(|w| w[0] == "--format")
        .map(|w| w[1].as_str())
        .unwrap_or("tsv");
    let rules = chem_env::load_rules_from_file(path);

    if format == "json" {
        #[derive(Serialize)]
        struct Row<'a> {
            template_id: &'a str,
            name: &'a str,
            smirks: &'a str,
            weight: f64,
            approx_count: f64,
        }
        let rows: Vec<Row> = rules
            .iter()
            .map(|r| Row {
                template_id: &r.template_id,
                name: &r.name,
                smirks: &r.smirks,
                weight: r.weight,
                approx_count: r.weight.exp() - 1.0,
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows)?);
    } else {
        println!("template_id\tname\tsmirks\tweight\tapprox_count");
        for r in &rules {
            println!(
                "{}\t{}\t{}\t{:.4}\t{:.1}",
                r.template_id,
                r.name,
                r.smirks,
                r.weight,
                r.weight.exp() - 1.0
            );
        }
    }
    Ok(())
}

/// Read raw template file → Vec<(smirks, count)>, skipping comments and blank lines.
fn read_template_lines(path: &str) -> Result<Vec<(String, f64)>> {
    let content = std::fs::read_to_string(path)?;
    Ok(content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|line| {
            let mut cols = line.splitn(2, '\t');
            let smirks = cols.next()?.trim().to_string();
            let count: f64 = cols
                .next()
                .and_then(|c| c.trim().parse().ok())
                .unwrap_or(1.0);
            Some((smirks, count))
        })
        .collect())
}

fn template_stats(args: &[String]) -> Result<()> {
    let path = args
        .first()
        .map(|s| s.as_str())
        .unwrap_or("data/templates_extracted_5000.smi");
    let raw = read_template_lines(path)?;
    let total = raw.len();

    let valid_count = raw
        .iter()
        .filter(|(smirks, _)| {
            smirks
                .split(">>")
                .next()
                .and_then(|r| chematic::smarts::parse_smarts(r).ok())
                .is_some()
        })
        .count();

    let mut counts: Vec<f64> = raw.iter().map(|(_, c)| *c).collect();
    counts.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mut lens: Vec<usize> = raw.iter().map(|(s, _)| s.len()).collect();
    lens.sort_unstable();

    fn pctf(v: &[f64], p: f64) -> f64 {
        if v.is_empty() {
            return 0.0;
        }
        v[((v.len() - 1) as f64 * p) as usize]
    }
    fn pctu(v: &[usize], p: f64) -> usize {
        if v.is_empty() {
            return 0;
        }
        v[((v.len() - 1) as f64 * p) as usize]
    }

    println!("Template file: {path}");
    println!("  Total:    {total}");
    println!("  Valid:    {valid_count}");
    println!("  Invalid:  {}", total - valid_count);
    println!();
    println!("  Frequency (count):");
    println!("    min:    {:.0}", pctf(&counts, 0.0));
    println!("    p25:    {:.0}", pctf(&counts, 0.25));
    println!("    median: {:.0}", pctf(&counts, 0.5));
    println!("    p75:    {:.0}", pctf(&counts, 0.75));
    println!("    p95:    {:.0}", pctf(&counts, 0.95));
    println!("    max:    {:.0}", pctf(&counts, 1.0));
    println!(
        "    mean:   {:.1}",
        if counts.is_empty() {
            0.0
        } else {
            counts.iter().sum::<f64>() / counts.len() as f64
        }
    );
    println!();
    println!("  SMIRKS length:");
    println!("    min:    {}", pctu(&lens, 0.0));
    println!("    median: {}", pctu(&lens, 0.5));
    println!("    p95:    {}", pctu(&lens, 0.95));
    println!("    max:    {}", pctu(&lens, 1.0));
    Ok(())
}

fn template_validate(args: &[String]) -> Result<()> {
    let path = args
        .first()
        .map(|s| s.as_str())
        .unwrap_or("data/templates_extracted_5000.smi");
    let raw = read_template_lines(path)?;
    let mut valid = 0usize;
    let mut invalid: Vec<(usize, String)> = Vec::new();
    for (i, (smirks, _)) in raw.iter().enumerate() {
        if smirks
            .split(">>")
            .next()
            .and_then(|r| chematic::smarts::parse_smarts(r).ok())
            .is_some()
        {
            valid += 1;
        } else {
            invalid.push((i + 1, smirks.clone()));
        }
    }
    println!("Valid: {valid}  Invalid: {}", invalid.len());
    for (line, smirks) in &invalid {
        let short = if smirks.len() > 70 {
            &smirks[..70]
        } else {
            smirks.as_str()
        };
        println!("  line {line:5}: {short}");
    }
    Ok(())
}

fn template_dedup(args: &[String]) -> Result<()> {
    let path = args
        .first()
        .map(|s| s.as_str())
        .unwrap_or("data/templates_extracted_5000.smi");
    let raw = read_template_lines(path)?;
    let total = raw.len();
    let mut seen: std::collections::HashMap<&str, Vec<usize>> = std::collections::HashMap::new();
    for (i, (smirks, _)) in raw.iter().enumerate() {
        seen.entry(smirks.as_str()).or_default().push(i + 1);
    }
    let unique = seen.len();
    let dup_entries = total - unique;
    println!("Total: {total}  Unique: {unique}  Duplicate entries: {dup_entries}");
    if dup_entries > 0 {
        println!();
        let mut groups: Vec<(&str, &Vec<usize>)> = seen
            .iter()
            .filter(|(_, v)| v.len() > 1)
            .map(|(k, v)| (*k, v))
            .collect();
        groups.sort_by_key(|(_, v)| std::cmp::Reverse(v.len()));
        println!("Duplicate groups (up to 20):");
        for (smirks, lines) in groups.iter().take(20) {
            let short = if smirks.len() > 60 {
                &smirks[..60]
            } else {
                smirks
            };
            let line_list: Vec<String> = lines.iter().map(|n| n.to_string()).collect();
            println!(
                "  {}x  {}  (lines: {})",
                lines.len(),
                short,
                line_list.join(", ")
            );
        }
    }
    Ok(())
}

fn template_explain(args: &[String]) -> Result<()> {
    let name = args.first().map(|s| s.as_str()).unwrap_or("");
    let templates_path = args
        .windows(2)
        .find(|w| w[0] == "--templates")
        .map(|w| w[1].as_str());

    let mut all_rules = chem_env::default_rules();
    if let Some(path) = templates_path {
        all_rules.extend(chem_env::load_rules_from_file(path));
    }

    let rule = all_rules
        .iter()
        .find(|r| r.name == name)
        .or_else(|| name.parse::<usize>().ok().and_then(|i| all_rules.get(i)));

    match rule {
        Some(r) => {
            let approx_count = (r.weight.exp() - 1.0).round() as u64;
            println!("Template: {}", r.name);
            println!("  SMIRKS:  {}", r.smirks);
            println!("  Weight:  {:.4}", r.weight);
            println!("  ~Count:  {approx_count}");
            if r.required_elements != 0 {
                println!("  Elem mask: 0x{:016x}", r.required_elements);
            }
        }
        None => {
            eprintln!("Template '{name}' not found.");
            eprintln!("Tip: use --templates <path> to include extracted templates.");
        }
    }
    Ok(())
}

fn template_coverage(args: &[String]) -> Result<()> {
    let targets_path = args
        .first()
        .map(|s| s.as_str())
        .unwrap_or("data/benchmark_targets.smi");
    let templates_path = args
        .windows(2)
        .find(|w| w[0] == "--templates")
        .map(|w| w[1].as_str());
    let depth: u32 = args
        .windows(2)
        .find(|w| w[0] == "--depth")
        .and_then(|w| w[1].parse().ok())
        .unwrap_or(1);

    let targets: Vec<String> = std::fs::read_to_string(targets_path)?
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.split_whitespace().next().unwrap_or(l).to_string())
        .collect();

    let env = chem_env::ChemEnv::load("data/building_blocks.smi")
        .unwrap_or_else(|_| chem_env::ChemEnv::in_memory(DEFAULT_BUILDING_BLOCKS));

    let mut rules = chem_env::default_rules();
    if let Some(path) = templates_path {
        let extra = chem_env::load_rules_from_file(path);
        eprintln!("Loaded {} extra templates from {path}", extra.len());
        rules.extend(extra);
    }

    let config = SearchConfig {
        max_depth: depth,
        max_routes: 1,
        ..Default::default()
    };

    let mut covered = 0usize;
    let mut uncovered: Vec<String> = Vec::new();
    for target in &targets {
        let solved = search::find_routes(target, &env, &rules, &config)
            .map(|(routes, _)| !routes.is_empty())
            .unwrap_or(false);
        if solved {
            covered += 1;
        } else {
            uncovered.push(target.clone());
        }
    }

    let total = targets.len();
    println!("Templates: {}  Depth: {depth}", rules.len());
    println!("Targets:   {total}");
    println!(
        "Covered:   {covered}/{total} ({:.1}%)",
        covered as f64 / total as f64 * 100.0
    );
    if !uncovered.is_empty() {
        let show = uncovered.len().min(20);
        println!(
            "\nUncovered ({}){}:",
            uncovered.len(),
            if uncovered.len() > 20 {
                " — first 20"
            } else {
                ""
            }
        );
        for t in uncovered.iter().take(show) {
            println!("  {t}");
        }
    }
    Ok(())
}

// ── Pareto / multi-objective support ──────────────────────────────────────

#[derive(Clone, Copy)]
enum ObjField {
    Cost,
    SuccessProb,
    Steps,
    Depth,
    Confidence,
    Convergency,
    AtomEconomy,
}

#[derive(Clone, Copy)]
enum ObjDir {
    Min,
    Max,
}

impl ObjField {
    fn as_str(self) -> &'static str {
        match self {
            ObjField::Cost => "cost",
            ObjField::SuccessProb => "success_probability",
            ObjField::Steps => "steps",
            ObjField::Depth => "depth",
            ObjField::Confidence => "confidence",
            ObjField::Convergency => "convergency",
            ObjField::AtomEconomy => "atom_economy",
        }
    }
}

impl ObjDir {
    fn as_str(self) -> &'static str {
        match self {
            ObjDir::Min => "min",
            ObjDir::Max => "max",
        }
    }
}

fn parse_objectives(spec: &str) -> Vec<(ObjField, ObjDir)> {
    spec.split(',')
        .filter_map(|part| {
            let (field, dir) = part.trim().split_once(':')?;
            let f = match field.trim() {
                "cost" => ObjField::Cost,
                "success_probability" | "success" => ObjField::SuccessProb,
                "steps" => ObjField::Steps,
                "depth" => ObjField::Depth,
                "confidence" => ObjField::Confidence,
                "convergency" => ObjField::Convergency,
                "atom_economy" | "atom_economy_avg" => ObjField::AtomEconomy,
                _ => return None,
            };
            let d = match dir.trim() {
                "min" => ObjDir::Min,
                "max" => ObjDir::Max,
                _ => return None,
            };
            Some((f, d))
        })
        .collect()
}

/// Route-level atom-economy objective: `None` (not evaluable) as soon as
/// any step isn't `Normal`, rather than silently averaging over only the
/// evaluable steps and hiding the rest -- averaging a good step with an
/// omitted bad one would report a route with a real problem as if it had
/// none (Issue #79 review round 2).
fn atom_economy_objective(route: &search::Route) -> Option<f64> {
    if route.steps.is_empty()
        || route
            .steps
            .iter()
            .any(|s| s.atom_economy_status != search::AtomEconomyStatus::Normal)
    {
        return None;
    }
    let sum: f64 = route
        .steps
        .iter()
        .map(|s| s.atom_economy.expect("Normal must carry a value"))
        .sum();
    Some(sum / route.steps.len() as f64)
}

/// `None` for every field means "not evaluable"; only `AtomEconomy` can
/// currently produce one. Every other field is always `Some`.
fn obj_value(route: &search::Route, field: ObjField) -> Option<f64> {
    match field {
        ObjField::Cost => Some(route.route_cost),
        ObjField::SuccessProb => Some(route.success_probability),
        ObjField::Steps => Some(route.steps.len() as f64),
        ObjField::Depth => Some(route.depth as f64),
        ObjField::Confidence => Some(route.confidence),
        ObjField::Convergency => Some(route.convergency),
        ObjField::AtomEconomy => atom_economy_objective(route),
    }
}

/// Compares `b`'s value against `a`'s under `dir`, returning
/// `(b_is_better, b_is_worse)`. A `None` (not-evaluable) value is always
/// worse than any `Some` value on that objective, regardless of `dir` --
/// evaluable beats non-evaluable, never converted to 0 or ±infinity. Two
/// `None`s tie (neither better nor worse).
fn obj_compare(dir: ObjDir, a: Option<f64>, b: Option<f64>) -> (bool, bool) {
    match (a, b) {
        (None, None) => (false, false),
        (Some(_), None) => (false, true),
        (None, Some(_)) => (true, false),
        (Some(va), Some(vb)) => match dir {
            ObjDir::Min => (vb < va, vb > va),
            ObjDir::Max => (vb > va, vb < va),
        },
    }
}

/// Returns true if route `b` dominates route `a`
/// (b is no worse on all objectives, strictly better on at least one).
fn dominates(a: &search::Route, b: &search::Route, objs: &[(ObjField, ObjDir)]) -> bool {
    let mut all_no_worse = true;
    let mut any_better = false;
    for &(field, dir) in objs {
        let va = obj_value(a, field);
        let vb = obj_value(b, field);
        let (b_better, b_worse) = obj_compare(dir, va, vb);
        if b_worse {
            all_no_worse = false;
        }
        if b_better {
            any_better = true;
        }
    }
    all_no_worse && any_better
}

fn pareto_front_indices(routes: &[search::Route], objs: &[(ObjField, ObjDir)]) -> Vec<usize> {
    (0..routes.len())
        .filter(|&i| !(0..routes.len()).any(|j| j != i && dominates(&routes[i], &routes[j], objs)))
        .collect()
}

fn tradeoff_label(
    idx: usize,
    front: &[usize],
    routes: &[search::Route],
    objs: &[(ObjField, ObjDir)],
) -> Option<String> {
    let mut labels: Vec<&'static str> = Vec::new();
    for &(field, dir) in objs {
        let my_val = obj_value(&routes[idx], field);
        // A route whose own value on this objective isn't evaluable is
        // never the unique best on it, even on a singleton front.
        if my_val.is_none() {
            continue;
        }
        let is_unique_best = front.iter().filter(|&&j| j != idx).all(|&j| {
            let other = obj_value(&routes[j], field);
            obj_compare(dir, other, my_val).0
        });
        if is_unique_best {
            labels.push(match (field, dir) {
                (ObjField::Cost, ObjDir::Min) => "cheapest",
                (ObjField::SuccessProb, ObjDir::Max) => "most_reliable",
                (ObjField::Steps, ObjDir::Min) | (ObjField::Depth, ObjDir::Min) => "shortest",
                (ObjField::Confidence, ObjDir::Max) => "highest_confidence",
                (ObjField::Convergency, ObjDir::Max) => "most_convergent",
                (ObjField::AtomEconomy, ObjDir::Max) => "best_atom_economy",
                _ => continue,
            });
        }
    }
    if labels.is_empty() {
        None
    } else {
        Some(labels.join("_and_"))
    }
}

#[cfg(test)]
mod pareto_tests {
    use super::*;

    fn step(
        atom_economy_status: search::AtomEconomyStatus,
        atom_economy: Option<f64>,
    ) -> search::ReactionStep {
        search::ReactionStep {
            rule: "ester_cleavage".to_string(),
            template_id: "rule:ester_cleavage".to_string(),
            target: "CC(=O)Oc1ccccc1".to_string(),
            precursors: vec!["CC(=O)O".to_string(), "Oc1ccccc1".to_string()],
            conditions: None,
            atom_economy,
            atom_economy_raw_percent: atom_economy,
            atom_economy_status,
            step_confidence: 1.0,
            procedure_hint: None,
            reaction_family: None,
            metadata_source: None,
            metadata_scope: None,
            evidence: None,
        }
    }

    fn route(steps: Vec<search::ReactionStep>) -> search::Route {
        search::Route {
            steps,
            depth: 1,
            score: 1.0,
            building_blocks: vec![],
            confidence: 1.0,
            convergency: 1.0,
            success_probability: 1.0,
            route_cost: 1.0,
        }
    }

    #[test]
    fn atom_economy_objective_is_average_when_all_steps_normal() {
        let r = route(vec![
            step(search::AtomEconomyStatus::Normal, Some(90.0)),
            step(search::AtomEconomyStatus::Normal, Some(80.0)),
        ]);
        assert_eq!(atom_economy_objective(&r), Some(85.0));
    }

    #[test]
    fn atom_economy_objective_is_none_when_any_step_is_above_expected_range() {
        // One good step (90%) averaged with one route-defect step must not
        // hide the defect behind a plausible-looking mean.
        let r = route(vec![
            step(search::AtomEconomyStatus::Normal, Some(90.0)),
            step(search::AtomEconomyStatus::AboveExpectedRange, None),
        ]);
        assert_eq!(atom_economy_objective(&r), None);
    }

    #[test]
    fn atom_economy_objective_is_none_when_any_step_is_not_evaluable() {
        let r = route(vec![
            step(search::AtomEconomyStatus::Normal, Some(90.0)),
            step(search::AtomEconomyStatus::NotEvaluable, None),
        ]);
        assert_eq!(atom_economy_objective(&r), None);
    }

    #[test]
    fn atom_economy_objective_is_none_for_a_zero_step_route() {
        assert_eq!(atom_economy_objective(&route(vec![])), None);
    }

    #[test]
    fn dominates_prefers_evaluable_atom_economy_over_not_evaluable() {
        // Same on every other objective; b evaluates cleanly, a doesn't.
        let a = route(vec![step(search::AtomEconomyStatus::NotEvaluable, None)]);
        let b = route(vec![step(search::AtomEconomyStatus::Normal, Some(90.0))]);
        let objs = vec![(ObjField::AtomEconomy, ObjDir::Max)];
        assert!(
            dominates(&a, &b, &objs),
            "b (evaluable) must dominate a (not evaluable)"
        );
        assert!(!dominates(&b, &a, &objs));
    }

    #[test]
    fn dominates_ties_when_both_atom_economy_not_evaluable() {
        let a = route(vec![step(search::AtomEconomyStatus::NotEvaluable, None)]);
        let b = route(vec![step(
            search::AtomEconomyStatus::AboveExpectedRange,
            None,
        )]);
        let objs = vec![(ObjField::AtomEconomy, ObjDir::Max)];
        assert!(!dominates(&a, &b, &objs));
        assert!(!dominates(&b, &a, &objs));
    }

    #[test]
    fn tradeoff_label_never_tags_a_not_evaluable_route_best_atom_economy_even_alone_on_front() {
        let routes = vec![route(vec![step(
            search::AtomEconomyStatus::NotEvaluable,
            None,
        )])];
        let objs = vec![(ObjField::AtomEconomy, ObjDir::Max)];
        let front = pareto_front_indices(&routes, &objs);
        assert_eq!(front, vec![0]);
        assert_eq!(tradeoff_label(0, &front, &routes, &objs), None);
    }

    #[test]
    fn tradeoff_label_tags_the_evaluable_route_best_atom_economy_against_a_not_evaluable_rival() {
        let routes = vec![
            route(vec![step(search::AtomEconomyStatus::NotEvaluable, None)]),
            route(vec![step(search::AtomEconomyStatus::Normal, Some(90.0))]),
        ];
        let objs = vec![(ObjField::AtomEconomy, ObjDir::Max)];
        let front = pareto_front_indices(&routes, &objs);
        assert_eq!(
            front,
            vec![1],
            "the not-evaluable route must be dominated off the front"
        );
        assert_eq!(
            tradeoff_label(1, &front, &routes, &objs),
            Some("best_atom_economy".to_string())
        );
    }
}

// ── Stock CSV support ──────────────────────────────────────────────────────

struct StockEntry {
    smiles: String,
    name: Option<String>,
    vendor: Option<String>,
    price_jpy: Option<f64>,
    hazard: Option<String>,
    available: bool,
}

/// Parse a stock CSV file.
/// Header (first non-comment line) and comment lines starting with `#` are skipped.
/// Columns: smiles, name, vendor, price_jpy, amount, hazard, available
fn load_stock_csv(path: &str) -> Vec<StockEntry> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut first = true;
    content
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            // skip header row
            if first {
                let is_header = l.to_ascii_lowercase().starts_with("smiles");
                first = false; // won't trigger again; closure captures mut ref
                if is_header {
                    return None;
                }
            }
            let cols: Vec<&str> = l.splitn(8, ',').collect();
            let smiles = cols.first()?.trim().to_string();
            if smiles.is_empty() {
                return None;
            }
            Some(StockEntry {
                smiles,
                name: cols
                    .get(1)
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                vendor: cols
                    .get(2)
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                price_jpy: cols.get(3).and_then(|s| s.trim().parse::<f64>().ok()),
                hazard: cols
                    .get(5)
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
                available: cols
                    .get(6)
                    .map(|s| s.trim().eq_ignore_ascii_case("true"))
                    .unwrap_or(true),
            })
        })
        .collect()
}

fn run_stock(args: &[String]) -> Result<()> {
    let cmd = args.first().map(|s| s.as_str()).unwrap_or("help");
    match cmd {
        "stats" => {
            let path = args.get(1).map(|s| s.as_str()).unwrap_or("data/stock.csv");
            let entries = load_stock_csv(path);
            if entries.is_empty() {
                println!("No entries found in {path}");
                return Ok(());
            }
            let available = entries.iter().filter(|e| e.available).count();
            let priced: Vec<f64> = entries.iter().filter_map(|e| e.price_jpy).collect();
            let (pmin, pmax) = if priced.is_empty() {
                ("—".to_string(), "—".to_string())
            } else {
                let mn = priced.iter().cloned().fold(f64::INFINITY, f64::min);
                let mx = priced.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
                (format!("{mn:.0}"), format!("{mx:.0}"))
            };
            let mut hazards: Vec<&str> =
                entries.iter().filter_map(|e| e.hazard.as_deref()).collect();
            hazards.sort_unstable();
            hazards.dedup();
            println!("Stock: {path}");
            println!("  Entries   : {}", entries.len());
            println!("  Available : {available}");
            println!("  Priced    : {} / {}", priced.len(), entries.len());
            println!("  Price JPY : {pmin} – {pmax}");
            println!(
                "  Hazards   : {}",
                if hazards.is_empty() {
                    "none".to_string()
                } else {
                    hazards.join(", ")
                }
            );
            let mut vendors: Vec<&str> =
                entries.iter().filter_map(|e| e.vendor.as_deref()).collect();
            vendors.sort_unstable();
            vendors.dedup();
            if !vendors.is_empty() {
                println!("  Vendors   : {}", vendors.join(", "));
            }
        }
        "validate" => {
            let path = args.get(1).map(|s| s.as_str()).unwrap_or("data/stock.csv");
            let entries = load_stock_csv(path);
            let mut valid = 0usize;
            let mut invalid: Vec<String> = Vec::new();
            for e in &entries {
                if chem_env::mol_from_smiles(&e.smiles).is_ok() {
                    valid += 1;
                } else {
                    let label = e.name.as_deref().unwrap_or("?");
                    invalid.push(format!("{} ({})", e.smiles, label));
                }
            }
            println!("Valid: {valid}  Invalid: {}", invalid.len());
            for s in &invalid {
                println!("  INVALID SMILES: {s}");
            }
        }
        "coverage" => {
            let targets_path = args.get(1).map(|s| s.as_str()).unwrap_or("targets.smi");
            let stock_path = args.get(2).map(|s| s.as_str()).unwrap_or("data/stock.csv");
            let entries = load_stock_csv(stock_path);
            let stock_set: std::collections::HashSet<&str> =
                entries.iter().map(|e| e.smiles.as_str()).collect();
            let targets: Vec<String> = std::fs::read_to_string(targets_path)
                .unwrap_or_default()
                .lines()
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(|l| l.split_whitespace().next().unwrap_or(l).to_string())
                .collect();
            let in_stock: Vec<&str> = targets
                .iter()
                .filter(|t| stock_set.contains(t.as_str()))
                .map(|t| t.as_str())
                .collect();
            println!(
                "Targets: {}  In stock: {}  Not in stock: {}",
                targets.len(),
                in_stock.len(),
                targets.len() - in_stock.len()
            );
        }
        _ => {
            println!("Usage: renkin stock <stats|validate|coverage> [args...]");
            println!("  stats <file.csv>                  — summary statistics");
            println!("  validate <file.csv>               — check SMILES validity");
            println!("  coverage <targets.smi> <file.csv> — check which targets are in stock");
        }
    }
    Ok(())
}

fn load_prices(path: &str) -> std::collections::HashMap<String, f64> {
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
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod evidence_cli_tests {
    use super::*;

    fn write_temp(name: &str, content: &str) -> String {
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, content).unwrap();
        path.to_str().unwrap().to_string()
    }

    fn read_output_rows(path: &str) -> Vec<serde_json::Value> {
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn evidence_match_preserves_jsonl_input_order() {
        let input = write_temp(
            "evidence_match_order_input.jsonl",
            concat!(
                r#"{"record_id": "zzz", "target_smiles": "CCO", "precursor_smiles": []}"#,
                "\n",
                r#"{"record_id": "aaa", "target_smiles": "CCO", "precursor_smiles": []}"#,
                "\n",
                r#"{"record_id": "mmm", "target_smiles": "CCO", "precursor_smiles": []}"#,
                "\n",
            ),
        );
        let output = std::env::temp_dir()
            .join("evidence_match_order_output.jsonl")
            .to_str()
            .unwrap()
            .to_string();

        let args = vec![
            "--input".to_string(),
            input.clone(),
            "--output".to_string(),
            output.clone(),
        ];
        evidence_match(&args).unwrap();

        let rows = read_output_rows(&output);
        let ids: Vec<&str> = rows
            .iter()
            .map(|r| r["record_id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["zzz", "aaa", "mmm"]);

        std::fs::remove_file(&input).ok();
        std::fs::remove_file(&output).ok();
    }

    #[test]
    fn evidence_match_malformed_json_line_is_hard_error_with_line_number() {
        let input = write_temp(
            "evidence_match_malformed_json_input.jsonl",
            concat!(
                r#"{"record_id": "ok-1", "target_smiles": "CCO", "precursor_smiles": []}"#,
                "\n",
                "{ this is not valid json",
                "\n",
            ),
        );
        let output = std::env::temp_dir()
            .join("evidence_match_malformed_json_output.jsonl")
            .to_str()
            .unwrap()
            .to_string();

        let args = vec![
            "--input".to_string(),
            input.clone(),
            "--output".to_string(),
            output.clone(),
        ];
        let err = evidence_match(&args).unwrap_err();
        let message = format!("{err:#}");
        assert!(
            message.contains(":2:"),
            "error should cite line 2: {message}"
        );
        assert!(!std::path::Path::new(&output).exists());

        std::fs::remove_file(&input).ok();
    }

    #[test]
    fn evidence_match_malformed_smiles_yields_invalid_input_without_aborting() {
        let input = write_temp(
            "evidence_match_malformed_smiles_input.jsonl",
            concat!(
                r#"{"record_id": "good", "target_smiles": "CC(=O)OCC", "precursor_smiles": ["CC(=O)O", "CCO"]}"#,
                "\n",
                r#"{"record_id": "bad-smiles", "target_smiles": "not(a smiles", "precursor_smiles": ["CCO"]}"#,
                "\n",
            ),
        );
        let output = std::env::temp_dir()
            .join("evidence_match_malformed_smiles_output.jsonl")
            .to_str()
            .unwrap()
            .to_string();

        let args = vec![
            "--input".to_string(),
            input.clone(),
            "--output".to_string(),
            output.clone(),
        ];
        evidence_match(&args).unwrap();

        let rows = read_output_rows(&output);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["record_id"], "good");
        assert_eq!(rows[0]["status"], "unique");
        assert_eq!(rows[1]["record_id"], "bad-smiles");
        assert_eq!(rows[1]["status"], "invalid_input");

        std::fs::remove_file(&input).ok();
        std::fs::remove_file(&output).ok();
    }

    #[test]
    fn evidence_validate_sidecar_ok_on_valid_file() {
        let path = write_temp(
            "evidence_validate_sidecar_valid.json",
            r#"{"schema_version": 2, "templates": {}}"#,
        );
        let args = vec!["--metadata".to_string(), path.clone()];
        assert!(evidence_validate_sidecar(&args).is_ok());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn evidence_validate_sidecar_errs_on_invalid_file() {
        let path = write_temp(
            "evidence_validate_sidecar_invalid.json",
            r#"{"schema_version": 99, "templates": {}}"#,
        );
        let args = vec!["--metadata".to_string(), path.clone()];
        assert!(evidence_validate_sidecar(&args).is_err());
        std::fs::remove_file(&path).ok();
    }
}
