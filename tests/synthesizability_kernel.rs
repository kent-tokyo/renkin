//! Top-level integration tests for the Synthesizability Kernel
//! (`docs/design/synthesizability-kernel-v0.md`), exercising only the
//! public API (`renkin::synthesizability::{assess_routes, AssessmentContext,
//! ...}`) exactly as an external caller would -- no `pub(crate)` internals
//! from `signals.rs`/`element_accounting.rs`/`provenance.rs` (those are
//! private modules not re-exported by `synthesizability::mod.rs`, so an
//! integration test in `tests/` genuinely cannot reach them; see this
//! agent's final report for what that means for design doc §9's fixture
//! plan).
//!
//! Section headers below follow the design doc §9 / task checklist
//! numbering (1-20), plus "Additional" tests for behavior found by reading
//! the merged code that the checklist didn't enumerate by name.

use renkin::chem_env::RetroRule;
use renkin::evidence::{ExampleMatch, ReactionExample, ResolvedReactionExample, StepEvidence};
use renkin::search::{ReactionStep, Route};
use renkin::synthesizability::{
    AssessmentContext, AssessmentStatus, ElementAccountingStatus, EvidencePolicy,
    ForwardValidationPolicy, ForwardValidationStatus, HardFailure, SYNTHESIZABILITY_SCHEMA_VERSION,
    StockTerminationStatus, SynthesizabilityConfig, ValidationGap, assess_routes,
};
use renkin::validation::StepValidationStatus;

// ---------------------------------------------------------------------
// Shared builders -- mirrors the style already used in each
// src/synthesizability/*.rs file's own #[cfg(test)] module.
// ---------------------------------------------------------------------

fn no_rules() -> Vec<RetroRule> {
    vec![]
}

fn step(rule: &str, template_id: &str, target: &str, precursors: &[&str]) -> ReactionStep {
    ReactionStep {
        rule: rule.to_string(),
        template_id: template_id.to_string(),
        target: target.to_string(),
        precursors: precursors.iter().map(|s| s.to_string()).collect(),
        conditions: None,
        atom_economy: None,
        step_confidence: 1.0,
        procedure_hint: None,
        reaction_family: None,
        metadata_source: None,
        metadata_scope: None,
        evidence: None,
    }
}

fn step_with_evidence(
    rule: &str,
    template_id: &str,
    target: &str,
    precursors: &[&str],
    evidence: StepEvidence,
) -> ReactionStep {
    ReactionStep {
        evidence: Some(evidence),
        ..step(rule, template_id, target, precursors)
    }
}

fn route(steps: Vec<ReactionStep>, building_blocks: &[&str]) -> Route {
    Route {
        depth: steps.len() as u32,
        steps,
        score: 0.0,
        building_blocks: building_blocks.iter().map(|s| s.to_string()).collect(),
        confidence: 1.0,
        convergency: 1.0,
        success_probability: 1.0,
        route_cost: 1.0,
    }
}

fn empty_context(rules: &[RetroRule]) -> AssessmentContext<'_> {
    AssessmentContext {
        stock: None,
        stock_source: None,
        template_metadata_hash: None,
        search_config_summary: None,
        git_commit: None,
        embedded_fallback_used: None,
        forward_validation: None,
        rules,
    }
}

fn resolved_example(match_kind: ExampleMatch, id: &str) -> ResolvedReactionExample {
    ResolvedReactionExample {
        match_kind,
        example: ReactionExample {
            id: id.to_string(),
            target_smiles: "CCO".to_string(),
            precursor_smiles: vec!["CC=O".to_string()],
            conditions: None,
            reported_yield: None,
            warnings: vec![],
            reference_ids: vec![],
            dataset_record_id: None,
            notes: None,
        },
    }
}

fn evidence_with(examples: Vec<ResolvedReactionExample>) -> StepEvidence {
    let total = examples.len();
    StepEvidence {
        condition_candidates: vec![],
        reported_yields: vec![],
        references: vec![],
        warnings: vec![],
        examples,
        template_examples_total: total,
    }
}

// ---------------------------------------------------------------------
// 1. Target parse failure -> InvalidTarget
// ---------------------------------------------------------------------

#[test]
fn invalid_target_smiles_is_invalid_target() {
    let rules = no_rules();
    let ctx = empty_context(&rules);
    let result = assess_routes(
        "not-a-smiles(((",
        &[],
        &ctx,
        &SynthesizabilityConfig::conservative(),
    )
    .expect("assess_routes must not Err for a bad target");
    assert_eq!(result.status, AssessmentStatus::InvalidTarget);
    assert_eq!(result.canonical_target, None);
    assert!(result.route_assessments.is_empty());
    assert_eq!(result.provenance.canonical_target, "");
}

// ---------------------------------------------------------------------
// 2. No routes supplied -> NoRouteFoundWithinBudget, never Indeterminate.
//
// Per design doc §3: today's `find_routes` has no time/node budget --
// depth and beam width *are* the budget -- so an empty route list can only
// mean the search frontier was exhausted. There is no signal anywhere in
// this schema version that could distinguish "space exhausted" from "search
// cut off early", so `AssessmentStatus::Indeterminate` is unreachable
// through the public API today, by construction. This test exists
// specifically so nobody adds a distinct "timeout" fixture later without
// first adding real budget tracking to `SearchConfig` (design doc §10 lists
// that as an explicit non-goal for this PR; §11.2 as a known limitation).
// ---------------------------------------------------------------------

#[test]
fn no_routes_is_always_no_route_found_within_budget_never_indeterminate() {
    let rules = no_rules();
    for search_config_summary in [
        None,
        Some("depth=6,beam=50,max_routes=20".to_string()),
        Some("depth=1,beam=1,max_routes=1".to_string()),
    ] {
        let ctx = AssessmentContext {
            search_config_summary,
            ..empty_context(&rules)
        };
        let result = assess_routes("CCO", &[], &ctx, &SynthesizabilityConfig::conservative())
            .expect("empty routes must not Err");
        assert_eq!(result.status, AssessmentStatus::NoRouteFoundWithinBudget);
        assert_ne!(result.status, AssessmentStatus::Indeterminate);
        assert!(result.canonical_target.is_some());
    }
}

// ---------------------------------------------------------------------
// 3. Stock: full termination -> AllLeavesVerifiedInConfiguredStock
// ---------------------------------------------------------------------

