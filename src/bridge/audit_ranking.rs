//! Conservative multi-objective ranking for already-audited routes.
//!
//! This is not a search heuristic. It ranks only after audit and keeps hard
//! rejection, needs-review, missing data, units, and process boundaries
//! explicit. Unknown values are never silently converted to zero.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

pub const AUDIT_RANKING_SCHEMA_VERSION: u32 = 1;
pub const WEIGHTED_RANKING_SCHEMA_VERSION: u32 = 1;

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

/// A fixed, auditable normalization contract for one weighted objective.  It
/// deliberately owns its unit, basis and bounds: rankings never derive their
/// scale from the set of routes currently being compared.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeightedAxis {
    pub key: String,
    pub direction: ObjectiveDirection,
    pub unit: String,
    pub basis: String,
    pub minimum: f64,
    pub maximum: f64,
    pub weight: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeightedProfile {
    pub schema_version: u32,
    pub profile_id: String,
    pub axes: Vec<WeightedAxis>,
}

/// File/CLI input for an auditable weighted ranking. The profile supplies
/// fixed normalization bounds; candidates remain separate from search output.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct WeightedRankingInput {
    pub profile: WeightedProfile,
    pub candidates: Vec<RankingCandidate>,
    #[serde(default = "default_sensitivity_fraction")]
    pub sensitivity_fraction: f64,
}

