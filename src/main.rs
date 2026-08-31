#![forbid(unsafe_code)]

use renkin::DEFAULT_BUILDING_BLOCKS;
use renkin::bridge;
use renkin::chem_env;
use renkin::display;
use renkin::evidence_match;
use renkin::ring_context;
use renkin::search::{self, SearchConfig};
use renkin::stock_import;
use renkin::vendor_stock;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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

/// Issue #99: which loaded templates are `Unsupported` for concrete
/// application (their `[#N]` hash-atoms have no usable concrete-element
/// reading, so they can never produce a route), grouped by reason. Pure
/// classification, no I/O -- see `report_hash_atom_unsupported` for the
/// load-time stderr summary this backs.
fn count_hash_atom_unsupported(
    rules: &[chem_env::RetroRule],
) -> std::collections::BTreeMap<&'static str, usize> {
    let mut by_reason: std::collections::BTreeMap<&'static str, usize> =
        std::collections::BTreeMap::new();
    for rule in rules {
        if let chem_env::ConcreteApplicationStatus::Unsupported { reason } =
            chem_env::concrete_application_status(&rule.smirks)
        {
            let key = match reason {
                chem_env::HashAtomUnsupportedReason::UnhandledSyntax => "unhandled_syntax",
                chem_env::HashAtomUnsupportedReason::InconsistentElement => "inconsistent_element",
                chem_env::HashAtomUnsupportedReason::VariantLimitExceeded { .. } => {
                    "variant_limit_exceeded"
                }
                chem_env::HashAtomUnsupportedReason::NoValidVariant => "no_valid_variant",
            };
            *by_reason.entry(key).or_insert(0) += 1;
        }
    }
    by_reason
}

/// Issue #99: a normal search run had no in-band signal that some loaded
/// templates are unsupported for concrete application — the only way to
/// see this was `examples/hashatom_corpus_stats.rs`, run offline against a
/// template file directly. Silent when there's nothing to report (both
/// checked-in corpora currently have zero unsupported templates); prints a
/// reason-broken-down summary otherwise, right after the existing
/// unconditional "Loaded N templates" line — load-time, not requiring a
/// search to actually run one to completion.
fn report_hash_atom_unsupported(rules: &[chem_env::RetroRule]) {
    let by_reason = count_hash_atom_unsupported(rules);
    let total: usize = by_reason.values().sum();
    if total == 0 {
        return;
    }
    let breakdown = by_reason
        .iter()
        .map(|(reason, count)| format!("{reason}={count}"))
        .collect::<Vec<_>>()
        .join(", ");
    eprintln!(
        "  {total} of {} loaded templates are unsupported for concrete application \
         (will never produce a route): {breakdown}",
        rules.len()
    );
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
    if args.get(1).map(|s| s.as_str()) == Some("doctor") {
        return run_doctor(&args[2..]);
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
    let mut spectator_bond_policy_arg: Option<String> = None;
    let mut element_accounting_policy_arg: Option<String> = None;
    let mut beam_diversity_policy_arg: Option<String> = None;
    let mut beam_diversity_slots_arg: Option<String> = None;
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
            "--spectator-bond-policy" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    bail!("--spectator-bond-policy requires a value (off|diagnostics-only|gated)");
                };
                spectator_bond_policy_arg = Some(v.clone());
            }
            "--element-accounting-policy" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    bail!(
                        "--element-accounting-policy requires a value (off|diagnostics-only|gated)"
                    );
                };
                element_accounting_policy_arg = Some(v.clone());
            }
            "--beam-diversity-policy" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    bail!("--beam-diversity-policy requires a value (off|diagnostics-only|active)");
                };
                beam_diversity_policy_arg = Some(v.clone());
            }
            "--beam-diversity-slots" => {
                i += 1;
                let Some(v) = args.get(i) else {
                    bail!("--beam-diversity-slots requires a <N> value");
                };
                beam_diversity_slots_arg = Some(v.clone());
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
             --spectator-bond-policy <policy>  off (default) | diagnostics-only | gated -- \
             detects a real target bond a retro-rule's own SMIRKS never declares broken but \
             chematic silently drops from precursors (docs/design/spectator-bond-fail-closed-\
             gating-v0.md). diagnostics-only records findings in --search-diagnostics output \
             only; gated additionally excludes the specific candidate a confident finding \
             applies to (v1: rules with no '#' in their SMIRKS only -- others stay \
             diagnostics-only regardless of this flag)\n  \
             --element-accounting-policy <policy>  off (default) | diagnostics-only | gated -- \
             detects a candidate whose target needs more of some heavy element than its \
             precursors collectively supply (docs/design/candidate-time-element-accounting-\
             gate-v0.md). diagnostics-only records the verdict in --search-diagnostics output \
             only; gated additionally excludes the specific candidate\n  \
             --beam-diversity-policy <policy>  off (default) | diagnostics-only | active -- \
             reserves --beam-diversity-slots beam slots for template-family diversity instead \
             of pure score, so a lower-scoring candidate from an underrepresented rule isn't \
             fully crowded out by many higher-scoring same-rule siblings \
             (docs/design/diversity-reserved-beam-v0.md). diagnostics-only records what active \
             would additionally keep without changing selection; active actually reserves the \
             slots\n  \
             --beam-diversity-slots <N>  Beam slots reserved under diagnostics-only/active \
             (default 0, i.e. no reservation even if the policy is opted into); ignored under \
             off\n  \
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
        report_hash_atom_unsupported(&extra);
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

    let spectator_bond_policy = match spectator_bond_policy_arg.as_deref() {
        None | Some("off") => renkin::spectator_bond::SpectatorBondPolicy::Off,
        Some("diagnostics-only") => renkin::spectator_bond::SpectatorBondPolicy::DiagnosticsOnly,
        Some("gated") => renkin::spectator_bond::SpectatorBondPolicy::Gated,
        Some(other) => {
            eprintln!(
                "error: invalid --spectator-bond-policy '{other}' \
                 (expected off|diagnostics-only|gated)"
            );
            std::process::exit(1);
        }
    };

    let element_accounting_policy = match element_accounting_policy_arg.as_deref() {
        None | Some("off") => search::ElementAccountingGatePolicy::Off,
        Some("diagnostics-only") => search::ElementAccountingGatePolicy::DiagnosticsOnly,
        Some("gated") => search::ElementAccountingGatePolicy::Gated,
        Some(other) => {
            eprintln!(
                "error: invalid --element-accounting-policy '{other}' \
                 (expected off|diagnostics-only|gated)"
            );
            std::process::exit(1);
        }
    };

    let beam_diversity_policy = match beam_diversity_policy_arg.as_deref() {
        None | Some("off") => search::BeamDiversityPolicy::Off,
        Some("diagnostics-only") => search::BeamDiversityPolicy::DiagnosticsOnly,
        Some("active") => search::BeamDiversityPolicy::Active,
        Some(other) => {
            eprintln!(
                "error: invalid --beam-diversity-policy '{other}' \
                 (expected off|diagnostics-only|active)"
            );
            std::process::exit(1);
        }
    };
    let beam_diversity_slots: usize = match beam_diversity_slots_arg.as_deref() {
        None => 0,
        Some(v) => match v.parse() {
            Ok(n) => n,
            Err(_) => {
                eprintln!(
                    "error: --beam-diversity-slots '{v}' is not a valid non-negative integer"
                );
                std::process::exit(1);
            }
        },
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
        spectator_bond_policy,
        element_accounting_policy,
        beam_diversity_policy,
        beam_diversity_slots,
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
    /// Reject routes containing any of these canonical building-block leaves.
    avoid_building_blocks: Option<Vec<String>>,
    max_steps: Option<usize>,
    /// Keep routes whose computed route cost is at or below this limit.
    /// The unit follows `route_cost`: price-map units when `--bb-prices` is
    /// supplied, otherwise the existing SA-score-based estimate.
    max_route_cost: Option<f64>,
    max_depth: Option<u32>,
    min_confidence: Option<f64>,
    min_success_probability: Option<f64>,
    /// Require at least one step from one of these named reaction families.
    require_reaction_families: Option<Vec<String>>,
    /// Reject routes containing a step from one of these named reaction families.
    avoid_reaction_families: Option<Vec<String>>,
    prefer_reaction_families: Option<Vec<String>>,
    objectives: Option<String>,
}