#[test]
fn stock_full_termination_all_leaves_verified() {
    let rules = no_rules();
    let routes = vec![route(
        vec![step(
            "esterification_retro",
            "rule:esterification_retro",
            "CCOC(=O)c1ccccc1",
            &["CCO", "O=C(O)c1ccccc1"],
        )],
        &["CCO", "O=C(O)c1ccccc1"],
    )];
    let stock = vec!["CCO".to_string(), "O=C(O)c1ccccc1".to_string()];
    let ctx = AssessmentContext {
        stock: Some(&stock),
        ..empty_context(&rules)
    };
    let result = assess_routes(
        "CCOC(=O)c1ccccc1",
        &routes,
        &ctx,
        &SynthesizabilityConfig::conservative(),
    )
    .unwrap();
    assert_eq!(
        result.route_assessments[0].stock_termination_status,
        StockTerminationStatus::AllLeavesVerifiedInConfiguredStock
    );
    assert!(result.route_assessments[0].hard_failures.is_empty());
    assert_eq!(result.status, AssessmentStatus::RouteSupported);
}

// ---------------------------------------------------------------------
// 4. Stock: leaf mismatch -> OneOrMoreLeavesNotInStock / HardFailure::
//    StockTerminalMismatch. The exact 3 real Issue #71 false positives
//    (see `data/comparison/results_100/per_target_audit.md` lines 36-38 and
//    design doc §9): the kernel's own independent stock check must report
//    these as NOT in stock, unlike the old `ChemEnv::is_building_block` VF2
//    fallback which accepted them incorrectly.
// ---------------------------------------------------------------------

/// Shared shape for all three Issue #71 fixtures: a single-step route whose
/// only leaf is the exact false-positive SMILES, checked against a stock
/// list that does not contain it (or any canonicalization of it). The
/// target/rule are fictitious pairings chosen only so the target's heavy
/// atoms are a strict subset of the leaf's (the one-directional element
/// accounting check, design doc §4.5, is satisfied trivially and does not
/// interfere with isolating the stock signal under test).
fn issue71_fixture_route(rule_name: &str, target: &str, false_positive_leaf: &str) -> Route {
    route(
        vec![step(
            rule_name,
            &format!("rule:{rule_name}"),
            target,
            &[false_positive_leaf],
        )],
        &[false_positive_leaf],
    )
}

#[test]
fn issue71_pentadiene_false_positive_is_not_in_stock() {
    // C=C/C/C=C claimed as leaf; real molecule is C=CCC=C (1,4-pentadiene).
    // Stock deliberately contains neither the raw string nor any
    // canonicalization of it.
    let rules = no_rules();
    let routes = vec![issue71_fixture_route(
        "cc_single_cleavage",
        "CC=C", // propene: C:3, a strict subset of the leaf's C:5
        "C=C/C/C=C",
    )];
    let stock = vec!["CCO".to_string()];
    let ctx = AssessmentContext {
        stock: Some(&stock),
        ..empty_context(&rules)
    };
    let result = assess_routes(
        "CC=C",
        &routes,
        &ctx,
        &SynthesizabilityConfig::conservative(),
    )
    .unwrap();
    assert_eq!(
        result.route_assessments[0].stock_termination_status,
        StockTerminationStatus::OneOrMoreLeavesNotInStock
    );
    assert!(result.route_assessments[0].hard_failures.contains(
        &HardFailure::StockTerminalMismatch {
            leaf: "C=C/C/C=C".to_string()
        }
    ));
    assert_eq!(result.status, AssessmentStatus::RoutesFoundButRejected);
    assert!(result.selected_route.is_none());
}

#[test]
fn issue71_glyoxylic_acid_false_positive_is_not_in_stock() {
    // O=C/C(=O)O claimed as leaf; real molecule is O=CC(=O)O (glyoxylic acid).
    let rules = no_rules();
    let routes = vec![issue71_fixture_route(
        "wittig_retro",
        "O=CO", // formic acid: C:1, O:2 -- subset of the leaf's C:2, O:3
        "O=C/C(=O)O",
    )];
    let stock = vec!["CCO".to_string()];
    let ctx = AssessmentContext {
        stock: Some(&stock),
        ..empty_context(&rules)
    };
    let result = assess_routes(
        "O=CO",
        &routes,
        &ctx,
        &SynthesizabilityConfig::conservative(),
    )
    .unwrap();
    assert_eq!(
        result.route_assessments[0].stock_termination_status,
        StockTerminationStatus::OneOrMoreLeavesNotInStock
    );
    assert!(result.route_assessments[0].hard_failures.contains(
        &HardFailure::StockTerminalMismatch {
            leaf: "O=C/C(=O)O".to_string()
        }
    ));
    assert_eq!(result.status, AssessmentStatus::RoutesFoundButRejected);
}

#[test]
fn issue71_phenylacetaldehyde_false_positive_is_not_in_stock() {
    // c1ccc(cc1)CC=O claimed as leaf; real molecule is O=CCc1ccccc1
    // (phenylacetaldehyde).
    let rules = no_rules();
    let routes = vec![issue71_fixture_route(
        "co_aliphatic_cleavage",
        "c1ccccc1", // benzene: C:6 -- subset of the leaf's C:8
        "c1ccc(cc1)CC=O",
    )];
    let stock = vec!["CCO".to_string()];
    let ctx = AssessmentContext {
        stock: Some(&stock),
        ..empty_context(&rules)
    };
    let result = assess_routes(
        "c1ccccc1",
        &routes,
        &ctx,
        &SynthesizabilityConfig::conservative(),
    )
    .unwrap();
    assert_eq!(
        result.route_assessments[0].stock_termination_status,
        StockTerminationStatus::OneOrMoreLeavesNotInStock
    );
    assert!(result.route_assessments[0].hard_failures.contains(
        &HardFailure::StockTerminalMismatch {
            leaf: "c1ccc(cc1)CC=O".to_string()
        }
    ));
    assert_eq!(result.status, AssessmentStatus::RoutesFoundButRejected);
}

// ---------------------------------------------------------------------
// 5. Stock not supplied -> StockNotSupplied / ValidationGap::
//    StockProvenanceHashMissing (top-level-integration-test version of an
//    existing assessment.rs-internal test).
// ---------------------------------------------------------------------

