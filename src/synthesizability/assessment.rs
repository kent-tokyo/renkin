//! `assess_routes()`: the Synthesizability Kernel's policy/decision layer
//! (`docs/design/synthesizability-kernel-v0.md` §4). Evaluates
//! already-produced `search::Route`s -- it never runs search itself, never
//! touches `search::find_routes`, and never modifies `search.rs`/
//! `chem_env.rs`/`evidence.rs`/`validation/*`.
//!
//! Signal computation (stock canonicalization, forward-validation rollup,
//! evidence tallying, element accounting) is delegated to `signals.rs`/
//! `element_accounting.rs` (Agent B); this file owns turning those signals
//! plus `SynthesizabilityConfig` policy into `HardFailure`/`ValidationGap`
//! lists, the overall `AssessmentStatus`, and route selection (§4.8).

use std::collections::HashSet;

use crate::chem_env;
use crate::search::Route;
use crate::synthesizability::schema::{
    AccountingFailurePolicy, AssessmentProvenance, AssessmentStatus, EvidencePolicy,
    ForwardValidationPolicy, ForwardValidationStatus, HardFailure, RouteAssessment,
    StockTerminationStatus, SynthesizabilityAssessment, SynthesizabilityConfig,
    SynthesizabilityConfigSummary, SYNTHESIZABILITY_SCHEMA_VERSION, ValidationGap,
};
use crate::synthesizability::{element_accounting, provenance, signals};
use crate::validation::StepValidationStatus;

/// Everything `assess_routes()` needs beyond the routes themselves and the
/// config -- caller-supplied inputs the kernel does not compute itself
/// (design doc §5, §5.1: `git_commit`/`embedded_fallback_used` are NEVER
/// computed in-crate, wasm32 constraint).
pub struct AssessmentContext<'a> {
    pub stock: Option<&'a [String]>,
    pub template_metadata_hash: Option<String>,
    pub search_config_summary: Option<String>,
    pub git_commit: Option<String>,
    pub embedded_fallback_used: Option<bool>,
    /// One entry per route in `routes` (same index), each an optional
    /// per-step forward-validation status list for that route.
    pub forward_validation: Option<&'a [Option<Vec<StepValidationStatus>>]>,
    pub rules: &'a [crate::chem_env::RetroRule],
}

