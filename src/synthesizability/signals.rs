//! Pure signal-extraction functions for the Synthesizability Kernel
//! (`docs/design/synthesizability-kernel-v0.md`, Agent B's file).
//!
//! Every function here is a pure function over already-produced data: a
//! caller-supplied stock list, a `search::Route`, or an optional per-step
//! validation-status slice. Nothing in this file runs search, invokes a
//! validator, or touches the filesystem/network -- see the design doc's
//! scope boundary (§0, §4.2-4.4).

use std::collections::HashSet;

use sha2::{Digest, Sha256};

use crate::chem_env::canonical_stock_identity_from_smiles;
use crate::evidence::{ExampleMatch, MetadataSource};
use crate::synthesizability::schema::{
    EvidenceCoverage, ForwardValidationStatus, StockInputStatus, StockTerminationStatus,
};
use crate::validation::StepValidationStatus;

// ---------------------------------------------------------------------
// 4.2 Stock termination -- an independent membership check that reuses
// `chem_env::canonical_stock_identity_from_smiles` (the shared identity
// primitive, post-#71/PR #74) but never calls `ChemEnv::is_building_block`
// itself, so a bug or policy change in that consumer's membership logic
// can't silently change this kernel's own verdict (see design doc §4.2).
// ---------------------------------------------------------------------

/// Canonicalizes one SMILES string under the shared stock-identity policy
/// (design doc §4.2): `chem_env::canonical_stock_identity_from_smiles`, the
/// same function `ChemEnv::is_building_block`/`search::is_bb` use, so this
/// kernel's independent stock check can never drift from `ChemEnv`'s own
/// policy while still never calling `ChemEnv::is_building_block` itself.
/// `None` if the SMILES fails to parse.
fn kernel_canonicalize(smiles: &str) -> Option<String> {
    canonical_stock_identity_from_smiles(smiles).ok()
}

/// Domain separator for [`stock_content_hash`]. Deliberately distinct from
/// `ChemEnv::content_sha256`'s `b"renkin-retrospect-stock-v1\0"`: this hash
/// is over a different, independently-canonicalized set (the caller-
/// supplied stock list for one `assess_routes()` call, canonicalized under
/// this module's own policy) and must never be mistaken for -- or collide
/// with -- that one.
const STOCK_HASH_DOMAIN: &[u8] = b"renkin-synthesizability-stock-v1\0";

/// sha256 over the sorted, deduped, kernel-canonicalized stock set --
/// order-independent (same style as `ChemEnv::content_sha256`), so
/// re-supplying the same stock in a different order produces an identical
/// hash (design doc §5's `AssessmentProvenance::stock_hash`).
fn stock_content_hash(canonical_set: &HashSet<String>) -> String {
    let mut sorted: Vec<&str> = canonical_set.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    let mut hasher = Sha256::new();
    hasher.update(STOCK_HASH_DOMAIN);
    hasher.update((sorted.len() as u64).to_be_bytes());
    for smi in sorted {
        hasher.update((smi.len() as u64).to_be_bytes());
        hasher.update(smi.as_bytes());
    }
    format!("sha256:{}", crate::sha256_hex(hasher.finalize()))
}