#[test]
fn stock_not_supplied_is_a_gap_not_a_hard_failure_top_level() {
    let rules = no_rules();
    let routes = vec![route(
        vec![step("rule:x", "rule:x", "CCO", &["CC=O"])],
        &["CC=O"],
    )];
    let ctx = empty_context(&rules); // stock: None
    let result = assess_routes(
        "CCO",
        &routes,
        &ctx,
        &SynthesizabilityConfig::conservative(),
    )
    .unwrap();
    assert_eq!(
        result.route_assessments[0].stock_termination_status,
        StockTerminationStatus::StockNotSupplied
    );
    assert!(
        result.route_assessments[0]
            .validation_gaps
            .contains(&ValidationGap::StockProvenanceHashMissing)
    );
    assert!(result.route_assessments[0].hard_failures.is_empty());
    assert_eq!(
        result.status,
        AssessmentStatus::RouteSupportedWithValidationGaps
    );
}

// ---------------------------------------------------------------------
// 6. Stock check disabled by config -> StockCheckNotPerformed
// ---------------------------------------------------------------------

#[test]
fn stock_check_disabled_by_config_is_not_performed() {
    let rules = no_rules();
    let routes = vec![route(
        vec![step("rule:x", "rule:x", "CCO", &["CC=O"])],
        &["CC=O"],
    )];
    let ctx = empty_context(&rules);
    let mut config = SynthesizabilityConfig::conservative();
    config.require_verified_stock_terminal = false;
    let result = assess_routes("CCO", &routes, &ctx, &config).unwrap();
    assert_eq!(
        result.route_assessments[0].stock_termination_status,
        StockTerminationStatus::StockCheckNotPerformed
    );
    assert!(result.route_assessments[0].hard_failures.is_empty());
    assert!(result.route_assessments[0].validation_gaps.is_empty());
    assert_eq!(result.status, AssessmentStatus::RouteSupported);
}

// ---------------------------------------------------------------------
// 7/8. Target-element accounting cross-language parity fixtures, mirrored
// verbatim from `scripts/tests/test_compare_validation.py`.
// ---------------------------------------------------------------------

/// Cross-language parity fixture, mirrored verbatim from
/// `scripts/tests/test_compare_validation.py::
/// test_esterification_is_accounted_despite_water_byproduct` (target
/// `TARGET = "CCOC(=O)c1ccccc1"`, precursors `ETHANOL = "CCO"` +
/// `BENZOIC_ACID = "O=C(O)c1ccccc1"`). The Python assertion is
/// `status == "accounted"`; the Rust equivalent is
/// `ElementAccountingStatus::Accounted`, which serializes (per
/// `#[serde(rename_all = "snake_case")]`) to exactly `"accounted"`.
#[test]
fn target_element_accounted_ethyl_benzoate_parity_with_python() {
    let rules = no_rules();
    let routes = vec![route(
        vec![step(
            "esterification_retro",
            "rule:esterification_retro",
            "CCOC(=O)c1ccccc1",
            &["CCO", "O=C(O)c1ccccc1"],
        )],
        &["CCO", "O=C(O)c1ccccc1"],
    )];
    let stock = vec!["CCO".to_string(), "O=C(O)c1ccccc1".to_string()];
    let ctx = AssessmentContext {
        stock: Some(&stock),
        ..empty_context(&rules)
    };
    let result = assess_routes(
        "CCOC(=O)c1ccccc1",
        &routes,
        &ctx,
        &SynthesizabilityConfig::conservative(),
    )
    .unwrap();
    assert_eq!(
        result.route_assessments[0].target_element_accounting_status,
        ElementAccountingStatus::Accounted
    );
    let json = serde_json::to_string(&result.route_assessments[0]).unwrap();
    assert!(json.contains("\"target_element_accounting_status\":\"accounted\""));
    assert_eq!(result.status, AssessmentStatus::RouteSupported);
}

/// Cross-language parity fixture, mirrored verbatim from
/// `scripts/tests/test_compare_validation.py::
/// test_atom_materializing_from_nowhere_is_unaccounted` (target
/// `"Clc1ccccc1"` from precursor `["Brc1ccccc1"]`: chlorobenzene's Cl has no
/// precursor source at all -- an MW-only check would wrongly pass this
/// since bromobenzene is heavier, but the per-element check must not). The
/// Python assertion is `status == "unaccounted_target_element"`; the Rust
/// equivalent is `ElementAccountingStatus::UnaccountedTargetElement`, which
/// serializes to exactly `"unaccounted_target_element"`.
#[test]
fn target_element_unaccounted_chlorobenzene_parity_with_python() {
    let rules = no_rules();
    let routes = vec![route(
        vec![step(
            "some_extracted_rule",
            "rule:some_extracted_rule",
            "Clc1ccccc1",
            &["Brc1ccccc1"],
        )],
        &["Brc1ccccc1"],
    )];
    let stock = vec!["Brc1ccccc1".to_string()];
    let ctx = AssessmentContext {
        stock: Some(&stock),
        ..empty_context(&rules)
    };
    let result = assess_routes(
        "Clc1ccccc1",
        &routes,
        &ctx,
        &SynthesizabilityConfig::conservative(),
    )
    .unwrap();
    assert_eq!(
        result.route_assessments[0].target_element_accounting_status,
        ElementAccountingStatus::UnaccountedTargetElement
    );
    let json = serde_json::to_string(&result.route_assessments[0]).unwrap();
    assert!(json.contains("\"target_element_accounting_status\":\"unaccounted_target_element\""));
    // Not on the reagent-omission allowlist -> conservative() hard-rejects.
    assert!(
        result.route_assessments[0]
            .hard_failures
            .contains(&HardFailure::UnaccountedTargetElement { step_index: 0 })
    );
    assert_eq!(result.status, AssessmentStatus::RoutesFoundButRejected);
}

// ---------------------------------------------------------------------
// 9/10/11. Forward validation
// ---------------------------------------------------------------------

#[test]
fn forward_validation_all_valid_end_to_end() {
    let rules = no_rules();
    let routes = vec![route(
        vec![step("rule:x", "rule:x", "CCO", &["CC=O"])],
        &["CC=O"],
    )];
    let fv: Vec<Option<Vec<StepValidationStatus>>> = vec![Some(vec![StepValidationStatus::Valid])];
    let ctx = AssessmentContext {
        forward_validation: Some(&fv),
        ..empty_context(&rules)
    };
    let result = assess_routes(
        "CCO",
        &routes,
        &ctx,
        &SynthesizabilityConfig::conservative(),
    )
    .unwrap();
    assert_eq!(
        result.route_assessments[0].forward_validation_status,
        ForwardValidationStatus::AllEvaluatedStepsValid
    );
    assert!(result.route_assessments[0].hard_failures.is_empty());
}

