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
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use renkin::DEFAULT_BUILDING_BLOCKS;
use renkin::chem_env::{ChemEnv, default_rules, load_rules_from_file};
use renkin::search::{Route, SearchConfig, exploration_contract, find_routes};
use renkin::validation::{
    RouteValidationStatus, StepValidationStatus, route_balanced, validate_route_steps,
};
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
    /// None when --plausibility not set or no routes found. Equivalent to
    /// `route_validation_status == Some("validated")`.
    #[serde(skip_serializing_if = "Option::is_none")]
    forward_validated: Option<bool>,
    /// Three-valued forward-validation rollup for the best route's steps:
    /// "validated" (all steps confirmed), "invalid" (≥1 step confirmed wrong),
    /// "partially_validated" (mix of confirmed and not-evaluable), or
    /// "not_evaluable" (no step could be checked). None when --plausibility
    /// not set or no routes found. See `renkin::validation` for why this
    /// replaces the old binary forward_validated as the source of truth:
    /// 8 graph-based rules (ester/amide/Suzuki/sulfonamide/sulfone/Boc/Cbz/
    /// aryl-ether) have no SMIRKS to reverse-apply and need a separate
    /// structural check, so "couldn't be checked" and "checked and wrong"
    /// must stay distinct.
    #[serde(skip_serializing_if = "Option::is_none")]
    route_validation_status: Option<String>,
    /// True if any step uses a low-frequency template (step_confidence < 0.1).
    #[serde(skip_serializing_if = "Option::is_none")]
    low_template_confidence: Option<bool>,
    // ── Failure diagnostics (always present; useful for taxonomy) ──
    beam_limit_hit: bool,
    max_depth_reached: bool,
    matched_templates: u64,
    stock_hits: u64,
    /// Phase D: retro_cache hits/misses for this target. With `--scorer` set,
    /// misses == ONNX inference calls (one per unique canonical intermediate;
    /// see `nn_rank` in search.rs), hits == reuses that needed no inference.
    retro_cache_hits: u64,
    retro_cache_misses: u64,
    /// Coverage-mode observability; omitted in standard mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    coverage_selected_stage: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    coverage_stage2_invoked: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    coverage_stage1_timeout: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    coverage_stage2_timeout: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    coverage_stage2_elapsed_ms: Option<f64>,
}