/// Evaluates `routes` for `target` under `context`/`config` and returns a
/// decisive, auditable [`SynthesizabilityAssessment`] (design doc §4.1's
/// status decision order). `Err` is reserved for a genuine internal
/// invariant violation that prevents returning *any* assessment at all
/// (there are none reachable in this function -- see the doc comment on
/// [`AssessmentStatus::EvaluationError`] for why a bad target/stock/route
/// input still produces `Ok` with that status, not an `Err`).
pub fn assess_routes(
    target: &str,
    routes: &[Route],
    context: &AssessmentContext,
    config: &SynthesizabilityConfig,
) -> anyhow::Result<SynthesizabilityAssessment> {
    let mut warnings: Vec<String> = Vec::new();

    // Inputs that don't depend on target validity, computed unconditionally
    // so every returned assessment (even InvalidTarget/EvaluationError) is
    // fully self-describing for audit purposes.
    let rules_count = context.rules.len();
    let rules_hash = provenance::compute_rules_hash(context.rules);
    let canonicalized_stock = signals::canonicalize_stock(context.stock);
    let stock_count = canonicalized_stock.count;
    let stock_hash = canonicalized_stock.hash.clone();
    if !canonicalized_stock.unparseable_raw_entries.is_empty() {
        warnings.push(format!(
            "{} stock entr{} failed to parse/canonicalize under the kernel's own pipeline and were excluded from the canonicalized stock set",
            canonicalized_stock.unparseable_raw_entries.len(),
            if canonicalized_stock.unparseable_raw_entries.len() == 1 { "y" } else { "ies" }
        ));
    }
    let assessment_config_hash = provenance::compute_assessment_config_hash(config);
    let config_used = SynthesizabilityConfigSummary {
        require_verified_stock_terminal: config.require_verified_stock_terminal,
        require_target_element_accounting: config.require_target_element_accounting,
        forward_validation_policy: config.forward_validation_policy,
        evidence_policy: config.evidence_policy,
        reagent_omission_template_allowlist: config.reagent_omission_template_allowlist.clone(),
        accounting_failure_policy: config.accounting_failure_policy,
        max_routes_to_assess: config.max_routes_to_assess,
        include_all_route_diagnostics: config.include_all_route_diagnostics,
    };

    // 1. Target SMILES must parse -- checked before anything else (§4.1 #1).
    let canonical_target = match chem_env::mol_from_smiles(target)
        .ok()
        .map(|mol| chem_env::to_canonical(&mol))
    {
        Some(c) => c,
        None => {
            return Ok(early_assessment(
                target,
                AssessmentStatus::InvalidTarget,
                None,
                rules_count,
                rules_hash,
                stock_count,
                stock_hash,
                context,
                assessment_config_hash,
                config_used,
                warnings,
            ));
        }
    };

    // 2. Kernel-integrity invariant: `forward_validation`, if supplied, must
    // have exactly one entry per route in `routes` (design doc §4.1 #2 --
    // "internal invariant violation", never a chemistry judgment).
    if let Some(fv) = context.forward_validation
        && fv.len() != routes.len()
    {
        warnings.push(format!(
            "AssessmentContext::forward_validation has {} entries but {} routes were supplied -- index correspondence cannot be trusted",
            fv.len(),
            routes.len()
        ));
        return Ok(early_assessment(
            target,
            AssessmentStatus::EvaluationError,
            Some(canonical_target),
            rules_count,
            rules_hash,
            stock_count,
            stock_hash,
            context,
            assessment_config_hash,
            config_used,
            warnings,
        ));
    }

    // 3. Zero routes supplied (design doc §4.1 #3, §3: with today's
    // `find_routes`, always the correct classification -- see design doc).
    if routes.is_empty() {
        return Ok(early_assessment(
            target,
            AssessmentStatus::NoRouteFoundWithinBudget,
            Some(canonical_target),
            rules_count,
            rules_hash,
            stock_count,
            stock_hash,
            context,
            assessment_config_hash,
            config_used,
            warnings,
        ));
    }

    // Truncate to `max_routes_to_assess` (a prefix, so indices into
    // `context.forward_validation` stay aligned with the kept routes).
    let assess_count = routes.len().min(config.max_routes_to_assess);
    if assess_count < routes.len() {
        warnings.push(format!(
            "Assessed only the first {assess_count} of {} supplied routes (max_routes_to_assess = {})",
            routes.len(),
            config.max_routes_to_assess
        ));
    }
    if assess_count == 0 {
        warnings.push(
            "max_routes_to_assess truncated every supplied route away; zero routes were actually assessed".to_string(),
        );
    }
    let assessed_routes = &routes[..assess_count];

    let mut route_assessments: Vec<RouteAssessment> = Vec::with_capacity(assessed_routes.len());
    for (idx, route) in assessed_routes.iter().enumerate() {
        route_assessments.push(assess_one_route(
            idx,
            route,
            &canonical_target,
            context,
            config,
            &canonicalized_stock,
        ));
    }

    // §4.8: lexicographic sort, best route first.
    route_assessments.sort_by(cmp_route_assessments);

    // §4.1 #5-#7: status + selection, both read off the now-sorted list's
    // first entry (guaranteed to hold the minimum `hard_failures.len()`
    // across all assessed routes by construction of the sort key).
    let (status, selected_route) = match route_assessments.first() {
        Some(best) if best.hard_failures.is_empty() => {
            let status = if best.validation_gaps.is_empty() {
                AssessmentStatus::RouteSupported
            } else {
                AssessmentStatus::RouteSupportedWithValidationGaps
            };
            (status, Some(best.clone()))
        }
        _ => (AssessmentStatus::RoutesFoundButRejected, None),
    };

    let reproducibility_hash = provenance::compute_reproducibility_hash(
        &rules_hash,
        &stock_hash,
        &assessment_config_hash,
        &canonical_target,
        &route_assessments,
    );

    Ok(SynthesizabilityAssessment {
        schema_version: SYNTHESIZABILITY_SCHEMA_VERSION,
        target: target.to_string(),
        canonical_target: Some(canonical_target.clone()),
        status,
        selected_route,
        route_assessments,
        provenance: AssessmentProvenance {
            renkin_version: env!("CARGO_PKG_VERSION").to_string(),
            assessment_schema_version: SYNTHESIZABILITY_SCHEMA_VERSION,
            canonical_target,
            rules_count,
            rules_hash,
            stock_count,
            stock_hash,
            // `AssessmentContext` carries no caller-supplied stock-source
            // label field today -- see this agent's final report.
            stock_source: None,
            template_metadata_hash: context.template_metadata_hash.clone(),
            search_config_summary: context.search_config_summary.clone(),
            assessment_config_hash,
            git_commit: context.git_commit.clone(),
            embedded_fallback_used: context.embedded_fallback_used,
            reproducibility_hash,
            reproducibility_exclusions: vec![
                "timing_ms".to_string(),
                "wall_clock_timestamp".to_string(),
            ],
        },
        config_used,
        warnings,
    })
}