/// `OneOrMoreStepsInvalid` is recorded regardless of policy, but only
/// gates (becomes `HardFailure::ForwardValidationFailed`) when the policy
/// requires it -- default `Ignore` never gates.
#[test]
fn forward_validation_one_invalid_step_gated_only_when_policy_requires_it() {
    let rules = no_rules();
    let routes = vec![route(
        vec![step("rule:x", "rule:x", "CCO", &["CC=O"])],
        &["CC=O"],
    )];
    let fv: Vec<Option<Vec<StepValidationStatus>>> =
        vec![Some(vec![StepValidationStatus::Invalid])];
    let ctx = AssessmentContext {
        forward_validation: Some(&fv),
        ..empty_context(&rules)
    };

    // Default policy (Ignore): recorded, but never gates.
    let ignored = assess_routes(
        "CCO",
        &routes,
        &ctx,
        &SynthesizabilityConfig::conservative(),
    )
    .unwrap();
    assert_eq!(
        ignored.route_assessments[0].forward_validation_status,
        ForwardValidationStatus::OneOrMoreStepsInvalid
    );
    assert!(
        !ignored.route_assessments[0]
            .hard_failures
            .iter()
            .any(|hf| matches!(hf, HardFailure::ForwardValidationFailed { .. }))
    );

    // RequireNoInvalid: now a hard failure attributed to the invalid step.
    let mut config = SynthesizabilityConfig::conservative();
    config.forward_validation_policy = ForwardValidationPolicy::RequireNoInvalid;
    let strict = assess_routes("CCO", &routes, &ctx, &config).unwrap();
    assert!(
        strict.route_assessments[0]
            .hard_failures
            .contains(&HardFailure::ForwardValidationFailed { step_index: 0 })
    );
    assert_eq!(strict.status, AssessmentStatus::RoutesFoundButRejected);
}

/// `NotEvaluated` (per-route entry is `None`) is never treated as `Invalid`
/// -- there is no `HardFailure` variant for "not evaluated" at all, only a
/// confirmed `Invalid` step can ever produce
/// `HardFailure::ForwardValidationFailed`. Whether it surfaces as
/// `ValidationGap::ForwardValidationNotRun` depends on policy: **only**
/// `RequireAllValid` records that gap (see `assess_forward_validation` in
/// `assessment.rs`) -- default `Ignore` and `RequireNoInvalid` both tolerate
/// `NotEvaluated` silently (no gap, no failure). This is narrower than "not
/// evaluated always produces a gap" -- verified against the real merged
/// code, not assumed; see this agent's final report.
#[test]
fn forward_validation_not_evaluated_is_never_a_hard_failure_gap_depends_on_policy() {
    let rules = no_rules();
    let routes = vec![route(
        vec![step("rule:x", "rule:x", "CCO", &["CC=O"])],
        &["CC=O"],
    )];
    let fv: Vec<Option<Vec<StepValidationStatus>>> = vec![None]; // per-route entry present but None
    let ctx = AssessmentContext {
        forward_validation: Some(&fv),
        ..empty_context(&rules)
    };

    // Default (Ignore): NotEvaluated, but no gap recorded at all.
    let ignored = assess_routes(
        "CCO",
        &routes,
        &ctx,
        &SynthesizabilityConfig::conservative(),
    )
    .unwrap();
    assert_eq!(
        ignored.route_assessments[0].forward_validation_status,
        ForwardValidationStatus::NotEvaluated
    );
    assert!(
        !ignored.route_assessments[0]
            .validation_gaps
            .contains(&ValidationGap::ForwardValidationNotRun)
    );
    assert!(ignored.route_assessments[0].hard_failures.is_empty());

    // RequireAllValid: NotEvaluated now surfaces as a validation gap --
    // never as a hard failure.
    let mut require_all = SynthesizabilityConfig::conservative();
    require_all.forward_validation_policy = ForwardValidationPolicy::RequireAllValid;
    let strict = assess_routes("CCO", &routes, &ctx, &require_all).unwrap();
    assert!(
        strict.route_assessments[0]
            .validation_gaps
            .contains(&ValidationGap::ForwardValidationNotRun)
    );
    assert!(
        !strict.route_assessments[0]
            .hard_failures
            .iter()
            .any(|hf| matches!(hf, HardFailure::ForwardValidationFailed { .. }))
    );

    // RequireNoInvalid: also tolerated -- no gap, no failure.
    let mut require_no_invalid = SynthesizabilityConfig::conservative();
    require_no_invalid.forward_validation_policy = ForwardValidationPolicy::RequireNoInvalid;
    let tolerant = assess_routes("CCO", &routes, &ctx, &require_no_invalid).unwrap();
    assert!(
        !tolerant.route_assessments[0]
            .validation_gaps
            .contains(&ValidationGap::ForwardValidationNotRun)
    );
    assert!(tolerant.route_assessments[0].hard_failures.is_empty());
}

// ---------------------------------------------------------------------
// 12/13/14. Evidence coverage
// ---------------------------------------------------------------------

#[test]
fn evidence_exact_substrate_present_is_never_gated() {
    let rules = no_rules();
    let evidence = evidence_with(vec![resolved_example(ExampleMatch::ExactSubstrate, "ex1")]);
    let routes = vec![route(
        vec![step_with_evidence(
            "rule:x",
            "rule:x",
            "CCO",
            &["CC=O"],
            evidence,
        )],
        &["CC=O"],
    )];
    let ctx = empty_context(&rules);
    let mut config = SynthesizabilityConfig::conservative();
    config.evidence_policy = EvidencePolicy::RequireExactSubstrate;
    let result = assess_routes("CCO", &routes, &ctx, &config).unwrap();
    let ra = &result.route_assessments[0];
    assert_eq!(ra.evidence_coverage.exact_substrate_evidence_steps, 1);
    assert!(
        !ra.validation_gaps
            .contains(&ValidationGap::NoExactSubstrateEvidence)
    );
    assert!(!ra.validation_gaps.contains(&ValidationGap::NoEvidence));
}