#[derive(Serialize)]
struct BenchReport {
    /// Component boundary contract used by this benchmark run.
    exploration_contract: renkin::search::ExplorationContract,
    /// Search strategy selected by `beam_width` (`a_star` or `beam`).
    search_strategy: &'static str,
    /// Orchestration mode (`standard` or staged `coverage`).
    search_mode: String,
    search_depth: u32,
    search_beam_width: usize,
    coverage_template_count: Option<usize>,
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
    /// Fraction of ALL targets solved (= success_rate). Mirror with explicit name for the metric trio.
    raw_solved_rate: f64,
    /// Fraction of ALL targets that are solved AND pass forward validation.
    /// None when --plausibility not set. Always ≤ raw_solved_rate. Equal to
    /// strict_validated_solved_rate — kept as a separate field for JSON
    /// compatibility with pre-tri-state consumers.
    #[serde(skip_serializing_if = "Option::is_none")]
    validated_solved_rate: Option<f64>,
    /// Fraction of ALL targets whose best route has route_validation_status ==
    /// "validated" (every step confirmed, none invalid or not-evaluable).
    /// None when --plausibility not set. Numerically equal to validated_solved_rate;
    /// the explicit name distinguishes it from evaluable_validation_pass_rate.
    #[serde(skip_serializing_if = "Option::is_none")]
    strict_validated_solved_rate: Option<f64>,
    /// Fraction of all steps (across solved targets' best routes) whose status
    /// was Valid or Invalid (i.e. NOT NotEvaluable) — how much of the route
    /// set a validation method could even render a verdict on. Low coverage
    /// means low pct_forward_validated reflects validator blind spots more
    /// than actual chemistry quality. None when --plausibility not set.
    #[serde(skip_serializing_if = "Option::is_none")]
    validation_coverage: Option<f64>,
    /// Fraction of evaluable steps (status Valid or Invalid) that were Valid.
    /// Unlike pct_forward_validated / validated_solved_rate, the denominator
    /// excludes NotEvaluable steps — this isolates validator *accuracy* on the
    /// steps it can actually judge from validator *coverage* (see
    /// validation_coverage). None when --plausibility not set or no step was
    /// evaluable.
    #[serde(skip_serializing_if = "Option::is_none")]
    evaluable_validation_pass_rate: Option<f64>,
    /// Fraction of ALL targets that are solved AND have best_depth ≤ --practical-max-steps.
    /// None when --practical-max-steps not set. Always ≤ raw_solved_rate.
    #[serde(skip_serializing_if = "Option::is_none")]
    practical_solved_rate: Option<f64>,
    /// Phase D: sum of per-target retro_cache_hits/misses. With `--scorer` set,
    /// total_retro_cache_misses is the total ONNX inference call count for the
    /// whole run; hit_rate = hits / (hits + misses).
    total_retro_cache_hits: u64,
    total_retro_cache_misses: u64,
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

/// Per-target identity keys for a `renkin-bench` JSON report, in result order.
///
/// `name` is NOT usable as an identity key: every USPTO-50k target has
/// `name == "UNK"` (verified — every data row in data/uspto50k_test.smi has
/// this), so keying on name collapses all targets into one "UNK" bucket and
/// silently discards per-target deltas (found during the Phase 31 PR #32
/// harness audit: a 100-target sample with 12 real regressions reported 0).
///
/// Identity precedence: canonical SMILES (unique for all but a handful of
/// targets in this dataset — a few duplicate molecules do occur), falling
/// back to `#<row index>:<smiles>` for any SMILES that repeats within a
/// single report, so those targets still get distinct keys.
fn target_identity_keys(report: &serde_json::Value) -> Result<Vec<String>> {
    let arr = report["results"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("'results' array missing or not an array"))?;
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for r in arr {
        *counts
            .entry(r["smiles"].as_str().unwrap_or(""))
            .or_insert(0) += 1;
    }
    Ok(arr
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let smiles = r["smiles"].as_str().unwrap_or("");
            if smiles.is_empty() || counts[smiles] > 1 {
                format!("#{i}:{smiles}")
            } else {
                smiles.to_string()
            }
        })
        .collect())
}

