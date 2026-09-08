//! Versioned, evidence-carrying route interchange output.
//!
//! This schema is an export contract for the normalized Bridge audit. It
//! preserves facts RENKIN actually observed and marks source metadata that
//! current adapters do not retain as `null`; it never invents source versions
//! or original node identifiers.

use serde::Serialize;
use serde_json::Value;

use crate::bridge::audit::{AuditFinding, AuditReport, AuditStatus, CheckStatus};
use crate::bridge::forward::EvidenceBasis;
use crate::bridge::private_stock::PrivateStockReport;
use crate::bridge::route_graph::ReactionEvidence;

pub const ROUTE_INTERCHANGE_SCHEMA_VERSION: u32 = 1;
pub const ADAPTER_LOSS_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LossDisposition {
    Preserved,
    Normalized,
    Inferred,
    Dropped,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AdapterLossField {
    pub field: String,
    pub disposition: LossDisposition,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AdapterLossReport {
    pub schema_version: u32,
    pub fields: Vec<AdapterLossField>,
}

impl AdapterLossReport {
    fn for_audit(report: &AuditReport) -> Self {
        let mut fields = vec![
            AdapterLossField {
                field: "target".into(),
                disposition: LossDisposition::Normalized,
                reason: "target is emitted in RENKIN canonical form".into(),
            },
            AdapterLossField {
                field: "precursors".into(),
                disposition: LossDisposition::Normalized,
                reason: "precursor identities are emitted in RENKIN canonical form".into(),
            },
            AdapterLossField {
                field: "canonical_node_id".into(),
                disposition: LossDisposition::Inferred,
                reason: "derived from normalized route hash and step index".into(),
            },
            AdapterLossField {
                field: "conditions".into(),
                disposition: LossDisposition::Unsupported,
                reason: "condition records are not part of the current canonical schema".into(),
            },
        ];
        if report
            .steps
            .iter()
            .all(|step| step.reaction_evidence.is_some())
        {
            fields.push(AdapterLossField {
                field: "reaction_provenance".into(),
                disposition: LossDisposition::Preserved,
                reason: "source reaction evidence is retained when supplied".into(),
            });
        } else {
            fields.push(AdapterLossField {
                field: "reaction_provenance".into(),
                disposition: LossDisposition::Unsupported,
                reason: "one or more source steps supplied no reaction evidence".into(),
            });
        }
        Self {
            schema_version: ADAPTER_LOSS_SCHEMA_VERSION,
            fields,
        }
    }

    pub fn validate_for_strict_import(&self) -> anyhow::Result<()> {
        if self.schema_version != ADAPTER_LOSS_SCHEMA_VERSION {
            anyhow::bail!(
                "unsupported adapter loss schema_version {}",
                self.schema_version
            );
        }
        if self.fields.is_empty() {
            anyhow::bail!("adapter loss report must contain at least one field record");
        }
        if self
            .fields
            .iter()
            .any(|field| field.field.trim().is_empty())
        {
            anyhow::bail!("adapter loss report contains an empty field name");
        }
        Ok(())
    }
}

/// Validate a canonical interchange document before an audit or replay uses
/// it. This validates the envelope and loss accounting only; chemistry remains
/// the responsibility of the normal audit pipeline.
pub fn validate_strict_import(value: &Value) -> anyhow::Result<()> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("canonical interchange must be a JSON object"))?;
    if object.get("schema_version").and_then(Value::as_u64)
        != Some(ROUTE_INTERCHANGE_SCHEMA_VERSION as u64)
    {
        anyhow::bail!("unsupported canonical interchange schema_version");
    }
    if object
        .get("route_id")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
    {
        anyhow::bail!("canonical interchange route_id is required");
    }
    if !object.get("steps").is_some_and(Value::is_array) {
        anyhow::bail!("canonical interchange steps must be an array");
    }
    let loss_report = object
        .get("loss_report")
        .ok_or_else(|| anyhow::anyhow!("canonical interchange loss_report is required"))?;
    let loss_object = loss_report
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("canonical interchange loss_report must be an object"))?;
    if loss_object.get("schema_version").and_then(Value::as_u64)
        != Some(ADAPTER_LOSS_SCHEMA_VERSION as u64)
    {
        anyhow::bail!("unsupported adapter loss schema_version");
    }
    let fields = loss_object
        .get("fields")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("adapter loss report fields must be an array"))?;
    if fields.is_empty() {
        anyhow::bail!("adapter loss report must contain at least one field record");
    }
    for field in fields {
        let field = field
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("adapter loss field must be an object"))?;
        if field
            .get("field")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
            || field
                .get("reason")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
        {
            anyhow::bail!("adapter loss field and reason are required");
        }
        match field.get("disposition").and_then(Value::as_str) {
            Some("preserved" | "normalized" | "inferred" | "dropped" | "unsupported") => {}
            _ => anyhow::bail!("unknown adapter loss disposition"),
        }
    }
    Ok(())
}

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
    pub loss_report: AdapterLossReport,
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
        loss_report: AdapterLossReport::for_audit(report),
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
        interchange
            .loss_report
            .validate_for_strict_import()
            .unwrap();
        validate_strict_import(&serde_json::to_value(&interchange).unwrap()).unwrap();
        assert!(interchange.loss_report.fields.iter().any(|field| {
            field.field == "reaction_provenance" && field.disposition == LossDisposition::Preserved
        }));
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

    #[test]
    fn strict_import_rejects_missing_loss_report() {
        let value = serde_json::json!({
            "schema_version": 1,
            "route_id": "sha256:test",
            "steps": []
        });
        let error = validate_strict_import(&value).unwrap_err().to_string();
        assert!(error.contains("loss_report"));
    }
}