#[test]
fn evidence_template_level_only_gap_present_under_require_exact_substrate_absent_under_ignore() {
    let rules = no_rules();
    let evidence = evidence_with(vec![resolved_example(ExampleMatch::TemplateOnly, "ex1")]);
    let routes = vec![route(
        vec![step_with_evidence(
            "rule:x",
            "rule:x",
            "CCO",
            &["CC=O"],
            evidence,
        )],
        &["CC=O"],
    )];
    let ctx = empty_context(&rules);

    // Default (Ignore): gap never surfaces even though coverage is
    // template-level only, not exact-substrate.
    let ignore_result = assess_routes(
        "CCO",
        &routes,
        &ctx,
        &SynthesizabilityConfig::conservative(),
    )
    .unwrap();
    let ra = &ignore_result.route_assessments[0];
    assert_eq!(ra.evidence_coverage.template_level_evidence_steps, 1);
    assert_eq!(ra.evidence_coverage.exact_substrate_evidence_steps, 0);
    assert!(
        !ra.validation_gaps
            .contains(&ValidationGap::NoExactSubstrateEvidence)
    );

    // RequireExactSubstrate: same route now produces the gap.
    let mut config = SynthesizabilityConfig::conservative();
    config.evidence_policy = EvidencePolicy::RequireExactSubstrate;
    let strict_result = assess_routes("CCO", &routes, &ctx, &config).unwrap();
    assert!(
        strict_result.route_assessments[0]
            .validation_gaps
            .contains(&ValidationGap::NoExactSubstrateEvidence)
    );
}

#[test]
fn evidence_none_at_all_gap_present_under_require_any_absent_under_ignore() {
    let rules = no_rules();
    let routes = vec![route(
        vec![step("rule:x", "rule:x", "CCO", &["CC=O"])], // evidence: None
        &["CC=O"],
    )];
    let ctx = empty_context(&rules);

    let ignore_result = assess_routes(
        "CCO",
        &routes,
        &ctx,
        &SynthesizabilityConfig::conservative(),
    )
    .unwrap();
    let ra = &ignore_result.route_assessments[0];
    assert_eq!(ra.evidence_coverage.steps_without_evidence, 1);
    assert!(!ra.validation_gaps.contains(&ValidationGap::NoEvidence));

    let mut config = SynthesizabilityConfig::conservative();
    config.evidence_policy = EvidencePolicy::RequireAnyEvidence;
    let strict_result = assess_routes("CCO", &routes, &ctx, &config).unwrap();
    assert!(
        strict_result.route_assessments[0]
            .validation_gaps
            .contains(&ValidationGap::NoEvidence)
    );
}

// ---------------------------------------------------------------------
// 15. Route order independence: the *set* of routes assessed together
// produces the same selected_route/status/full assessment list regardless
// of which order they were handed in (route_id itself is already tested at
// the hash-function level by Agent C).
// ---------------------------------------------------------------------

#[test]
fn route_order_independence_selected_route_and_full_assessment_list_match() {
    let rules = no_rules();
    let clean = route(vec![step("rule:x", "rule:x", "CCO", &["CC=O"])], &["CC=O"]);
    let mismatched = route(
        vec![step("rule:y", "rule:y", "CCO", &["Oc1ccccc1"])],
        &["Oc1ccccc1"], // phenol -- not in the configured stock below
    );
    let stock = vec!["CC=O".to_string()];
    let ctx = AssessmentContext {
        stock: Some(&stock),
        ..empty_context(&rules)
    };
    // include_all_route_diagnostics: true -- this test is about the *full*
    // route_assessments list staying order-independent, not just the
    // selected route (which conservative()'s default single-route output
    // would trivially satisfy regardless of ordering).
    let mut config = SynthesizabilityConfig::conservative();
    config.include_all_route_diagnostics = true;

    let forward =
        assess_routes("CCO", &[clean.clone(), mismatched.clone()], &ctx, &config).unwrap();
    let backward = assess_routes("CCO", &[mismatched, clean], &ctx, &config).unwrap();

    assert_eq!(forward.status, backward.status);
    assert_eq!(forward.selected_route, backward.selected_route);
    assert_eq!(forward.route_assessments, backward.route_assessments);
    // Sanity: the sort actually did something (not a trivial 1-route case).
    assert_eq!(forward.route_assessments.len(), 2);
    assert!(forward.route_assessments[0].hard_failures.is_empty());
}

// ---------------------------------------------------------------------
// 16. Step/precursor order independence inside one route, through the real
// public assess_routes entry point (provenance.rs already unit-tests
// compute_route_id directly; this catches any wiring gap between
// assessment.rs and provenance.rs that the lower-level test would miss).
// ---------------------------------------------------------------------

#[test]
fn route_id_is_independent_of_precursor_order_within_a_step_via_public_api() {
    let rules = no_rules();
    let route_a = route(
        vec![step(
            "rule:x",
            "rule:x",
            "CCOC(=O)c1ccccc1",
            &["CCO", "O=C(O)c1ccccc1"],
        )],
        &["CCO", "O=C(O)c1ccccc1"],
    );
    let route_b = route(
        vec![step(
            "rule:x",
            "rule:x",
            "CCOC(=O)c1ccccc1",
            &["O=C(O)c1ccccc1", "CCO"],
        )],
        &["O=C(O)c1ccccc1", "CCO"],
    );
    let ctx = empty_context(&rules);
    let config = SynthesizabilityConfig::conservative();

    let result_a = assess_routes("CCOC(=O)c1ccccc1", &[route_a], &ctx, &config).unwrap();
    let result_b = assess_routes("CCOC(=O)c1ccccc1", &[route_b], &ctx, &config).unwrap();

    assert_eq!(
        result_a.route_assessments[0].route_id,
        result_b.route_assessments[0].route_id
    );
}

// ---------------------------------------------------------------------
// 17. Byte-identical JSON for byte-identical inputs -- nothing time-based
// is ever computed in this module (design doc §6), so this holds with zero
// exceptions. Deliberately NOT the thinnest possible fixture: a single
// clean route with zero hard failures and no sorting would never touch the
// `HashSet<String>` dedup loop in `assess_stock_termination` (whose
// iteration order is per-instance-random in Rust, not just per-process) or
// the §4.8 multi-route `sort_by`. This fixture has one route with TWO
// unmatched leaves (exercises the dedup/collection loop) plus a second,
// clean route (exercises cross-route sorting) so the strongest claim in §6
// actually gets exercised, not just assumed from a trivial case.
// ---------------------------------------------------------------------

