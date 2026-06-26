#![forbid(unsafe_code)]

/// RENKIN Benchmark Runner
///
/// Usage:
///   renkin-bench --input <smiles_file> [--depth <N>] [--beam-width <N>]
///
/// Input file format (one SMILES per line, optional name after whitespace):
///   CC(=O)Oc1ccccc1C(=O)O  aspirin
///   c1ccc(N)cc1C(=O)O       anthranilic_acid
///
/// Output (JSON):
///   {
///     "total": 10, "solved": 8, "success_rate": 0.8,
///     "avg_depth": 1.5, "avg_time_ms": 12.3,
///     "results": [...]
///   }
use std::time::Instant;

use anyhow::{Result, bail};
use renkin::DEFAULT_BUILDING_BLOCKS;
use renkin::chem_env::{ChemEnv, default_rules, load_rules_from_file};
use renkin::search::{SearchConfig, find_routes};
use serde::Serialize;

#[derive(Serialize)]
struct BenchResult {
    smiles: String,
    name: String,
    solved: bool,
    routes_found: usize,
    best_depth: Option<u32>,
    time_ms: f64,
    nodes_expanded: u64,
    best_confidence: Option<f64>,
    best_success_prob: Option<f64>,
    best_convergency: Option<f64>,
    best_route_cost: Option<f64>,
}

#[derive(Serialize)]
struct BenchReport {
    total: usize,
    solved: usize,
    success_rate: f64,
    avg_depth: f64,
    avg_time_ms: f64,
    avg_nodes_expanded: f64,
    /// Average best_confidence over solved targets (None-safe).
    avg_confidence: f64,
    /// Average best_convergency over solved targets.
    avg_convergency: f64,
    /// Average best_success_prob over solved targets (Retro-prob style).
    avg_success_prob: f64,
    /// Average best_route_cost over solved targets.
    avg_route_cost: f64,
    results: Vec<BenchResult>,
}

