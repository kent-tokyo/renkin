//! Deterministic, evidence-aware chemical route review.
//!
//! This is intentionally not a quality score and never substitutes for a
//! chemist. It reports which review dimensions are supported by the route
//! audit and which remain unevaluable.

use serde::Serialize;

use crate::bridge::audit::{AuditReport, AuditStatus, CheckStatus};

pub const CHEMICAL_REVIEW_RUBRIC_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    Pass,
    Review,
    NotEvaluable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewSeverity {
    Informational,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDimension {
    Structure,
    Stock,
    ForwardReplay,
    SelectivityRisk,
    ConditionRisk,
    SubstrateScopeRisk,
    ProtectingGroupIssue,
    StrategicRouteIssue,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReviewFinding {
    pub dimension: ReviewDimension,
    pub code: String,
    pub status: ReviewStatus,
    pub severity: ReviewSeverity,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChemicalReview {
    pub rubric_version: u32,
    pub judge_id: &'static str,
    pub status: ReviewStatus,
    pub findings: Vec<ReviewFinding>,
}

fn finding(
    dimension: ReviewDimension,
    code: &str,
    status: ReviewStatus,
    severity: ReviewSeverity,
    reason: &str,
) -> ReviewFinding {
    ReviewFinding {
        dimension,
        code: code.to_string(),
        status,
        severity,
        reason: reason.to_string(),
    }
}

/// Derive the review from existing audit facts only. No conditions,
/// selectivity, substrate scope, protecting-group semantics, or strategic
/// intent are inferred when the interchange document does not carry them.
pub fn review_report(report: &AuditReport) -> ChemicalReview {
    let mut findings = Vec::new();
    let structure_status = if report.route_tree_parseable {
        ReviewStatus::Pass
    } else {
        ReviewStatus::Review
    };
    findings.push(finding(
        ReviewDimension::Structure,
        if report.route_tree_parseable {
            "structure_audit_passed"
        } else {
            "route_tree_not_parseable"
        },
        structure_status,
        if report.route_tree_parseable {
            ReviewSeverity::Informational
        } else {
            ReviewSeverity::High
        },
        if report.route_tree_parseable {
            "route graph was normalized and structural checks completed"
        } else {
            "route graph could not be normalized; inspect the audit findings"
        },
    ));

    findings.push(match report.stock_validation.as_ref().map(|v| v.status) {
        Some(CheckStatus::Pass) => finding(
            ReviewDimension::Stock,
            "stock_audit_passed",
            ReviewStatus::Pass,
            ReviewSeverity::Informational,
            "all audited leaves matched the configured stock set",
        ),
        Some(CheckStatus::Fail) => finding(
            ReviewDimension::Stock,
            "stock_policy_mismatch",
            ReviewStatus::Review,
            ReviewSeverity::High,
            "one or more leaves failed the configured stock check",
        ),
        Some(CheckStatus::NotEvaluable) | None => finding(
            ReviewDimension::Stock,
            "stock_not_provided",
            ReviewStatus::NotEvaluable,
            ReviewSeverity::Informational,
            "no stock set was supplied to the audit",
        ),
    });

    let forward_failed = report
        .steps
        .iter()
        .any(|s| s.forward_validation.status == CheckStatus::Fail);
    let forward_unknown = report
        .steps
        .iter()
        .any(|s| s.forward_validation.status == CheckStatus::NotEvaluable);
    findings.push(if forward_failed {
        finding(
            ReviewDimension::ForwardReplay,
            "forward_replay_failed",
            ReviewStatus::Review,
            ReviewSeverity::High,
            "at least one declared reaction did not reproduce its recorded parent",
        )
    } else if forward_unknown {
        finding(
            ReviewDimension::ForwardReplay,
            "forward_replay_not_evaluable",
            ReviewStatus::NotEvaluable,
            ReviewSeverity::Informational,
            "reaction provenance was missing or could not be replayed",
        )
    } else if report.steps.is_empty() {
        finding(
            ReviewDimension::ForwardReplay,
            "no_reaction_steps",
            ReviewStatus::NotEvaluable,
            ReviewSeverity::Informational,
            "the route contains no reaction step to replay",
        )
    } else {
        finding(
            ReviewDimension::ForwardReplay,
            "forward_replay_passed",
            ReviewStatus::Pass,
            ReviewSeverity::Informational,
            "all declared reaction steps reproduced their recorded parents",
        )
    });

    for (dimension, code, reason) in [
        (
            ReviewDimension::SelectivityRisk,
            "selectivity_evidence_not_provided",
            "the route record has no selectivity evidence or stereochemical outcome rubric",
        ),
        (
            ReviewDimension::ConditionRisk,
            "condition_evidence_not_provided",
            "the route record has no experimentally validated condition record",
        ),
        (
            ReviewDimension::SubstrateScopeRisk,
            "substrate_scope_evidence_not_provided",
            "reaction identity alone does not establish substrate-specific scope",
        ),
        (
            ReviewDimension::ProtectingGroupIssue,
            "protecting_group_review_not_evaluable",
            "protecting-group compatibility was not declared by the route source",
        ),
        (
            ReviewDimension::StrategicRouteIssue,
            "strategic_route_review_not_evaluable",
            "route strategy and alternatives require an explicit review rubric or human judgement",
        ),
    ] {
        findings.push(finding(
            dimension,
            code,
            ReviewStatus::NotEvaluable,
            ReviewSeverity::Informational,
            reason,
        ));
    }

    let status = if report.status == AuditStatus::Fail
        || structure_status == ReviewStatus::Review
        || forward_failed
        || report
            .stock_validation
            .as_ref()
            .is_some_and(|v| v.status == CheckStatus::Fail)
    {
        ReviewStatus::Review
    } else if findings
        .iter()
        .any(|f| f.status == ReviewStatus::NotEvaluable)
    {
        ReviewStatus::NotEvaluable
    } else {
        ReviewStatus::Pass
    };
    ChemicalReview {
        rubric_version: CHEMICAL_REVIEW_RUBRIC_VERSION,
        judge_id: "renkin-deterministic",
        status,
        findings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_stock_and_chemical_evidence_are_not_evaluable() {
        let report = AuditReport {
            source: crate::bridge::route_graph::RouteSource::Renkin,
            status: AuditStatus::Partial,
            route_tree_parseable: true,
            reaction_steps_parseable: Some(true),
            stock_validation: None,
            target_element_accounting_status: None,
            normalized_route_sha256: None,
            steps: vec![],
            findings: vec![],
        };
        let review = review_report(&report);
        assert_eq!(review.rubric_version, 1);
        assert_eq!(review.judge_id, "renkin-deterministic");
        assert_eq!(review.status, ReviewStatus::NotEvaluable);
        assert!(
            review
                .findings
                .iter()
                .any(|f| f.code == "stock_not_provided")
        );
        assert!(
            review
                .findings
                .iter()
                .any(|f| f.code == "condition_evidence_not_provided")
        );
    }
}