#[test]
fn byte_identical_inputs_produce_byte_identical_json() {
    let rules = no_rules();
    let worse = route(
        vec![step(
            "rule:x",
            "rule:x",
            "CCOC(=O)c1ccccc1",
            &["CCN", "CCCl"],
        )],
        &["CCN", "CCCl"], // neither is in the configured stock below
    );
    let better = route(
        vec![step(
            "esterification_retro",
            "rule:esterification_retro",
            "CCOC(=O)c1ccccc1",
            &["CCO", "O=C(O)c1ccccc1"],
        )],
        &["CCO", "O=C(O)c1ccccc1"],
    );
    let routes = vec![worse, better];
    let stock = vec!["CCO".to_string(), "O=C(O)c1ccccc1".to_string()];
    let ctx = AssessmentContext {
        stock: Some(&stock),
        stock_source: Some("data/building_blocks.smi".to_string()),
        template_metadata_hash: Some("sha256:abc".to_string()),
        search_config_summary: Some("depth=6,beam=50,max_routes=20".to_string()),
        git_commit: Some("deadbeef".to_string()),
        embedded_fallback_used: Some(false),
        ..empty_context(&rules)
    };
    // include_all_route_diagnostics: true -- this test wants to see both
    // routes' full diagnostics (see the comment above), not just the
    // selected one.
    let mut config = SynthesizabilityConfig::conservative();
    config.include_all_route_diagnostics = true;

    let result_a = assess_routes("CCOC(=O)c1ccccc1", &routes, &ctx, &config).unwrap();
    let result_b = assess_routes("CCOC(=O)c1ccccc1", &routes, &ctx, &config).unwrap();

    // Sanity: the fixture actually exercises what the comment above claims.
    assert_eq!(result_a.route_assessments.len(), 2);
    assert!(result_a.route_assessments.iter().any(|ra| {
        ra.hard_failures
            .iter()
            .filter(|hf| matches!(hf, HardFailure::StockTerminalMismatch { .. }))
            .count()
            >= 2
    }));

    let json_a = serde_json::to_string(&result_a).unwrap();
    let json_b = serde_json::to_string(&result_b).unwrap();
    assert_eq!(json_a, json_b);
}

// ---------------------------------------------------------------------
// 18. Malformed provenance / kernel-integrity input -> EvaluationError
// (top-level-integration-test version of an existing assessment.rs-internal
// test).
// ---------------------------------------------------------------------

#[test]
fn forward_validation_length_mismatch_is_evaluation_error_top_level() {
    let rules = no_rules();
    let routes = vec![
        route(vec![step("rule:x", "rule:x", "CCO", &["CC=O"])], &["CC=O"]),
        route(vec![step("rule:y", "rule:y", "CCN", &["CCCl"])], &["CCCl"]),
    ];
    // Length 1 != routes.len() == 2 -- index correspondence cannot be trusted.
    let fv: Vec<Option<Vec<StepValidationStatus>>> = vec![Some(vec![StepValidationStatus::Valid])];
    let ctx = AssessmentContext {
        forward_validation: Some(&fv),
        ..empty_context(&rules)
    };
    let result = assess_routes(
        "CCO",
        &routes,
        &ctx,
        &SynthesizabilityConfig::conservative(),
    )
    .expect("a malformed context must surface as a status, not an Err");
    assert_eq!(result.status, AssessmentStatus::EvaluationError);
    assert!(result.route_assessments.is_empty());
    assert!(result.selected_route.is_none());
    assert!(!result.warnings.is_empty());
}

// ---------------------------------------------------------------------
// 19. Embedded stock fallback is explicit, not silent: Some(true)/
// Some(false)/None are echoed verbatim into AssessmentProvenance with no
// substitution/defaulting logic anywhere.
// ---------------------------------------------------------------------

#[test]
fn embedded_fallback_used_is_echoed_verbatim_never_defaulted() {
    let rules = no_rules();
    for input in [None, Some(true), Some(false)] {
        let ctx = AssessmentContext {
            embedded_fallback_used: input,
            ..empty_context(&rules)
        };
        let result =
            assess_routes("CCO", &[], &ctx, &SynthesizabilityConfig::conservative()).unwrap();
        assert_eq!(result.provenance.embedded_fallback_used, input);
    }
}

// ---------------------------------------------------------------------
// 20. PR #70/#71-lineage regression class: a multi-step route where an
// INTERMEDIATE leaf (not the top-level target) is one of the mismatched
// SMILES -- confirms the hard failure surfaces with the correct leaf
// identity even when it's several steps deep. NB: `HardFailure::
// StockTerminalMismatch` carries only `leaf: String`, not a `step_index` --
// there is no per-step attribution field on this variant in the merged
// schema (unlike UnaccountedTargetElement/ForwardValidationFailed, which do
// carry step_index). See this agent's final report.
// ---------------------------------------------------------------------

#[test]
fn issue71_style_mismatch_surfaces_correctly_when_leaf_is_not_the_top_level_target() {
    let rules = no_rules();
    // step 0: overall target "CCO" from precursor "CC=O" (further reduced).
    // step 1: "CC=O" from the two real leaves -- one of which is the exact
    // 1,4-pentadiene false positive, several steps removed from the
    // top-level target "CCO".
    let routes = vec![route(
        vec![
            step("rule:step0", "rule:step0", "CCO", &["CC=O"]),
            step(
                "rule:step1",
                "rule:step1",
                "CC=O",
                &["C=C/C/C=C", "O=C/C(=O)O"],
            ),
        ],
        &["C=C/C/C=C", "O=C/C(=O)O"],
    )];
    // Stock verifies the OTHER leaf but not the false positive -- so exactly
    // one leaf, from the deeper step, must be flagged.
    let stock = vec!["O=C/C(=O)O".to_string()];
    let ctx = AssessmentContext {
        stock: Some(&stock),
        ..empty_context(&rules)
    };
    let result = assess_routes(
        "CCO",
        &routes,
        &ctx,
        &SynthesizabilityConfig::conservative(),
    )
    .unwrap();

    assert_eq!(
        result.route_assessments[0].stock_termination_status,
        StockTerminationStatus::OneOrMoreLeavesNotInStock
    );
    assert!(result.route_assessments[0].hard_failures.contains(
        &HardFailure::StockTerminalMismatch {
            leaf: "C=C/C/C=C".to_string()
        }
    ));
    assert!(!result.route_assessments[0].hard_failures.contains(
        &HardFailure::StockTerminalMismatch {
            leaf: "O=C/C(=O)O".to_string()
        }
    ));
    // The flagged leaf is not the top-level target -- this is the whole
    // point of the regression class (a leaf several steps deep, not the
    // molecule the caller asked about).
    assert_ne!(result.canonical_target.unwrap(), "C=C/C/C=C");
    assert_eq!(result.status, AssessmentStatus::RoutesFoundButRejected);
}