/// Result of canonicalizing a supplied stock list under the shared
/// identity policy (design doc §4.2 -- reuses
/// `chem_env::canonical_stock_identity_from_smiles`, NEVER
/// `ChemEnv::is_building_block`). Computed once per `assess_routes()` call
/// (not once per route) so `stock_count`/`stock_hash` in provenance and
/// every route's stock check share one canonicalization pass.
///
/// `count == 0` alone cannot distinguish "no stock was supplied" from "a
/// non-empty stock was supplied but every entry failed to parse" -- both
/// produce an empty `canonical_set`. [`Self::input_status`] is the field
/// that disambiguates them; callers that care about this distinction
/// (`assessment.rs`) must read it rather than inferring from `count`.
pub(crate) struct CanonicalizedStock {
    /// Number of distinct, successfully-canonicalized stock entries (i.e.
    /// `canonical_set.len()`) -- the same "count what's actually usable"
    /// convention as `ChemEnv::bb_count`. Entries that failed to parse are
    /// not counted here (see `unparseable_raw_entries`), and duplicates
    /// collapse to one (`canonical_set` is a `HashSet`).
    pub count: usize,
    /// sha256:<hex> over sorted canonicalized entries. Empty string if
    /// `stock` is `None`.
    pub hash: String,
    pub canonical_set: HashSet<String>,
    /// Raw stock-list entries (as supplied) that failed to parse/
    /// standardize -- a data-quality warning about the *stock list*
    /// itself, distinct from a route leaf failing the same way.
    pub unparseable_raw_entries: Vec<String>,
    /// Which of [`StockInputStatus`]'s five cases the raw `stock` argument
    /// fell into. See [`Self`]'s own doc comment for why this can't be
    /// re-derived from `count` alone.
    pub input_status: StockInputStatus,
}

/// `stock: None` => `count: 0, hash: String::new(), canonical_set: empty,
/// unparseable_raw_entries: empty, input_status: NotSupplied`.
pub(crate) fn canonicalize_stock(stock: Option<&[String]>) -> CanonicalizedStock {
    let Some(entries) = stock else {
        return CanonicalizedStock {
            count: 0,
            hash: String::new(),
            canonical_set: HashSet::new(),
            unparseable_raw_entries: Vec::new(),
            input_status: StockInputStatus::NotSupplied,
        };
    };

    if entries.is_empty() {
        return CanonicalizedStock {
            count: 0,
            hash: stock_content_hash(&HashSet::new()),
            canonical_set: HashSet::new(),
            unparseable_raw_entries: Vec::new(),
            input_status: StockInputStatus::SuppliedButEmpty,
        };
    }

    let mut canonical_set: HashSet<String> = HashSet::new();
    let mut unparseable_raw_entries = Vec::new();
    for raw in entries {
        match kernel_canonicalize(raw) {
            Some(canon) => {
                canonical_set.insert(canon);
            }
            None => unparseable_raw_entries.push(raw.clone()),
        }
    }

    let input_status = if canonical_set.is_empty() {
        StockInputStatus::AllEntriesInvalid
    } else if unparseable_raw_entries.is_empty() {
        StockInputStatus::FullyUsable
    } else {
        StockInputStatus::PartiallyUsable
    };

    let hash = stock_content_hash(&canonical_set);
    CanonicalizedStock {
        count: canonical_set.len(),
        hash,
        canonical_set,
        unparseable_raw_entries,
        input_status,
    }
}

/// One route's independent stock-termination check result (design doc
/// §4.2).
pub(crate) struct StockCheckResult {
    pub status: StockTerminationStatus,
    /// Canonical form of every leaf not found in `canonicalized_stock`.
    /// Empty unless `status == OneOrMoreLeavesNotInStock`.
    pub unmatched_leaves: Vec<String>,
    /// Raw leaf SMILES (from `route.building_blocks`) that failed to
    /// parse/standardize under the kernel's own pipeline. Empty unless
    /// `status == StockIdentityUnavailable`.
    pub unparseable_leaves: Vec<String>,
}