fn apply_constraints(routes: &mut Vec<search::Route>, c: &ConstraintSpec) {
    if let Some(n) = c.max_steps {
        routes.retain(|r| r.steps.len() <= n);
    }
    if let Some(ref blocked) = c.avoid_building_blocks {
        routes.retain(|r| {
            !r.building_blocks
                .iter()
                .any(|bb| blocked.iter().any(|candidate| candidate == bb))
        });
    }
    if let Some(max_cost) = c.max_route_cost {
        routes.retain(|r| r.route_cost <= max_cost);
    }
    if let Some(ref fams) = c.require_reaction_families {
        routes.retain(|r| {
            r.steps.iter().any(|s| {
                s.reaction_family
                    .as_deref()
                    .is_some_and(|f| fams.iter().any(|required| required == f))
            })
        });
    }
    if let Some(ref fams) = c.avoid_reaction_families {
        routes.retain(|r| {
            !r.steps.iter().any(|s| {
                s.reaction_family
                    .as_deref()
                    .is_some_and(|f| fams.iter().any(|avoided| avoided == f))
            })
        });
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

/// Whether bare boolean flag `name` (e.g. `--force`) appears anywhere in `args`.
fn flag_present(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

/// Lexical same-path check via `std::path::absolute` (pure path
/// normalization, no filesystem access -- works even when one side
/// doesn't exist yet, unlike `std::fs::canonicalize`). Not symlink-aware;
/// just enough to catch the common "--input and --output point at
/// literally the same path" mistake before any file gets truncated.
fn same_path(a: &str, b: &str) -> bool {
    match (std::path::absolute(a), std::path::absolute(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Writes `content_a`/`content_b` to `path_a`/`path_b` with best-effort
/// all-or-nothing semantics: both are first written to a `.stock-import-tmp`
/// sibling file in the SAME directory as their real destination (so the
/// final rename is same-filesystem, hence atomic) and fsynced -- only once
/// both temp writes succeed does either destination get touched. If the
/// first rename succeeds but the second fails, and the first destination
/// did not exist before this call, it's removed again to restore the
/// pre-call state.
///
/// ponytail: if the first destination DID already exist (an allowed
/// `--force` overwrite) and the second rename then fails, the original
/// bytes were never backed up and can't be restored -- that narrow window
/// is a known, documented ceiling, not silently swallowed (the returned
/// error says exactly which artifact is now out of sync). Add a real
/// backup-and-restore only if a user actually hits this.
fn write_two_artifacts_atomically(
    path_a: &str,
    content_a: &[u8],
    path_b: &str,
    content_b: &[u8],
) -> Result<()> {
    fn write_and_sync(path: &str, content: &[u8]) -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::File::create(path)?;
        f.write_all(content)?;
        f.sync_all()
    }

    let a_existed = std::path::Path::new(path_a).exists();

    let tmp_a = format!("{path_a}.stock-import-tmp");
    let tmp_b = format!("{path_b}.stock-import-tmp");

    write_and_sync(&tmp_a, content_a)
        .with_context(|| format!("failed to write temp file for {path_a:?}"))?;
    write_and_sync(&tmp_b, content_b)
        .with_context(|| format!("failed to write temp file for {path_b:?}"))?;

    std::fs::rename(&tmp_a, path_a)
        .with_context(|| format!("failed to move temp file into place at {path_a:?}"))?;

    if let Err(e) = std::fs::rename(&tmp_b, path_b) {
        std::fs::remove_file(&tmp_b).ok();
        let recovery_note = if a_existed {
            format!(
                "{path_a:?} was pre-existing and has already been overwritten with new \
                 content that now has no matching manifest at {path_b:?} -- re-run the import \
                 to fix"
            )
        } else {
            std::fs::remove_file(path_a).ok();
            format!("{path_a:?} has been removed again to avoid leaving it without a manifest")
        };
        return Err(e).with_context(|| {
            format!("failed to move temp file into place at {path_b:?} -- {recovery_note}")
        });
    }

    Ok(())
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

/// `renkin audit-route <PATH> [--format auto|renkin|aizynthfinder|syntheseus|synplanner] [--stock <PATH>]
/// [--private-stock <CSV|TSV>] [--stock-policy <JSON>] [--policy informational|standard|strict]
/// [--chemical-review] [--interchange] [--output human|json]` --
/// audits every route in a RENKIN `--format json`
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
        .context("renkin audit-route: <PATH> is required (usage: renkin audit-route <PATH> [--format auto|renkin|aizynthfinder|syntheseus|synplanner] [--stock <PATH>] [--private-stock <CSV|TSV>] [--stock-policy <JSON>] [--policy informational|standard|strict] [--chemical-review] [--interchange] [--output human|json])")?;
    let format = flag_value(args, "--format").unwrap_or("auto");
    if ![
        "auto",
        "renkin",
        "aizynthfinder",
        "syntheseus",
        "synplanner",
    ]
    .contains(&format)
    {
        bail!(
            "renkin audit-route: unsupported --format {format:?} (only auto|renkin|aizynthfinder|syntheseus|synplanner supported)"
        );
    }
    let output_format = flag_value(args, "--output").unwrap_or("human");
    if output_format != "human" && output_format != "json" {
        bail!(
            "renkin audit-route: unsupported --output {output_format:?} (only human|json supported)"
        );
    }
    let policy_str = flag_value(args, "--policy").unwrap_or("standard");
    let policy: bridge::AuditPolicy = policy_str.parse().map_err(|_| {
        anyhow::anyhow!(
            "renkin audit-route: unsupported --policy {policy_str:?} (only informational|standard|strict supported)"
        )
    })?;

    let content = read_maybe_gzip(&path)?;
    let stock = flag_value(args, "--stock")
        .map(load_audit_stock)
        .transpose()?;
    let rules = chem_env::default_rules();

    let mut out = bridge::build_audit_route_report_with_options(
        &content,
        format,
        stock.as_ref(),
        &rules,
        policy,
        args.iter().any(|a| a == "--chemical-review"),
    )
    .with_context(|| format!("{path}: audit input rejected"))?;

    let private_stock_path = flag_value(args, "--private-stock");
    let stock_policy_path = flag_value(args, "--stock-policy");
    match (private_stock_path, stock_policy_path) {
        (Some(vendor_path), Some(policy_path)) => {
            let vendor_content = std::fs::read_to_string(vendor_path)
                .with_context(|| format!("failed to read --private-stock {vendor_path}"))?;
            let records = vendor_stock::import_vendor_table(&vendor_content, None)
                .with_context(|| format!("failed to parse --private-stock {vendor_path}"))?;
            let index = vendor_stock::VendorStockIndex::from_records(records)
                .with_context(|| format!("failed to index --private-stock {vendor_path}"))?;
            let policy_content = std::fs::read_to_string(policy_path)
                .with_context(|| format!("failed to read --stock-policy {policy_path}"))?;
            let policy: bridge::PrivateStockPolicy = serde_json::from_str(&policy_content)
                .with_context(|| format!("failed to parse --stock-policy {policy_path}"))?;
            out.attach_private_stock(&index, &policy)?;
        }
        (None, None) => {}
        _ => bail!(
            "renkin audit-route: --private-stock and --stock-policy must be provided together"
        ),
    }

    if args.iter().any(|a| a == "--interchange") {
        out.attach_interchange();
    }

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
    fn max_route_cost_constraint_keeps_boundary_and_filters_expensive_routes() {
        let mut affordable = route(vec![]);
        affordable.route_cost = 10.0;
        let mut expensive = route(vec![]);
        expensive.route_cost = 10.01;
        let mut routes = vec![expensive, affordable];

        apply_constraints(
            &mut routes,
            &ConstraintSpec {
                max_route_cost: Some(10.0),
                ..Default::default()
            },
        );

        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].route_cost, 10.0);
    }

    #[test]
    fn required_reaction_family_constraint_filters_routes_without_a_matching_step() {
        let mut matching = route(vec![step(search::AtomEconomyStatus::Normal, Some(90.0))]);
        matching.steps[0].reaction_family = Some("suzuki_coupling".to_string());
        let non_matching = route(vec![step(search::AtomEconomyStatus::Normal, Some(90.0))]);
        let mut routes = vec![non_matching, matching];

        apply_constraints(
            &mut routes,
            &ConstraintSpec {
                require_reaction_families: Some(vec!["suzuki_coupling".to_string()]),
                ..Default::default()
            },
        );

        assert_eq!(routes.len(), 1);
        assert_eq!(
            routes[0].steps[0].reaction_family.as_deref(),
            Some("suzuki_coupling")
        );
    }

    #[test]
    fn avoided_reaction_family_constraint_filters_routes_with_a_matching_step() {
        let mut avoided = route(vec![step(search::AtomEconomyStatus::Normal, Some(90.0))]);
        avoided.steps[0].reaction_family = Some("heck_reaction".to_string());
        let allowed = route(vec![step(search::AtomEconomyStatus::Normal, Some(90.0))]);
        let mut routes = vec![avoided, allowed];

        apply_constraints(
            &mut routes,
            &ConstraintSpec {
                avoid_reaction_families: Some(vec!["heck_reaction".to_string()]),
                ..Default::default()
            },
        );

        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].steps[0].reaction_family, None);
    }

    #[test]
    fn avoided_building_block_constraint_filters_routes_with_a_matching_leaf() {
        let mut blocked = route(vec![]);
        blocked.building_blocks = vec!["Brc1ccccc1".to_string()];
        let mut allowed = route(vec![]);
        allowed.building_blocks = vec!["c1ccccc1".to_string()];
        let mut routes = vec![blocked, allowed];

        apply_constraints(
            &mut routes,
            &ConstraintSpec {
                avoid_building_blocks: Some(vec!["Brc1ccccc1".to_string()]),
                ..Default::default()
            },
        );

        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].building_blocks[0], "c1ccccc1");
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
        "import" => stock_import_cli(&args[1..])?,
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
        "vendor-index" => {
            let path = args
                .get(1)
                .ok_or_else(|| anyhow::anyhow!("vendor-index requires a CSV/TSV path"))?;
            let content = std::fs::read_to_string(path)
                .with_context(|| format!("failed to read vendor table: {path}"))?;
            let records = vendor_stock::import_vendor_table(&content, None)?;
            let index = vendor_stock::VendorStockIndex::from_records(records)?;
            println!(
                "{{\"schema_version\":{},\"records\":{},\"unique_inchikeys\":{}}}",
                vendor_stock::VENDOR_STOCK_SCHEMA_VERSION,
                index.records().len(),
                index.unique_inchi_keys()
            );
        }
        "vendor-match" => {
            let path = args
                .get(1)
                .ok_or_else(|| anyhow::anyhow!("vendor-match requires a CSV/TSV path"))?;
            let smiles = args
                .get(2)
                .ok_or_else(|| anyhow::anyhow!("vendor-match requires a query SMILES"))?;
            let max_mode = parse_vendor_match_mode(args.get(3).map(String::as_str))?;
            let content = std::fs::read_to_string(path)
                .with_context(|| format!("failed to read vendor table: {path}"))?;
            let index = vendor_stock::VendorStockIndex::from_records(
                vendor_stock::import_vendor_table(&content, None)?,
            )?;
            match index.lookup(smiles, max_mode)? {
                Some(found) => println!("{}", serde_json::to_string(&found)?),
                None => println!("null"),
            }
        }
        _ => {
            println!(
                "Usage: renkin stock <import|stats|validate|coverage|vendor-index|vendor-match> [args...]"
            );
            println!("  import --input <in.smi> --output <out.smi> --manifest <out.manifest.json>");
            println!(
                "         --source-label <label> [--source-revision <rev>] [--license <license>]"
            );
            println!(
                "         [--fail-on-rejection] [--force] — deterministic .smi import + provenance manifest"
            );
            println!("  stats <file.csv>                  — summary statistics");
            println!("  validate <file.csv>               — check SMILES validity");
            println!("  coverage <targets.smi> <file.csv> — check which targets are in stock");
            println!("  vendor-index <file.csv|tsv>       — build and summarize a vendor index");
            println!(
                "  vendor-match <file> <SMILES> [exact|parent-ignoring-salts|stereo-ignored|tautomer-related]"
            );
        }
    }
    Ok(())
}