fn default_sensitivity_fraction() -> f64 {
    0.1
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WeightedVerdict {
    pub route_id: String,
    pub rank: usize,
    pub score: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WeightSensitivityScenario {
    pub axis_key: String,
    pub multiplier: f64,
    pub top_route_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WeightedRankingReceipt {
    pub schema_version: u32,
    pub profile_id: String,
    pub profile_sha256: String,
    pub verdicts: Vec<WeightedVerdict>,
    pub sensitivity_fraction: f64,
    pub sensitivity: Vec<WeightSensitivityScenario>,
}

/// Rank only fully comparable, eligible routes with a fixed profile.  Missing
/// data and an incompatible contract are errors rather than a reason to
/// silently renormalize the remaining weights.
pub fn weighted_rank_with_sensitivity(
    candidates: &[RankingCandidate],
    profile: &WeightedProfile,
    sensitivity_fraction: f64,
) -> anyhow::Result<WeightedRankingReceipt> {
    validate_weighted_profile(profile)?;
    if !sensitivity_fraction.is_finite() || !(0.0..1.0).contains(&sensitivity_fraction) {
        anyhow::bail!("weight sensitivity fraction must be finite and in [0, 1)");
    }
    let baseline = weighted_verdicts(candidates, profile, &profile.axes)?;
    let mut sensitivity = Vec::with_capacity(profile.axes.len() * 2);
    for (index, axis) in profile.axes.iter().enumerate() {
        for multiplier in [1.0 - sensitivity_fraction, 1.0 + sensitivity_fraction] {
            let mut varied = profile.axes.clone();
            varied[index].weight *= multiplier;
            normalize_weights(&mut varied)?;
            let verdicts = weighted_verdicts(candidates, profile, &varied)?;
            let top_route_ids = verdicts
                .iter()
                .take_while(|entry| entry.rank == 1)
                .map(|entry| entry.route_id.clone())
                .collect();
            sensitivity.push(WeightSensitivityScenario {
                axis_key: axis.key.clone(),
                multiplier,
                top_route_ids,
            });
        }
    }
    let profile_sha256 = crate::mcp::audit_receipt::sha256_value(&serde_json::to_value(profile)?);
    Ok(WeightedRankingReceipt {
        schema_version: WEIGHTED_RANKING_SCHEMA_VERSION,
        profile_id: profile.profile_id.clone(),
        profile_sha256,
        verdicts: baseline,
        sensitivity_fraction,
        sensitivity,
    })
}

fn validate_weighted_profile(profile: &WeightedProfile) -> anyhow::Result<()> {
    if profile.schema_version != WEIGHTED_RANKING_SCHEMA_VERSION
        || profile.profile_id.trim().is_empty()
    {
        anyhow::bail!(
            "weighted ranking profile has unsupported schema_version or empty profile_id"
        );
    }
    let mut keys = BTreeSet::new();
    for axis in &profile.axes {
        if axis.key.trim().is_empty()
            || axis.unit.trim().is_empty()
            || axis.basis.trim().is_empty()
            || !axis.minimum.is_finite()
            || !axis.maximum.is_finite()
            || !axis.weight.is_finite()
            || axis.minimum >= axis.maximum
            || axis.weight <= 0.0
            || !keys.insert(&axis.key)
        {
            anyhow::bail!(
                "weighted ranking profile axes must be unique, finite, and positively weighted"
            );
        }
    }
    if profile.axes.is_empty() {
        anyhow::bail!("weighted ranking profile requires at least one axis");
    }
    let sum: f64 = profile.axes.iter().map(|axis| axis.weight).sum();
    if (sum - 1.0).abs() > 1e-12 {
        anyhow::bail!("weighted ranking profile weights must sum to 1");
    }
    Ok(())
}

fn weighted_verdicts(
    candidates: &[RankingCandidate],
    profile: &WeightedProfile,
    weights: &[WeightedAxis],
) -> anyhow::Result<Vec<WeightedVerdict>> {
    let mut scored = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        if candidate.eligibility != AuditEligibility::Eligible || !has_complete_axes(candidate) {
            anyhow::bail!(
                "weighted ranking requires every candidate to be eligible with complete axes"
            );
        }
        if candidate.axes.len() != profile.axes.len() {
            anyhow::bail!("weighted ranking candidate does not match profile axis count");
        }
        let candidate_axes = candidate
            .axes
            .iter()
            .map(|axis| (&axis.key, axis))
            .collect::<BTreeMap<_, _>>();
        let mut score = 0.0;
        for weight in weights {
            let axis = candidate_axes.get(&weight.key).ok_or_else(|| {
                anyhow::anyhow!("weighted ranking candidate is missing profile axis")
            })?;
            if axis.direction != weight.direction
                || axis.unit != weight.unit
                || axis.basis != weight.basis
            {
                anyhow::bail!("weighted ranking candidate axis contract differs from profile");
            }
            let value = axis.value.expect("complete axes checked above");
            if value < weight.minimum || value > weight.maximum {
                anyhow::bail!("weighted ranking value falls outside profile normalization range");
            }
            let span = weight.maximum - weight.minimum;
            let normalized = match weight.direction {
                ObjectiveDirection::Minimize => (weight.maximum - value) / span,
                ObjectiveDirection::Maximize => (value - weight.minimum) / span,
            };
            score += weight.weight * normalized;
        }
        scored.push((candidate.route_id.clone(), score));
    }
    scored.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    let mut prior_score = None;
    let mut rank = 0;
    Ok(scored
        .into_iter()
        .enumerate()
        .map(|(index, (route_id, score))| {
            if prior_score != Some(score) {
                rank = index + 1;
                prior_score = Some(score);
            }
            WeightedVerdict {
                route_id,
                rank,
                score,
            }
        })
        .collect())
}

fn normalize_weights(axes: &mut [WeightedAxis]) -> anyhow::Result<()> {
    let sum: f64 = axes.iter().map(|axis| axis.weight).sum();
    if !sum.is_finite() || sum <= 0.0 {
        anyhow::bail!("varied weighted ranking profile has invalid total weight");
    }
    for axis in axes {
        axis.weight /= sum;
    }
    Ok(())
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

    fn profile() -> WeightedProfile {
        WeightedProfile {
            schema_version: 1,
            profile_id: "procurement-sustainability-v1".into(),
            axes: vec![
                WeightedAxis {
                    key: "price".into(),
                    direction: ObjectiveDirection::Minimize,
                    unit: "USD".into(),
                    basis: "vendor policy v1".into(),
                    minimum: 0.0,
                    maximum: 100.0,
                    weight: 0.5,
                },
                WeightedAxis {
                    key: "pmi".into(),
                    direction: ObjectiveDirection::Minimize,
                    unit: "kg/kg".into(),
                    basis: "batch boundary v1".into(),
                    minimum: 0.0,
                    maximum: 100.0,
                    weight: 0.5,
                },
            ],
        }
    }

    #[test]
    fn weighted_profile_is_fixed_and_records_plus_minus_ten_percent_sensitivity() {
        let receipt = weighted_rank_with_sensitivity(
            &[
                candidate("sha256:a", Some(10.0), Some(30.0)),
                candidate("sha256:b", Some(20.0), Some(10.0)),
            ],
            &profile(),
            0.1,
        )
        .unwrap();
        assert_eq!(receipt.verdicts.len(), 2);
        assert_eq!(receipt.sensitivity.len(), 4);
        assert!(receipt.profile_sha256.starts_with("sha256:"));
    }

    #[test]
    fn weighted_profile_refuses_missing_values_or_dynamic_normalization() {
        let error = weighted_rank_with_sensitivity(
            &[candidate("sha256:a", Some(10.0), None)],
            &profile(),
            0.1,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("complete axes"));
        let mut bad = profile();
        bad.axes[0].weight = 0.7;
        assert!(
            weighted_rank_with_sensitivity(
                &[candidate("sha256:a", Some(10.0), Some(20.0))],
                &bad,
                0.1
            )
            .is_err()
        );
    }
}