/// Builds the short-circuit assessment for `InvalidTarget`/`EvaluationError`/
/// `NoRouteFoundWithinBudget` (design doc §4.1 #1-#3): no routes were (or
/// safely could be) assessed, but the provenance record is still complete
/// and self-describing.
#[allow(clippy::too_many_arguments)]
fn early_assessment(
    target: &str,
    status: AssessmentStatus,
    canonical_target: Option<String>,
    rules_count: usize,
    rules_hash: String,
    stock_count: usize,
    stock_hash: String,
    context: &AssessmentContext,
    assessment_config_hash: String,
    config_used: SynthesizabilityConfigSummary,
    warnings: Vec<String>,
) -> SynthesizabilityAssessment {
    // `AssessmentProvenance::canonical_target` is a required `String`, not
    // `Option<String>` -- on `InvalidTarget` there is nothing to
    // canonicalize, so this is `""`, documented here rather than fabricated.
    let canonical_target_for_hash = canonical_target.clone().unwrap_or_default();
    let route_assessments: Vec<RouteAssessment> = Vec::new();
    let reproducibility_hash = provenance::compute_reproducibility_hash(
        &rules_hash,
        &stock_hash,
        &assessment_config_hash,
        &canonical_target_for_hash,
        &route_assessments,
    );

    SynthesizabilityAssessment {
        schema_version: SYNTHESIZABILITY_SCHEMA_VERSION,
        target: target.to_string(),
        canonical_target,
        status,
        selected_route: None,
        route_assessments,
        provenance: AssessmentProvenance {
            renkin_version: env!("CARGO_PKG_VERSION").to_string(),
            assessment_schema_version: SYNTHESIZABILITY_SCHEMA_VERSION,
            canonical_target: canonical_target_for_hash,
            rules_count,
            rules_hash,
            stock_count,
            stock_hash,
            stock_source: None,
            template_metadata_hash: context.template_metadata_hash.clone(),
            search_config_summary: context.search_config_summary.clone(),
            assessment_config_hash,
            git_commit: context.git_commit.clone(),
            embedded_fallback_used: context.embedded_fallback_used,
            reproducibility_hash,
            reproducibility_exclusions: vec![
                "timing_ms".to_string(),
                "wall_clock_timestamp".to_string(),
            ],
        },
        config_used,
        warnings,
    }
}

/// Assesses a single route: computes every §4.2-§4.5 signal, then applies
/// `SynthesizabilityConfig` policy to turn them into `hard_failures`/
/// `validation_gaps` (§4.6). Two checks -- route-structure parseability and
/// intra-route graph connectivity -- are the kernel's own basic integrity
/// checks (no design-doc algorithm specified for either; see this agent's
/// final report) and are unconditional, never gated by config.
fn assess_one_route(
    route_index: usize,
    route: &Route,
    canonical_target: &str,
    context: &AssessmentContext,
    config: &SynthesizabilityConfig,
    canonicalized_stock: &signals::CanonicalizedStock,
) -> RouteAssessment {
    let mut hard_failures: Vec<HardFailure> = Vec::new();
    let mut validation_gaps: Vec<ValidationGap> = Vec::new();
    let mut route_warnings: Vec<String> = Vec::new();

    check_route_structure(route, &mut hard_failures, &mut route_warnings);

    let stock_termination_status = assess_stock_termination(
        route,
        config,
        canonicalized_stock,
        &mut hard_failures,
        &mut validation_gaps,
        &mut route_warnings,
    );

    let target_element_accounting_status = assess_element_accounting(
        route,
        config,
        &mut hard_failures,
        &mut validation_gaps,
        &mut route_warnings,
    );

    let forward_validation_status = assess_forward_validation(
        route_index,
        route,
        context,
        config,
        &mut hard_failures,
        &mut validation_gaps,
        &mut route_warnings,
    );

    let evidence_coverage = signals::compute_evidence_coverage(route);
    assess_evidence(route, &evidence_coverage, config, &mut validation_gaps);

    let route_id = provenance::compute_route_id(canonical_target, route);

    RouteAssessment {
        route_id,
        route_depth: route.depth,
        route_cost: route.route_cost,
        stock_termination_status,
        target_element_accounting_status,
        forward_validation_status,
        evidence_coverage,
        hard_failures,
        validation_gaps,
        warnings: route_warnings,
    }
}