fn parse_vendor_match_mode(value: Option<&str>) -> Result<vendor_stock::MatchMode> {
    match value.unwrap_or("exact") {
        "exact" => Ok(vendor_stock::MatchMode::Exact),
        "parent-ignoring-salts" => Ok(vendor_stock::MatchMode::ParentIgnoringSalts),
        "stereo-ignored" => Ok(vendor_stock::MatchMode::StereoIgnored),
        "tautomer-related" => Ok(vendor_stock::MatchMode::TautomerRelated),
        other => anyhow::bail!(
            "unknown vendor match mode '{other}' (expected exact, parent-ignoring-salts, stereo-ignored, or tautomer-related)"
        ),
    }
}

/// `renkin stock import --input <path> --output <path> --manifest <path>
/// --source-label <label> [--source-revision <rev>] [--license <license>]
/// [--fail-on-rejection] [--force]` -- CLI wrapper around
/// `stock_import::import_stock_from_path`, the sole source of the actual
/// canonicalize/dedup/manifest logic (never reimplemented here). Writes
/// both the output `.smi` and the manifest JSON to temp files first, then
/// renames both into place (`write_two_artifacts_atomically`), so a crash
/// or write failure never leaves a stale/partial destination pair.
/// Rejected/duplicate rows never abort the import by themselves -- only
/// `--fail-on-rejection` turns a nonzero `rejected_rows` into a nonzero
/// exit code, and even then the artifacts are still written first (a
/// rejected-but-imported run leaves useful output on disk, not nothing).
/// stdout carries only the machine-readable summary JSON; progress
/// warnings go to stderr.
fn stock_import_cli(args: &[String]) -> Result<()> {
    let input_path =
        flag_value(args, "--input").context("renkin stock import: --input <path> is required")?;
    let output_path =
        flag_value(args, "--output").context("renkin stock import: --output <path> is required")?;
    let manifest_path = flag_value(args, "--manifest")
        .context("renkin stock import: --manifest <path> is required")?;
    let source_label = flag_value(args, "--source-label")
        .context("renkin stock import: --source-label <label> is required")?;
    let source_revision = flag_value(args, "--source-revision").map(str::to_string);
    let license = flag_value(args, "--license").map(str::to_string);
    let fail_on_rejection = flag_present(args, "--fail-on-rejection");
    let force = flag_present(args, "--force");

    if same_path(input_path, output_path) {
        bail!(
            "renkin stock import: --input and --output must not be the same path ({input_path:?})"
        );
    }
    if same_path(input_path, manifest_path) {
        bail!(
            "renkin stock import: --input and --manifest must not be the same path ({input_path:?})"
        );
    }
    if same_path(output_path, manifest_path) {
        bail!(
            "renkin stock import: --output and --manifest must not be the same path ({output_path:?})"
        );
    }
    if !force && std::path::Path::new(output_path).exists() {
        bail!(
            "renkin stock import: --output {output_path:?} already exists (use --force to overwrite)"
        );
    }
    if !force && std::path::Path::new(manifest_path).exists() {
        bail!(
            "renkin stock import: --manifest {manifest_path:?} already exists (use --force to overwrite)"
        );
    }

    let options = stock_import::StockImportOptions {
        source_label: source_label.to_string(),
        source_revision,
        license,
    };
    let (accepted, manifest) =
        stock_import::import_stock_from_path(std::path::Path::new(input_path), &options)
            .with_context(|| {
                format!("renkin stock import: failed to import --input {input_path:?}")
            })?;

    let output_bytes = stock_import::render_output(&accepted);
    let manifest_json = serde_json::to_string_pretty(&manifest)?;

    write_two_artifacts_atomically(
        output_path,
        &output_bytes,
        manifest_path,
        manifest_json.as_bytes(),
    )?;

    if manifest.rejected_rows > 0 {
        eprintln!(
            "warning: {} of {} input rows were rejected ({:?}) -- see {manifest_path:?}",
            manifest.rejected_rows, manifest.input_rows, manifest.rejection_reasons
        );
    }
    if manifest.duplicate_rows > 0 {
        eprintln!(
            "warning: {} input rows were in-file duplicates (kept first occurrence only) -- \
             see {manifest_path:?}",
            manifest.duplicate_rows
        );
    }

    #[derive(Serialize)]
    struct StockImportCliSummary<'a> {
        output_path: &'a str,
        manifest_path: &'a str,
        manifest: &'a stock_import::StockManifest,
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&StockImportCliSummary {
            output_path,
            manifest_path,
            manifest: &manifest,
        })?
    );

    if fail_on_rejection && manifest.rejected_rows > 0 {
        bail!(
            "renkin stock import: --fail-on-rejection: {} rows were rejected (artifacts were \
             still written to {output_path:?} / {manifest_path:?})",
            manifest.rejected_rows
        );
    }

    Ok(())
}

