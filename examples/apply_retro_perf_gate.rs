//! Reproducible apply_retro/run_reactants performance-regression gate.
//!
//! Runs a fixed target corpus through `find_routes` with a pinned config and
//! reports, per target: elapsed time, `apply_retro` call count (always
//! available), `run_reactants`-level counters (only when built with
//! `--features perf-instrumentation`, else zero), route/candidate outcome, and
//! any error. Also emits SHA-256 provenance for every input file plus run
//! metadata (chematic pin, release/thread/OS info, warmup/measured counts) so
//! two runs (e.g. baseline vs. fix branch) can be compared without ambiguity
//! about what was actually measured.
//!
//! Not a pass/fail CI check (wall-clock varies run to run and machine to
//! machine) -- a deterministic data generator for a human/PR-body comparison.
//! See `artifacts/perf_root_cause/` for the investigation this gate exists to
//! re-run, and `next_gate.json` for one prior gate-criteria proposal.
//!
//! Usage:
//!   cargo run --release --example apply_retro_perf_gate -- \
//!     --targets artifacts/perf_root_cause/gate_inputs/probe_30_v2.smi \
//!     --templates data/templates_extracted_5000.smi \
//!     --stock data/building_blocks.smi \
//!     --depth 5 --beam-width 100 --warmup 1 --label "fix-branch"
//!
//! With run_reactants-level counters:
//!   cargo run --release --features perf-instrumentation --example apply_retro_perf_gate -- ...

use std::time::Instant;

use anyhow::{Context, Result, bail};
use renkin::chem_env::{
    ChemEnv, apply_retro_call_count, load_rules_from_file, reset_apply_retro_call_count,
};
use renkin::search::{SearchConfig, find_routes};
use sha2::{Digest, Sha256};

fn sha256_file(path: &str) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {path}"))?;
    Ok(format!("{:x}", Sha256::digest(&bytes)))
}

#[cfg(feature = "perf-instrumentation")]
fn run_reactants_calls_delta() -> u64 {
    chematic_rxn::perf_counters::snapshot().run_reactants_calls
}
#[cfg(not(feature = "perf-instrumentation"))]
fn run_reactants_calls_delta() -> u64 {
    0
}
#[cfg(feature = "perf-instrumentation")]
fn reset_run_reactants_calls() {
    chematic_rxn::perf_counters::reset();
}
#[cfg(not(feature = "perf-instrumentation"))]
fn reset_run_reactants_calls() {}

struct Args {
    targets: String,
    templates: String,
    stock: String,
    depth: u32,
    beam_width: usize,
    warmup: usize,
    label: String,
}

fn parse_args() -> Result<Args> {
    let raw: Vec<String> = std::env::args().collect();
    let mut a = Args {
        targets: String::new(),
        templates: String::new(),
        stock: String::new(),
        depth: 5,
        beam_width: 100,
        warmup: 1,
        label: "unlabeled".to_string(),
    };
    let mut i = 1;
    while i < raw.len() {
        match raw[i].as_str() {
            "--targets" => {
                a.targets = raw.get(i + 1).cloned().unwrap_or_default();
                i += 2;
            }
            "--templates" => {
                a.templates = raw.get(i + 1).cloned().unwrap_or_default();
                i += 2;
            }
            "--stock" => {
                a.stock = raw.get(i + 1).cloned().unwrap_or_default();
                i += 2;
            }
            "--depth" => {
                a.depth = raw.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(5);
                i += 2;
            }
            "--beam-width" => {
                a.beam_width = raw.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(100);
                i += 2;
            }
            "--warmup" => {
                a.warmup = raw.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(1);
                i += 2;
            }
            "--label" => {
                a.label = raw.get(i + 1).cloned().unwrap_or_default();
                i += 2;
            }
            other => bail!("unknown argument: {other}"),
        }
    }
    if a.targets.is_empty() || a.templates.is_empty() || a.stock.is_empty() {
        bail!("--targets, --templates, and --stock are all required");
    }
    Ok(a)
}