/// Kernel-internal structural sanity checks -- always run, never gated by
/// config (design doc §4.6 lists `RouteStructureUnparseable`/
/// `RouteGraphInconsistent` without a config toggle; unlike stock/
/// accounting/forward-validation/evidence, there is no "policy" under which
/// a structurally broken route becomes acceptable).
fn check_route_structure(
    route: &Route,
    hard_failures: &mut Vec<HardFailure>,
    route_warnings: &mut Vec<String>,
) {
    // Re-parse every step's target/precursor SMILES under the kernel's own
    // pipeline (design doc §4.2's "independent re-verification" philosophy,
    // applied to route structure, not just stock identity) rather than
    // trusting `search::Route`'s strings are already well-formed.
    let any_unparseable = route.steps.iter().any(|step| {
        provenance::try_canonicalize(&step.target).is_none()
            || step
                .precursors
                .iter()
                .any(|p| provenance::try_canonicalize(p).is_none())
    });
    if any_unparseable {
        hard_failures.push(HardFailure::RouteStructureUnparseable);
    }

    if route.steps.is_empty() {
        // A zero-step route (`search::find_routes` produces one when the
        // target itself is already a stock hit -- `search.rs`'s
        // `collect_path(None)` case) is a legal route, not an
        // inconsistency. `Route::building_blocks` is empty in this case
        // too (`extract_building_blocks` has no steps to scan), so the
        // stock-termination check below is vacuous for it; flagged here
        // rather than silently unnoticed.
        route_warnings.push(
            "Route has zero steps (the target itself was classified as a stock hit by search); stock-termination verification for this route is vacuous.".to_string(),
        );
        return;
    }

    // Every precursor referenced by any step must either be a declared
    // leaf (`building_blocks`) or the target of some other step in this
    // same route -- otherwise the route's graph doesn't actually connect,
    // independent of any single step's chemistry.
    let leaves: HashSet<&str> = route.building_blocks.iter().map(String::as_str).collect();
    let targets: HashSet<&str> = route.steps.iter().map(|s| s.target.as_str()).collect();
    let disconnected = route
        .steps
        .iter()
        .flat_map(|s| s.precursors.iter())
        .any(|p| !leaves.contains(p.as_str()) && !targets.contains(p.as_str()));
    if disconnected {
        hard_failures.push(HardFailure::RouteGraphInconsistent);
    }
}

/// §4.2 policy: `require_verified_stock_terminal` gates whether the check
/// even runs at all -- when `false`, `signals::check_stock_termination` is
/// never called (per Agent B's contract) and the status is set directly,
/// contributing nothing to `hard_failures`/`validation_gaps` (mirrors the
/// "always compute the record, config decides how much it counts" pattern
/// used for every other §4.7-gated dimension in this file).
fn assess_stock_termination(
    route: &Route,
    config: &SynthesizabilityConfig,
    canonicalized_stock: &signals::CanonicalizedStock,
    hard_failures: &mut Vec<HardFailure>,
    validation_gaps: &mut Vec<ValidationGap>,
    route_warnings: &mut Vec<String>,
) -> StockTerminationStatus {
    if !config.require_verified_stock_terminal {
        return StockTerminationStatus::StockCheckNotPerformed;
    }

    let result = signals::check_stock_termination(&route.building_blocks, canonicalized_stock);
    match result.status {
        StockTerminationStatus::AllLeavesVerifiedInConfiguredStock => {}
        StockTerminationStatus::OneOrMoreLeavesNotInStock
        | StockTerminationStatus::StockIdentityUnavailable
        | StockTerminationStatus::StockCheckError => {
            // Dedup: a leaf could in principle appear in both lists.
            let mut seen: HashSet<String> = HashSet::new();
            for leaf in result
                .unmatched_leaves
                .iter()
                .chain(result.unparseable_leaves.iter())
            {
                if seen.insert(leaf.clone()) {
                    hard_failures.push(HardFailure::StockTerminalMismatch { leaf: leaf.clone() });
                }
            }
        }
        StockTerminationStatus::StockNotSupplied => {
            // Nothing was supplied to verify against at all -- distinct
            // from a genuine mismatch (never "assume it's fine", design
            // doc §4.2/§4.7), recorded as a validation gap rather than a
            // hard failure: the caller didn't claim these leaves are in
            // stock, the kernel just can't confirm or deny it.
            validation_gaps.push(ValidationGap::StockProvenanceHashMissing);
        }
        StockTerminationStatus::StockCheckNotPerformed => {
            // Per Agent B's contract, `check_stock_termination` never
            // returns this on its own -- reaching here means the contract
            // was violated at merge time, not a chemistry judgment.
            route_warnings.push(
                "signals::check_stock_termination unexpectedly returned StockCheckNotPerformed while the check was required".to_string(),
            );
        }
    }
    result.status
}

/// §4.5 + §4.7 policy: `require_target_element_accounting` gates whether
/// accounting failures are collected as failures/gaps at all --
/// `element_accounting::compute_element_accounting` always runs (so the
/// status is always on the record), but if the config says this dimension
/// doesn't count, nothing is added to `hard_failures`/`validation_gaps`,
/// including the allowlisted (reagent-omission) case.
fn assess_element_accounting(
    route: &Route,
    config: &SynthesizabilityConfig,
    hard_failures: &mut Vec<HardFailure>,
    validation_gaps: &mut Vec<ValidationGap>,
    route_warnings: &mut Vec<String>,
) -> crate::synthesizability::schema::ElementAccountingStatus {
    let result = element_accounting::compute_element_accounting(route);
    if !config.require_target_element_accounting {
        return result.status;
    }

    let mut failing = result.failing_step_indices.clone();
    failing.sort_unstable();
    failing.dedup();
    for step_index in failing {
        let Some(step) = route.steps.get(step_index) else {
            route_warnings.push(format!(
                "element_accounting reported a failing step_index {step_index} out of range for this route ({} steps)",
                route.steps.len()
            ));
            continue;
        };
        if config
            .reagent_omission_template_allowlist
            .contains(&step.template_id)
        {
            // Unconditional on `accounting_failure_policy` (design doc
            // §4.5): an allowlisted, intentionally-unmodeled reagent
            // omission is never a hard failure, even under
            // `conservative()`.
            validation_gaps.push(ValidationGap::ReagentOmissionAccountingGap {
                step_index,
                template_id: step.template_id.clone(),
            });
        } else {
            match config.accounting_failure_policy {
                AccountingFailurePolicy::HardFailure => {
                    hard_failures.push(HardFailure::UnaccountedTargetElement { step_index });
                }
                AccountingFailurePolicy::ValidationGap => {
                    validation_gaps.push(ValidationGap::UnaccountedTargetElementNotEnforced {
                        step_index,
                    });
                }
            }
        }
    }
    result.status
}