// ..Default::default() is needed when nn-scoring feature is enabled (adds nn_scorer field).
// When the feature is off, all fields are explicit, making the spread redundant — suppress lint.
#[allow(clippy::needless_update)]
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    let mut input_path: Option<String> = None;
    let mut bb_path: Option<String> = None;
    let mut templates_path: Option<String> = None;
    let mut max_depth: u32 = 5;
    let mut beam_width: usize = 0;
    let mut max_routes: usize = 1;
    let mut bond_index = false;
    #[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
    let mut scorer_path: Option<String> = None;
    #[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
    let mut scorer_top_k: Option<usize> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--input" | "-i" => {
                i += 1;
                if i < args.len() {
                    input_path = Some(args[i].clone());
                }
            }
            "--depth" | "-d" => {
                i += 1;
                if i < args.len() {
                    max_depth = args[i].parse().unwrap_or(5);
                }
            }
            "--beam-width" | "-w" => {
                i += 1;
                if i < args.len() {
                    beam_width = args[i].parse().unwrap_or(0);
                }
            }
            "--max-routes" | "-n" => {
                i += 1;
                if i < args.len() {
                    max_routes = args[i].parse().unwrap_or(1);
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
            "--bond-index" => {
                bond_index = true;
            }
            #[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
            "--scorer" => {
                i += 1;
                if i < args.len() {
                    scorer_path = Some(args[i].clone());
                }
            }
            #[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
            "--scorer-top-k" => {
                i += 1;
                if i < args.len() {
                    scorer_top_k = args[i].parse().ok();
                }
            }
            _ => {}
        }
        i += 1;
    }

    let Some(input) = input_path else {
        bail!(
            "Usage: renkin-bench --input <smiles_file> [--depth <N>] \
             [--beam-width <N>] [--building-blocks <path>] [--templates <path>] \
             [--scorer <onnx_path>]"
        );
    };

    let content = std::fs::read_to_string(&input)?;
    let targets: Vec<(String, String)> = content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|line| {
            let mut parts = line.splitn(2, char::is_whitespace);
            let smiles = parts.next().unwrap_or("").to_string();
            let name = parts.next().unwrap_or("").trim().to_string();
            (smiles, name)
        })
        .collect();

    if targets.is_empty() {
        bail!("No targets found in {input}");
    }

    let env = match bb_path {
        Some(ref path) => ChemEnv::load(path)?,
        None => ChemEnv::load("data/building_blocks.smi")
            .unwrap_or_else(|_| ChemEnv::in_memory(DEFAULT_BUILDING_BLOCKS)),
    };

    let mut rules = default_rules();
    if let Some(ref path) = templates_path {
        let extra = load_rules_from_file(path);
        eprintln!("Loaded {} templates from {path}", extra.len());
        rules.extend(extra);
    }
    #[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
    let nn_scorer: Option<std::sync::Arc<renkin::scorer::nn::TemplateScorer>> =
        scorer_path.as_deref().map(|p| {
            // Default: all rules (reranker mode). Pass --scorer-top-k N to filter.
            let top_k = scorer_top_k.unwrap_or(rules.len());
            let rules_offset = default_rules().len();
            renkin::scorer::nn::TemplateScorer::from_path(p, top_k, rules_offset)
                .map(std::sync::Arc::new)
                .unwrap_or_else(|e| {
                    eprintln!("scorer load error: {e}");
                    std::process::exit(1)
                })
        });

    let config = SearchConfig {
        max_depth,
        max_routes,
        beam_width,
        bond_index,
        #[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
        nn_scorer,
        ..Default::default()
    };

    eprintln!(
        "Benchmarking {} targets (depth={}, beam_width={}) ...",
        targets.len(),
        max_depth,
        beam_width
    );

    let mut results = Vec::new();
    let mut total_depth_sum = 0u32;
    let mut solved_count = 0usize;

    for (smiles, name) in &targets {
        let t0 = Instant::now();
        let (routes, stats) = find_routes(smiles, &env, &rules, &config).unwrap_or_default();
        let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;

        let solved = !routes.is_empty();
        let best_depth = routes.iter().map(|r| r.depth).min();
        let best_confidence = routes.first().map(|r| r.confidence);
        let best_success_prob = routes.first().map(|r| r.success_probability);
        let best_convergency = routes.first().map(|r| r.convergency);
        let best_route_cost = routes.first().map(|r| r.route_cost);

        if solved {
            solved_count += 1;
            if let Some(d) = best_depth {
                total_depth_sum += d;
            }
        }

        eprintln!(
            "  [{}/{}] {} → {} route(s) in {:.1}ms (nodes={})",
            results.len() + 1,
            targets.len(),
            smiles,
            routes.len(),
            elapsed_ms,
            stats.nodes_expanded,
        );

        results.push(BenchResult {
            smiles: smiles.clone(),
            name: name.clone(),
            solved,
            routes_found: routes.len(),
            best_depth,
            time_ms: elapsed_ms,
            nodes_expanded: stats.nodes_expanded,
            best_confidence,
            best_success_prob,
            best_convergency,
            best_route_cost,
        });
    }

    let total = results.len();
    let success_rate = solved_count as f64 / total as f64;
    let avg_depth = if solved_count > 0 {
        total_depth_sum as f64 / solved_count as f64
    } else {
        0.0
    };
    let avg_time_ms = results.iter().map(|r| r.time_ms).sum::<f64>() / total as f64;
    let avg_nodes_expanded =
        results.iter().map(|r| r.nodes_expanded as f64).sum::<f64>() / total as f64;

    let solved_results: Vec<&BenchResult> = results.iter().filter(|r| r.solved).collect();
    let avg_confidence = if solved_results.is_empty() {
        0.0
    } else {
        solved_results.iter().filter_map(|r| r.best_confidence).sum::<f64>()
            / solved_results.len() as f64
    };
    let avg_convergency = if solved_results.is_empty() {
        0.0
    } else {
        solved_results.iter().filter_map(|r| r.best_convergency).sum::<f64>()
            / solved_results.len() as f64
    };
    let avg_success_prob = if solved_results.is_empty() {
        0.0
    } else {
        solved_results.iter().filter_map(|r| r.best_success_prob).sum::<f64>()
            / solved_results.len() as f64
    };
    let avg_route_cost = if solved_results.is_empty() {
        0.0
    } else {
        solved_results.iter().filter_map(|r| r.best_route_cost).sum::<f64>()
            / solved_results.len() as f64
    };

    let report = BenchReport {
        total,
        solved: solved_count,
        success_rate,
        avg_depth,
        avg_time_ms,
        avg_nodes_expanded,
        avg_confidence,
        avg_convergency,
        avg_success_prob,
        avg_route_cost,
        results,
    };

    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
