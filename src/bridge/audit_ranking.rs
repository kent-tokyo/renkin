//! Conservative multi-objective ranking for already-audited routes.
//!
//! This is not a search heuristic. It ranks only after audit and keeps hard
//! rejection, needs-review, missing data, units, and process boundaries
//! explicit. Unknown values are never silently converted to zero.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

pub const AUDIT_RANKING_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditEligibility {
    Eligible,
    Rejected,
    NeedsReview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectiveDirection {
    Minimize,
    Maximize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RankingAxis {
    pub key: String,
    pub direction: ObjectiveDirection,
    /// `None` means the candidate is incomparable on this axis.
    pub value: Option<f64>,
    pub unit: String,
    /// For example, a procurement policy revision or a process-mass boundary.
    pub basis: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RankingCandidate {
    pub route_id: String,
    pub eligibility: AuditEligibility,
    pub axes: Vec<RankingAxis>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParetoStatus {
    Nondominated,
    Dominated,
    Incomparable,
    Rejected,
    NeedsReview,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ParetoVerdict {
    pub route_id: String,
    pub status: ParetoStatus,
    pub dominated_by: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ParetoReceipt {
    pub schema_version: u32,
    pub verdicts: Vec<ParetoVerdict>,
}

/// Produce a deterministic Pareto receipt. Candidates are only compared when
/// both are eligible, all axis values are finite, and their axis keys,
/// directions, units, and bases match exactly. Every other eligible candidate
/// is returned as `incomparable` rather than being ranked optimistically.
pub fn pareto_rank(candidates: &[RankingCandidate]) -> anyhow::Result<ParetoReceipt> {
    let mut by_id = BTreeMap::new();
    for candidate in candidates {
        if candidate.route_id.trim().is_empty()
            || by_id.insert(&candidate.route_id, candidate).is_some()
        {
            anyhow::bail!("audit ranking requires unique, non-empty route_id values");
        }
        validate_axes(&candidate.axes)?;
    }

    let mut verdicts = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let status = match candidate.eligibility {
            AuditEligibility::Rejected => ParetoStatus::Rejected,
            AuditEligibility::NeedsReview => ParetoStatus::NeedsReview,
            AuditEligibility::Eligible if !has_complete_axes(candidate) => {
                ParetoStatus::Incomparable
            }
            AuditEligibility::Eligible => {
                let comparable = candidates
                    .iter()
                    .filter(|other| {
                        other.route_id != candidate.route_id
                            && other.eligibility == AuditEligibility::Eligible
                            && has_complete_axes(other)
                            && same_axis_contract(candidate, other)
                    })
                    .collect::<Vec<_>>();
                if comparable.is_empty()
                    && candidates.iter().any(|other| {
                        other.route_id != candidate.route_id
                            && other.eligibility == AuditEligibility::Eligible
                    })
                {
                    ParetoStatus::Incomparable
                } else if comparable.iter().any(|other| dominates(other, candidate)) {
                    ParetoStatus::Dominated
                } else {
                    ParetoStatus::Nondominated
                }
            }
        };
        let mut dominated_by = candidates
            .iter()
            .filter(|other| {
                candidate.eligibility == AuditEligibility::Eligible
                    && other.eligibility == AuditEligibility::Eligible
                    && has_complete_axes(candidate)
                    && has_complete_axes(other)
                    && same_axis_contract(candidate, other)
                    && dominates(other, candidate)
            })
            .map(|other| other.route_id.clone())
            .collect::<Vec<_>>();
        dominated_by.sort();
        verdicts.push(ParetoVerdict {
            route_id: candidate.route_id.clone(),
            status,
            dominated_by,
        });
    }
    verdicts.sort_by(|left, right| left.route_id.cmp(&right.route_id));
    Ok(ParetoReceipt {
        schema_version: AUDIT_RANKING_SCHEMA_VERSION,
        verdicts,
    })
}

fn validate_axes(axes: &[RankingAxis]) -> anyhow::Result<()> {
    if axes.is_empty() {
        anyhow::bail!("audit ranking candidate requires at least one axis");
    }
    let mut keys = BTreeSet::new();
    for axis in axes {
        if axis.key.trim().is_empty() || axis.unit.trim().is_empty() || axis.basis.trim().is_empty()
        {
            anyhow::bail!("audit ranking axis key, unit, and basis are required");
        }
        if !keys.insert(&axis.key) {
            anyhow::bail!("audit ranking axis keys must be unique per route");
        }
        if axis.value.is_some_and(|value| !value.is_finite()) {
            anyhow::bail!("audit ranking axis values must be finite when supplied");
        }
    }
    Ok(())
}

fn has_complete_axes(candidate: &RankingCandidate) -> bool {
    candidate.axes.iter().all(|axis| axis.value.is_some())
}

fn same_axis_contract(left: &RankingCandidate, right: &RankingCandidate) -> bool {
    if left.axes.len() != right.axes.len() {
        return false;
    }
    let right = right
        .axes
        .iter()
        .map(|axis| (&axis.key, axis))
        .collect::<BTreeMap<_, _>>();
    left.axes.iter().all(|axis| {
        right.get(&axis.key).is_some_and(|other| {
            axis.direction == other.direction
                && axis.unit == other.unit
                && axis.basis == other.basis
        })
    })
}

fn dominates(left: &RankingCandidate, right: &RankingCandidate) -> bool {
    let right = right
        .axes
        .iter()
        .map(|axis| (&axis.key, axis))
        .collect::<BTreeMap<_, _>>();
    let mut strictly_better = false;
    for axis in &left.axes {
        let Some(other) = right.get(&axis.key) else {
            return false;
        };
        let (Some(left), Some(right)) = (axis.value, other.value) else {
            return false;
        };
        let (not_worse, better) = match axis.direction {
            ObjectiveDirection::Minimize => (left <= right, left < right),
            ObjectiveDirection::Maximize => (left >= right, left > right),
        };
        if !not_worse {
            return false;
        }
        strictly_better |= better;
    }
    strictly_better
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(route_id: &str, price: Option<f64>, pmi: Option<f64>) -> RankingCandidate {
        RankingCandidate {
            route_id: route_id.into(),
            eligibility: AuditEligibility::Eligible,
            axes: vec![
                RankingAxis {
                    key: "price".into(),
                    direction: ObjectiveDirection::Minimize,
                    value: price,
                    unit: "USD".into(),
                    basis: "vendor policy v1".into(),
                },
                RankingAxis {
                    key: "pmi".into(),
                    direction: ObjectiveDirection::Minimize,
                    value: pmi,
                    unit: "kg/kg".into(),
                    basis: "batch boundary v1".into(),
                },
            ],
        }
    }

    #[test]
    fn pareto_keeps_tradeoffs_and_marks_dominated_routes() {
        let receipt = pareto_rank(&[
            candidate("sha256:a", Some(10.0), Some(20.0)),
            candidate("sha256:b", Some(12.0), Some(18.0)),
            candidate("sha256:c", Some(15.0), Some(25.0)),
        ])
        .unwrap();
        assert_eq!(receipt.verdicts[0].status, ParetoStatus::Nondominated);
        assert_eq!(receipt.verdicts[1].status, ParetoStatus::Nondominated);
        assert_eq!(receipt.verdicts[2].status, ParetoStatus::Dominated);
        assert_eq!(
            receipt.verdicts[2].dominated_by,
            vec!["sha256:a", "sha256:b"]
        );
    }

    #[test]
    fn missing_or_incompatible_axes_are_not_silently_ranked() {
        let mut incompatible = candidate("sha256:b", Some(5.0), Some(1.0));
        incompatible.axes[0].unit = "EUR".into();
        let receipt =
            pareto_rank(&[candidate("sha256:a", Some(10.0), None), incompatible]).unwrap();
        assert!(
            receipt
                .verdicts
                .iter()
                .all(|verdict| verdict.status == ParetoStatus::Incomparable)
        );
    }

    #[test]
    fn hard_gate_statuses_are_not_overridden_by_objectives() {
        let mut rejected = candidate("sha256:rejected", Some(0.0), Some(0.0));
        rejected.eligibility = AuditEligibility::Rejected;
        let mut review = candidate("sha256:review", Some(1.0), Some(1.0));
        review.eligibility = AuditEligibility::NeedsReview;
        let receipt = pareto_rank(&[rejected, review]).unwrap();
        assert_eq!(receipt.verdicts[0].status, ParetoStatus::Rejected);
        assert_eq!(receipt.verdicts[1].status, ParetoStatus::NeedsReview);
    }
}