/// Severity of one `renkin doctor stock` check. `Fail` is the only
/// severity that turns the CLI's exit code non-zero; `Warn` is reported
/// but does not fail the run (e.g. an importer-version difference or
/// missing optional provenance is worth flagging, not blocking).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum DoctorSeverity {
    Pass,
    Warn,
    Fail,
}

impl DoctorSeverity {
    fn worse(self, other: Self) -> Self {
        use DoctorSeverity::*;
        match (self, other) {
            (Fail, _) | (_, Fail) => Fail,
            (Warn, _) | (_, Warn) => Warn,
            _ => Pass,
        }
    }

    fn label(self) -> &'static str {
        match self {
            DoctorSeverity::Pass => "PASS",
            DoctorSeverity::Warn => "WARN",
            DoctorSeverity::Fail => "FAIL",
        }
    }
}

#[derive(Debug, Serialize)]
struct StockDoctorCheck {
    name: &'static str,
    severity: DoctorSeverity,
    message: String,
}

#[derive(Debug, Serialize)]
struct StockDoctorReport {
    stock_path: String,
    manifest_path: String,
    input_path: Option<String>,
    overall: DoctorSeverity,
    checks: Vec<StockDoctorCheck>,
}

/// Pure report-building logic for `renkin doctor stock`, kept separate
/// from the CLI's exit-code side effects (`doctor_stock` below) so it can
/// be unit-tested directly without spawning a subprocess. Returns `Err`
/// only for invocation-level problems -- missing/unreadable required
/// files, or a manifest that isn't even valid JSON for the current
/// `StockManifest` shape -- the CLI wrapper maps those to exit code 2.
/// Every check the report itself records (schema_version disagreement,
/// hash mismatches, ...) is a findable, reportable condition, not an
/// invocation error, and always yields `Ok(report)` with that check's own
/// severity set to `Fail`/`Warn` rather than aborting the whole report.
fn build_stock_doctor_report(args: &[String]) -> Result<StockDoctorReport> {
    let stock_path =
        flag_value(args, "--stock").context("renkin doctor stock: --stock <path> is required")?;
    let manifest_path = flag_value(args, "--manifest")
        .context("renkin doctor stock: --manifest <path> is required")?;
    let input_path = flag_value(args, "--input");

    let stock_bytes = std::fs::read(stock_path)
        .with_context(|| format!("renkin doctor stock: failed to read --stock {stock_path:?}"))?;
    let manifest_text = std::fs::read_to_string(manifest_path).with_context(|| {
        format!("renkin doctor stock: failed to read --manifest {manifest_path:?}")
    })?;
    let manifest: stock_import::StockManifest =
        serde_json::from_str(&manifest_text).with_context(|| {
            format!("renkin doctor stock: {manifest_path:?} is not a valid stock manifest")
        })?;
    let input_bytes = input_path
        .map(|p| {
            std::fs::read(p)
                .with_context(|| format!("renkin doctor stock: failed to read --input {p:?}"))
        })
        .transpose()?;

    let mut checks = Vec::new();

    checks.push(
        if manifest.schema_version == stock_import::STOCK_MANIFEST_SCHEMA_VERSION {
            StockDoctorCheck {
                name: "schema_version",
                severity: DoctorSeverity::Pass,
                message: format!("schema_version {} is supported", manifest.schema_version),
            }
        } else {
            StockDoctorCheck {
                name: "schema_version",
                severity: DoctorSeverity::Fail,
                message: format!(
                    "manifest schema_version {} is not supported by this build (expects {})",
                    manifest.schema_version,
                    stock_import::STOCK_MANIFEST_SCHEMA_VERSION
                ),
            }
        },
    );

    let stock_actual_sha256 = format!(
        "sha256:{}",
        renkin::sha256_hex(Sha256::digest(&stock_bytes))
    );
    checks.push(if stock_actual_sha256 == manifest.output_sha256 {
        StockDoctorCheck {
            name: "output_hash",
            severity: DoctorSeverity::Pass,
            message: "stock file SHA-256 matches manifest.output_sha256".to_string(),
        }
    } else {
        StockDoctorCheck {
            name: "output_hash",
            severity: DoctorSeverity::Fail,
            message: format!(
                "stock file SHA-256 {stock_actual_sha256} does not match manifest.output_sha256 {}",
                manifest.output_sha256
            ),
        }
    });

    if let (Some(input_path), Some(input_bytes)) = (input_path, &input_bytes) {
        let input_actual_sha256 =
            format!("sha256:{}", renkin::sha256_hex(Sha256::digest(input_bytes)));
        checks.push(if input_actual_sha256 == manifest.input_sha256 {
            StockDoctorCheck {
                name: "input_hash",
                severity: DoctorSeverity::Pass,
                message: format!("{input_path} SHA-256 matches manifest.input_sha256"),
            }
        } else {
            StockDoctorCheck {
                name: "input_hash",
                severity: DoctorSeverity::Fail,
                message: format!(
                    "{input_path} SHA-256 {input_actual_sha256} does not match \
                     manifest.input_sha256 {}",
                    manifest.input_sha256
                ),
            }
        });
    }

    let arithmetic_ok = manifest.input_rows == manifest.accepted_rows + manifest.rejected_rows
        && manifest.accepted_rows == manifest.unique_structures + manifest.duplicate_rows;
    checks.push(if arithmetic_ok {
        StockDoctorCheck {
            name: "manifest_arithmetic",
            severity: DoctorSeverity::Pass,
            message: "manifest row counts are internally consistent".to_string(),
        }
    } else {
        StockDoctorCheck {
            name: "manifest_arithmetic",
            severity: DoctorSeverity::Fail,
            message: format!(
                "manifest row counts are NOT internally consistent (input_rows={}, \
                 accepted_rows={}, rejected_rows={}, unique_structures={}, duplicate_rows={})",
                manifest.input_rows,
                manifest.accepted_rows,
                manifest.rejected_rows,
                manifest.unique_structures,
                manifest.duplicate_rows
            ),
        }
    });

    let stock_text = String::from_utf8_lossy(&stock_bytes);
    let stock_line_count = stock_text.lines().filter(|l| !l.trim().is_empty()).count() as u64;
    checks.push(if stock_line_count == manifest.unique_structures {
        StockDoctorCheck {
            name: "stock_line_count",
            severity: DoctorSeverity::Pass,
            message: format!(
                "stock file has {stock_line_count} lines, matching manifest.unique_structures"
            ),
        }
    } else {
        StockDoctorCheck {
            name: "stock_line_count",
            severity: DoctorSeverity::Fail,
            message: format!(
                "stock file has {stock_line_count} lines but manifest.unique_structures is {}",
                manifest.unique_structures
            ),
        }
    });

    let reimport_options = stock_import::StockImportOptions {
        source_label: "doctor-reimport-probe".to_string(),
        source_revision: None,
        license: None,
    };
    let (reimport_accepted, reimport_manifest) =
        stock_import::import_stock(stock_bytes.as_slice(), &reimport_options).with_context(
            || "renkin doctor stock: failed to re-import --stock for the idempotency check",
        )?;
    let reimport_bytes = stock_import::render_output(&reimport_accepted);
    let byte_identical = reimport_bytes == stock_bytes;
    let idempotent = byte_identical
        && reimport_manifest.rejected_rows == 0
        && reimport_manifest.duplicate_rows == 0;
    checks.push(if idempotent {
        StockDoctorCheck {
            name: "reimport_idempotency",
            severity: DoctorSeverity::Pass,
            message: "re-importing the stock file is a no-op (already canonical, deduped, sorted)"
                .to_string(),
        }
    } else {
        StockDoctorCheck {
            name: "reimport_idempotency",
            severity: DoctorSeverity::Fail,
            message: format!(
                "re-importing the stock file is NOT a no-op (byte_identical={byte_identical}, \
                 rejected_rows={}, duplicate_rows={}) -- the stock file may have been \
                 hand-edited after generation",
                reimport_manifest.rejected_rows, reimport_manifest.duplicate_rows
            ),
        }
    });

    let current_normalization = stock_import::current_normalization_contract();
    checks.push(if manifest.normalization == current_normalization {
        StockDoctorCheck {
            name: "normalization_contract",
            severity: DoctorSeverity::Pass,
            message: "manifest normalization policy matches this build's current \
                      STANDARDIZE_OPTS"
                .to_string(),
        }
    } else {
        StockDoctorCheck {
            name: "normalization_contract",
            severity: DoctorSeverity::Fail,
            message: format!(
                "manifest normalization policy does not match this build's current \
                 STANDARDIZE_OPTS (manifest: {:?}, current: {current_normalization:?})",
                manifest.normalization
            ),
        }
    });

    let current_importer_version = env!("CARGO_PKG_VERSION");
    checks.push(if manifest.importer_version == current_importer_version {
        StockDoctorCheck {
            name: "importer_version",
            severity: DoctorSeverity::Pass,
            message: format!(
                "manifest importer_version {} matches this build",
                manifest.importer_version
            ),
        }
    } else {
        StockDoctorCheck {
            name: "importer_version",
            severity: DoctorSeverity::Warn,
            message: format!(
                "manifest importer_version {} differs from this build's \
                 {current_importer_version} (not necessarily a problem)",
                manifest.importer_version
            ),
        }
    });

    let missing_provenance: Vec<&str> = [
        ("source_revision", manifest.source.revision.is_none()),
        ("license", manifest.source.license.is_none()),
    ]
    .into_iter()
    .filter(|(_, missing)| *missing)
    .map(|(name, _)| name)
    .collect();
    checks.push(if missing_provenance.is_empty() {
        StockDoctorCheck {
            name: "source_provenance",
            severity: DoctorSeverity::Pass,
            message: "source label, revision, and license are all recorded".to_string(),
        }
    } else {
        StockDoctorCheck {
            name: "source_provenance",
            severity: DoctorSeverity::Warn,
            message: format!("manifest is missing: {}", missing_provenance.join(", ")),
        }
    });

    let overall = checks
        .iter()
        .fold(DoctorSeverity::Pass, |acc, c| acc.worse(c.severity));

    Ok(StockDoctorReport {
        stock_path: stock_path.to_string(),
        manifest_path: manifest_path.to_string(),
        input_path: input_path.map(str::to_string),
        overall,
        checks,
    })
}