/// §4.3 + §4.7 policy. Reads `context.forward_validation[route_index]`
/// directly (not just the rolled-up status) so per-step `Invalid`/
/// `NotEvaluable` entries can be attributed to a `step_index`.
fn assess_forward_validation(
    route_index: usize,
    route: &Route,
    context: &AssessmentContext,
    config: &SynthesizabilityConfig,
    hard_failures: &mut Vec<HardFailure>,
    validation_gaps: &mut Vec<ValidationGap>,
    route_warnings: &mut Vec<String>,
) -> ForwardValidationStatus {
    let per_step: Option<&[StepValidationStatus]> = context
        .forward_validation
        .and_then(|all| all.get(route_index))
        .and_then(|opt| opt.as_deref());

    let status = signals::rollup_forward_validation(per_step, route.steps.len());

    if config.forward_validation_policy == ForwardValidationPolicy::Ignore {
        return status;
    }

    let invalid_step_indices = |steps: &[StepValidationStatus]| -> Vec<usize> {
        steps
            .iter()
            .enumerate()
            .filter(|(_, s)| **s == StepValidationStatus::Invalid)
            .map(|(i, _)| i)
            .collect()
    };
    let not_evaluable_step_indices = |steps: &[StepValidationStatus]| -> Vec<usize> {
        steps
            .iter()
            .enumerate()
            .filter(|(_, s)| **s == StepValidationStatus::NotEvaluable)
            .map(|(i, _)| i)
            .collect()
    };

    match config.forward_validation_policy {
        ForwardValidationPolicy::Ignore => unreachable!("handled above"),
        ForwardValidationPolicy::RequireAllValid => match status {
            ForwardValidationStatus::AllEvaluatedStepsValid => {}
            ForwardValidationStatus::OneOrMoreStepsInvalid => {
                if let Some(steps) = per_step {
                    for step_index in invalid_step_indices(steps) {
                        hard_failures.push(HardFailure::ForwardValidationFailed { step_index });
                    }
                }
            }
            ForwardValidationStatus::NotEvaluated => {
                validation_gaps.push(ValidationGap::ForwardValidationNotRun);
            }
            ForwardValidationStatus::PartiallyEvaluated => {
                if let Some(steps) = per_step {
                    for step_index in not_evaluable_step_indices(steps) {
                        validation_gaps.push(ValidationGap::StepNotEvaluable { step_index });
                    }
                }
            }
            ForwardValidationStatus::ValidatorError => {
                route_warnings.push(
                    "forward-validation input was structurally inconsistent with this route (e.g. wrong step count); RequireAllValid could not be enforced".to_string(),
                );
            }
        },
        ForwardValidationPolicy::RequireNoInvalid => {
            if status == ForwardValidationStatus::OneOrMoreStepsInvalid
                && let Some(steps) = per_step
            {
                for step_index in invalid_step_indices(steps) {
                    hard_failures.push(HardFailure::ForwardValidationFailed { step_index });
                }
            }
            // AllEvaluatedStepsValid / NotEvaluated / PartiallyEvaluated /
            // ValidatorError: tolerated by this policy (design doc §4.3),
            // no failure or gap recorded.
        }
    }
    status
}

/// §4.4 + §4.7 policy. `EvidenceCoverage` is an aggregate (route-level)
/// tally, not a per-step list, so both gap variants here are route-level
/// (unit-shaped), matching [`ValidationGap::NoEvidence`]/
/// [`ValidationGap::NoExactSubstrateEvidence`]'s own shape.
fn assess_evidence(
    route: &Route,
    coverage: &crate::synthesizability::schema::EvidenceCoverage,
    config: &SynthesizabilityConfig,
    validation_gaps: &mut Vec<ValidationGap>,
) {
    match config.evidence_policy {
        EvidencePolicy::Ignore => {}
        EvidencePolicy::RequireAnyEvidence => {
            if coverage.steps_without_evidence > 0 {
                validation_gaps.push(ValidationGap::NoEvidence);
            }
        }
        EvidencePolicy::RequireExactSubstrate => {
            if coverage.exact_substrate_evidence_steps < route.steps.len() {
                validation_gaps.push(ValidationGap::NoExactSubstrateEvidence);
            }
        }
    }
}