/// Attempts the check unconditionally given `canonicalized_stock`.
/// Precedence when multiple conditions could apply to different leaves in
/// the same route: (1) if `canonicalized_stock.count == 0`, branch on
/// `canonicalized_stock.input_status` -> `StockNotSupplied` (caller passed
/// `None`) or `StockSuppliedButEmpty` (caller passed `Some(&[])`)
/// regardless of route leaves -- `StockInputStatus::AllEntriesInvalid`
/// never reaches this function in practice (`assess_routes()` short-circuits
/// to `AssessmentStatus::EvaluationError` before assessing any route in that
/// case), but if it somehow did, this falls back to `StockNotSupplied` as
/// the safer of the two; (2) else if ANY leaf's SMILES fails to parse/standardize ->
/// `StockIdentityUnavailable` (report all such leaves in
/// `unparseable_leaves`); (3) else if ANY leaf's canonical form is not in
/// `canonicalized_stock.canonical_set` -> `OneOrMoreLeavesNotInStock`
/// (report all such leaves in `unmatched_leaves`); (4) else ->
/// `AllLeavesVerifiedInConfiguredStock`. There is no code path in this
/// function that returns `StockCheckNotPerformed` or `StockCheckError` --
/// `StockCheckNotPerformed` is the caller's (assessment.rs's) job when
/// `SynthesizabilityConfig::require_verified_stock_terminal` is false (it
/// must not call this function at all in that case); `StockCheckError` is
/// never constructed here -- every failure mode this function can observe
/// already has a more specific status.
///
/// **Edge case, flagged for the caller**: an empty `route_leaves` (e.g. a
/// depth-0 route, where the target itself is already a stock leaf --
/// `search::find_routes`'s `extract_building_blocks` derives
/// `Route::building_blocks` purely from step precursors, so a zero-step
/// route's `building_blocks` is itself empty, NOT `[target]`) makes every
/// "any leaf ..." condition below vacuously false, so this function
/// returns `AllLeavesVerifiedInConfiguredStock` for a route where *zero*
/// leaves were actually checked against the configured stock. This matches
/// this codebase's existing vacuous-truth convention for empty inputs
/// (compare `validation::aggregate_route(&[])` -> `Validated`), but it
/// means a depth-0 route's target is never independently re-verified
/// against `canonicalized_stock` through this path -- only RENKIN's own
/// (possibly-buggy, per design doc §4.2) building-block check inside
/// `find_routes` decided that route was "solved" in the first place.
/// `unmatched_leaves` is also not deduplicated -- if `route_leaves`
/// repeats an entry, so does this list; dedup on the caller side if that
/// matters for a downstream count (e.g. `hard_failures.len()`, the primary
/// key in design doc §4.8's route-selection ordering).
pub(crate) fn check_stock_termination(
    route_leaves: &[String],
    canonicalized_stock: &CanonicalizedStock,
) -> StockCheckResult {
    if canonicalized_stock.count == 0 {
        let status = if canonicalized_stock.input_status == StockInputStatus::SuppliedButEmpty {
            StockTerminationStatus::StockSuppliedButEmpty
        } else {
            StockTerminationStatus::StockNotSupplied
        };
        return StockCheckResult {
            status,
            unmatched_leaves: Vec::new(),
            unparseable_leaves: Vec::new(),
        };
    }

    let mut unparseable_leaves = Vec::new();
    let mut canon_leaves = Vec::with_capacity(route_leaves.len());
    for leaf in route_leaves {
        match kernel_canonicalize(leaf) {
            Some(canon) => canon_leaves.push(canon),
            None => unparseable_leaves.push(leaf.clone()),
        }
    }
    if !unparseable_leaves.is_empty() {
        return StockCheckResult {
            status: StockTerminationStatus::StockIdentityUnavailable,
            unmatched_leaves: Vec::new(),
            unparseable_leaves,
        };
    }

    let unmatched_leaves: Vec<String> = canon_leaves
        .into_iter()
        .filter(|c| !canonicalized_stock.canonical_set.contains(c))
        .collect();
    if !unmatched_leaves.is_empty() {
        return StockCheckResult {
            status: StockTerminationStatus::OneOrMoreLeavesNotInStock,
            unmatched_leaves,
            unparseable_leaves: Vec::new(),
        };
    }

    StockCheckResult {
        status: StockTerminationStatus::AllLeavesVerifiedInConfiguredStock,
        unmatched_leaves: Vec::new(),
        unparseable_leaves: Vec::new(),
    }
}

// ---------------------------------------------------------------------
// 4.3 Forward-validation rollup -- taken as input, never (re)computed here.
// ---------------------------------------------------------------------