fn run_doctor(args: &[String]) -> Result<()> {
    let cmd = args.first().map(|s| s.as_str()).unwrap_or("help");
    let rest = if args.len() > 1 {
        &args[1..]
    } else {
        &[] as &[String]
    };
    match cmd {
        "stock" => doctor_stock(rest),
        "templates" => doctor_templates(rest),
        _ => {
            println!("Usage: renkin doctor <cmd> [args]");
            println!(
                "  stock      --stock <normalized.smi> --manifest <normalized.manifest.json> \
                 [--input <original.smi>] [--output human|json]"
            );
            println!(
                "             — verifies a renkin stock import output/manifest pair; exit 0 \
                 PASS (warnings allowed), 1 a check FAILed, 2 invocation/input error"
            );
            println!("  templates  --templates <file.smi> [--output human|json]");
            println!(
                "             — reports an extracted-template corpus's own SHA-256, whether \
                 its header records a revision-pinned source, and load/concrete-application \
                 counts (issue #100); same exit-code contract as `stock` above"
            );
            Ok(())
        }
    }
}

/// CLI wrapper around `build_stock_doctor_report`: prints the report
/// (human text by default, or `--output json`) and terminates with a
/// stable exit code -- 0 when every check is Pass/Warn, 1 when any check
/// is Fail, 2 for an invocation-level problem (bad flags, unreadable
/// files, a manifest that doesn't even parse). Never returns normally;
/// every path ends in `std::process::exit`, matching this file's existing
/// convention for CLI-level fatal control flow (see the `ring-context`/
/// `spectator-bond` flag handling above).
fn doctor_stock(args: &[String]) -> Result<()> {
    let output_format = flag_value(args, "--output").unwrap_or("human");
    if output_format != "human" && output_format != "json" {
        eprintln!(
            "error: renkin doctor stock: unsupported --output {output_format:?} (only \
             human|json supported)"
        );
        std::process::exit(2);
    }

    let report = match build_stock_doctor_report(args) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e:#}");
            std::process::exit(2);
        }
    };

    if output_format == "json" {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "renkin doctor stock: {} ({})",
            report.overall.label(),
            report.stock_path
        );
        for check in &report.checks {
            println!(
                "  [{}] {}: {}",
                check.severity.label(),
                check.name,
                check.message
            );
        }
    }

    std::process::exit(match report.overall {
        DoctorSeverity::Fail => 1,
        DoctorSeverity::Warn | DoctorSeverity::Pass => 0,
    });
}

#[derive(Debug, Serialize)]
struct TemplateDoctorReport {
    templates_path: String,
    overall: DoctorSeverity,
    checks: Vec<StockDoctorCheck>,
}

