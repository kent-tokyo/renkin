//! Versioned, evidence-carrying route interchange output.
//!
//! This schema is an export contract for the normalized Bridge audit. It
//! preserves facts RENKIN actually observed and marks source metadata that
//! current adapters do not retain as `null`; it never invents source versions
//! or original node identifiers.

use serde::Serialize;

use crate::bridge::audit::{AuditFinding, AuditReport, AuditStatus, CheckStatus};
use crate::bridge::forward::EvidenceBasis;
use crate::bridge::private_stock::PrivateStockReport;
use crate::bridge::route_graph::ReactionEvidence;

pub const ROUTE_INTERCHANGE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize)]
pub struct RouteInterchange {
    pub schema_version: u32,
    pub source_tool: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_route_id: Option<String>,
    pub route_id: String,
    pub audit_status: AuditStatus,
    pub steps: Vec<InterchangeStep>,
    pub audit_findings: Vec<AuditFinding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stock_provenance: Option<StockProvenance>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InterchangeStep {
    pub canonical_node_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_node_id: Option<String>,
    pub target: String,
    pub precursors: Vec<String>,
    pub reaction_provenance: ReactionProvenance,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReactionProvenance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reaction_evidence: Option<ReactionEvidence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence_basis: Option<EvidenceBasis>,
    pub forward_replay_status: CheckStatus,
    /// Whether this export retained the source tool's reaction representation.
    /// `false` is explicit when the adapter supplied no reaction evidence.
    pub source_representation_retained: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct StockProvenance {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configured_stock_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_stock_policy_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_stock: Option<PrivateStockReport>,
}

pub fn from_audit_report(
    source_tool: &'static str,
    source_version: Option<String>,
    source_route_id: Option<String>,
    original_node_ids: &[Option<String>],
    report: &AuditReport,
    stock: Option<StockProvenance>,
) -> RouteInterchange {
    let route_id = report
        .normalized_route_sha256
        .clone()
        .unwrap_or_else(|| "unavailable".to_string());
    let steps = report
        .steps
        .iter()
        .enumerate()
        .map(|(index, step)| InterchangeStep {
            canonical_node_id: format!("{route_id}:step:{index}"),
            original_node_id: original_node_ids.get(index).cloned().flatten(),
            target: step.target.clone(),
            precursors: step.precursors.clone(),
            reaction_provenance: ReactionProvenance {
                source_representation_retained: step.reaction_evidence.is_some(),
                reaction_evidence: step.reaction_evidence.clone(),
                evidence_basis: step.forward_validation.evidence_basis,
                forward_replay_status: step.forward_validation.status,
            },
        })
        .collect();
    RouteInterchange {
        schema_version: ROUTE_INTERCHANGE_SCHEMA_VERSION,
        source_tool,
        source_version,
        source_route_id,
        route_id,
        audit_status: report.status,
        steps,
        audit_findings: report.findings.clone(),
        stock_provenance: stock,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::audit::{AuditReport, AuditedStep};
    use crate::bridge::route_graph::RouteSource;

    #[test]
    fn export_does_not_invent_source_identity() {
        let report = AuditReport {
            source: RouteSource::AiZynthFinder,
            status: AuditStatus::Partial,
            route_tree_parseable: true,
            reaction_steps_parseable: Some(true),
            stock_validation: None,
            target_element_accounting_status: None,
            normalized_route_sha256: Some("sha256:abc".into()),
            steps: vec![AuditedStep {
                target: "CCO".into(),
                precursors: vec!["C".into(), "CO".into()],
                reaction_evidence: Some(ReactionEvidence::SyntheseusReaction {
                    reaction_smiles: "C.CO>>CCO".into(),
                }),
                forward_validation: crate::bridge::forward::ForwardValidationResult {
                    status: CheckStatus::NotEvaluable,
                    method: "declared_reaction_replay",
                    evidence_basis: None,
                    reason: None,
                },
            }],
            findings: vec![],
        };
        let interchange = from_audit_report("aizynthfinder", None, None, &[], &report, None);
        assert_eq!(interchange.schema_version, 1);
        assert!(interchange.source_version.is_none());
        assert!(interchange.source_route_id.is_none());
        assert!(interchange.steps[0].original_node_id.is_none());
        assert!(
            interchange.steps[0]
                .reaction_provenance
                .source_representation_retained
        );
        assert!(matches!(
            interchange.steps[0].reaction_provenance.reaction_evidence,
            Some(ReactionEvidence::SyntheseusReaction { .. })
        ));
    }
}