/// Route-level rollup of per-step forward-validation results (design doc
/// §4.3). `None` supplied -> `NotEvaluated`. All
/// `validation::StepValidationStatus::Valid` -> `AllEvaluatedStepsValid`.
/// Any `Invalid` -> `OneOrMoreStepsInvalid`. Otherwise (some `Valid`, some
/// `NotEvaluable`, no `Invalid`) -> `PartiallyEvaluated`. If
/// `per_step.len() != step_count` when `per_step` is `Some` ->
/// `ValidatorError` (structurally inconsistent input).
pub(crate) fn rollup_forward_validation(
    per_step: Option<&[StepValidationStatus]>,
    step_count: usize,
) -> ForwardValidationStatus {
    let Some(statuses) = per_step else {
        return ForwardValidationStatus::NotEvaluated;
    };
    if statuses.len() != step_count {
        return ForwardValidationStatus::ValidatorError;
    }
    // Invalid dominates regardless of position (mirrors
    // `validation::aggregate_route`'s "don't trust a route with even one
    // confirmed-wrong step" rule).
    if statuses.contains(&StepValidationStatus::Invalid) {
        return ForwardValidationStatus::OneOrMoreStepsInvalid;
    }
    if statuses.iter().all(|s| *s == StepValidationStatus::Valid) {
        return ForwardValidationStatus::AllEvaluatedStepsValid;
    }
    ForwardValidationStatus::PartiallyEvaluated
}

// ---------------------------------------------------------------------
// 4.4 Evidence coverage -- direct reuse of `src/evidence.rs`, no new logic.
// ---------------------------------------------------------------------

