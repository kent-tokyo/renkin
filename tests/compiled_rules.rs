#![cfg(feature = "perf-instrumentation")]

use renkin::chem_env::{ChemEnv, RetroRule, template_id_for_smirks};
use renkin::coverage_mode::run_coverage_mode;
use renkin::search::{SearchConfig, find_routes};

#[test]
fn one_search_parses_each_smirks_once_even_across_multiple_expansions() {
    let smirks = "[C:1]-[C:2]>>[C:1].[C:2]";
    let rules = vec![RetroRule {
        name: "test_cc_cleavage".to_string(),
        template_id: template_id_for_smirks(smirks),
        smirks: smirks.to_string(),
        weight: 1.0,
        required_elements: 0,
    }];
    let env = ChemEnv::in_memory(&[]);
    let config = SearchConfig {
        max_depth: 3,
        max_routes: 1,
        ..SearchConfig::default()
    };

    let target = chematic::smiles::parse("CCCCC").unwrap();
    chematic_rxn::perf_counters::reset();
    for _ in 0..8 {
        let _ = chematic_rxn::run_reactants(smirks, &[&target]).unwrap();
    }
    let legacy = chematic_rxn::perf_counters::snapshot();
    assert_eq!(legacy.reaction_parse_calls, 8);

    chematic_rxn::perf_counters::reset();
    let _ = find_routes("CCCCC", &env, &rules, &config).unwrap();
    let counters = chematic_rxn::perf_counters::snapshot();

    assert_eq!(counters.reaction_parse_calls, 1);
    assert!(
        counters.run_reactants_calls > 1,
        "fixture must exercise the same compiled rule across multiple expansions: {counters:?}"
    );

    chematic_rxn::perf_counters::reset();
    let coverage = run_coverage_mode("CCCCC", &env, &rules, &config, &rules, None).unwrap();
    let staged = chematic_rxn::perf_counters::snapshot();
    assert!(coverage.stage2_invoked);
    assert_eq!(
        staged.reaction_parse_calls, 1,
        "Stage 1 and Stage 2 must share one prepared ruleset"
    );
}
