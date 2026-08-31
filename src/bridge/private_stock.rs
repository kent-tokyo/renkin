//! Local, policy-aware vendor stock decisions for route leaves.
//!
//! The policy is deliberately evaluated without network access. Exact stock
//! identity is required; relaxed vendor match modes are never used to turn a
//! near match into a purchasable building block.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::bridge::audit::AuditReport;
use crate::vendor_stock::VendorStockIndex;

pub const PRIVATE_STOCK_POLICY_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateStockPolicy {
    pub schema_version: u32,
    pub source_label: String,
    #[serde(default)]
    pub source_revision: Option<String>,
    #[serde(default)]
    pub allowed_vendors: Vec<String>,
    #[serde(default)]
    pub blocked_vendors: Vec<String>,
    #[serde(default)]
    pub max_price: Option<f64>,
    #[serde(default)]
    pub max_lead_time_days: Option<u32>,
    #[serde(default = "default_require_available")]
    pub require_available: bool,
    #[serde(default)]
    pub blocked_smiles: Vec<String>,
}

fn default_require_available() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivateStockDecision {
    Matched,
    Rejected,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivateStockReason {
    PrivateInventoryHit,
    VendorNotAllowed,
    VendorBlocked,
    PriceLimitExceeded,
    LeadTimeExceeded,
    NotAvailable,
    ProhibitedSubstance,
    NoExactVendorRecord,
    InvalidLeafSmiles,
}

#[derive(Debug, Clone, Serialize)]
pub struct PrivateStockDecisionRecord {
    pub smiles: String,
    pub decision: PrivateStockDecision,
    pub reason: PrivateStockReason,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vendor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lead_time_days: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PrivateStockReport {
    pub schema_version: u32,
    pub source_label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_revision: Option<String>,
    pub decisions: Vec<PrivateStockDecisionRecord>,
}

impl PrivateStockPolicy {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.schema_version != PRIVATE_STOCK_POLICY_SCHEMA_VERSION {
            anyhow::bail!(
                "unsupported private stock policy schema_version {}",
                self.schema_version
            );
        }
        if self.source_label.trim().is_empty() {
            anyhow::bail!("private stock policy source_label must not be empty");
        }
        if self.max_price.is_some_and(|p| !p.is_finite() || p < 0.0) {
            anyhow::bail!("private stock policy max_price must be finite and non-negative");
        }
        Ok(())
    }
}

fn leaves(report: &AuditReport) -> BTreeSet<String> {
    let targets: BTreeSet<&str> = report.steps.iter().map(|s| s.target.as_str()).collect();
    report
        .steps
        .iter()
        .flat_map(|s| s.precursors.iter())
        .filter(|s| !targets.contains(s.as_str()))
        .cloned()
        .collect()
}

fn vendor_allowed(vendor: Option<&str>, policy: &PrivateStockPolicy) -> bool {
    let Some(vendor) = vendor else {
        return policy.allowed_vendors.is_empty();
    };
    (policy.allowed_vendors.is_empty() || policy.allowed_vendors.iter().any(|v| v == vendor))
        && !policy.blocked_vendors.iter().any(|v| v == vendor)
}

pub fn assess_report(
    report: &AuditReport,
    index: &VendorStockIndex,
    policy: &PrivateStockPolicy,
) -> PrivateStockReport {
    let blocked: BTreeSet<String> = policy.blocked_smiles.iter().cloned().collect();
    let mut decisions = Vec::new();
    for smiles in leaves(report) {
        if blocked.contains(&smiles) {
            decisions.push(PrivateStockDecisionRecord {
                smiles: smiles.clone(),
                decision: PrivateStockDecision::Rejected,
                reason: PrivateStockReason::ProhibitedSubstance,
                vendor: None,
                catalog_id: None,
                price: None,
                lead_time_days: None,
            });
            continue;
        }
        let found = match index.lookup(&smiles, crate::vendor_stock::MatchMode::Exact) {
            Ok(found) => found,
            Err(_) => {
                decisions.push(PrivateStockDecisionRecord {
                    smiles,
                    decision: PrivateStockDecision::Unknown,
                    reason: PrivateStockReason::InvalidLeafSmiles,
                    vendor: None,
                    catalog_id: None,
                    price: None,
                    lead_time_days: None,
                });
                continue;
            }
        };
        let Some(found) = found else {
            decisions.push(PrivateStockDecisionRecord {
                smiles,
                decision: PrivateStockDecision::Unknown,
                reason: PrivateStockReason::NoExactVendorRecord,
                vendor: None,
                catalog_id: None,
                price: None,
                lead_time_days: None,
            });
            continue;
        };
        let mut rejected_reason = None;
        for &record_index in &found.record_indices {
            let record = &index.records()[record_index];
            if !vendor_allowed(record.vendor.as_deref(), policy) {
                rejected_reason.get_or_insert(
                    if record
                        .vendor
                        .as_deref()
                        .is_some_and(|v| policy.blocked_vendors.iter().any(|b| b == v))
                    {
                        PrivateStockReason::VendorBlocked
                    } else {
                        PrivateStockReason::VendorNotAllowed
                    },
                );
                continue;
            }
            if policy.require_available && !record.available {
                rejected_reason.get_or_insert(PrivateStockReason::NotAvailable);
                continue;
            }
            if policy
                .max_price
                .is_some_and(|max| record.price.is_none_or(|p| p > max))
            {
                rejected_reason.get_or_insert(PrivateStockReason::PriceLimitExceeded);
                continue;
            }
            if policy
                .max_lead_time_days
                .is_some_and(|max| record.lead_time_days.is_none_or(|d| d > max))
            {
                rejected_reason.get_or_insert(PrivateStockReason::LeadTimeExceeded);
                continue;
            }
            decisions.push(PrivateStockDecisionRecord {
                smiles: smiles.clone(),
                decision: PrivateStockDecision::Matched,
                reason: PrivateStockReason::PrivateInventoryHit,
                vendor: record.vendor.clone(),
                catalog_id: record.id.clone(),
                price: record.price,
                lead_time_days: record.lead_time_days,
            });
            rejected_reason = None;
            break;
        }
        if let Some(reason) = rejected_reason {
            decisions.push(PrivateStockDecisionRecord {
                smiles,
                decision: PrivateStockDecision::Rejected,
                reason,
                vendor: None,
                catalog_id: None,
                price: None,
                lead_time_days: None,
            });
        }
    }
    PrivateStockReport {
        schema_version: PRIVATE_STOCK_POLICY_SCHEMA_VERSION,
        source_label: policy.source_label.clone(),
        source_revision: policy.source_revision.clone(),
        decisions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::audit::{AuditReport, AuditStatus, CheckStatus};
    use crate::bridge::route_graph::RouteSource;
    use crate::vendor_stock::VendorStockRecord;

    fn report() -> AuditReport {
        AuditReport {
            source: RouteSource::Renkin,
            status: AuditStatus::Partial,
            route_tree_parseable: true,
            reaction_steps_parseable: Some(true),
            stock_validation: None,
            target_element_accounting_status: None,
            normalized_route_sha256: None,
            steps: vec![crate::bridge::audit::AuditedStep {
                target: "CC=O".into(),
                precursors: vec!["CCO".into(), "CO".into()],
                forward_validation: crate::bridge::forward::ForwardValidationResult {
                    status: CheckStatus::NotEvaluable,
                    method: "test",
                    evidence_basis: None,
                    reason: None,
                },
            }],
            findings: vec![],
        }
    }

    #[test]
    fn policy_returns_distinct_match_reject_and_unknown() {
        let index = VendorStockIndex::from_records(vec![VendorStockRecord {
            id: Some("e".into()),
            smiles: "CCO".into(),
            vendor: Some("Acme".into()),
            price: Some(12.0),
            lead_time_days: Some(3),
            available: true,
        }])
        .unwrap();
        let policy = PrivateStockPolicy {
            schema_version: 1,
            source_label: "private".into(),
            source_revision: None,
            allowed_vendors: vec!["Acme".into()],
            blocked_vendors: vec![],
            max_price: Some(20.0),
            max_lead_time_days: Some(5),
            require_available: true,
            blocked_smiles: vec![],
        };
        let out = assess_report(&report(), &index, &policy);
        assert_eq!(
            out.decisions
                .iter()
                .find(|d| d.smiles == "CCO")
                .unwrap()
                .decision,
            PrivateStockDecision::Matched
        );
        assert_eq!(
            out.decisions
                .iter()
                .find(|d| d.smiles == "CO")
                .unwrap()
                .decision,
            PrivateStockDecision::Unknown
        );
    }
}