/// Pure report-building logic for `renkin doctor templates` (issue #100).
/// Unlike `renkin doctor stock`, there is no separate manifest file to
/// compare against -- extracted templates are generated by
/// `scripts/extract_templates.py`, a Python tool this crate doesn't
/// control the output of, and that script already writes its own
/// provenance (`# Source: {dataset}@{revision}`) directly into the file's
/// header comment rather than a sidecar JSON. So every check here is
/// computed fresh from the template file itself: its own SHA-256 (for a
/// caller to record externally), whether its header records a
/// revision-pinned source, and the load/classification counts
/// `examples/hashatom_corpus_stats.rs` already computes per-file --
/// reusing that exact logic (`load_rules_from_file`/
/// `concrete_application_status`) rather than a second implementation, so
/// the two can never silently disagree.
fn build_template_doctor_report(args: &[String]) -> Result<TemplateDoctorReport> {
    let templates_path = flag_value(args, "--templates")
        .context("renkin doctor templates: --templates <path> is required")?;

    let content = std::fs::read_to_string(templates_path).with_context(|| {
        format!("renkin doctor templates: failed to read --templates {templates_path:?}")
    })?;

    let mut checks = Vec::new();

    let sha256 = format!(
        "sha256:{}",
        renkin::sha256_hex(Sha256::digest(content.as_bytes()))
    );
    checks.push(StockDoctorCheck {
        name: "sha256",
        severity: DoctorSeverity::Pass,
        message: format!("{templates_path} content hash is {sha256}"),
    });

    let source_line = content
        .lines()
        .find(|l| l.trim_start().starts_with("# Source:"));
    checks.push(match source_line {
        Some(line) if line.contains('@') => StockDoctorCheck {
            name: "source_header",
            severity: DoctorSeverity::Pass,
            message: format!("revision-pinned source header found: {}", line.trim()),
        },
        Some(line) => StockDoctorCheck {
            name: "source_header",
            severity: DoctorSeverity::Warn,
            message: format!(
                "source header present but not revision-pinned (no '@revision'): {}",
                line.trim()
            ),
        },
        None => StockDoctorCheck {
            name: "source_header",
            severity: DoctorSeverity::Warn,
            message: "no '# Source:' header line found -- generation provenance is unrecorded"
                .to_string(),
        },
    });

    let raw_template_lines = content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .count();
    let rules = chem_env::load_rules_from_file(templates_path);
    let logical_rules_loaded = rules.len();
    let load_rejected = raw_template_lines.saturating_sub(logical_rules_loaded);

    checks.push(if raw_template_lines == 0 {
        StockDoctorCheck {
            name: "load_success",
            severity: DoctorSeverity::Fail,
            message: "0 raw template lines found in file".to_string(),
        }
    } else if logical_rules_loaded == 0 {
        StockDoctorCheck {
            name: "load_success",
            severity: DoctorSeverity::Fail,
            message: format!(
                "0 of {raw_template_lines} raw template lines loaded as a usable RetroRule"
            ),
        }
    } else if load_rejected > 0 {
        StockDoctorCheck {
            name: "load_success",
            severity: DoctorSeverity::Warn,
            message: format!(
                "{logical_rules_loaded}/{raw_template_lines} raw lines loaded \
                 ({load_rejected} rejected by load_rules_from_file, reasons not further \
                 classified here -- unrelated to hash-atom support, see Issue #88)"
            ),
        }
    } else {
        StockDoctorCheck {
            name: "load_success",
            severity: DoctorSeverity::Pass,
            message: format!("all {raw_template_lines} raw template lines loaded"),
        }
    });

    let mut direct = 0usize;
    let mut hash_atom_supported = 0usize;
    let mut unsupported_by_reason: std::collections::BTreeMap<&'static str, usize> =
        std::collections::BTreeMap::new();
    for rule in &rules {
        match chem_env::concrete_application_status(&rule.smirks) {
            chem_env::ConcreteApplicationStatus::Direct => direct += 1,
            chem_env::ConcreteApplicationStatus::HashAtomVariants { .. } => {
                hash_atom_supported += 1
            }
            chem_env::ConcreteApplicationStatus::Unsupported { reason } => {
                let key = match reason {
                    chem_env::HashAtomUnsupportedReason::UnhandledSyntax => "unhandled_syntax",
                    chem_env::HashAtomUnsupportedReason::InconsistentElement => {
                        "inconsistent_element"
                    }
                    chem_env::HashAtomUnsupportedReason::VariantLimitExceeded { .. } => {
                        "variant_limit_exceeded"
                    }
                    chem_env::HashAtomUnsupportedReason::NoValidVariant => "no_valid_variant",
                };
                *unsupported_by_reason.entry(key).or_insert(0) += 1;
            }
        }
    }
    let unsupported: usize = unsupported_by_reason.values().sum();
    let applicable = direct + hash_atom_supported;
    checks.push(if logical_rules_loaded == 0 {
        StockDoctorCheck {
            name: "concrete_application",
            severity: DoctorSeverity::Warn,
            message: "no loaded templates to classify".to_string(),
        }
    } else if unsupported > 0 {
        StockDoctorCheck {
            name: "concrete_application",
            severity: DoctorSeverity::Warn,
            message: format!(
                "{applicable}/{logical_rules_loaded} loaded templates are concretely \
                 applicable ({unsupported} unsupported: {unsupported_by_reason:?})"
            ),
        }
    } else {
        StockDoctorCheck {
            name: "concrete_application",
            severity: DoctorSeverity::Pass,
            message: format!(
                "all {logical_rules_loaded} loaded templates are concretely applicable \
                 ({direct} direct, {hash_atom_supported} via hash-atom variant expansion)"
            ),
        }
    });

    let overall = checks
        .iter()
        .fold(DoctorSeverity::Pass, |acc, c| acc.worse(c.severity));

    Ok(TemplateDoctorReport {
        templates_path: templates_path.to_string(),
        overall,
        checks,
    })
}

/// CLI wrapper around `build_template_doctor_report`, matching
/// `doctor_stock`'s output/exit-code contract exactly (0 PASS/WARN, 1 a
/// check FAILed, 2 invocation error) for consistency across `renkin
/// doctor` subcommands.
fn doctor_templates(args: &[String]) -> Result<()> {
    let output_format = flag_value(args, "--output").unwrap_or("human");
    if output_format != "human" && output_format != "json" {
        eprintln!(
            "error: renkin doctor templates: unsupported --output {output_format:?} (only \
             human|json supported)"
        );
        std::process::exit(2);
    }

    let report = match build_template_doctor_report(args) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e:#}");
            std::process::exit(2);
        }
    };

    if output_format == "json" {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "renkin doctor templates: {} ({})",
            report.overall.label(),
            report.templates_path
        );
        for check in &report.checks {
            println!(
                "  [{}] {}: {}",
                check.severity.label(),
                check.name,
                check.message
            );
        }
    }

    std::process::exit(match report.overall {
        DoctorSeverity::Fail => 1,
        DoctorSeverity::Warn | DoctorSeverity::Pass => 0,
    });
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

#[cfg(test)]
mod hash_atom_unsupported_report_tests {
    use super::*;

