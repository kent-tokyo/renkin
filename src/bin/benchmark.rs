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

use anyhow::{bail, Result};
use renkin::chem_env::{ChemEnv, default_rules};
use renkin::search::{SearchConfig, find_routes};
use renkin::DEFAULT_BUILDING_BLOCKS;
use serde::Serialize;

#[derive(Serialize)]
struct BenchResult {
    smiles: String,
    name: String,
    solved: bool,
    routes_found: usize,
    best_depth: Option<u32>,
    time_ms: f64,
}

#[derive(Serialize)]
struct BenchReport {
    total: usize,
    solved: usize,
    success_rate: f64,
    avg_depth: f64,
    avg_time_ms: f64,
    results: Vec<BenchResult>,
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    let mut input_path: Option<String> = None;
    let mut bb_path: Option<String> = None;
    let mut max_depth: u32 = 5;
    let mut beam_width: usize = 0;
    let mut max_routes: usize = 1;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--input" | "-i" => {
                i += 1;
                if i < args.len() { input_path = Some(args[i].clone()); }
            }
            "--depth" | "-d" => {
                i += 1;
                if i < args.len() { max_depth = args[i].parse().unwrap_or(5); }
            }
            "--beam-width" | "-w" => {
                i += 1;
                if i < args.len() { beam_width = args[i].parse().unwrap_or(0); }
            }
            "--max-routes" | "-n" => {
                i += 1;
                if i < args.len() { max_routes = args[i].parse().unwrap_or(1); }
            }
            "--building-blocks" | "-b" => {
                i += 1;
                if i < args.len() { bb_path = Some(args[i].clone()); }
            }
            _ => {}
        }
        i += 1;
    }

    let Some(input) = input_path else {
        bail!(
            "Usage: renkin-bench --input <smiles_file> [--depth <N>] \
             [--beam-width <N>] [--building-blocks <path>]"
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

    let rules = default_rules();
    let config = SearchConfig { max_depth, max_routes, beam_width };

    eprintln!("Benchmarking {} targets (depth={}, beam_width={}) ...", targets.len(), max_depth, beam_width);

    let mut results = Vec::new();
    let mut total_depth_sum = 0u32;
    let mut solved_count = 0usize;

    for (smiles, name) in &targets {
        let t0 = Instant::now();
        let routes = find_routes(smiles, &env, &rules, &config).unwrap_or_default();
        let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;

        let solved = !routes.is_empty();
        let best_depth = routes.iter().map(|r| r.depth).min();

        if solved {
            solved_count += 1;
            if let Some(d) = best_depth { total_depth_sum += d; }
        }

        eprintln!("  [{}/{}] {} → {} route(s) in {:.1}ms",
            results.len() + 1, targets.len(), smiles, routes.len(), elapsed_ms);

        results.push(BenchResult {
            smiles: smiles.clone(),
            name: name.clone(),
            solved,
            routes_found: routes.len(),
            best_depth,
            time_ms: elapsed_ms,
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

    let report = BenchReport {
        total,
        solved: solved_count,
        success_rate,
        avg_depth,
        avg_time_ms,
        results,
    };

    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
