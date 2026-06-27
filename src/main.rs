#![forbid(unsafe_code)]

use renkin::DEFAULT_BUILDING_BLOCKS;
use renkin::chem_env;
use renkin::display;
use renkin::search::{self, SearchConfig};

use anyhow::{Result, bail};
use serde::Serialize;

#[derive(Serialize)]
struct Output {
    target: String,
    routes_found: usize,
    routes: Vec<search::Route>,
    /// P(at least one route succeeds) = 1 − Π(1 − route.success_probability).
    joint_success_probability: f64,
}

// ..Default::default() is needed when nn-scoring feature is enabled (adds nn_scorer field).
// When the feature is off, all fields are explicit, making the spread redundant — suppress lint.
#[allow(clippy::needless_update)]
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    let mut target: Option<String> = None;
    let mut max_depth: u32 = 5;
    let mut bb_path: Option<String> = None;
    let mut templates_path: Option<String> = None;
    let mut max_routes: usize = 5;
    let mut beam_width: usize = 0;
    let mut format: String = "json".to_string();
    let mut avoid_elements: String = String::new();
    let mut require_elements: String = String::new();
    let mut verbose = false;
    let mut bond_index = false;
    let mut bb_prices_path: Option<String> = None;
    #[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
    let mut scorer_path: Option<String> = None;

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
            "--bond-index" => {
                bond_index = true;
            }
            "--bb-prices" => {
                i += 1;
                if i < args.len() {
                    bb_prices_path = Some(args[i].clone());
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
             --format / -f      Output format: json (default), tree, mermaid\n  \
             --avoid-elements / -e  Comma-separated elements to ban from BBs (e.g. \"Br,I\")\n  \
             --require-elements / -r  Comma-separated elements each route must supply (e.g. \"B\")\n  \
             --verbose / -v         Print search statistics to stderr\n  \
             --bond-index           Bond-center template index: ~24%% faster, no accuracy loss\n  \
             --bb-prices <path>     CSV (SMILES,price_per_gram) for route cost scoring"
        );
    };

    let env = match bb_path {
        Some(ref path) => chem_env::ChemEnv::load(path)?,
        None => chem_env::ChemEnv::load("data/building_blocks.smi")
            .unwrap_or_else(|_| chem_env::ChemEnv::in_memory(DEFAULT_BUILDING_BLOCKS)),
    };

    let mut rules = chem_env::default_rules();
    if let Some(ref path) = templates_path {
        let extra = chem_env::load_rules_from_file(path);
        eprintln!("Loaded {} templates from {path}", extra.len());
        rules.extend(extra);
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

    let bb_price_map = bb_prices_path.as_deref().map(load_prices);

    let config = SearchConfig {
        max_depth,
        max_routes,
        beam_width,
        forbidden_elements: chem_env::elem_symbols_to_mask(&avoid_elements),
        required_element_present: chem_env::elem_symbols_to_mask(&require_elements),
        verbose,
        bond_index,
        bb_price_map,
        #[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
        nn_scorer,
        ..Default::default()
    };
    let (routes, stats) = search::find_routes(&target_smiles, &env, &rules, &config)?;

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
        _ => {
            if routes.is_empty() {
                let (causes, suggestions) = diagnose(&stats, max_depth);
                let out = serde_json::json!({
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
                    routes,
                };
                println!("{}", serde_json::to_string_pretty(&output)?);
            }
        }
    }
    Ok(())
}

fn diagnose(stats: &search::SearchStats, max_depth: u32) -> (Vec<&'static str>, Vec<String>) {
    let mut causes: Vec<&'static str> = Vec::new();
    let mut suggestions: Vec<String> = Vec::new();
    if stats.stock_hits == 0 {
        causes.push("no matching building block in stock");
        suggestions.push("add a custom stock file with --building-blocks".to_string());
    }
    if stats.max_depth_reached {
        causes.push("search depth exhausted");
        suggestions.push(format!("try --depth {}", max_depth + 2));
    }
    if stats.beam_limit_hit {
        causes.push("beam width too narrow — candidates were pruned");
        suggestions.push("try --beam-width 200".to_string());
    }
    if stats.matched_templates < 5 {
        causes.push("few or no templates matched the target");
        suggestions.push("try --templates data/templates_extracted_50000.smi".to_string());
    }
    (causes, suggestions)
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
