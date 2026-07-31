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
    SYNTHESIZABILITY_SCHEMA_VERSION, StockTerminationStatus, SynthesizabilityAssessment,
    SynthesizabilityConfig, SynthesizabilityConfigSummary, ValidationGap,
};
use crate::synthesizability::{element_accounting, provenance, signals};
use crate::validation::StepValidationStatus;

/// Everything `assess_routes()` needs beyond the routes themselves and the
/// config -- caller-supplied inputs the kernel does not compute itself
/// (design doc §5, §5.1: `git_commit`/`embedded_fallback_used` are NEVER
/// computed in-crate, wasm32 constraint).
pub struct AssessmentContext<'a> {
    pub stock: Option<&'a [String]>,
    /// Caller-supplied label for where `stock` came from (a file path,
    /// `"embedded"`, etc.) -- echoed verbatim into
    /// `AssessmentProvenance::stock_source`. `None` means the caller did
    /// not say, never a silent default guess.
    pub stock_source: Option<String>,
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
    let stock_input_status = canonicalized_stock.input_status;
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
                stock_input_status,
                context,
                assessment_config_hash,
                config_used,
                warnings,
            ));
        }
    };

    // 1.5. Kernel-integrity invariant: a non-empty stock list where every
    // single entry failed to parse is a data-quality failure on the
    // caller's input, not the same thing as "no stock was supplied" -- it
    // must not be silently absorbed into `StockTerminationStatus::StockNotSupplied`
    // (design doc §4.2/§4.7; see `StockInputStatus::AllEntriesInvalid`).
    if stock_input_status == crate::synthesizability::schema::StockInputStatus::AllEntriesInvalid {
        warnings.push(format!(
            "All {} supplied stock entries failed to parse/canonicalize -- this is a data-quality failure on the stock input, not the same as no stock being supplied at all",
            context.stock.map(<[String]>::len).unwrap_or(0)
        ));
        return Ok(early_assessment(
            target,
            AssessmentStatus::EvaluationError,
            Some(canonical_target),
            rules_count,
            rules_hash,
            stock_count,
            stock_hash,
            stock_input_status,
            context,
            assessment_config_hash,
            config_used,
            warnings,
        ));
    }

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
            stock_input_status,
            context,
            assessment_config_hash,
            config_used,
            warnings,
        ));
    }

    // 2.5. Kernel-integrity invariant, one level deeper than #2: under
    // `ForwardValidationPolicy::RequireAllValid`, a route's own per-step
    // forward-validation slice must match that route's own step count.
    // A mismatch here is `ForwardValidationStatus::ValidatorError` (design
    // doc §4.3) -- the same "caller's input has the wrong shape" class of
    // problem as #2, just discovered per-route instead of across routes.
    // Under `RequireAllValid` specifically (the caller asked this kernel to
    // actually enforce forward validation), this must not silently
    // downgrade to a per-route warning that still lets the route reach
    // `RouteSupported` -- checked before assessing any route so it never
    // has to un-do a route's already-computed status. Under `Ignore`/
    // `RequireNoInvalid`, a `ValidatorError` is intentionally tolerated
    // (design doc §4.3/§4.7 -- neither validator's reliability has been
    // measured yet), so this check is scoped to `RequireAllValid` only.
    if config.forward_validation_policy == ForwardValidationPolicy::RequireAllValid
        && let Some(fv) = context.forward_validation
    {
        let mismatched_route = routes.iter().zip(fv.iter()).position(|(route, per_step)| {
            per_step
                .as_ref()
                .is_some_and(|steps| steps.len() != route.steps.len())
        });
        if let Some(route_index) = mismatched_route {
            warnings.push(format!(
                "route {route_index}'s forward-validation slice does not match its own step count -- RequireAllValid cannot be enforced for this assessment"
            ));
            return Ok(early_assessment(
                target,
                AssessmentStatus::EvaluationError,
                Some(canonical_target),
                rules_count,
                rules_hash,
                stock_count,
                stock_hash,
                stock_input_status,
                context,
                assessment_config_hash,
                config_used,
                warnings,
            ));
        }
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
            stock_input_status,
            context,
            assessment_config_hash,
            config_used,
            warnings,
        ));
    }

    // Every supplied route is assessed -- `context.forward_validation` (if
    // supplied) is indexed against the *original* `routes` order, so
    // truncation must never happen before assessment (it would either
    // desync that indexing or, worse, silently drop the genuinely best
    // route in favor of a worse one that merely appeared earlier in
    // `routes`, defeating the point of §4.8's quality sort below).
    // `max_routes_to_assess` instead bounds the *output* list, applied
    // after sorting, so it only ever trims the long tail.
    let mut route_assessments: Vec<RouteAssessment> = Vec::with_capacity(routes.len());
    for (idx, route) in routes.iter().enumerate() {
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

    // §4.1 #5-#7: status + selection are computed from the FULL sorted
    // list, *before* any output-shaping below. `max_routes_to_assess`/
    // `include_all_route_diagnostics` are display/output-size knobs only
    // -- they must never change the verdict itself (e.g. setting
    // `max_routes_to_assess: 0` must not turn a genuinely clean route into
    // `RoutesFoundButRejected` just because it can't be shown).
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

    // Reproducibility is a property of the full assessed set, independent
    // of how much of it is echoed back in `route_assessments` below.
    let reproducibility_hash = provenance::compute_reproducibility_hash(
        &rules_hash,
        &stock_hash,
        &assessment_config_hash,
        &canonical_target,
        &route_assessments,
    );

    // Now shape the *output* list. `include_all_route_diagnostics == true`
    // returns every assessed route (sorted, truncated to
    // `max_routes_to_assess`); `false` (the `conservative()`/`diagnostic()`
    // default) returns only the single best-ranked route -- note this is
    // `route_assessments.first()`, not `selected_route`: even when every
    // route was rejected (`status == RoutesFoundButRejected`,
    // `selected_route: None`), a caller still needs to see *which* route
    // came closest and *why* it was rejected, or `include_all_route_diagnostics:
    // false` would silently hide all diagnostic detail on every rejected
    // assessment -- exactly the kind of "auditable" failure this kernel
    // exists to prevent. `max_routes_to_assess: 0` still means "show
    // nothing" in either case, for consistency.
    let total_assessed = route_assessments.len();
    let route_assessments: Vec<RouteAssessment> = if config.include_all_route_diagnostics {
        let mut out = route_assessments;
        if total_assessed > config.max_routes_to_assess {
            warnings.push(format!(
                "Computed assessments for all {total_assessed} supplied routes; output truncated to the top {} by §4.8 quality ordering (max_routes_to_assess)",
                config.max_routes_to_assess
            ));
            out.truncate(config.max_routes_to_assess);
        }
        out
    } else {
        match route_assessments.first() {
            Some(r) if config.max_routes_to_assess > 0 => vec![r.clone()],
            _ => Vec::new(),
        }
    };
    if config.max_routes_to_assess == 0 {
        warnings.push(
            "max_routes_to_assess is 0; no route can appear in the output even though routes were supplied and assessed".to_string(),
        );
    }

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
            stock_input_status,
            stock_source: context.stock_source.clone(),
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
    stock_input_status: crate::synthesizability::schema::StockInputStatus,
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
            stock_input_status,
            stock_source: context.stock_source.clone(),
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
        canonical_target,
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
        // inconsistency. `assess_stock_termination` independently
        // re-verifies the canonical target itself against the configured
        // stock for exactly this shape (rather than trusting
        // `Route::building_blocks`, which is empty here and would
        // otherwise make the check vacuous) -- see that function.
        route_warnings.push(
            "Route has zero steps (the target itself was classified as a stock hit by search); the canonical target itself is independently re-verified against the configured stock in its place.".to_string(),
        );
        return;
    }

    // Every precursor referenced by any step must either be a declared
    // leaf (`building_blocks`) or the target of some other step in this
    // same route -- otherwise the route's graph doesn't actually connect,
    // independent of any single step's chemistry.
    //
    // Compared by canonicalized identity (`provenance::canonicalize_or_raw`),
    // never raw string equality: the same molecule can legitimately be
    // spelled differently across steps/leaves in a hand-built or
    // externally-composed `Route` (this function's own unparseable check,
    // two blocks above, already re-canonicalizes for exactly this reason --
    // comparing raw strings here would spuriously flag a chemically
    // connected route as `RouteGraphInconsistent` whenever two equivalent
    // SMILES notations of the same molecule don't happen to match
    // byte-for-byte).
    let leaves: HashSet<String> = route
        .building_blocks
        .iter()
        .map(|s| provenance::canonicalize_or_raw(s))
        .collect();
    let targets: HashSet<String> = route
        .steps
        .iter()
        .map(|s| provenance::canonicalize_or_raw(&s.target))
        .collect();
    let disconnected = route
        .steps
        .iter()
        .flat_map(|s| s.precursors.iter())
        .any(|p| {
            let canon = provenance::canonicalize_or_raw(p);
            !leaves.contains(&canon) && !targets.contains(&canon)
        });
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
    canonical_target: &str,
    config: &SynthesizabilityConfig,
    canonicalized_stock: &signals::CanonicalizedStock,
    hard_failures: &mut Vec<HardFailure>,
    validation_gaps: &mut Vec<ValidationGap>,
    route_warnings: &mut Vec<String>,
) -> StockTerminationStatus {
    if !config.require_verified_stock_terminal {
        return StockTerminationStatus::StockCheckNotPerformed;
    }

    // A zero-step route's implicit claim (`search.rs`'s depth-0 case) is
    // "the target itself is already a stock hit" -- but `route.building_blocks`
    // is empty for this route shape (`extract_building_blocks` has no
    // steps to scan), so checking it directly would be vacuously true
    // without ever verifying anything, undermining this kernel's whole
    // "independently re-verify, never trust the tool's own classification"
    // premise (design doc §4.2). Check the canonical target itself in that
    // case instead of the (empty) leaf list.
    let leaves_to_check: Vec<String> = if route.steps.is_empty() {
        vec![canonical_target.to_string()]
    } else {
        route.building_blocks.clone()
    };

    let result = signals::check_stock_termination(&leaves_to_check, canonicalized_stock);
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
        StockTerminationStatus::StockNotSupplied
        | StockTerminationStatus::StockSuppliedButEmpty => {
            // Nothing usable was supplied to verify against at all --
            // distinct from a genuine mismatch (never "assume it's fine",
            // design doc §4.2/§4.7), recorded as a validation gap rather
            // than a hard failure: the caller didn't claim these leaves
            // are in stock, the kernel just can't confirm or deny it.
            // Both variants land on the same gap here -- the per-route
            // consequence is identical either way; `StockInputStatus` on
            // `AssessmentProvenance` is where the two are told apart.
            validation_gaps.push(ValidationGap::StockNotSupplied);
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
                    validation_gaps
                        .push(ValidationGap::UnaccountedTargetElementNotEnforced { step_index });
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
        ForwardValidationPolicy::Ignore => {}
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
                // `assess_routes()`'s own per-route length check (§4.1 #2.5)
                // already short-circuits the whole assessment to
                // `EvaluationError` before any route reaches this function
                // under `RequireAllValid` -- reaching here means that guard
                // was bypassed (e.g. a future direct call to
                // `assess_one_route`), not a chemistry judgment. Kept as a
                // defensive warning rather than a silent no-op.
                route_warnings.push(
                    "forward-validation input was structurally inconsistent with this route (e.g. wrong step count); RequireAllValid could not be enforced -- this should be unreachable, the assessment-level guard should have caught it first".to_string(),
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
        .then_with(|| {
            stock_rank(a.stock_termination_status).cmp(&stock_rank(b.stock_termination_status))
        })
        .then_with(|| {
            accounting_rank(a.target_element_accounting_status)
                .cmp(&accounting_rank(b.target_element_accounting_status))
        })
        .then_with(|| {
            forward_rank(a.forward_validation_status)
                .cmp(&forward_rank(b.forward_validation_status))
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
            stock_source: None,
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
        let routes = vec![route(
            vec![step("rule:x", "rule:x", "CCO", &["CC=O"])],
            &["CC=O"],
        )];
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
    fn per_route_validator_error_escalates_under_require_all_valid() {
        // The route itself has 1 step, but its own forward-validation slice
        // has 2 entries -- ForwardValidationStatus::ValidatorError. Under
        // RequireAllValid this must not silently downgrade to a warning
        // that still lets the route reach RouteSupported.
        let rules: Vec<RetroRule> = vec![];
        let routes = vec![route(
            vec![step("rule:x", "rule:x", "CCO", &["CC=O"])],
            &["CC=O"],
        )];
        let fv: Vec<Option<Vec<StepValidationStatus>>> = vec![Some(vec![
            StepValidationStatus::Valid,
            StepValidationStatus::Valid,
        ])];
        let ctx = AssessmentContext {
            forward_validation: Some(&fv),
            ..empty_context(&rules)
        };
        let config = SynthesizabilityConfig {
            forward_validation_policy: ForwardValidationPolicy::RequireAllValid,
            ..SynthesizabilityConfig::conservative()
        };
        let result = assess_routes("CCO", &routes, &ctx, &config)
            .expect("per-route length mismatch must surface as a status, not an Err");
        assert_eq!(result.status, AssessmentStatus::EvaluationError);
        assert!(result.route_assessments.is_empty());
    }

    #[test]
    fn per_route_validator_error_tolerated_under_ignore_policy() {
        // Same structurally-inconsistent input as above, but under the
        // default Ignore policy -- ValidatorError is intentionally
        // tolerated (design doc §4.3/§4.7), so this must not escalate.
        let rules: Vec<RetroRule> = vec![];
        let routes = vec![route(
            vec![step("rule:x", "rule:x", "CCO", &["CC=O"])],
            &["CC=O"],
        )];
        let fv: Vec<Option<Vec<StepValidationStatus>>> = vec![Some(vec![
            StepValidationStatus::Valid,
            StepValidationStatus::Valid,
        ])];
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
        .expect("must not Err");
        assert_ne!(result.status, AssessmentStatus::EvaluationError);
        assert_eq!(
            result.route_assessments[0].forward_validation_status,
            ForwardValidationStatus::ValidatorError
        );
    }

    #[test]
    fn stock_entries_all_invalid_escalates_to_evaluation_error() {
        // A non-empty stock list where every entry fails to parse is a
        // data-quality failure on the caller's own input, not "no stock" --
        // must not silently collapse into StockNotSupplied/proceed as if
        // nothing were wrong.
        let rules: Vec<RetroRule> = vec![];
        let routes = vec![route(
            vec![step("rule:x", "rule:x", "CCO", &["CC=O"])],
            &["CC=O"],
        )];
        let stock = vec!["[C(".to_string(), "not-a-smiles(((".to_string()];
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
        .expect("all-invalid stock must surface as a status, not an Err");
        assert_eq!(result.status, AssessmentStatus::EvaluationError);
        assert!(result.route_assessments.is_empty());
        assert_eq!(
            result.provenance.stock_input_status,
            crate::synthesizability::schema::StockInputStatus::AllEntriesInvalid
        );
    }

    #[test]
    fn invalid_target_wins_over_all_invalid_stock() {
        // §4.1 decision order: InvalidTarget strictly beats EvaluationError
        // even when both conditions hold at once.
        let rules: Vec<RetroRule> = vec![];
        let stock = vec!["[C(".to_string()];
        let ctx = AssessmentContext {
            stock: Some(&stock),
            ..empty_context(&rules)
        };
        let result = assess_routes(
            "not-a-smiles(((",
            &[],
            &ctx,
            &SynthesizabilityConfig::conservative(),
        )
        .expect("must not Err");
        assert_eq!(result.status, AssessmentStatus::InvalidTarget);
    }

    #[test]
    fn clean_route_with_verified_stock_is_route_supported() {
        let rules: Vec<RetroRule> = vec![];
        let routes = vec![route(
            vec![step("rule:x", "rule:x", "CCO", &["CC=O"])],
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
        assert_eq!(result.status, AssessmentStatus::RouteSupported);
        let selected = result.selected_route.expect("a route must be selected");
        assert!(selected.hard_failures.is_empty());
        assert!(selected.validation_gaps.is_empty());
    }

    #[test]
    fn unsupplied_stock_is_a_gap_not_a_hard_failure() {
        let rules: Vec<RetroRule> = vec![];
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
            result.status,
            AssessmentStatus::RouteSupportedWithValidationGaps
        );
        let selected = result.selected_route.unwrap();
        assert!(selected.hard_failures.is_empty());
        assert!(
            selected
                .validation_gaps
                .contains(&ValidationGap::StockNotSupplied)
        );
    }

    #[test]
    fn stock_mismatch_is_a_hard_failure() {
        let rules: Vec<RetroRule> = vec![];
        let routes = vec![route(
            vec![step("rule:x", "rule:x", "CCO", &["CC=O"])],
            &["CC=O"],
        )];
        // A valid, parseable SMILES that is simply a different molecule
        // from "CC=O" -- not an invalid/non-SMILES placeholder, which
        // canonicalize_stock would instead reject as unparseable
        // (count == 0 -> StockNotSupplied, not a real mismatch).
        let stock = vec!["c1ccccc1".to_string()];
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
            result.route_assessments[0].hard_failures.iter().any(
                |hf| matches!(hf, HardFailure::StockTerminalMismatch { leaf } if leaf == "CC=O")
            )
        );
    }

    #[test]
    fn disabling_stock_requirement_skips_the_check_entirely() {
        let rules: Vec<RetroRule> = vec![];
        let routes = vec![route(
            vec![step("rule:x", "rule:x", "CCO", &["CC=O"])],
            &["CC=O"],
        )];
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
    fn route_graph_connectivity_uses_canonical_identity_not_raw_string_equality() {
        // Independent-reviewer-found regression: the same molecule
        // (phenylacetaldehyde) spelled two different ways across steps --
        // step 0's precursor and step 1's target are the identical
        // molecule but not the identical string. A raw-string connectivity
        // check would spuriously flag this as RouteGraphInconsistent even
        // though the route genuinely connects. Not reachable through
        // today's `find_routes` (which always threads the exact same
        // string forward), but a real correctness requirement for
        // `assess_routes`'s own documented contract of taking any
        // already-produced `Route`, hand-built or externally composed.
        let rules: Vec<RetroRule> = vec![];
        let routes = vec![route(
            vec![
                step(
                    "rule:x",
                    "rule:x",
                    "CC(=O)c1ccc(cc1)CC=O", // some earlier target
                    &["c1ccc(cc1)CC=O"],    // phenylacetaldehyde, notation A
                ),
                step(
                    "rule:y",
                    "rule:y",
                    "O=CCc1ccccc1", // phenylacetaldehyde, notation B -- same molecule
                    &["CC=O"],
                ),
            ],
            &["CC=O"],
        )];
        let ctx = empty_context(&rules);
        let result = assess_routes(
            "CC(=O)c1ccc(cc1)CC=O",
            &routes,
            &ctx,
            &SynthesizabilityConfig::conservative(),
        )
        .unwrap();
        assert!(
            !result.route_assessments[0]
                .hard_failures
                .contains(&HardFailure::RouteGraphInconsistent),
            "hard_failures should not contain RouteGraphInconsistent: {:?}",
            result.route_assessments[0].hard_failures
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
    fn zero_step_route_verifies_the_target_itself_against_stock_when_present() {
        // building_blocks is empty for a zero-step route, so checking it
        // directly would vacuously "succeed" without ever consulting the
        // configured stock. The canonical target must be checked in its
        // place -- regression for the depth=0 stock re-verification fix.
        let rules: Vec<RetroRule> = vec![];
        let routes = vec![route(vec![], &[])];
        let stock = vec!["CCO".to_string()];
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
            StockTerminationStatus::AllLeavesVerifiedInConfiguredStock
        );
        assert_eq!(result.status, AssessmentStatus::RouteSupported);
    }

    #[test]
    fn zero_step_route_target_not_in_stock_is_a_hard_failure_not_vacuous_success() {
        let rules: Vec<RetroRule> = vec![];
        let routes = vec![route(vec![], &[])];
        // A valid, parseable, but different molecule from the target --
        // before this fix, the empty `building_blocks` list would make the
        // stock check vacuously pass regardless of what's configured here.
        let stock = vec!["c1ccccc1".to_string()];
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
        assert!(
            result.route_assessments[0]
                .hard_failures
                .iter()
                .any(|hf| matches!(hf, HardFailure::StockTerminalMismatch { .. }))
        );
        assert_eq!(result.status, AssessmentStatus::RoutesFoundButRejected);
    }

    #[test]
    fn allowlisted_accounting_failure_is_a_gap_under_conservative() {
        let rules: Vec<RetroRule> = vec![];
        let routes = vec![route(
            vec![step(
                "boc_deprotection_retro",
                "rule:boc_deprotection_retro",
                // Genuine per-element violation, not a stubbed trigger: the
                // target's Br has no precursor source at all.
                "Brc1ccccc1",
                &["c1ccccc1"],
            )],
            &["c1ccccc1"],
        )];
        let stock = vec!["c1ccccc1".to_string()];
        let ctx = AssessmentContext {
            stock: Some(&stock),
            ..empty_context(&rules)
        };
        let result = assess_routes(
            "Brc1ccccc1",
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
                if template_id == "rule:boc_deprotection_retro"
        )));
    }

    #[test]
    fn non_allowlisted_accounting_failure_is_hard_failure_under_conservative_and_gap_under_diagnostic()
     {
        let rules: Vec<RetroRule> = vec![];
        let routes = vec![route(
            vec![step(
                "some_other_rule",
                "rule:some_other_rule",
                // Not on the default reagent-omission allowlist, and a
                // genuine per-element violation (target's Br has no
                // precursor source), not a stubbed trigger.
                "Brc1ccccc1",
                &["c1ccccc1"],
            )],
            &["c1ccccc1"],
        )];
        let stock = vec!["c1ccccc1".to_string()];
        let ctx = AssessmentContext {
            stock: Some(&stock),
            ..empty_context(&rules)
        };

        let conservative = assess_routes(
            "Brc1ccccc1",
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
                .iter()
                .any(|hf| matches!(hf, HardFailure::UnaccountedTargetElement { step_index: 0 }))
        );

        let diagnostic = assess_routes(
            "Brc1ccccc1",
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
                .any(|g| matches!(
                    g,
                    ValidationGap::UnaccountedTargetElementNotEnforced { step_index: 0 }
                ))
        );
    }

    #[test]
    fn disabling_accounting_requirement_ignores_even_allowlisted_failures() {
        let rules: Vec<RetroRule> = vec![];
        let routes = vec![route(
            vec![step(
                "some_other_rule",
                "rule:some_other_rule",
                // Not on the default reagent-omission allowlist, and a
                // genuine per-element violation (target's Br has no
                // precursor source), not a stubbed trigger.
                "Brc1ccccc1",
                &["c1ccccc1"],
            )],
            &["c1ccccc1"],
        )];
        let stock = vec!["c1ccccc1".to_string()];
        let ctx = AssessmentContext {
            stock: Some(&stock),
            ..empty_context(&rules)
        };
        let mut config = SynthesizabilityConfig::conservative();
        config.require_target_element_accounting = false;
        let result = assess_routes("Brc1ccccc1", &routes, &ctx, &config).unwrap();
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
        // Exercise max_routes_to_assess's own truncation, not
        // include_all_route_diagnostics' default single-route cap (which
        // would make `route_assessments.len() == 1` trivially true
        // regardless of max_routes_to_assess).
        config.include_all_route_diagnostics = true;
        config.max_routes_to_assess = 1;
        let result = assess_routes("CCO", &routes, &ctx, &config).unwrap();
        assert_eq!(result.route_assessments.len(), 1);
        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.contains("max_routes_to_assess"))
        );
    }

    #[test]
    fn max_routes_to_assess_truncation_never_drops_the_best_route() {
        // The genuinely clean route is supplied SECOND (i.e. later in
        // caller order than a route with a stock mismatch). Truncation must
        // happen after §4.8 sorting, not on the caller-supplied order --
        // otherwise the worse, first-supplied route would survive and the
        // better, second-supplied one would be silently dropped.
        let rules: Vec<RetroRule> = vec![];
        let routes = vec![
            // Worse: "c1ccccc1" is not in the configured stock.
            route(
                vec![step("rule:x", "rule:x", "CCO", &["c1ccccc1"])],
                &["c1ccccc1"],
            ),
            // Better: leaf is in the configured stock.
            route(vec![step("rule:x", "rule:x", "CCO", &["CC=O"])], &["CC=O"]),
        ];
        let stock = vec!["CC=O".to_string()];
        let ctx = AssessmentContext {
            stock: Some(&stock),
            ..empty_context(&rules)
        };
        let mut config = SynthesizabilityConfig::conservative();
        config.include_all_route_diagnostics = true;
        config.max_routes_to_assess = 1;
        let result = assess_routes("CCO", &routes, &ctx, &config).unwrap();
        assert_eq!(result.route_assessments.len(), 1);
        let survivor = &result.route_assessments[0];
        assert!(survivor.hard_failures.is_empty());
        assert_eq!(
            survivor.stock_termination_status,
            StockTerminationStatus::AllLeavesVerifiedInConfiguredStock
        );
        assert_eq!(result.status, AssessmentStatus::RouteSupported);
    }

    #[test]
    fn max_routes_to_assess_zero_never_changes_the_verdict() {
        // A pure output-size knob must never flip a genuinely clean,
        // fully-supported route's verdict just because it can't be shown.
        let rules: Vec<RetroRule> = vec![];
        let routes = vec![route(
            vec![step("rule:x", "rule:x", "CCO", &["CC=O"])],
            &["CC=O"],
        )];
        let stock = vec!["CC=O".to_string()];
        let ctx = AssessmentContext {
            stock: Some(&stock),
            ..empty_context(&rules)
        };
        let mut config = SynthesizabilityConfig::conservative();
        config.max_routes_to_assess = 0;
        let result = assess_routes("CCO", &routes, &ctx, &config).unwrap();
        assert_eq!(
            result.status,
            AssessmentStatus::RouteSupported,
            "max_routes_to_assess=0 must not turn a clean route into RoutesFoundButRejected"
        );
        assert!(result.selected_route.is_some());
        assert!(result.route_assessments.is_empty());
        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.contains("max_routes_to_assess is 0"))
        );
    }

    #[test]
    fn include_all_route_diagnostics_toggles_output_breadth_not_the_verdict() {
        let rules: Vec<RetroRule> = vec![];
        let routes = vec![
            route(
                vec![step("rule:x", "rule:x", "CCO", &["c1ccccc1"])],
                &["c1ccccc1"],
            ),
            route(vec![step("rule:x", "rule:x", "CCO", &["CC=O"])], &["CC=O"]),
        ];
        let stock = vec!["CC=O".to_string()];
        let ctx = AssessmentContext {
            stock: Some(&stock),
            ..empty_context(&rules)
        };

        let mut narrow = SynthesizabilityConfig::conservative();
        narrow.include_all_route_diagnostics = false;
        let narrow_result = assess_routes("CCO", &routes, &ctx, &narrow).unwrap();
        assert_eq!(narrow_result.route_assessments.len(), 1);
        assert_eq!(narrow_result.status, AssessmentStatus::RouteSupported);

        let mut wide = SynthesizabilityConfig::conservative();
        wide.include_all_route_diagnostics = true;
        let wide_result = assess_routes("CCO", &routes, &ctx, &wide).unwrap();
        assert_eq!(wide_result.route_assessments.len(), 2);
        // The verdict itself must be identical regardless of how much
        // diagnostic detail was requested.
        assert_eq!(wide_result.status, narrow_result.status);
        assert_eq!(
            wide_result.selected_route.as_ref().map(|r| &r.route_id),
            narrow_result.selected_route.as_ref().map(|r| &r.route_id)
        );
    }

    #[test]
    fn stock_source_label_is_echoed_into_provenance() {
        let rules: Vec<RetroRule> = vec![];
        let routes = vec![route(
            vec![step("rule:x", "rule:x", "CCO", &["CC=O"])],
            &["CC=O"],
        )];
        let stock = vec!["CC=O".to_string()];
        let ctx = AssessmentContext {
            stock: Some(&stock),
            stock_source: Some("data/building_blocks.smi".to_string()),
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
            result.provenance.stock_source,
            Some("data/building_blocks.smi".to_string())
        );
    }
}
