#![forbid(unsafe_code)]

/// RENKIN Benchmark Runner
///
/// Usage:
///   renkin-bench --input <smiles_file|paroutes.json> [--input-format smi|paroutes]
///                [--depth <N>] [--beam-width <N>]
///
///   renkin-bench compare <baseline.json> <current.json>
///       Compare two renkin-bench JSON outputs and show solved-rate delta,
///       newly solved targets, and regressions.
///
/// Input formats:
///   smi (default): one SMILES per line, optional name after whitespace
///   paroutes: PaRoutes JSON — list of route trees (Genheden et al., 2022)
///
/// Output (JSON):
///   {
///     "total": 10, "solved": 8, "success_rate": 0.8,
///     "avg_depth": 1.5, "avg_time_ms": 12.3,
///     "avg_route_diversity": 0.62,
///     "results": [...]
///   }
use std::io::Write as _;
use std::time::Instant;

use anyhow::{Result, bail};
use chematic::chem::molecular_weight;
use chematic::rxn::run_reactants;
use chematic::smiles::canonical_smiles;
use renkin::DEFAULT_BUILDING_BLOCKS;
use renkin::chem_env::{ChemEnv, RetroRule, default_rules, load_rules_from_file, mol_from_smiles};
use renkin::search::{Route, SearchConfig, find_routes};
use rustc_hash::FxHashSet;
use serde::Serialize;

// ── PaRoutes JSON helpers ────────────────────────────────────────────────────

/// Parse a PaRoutes-format JSON file into (smiles, name, gt_depth) tuples.
/// Each entry is a route tree rooted at the target molecule.
fn parse_paroutes(path: &str) -> Result<Vec<(String, String, Option<u32>)>> {
    let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    let arr = json
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("PaRoutes JSON: expected top-level array"))?;
    Ok(arr
        .iter()
        .enumerate()
        .map(|(i, node)| {
            let smiles = node["smiles"].as_str().unwrap_or("").to_string();
            let gt_depth = count_reactions(node);
            (smiles, format!("paroutes_{i}"), Some(gt_depth))
        })
        .collect())
}

/// Count the maximum reaction-node depth in a PaRoutes route tree.
/// mol/reaction nodes alternate, so reaction count == synthesis step count.
fn count_reactions(node: &serde_json::Value) -> u32 {
    node.get("children")
        .and_then(|c| c.as_array())
        .map(|kids| {
            kids.iter()
                .map(|k| {
                    let is_rxn = k.get("type").and_then(|t| t.as_str()) == Some("reaction");
                    is_rxn as u32 + count_reactions(k)
                })
                .max()
                .unwrap_or(0)
        })
        .unwrap_or(0)
}

// ── Route diversity ──────────────────────────────────────────────────────────

/// 1 - avg pairwise Jaccard similarity of building-block sets across routes.
/// Returns 0.0 when fewer than 2 routes are available.
fn route_diversity(routes: &[Route]) -> f64 {
    if routes.len() < 2 {
        return 0.0;
    }
    let mut total_sim = 0.0;
    let mut count = 0usize;
    for i in 0..routes.len() {
        for j in (i + 1)..routes.len() {
            let a: FxHashSet<&str> = routes[i]
                .building_blocks
                .iter()
                .map(|s| s.as_str())
                .collect();
            let b: FxHashSet<&str> = routes[j]
                .building_blocks
                .iter()
                .map(|s| s.as_str())
                .collect();
            let inter = a.intersection(&b).count();
            let union = a.len() + b.len() - inter;
            total_sim += if union == 0 {
                1.0
            } else {
                inter as f64 / union as f64
            };
            count += 1;
        }
    }
    1.0 - (total_sim / count as f64)
}

// ── Atom balance ─────────────────────────────────────────────────────────────

/// True if target_MW ≤ Σ precursor_MW (within 1% float tolerance).
/// In retrosynthesis the target is split from precursors; precursors must
/// carry at least as many atoms (by weight) as the target. Violation means
/// a template caused atoms to appear from nowhere — a CompleteRXN-style defect.
fn step_balanced(target: &str, precursors: &[String]) -> bool {
    let target_mw = mol_from_smiles(target)
        .ok()
        .map(|m| molecular_weight(&m))
        .unwrap_or(0.0);
    if target_mw == 0.0 {
        return true;
    }
    let precursor_mw: f64 = precursors
        .iter()
        .filter_map(|s| mol_from_smiles(s).ok())
        .map(|m| molecular_weight(&m))
        .sum();
    target_mw <= precursor_mw * 1.01
}