/// Route-level rollup of per-step evidence coverage (design doc §4.4).
/// Reads each step's `ReactionStep.evidence: Option<StepEvidence>` directly
/// -- no new evidence logic, a pure tally.
///
/// Interpretation note (the design doc and schema doc comments specify the
/// `examples[].match_kind`-based exact/template-only split precisely, but
/// don't explicitly say whether "at least one condition/warning/reported-
/// yield attached" should look only at `StepEvidence`'s own template-level
/// `condition_candidates`/`warnings`/`reported_yields`, or also at the
/// per-example nested `ReactionExample::conditions`/`warnings`/
/// `reported_yield` fields): this tally counts a step as having
/// conditions/warnings/a reported yield if *either* the template-level
/// list is non-empty *or* at least one attached example carries that data,
/// since `EvidenceCoverage`'s field docs say "at least one ... attached"
/// without qualifying which location -- excluding example-level data would
/// under-count real evidence schema_version 2 sidecars actually carry
/// (reported yields in particular are schema_version-2-only under
/// `examples[].reported_yield`, so counting only the template-level list
/// would silently zero out `steps_with_reported_yield` for exactly the
/// sidecars that use it).
///
/// **Must not count a `ReportedYield` whose `source ==
/// MetadataSource::ModelPrediction`** toward `steps_with_reported_yield`,
/// per design doc §4.4's explicit guard, checked at both locations above.
pub(crate) fn compute_evidence_coverage(route: &crate::search::Route) -> EvidenceCoverage {
    let mut coverage = EvidenceCoverage::default();

    for step in &route.steps {
        let Some(evidence) = &step.evidence else {
            coverage.steps_without_evidence += 1;
            continue;
        };

        let has_exact = evidence
            .examples
            .iter()
            .any(|e| e.match_kind == ExampleMatch::ExactSubstrate);
        let has_template_only = evidence
            .examples
            .iter()
            .any(|e| e.match_kind == ExampleMatch::TemplateOnly);
        if has_exact {
            coverage.exact_substrate_evidence_steps += 1;
        } else if has_template_only {
            coverage.template_level_evidence_steps += 1;
        }

        let has_reported_yield = evidence
            .reported_yields
            .iter()
            .any(|y| y.source != MetadataSource::ModelPrediction)
            || evidence.examples.iter().any(|e| {
                e.example
                    .reported_yield
                    .as_ref()
                    .is_some_and(|y| y.source != MetadataSource::ModelPrediction)
            });
        if has_reported_yield {
            coverage.steps_with_reported_yield += 1;
        }

        let has_conditions = !evidence.condition_candidates.is_empty()
            || evidence
                .examples
                .iter()
                .any(|e| e.example.conditions.is_some());
        if has_conditions {
            coverage.steps_with_conditions += 1;
        }

        let has_warnings = !evidence.warnings.is_empty()
            || evidence
                .examples
                .iter()
                .any(|e| !e.example.warnings.is_empty());
        if has_warnings {
            coverage.steps_with_warnings += 1;
        }
    }

    coverage
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::{
        ConditionCandidate, EvidenceScope, ReactionExample, ReportedYield, ResolvedReactionExample,
        StepEvidence, YieldBasis, YieldPercentage,
    };
    use crate::search::{ReactionStep, Route};

    fn leaf(rule: &str, target: &str, precursors: &[&str]) -> ReactionStep {
        ReactionStep {
            rule: rule.to_string(),
            template_id: format!("rule:{rule}"),
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
            steps,
            depth: 1,
            score: 0.0,
            building_blocks: building_blocks.iter().map(|s| s.to_string()).collect(),
            confidence: 1.0,
            convergency: 1.0,
            success_probability: 1.0,
            route_cost: 1.0,
        }
    }

    // ── canonicalize_stock / check_stock_termination: all 4 reachable states ──

    #[test]
    fn stock_none_gives_empty_canonicalization() {
        let stock = canonicalize_stock(None);
        assert_eq!(stock.count, 0);
        assert_eq!(stock.hash, "");
        assert!(stock.canonical_set.is_empty());
        assert!(stock.unparseable_raw_entries.is_empty());
        assert_eq!(stock.input_status, StockInputStatus::NotSupplied);
    }

    #[test]
    fn stock_termination_not_supplied_when_stock_is_none() {
        let stock = canonicalize_stock(None);
        let leaves = vec!["CCO".to_string()];
        let result = check_stock_termination(&leaves, &stock);
        assert_eq!(result.status, StockTerminationStatus::StockNotSupplied);
        assert!(result.unmatched_leaves.is_empty());
        assert!(result.unparseable_leaves.is_empty());
    }

    #[test]
    fn stock_supplied_but_empty_is_distinct_from_not_supplied() {
        let entries: Vec<String> = vec![];
        let stock = canonicalize_stock(Some(&entries));
        assert_eq!(stock.count, 0);
        assert_eq!(stock.input_status, StockInputStatus::SuppliedButEmpty);

        let leaves = vec!["CCO".to_string()];
        let result = check_stock_termination(&leaves, &stock);
        assert_eq!(result.status, StockTerminationStatus::StockSuppliedButEmpty);
        assert_ne!(result.status, StockTerminationStatus::StockNotSupplied);
    }

    #[test]
    fn stock_entries_all_invalid_is_distinct_from_not_supplied_or_empty() {
        let entries = vec!["[C(".to_string(), "not-a-smiles(((".to_string()];
        let stock = canonicalize_stock(Some(&entries));
        assert_eq!(stock.count, 0);
        assert_eq!(stock.unparseable_raw_entries.len(), 2);
        assert_eq!(stock.input_status, StockInputStatus::AllEntriesInvalid);
    }

    #[test]
    fn stock_partially_usable_when_some_entries_invalid() {
        let entries = vec!["CCO".to_string(), "[C(".to_string()];
        let stock = canonicalize_stock(Some(&entries));
        assert_eq!(stock.count, 1);
        assert_eq!(stock.input_status, StockInputStatus::PartiallyUsable);
    }

    #[test]
    fn stock_fully_usable_when_every_entry_parses() {
        let entries = vec!["CCO".to_string(), "c1ccccc1".to_string()];
        let stock = canonicalize_stock(Some(&entries));
        assert_eq!(stock.input_status, StockInputStatus::FullyUsable);
    }

    #[test]
    fn stock_termination_all_verified() {
        let entries = vec!["CCO".to_string(), "c1ccccc1".to_string()];
        let stock = canonicalize_stock(Some(&entries));
        assert_eq!(stock.count, 2);
        assert!(!stock.hash.is_empty());

        let leaves = vec!["CCO".to_string()];
        let result = check_stock_termination(&leaves, &stock);
        assert_eq!(
            result.status,
            StockTerminationStatus::AllLeavesVerifiedInConfiguredStock
        );
        assert!(result.unmatched_leaves.is_empty());
        assert!(result.unparseable_leaves.is_empty());
    }

    #[test]
    fn stock_termination_one_or_more_leaves_not_in_stock() {
        let entries = vec!["CCO".to_string()];
        let stock = canonicalize_stock(Some(&entries));

        let leaves = vec!["CCO".to_string(), "c1ccccc1".to_string()];
        let result = check_stock_termination(&leaves, &stock);
        assert_eq!(
            result.status,
            StockTerminationStatus::OneOrMoreLeavesNotInStock
        );
        assert_eq!(result.unmatched_leaves.len(), 1);
        assert!(result.unparseable_leaves.is_empty());
    }

    #[test]
    fn stock_termination_identity_unavailable_takes_precedence_over_unmatched() {
        let entries = vec!["CCO".to_string()];
        let stock = canonicalize_stock(Some(&entries));

        // "c1ccccc1" is a valid, unmatched leaf; "[C(" fails to
        // parse. Precedence rule (2) beats rule (3): the whole result must
        // be StockIdentityUnavailable, not OneOrMoreLeavesNotInStock.
        let leaves = vec!["c1ccccc1".to_string(), "[C(".to_string()];
        let result = check_stock_termination(&leaves, &stock);
        assert_eq!(
            result.status,
            StockTerminationStatus::StockIdentityUnavailable
        );
        assert_eq!(result.unparseable_leaves, vec!["[C(".to_string()]);
        assert!(result.unmatched_leaves.is_empty());
    }

    #[test]
    fn stock_canonicalization_reports_unparseable_raw_stock_entries() {
        let entries = vec!["CCO".to_string(), "[C(".to_string()];
        let stock = canonicalize_stock(Some(&entries));
        assert_eq!(stock.count, 1);
        assert_eq!(stock.unparseable_raw_entries, vec!["[C(".to_string()]);
    }

    #[test]
    fn stock_canonicalization_is_order_independent() {
        let a = vec!["CCO".to_string(), "c1ccccc1".to_string()];
        let b = vec!["c1ccccc1".to_string(), "CCO".to_string()];
        let stock_a = canonicalize_stock(Some(&a));
        let stock_b = canonicalize_stock(Some(&b));
        assert_eq!(stock_a.hash, stock_b.hash);
    }

    // ── rollup_forward_validation: all 5 states ──

    #[test]
    fn forward_validation_not_evaluated_when_none() {
        assert_eq!(
            rollup_forward_validation(None, 2),
            ForwardValidationStatus::NotEvaluated
        );
    }

    #[test]
    fn forward_validation_all_valid() {
        let statuses = vec![StepValidationStatus::Valid, StepValidationStatus::Valid];
        assert_eq!(
            rollup_forward_validation(Some(&statuses), 2),
            ForwardValidationStatus::AllEvaluatedStepsValid
        );
    }

    #[test]
    fn forward_validation_one_invalid_dominates() {
        let statuses = vec![StepValidationStatus::Valid, StepValidationStatus::Invalid];
        assert_eq!(
            rollup_forward_validation(Some(&statuses), 2),
            ForwardValidationStatus::OneOrMoreStepsInvalid
        );
    }

    #[test]
    fn forward_validation_partially_evaluated() {
        let statuses = vec![
            StepValidationStatus::Valid,
            StepValidationStatus::NotEvaluable,
        ];
        assert_eq!(
            rollup_forward_validation(Some(&statuses), 2),
            ForwardValidationStatus::PartiallyEvaluated
        );
    }

    #[test]
    fn forward_validation_validator_error_on_length_mismatch() {
        let statuses = vec![StepValidationStatus::Valid];
        assert_eq!(
            rollup_forward_validation(Some(&statuses), 2),
            ForwardValidationStatus::ValidatorError
        );
    }

    // ── compute_evidence_coverage ──

    #[test]
    fn evidence_coverage_tally_on_hand_built_route() {
        let exact_example = ResolvedReactionExample {
            match_kind: ExampleMatch::ExactSubstrate,
            example: ReactionExample {
                id: "ex1".to_string(),
                target_smiles: "CC(=O)Oc1ccccc1C(=O)O".to_string(),
                precursor_smiles: vec!["CC(=O)Cl".to_string(), "Oc1ccccc1C(=O)O".to_string()],
                conditions: Some(ConditionCandidate {
                    catalysts: vec![],
                    reagents: vec![],
                    bases: vec![],
                    solvents: vec![],
                    temperature_c: None,
                    time_hours: None,
                    atmosphere: None,
                    notes: None,
                    source: MetadataSource::Literature,
                    scope: EvidenceScope::SubstrateSpecific,
                    reference_ids: vec![],
                }),
                reported_yield: Some(ReportedYield {
                    percentage: YieldPercentage::Single(90.0),
                    basis: YieldBasis::Isolated,
                    source: MetadataSource::Literature,
                    scope: EvidenceScope::SubstrateSpecific,
                    reference_ids: vec![],
                }),
                warnings: vec![],
                reference_ids: vec![],
                dataset_record_id: None,
                notes: None,
            },
        };
        let step_with_exact_evidence = ReactionStep {
            evidence: Some(StepEvidence {
                condition_candidates: vec![],
                reported_yields: vec![],
                references: vec![],
                warnings: vec![],
                examples: vec![exact_example],
                template_examples_total: 1,
            }),
            ..leaf(
                "acyl_chloride_from_acid",
                "CC(=O)Oc1ccccc1C(=O)O",
                &["CC(=O)Cl", "Oc1ccccc1C(=O)O"],
            )
        };

        let model_prediction_yield_example = ResolvedReactionExample {
            match_kind: ExampleMatch::TemplateOnly,
            example: ReactionExample {
                id: "ex2".to_string(),
                target_smiles: "CCO".to_string(),
                precursor_smiles: vec!["CC=O".to_string()],
                conditions: None,
                reported_yield: Some(ReportedYield {
                    percentage: YieldPercentage::Single(50.0),
                    basis: YieldBasis::Unknown,
                    source: MetadataSource::ModelPrediction,
                    scope: EvidenceScope::SubstrateSpecific,
                    reference_ids: vec![],
                }),
                warnings: vec![],
                reference_ids: vec![],
                dataset_record_id: None,
                notes: None,
            },
        };
        let step_with_template_only_evidence = ReactionStep {
            evidence: Some(StepEvidence {
                condition_candidates: vec![],
                reported_yields: vec![],
                references: vec![],
                warnings: vec![],
                examples: vec![model_prediction_yield_example],
                template_examples_total: 1,
            }),
            ..leaf("alcohol_oxidation_retro", "CCO", &["CC=O"])
        };

        let step_without_evidence = leaf("ester_cleavage", "CC(=O)Oc1ccccc1", &["CCO", "CC(=O)O"]);

        let r = route(
            vec![
                step_with_exact_evidence,
                step_with_template_only_evidence,
                step_without_evidence,
            ],
            &["Oc1ccccc1C(=O)O", "CC=O", "CCO", "CC(=O)O"],
        );

        let coverage = compute_evidence_coverage(&r);
        assert_eq!(coverage.exact_substrate_evidence_steps, 1);
        assert_eq!(coverage.template_level_evidence_steps, 1);
        assert_eq!(coverage.steps_without_evidence, 1);
        // The exact-substrate step's reported yield is Literature-sourced ->
        // counted. The template-only step's reported yield is
        // ModelPrediction-sourced -> must NOT be counted (design doc §4.4).
        assert_eq!(coverage.steps_with_reported_yield, 1);
        assert_eq!(coverage.steps_with_conditions, 1);
        assert_eq!(coverage.steps_with_warnings, 0);
    }

    #[test]
    fn evidence_coverage_empty_route_is_all_zero() {
        let r = route(vec![], &[]);
        assert_eq!(compute_evidence_coverage(&r), EvidenceCoverage::default());
    }
}
