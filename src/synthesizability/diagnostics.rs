//! Transparent route-feasibility diagnostics derived from an existing
//! [`RouteAssessment`](super::RouteAssessment).
//!
//! This module does not predict feasibility, train a model, decompose a target
//! into scored fragments, or change search/ranking. It only projects facts the
//! Synthesizability Kernel already computed and identifies missing per-step
//! evidence/condition records. There is deliberately no aggregate numeric
//! feasibility score.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use serde::Serialize;

use crate::evidence::ExampleMatch;
use crate::search::Route;

use super::{
    ElementAccountingStatus, EvidenceCoverage, ForwardValidationStatus, HardFailure,
    RouteAssessment, StockTerminationStatus, ValidationGap, provenance,
};

pub const ROUTE_FEASIBILITY_DIAGNOSTICS_SCHEMA_VERSION: u32 = 1;

/// A categorical summary, never a probability or laboratory-success claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FeasibilityDiagnosticDisposition {
    SupportedByAvailableChecks,
    ReviewNeeded,
    RejectedByConfiguredChecks,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteCompletionStatus {
    CompleteToVerifiedStock,
    CompletionNotVerified,
    DoesNotReachConfiguredStock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuralDiagnosticStatus {
    Pass,
    Fail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageStatus {
    NotApplicable,
    Missing,
    Partial,
    Complete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitingStepCode {
    TargetElementUnaccounted,
    ForwardValidationFailed,
    ForwardValidationNotEvaluable,
    ReagentOmissionAccountingGap,
    UnaccountedTargetElementNotEnforced,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LimitingStep {
    pub step_index: usize,
    pub template_id: Option<String>,
    pub reasons: Vec<LimitingStepCode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MissingInformationCode {
    StockNotSupplied,
    StockSuppliedButEmpty,
    StockIdentityUnavailable,
    StockCheckNotPerformed,
    StockCheckError,
    StockProvenanceHashMissing,
    ElementAccountingNotEvaluable,
    ForwardValidationNotRun,
    ForwardValidationPartiallyEvaluated,
    ForwardValidatorError,
    ReactionEvidenceNotAttached,
    ExactSubstrateEvidenceNotAttached,
    EvidenceBackedConditionsNotAttached,
    BestEffortRouteOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MissingInformation {
    pub code: MissingInformationCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_index: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RouteFeasibilityDiagnostics {
    pub schema_version: u32,
    pub route_id: String,
    pub disposition: FeasibilityDiagnosticDisposition,
    pub completion_status: RouteCompletionStatus,
    pub route_depth: u32,
    pub route_cost: f64,
    pub structural_status: StructuralDiagnosticStatus,
    pub stock_endpoint_status: StockTerminationStatus,
    pub target_element_accounting_status: ElementAccountingStatus,
    pub forward_validation_status: ForwardValidationStatus,
    pub evidence_coverage: EvidenceCoverage,
    pub evidence_coverage_status: CoverageStatus,
    pub evidence_backed_condition_coverage_status: CoverageStatus,
    pub limiting_steps: Vec<LimitingStep>,
    pub hard_failures: Vec<HardFailure>,
    pub policy_validation_gaps: Vec<ValidationGap>,
    pub missing_information: Vec<MissingInformation>,
    pub warnings: Vec<String>,
    /// Stable interpretation boundary shipped with every serialized report.
    pub interpretation: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeasibilityDiagnosticsError {
    InvalidTarget,
    AssessmentRouteMismatch {
        expected_route_id: String,
        assessment_route_id: String,
    },
}

impl fmt::Display for FeasibilityDiagnosticsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTarget => write!(f, "target SMILES cannot be canonicalized"),
            Self::AssessmentRouteMismatch {
                expected_route_id,
                assessment_route_id,
            } => write!(
                f,
                "route assessment does not match route: expected {expected_route_id}, got {assessment_route_id}"
            ),
        }
    }
}

impl Error for FeasibilityDiagnosticsError {}

fn coverage_status(total_steps: usize, covered_steps: usize) -> CoverageStatus {
    if total_steps == 0 {
        CoverageStatus::NotApplicable
    } else if covered_steps == 0 {
        CoverageStatus::Missing
    } else if covered_steps >= total_steps {
        CoverageStatus::Complete
    } else {
        CoverageStatus::Partial
    }
}

fn has_exact_substrate_evidence(step: &crate::search::ReactionStep) -> bool {
    step.evidence.as_ref().is_some_and(|evidence| {
        evidence
            .examples
            .iter()
            .any(|example| example.match_kind == ExampleMatch::ExactSubstrate)
    })
}

fn has_evidence_backed_conditions(step: &crate::search::ReactionStep) -> bool {
    step.evidence.as_ref().is_some_and(|evidence| {
        !evidence.condition_candidates.is_empty()
            || evidence
                .examples
                .iter()
                .any(|example| example.example.conditions.is_some())
    })
}

fn push_limiting_step(
    by_step: &mut BTreeMap<usize, Vec<LimitingStepCode>>,
    step_index: usize,
    code: LimitingStepCode,
) {
    let reasons = by_step.entry(step_index).or_default();
    if !reasons.contains(&code) {
        reasons.push(code);
    }
}

/// Build a deterministic, non-scalar feasibility report from one route and
/// its matching Synthesizability Kernel assessment.
///
/// `target` is used only to recompute the kernel route ID and reject an
/// accidentally mismatched `(route, assessment)` pair. The function performs
/// no search and cannot affect route ranking.
pub fn diagnose_route_feasibility(
    target: &str,
    route: &Route,
    assessment: &RouteAssessment,
) -> Result<RouteFeasibilityDiagnostics, FeasibilityDiagnosticsError> {
    let canonical_target =
        provenance::try_canonicalize(target).ok_or(FeasibilityDiagnosticsError::InvalidTarget)?;
    let expected_route_id = provenance::compute_route_id(&canonical_target, route);
    if expected_route_id != assessment.route_id {
        return Err(FeasibilityDiagnosticsError::AssessmentRouteMismatch {
            expected_route_id,
            assessment_route_id: assessment.route_id.clone(),
        });
    }

    let structural_status = if assessment.hard_failures.iter().any(|failure| {
        matches!(
            failure,
            HardFailure::RouteStructureUnparseable | HardFailure::RouteGraphInconsistent
        )
    }) {
        StructuralDiagnosticStatus::Fail
    } else {
        StructuralDiagnosticStatus::Pass
    };

    let completion_status = match assessment.stock_termination_status {
        StockTerminationStatus::AllLeavesVerifiedInConfiguredStock => {
            RouteCompletionStatus::CompleteToVerifiedStock
        }
        StockTerminationStatus::OneOrMoreLeavesNotInStock => {
            RouteCompletionStatus::DoesNotReachConfiguredStock
        }
        _ => RouteCompletionStatus::CompletionNotVerified,
    };

    let mut by_step: BTreeMap<usize, Vec<LimitingStepCode>> = BTreeMap::new();
    for failure in &assessment.hard_failures {
        match failure {
            HardFailure::UnaccountedTargetElement { step_index } => push_limiting_step(
                &mut by_step,
                *step_index,
                LimitingStepCode::TargetElementUnaccounted,
            ),
            HardFailure::ForwardValidationFailed { step_index } => push_limiting_step(
                &mut by_step,
                *step_index,
                LimitingStepCode::ForwardValidationFailed,
            ),
            _ => {}
        }
    }
    for gap in &assessment.validation_gaps {
        match gap {
            ValidationGap::StepNotEvaluable { step_index } => push_limiting_step(
                &mut by_step,
                *step_index,
                LimitingStepCode::ForwardValidationNotEvaluable,
            ),
            ValidationGap::ReagentOmissionAccountingGap { step_index, .. } => push_limiting_step(
                &mut by_step,
                *step_index,
                LimitingStepCode::ReagentOmissionAccountingGap,
            ),
            ValidationGap::UnaccountedTargetElementNotEnforced { step_index } => {
                push_limiting_step(
                    &mut by_step,
                    *step_index,
                    LimitingStepCode::UnaccountedTargetElementNotEnforced,
                )
            }
            _ => {}
        }
    }
    let limiting_steps = by_step
        .into_iter()
        .map(|(step_index, reasons)| LimitingStep {
            step_index,
            template_id: route
                .steps
                .get(step_index)
                .map(|step| step.template_id.clone()),
            reasons,
        })
        .collect();

    let mut missing_information = Vec::new();
    let mut push_missing = |code, step_index| {
        let item = MissingInformation { code, step_index };
        if !missing_information.contains(&item) {
            missing_information.push(item);
        }
    };

    match assessment.stock_termination_status {
        StockTerminationStatus::StockNotSupplied => {
            push_missing(MissingInformationCode::StockNotSupplied, None)
        }
        StockTerminationStatus::StockSuppliedButEmpty => {
            push_missing(MissingInformationCode::StockSuppliedButEmpty, None)
        }
        StockTerminationStatus::StockIdentityUnavailable => {
            push_missing(MissingInformationCode::StockIdentityUnavailable, None)
        }
        StockTerminationStatus::StockCheckNotPerformed => {
            push_missing(MissingInformationCode::StockCheckNotPerformed, None)
        }
        StockTerminationStatus::StockCheckError => {
            push_missing(MissingInformationCode::StockCheckError, None)
        }
        StockTerminationStatus::AllLeavesVerifiedInConfiguredStock
        | StockTerminationStatus::OneOrMoreLeavesNotInStock => {}
    }

    if assessment.target_element_accounting_status == ElementAccountingStatus::NotEvaluable {
        push_missing(MissingInformationCode::ElementAccountingNotEvaluable, None);
    }
    if !route.steps.is_empty() {
        match assessment.forward_validation_status {
            ForwardValidationStatus::NotEvaluated => {
                push_missing(MissingInformationCode::ForwardValidationNotRun, None)
            }
            ForwardValidationStatus::PartiallyEvaluated => push_missing(
                MissingInformationCode::ForwardValidationPartiallyEvaluated,
                None,
            ),
            ForwardValidationStatus::ValidatorError => {
                push_missing(MissingInformationCode::ForwardValidatorError, None)
            }
            ForwardValidationStatus::AllEvaluatedStepsValid
            | ForwardValidationStatus::OneOrMoreStepsInvalid => {}
        }
    }

    for gap in &assessment.validation_gaps {
        match gap {
            ValidationGap::StockProvenanceHashMissing => {
                push_missing(MissingInformationCode::StockProvenanceHashMissing, None)
            }
            ValidationGap::BestEffortRouteOnly => {
                push_missing(MissingInformationCode::BestEffortRouteOnly, None)
            }
            _ => {}
        }
    }

    for (step_index, step) in route.steps.iter().enumerate() {
        if step.evidence.is_none() {
            push_missing(
                MissingInformationCode::ReactionEvidenceNotAttached,
                Some(step_index),
            );
        } else if !has_exact_substrate_evidence(step) {
            push_missing(
                MissingInformationCode::ExactSubstrateEvidenceNotAttached,
                Some(step_index),
            );
        }
        if !has_evidence_backed_conditions(step) {
            push_missing(
                MissingInformationCode::EvidenceBackedConditionsNotAttached,
                Some(step_index),
            );
        }
    }

    let evidence_covered_steps = route
        .steps
        .len()
        .saturating_sub(assessment.evidence_coverage.steps_without_evidence);
    let evidence_coverage_status = coverage_status(route.steps.len(), evidence_covered_steps);
    let evidence_backed_condition_coverage_status = coverage_status(
        route.steps.len(),
        assessment.evidence_coverage.steps_with_conditions,
    );

    let disposition = if !assessment.hard_failures.is_empty() {
        FeasibilityDiagnosticDisposition::RejectedByConfiguredChecks
    } else if !assessment.validation_gaps.is_empty() || !missing_information.is_empty() {
        FeasibilityDiagnosticDisposition::ReviewNeeded
    } else {
        FeasibilityDiagnosticDisposition::SupportedByAvailableChecks
    };

    Ok(RouteFeasibilityDiagnostics {
        schema_version: ROUTE_FEASIBILITY_DIAGNOSTICS_SCHEMA_VERSION,
        route_id: assessment.route_id.clone(),
        disposition,
        completion_status,
        route_depth: assessment.route_depth,
        route_cost: assessment.route_cost,
        structural_status,
        stock_endpoint_status: assessment.stock_termination_status,
        target_element_accounting_status: assessment.target_element_accounting_status,
        forward_validation_status: assessment.forward_validation_status,
        evidence_coverage: assessment.evidence_coverage,
        evidence_coverage_status,
        evidence_backed_condition_coverage_status,
        limiting_steps,
        hard_failures: assessment.hard_failures.clone(),
        policy_validation_gaps: assessment.validation_gaps.clone(),
        missing_information,
        warnings: assessment.warnings.clone(),
        interpretation: "Deterministic diagnostics from existing route, stock, validation, and evidence records; not an experimental-success probability or a synthetic-accessibility score.",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::{AtomEconomyStatus, ReactionStep};

    fn step(template_id: &str) -> ReactionStep {
        ReactionStep {
            rule: template_id.to_string(),
            template_id: template_id.to_string(),
            target: "CC".to_string(),
            precursors: vec!["C".to_string()],
            conditions: None,
            atom_economy: None,
            atom_economy_raw_percent: None,
            atom_economy_status: AtomEconomyStatus::NotEvaluable,
            step_confidence: 1.0,
            procedure_hint: None,
            reaction_family: None,
            metadata_source: None,
            metadata_scope: None,
            evidence: None,
        }
    }

    fn route(step_count: usize) -> Route {
        Route {
            steps: (0..step_count)
                .map(|index| step(&format!("rule:{index}")))
                .collect(),
            depth: step_count as u32,
            score: 0.0,
            building_blocks: if step_count == 0 {
                Vec::new()
            } else {
                vec!["C".to_string()]
            },
            confidence: 1.0,
            convergency: 0.0,
            success_probability: 1.0,
            route_cost: step_count as f64,
        }
    }

    fn assessment(target: &str, route: &Route) -> RouteAssessment {
        let canonical_target = provenance::try_canonicalize(target).unwrap();
        RouteAssessment {
            route_id: provenance::compute_route_id(&canonical_target, route),
            route_depth: route.depth,
            route_cost: route.route_cost,
            stock_termination_status: StockTerminationStatus::AllLeavesVerifiedInConfiguredStock,
            target_element_accounting_status: ElementAccountingStatus::Accounted,
            forward_validation_status: ForwardValidationStatus::AllEvaluatedStepsValid,
            evidence_coverage: EvidenceCoverage {
                steps_without_evidence: route.steps.len(),
                ..EvidenceCoverage::default()
            },
            hard_failures: Vec::new(),
            validation_gaps: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn direct_stock_route_is_complete_without_step_level_gaps() {
        let route = route(0);
        let report = diagnose_route_feasibility("C", &route, &assessment("C", &route)).unwrap();
        assert_eq!(
            report.completion_status,
            RouteCompletionStatus::CompleteToVerifiedStock
        );
        assert_eq!(
            report.evidence_coverage_status,
            CoverageStatus::NotApplicable
        );
        assert!(report.missing_information.is_empty());
        assert_eq!(
            report.disposition,
            FeasibilityDiagnosticDisposition::SupportedByAvailableChecks
        );
    }

    #[test]
    fn unverified_endpoint_and_missing_step_records_are_explicit() {
        let route = route(1);
        let mut assessment = assessment("CC", &route);
        assessment.stock_termination_status = StockTerminationStatus::StockNotSupplied;
        assessment.forward_validation_status = ForwardValidationStatus::NotEvaluated;
        let report = diagnose_route_feasibility("CC", &route, &assessment).unwrap();
        assert_eq!(
            report.completion_status,
            RouteCompletionStatus::CompletionNotVerified
        );
        assert!(report.missing_information.contains(&MissingInformation {
            code: MissingInformationCode::StockNotSupplied,
            step_index: None,
        }));
        assert!(report.missing_information.contains(&MissingInformation {
            code: MissingInformationCode::ReactionEvidenceNotAttached,
            step_index: Some(0),
        }));
        assert!(report.missing_information.contains(&MissingInformation {
            code: MissingInformationCode::EvidenceBackedConditionsNotAttached,
            step_index: Some(0),
        }));
        assert_eq!(
            report.disposition,
            FeasibilityDiagnosticDisposition::ReviewNeeded
        );
    }

    #[test]
    fn hard_failures_are_not_hidden_by_missing_information() {
        let route = route(1);
        let mut assessment = assessment("CC", &route);
        assessment.stock_termination_status = StockTerminationStatus::OneOrMoreLeavesNotInStock;
        assessment.hard_failures = vec![
            HardFailure::RouteGraphInconsistent,
            HardFailure::StockTerminalMismatch {
                leaf: "C".to_string(),
            },
        ];
        let report = diagnose_route_feasibility("CC", &route, &assessment).unwrap();
        assert_eq!(report.structural_status, StructuralDiagnosticStatus::Fail);
        assert_eq!(
            report.completion_status,
            RouteCompletionStatus::DoesNotReachConfiguredStock
        );
        assert_eq!(
            report.disposition,
            FeasibilityDiagnosticDisposition::RejectedByConfiguredChecks
        );
    }

    #[test]
    fn multiple_limiting_steps_remain_separate_and_ordered() {
        let route = route(2);
        let mut assessment = assessment("CC", &route);
        assessment.hard_failures = vec![
            HardFailure::ForwardValidationFailed { step_index: 0 },
            HardFailure::UnaccountedTargetElement { step_index: 1 },
        ];
        let report = diagnose_route_feasibility("CC", &route, &assessment).unwrap();
        assert_eq!(report.limiting_steps.len(), 2);
        assert_eq!(report.limiting_steps[0].step_index, 0);
        assert_eq!(report.limiting_steps[1].step_index, 1);
        assert_eq!(
            report.limiting_steps[0].reasons,
            vec![LimitingStepCode::ForwardValidationFailed]
        );
        assert_eq!(
            report.limiting_steps[1].reasons,
            vec![LimitingStepCode::TargetElementUnaccounted]
        );
    }

    #[test]
    fn mismatched_assessment_is_rejected() {
        let route = route(1);
        let mut wrong = assessment("CC", &route);
        wrong.route_id = "sha256:not-this-route".to_string();
        assert!(matches!(
            diagnose_route_feasibility("CC", &route, &wrong),
            Err(FeasibilityDiagnosticsError::AssessmentRouteMismatch { .. })
        ));
    }
}
