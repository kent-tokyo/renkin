//! Lightweight smoke measurement for `SearchConfig::element_accounting_policy`
//! (v0.37.0 rollout plan, design doc §9 stage 5) against a small real-target
//! sample -- deliberately NOT a formal remeasurement, same discipline as
//! `spectator_bond_smoke.rs` (this example's own direct precedent, mirrored
//! as closely as the two mechanisms' different diagnostic surfaces allow).
//!
//! Same sample/rules/search parameters as `spectator_bond_smoke.rs`: first
//! N=15 lines of the *existing* n=300 sample from Finding #4's pilot
//! (`data/finding4_pilot_2026-08-23/target_sample_n300_seed42.smi`),
//! `default_rules()` + `data/templates_extracted_5000.smi`, depth=5,
//! beam=100, max_routes=1, 90s per-target cooperative-cancellation timeout
//! via `SearchControl::with_timeout` built fresh per target.
//!
//! Structural difference from `spectator_bond_smoke.rs`: that example runs
//! once under `DiagnosticsOnly` and *correlates* findings (rule/target
//! pairs) against the one route found, since `SpectatorBondLoss` findings
//! are detected at the (rule, target) level independent of which candidate
//! survives. `ElementAccountingGateVerdict` is already directly per-
//! candidate, and `CrowdOutDiagnostics::element_accounting_gated_out` is
//! only ever populated under `Gated` (empty under `DiagnosticsOnly`, by
//! design -- see that field's own doc), so there is no equivalent
//! after-the-fact correlation available from a single `DiagnosticsOnly`
//! pass. This example instead runs each target **twice**, once under `Off`
//! and once under `Gated`, and reports the direct route-found delta
//! alongside the real excluded-candidate count -- a more direct measurement
//! of exactly what design doc §9 stage 5 asks for ("excluded-candidate
//! counts and route-count deltas"), at the cost of 2x the search calls per
//! target (still cheap at N=15).

use renkin::chem_env::{ChemEnv, default_rules, load_rules_from_file};
use renkin::search::{
    ElementAccountingGatePolicy, SearchConfig, SearchControl, SearchTermination,
    find_routes_with_control,
};
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
        "element_accounting_smoke: {} targets from {SAMPLE_PATH} (first {N} of its own n=300), \
         {TIMEOUT_SECS}s timeout each, run under both Off and Gated",
        targets.len()
    );

    let off_config = SearchConfig {
        max_depth: 5,
        max_routes: 1,
        beam_width: 100,
        element_accounting_policy: ElementAccountingGatePolicy::Off,
        ..Default::default()
    };
    let gated_config = SearchConfig {
        max_depth: 5,
        max_routes: 1,
        beam_width: 100,
        element_accounting_policy: ElementAccountingGatePolicy::Gated,
        ..Default::default()
    };

    let mut off_solved = 0usize;
    let mut off_timed_out = 0usize;
    let mut gated_solved = 0usize;
    let mut gated_timed_out = 0usize;
    let mut total_gated_out = 0usize;
    let mut targets_with_gated_out = 0usize;
    let mut regressions = 0usize; // Off solved, Gated did not
    let mut new_solves = 0usize; // Gated solved, Off did not

    for (i, target) in targets.iter().enumerate() {
        let off_control = SearchControl::with_timeout(Duration::from_secs(TIMEOUT_SECS));
        let off_result = find_routes_with_control(target, &env, &rules, &off_config, &off_control);
        let gated_control = SearchControl::with_timeout(Duration::from_secs(TIMEOUT_SECS));
        let gated_result =
            find_routes_with_control(target, &env, &rules, &gated_config, &gated_control);

        let (Ok(off_result), Ok(gated_result)) = (off_result, gated_result) else {
            println!("[{}/{}] {target}\tERROR", i + 1, targets.len());
            continue;
        };

        let off_found = !off_result.routes.is_empty();
        let gated_found = !gated_result.routes.is_empty();
        if off_found {
            off_solved += 1;
        }
        if matches!(off_result.termination, SearchTermination::DeadlineExceeded) {
            off_timed_out += 1;
        }
        if gated_found {
            gated_solved += 1;
        }
        if matches!(
            gated_result.termination,
            SearchTermination::DeadlineExceeded
        ) {
            gated_timed_out += 1;
        }
        if off_found && !gated_found {
            regressions += 1;
        }
        if gated_found && !off_found {
            new_solves += 1;
        }

        let gated_out = gated_result
            .stats
            .crowd_out
            .element_accounting_gated_out
            .len();
        total_gated_out += gated_out;
        if gated_out > 0 {
            targets_with_gated_out += 1;
        }

        println!(
            "[{}/{}] {target}\toff_found={off_found}\tgated_found={gated_found}\tgated_out={gated_out}",
            i + 1,
            targets.len()
        );
        // Print at most 5 example exclusions per target -- some real
        // targets generate hundreds/thousands of them (e.g. a symmetric
        // ring-fused scaffold matching the same rule many times over), and
        // printing every one is pure I/O overhead that tells us nothing
        // beyond the count already reported above.
        for record in gated_result
            .stats
            .crowd_out
            .element_accounting_gated_out
            .iter()
            .take(5)
        {
            println!(
                "    excluded: rule={} target={} precursors={:?}",
                record.rule_name, record.target_smiles, record.precursor_smiles
            );
        }
    }

    println!("\n=== summary ===");
    println!("targets: {}", targets.len());
    println!("off: solved={off_solved} timed_out={off_timed_out}");
    println!("gated: solved={gated_solved} timed_out={gated_timed_out}");
    println!("regressions (off solved, gated did not): {regressions}");
    println!("new solves (gated solved, off did not): {new_solves}");
    println!(
        "total excluded candidates under gated: {total_gated_out} ({targets_with_gated_out}/{} targets with >=1 exclusion)",
        targets.len()
    );
}