/// Design doc §4.8's 9-key lexicographic route-selection order, ascending
/// (best route first): fewer `hard_failures`, then fewer `validation_gaps`,
/// then stock/accounting/forward-validation/evidence signal quality, then
/// the existing `route_cost`/`route_depth` heuristics as a tie-break, then
/// `route_id` for full determinism.
fn cmp_route_assessments(a: &RouteAssessment, b: &RouteAssessment) -> std::cmp::Ordering {
    a.hard_failures
        .len()
        .cmp(&b.hard_failures.len())
        .then_with(|| a.validation_gaps.len().cmp(&b.validation_gaps.len()))
        .then_with(|| stock_rank(a.stock_termination_status).cmp(&stock_rank(b.stock_termination_status)))
        .then_with(|| {
            accounting_rank(a.target_element_accounting_status)
                .cmp(&accounting_rank(b.target_element_accounting_status))
        })
        .then_with(|| {
            forward_rank(a.forward_validation_status).cmp(&forward_rank(b.forward_validation_status))
        })
        .then_with(|| {
            b.evidence_coverage
                .exact_substrate_evidence_steps
                .cmp(&a.evidence_coverage.exact_substrate_evidence_steps)
        })
        .then_with(|| {
            b.evidence_coverage
                .template_level_evidence_steps
                .cmp(&a.evidence_coverage.template_level_evidence_steps)
        })
        .then_with(|| a.route_cost.total_cmp(&b.route_cost))
        .then_with(|| a.route_depth.cmp(&b.route_depth))
        .then_with(|| a.route_id.cmp(&b.route_id))
}

/// `true` (verified) sorts first.
fn stock_rank(status: StockTerminationStatus) -> u8 {
    match status {
        StockTerminationStatus::AllLeavesVerifiedInConfiguredStock => 0,
        _ => 1,
    }
}

/// `true` (accounted) sorts first.
fn accounting_rank(status: crate::synthesizability::schema::ElementAccountingStatus) -> u8 {
    match status {
        crate::synthesizability::schema::ElementAccountingStatus::Accounted => 0,
        _ => 1,
    }
}

