//! Lightweight smoke measurement for `SearchConfig::spectator_bond_policy`
//! (v0.35.0 rollout plan, PR #186) against a small real-target sample --
//! deliberately NOT a formal remeasurement (no heavy 300/4,907-target run;
//! see `docs/validation/finding4-validator-pilot-2026-08-23.md` for why that
//! scale is expensive and not needed here).
//!
//! Reuses the first N lines of the *existing* n=300 sample from Finding #4's
//! pilot (`data/finding4_pilot_2026-08-23/target_sample_n300_seed42.smi`,
//! itself `random.seed(42)`-drawn from `data/uspto50k_test.smi`'s 4,907
//! targets) rather than drawing a fresh sample -- that sample's own ordering
//! is already randomized, so taking its prefix needs no new sampling
//! decision and reuses already-documented provenance.
//!
//! Same search configuration as that pilot (depth=5, beam=100,
//! `default_rules()` + `data/templates_extracted_5000.smi`, 90s per-target
//! cooperative-cancellation timeout via `SearchControl::with_timeout` built
//! fresh per target -- see `examples/inspect_validation.rs`'s own doc
//! comment for why a shared/hoisted control silently breaks this).
//!
//! For each target: runs `find_routes_with_control` once with
//! `spectator_bond_policy: SpectatorBondPolicy::DiagnosticsOnly`, then
//! reports every `SpectatorBondLossFinding` surfaced and, for each one,
//! whether any *found* route's own steps use that exact
//! `(rule_name, target_smiles)` pair -- i.e. what `Gated` policy would
//! actually touch on this sample, without switching this example to that
//! policy itself (fail-closed gating landed in PR #188, after this example
//! was first written; a separate `Gated`-policy run is its own follow-up
//! measurement, not folded into this one). Diagnostic-only vs. default
//! route-count parity is already unit-tested
//! (`spectator_bond_policy_diagnostics_only_runs_without_error_on_default_rules`);
//! not re-verified per-target here to keep this lightweight.

use renkin::chem_env::{ChemEnv, default_rules, load_rules_from_file};
use renkin::search::{SearchConfig, SearchControl, find_routes_with_control};
use renkin::spectator_bond::SpectatorBondPolicy;
use std::time::Duration;

const SAMPLE_PATH: &str = "data/finding4_pilot_2026-08-23/target_sample_n300_seed42.smi";
const N: usize = 15;
const TIMEOUT_SECS: u64 = 90;

fn main() {
    let env = ChemEnv::load("data/building_blocks.smi").expect("load building blocks");
    let mut rules = default_rules();
    rules.extend(load_rules_from_file("data/templates_extracted_5000.smi"));

    let sample = std::fs::read_to_string(SAMPLE_PATH)
        .unwrap_or_else(|e| panic!("failed to read {SAMPLE_PATH}: {e} -- run from the crate root"));
    let targets: Vec<&str> = sample
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take(N)
        .collect();
    eprintln!(
        "spectator_bond_smoke: {} targets from {SAMPLE_PATH} (first {N} of its own n=300), \
         {TIMEOUT_SECS}s timeout each",
        targets.len()
    );

    let config = SearchConfig {
        max_depth: 5,
        max_routes: 1,
        beam_width: 100,
        spectator_bond_policy: SpectatorBondPolicy::DiagnosticsOnly,
        ..Default::default()
    };

    let mut total_findings = 0usize;
    let mut case_a_count = 0usize;
    let mut case_b_count = 0usize;
    let mut targets_with_findings = 0usize;
    let mut targets_timed_out = 0usize;
    let mut targets_solved = 0usize;
    let mut route_impacted_targets = 0usize;

    for (i, target) in targets.iter().enumerate() {
        let control = SearchControl::with_timeout(Duration::from_secs(TIMEOUT_SECS));
        let t0 = std::time::Instant::now();
        let result = find_routes_with_control(target, &env, &rules, &config, &control);
        let elapsed = t0.elapsed().as_secs_f64();

        let Ok(result) = result else {
            eprintln!(
                "[{}/{}] {target}\tERROR\t{elapsed:.1}s",
                i + 1,
                targets.len()
            );
            continue;
        };

        let findings = &result.stats.crowd_out.spectator_bond_loss_findings;
        if result.routes.is_empty() {
            let status = match result.termination {
                renkin::search::SearchTermination::DeadlineExceeded => {
                    targets_timed_out += 1;
                    "TIMEOUT"
                }
                renkin::search::SearchTermination::Completed => "UNSOLVED",
            };
            println!(
                "[{}/{}] {target}\t{status}\t{elapsed:.1}s\tfindings={}",
                i + 1,
                targets.len(),
                findings.len()
            );
        } else {
            targets_solved += 1;
            let route = &result.routes[0];
            let route_pairs: std::collections::HashSet<(String, String)> = route
                .steps
                .iter()
                .map(|s| (s.rule.clone(), s.target.clone()))
                .collect();
            let impacted = findings
                .iter()
                .any(|f| route_pairs.contains(&(f.rule_name.clone(), f.target_smiles.clone())));
            if impacted {
                route_impacted_targets += 1;
            }
            println!(
                "[{}/{}] {target}\tROUTE\t{elapsed:.1}s\tdepth={}\tfindings={}\troute_impacted={impacted}",
                i + 1,
                targets.len(),
                route.depth,
                findings.len()
            );
        }

        if !findings.is_empty() {
            targets_with_findings += 1;
        }
        for f in findings {
            total_findings += 1;
            match f.case {
                renkin::spectator_bond::SpectatorBondLossCase::MatchedPairUndeclared => {
                    case_a_count += 1
                }
                renkin::spectator_bond::SpectatorBondLossCase::CrossProductTerritory => {
                    case_b_count += 1
                }
            }
            println!(
                "    finding: rule={} case={:?} target={} lost_bonds={} evidence={}",
                f.rule_name,
                f.case,
                f.target_smiles,
                f.lost_bonds.len(),
                f.evidence
            );
        }
    }

    println!("\n=== summary ===");
    println!("targets: {}", targets.len());
    println!("solved (route found): {targets_solved}");
    println!("timed out: {targets_timed_out}");
    println!("targets with >=1 finding: {targets_with_findings}");
    println!("total findings: {total_findings} (case A: {case_a_count}, case B: {case_b_count})");
    println!("targets whose found route was itself impacted: {route_impacted_targets}");
}