fn main() -> Result<()> {
    let args = parse_args()?;

    let targets_sha256 = sha256_file(&args.targets)?;
    let templates_sha256 = sha256_file(&args.templates)?;
    let stock_sha256 = sha256_file(&args.stock)?;
    let cargo_lock_sha256 = sha256_file(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.lock"))?;
    let cargo_lock = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.lock"))?;
    let chematic_pin = cargo_lock
        .lines()
        .skip_while(|l| *l != "name = \"chematic\"")
        .find(|l| l.starts_with("source ="))
        .unwrap_or("source = \"unknown\"")
        .trim()
        .to_string();

    let env = ChemEnv::load(&args.stock)?;
    let stock_compound_count = env.bb_count();
    // ChemEnv::load reads exactly the path given (no fallback logic in this
    // crate) -- an explicit stock path was always used, so this is a fact
    // about this run, not an assumption.
    let embedded_fallback_used = false;

    let rules = load_rules_from_file(&args.templates);
    if rules.is_empty() {
        bail!(
            "loaded 0 templates from {} -- refusing to run a gate against an empty rule set",
            args.templates
        );
    }

    let targets: Vec<String> = std::fs::read_to_string(&args.targets)?
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.split_whitespace().next().unwrap_or(l).to_string())
        .collect();

    let config = SearchConfig {
        max_depth: args.depth,
        max_routes: 5,
        beam_width: args.beam_width,
        verbose: false,
        ..Default::default()
    };

    let thread_count = std::env::var("RAYON_NUM_THREADS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1)
        });

    // Warmup (not recorded): touches page cache / lazy-initialized state so the
    // first measured target isn't penalized for one-time setup cost.
    for smiles in targets.iter().take(args.warmup) {
        let _ = find_routes(smiles, &env, &rules, &config);
    }

    let mut per_target = Vec::with_capacity(targets.len());
    let overall_start = Instant::now();
    for (idx, smiles) in targets.iter().enumerate() {
        reset_apply_retro_call_count();
        reset_run_reactants_calls();
        let t0 = Instant::now();
        let outcome = find_routes(smiles, &env, &rules, &config);
        let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
        let apply_retro_calls = apply_retro_call_count();
        let run_reactants_calls = run_reactants_calls_delta();

        let record = match outcome {
            Ok((routes, stats)) => serde_json::json!({
                "index": idx,
                "target": smiles,
                "elapsed_ms": elapsed_ms,
                "apply_retro_calls": apply_retro_calls,
                "run_reactants_calls": run_reactants_calls,
                "matched_templates": stats.matched_templates,
                "retro_cache_hits": stats.retro_cache_hits,
                "retro_cache_misses": stats.retro_cache_misses,
                "nodes_expanded": stats.nodes_expanded,
                "routes_found": routes.len(),
                "solved": !routes.is_empty(),
                "best_route_depth": routes.first().map(|r| r.depth),
                "best_route_building_blocks": routes.first().map(|r| r.building_blocks.clone()),
                "error": null,
            }),
            Err(e) => serde_json::json!({
                "index": idx,
                "target": smiles,
                "elapsed_ms": elapsed_ms,
                "apply_retro_calls": apply_retro_calls,
                "run_reactants_calls": run_reactants_calls,
                "matched_templates": null,
                "retro_cache_hits": null,
                "retro_cache_misses": null,
                "nodes_expanded": null,
                "routes_found": 0,
                "solved": false,
                "best_route_depth": null,
                "best_route_building_blocks": null,
                "error": e.to_string(),
            }),
        };
        per_target.push(record);
    }
    let total_elapsed_s = overall_start.elapsed().as_secs_f64();

    let solved_count = per_target.iter().filter(|r| r["solved"] == true).count();
    let mut elapsed_ms: Vec<f64> = per_target
        .iter()
        .map(|r| r["elapsed_ms"].as_f64().unwrap_or(0.0))
        .collect();
    elapsed_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pct = |p: f64| -> f64 {
        if elapsed_ms.is_empty() {
            return 0.0;
        }
        let idx = ((elapsed_ms.len() as f64 - 1.0) * p).round() as usize;
        elapsed_ms[idx.min(elapsed_ms.len() - 1)]
    };

    let report = serde_json::json!({
        "label": args.label,
        "run_metadata": {
            "chematic_pin": chematic_pin,
            "cargo_lock_sha256": cargo_lock_sha256,
            "targets_file": args.targets,
            "targets_sha256": targets_sha256,
            "templates_file": args.templates,
            "templates_sha256": templates_sha256,
            "stock_file": args.stock,
            "stock_sha256": stock_sha256,
            "stock_compound_count": stock_compound_count,
            "embedded_fallback_used": embedded_fallback_used,
            "rules_loaded": rules.len(),
            "release_mode": !cfg!(debug_assertions),
            "thread_count": thread_count,
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "depth": args.depth,
            "beam_width": args.beam_width,
            "warmup_runs": args.warmup,
            "measured_runs": targets.len(),
        },
        "aggregate": {
            "total_elapsed_s": total_elapsed_s,
            "solved_count": solved_count,
            "target_count": targets.len(),
            "elapsed_ms_p50": pct(0.50),
            "elapsed_ms_p90": pct(0.90),
            "elapsed_ms_p95": pct(0.95),
            "elapsed_ms_max": elapsed_ms.last().copied().unwrap_or(0.0),
        },
        "per_target": per_target,
    });

    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