// ---------------------------------------------------------------------
// Additional: AccountingFailurePolicy::HardFailure vs ValidationGap
// end-to-end via the public API only (independent fixture from
// assessment.rs's own non_allowlisted_accounting_failure_.../
// allowlisted_accounting_failure_... tests -- same chlorobenzene/
// bromobenzene parity chemistry as items 7/8 above, but different template
// ids, so this is a genuinely separate fixture, not a copy).
// ---------------------------------------------------------------------

#[test]
fn allowlisted_accounting_failure_is_a_validation_gap_end_to_end() {
    let rules = no_rules();
    let routes = vec![route(
        vec![step(
            "cbz_deprotection_retro",
            "rule:cbz_deprotection_retro",
            "Clc1ccccc1",
            &["Brc1ccccc1"],
        )],
        &["Brc1ccccc1"],
    )];
    let stock = vec!["Brc1ccccc1".to_string()];
    let ctx = AssessmentContext {
        stock: Some(&stock),
        ..empty_context(&rules)
    };
    let result = assess_routes(
        "Clc1ccccc1",
        &routes,
        &ctx,
        &SynthesizabilityConfig::conservative(),
    )
    .unwrap();
    assert_eq!(
        result.status,
        AssessmentStatus::RouteSupportedWithValidationGaps
    );
    let selected = result.selected_route.unwrap();
    assert!(selected.hard_failures.is_empty());
    assert!(selected.validation_gaps.iter().any(|g| matches!(
        g,
        ValidationGap::ReagentOmissionAccountingGap { template_id, .. }
            if template_id == "rule:cbz_deprotection_retro"
    )));
}

#[test]
fn non_allowlisted_accounting_failure_is_hard_failure_under_conservative_and_gap_under_diagnostic_end_to_end()
 {
    let rules = no_rules();
    let routes = vec![route(
        vec![step(
            "custom_extracted_rule",
            "rule:custom_extracted_rule",
            "Clc1ccccc1",
            &["Brc1ccccc1"],
        )],
        &["Brc1ccccc1"],
    )];
    let stock = vec!["Brc1ccccc1".to_string()];
    let ctx = AssessmentContext {
        stock: Some(&stock),
        ..empty_context(&rules)
    };

    let conservative = assess_routes(
        "Clc1ccccc1",
        &routes,
        &ctx,
        &SynthesizabilityConfig::conservative(),
    )
    .unwrap();
    assert_eq!(
        conservative.status,
        AssessmentStatus::RoutesFoundButRejected
    );
    assert!(
        conservative.route_assessments[0]
            .hard_failures
            .contains(&HardFailure::UnaccountedTargetElement { step_index: 0 })
    );

    let diagnostic = assess_routes(
        "Clc1ccccc1",
        &routes,
        &ctx,
        &SynthesizabilityConfig::diagnostic(),
    )
    .unwrap();
    assert_eq!(
        diagnostic.status,
        AssessmentStatus::RouteSupportedWithValidationGaps
    );
    assert!(
        diagnostic.route_assessments[0]
            .validation_gaps
            .contains(&ValidationGap::UnaccountedTargetElementNotEnforced { step_index: 0 })
    );
}

/// Design doc §9's explicit allowlist fixture *pair*: alongside the
/// allowlisted (`cbz_deprotection_retro`) and generic-non-allowlisted
/// (`custom_extracted_rule`) cases above, `rule:aryl_amine_retro`
/// specifically must ALSO be a hard failure under `conservative()` -- issue
/// #73 is unresolved, and unlike Boc/Cbz this rule has no existing
/// graph-rule exact-formula carve-out (see
/// `SynthesizabilityConfig::default_reagent_omission_allowlist` in
/// schema.rs, and `conservative_config_has_documented_defaults` in
/// schema.rs's own tests, which asserts its *absence* from the allowlist).
/// This test pins the behavioral *consequence* of that absence end-to-end:
/// a generic non-allowlisted-template test (like the one above) would keep
/// passing even if `aryl_amine_retro` were quietly added to the default
/// allowlist later -- only a test naming this exact template id catches
/// that regression.
#[test]
fn aryl_amine_retro_accounting_failure_is_a_hard_failure_not_allowlisted_per_issue_73() {
    let rules = no_rules();
    let routes = vec![route(
        vec![step(
            "aryl_amine_retro",
            "rule:aryl_amine_retro",
            "Clc1ccccc1",
            &["Brc1ccccc1"],
        )],
        &["Brc1ccccc1"],
    )];
    let stock = vec!["Brc1ccccc1".to_string()];
    let ctx = AssessmentContext {
        stock: Some(&stock),
        ..empty_context(&rules)
    };
    let result = assess_routes(
        "Clc1ccccc1",
        &routes,
        &ctx,
        &SynthesizabilityConfig::conservative(),
    )
    .unwrap();
    assert_eq!(result.status, AssessmentStatus::RoutesFoundButRejected);
    assert!(
        result.route_assessments[0]
            .hard_failures
            .contains(&HardFailure::UnaccountedTargetElement { step_index: 0 })
    );
    assert!(
        !result.route_assessments[0]
            .validation_gaps
            .iter()
            .any(|g| matches!(g, ValidationGap::ReagentOmissionAccountingGap { .. }))
    );
    // Belt-and-suspenders: pin the allowlist itself too, so this test fails
    // loudly (rather than silently passing for the wrong reason) if
    // aryl_amine_retro is ever added to the default allowlist without this
    // test being updated in lockstep.
    assert!(
        !SynthesizabilityConfig::conservative()
            .reagent_omission_template_allowlist
            .iter()
            .any(|t| t == "rule:aryl_amine_retro")
    );
}

