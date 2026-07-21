//! Ad hoc analysis tool for the Phase 31 corrected-baseline investigation:
//! reconstructs each target's best route (same config as the benchmark run)
//! and prints per-STEP validation status by rule, so Invalid verdicts can be
//! classified as real chemistry errors vs. validator false-negatives instead
//! of only ever seeing the route-level rollup. Not part of any measured
//! binary — reads targets from stdin, one SMILES per line.
use renkin::chem_env::{ChemEnv, default_rules, load_rules_from_file};
use renkin::search::{SearchConfig, find_routes};
use renkin::validation::atom_conservation::step_balanced;
use renkin::validation::validate_route_steps;
use std::io::Read;

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

    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();

    for line in input.lines() {
        let smiles = line.trim();
        if smiles.is_empty() || smiles.starts_with('#') {
            continue;
        }
        let Ok((routes, _stats)) = find_routes(smiles, &env, &rules, &config) else {
            println!("{smiles}\tERROR");
            continue;
        };
        let Some(route) = routes.first() else {
            println!("{smiles}\tUNSOLVED");
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