    fn rule(name: &str, smirks: &str) -> chem_env::RetroRule {
        chem_env::RetroRule {
            name: name.to_string(),
            template_id: chem_env::template_id_for_smirks(smirks),
            smirks: smirks.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn no_unsupported_templates_yields_empty_map() {
        let rules = vec![rule(
            "ester_cleavage",
            "[O:3]=[C:2]-[OH:1]>>C-[O:1]-[C:2]=[O:3]",
        )];
        assert!(count_hash_atom_unsupported(&rules).is_empty());
    }

    #[test]
    fn inconsistent_element_is_classified_and_counted() {
        // Same atom-map number (1) resolves to carbon on the reactant side
        // and nitrogen on the product side -- internally inconsistent.
        let rules = vec![rule("bad", "[#6:1]>>[#7:1]")];
        let by_reason = count_hash_atom_unsupported(&rules);
        assert_eq!(by_reason.get("inconsistent_element"), Some(&1));
        assert_eq!(by_reason.values().sum::<usize>(), 1);
    }

    #[test]
    fn multiple_unsupported_rules_of_the_same_reason_accumulate() {
        let rules = vec![
            rule("bad1", "[#6:1]>>[#7:1]"),
            rule("bad2", "[#8:2]>>[#9:2]"),
            rule("ok", "[O:3]=[C:2]-[OH:1]>>C-[O:1]-[C:2]=[O:3]"),
        ];
        let by_reason = count_hash_atom_unsupported(&rules);
        assert_eq!(by_reason.get("inconsistent_element"), Some(&2));
        assert_eq!(by_reason.values().sum::<usize>(), 2);
    }
}

#[cfg(test)]
mod stock_import_cli_tests {
    use super::*;

    fn unique_temp_dir(label: &str) -> std::path::PathBuf {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "renkin_stock_import_cli_unit_{label}_{}_{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // ── write_two_artifacts_atomically ──────────────────────────────────

    #[test]
    fn atomic_write_rolls_back_output_when_second_rename_fails() {
        let dir = unique_temp_dir("atomicity");
        let path_a = dir.join("out.smi");
        // A pre-existing directory at path_b makes `fs::rename(tmp_b, path_b)`
        // fail deterministically (can't rename a file onto a directory).
        let path_b = dir.join("out.manifest.json");
        std::fs::create_dir_all(&path_b).unwrap();

        let result = write_two_artifacts_atomically(
            path_a.to_str().unwrap(),
            b"content-a",
            path_b.to_str().unwrap(),
            b"content-b",
        );
        assert!(result.is_err(), "{result:?}");
        assert!(
            !path_a.exists(),
            "output must be rolled back to 'did not exist' when the manifest rename fails \
             and output didn't exist before this call"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn atomic_write_succeeds_when_both_renames_succeed() {
        let dir = unique_temp_dir("atomicity_ok");
        let path_a = dir.join("out.smi");
        let path_b = dir.join("out.manifest.json");
        write_two_artifacts_atomically(
            path_a.to_str().unwrap(),
            b"content-a",
            path_b.to_str().unwrap(),
            b"content-b",
        )
        .unwrap();
        assert_eq!(std::fs::read(&path_a).unwrap(), b"content-a");
        assert_eq!(std::fs::read(&path_b).unwrap(), b"content-b");
        // No leftover temp files.
        assert!(!dir.join("out.smi.stock-import-tmp").exists());
        assert!(!dir.join("out.manifest.json.stock-import-tmp").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    // ── build_stock_doctor_report ────────────────────────────────────────

    /// Writes a real stock/manifest pair (via the actual `import_stock`
    /// core, not hand-authored JSON) into a fresh temp dir and returns
    /// (stock_path, manifest_path, input_path, the manifest). Each
    /// "altered" test below tampers with the written files, not this
    /// helper's output, so the untampered baseline always matches what
    /// `stock_import` itself would really produce.
    fn setup_stock_and_manifest(
        dir: &std::path::Path,
        source_revision: Option<&str>,
        license: Option<&str>,
    ) -> (String, String, String, stock_import::StockManifest) {
        let input = "CCO ethanol\nCC(=O)O acetic\n";
        let input_path = dir.join("input.smi");
        std::fs::write(&input_path, input).unwrap();

        let options = stock_import::StockImportOptions {
            source_label: "unit-test".to_string(),
            source_revision: source_revision.map(str::to_string),
            license: license.map(str::to_string),
        };
        let (accepted, manifest) = stock_import::import_stock(input.as_bytes(), &options).unwrap();

        let stock_path = dir.join("stock.smi");
        let manifest_path = dir.join("stock.manifest.json");
        std::fs::write(&stock_path, stock_import::render_output(&accepted)).unwrap();
        std::fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        (
            stock_path.to_str().unwrap().to_string(),
            manifest_path.to_str().unwrap().to_string(),
            input_path.to_str().unwrap().to_string(),
            manifest,
        )
    }

    fn find_check<'a>(report: &'a StockDoctorReport, name: &str) -> &'a StockDoctorCheck {
        report
            .checks
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("no check named {name:?} in {report:?}"))
    }

    #[test]
    fn doctor_report_all_consistent_is_pass() {
        let dir = unique_temp_dir("all_consistent");
        let (stock_path, manifest_path, input_path, _) =
            setup_stock_and_manifest(&dir, Some("rev-1"), Some("CC0"));
        let args = vec![
            "--stock".to_string(),
            stock_path,
            "--manifest".to_string(),
            manifest_path,
            "--input".to_string(),
            input_path,
        ];
        let report = build_stock_doctor_report(&args).unwrap();
        assert_eq!(report.overall, DoctorSeverity::Pass, "{report:?}");
        for check in &report.checks {
            assert_eq!(check.severity, DoctorSeverity::Pass, "{report:?}");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn doctor_report_altered_stock_hash_is_fail() {
        let dir = unique_temp_dir("altered_hash");
        let (stock_path, manifest_path, _, _) = setup_stock_and_manifest(&dir, None, None);
        let mut bytes = std::fs::read(&stock_path).unwrap();
        bytes.push(b'\n');
        std::fs::write(&stock_path, bytes).unwrap();

        let args = vec![
            "--stock".to_string(),
            stock_path,
            "--manifest".to_string(),
            manifest_path,
        ];
        let report = build_stock_doctor_report(&args).unwrap();
        assert_eq!(report.overall, DoctorSeverity::Fail, "{report:?}");
        assert_eq!(
            find_check(&report, "output_hash").severity,
            DoctorSeverity::Fail,
            "{report:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn doctor_report_altered_manifest_counts_is_fail() {
        let dir = unique_temp_dir("altered_counts");
        let (stock_path, manifest_path, _, _) = setup_stock_and_manifest(&dir, None, None);
        let mut value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
        value["input_rows"] = serde_json::json!(999);
        std::fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&value).unwrap(),
        )
        .unwrap();

        let args = vec![
            "--stock".to_string(),
            stock_path,
            "--manifest".to_string(),
            manifest_path,
        ];
        let report = build_stock_doctor_report(&args).unwrap();
        assert_eq!(report.overall, DoctorSeverity::Fail, "{report:?}");
        assert_eq!(
            find_check(&report, "manifest_arithmetic").severity,
            DoctorSeverity::Fail,
            "{report:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn doctor_report_normalization_mismatch_is_fail() {
        let dir = unique_temp_dir("norm_mismatch");
        let (stock_path, manifest_path, _, _) = setup_stock_and_manifest(&dir, None, None);
        let mut value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
        let current = !value["normalization"]["remove_explicit_h"]
            .as_bool()
            .unwrap();
        value["normalization"]["remove_explicit_h"] = serde_json::json!(current);
        std::fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&value).unwrap(),
        )
        .unwrap();

        let args = vec![
            "--stock".to_string(),
            stock_path,
            "--manifest".to_string(),
            manifest_path,
        ];
        let report = build_stock_doctor_report(&args).unwrap();
        assert_eq!(report.overall, DoctorSeverity::Fail, "{report:?}");
        assert_eq!(
            find_check(&report, "normalization_contract").severity,
            DoctorSeverity::Fail,
            "{report:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn doctor_report_importer_version_difference_is_warn_only() {
        let dir = unique_temp_dir("version_diff");
        let (stock_path, manifest_path, _, _) = setup_stock_and_manifest(&dir, None, None);
        let mut value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
        value["importer_version"] = serde_json::json!("0.0.1-fake");
        std::fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&value).unwrap(),
        )
        .unwrap();

        let args = vec![
            "--stock".to_string(),
            stock_path,
            "--manifest".to_string(),
            manifest_path,
        ];
        let report = build_stock_doctor_report(&args).unwrap();
        assert_eq!(
            find_check(&report, "importer_version").severity,
            DoctorSeverity::Warn,
            "{report:?}"
        );
        assert_eq!(
            report.overall,
            DoctorSeverity::Warn,
            "an importer_version-only difference must not escalate the overall verdict to \
             Fail: {report:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn doctor_report_missing_revision_and_license_is_warn_only() {
        let dir = unique_temp_dir("missing_provenance");
        let (stock_path, manifest_path, _, _) = setup_stock_and_manifest(&dir, None, None);
        let args = vec![
            "--stock".to_string(),
            stock_path,
            "--manifest".to_string(),
            manifest_path,
        ];
        let report = build_stock_doctor_report(&args).unwrap();
        let check = find_check(&report, "source_provenance");
        assert_eq!(check.severity, DoctorSeverity::Warn, "{report:?}");
        assert!(check.message.contains("source_revision"), "{check:?}");
        assert!(check.message.contains("license"), "{check:?}");
        assert_eq!(
            report.overall,
            DoctorSeverity::Warn,
            "missing optional provenance must not escalate the overall verdict to Fail: \
             {report:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn doctor_report_input_hash_mismatch_is_fail() {
        let dir = unique_temp_dir("input_hash_mismatch");
        let (stock_path, manifest_path, _, _) = setup_stock_and_manifest(&dir, None, None);
        let wrong_input_path = dir.join("wrong_input.smi");
        std::fs::write(&wrong_input_path, "CCCC not the original input\n").unwrap();

        let args = vec![
            "--stock".to_string(),
            stock_path,
            "--manifest".to_string(),
            manifest_path,
            "--input".to_string(),
            wrong_input_path.to_str().unwrap().to_string(),
        ];
        let report = build_stock_doctor_report(&args).unwrap();
        assert_eq!(report.overall, DoctorSeverity::Fail, "{report:?}");
        assert_eq!(
            find_check(&report, "input_hash").severity,
            DoctorSeverity::Fail,
            "{report:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn doctor_report_unsupported_schema_version_is_fail() {
        let dir = unique_temp_dir("bad_schema");
        let (stock_path, manifest_path, _, _) = setup_stock_and_manifest(&dir, None, None);
        let mut value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
        value["schema_version"] = serde_json::json!(999);
        std::fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&value).unwrap(),
        )
        .unwrap();

        let args = vec![
            "--stock".to_string(),
            stock_path,
            "--manifest".to_string(),
            manifest_path,
        ];
        let report = build_stock_doctor_report(&args).unwrap();
        assert_eq!(report.overall, DoctorSeverity::Fail, "{report:?}");
        assert_eq!(
            find_check(&report, "schema_version").severity,
            DoctorSeverity::Fail,
            "{report:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn doctor_report_missing_required_flag_is_invocation_error() {
        let err =
            build_stock_doctor_report(&["--manifest".to_string(), "x".to_string()]).unwrap_err();
        assert!(format!("{err:#}").contains("--stock"), "{err:#}");
    }

    #[test]
    fn doctor_report_malformed_manifest_json_is_invocation_error() {
        let dir = unique_temp_dir("malformed_manifest");
        let (stock_path, _, _, _) = setup_stock_and_manifest(&dir, None, None);
        let manifest_path = dir.join("not_json.manifest.json");
        std::fs::write(&manifest_path, "not valid json").unwrap();

        let args = vec![
            "--stock".to_string(),
            stock_path,
            "--manifest".to_string(),
            manifest_path.to_str().unwrap().to_string(),
        ];
        assert!(build_stock_doctor_report(&args).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    fn find_template_check<'a>(
        report: &'a TemplateDoctorReport,
        name: &str,
    ) -> &'a StockDoctorCheck {
        report
            .checks
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("no {name} check in {report:?}"))
    }

    #[test]
    fn template_doctor_pinned_source_and_all_applicable_is_pass() {
        let dir = unique_temp_dir("template_doctor_pass");
        let path = dir.join("templates.smi");
        std::fs::write(
            &path,
            "# Source: bisectgroup/USPTO_50K@08a575f0546b2be57242997fd45f684d6814d5a9\n\
             [C:1][OH:2]>>[C:1]=O\t5\n",
        )
        .unwrap();

        let args = vec![
            "--templates".to_string(),
            path.to_str().unwrap().to_string(),
        ];
        let report = build_template_doctor_report(&args).unwrap();
        assert_eq!(report.overall, DoctorSeverity::Pass, "{report:?}");
        assert_eq!(
            find_template_check(&report, "source_header").severity,
            DoctorSeverity::Pass
        );
        assert_eq!(
            find_template_check(&report, "load_success").severity,
            DoctorSeverity::Pass
        );
        assert_eq!(
            find_template_check(&report, "concrete_application").severity,
            DoctorSeverity::Pass
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn template_doctor_unpinned_source_is_warn_not_fail() {
        let dir = unique_temp_dir("template_doctor_unpinned");
        let path = dir.join("templates.smi");
        std::fs::write(
            &path,
            "# Source: bisectgroup/USPTO_50K (train split)\n[C:1][OH:2]>>[C:1]=O\t5\n",
        )
        .unwrap();

        let args = vec![
            "--templates".to_string(),
            path.to_str().unwrap().to_string(),
        ];
        let report = build_template_doctor_report(&args).unwrap();
        assert_eq!(report.overall, DoctorSeverity::Warn, "{report:?}");
        assert_eq!(
            find_template_check(&report, "source_header").severity,
            DoctorSeverity::Warn
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn template_doctor_missing_source_header_is_warn() {
        let dir = unique_temp_dir("template_doctor_no_header");
        let path = dir.join("templates.smi");
        std::fs::write(&path, "[C:1][OH:2]>>[C:1]=O\t5\n").unwrap();

        let args = vec![
            "--templates".to_string(),
            path.to_str().unwrap().to_string(),
        ];
        let report = build_template_doctor_report(&args).unwrap();
        assert_eq!(
            find_template_check(&report, "source_header").severity,
            DoctorSeverity::Warn
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn template_doctor_unparseable_line_is_warn_not_fail() {
        let dir = unique_temp_dir("template_doctor_rejected_line");
        let path = dir.join("templates.smi");
        std::fs::write(
            &path,
            "[C:1][OH:2]>>[C:1]=O\t5\nthis is not valid smirks at all\t1\n",
        )
        .unwrap();

        let args = vec![
            "--templates".to_string(),
            path.to_str().unwrap().to_string(),
        ];
        let report = build_template_doctor_report(&args).unwrap();
        assert_eq!(
            find_template_check(&report, "load_success").severity,
            DoctorSeverity::Warn,
            "{report:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn template_doctor_empty_file_is_fail() {
        let dir = unique_temp_dir("template_doctor_empty");
        let path = dir.join("templates.smi");
        std::fs::write(&path, "# Source: nothing here\n").unwrap();

        let args = vec![
            "--templates".to_string(),
            path.to_str().unwrap().to_string(),
        ];
        let report = build_template_doctor_report(&args).unwrap();
        assert_eq!(report.overall, DoctorSeverity::Fail, "{report:?}");
        assert_eq!(
            find_template_check(&report, "load_success").severity,
            DoctorSeverity::Fail
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn template_doctor_missing_required_flag_is_invocation_error() {
        let err = build_template_doctor_report(&[]).unwrap_err();
        assert!(format!("{err:#}").contains("--templates"));
    }

    #[test]
    fn template_doctor_missing_file_is_invocation_error() {
        let args = vec![
            "--templates".to_string(),
            "/nonexistent/path/templates.smi".to_string(),
        ];
        assert!(build_template_doctor_report(&args).is_err());
    }
}