/// Diff two `renkin-bench` JSON reports' per-target solved state.
///
/// Fails loudly (instead of silently comparing misaligned data) if the two
/// runs don't cover the same target set: different target counts, or a
/// target present in one but not the other.
///
/// Returns `(gained, lost)`: identity keys newly solved / newly unsolved in
/// `curr` relative to `base`, both sorted.
fn diff_solved_sets(
    base: &serde_json::Value,
    curr: &serde_json::Value,
) -> Result<(Vec<String>, Vec<String>)> {
    let base_arr = base["results"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("baseline: 'results' array missing or not an array"))?;
    let curr_arr = curr["results"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("current: 'results' array missing or not an array"))?;

    if base_arr.len() != curr_arr.len() {
        bail!(
            "target-set mismatch: baseline has {} target(s), current has {} target(s) — \
             refusing to compare misaligned runs",
            base_arr.len(),
            curr_arr.len()
        );
    }

    let base_keys = target_identity_keys(base)?;
    let curr_keys = target_identity_keys(curr)?;

    let base_set: FxHashSet<&str> = base_keys.iter().map(String::as_str).collect();
    let curr_set: FxHashSet<&str> = curr_keys.iter().map(String::as_str).collect();
    let mut only_in_base: Vec<&str> = base_set.difference(&curr_set).copied().collect();
    let mut only_in_curr: Vec<&str> = curr_set.difference(&base_set).copied().collect();
    if !only_in_base.is_empty() || !only_in_curr.is_empty() {
        only_in_base.sort_unstable();
        only_in_curr.sort_unstable();
        bail!(
            "target-set mismatch: {} target(s) only in baseline, {} target(s) only in current \
             (e.g. baseline-only={:?}, current-only={:?}) — refusing to compare misaligned runs",
            only_in_base.len(),
            only_in_curr.len(),
            only_in_base.iter().take(3).collect::<Vec<_>>(),
            only_in_curr.iter().take(3).collect::<Vec<_>>()
        );
    }

    let base_map: std::collections::HashMap<&str, bool> = base_keys
        .iter()
        .zip(base_arr.iter())
        .map(|(k, r)| (k.as_str(), r["solved"].as_bool().unwrap_or(false)))
        .collect();
    let curr_map: std::collections::HashMap<&str, bool> = curr_keys
        .iter()
        .zip(curr_arr.iter())
        .map(|(k, r)| (k.as_str(), r["solved"].as_bool().unwrap_or(false)))
        .collect();

    let mut gained: Vec<String> = Vec::new();
    let mut lost: Vec<String> = Vec::new();
    for (&key, &now) in &curr_map {
        match base_map.get(key) {
            Some(&before) if !before && now => gained.push(key.to_string()),
            Some(&before) if before && !now => lost.push(key.to_string()),
            _ => {}
        }
    }
    gained.sort_unstable();
    lost.sort_unstable();
    Ok((gained, lost))
}

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

    let (gained, lost) = diff_solved_sets(&base, &curr)?;

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
        for key in &gained {
            println!("  + {key}");
        }
    }
    println!();
    if lost.is_empty() {
        println!("Regressions (0): (none)");
    } else {
        println!("Regressions ({}):", lost.len());
        for key in &lost {
            println!("  - {key}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod compare_tests {
    use super::*;
    use serde_json::json;

    /// Build a minimal renkin-bench report with `name` == "UNK" for every
    /// target, mirroring the real USPTO-50k data (all rows have name=UNK).
    fn report(entries: &[(&str, bool)]) -> serde_json::Value {
        let results: Vec<_> = entries
            .iter()
            .map(|(smiles, solved)| json!({"smiles": smiles, "name": "UNK", "solved": solved}))
            .collect();
        json!({ "success_rate": 0.0, "avg_time_ms": 0.0, "results": results })
    }

    /// Pins the dedup-key bug: all targets share name="UNK", and two targets
    /// have genuinely different before/after outcomes (one newly solved, one
    /// newly a regression). Keying on `name` alone collapses everything into
    /// one "UNK" bucket and both deltas vanish. On unfixed code this test
    /// fails (gained/lost end up empty or wrong); after the fix it passes.
    #[test]
    fn dedup_key_bug_is_fixed() {
        let base = report(&[
            ("CCO", true),  // unchanged: solved -> solved
            ("CCN", false), // regression: unsolved -> solved is NOT this one
            ("CCC", true),  // regression: solved -> unsolved
            ("CCF", false), // unchanged: unsolved -> unsolved
        ]);
        let curr = report(&[
            ("CCO", true),
            ("CCN", true),  // gained
            ("CCC", false), // lost
            ("CCF", false),
        ]);

        let (gained, lost) = diff_solved_sets(&base, &curr).expect("reports should diff cleanly");
        assert_eq!(gained, vec!["CCN".to_string()]);
        assert_eq!(lost, vec!["CCC".to_string()]);
    }

    #[test]
    fn duplicate_smiles_within_a_report_still_get_distinct_keys() {
        // Same SMILES appears twice in each report (as happens ~4x in the
        // real USPTO-50k test set) with different outcomes at each position.
        let base = report(&[("CCO", true), ("CCO", false)]);
        let curr = report(&[("CCO", true), ("CCO", true)]);

        let (gained, lost) = diff_solved_sets(&base, &curr).expect("reports should diff cleanly");
        // Only the second occurrence (index 1) changed outcome.
        assert_eq!(gained, vec!["#1:CCO".to_string()]);
        assert!(lost.is_empty());
    }

    #[test]
    fn mismatched_target_counts_fail_loudly() {
        let base = report(&[("CCO", true), ("CCN", false)]);
        let curr = report(&[("CCO", true)]);

        let err = diff_solved_sets(&base, &curr).expect_err("count mismatch must error");
        assert!(err.to_string().contains("target-set mismatch"));
    }

    #[test]
    fn mismatched_target_identity_fails_loudly() {
        // Same count, but current swapped one target for a different molecule.
        let base = report(&[("CCO", true), ("CCN", false)]);
        let curr = report(&[("CCO", true), ("CCC", false)]);

        let err = diff_solved_sets(&base, &curr).expect_err("identity mismatch must error");
        assert!(err.to_string().contains("target-set mismatch"));
    }
}

// ── cascade subcommand ───────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct StageConfig {
    name: String,
    depth: u32,
    beam_width: usize,
    #[serde(default)]
    templates: Option<String>,
    #[serde(default)]
    top_templates: Option<usize>,
    #[serde(default)]
    building_blocks: Option<String>,
    #[serde(default)]
    only_unsolved_from_previous: bool,
}

#[derive(Serialize)]
struct StageResult {
    name: String,
    attempted: usize,
    newly_solved: usize,
    cumulative_solved: usize,
}

#[derive(Serialize)]
struct CascadeReport {
    total: usize,
    stages: Vec<StageResult>,
    total_solved: usize,
    raw_solved_rate: f64,
    /// Fraction of solved targets whose best route passes atom balance (cheap, always computed).
    atom_balance_pass_rate: f64,
    /// Fraction of solved targets whose best route passes forward validation.
    /// None unless --quality is set (forward validation is O(steps × templates), slow).
    #[serde(skip_serializing_if = "Option::is_none")]
    forward_validation_pass_rate: Option<f64>,
}

fn cmd_cascade(args: &[String]) -> Result<()> {
    let mut input_path: Option<String> = None;
    let mut stage_paths: Vec<String> = Vec::new();
    let mut quality = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--input" | "-i" => {
                i += 1;
                input_path = args.get(i).cloned();
            }
            "--stage" => {
                i += 1;
                if let Some(p) = args.get(i) {
                    stage_paths.push(p.clone());
                }
            }
            "--quality" => {
                quality = true;
            }
            _ => {}
        }
        i += 1;
    }

    let input = input_path.ok_or_else(|| {
        anyhow::anyhow!(
            "Usage: renkin-bench cascade --input <smi> --stage <cfg.json> [--stage ...]"
        )
    })?;

    // Parse all targets once.
    let all_targets: Vec<(String, String)> = std::fs::read_to_string(&input)?
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

    let total = all_targets.len();
    if total == 0 {
        bail!("No targets found in {input}");
    }

    if stage_paths.is_empty() {
        bail!("At least one --stage <config.json> required");
    }

    // solved_set: SMILES that have been solved in any prior stage.
    let mut solved_set: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut stage_results: Vec<StageResult> = Vec::new();
    // Per solved-target quality (computed in the stage that first solves it).
    let mut n_balanced = 0usize;
    let mut n_fwd_validated = 0usize;

    for stage_path in &stage_paths {
        let cfg: StageConfig = serde_json::from_str(
            &std::fs::read_to_string(stage_path)
                .map_err(|e| anyhow::anyhow!("failed to read stage config {stage_path}: {e}"))?,
        )?;

        // Decide which targets to attempt.
        let candidates: Vec<&(String, String)> = if cfg.only_unsolved_from_previous {
            all_targets
                .iter()
                .filter(|(s, _)| !solved_set.contains(s))
                .collect()
        } else {
            all_targets.iter().collect()
        };

        let env = match &cfg.building_blocks {
            Some(p) => ChemEnv::load(p)?,
            None => ChemEnv::load("data/building_blocks.smi")
                .unwrap_or_else(|_| ChemEnv::in_memory(DEFAULT_BUILDING_BLOCKS)),
        };

        let mut rules = default_rules();
        if let Some(ref p) = cfg.templates {
            let mut extra = load_rules_from_file(p);
            if let Some(k) = cfg.top_templates {
                extra = renkin::chem_env::top_templates_by_weight(extra, k);
            }
            eprintln!("[{}] Loaded {} templates from {p}", cfg.name, extra.len());
            rules.extend(extra);
        }

        let config = SearchConfig {
            max_depth: cfg.depth,
            beam_width: cfg.beam_width,
            max_routes: 1,
            ..Default::default()
        };

        eprintln!(
            "[{}] Attempting {}/{} targets (depth={}, beam={}) ...",
            cfg.name,
            candidates.len(),
            total,
            cfg.depth,
            cfg.beam_width
        );

        let mut newly_solved = 0usize;
        for (smiles, _name) in &candidates {
            let (routes, _) = find_routes(smiles, &env, &rules, &config).unwrap_or_default();
            if let Some(best) = routes.first()
                && solved_set.insert(smiles.clone())
            {
                newly_solved += 1;
                // Compute quality in the stage that first solved it (has the right `rules`).
                if route_balanced(best) {
                    n_balanced += 1;
                }
                if quality {
                    let (_, route_status) = validate_route_steps(&best.steps, &rules);
                    if route_status == RouteValidationStatus::Validated {
                        n_fwd_validated += 1;
                    }
                }
            }
        }

        let cumulative = solved_set.len();
        eprintln!(
            "[{}] +{} newly solved → {}/{} ({:.1}%) cumulative",
            cfg.name,
            newly_solved,
            cumulative,
            total,
            cumulative as f64 / total as f64 * 100.0
        );

        stage_results.push(StageResult {
            name: cfg.name,
            attempted: candidates.len(),
            newly_solved,
            cumulative_solved: cumulative,
        });
    }

    let total_solved = solved_set.len();
    let atom_balance_pass_rate = if total_solved > 0 {
        n_balanced as f64 / total_solved as f64
    } else {
        0.0
    };
    let forward_validation_pass_rate = if quality && total_solved > 0 {
        Some(n_fwd_validated as f64 / total_solved as f64)
    } else {
        None
    };
    let report = CascadeReport {
        total,
        stages: stage_results,
        total_solved,
        raw_solved_rate: total_solved as f64 / total as f64,
        atom_balance_pass_rate,
        forward_validation_pass_rate,
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
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
    if args.get(1).map(|s| s.as_str()) == Some("cascade") {
        return cmd_cascade(&args[2..]);
    }

    let mut input_path: Option<String> = None;
    let mut input_format = "smi".to_string();
    let mut bb_path: Option<String> = None;
    let mut templates_path: Option<String> = None;
    let mut top_templates: Option<usize> = None;
    let mut max_depth: u32 = 5;
    let mut beam_width: usize = 0;
    let mut search_mode = "standard".to_string();
    let mut coverage_templates_path: Option<String> = None;
    let mut coverage_timeout_secs: Option<u64> = None;
    let mut max_routes: usize = 1;
    let mut bond_index = false;
    let mut plausibility = false;
    let mut failure_taxonomy = false;
    let mut practical_max_steps: Option<u32> = None;
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
            "--search-mode" => {
                i += 1;
                if let Some(value) = args.get(i) {
                    search_mode = value.clone();
                }
            }
            "--coverage-templates" => {
                i += 1;
                coverage_templates_path = args.get(i).cloned();
            }
            "--coverage-timeout-secs" => {
                i += 1;
                coverage_timeout_secs = args.get(i).and_then(|s| s.parse().ok());
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
            "--top-templates" => {
                i += 1;
                top_templates = args.get(i).and_then(|s| s.parse().ok());
            }
            "--bond-index" => {
                bond_index = true;
            }
            "--plausibility" => {
                plausibility = true;
            }
            "--failure-taxonomy" => {
                failure_taxonomy = true;
            }
            "--practical-max-steps" => {
                i += 1;
                practical_max_steps = args.get(i).and_then(|s| s.parse().ok());
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
             [--beam-width <N>] [--search-mode standard|coverage] \
             [--coverage-templates <path>] [--coverage-timeout-secs <N>] \
             [--building-blocks <path>] [--templates <path>] \
             [--scorer <onnx_path>]"
        );
    };

    if !matches!(search_mode.as_str(), "standard" | "coverage") {
        bail!("invalid --search-mode '{search_mode}' (expected standard|coverage)");
    }
    if search_mode == "coverage" && coverage_templates_path.is_none() {
        bail!("--search-mode coverage requires --coverage-templates <path>");
    }
    if search_mode == "standard" && coverage_templates_path.is_some() {
        bail!("--coverage-templates requires --search-mode coverage");
    }
    if search_mode == "standard" && coverage_timeout_secs.is_some() {
        bail!("--coverage-timeout-secs requires --search-mode coverage");
    }
    if coverage_timeout_secs == Some(0) {
        bail!("--coverage-timeout-secs must be a positive integer");
    }
    let coverage_timeout = coverage_timeout_secs.map(Duration::from_secs);

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
        let mut extra = load_rules_from_file(path);
        if let Some(k) = top_templates {
            extra = renkin::chem_env::top_templates_by_weight(extra, k);
        }
        eprintln!("Loaded {} templates from {path}", extra.len());
        rules.extend(extra);
    }
    let coverage_rules = coverage_templates_path
        .as_deref()
        .map(renkin::coverage_mode::load_coverage_rules)
        .transpose()?;
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
        "Benchmarking {} targets (format={}, mode={}, depth={}, beam_width={}) ...",
        targets.len(),
        input_format,
        search_mode.clone(),
        max_depth,
        beam_width
    );

    let mut results = Vec::new();
    let mut total_depth_sum = 0u32;
    let mut solved_count = 0usize;
    // Step-level tri-state tallies (only incremented under --plausibility), used to
    // separate validator *coverage* (steps_evaluable / steps_checked) from validator
    // *accuracy on what it can judge* (steps_valid / steps_evaluable). See
    // validation_coverage / evaluable_validation_pass_rate in BenchReport.
    let mut steps_checked = 0usize;
    let mut steps_evaluable = 0usize;
    let mut steps_valid = 0usize;

    for (smiles, name, gt_depth) in &targets {
        let t0 = Instant::now();
        let (routes, stats, coverage_meta) = if let Some(ref stage2_rules) = coverage_rules {
            let result = renkin::coverage_mode::run_coverage_mode(
                smiles,
                &env,
                &rules,
                &config,
                stage2_rules,
                coverage_timeout,
            )?;
            let meta = (
                Some(match result.selected_stage {
                    renkin::coverage_mode::SelectedStage::Stage1 => "stage1",
                    renkin::coverage_mode::SelectedStage::Stage2 => "stage2",
                }),
                Some(result.stage2_invoked),
                Some(result.stage1_timeout),
                Some(result.stage2_timeout),
                result.stage2_elapsed_ms,
            );
            (result.routes, result.stats, Some(meta))
        } else {
            let (routes, stats) = find_routes(smiles, &env, &rules, &config).unwrap_or_default();
            (routes, stats, None)
        };
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
        let (forward_validated, route_validation_status) = if plausibility {
            match routes.first() {
                Some(r) => {
                    let validation_rules = coverage_rules.as_deref().unwrap_or(&rules);
                    let (statuses, route_status) = validate_route_steps(&r.steps, validation_rules);
                    steps_checked += statuses.len();
                    steps_evaluable += statuses
                        .iter()
                        .filter(|s| **s != StepValidationStatus::NotEvaluable)
                        .count();
                    steps_valid += statuses
                        .iter()
                        .filter(|s| **s == StepValidationStatus::Valid)
                        .count();
                    (
                        Some(route_status == RouteValidationStatus::Validated),
                        Some(route_validation_status_str(route_status).to_string()),
                    )
                }
                None => (None, None),
            }
        } else {
            (None, None)
        };
        let low_template_confidence = routes.first().map(route_low_confidence);
        let (
            coverage_selected_stage,
            coverage_stage2_invoked,
            coverage_stage1_timeout,
            coverage_stage2_timeout,
            coverage_stage2_elapsed_ms,
        ) = coverage_meta.unwrap_or((None, None, None, None, None));

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
            route_validation_status,
            low_template_confidence,
            beam_limit_hit: stats.beam_limit_hit,
            max_depth_reached: stats.max_depth_reached,
            matched_templates: stats.matched_templates,
            stock_hits: stats.stock_hits,
            retro_cache_hits: stats.retro_cache_hits,
            retro_cache_misses: stats.retro_cache_misses,
            coverage_selected_stage,
            coverage_stage2_invoked,
            coverage_stage1_timeout,
            coverage_stage2_timeout,
            coverage_stage2_elapsed_ms,
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

    // ── Metric trio: raw ≥ validated ≥ practical (all as fraction of ALL targets) ──
    let raw_solved_rate = success_rate;
    // validated: solved AND every step passes forward validation (needs --plausibility).
    let validated_solved_rate = if plausibility {
        Some(n_fwd_validated as f64 / total as f64)
    } else {
        None
    };
    // strict_validated_solved_rate is numerically identical to validated_solved_rate —
    // both count targets whose best route's RouteValidationStatus is Validated. Kept as
    // a separate, explicitly-named field per the tri-state validation design so callers
    // don't have to know that the pre-existing field means the same thing.
    let strict_validated_solved_rate = validated_solved_rate;
    // Coverage: how much of the checked route surface a validation method could judge
    // at all (Valid or Invalid), vs. NotEvaluable. Low coverage means pct_forward_validated
    // is dominated by validator blind spots, not chemistry quality.
    let validation_coverage = if plausibility && steps_checked > 0 {
        Some(steps_evaluable as f64 / steps_checked as f64)
    } else {
        None
    };
    // Accuracy on the subset the validator could actually judge, denominator excludes
    // NotEvaluable steps — isolates "is what we can check actually right" from "how much
    // can we check" (validation_coverage).
    let evaluable_validation_pass_rate = if plausibility && steps_evaluable > 0 {
        Some(steps_valid as f64 / steps_evaluable as f64)
    } else {
        None
    };
    // practical: solved AND route depth within the practical step budget.
    let practical_solved_rate = practical_max_steps.map(|max_steps| {
        let n_practical = solved_results
            .iter()
            .filter(|r| r.best_depth.is_some_and(|d| d <= max_steps))
            .count();
        n_practical as f64 / total as f64
    });

    let report = BenchReport {
        exploration_contract: exploration_contract(),
        search_strategy: config.strategy_name(),
        search_mode,
        search_depth: max_depth,
        search_beam_width: beam_width,
        coverage_template_count: coverage_rules.as_ref().map(Vec::len),
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
        raw_solved_rate,
        validated_solved_rate,
        strict_validated_solved_rate,
        validation_coverage,
        evaluable_validation_pass_rate,
        practical_solved_rate,
        total_retro_cache_hits: results.iter().map(|r| r.retro_cache_hits).sum(),
        total_retro_cache_misses: results.iter().map(|r| r.retro_cache_misses).sum(),
        results,
    };

    if failure_taxonomy {
        let unsolved: Vec<&BenchResult> = report.results.iter().filter(|r| !r.solved).collect();
        let n = unsolved.len();
        if n > 0 {
            let beam_hit = unsolved.iter().filter(|r| r.beam_limit_hit).count();
            let depth_hit = unsolved.iter().filter(|r| r.max_depth_reached).count();
            let no_tmpl = unsolved.iter().filter(|r| r.matched_templates < 3).count();
            // stock_near_miss: stock was reached (hits > 0) but no route found
            let stock_near = unsolved.iter().filter(|r| r.stock_hits > 0).count();
            eprintln!();
            eprintln!("=== Failure Taxonomy ({n} unsolved) ===");
            eprintln!(
                "  beam_limit_hit    : {:4} ({:.1}%)",
                beam_hit,
                beam_hit as f64 / n as f64 * 100.0
            );
            eprintln!(
                "  max_depth_reached : {:4} ({:.1}%)",
                depth_hit,
                depth_hit as f64 / n as f64 * 100.0
            );
            eprintln!(
                "  no_template_match : {:4} ({:.1}%)",
                no_tmpl,
                no_tmpl as f64 / n as f64 * 100.0
            );
            eprintln!(
                "  stock_near_miss   : {:4} ({:.1}%)",
                stock_near,
                stock_near as f64 / n as f64 * 100.0
            );
            eprintln!("(categories overlap; one target may have multiple causes)");
        } else {
            eprintln!("=== Failure Taxonomy: all targets solved ===");
        }
    }

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

/// True if any step uses a template with step_confidence < 0.1 (rare template).
fn route_low_confidence(route: &Route) -> bool {
    route.steps.iter().any(|s| s.step_confidence < 0.1)
}

fn route_validation_status_str(status: RouteValidationStatus) -> &'static str {
    match status {
        RouteValidationStatus::Validated => "validated",
        RouteValidationStatus::Invalid => "invalid",
        RouteValidationStatus::PartiallyValidated => "partially_validated",
        RouteValidationStatus::NotEvaluable => "not_evaluable",
    }
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