fn route_balanced(route: &Route) -> bool {
    route
        .steps
        .iter()
        .all(|s| step_balanced(&s.target, &s.precursors))
}

// ── Output structs ───────────────────────────────────────────────────────────

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
    /// Route diversity ∈ [0, 1] across returned routes (None when routes_found < 2).
    #[serde(skip_serializing_if = "Option::is_none")]
    route_diversity: Option<f64>,
    /// Ground-truth synthesis depth from PaRoutes (None in smi mode).
    #[serde(skip_serializing_if = "Option::is_none")]
    gt_depth: Option<u32>,
    /// best_depth - gt_depth (None unless both are present).
    #[serde(skip_serializing_if = "Option::is_none")]
    depth_delta: Option<i32>,
    /// True if every step of the best route satisfies target_MW ≤ Σ precursor_MW.
    /// None when no routes found. Flags templates that cause atoms to appear from nowhere.
    #[serde(skip_serializing_if = "Option::is_none")]
    atom_balance_ok: Option<bool>,
    /// True if every step passes forward validation (precursors → target confirmed).
    /// None when --plausibility not set or no routes found.
    #[serde(skip_serializing_if = "Option::is_none")]
    forward_validated: Option<bool>,
    /// True if any step uses a low-frequency template (step_confidence < 0.1).
    #[serde(skip_serializing_if = "Option::is_none")]
    low_template_confidence: Option<bool>,
}

#[derive(Serialize)]
struct BenchReport {
    total: usize,
    solved: usize,
    success_rate: f64,
    avg_depth: f64,
    avg_time_ms: f64,
    avg_nodes_expanded: f64,
    avg_confidence: f64,
    avg_convergency: f64,
    avg_success_prob: f64,
    avg_route_cost: f64,
    /// Average route diversity over targets with ≥2 routes.
    avg_route_diversity: f64,
    /// Average (renkin_depth - gt_depth) over solved targets; 0.0 in smi mode.
    avg_depth_delta: f64,
    /// Percentage of solved targets where the best route passes atom balance check.
    pct_atom_balanced: f64,
    /// Percentage of solved targets where every step passes forward validation.
    /// None when --plausibility not set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pct_forward_validated: Option<f64>,
    /// Percentage of solved targets where ≥1 step uses a low-frequency template (confidence < 0.1).
    pct_low_template_confidence: f64,
    /// Composite plausibility score ∈ [0, 1]: mean of (atom_balance + fwd_validated + high_confidence).
    /// None when --plausibility not set.
    #[serde(skip_serializing_if = "Option::is_none")]
    plausibility_score: Option<f64>,
    results: Vec<BenchResult>,
}

// ── quietset export ──────────────────────────────────────────────────────────

#[derive(Serialize)]
struct QuietsetObs {
    sample_id: String,
    label: &'static str,
    score: f64,
    evaluator_id: String,
    budget: usize,
    seed: u32,
}

// ── compare subcommand ───────────────────────────────────────────────────────

