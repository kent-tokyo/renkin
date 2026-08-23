//! Ad hoc analysis tool for the Phase 31 corrected-baseline investigation:
//! reconstructs each target's best route (same config as the benchmark run)
//! and prints per-STEP validation status by rule, so Invalid verdicts can be
//! classified as real chemistry errors vs. validator false-negatives instead
//! of only ever seeing the route-level rollup. Not part of any measured
//! binary — reads targets from stdin, one SMILES per line.
//!
//! Optional `INSPECT_VALIDATION_TIMEOUT_SECS` env var (unset = unlimited,
//! the original/default behavior): per-target cooperative-cancellation
//! deadline (`SearchControl::with_timeout`), added for Finding #4's
//! rule-stratified sample after Issue #128's root cause (chematic's
//! canonical_smiles combinatorial cost on locally-symmetric molecules --
//! Boc/tBu/pivaloyl groups, rings, cages) confirmed that per-target latency
//! in a real USPTO-50k-shaped sample is not predictable up front: a small,
//! not-reliably-identifiable-in-advance minority of targets can run for
//! several minutes. A hard OS-level `timeout` wrapper was already found
//! unreliable against this exact codebase (didn't actually kill the process
//! at the requested mark on one prior attempt) -- this native, cooperative
//! deadline is checked at the search loop's own existing checkpoints
//! instead, same mechanism `find_routes_with_control`'s own doc comment
//! documents as a *soft* bound (worst-case overshoot is bounded by the
//! slowest single stretch of synchronous work between two checkpoints, not
//! a hard real-time guarantee) -- adequate here since the goal is bounding
//! a *batch's* total wall-clock, not any one target's exactly.
use renkin::chem_env::{ChemEnv, default_rules, load_rules_from_file};
use renkin::search::{SearchConfig, SearchControl, SearchTermination, find_routes_with_control};
use renkin::validation::atom_conservation::step_balanced;
use renkin::validation::validate_route_steps;
use std::io::Read;
use std::time::Duration;

fn main() {
    let env = ChemEnv::load("data/building_blocks.smi").expect("load building blocks");
    let mut rules = default_rules();
    rules.extend(load_rules_from_file("data/templates_extracted_5000.smi"));

    let config = SearchConfig {
        max_depth: 5,
        max_routes: 1,
        beam_width: 100,
        ..Default::default()
    };

    let timeout_secs: Option<u64> = std::env::var("INSPECT_VALIDATION_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok());

    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();

    for line in input.lines() {
        let smiles = line.trim();
        if smiles.is_empty() || smiles.starts_with('#') {
            continue;
        }
        // Fresh per-target budget -- a deadline built once outside this loop
        // would bake in an *absolute* Instant, silently turning "N seconds
        // per target" into "N seconds for the whole batch", after which
        // every remaining target's very first checkpoint check would see
        // the shared deadline already passed and report an instant,
        // meaningless TIMEOUT. Caught exactly this way on the harness's
        // first real run (299/300 TIMEOUT in 96s total -- impossible if
        // each of 300 targets had genuinely spent up to 90s).
        let control = timeout_secs
            .map(|secs| SearchControl::with_timeout(Duration::from_secs(secs)))
            .unwrap_or_else(SearchControl::unlimited);
        let Ok(result) = find_routes_with_control(smiles, &env, &rules, &config, &control) else {
            println!("{smiles}\tERROR");
            continue;
        };
        let Some(route) = result.routes.first() else {
            let status = match result.termination {
                SearchTermination::Completed => "UNSOLVED",
                SearchTermination::DeadlineExceeded => "TIMEOUT",
            };
            println!("{smiles}\t{status}");
            continue;
        };
        let (statuses, route_status) = validate_route_steps(&route.steps, &rules);
        println!("{smiles}\tROUTE\t{route_status:?}\tdepth={}", route.depth);
        for (step, status) in route.steps.iter().zip(statuses.iter()) {
            let balanced = step_balanced(&step.target, &step.precursors);
            println!(
                "{smiles}\tSTEP\t{status:?}\tbalanced={balanced}\trule={}\ttarget={}\tprecursors={}",
                step.rule,
                step.target,
                step.precursors.join(".")
            );
        }
    }
}