/// `AllEvaluatedStepsValid` > `PartiallyEvaluated` > everything else.
/// Design doc §4.8 doesn't separately rank `ValidatorError` -- it lands in
/// the same worst bucket as `NotEvaluated`/`OneOrMoreStepsInvalid` (both of
/// which, per §4.1/§4.6, are only reachable *here* -- i.e. not already
/// excluded via a hard failure -- when the configured
/// `ForwardValidationPolicy` didn't turn them into one).
fn forward_rank(status: ForwardValidationStatus) -> u8 {
    match status {
        ForwardValidationStatus::AllEvaluatedStepsValid => 0,
        ForwardValidationStatus::PartiallyEvaluated => 1,
        ForwardValidationStatus::NotEvaluated
        | ForwardValidationStatus::OneOrMoreStepsInvalid
        | ForwardValidationStatus::ValidatorError => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chem_env::RetroRule;
    use crate::search::ReactionStep;
    use crate::synthesizability::schema::ElementAccountingStatus;

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

    fn empty_context<'a>(rules: &'a [RetroRule]) -> AssessmentContext<'a> {
        AssessmentContext {
            stock: None,
            template_metadata_hash: None,
            search_config_summary: None,
            git_commit: None,
            embedded_fallback_used: None,
            forward_validation: None,
            rules,
        }
    }

    #[test]
    fn invalid_target_short_circuits_before_anything_else() {
        let rules: Vec<RetroRule> = vec![];
        let ctx = empty_context(&rules);
        let result = assess_routes(
            "this-is-not-a-smiles(((",
            &[],
            &ctx,
            &SynthesizabilityConfig::conservative(),
        )
        .expect("assess_routes should not Err for a bad target");
        assert_eq!(result.status, AssessmentStatus::InvalidTarget);
        assert_eq!(result.canonical_target, None);
        assert!(result.route_assessments.is_empty());
        assert_eq!(result.provenance.canonical_target, "");
    }

    #[test]
    fn zero_routes_is_no_route_found_within_budget() {
        let rules: Vec<RetroRule> = vec![];
        let ctx = empty_context(&rules);
        let result = assess_routes("CCO", &[], &ctx, &SynthesizabilityConfig::conservative())
            .expect("valid target, empty routes must not Err");
        assert_eq!(result.status, AssessmentStatus::NoRouteFoundWithinBudget);
        assert!(result.canonical_target.is_some());
    }

    #[test]
    fn forward_validation_length_mismatch_is_evaluation_error() {
        let rules: Vec<RetroRule> = vec![];
        let routes = vec![route(vec![step("rule:x", "rule:x", "CCO", &["CC=O"])], &["CC=O"])];
        let fv: Vec<Option<Vec<StepValidationStatus>>> = vec![]; // length 0 != routes.len() == 1
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
        .expect("length mismatch must surface as a status, not an Err");
        assert_eq!(result.status, AssessmentStatus::EvaluationError);
    }

    #[test]
    fn clean_route_with_verified_stock_is_route_supported() {
        let rules: Vec<RetroRule> = vec![];
        let routes = vec![route(vec![step("rule:x", "rule:x", "CCO", &["CC=O"])], &["CC=O"])];
        let stock = vec!["CC=O".to_string()];
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
        assert_eq!(result.status, AssessmentStatus::RouteSupported);
        let selected = result.selected_route.expect("a route must be selected");
        assert!(selected.hard_failures.is_empty());
        assert!(selected.validation_gaps.is_empty());
    }

    #[test]
    fn unsupplied_stock_is_a_gap_not_a_hard_failure() {
        let rules: Vec<RetroRule> = vec![];
        let routes = vec![route(vec![step("rule:x", "rule:x", "CCO", &["CC=O"])], &["CC=O"])];
        let ctx = empty_context(&rules); // stock: None
        let result = assess_routes(
            "CCO",
            &routes,
            &ctx,
            &SynthesizabilityConfig::conservative(),
        )
        .unwrap();
        assert_eq!(result.status, AssessmentStatus::RouteSupportedWithValidationGaps);
        let selected = result.selected_route.unwrap();
        assert!(selected.hard_failures.is_empty());
        assert!(
            selected
                .validation_gaps
                .contains(&ValidationGap::StockProvenanceHashMissing)
        );
    }

    #[test]
    fn stock_mismatch_is_a_hard_failure() {
        let rules: Vec<RetroRule> = vec![];
        let routes = vec![route(vec![step("rule:x", "rule:x", "CCO", &["CC=O"])], &["CC=O"])];
        let stock = vec!["completely-different-leaf".to_string()];
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
        assert_eq!(result.status, AssessmentStatus::RoutesFoundButRejected);
        assert!(result.selected_route.is_none());
        assert!(
            result.route_assessments[0]
                .hard_failures
                .iter()
                .any(|hf| matches!(hf, HardFailure::StockTerminalMismatch { leaf } if leaf == "CC=O"))
        );
    }

    #[test]
    fn disabling_stock_requirement_skips_the_check_entirely() {
        let rules: Vec<RetroRule> = vec![];
        let routes = vec![route(vec![step("rule:x", "rule:x", "CCO", &["CC=O"])], &["CC=O"])];
        let ctx = empty_context(&rules); // stock: None
        let mut config = SynthesizabilityConfig::conservative();
        config.require_verified_stock_terminal = false;
        let result = assess_routes("CCO", &routes, &ctx, &config).unwrap();
        let selected = result.selected_route.expect("still supported");
        assert_eq!(
            selected.stock_termination_status,
            StockTerminationStatus::StockCheckNotPerformed
        );
        assert!(selected.hard_failures.is_empty());
        assert!(selected.validation_gaps.is_empty());
        assert_eq!(result.status, AssessmentStatus::RouteSupported);
    }

    #[test]
    fn route_graph_inconsistent_when_a_precursor_is_orphaned() {
        let rules: Vec<RetroRule> = vec![];
        // "orphan-leaf" is neither a declared building block nor any
        // step's target -- the route doesn't actually connect.
        let routes = vec![route(
            vec![step("rule:x", "rule:x", "CCO", &["orphan-leaf"])],
            &["CC=O"], // declared leaf doesn't match the actual precursor
        )];
        let ctx = empty_context(&rules);
        let result = assess_routes(
            "CCO",
            &routes,
            &ctx,
            &SynthesizabilityConfig::conservative(),
        )
        .unwrap();
        assert!(
            result.route_assessments[0]
                .hard_failures
                .contains(&HardFailure::RouteGraphInconsistent)
        );
    }

    #[test]
    fn route_structure_unparseable_when_a_step_smiles_is_garbage() {
        let rules: Vec<RetroRule> = vec![];
        let routes = vec![route(
            vec![step("rule:x", "rule:x", "not-a-smiles(((", &["CC=O"])],
            &["CC=O"],
        )];
        let ctx = empty_context(&rules);
        let result = assess_routes(
            "CCO",
            &routes,
            &ctx,
            &SynthesizabilityConfig::conservative(),
        )
        .unwrap();
        assert!(
            result.route_assessments[0]
                .hard_failures
                .contains(&HardFailure::RouteStructureUnparseable)
        );
    }

    #[test]
    fn zero_step_route_is_legal_not_inconsistent() {
        let rules: Vec<RetroRule> = vec![];
        let routes = vec![route(vec![], &[])];
        let ctx = empty_context(&rules);
        let result = assess_routes(
            "CCO",
            &routes,
            &ctx,
            &SynthesizabilityConfig::conservative(),
        )
        .unwrap();
        assert!(result.route_assessments[0].hard_failures.is_empty());
        assert!(!result.route_assessments[0].warnings.is_empty());
    }

    #[test]
    fn allowlisted_accounting_failure_is_a_gap_under_conservative() {
        let rules: Vec<RetroRule> = vec![];
        let routes = vec![route(
            vec![step(
                "STUB_ACCOUNTING_FAIL",
                "rule:boc_deprotection_retro",
                "CCO",
                &["CC=O"],
            )],
            &["CC=O"],
        )];
        let stock = vec!["CC=O".to_string()];
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
        assert_eq!(result.status, AssessmentStatus::RouteSupportedWithValidationGaps);
        let selected = result.selected_route.unwrap();
        assert!(selected.hard_failures.is_empty());
        assert!(selected.validation_gaps.iter().any(|g| matches!(
            g,
            ValidationGap::ReagentOmissionAccountingGap { template_id, .. }
                if template_id == "rule:boc_deprotection_retro"
        )));
    }

    #[test]
    fn non_allowlisted_accounting_failure_is_hard_failure_under_conservative_and_gap_under_diagnostic()
     {
        let rules: Vec<RetroRule> = vec![];
        let routes = vec![route(
            vec![step(
                "STUB_ACCOUNTING_FAIL",
                "rule:some_other_rule",
                "CCO",
                &["CC=O"],
            )],
            &["CC=O"],
        )];
        let stock = vec!["CC=O".to_string()];
        let ctx = AssessmentContext {
            stock: Some(&stock),
            ..empty_context(&rules)
        };

        let conservative = assess_routes(
            "CCO",
            &routes,
            &ctx,
            &SynthesizabilityConfig::conservative(),
        )
        .unwrap();
        assert_eq!(conservative.status, AssessmentStatus::RoutesFoundButRejected);
        assert!(
            conservative.route_assessments[0]
                .hard_failures
                .iter()
                .any(|hf| matches!(hf, HardFailure::UnaccountedTargetElement { step_index: 0 }))
        );

        let diagnostic = assess_routes(
            "CCO",
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
                .iter()
                .any(|g| matches!(g, ValidationGap::UnaccountedTargetElementNotEnforced { step_index: 0 }))
        );
    }

    #[test]
    fn disabling_accounting_requirement_ignores_even_allowlisted_failures() {
        let rules: Vec<RetroRule> = vec![];
        let routes = vec![route(
            vec![step(
                "STUB_ACCOUNTING_FAIL",
                "rule:some_other_rule",
                "CCO",
                &["CC=O"],
            )],
            &["CC=O"],
        )];
        let stock = vec!["CC=O".to_string()];
        let ctx = AssessmentContext {
            stock: Some(&stock),
            ..empty_context(&rules)
        };
        let mut config = SynthesizabilityConfig::conservative();
        config.require_target_element_accounting = false;
        let result = assess_routes("CCO", &routes, &ctx, &config).unwrap();
        assert_eq!(result.status, AssessmentStatus::RouteSupported);
        let selected = result.selected_route.unwrap();
        assert_eq!(
            selected.target_element_accounting_status,
            ElementAccountingStatus::UnaccountedTargetElement
        );
        assert!(selected.hard_failures.is_empty());
        assert!(selected.validation_gaps.is_empty());
    }

    #[test]
    fn selection_prefers_fewer_hard_failures_over_route_cost() {
        let rules: Vec<RetroRule> = vec![];
        let cheap_but_broken = {
            let mut r = route(
                vec![step("rule:x", "rule:x", "not-a-smiles(((", &["CC=O"])],
                &["CC=O"],
            );
            r.route_cost = 0.1;
            r
        };
        let clean_but_pricier = {
            let mut r = route(vec![step("rule:y", "rule:y", "CCO", &["CC=O"])], &["CC=O"]);
            r.route_cost = 9.0;
            r
        };
        let routes = vec![cheap_but_broken, clean_but_pricier];
        let stock = vec!["CC=O".to_string()];
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
        assert_eq!(result.status, AssessmentStatus::RouteSupported);
        let selected = result.selected_route.unwrap();
        assert_eq!(selected.route_cost, 9.0);
        assert!(selected.hard_failures.is_empty());
    }

    #[test]
    fn max_routes_to_assess_truncates_and_warns() {
        let rules: Vec<RetroRule> = vec![];
        let routes = vec![
            route(vec![step("rule:x", "rule:x", "CCO", &["CC=O"])], &["CC=O"]),
            route(vec![step("rule:x", "rule:x", "CCO", &["CC=O"])], &["CC=O"]),
        ];
        let stock = vec!["CC=O".to_string()];
        let ctx = AssessmentContext {
            stock: Some(&stock),
            ..empty_context(&rules)
        };
        let mut config = SynthesizabilityConfig::conservative();
        config.max_routes_to_assess = 1;
        let result = assess_routes("CCO", &routes, &ctx, &config).unwrap();
        assert_eq!(result.route_assessments.len(), 1);
        assert!(result.warnings.iter().any(|w| w.contains("max_routes_to_assess")));
    }
}