fn cmd_compare(paths: &[String]) -> Result<()> {
    if paths.len() < 2 {
        bail!("Usage: renkin-bench compare <baseline.json> <current.json>");
    }
    let base: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&paths[0])?)?;
    let curr: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&paths[1])?)?;

    let base_rate = base["success_rate"].as_f64().unwrap_or(0.0) * 100.0;
    let curr_rate = curr["success_rate"].as_f64().unwrap_or(0.0) * 100.0;
    let delta = curr_rate - base_rate;
    let sign = if delta >= 0.0 { "+" } else { "" };

    let base_time = base["avg_time_ms"].as_f64().unwrap_or(0.0);
    let curr_time = curr["avg_time_ms"].as_f64().unwrap_or(0.0);
    let time_delta = curr_time - base_time;
    let time_sign = if time_delta >= 0.0 { "+" } else { "" };

    // Build solved-state maps keyed by name (fall back to smiles)
    let solved_map = |report: &serde_json::Value| -> std::collections::HashMap<String, bool> {
        report["results"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|r| {
                        let key = r["name"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .unwrap_or_else(|| r["smiles"].as_str().unwrap_or(""))
                            .to_string();
                        let solved = r["solved"].as_bool().unwrap_or(false);
                        (key, solved)
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    let base_map = solved_map(&base);
    let curr_map = solved_map(&curr);

    let mut gained: Vec<&str> = Vec::new();
    let mut lost: Vec<&str> = Vec::new();
    for (name, &now) in &curr_map {
        match base_map.get(name) {
            Some(&before) if !before && now => gained.push(name),
            Some(&before) if before && !now => lost.push(name),
            _ => {}
        }
    }
    gained.sort_unstable();
    lost.sort_unstable();

    println!("=== renkin-bench compare ===");
    println!("Baseline : {}  ({:.1}%)", paths[0], base_rate);
    println!("Current  : {}  ({:.1}%)", paths[1], curr_rate);
    println!("Delta    : {}{:.1} pp", sign, delta);
    println!();
    println!(
        "Timing   : {:.1} ms → {:.1} ms  ({}{:.1} ms)",
        base_time, curr_time, time_sign, time_delta
    );
    println!();

    if gained.is_empty() {
        println!("Newly solved (0): (none)");
    } else {
        println!("Newly solved ({}):", gained.len());
        for name in &gained {
            println!("  + {name}");
        }
    }
    println!();
    if lost.is_empty() {
        println!("Regressions (0): (none)");
    } else {
        println!("Regressions ({}):", lost.len());
        for name in &lost {
            println!("  - {name}");
        }
    }
    Ok(())
}

// ..Default::default() is needed when nn-scoring feature is enabled (adds nn_scorer field).
// When the feature is off, all fields are explicit, making the spread redundant — suppress lint.
#[allow(clippy::needless_update)]
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.get(1).map(|s| s.as_str()) == Some("compare") {
        return cmd_compare(&args[2..]);
    }

    let mut input_path: Option<String> = None;
    let mut input_format = "smi".to_string();
    let mut bb_path: Option<String> = None;
    let mut templates_path: Option<String> = None;
    let mut max_depth: u32 = 5;
    let mut beam_width: usize = 0;
    let mut max_routes: usize = 1;
    let mut bond_index = false;
    let mut plausibility = false;
    let mut quietset_out: Option<String> = None;
    let mut evaluator_id: Option<String> = None;
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
            "--input-format" => {
                i += 1;
                if i < args.len() {
                    input_format = args[i].clone();
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
            "--plausibility" => {
                plausibility = true;
            }
            "--quietset-out" => {
                i += 1;
                quietset_out = args.get(i).cloned();
            }
            "--evaluator-id" => {
                i += 1;
                evaluator_id = args.get(i).cloned();
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
            "Usage: renkin-bench --input <smiles_file|paroutes.json> \
             [--input-format smi|paroutes] [--depth <N>] \
             [--beam-width <N>] [--building-blocks <path>] [--templates <path>] \
             [--scorer <onnx_path>]"
        );
    };

    // Parse targets depending on format
    let targets: Vec<(String, String, Option<u32>)> = if input_format == "paroutes" {
        parse_paroutes(&input)?
    } else {
        std::fs::read_to_string(&input)?
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|line| {
                let mut parts = line.splitn(2, char::is_whitespace);
                let smiles = parts.next().unwrap_or("").to_string();
                let name = parts.next().unwrap_or("").trim().to_string();
                (smiles, name, None)
            })
            .collect()
    };

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
        "Benchmarking {} targets (format={}, depth={}, beam_width={}) ...",
        targets.len(),
        input_format,
        max_depth,
        beam_width
    );

    let mut results = Vec::new();
    let mut total_depth_sum = 0u32;
    let mut solved_count = 0usize;

    for (smiles, name, gt_depth) in &targets {
        let t0 = Instant::now();
        let (routes, stats) = find_routes(smiles, &env, &rules, &config).unwrap_or_default();
        let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;

        let solved = !routes.is_empty();
        let best_depth = routes.iter().map(|r| r.depth).min();
        let best_confidence = routes.first().map(|r| r.confidence);
        let best_success_prob = routes.first().map(|r| r.success_probability);
        let best_convergency = routes.first().map(|r| r.convergency);
        let best_route_cost = routes.first().map(|r| r.route_cost);
        let diversity = if routes.len() >= 2 {
            Some(route_diversity(&routes))
        } else {
            None
        };
        let depth_delta = match (best_depth, gt_depth) {
            (Some(bd), Some(gd)) => Some(bd as i32 - *gd as i32),
            _ => None,
        };
        let atom_balance_ok = routes.first().map(route_balanced);
        let forward_validated = if plausibility {
            routes.first().map(|r| route_forward_validated(r, &rules))
        } else {
            None
        };
        let low_template_confidence = routes.first().map(route_low_confidence);

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
            route_diversity: diversity,
            gt_depth: *gt_depth,
            depth_delta,
            atom_balance_ok,
            forward_validated,
            low_template_confidence,
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
    let avg_confidence = avg_opt(&solved_results, |r| r.best_confidence);
    let avg_convergency = avg_opt(&solved_results, |r| r.best_convergency);
    let avg_success_prob = avg_opt(&solved_results, |r| r.best_success_prob);
    let avg_route_cost = avg_opt(&solved_results, |r| r.best_route_cost);

    let diversity_results: Vec<&BenchResult> = results
        .iter()
        .filter(|r| r.route_diversity.is_some())
        .collect();
    let avg_route_diversity = avg_opt(&diversity_results, |r| r.route_diversity);

    let delta_results: Vec<&BenchResult> = solved_results
        .iter()
        .filter(|r| r.depth_delta.is_some())
        .copied()
        .collect();
    let avg_depth_delta = if delta_results.is_empty() {
        0.0
    } else {
        delta_results
            .iter()
            .filter_map(|r| r.depth_delta)
            .map(|d| d as f64)
            .sum::<f64>()
            / delta_results.len() as f64
    };

    let n_balanced = solved_results
        .iter()
        .filter(|r| r.atom_balance_ok == Some(true))
        .count();
    let pct_atom_balanced = if solved_count > 0 {
        n_balanced as f64 / solved_count as f64 * 100.0
    } else {
        0.0
    };

    let n_fwd_validated = solved_results
        .iter()
        .filter(|r| r.forward_validated == Some(true))
        .count();
    let pct_forward_validated = if plausibility && solved_count > 0 {
        Some(n_fwd_validated as f64 / solved_count as f64 * 100.0)
    } else {
        None
    };
    let n_low_conf = solved_results
        .iter()
        .filter(|r| r.low_template_confidence == Some(true))
        .count();
    let pct_low_template_confidence = if solved_count > 0 {
        n_low_conf as f64 / solved_count as f64 * 100.0
    } else {
        0.0
    };
    let plausibility_score = pct_forward_validated.map(|fv| {
        (pct_atom_balanced / 100.0 + fv / 100.0 + (100.0 - pct_low_template_confidence) / 100.0)
            / 3.0
    });

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
        avg_route_diversity,
        avg_depth_delta,
        pct_atom_balanced,
        pct_forward_validated,
        pct_low_template_confidence,
        plausibility_score,
        results,
    };

    println!("{}", serde_json::to_string_pretty(&report)?);

    if let Some(path) = quietset_out {
        let eid = evaluator_id.unwrap_or_else(|| format!("renkin-d{max_depth}-b{beam_width}"));
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        let mut w = std::io::BufWriter::new(file);
        for r in &report.results {
            let obs = QuietsetObs {
                sample_id: r.name.clone(),
                label: if r.solved { "solved" } else { "unsolved" },
                score: r.best_success_prob.unwrap_or(0.0),
                evaluator_id: eid.clone(),
                budget: beam_width,
                seed: 1,
            };
            writeln!(w, "{}", serde_json::to_string(&obs)?)?;
        }
    }

    Ok(())
}

// ── Plausibility checks ──────────────────────────────────────────────────────

/// True if every step of the route passes forward validation:
/// applying each step's precursors forward reproduces the step's target.
fn route_forward_validated(route: &Route, rules: &[RetroRule]) -> bool {
    route.steps.iter().all(|step| {
        let Ok(reactant_mols): Result<Vec<_>, _> =
            step.precursors.iter().map(|s| mol_from_smiles(s)).collect()
        else {
            return false;
        };
        let Ok(target_mol) = mol_from_smiles(&step.target) else {
            return false;
        };
        let target_canon = canonical_smiles(&target_mol);
        let mol_refs: Vec<_> = reactant_mols.iter().collect();
        rules.iter().filter(|r| !r.smirks.is_empty()).any(|rule| {
            let Some((lhs, rhs)) = rule.smirks.split_once(">>") else {
                return false;
            };
            let fwd = format!("{rhs}>>{lhs}");
            run_reactants(&fwd, &mol_refs)
                .into_iter()
                .flatten()
                .flatten()
                .any(|m| canonical_smiles(&m) == target_canon)
        })
    })
}

/// True if any step uses a template with step_confidence < 0.1 (rare template).
fn route_low_confidence(route: &Route) -> bool {
    route.steps.iter().any(|s| s.step_confidence < 0.1)
}

fn avg_opt(rows: &[&BenchResult], f: impl Fn(&BenchResult) -> Option<f64>) -> f64 {
    if rows.is_empty() {
        return 0.0;
    }
    let vals: Vec<f64> = rows.iter().filter_map(|r| f(r)).collect();
    if vals.is_empty() {
        0.0
    } else {
        vals.iter().sum::<f64>() / vals.len() as f64
    }
}