// ---------------------------------------------------------------------
// Additional: max_routes_to_assess truncates the *sorted* (best-first)
// list, not the caller-supplied order -- independent fixture from
// assessment.rs's own max_routes_to_assess_truncation_never_drops_the_
// best_route test.
// ---------------------------------------------------------------------

#[test]
fn max_routes_to_assess_truncates_to_the_best_route_regardless_of_input_order() {
    let rules = no_rules();
    let worse = route(
        vec![step("rule:x", "rule:x", "CCO", &["CC=O", "CCN"])],
        &["CC=O", "CCN"], // "CCN" is not in the configured stock below
    );
    let better = route(vec![step("rule:y", "rule:y", "CCO", &["CC=O"])], &["CC=O"]);
    let stock = vec!["CC=O".to_string()];
    let ctx = AssessmentContext {
        stock: Some(&stock),
        ..empty_context(&rules)
    };
    let mut config = SynthesizabilityConfig::conservative();
    // Exercise max_routes_to_assess's own truncation, not
    // include_all_route_diagnostics' default single-route cap.
    config.include_all_route_diagnostics = true;
    config.max_routes_to_assess = 1;

    // Worse (broken) route supplied FIRST in caller order.
    let result = assess_routes("CCO", &[worse, better], &ctx, &config).unwrap();
    assert_eq!(result.route_assessments.len(), 1);
    let survivor = &result.route_assessments[0];
    assert!(survivor.hard_failures.is_empty());
    assert_eq!(
        survivor.stock_termination_status,
        StockTerminationStatus::AllLeavesVerifiedInConfiguredStock
    );
    assert_eq!(result.status, AssessmentStatus::RouteSupported);
    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.contains("max_routes_to_assess"))
    );
}

// ---------------------------------------------------------------------
// Additional: stock_source echoed into provenance -- independent
// top-level version of an existing assessment.rs-internal test.
// ---------------------------------------------------------------------

#[test]
fn stock_source_label_echoed_into_provenance_top_level() {
    let rules = no_rules();
    let routes = vec![route(
        vec![step("rule:x", "rule:x", "CCO", &["CC=O"])],
        &["CC=O"],
    )];
    let stock = vec!["CC=O".to_string()];
    let ctx = AssessmentContext {
        stock: Some(&stock),
        stock_source: Some("embedded".to_string()),
        ..empty_context(&rules)
    };
    let result = assess_routes(
        "CCO",
        &routes,
        &ctx,
        &SynthesizabilityConfig::conservative(),
    )
    .unwrap();
    assert_eq!(result.provenance.stock_source, Some("embedded".to_string()));
}

// ---------------------------------------------------------------------
// Additional: full-provenance smoke test -- every AssessmentProvenance /
// SynthesizabilityConfigSummary field populated as expected for a normal,
// fully-supported run.
// ---------------------------------------------------------------------

#[test]
fn full_provenance_smoke_test_every_field_populated_for_a_normal_run() {
    let rules = vec![RetroRule {
        name: "esterification_retro".to_string(),
        template_id: "rule:esterification_retro".to_string(),
        smirks: "[C:1](=O)O.[O:2]>>[C:1](=O)[O:2]".to_string(),
        weight: 1.0,
        required_elements: 0,
    }];
    let routes = vec![route(
        vec![step(
            "esterification_retro",
            "rule:esterification_retro",
            "CCOC(=O)c1ccccc1",
            &["CCO", "O=C(O)c1ccccc1"],
        )],
        &["CCO", "O=C(O)c1ccccc1"],
    )];
    let stock = vec!["CCO".to_string(), "O=C(O)c1ccccc1".to_string()];
    let ctx = AssessmentContext {
        stock: Some(&stock),
        stock_source: Some("data/building_blocks.smi".to_string()),
        template_metadata_hash: Some("sha256:meta".to_string()),
        search_config_summary: Some("depth=6,beam=50,max_routes=20".to_string()),
        git_commit: Some("deadbeef".to_string()),
        embedded_fallback_used: Some(false),
        forward_validation: None,
        rules: &rules,
    };
    let config = SynthesizabilityConfig::conservative();
    let result = assess_routes("CCOC(=O)c1ccccc1", &routes, &ctx, &config).unwrap();

    assert_eq!(result.status, AssessmentStatus::RouteSupported);

    let prov = &result.provenance;
    assert!(!prov.renkin_version.is_empty());
    assert_eq!(
        prov.assessment_schema_version,
        SYNTHESIZABILITY_SCHEMA_VERSION
    );
    assert_eq!(
        prov.canonical_target,
        result.canonical_target.clone().unwrap()
    );
    assert_eq!(prov.rules_count, 1);
    assert!(prov.rules_hash.starts_with("sha256:"));
    assert_eq!(prov.stock_count, 2);
    assert!(prov.stock_hash.starts_with("sha256:"));
    assert_eq!(
        prov.stock_source,
        Some("data/building_blocks.smi".to_string())
    );
    assert_eq!(prov.template_metadata_hash, Some("sha256:meta".to_string()));
    assert_eq!(
        prov.search_config_summary,
        Some("depth=6,beam=50,max_routes=20".to_string())
    );
    assert!(prov.assessment_config_hash.starts_with("sha256:"));
    assert_eq!(prov.git_commit, Some("deadbeef".to_string()));
    assert_eq!(prov.embedded_fallback_used, Some(false));
    assert!(prov.reproducibility_hash.starts_with("sha256:"));
    assert!(!prov.reproducibility_exclusions.is_empty());
    assert!(
        prov.reproducibility_exclusions
            .contains(&"timing_ms".to_string())
    );
    assert!(
        prov.reproducibility_exclusions
            .contains(&"wall_clock_timestamp".to_string())
    );

    let cfg = &result.config_used;
    assert_eq!(
        cfg.require_verified_stock_terminal,
        config.require_verified_stock_terminal
    );
    assert_eq!(
        cfg.require_target_element_accounting,
        config.require_target_element_accounting
    );
    assert_eq!(cfg.max_routes_to_assess, config.max_routes_to_assess);
    assert_eq!(
        cfg.reagent_omission_template_allowlist,
        config.reagent_omission_template_allowlist
    );
}
