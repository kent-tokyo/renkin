//! Provenance-only receipt for externally produced mechanistic evidence.
//!
//! RENKIN does not run quantum chemistry here. This type prevents values from
//! different physical quantities or missing calculation conditions being
//! silently treated as one comparable route score.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::bridge::audit_ranking::{ObjectiveDirection, RankingAxis};

pub const MECHANISTIC_EVIDENCE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MechanisticQuantity {
    ElectronicBarrier,
    EnthalpyActivation,
    GibbsActivation,
    HomoLumoGap,
    FukuiIndex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceOrigin {
    Reported,
    Predicted,
    Computed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalculationContext {
    pub method: String,
    pub functional: Option<String>,
    pub basis_set: Option<String>,
    pub solvent_model: Option<String>,
    pub temperature_kelvin: Option<String>,
    pub charge: Option<i32>,
    pub multiplicity: Option<u32>,
    pub geometry_sha256: Option<String>,
    pub transition_state_sha256: Option<String>,
    pub convergence_checked: Option<bool>,
    pub frequency_checked: Option<bool>,
    pub irc_checked: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MechanisticEvidenceReceipt {
    pub schema_version: u32,
    pub route_id: String,
    pub step_index: usize,
    pub quantity: MechanisticQuantity,
    pub value: f64,
    pub unit: String,
    pub origin: EvidenceOrigin,
    pub source_sha256: String,
    pub reaction_mapping_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calculation: Option<CalculationContext>,
}

/// The explicit contract under which mechanistic evidence may become a
/// post-audit ranking axis.  It names one reaction step and never aggregates
/// barriers or orbital values across a route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MechanisticAxisSpec {
    pub key: String,
    pub quantity: MechanisticQuantity,
    pub unit: String,
    pub step_index: usize,
    pub direction: ObjectiveDirection,
    /// Caller-defined scientific comparison basis (for example, a protocol
    /// revision); computed records additionally require identical contexts.
    pub basis: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum MechanisticEvidenceError {
    UnsupportedSchema(u32),
    MissingIdentity,
    InvalidValue,
    InvalidUnit,
    InvalidHash(&'static str),
    MissingCalculationContext,
    IncompleteComputedContext,
    InvalidAxisSpec,
    IncomparableForRanking,
}

impl fmt::Display for MechanisticEvidenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchema(version) => write!(
                formatter,
                "unsupported mechanistic evidence schema_version {version}"
            ),
            Self::MissingIdentity => write!(
                formatter,
                "mechanistic evidence route_id, source_sha256, and reaction_mapping_sha256 are required"
            ),
            Self::InvalidValue => write!(formatter, "mechanistic evidence value must be finite"),
            Self::InvalidUnit => write!(
                formatter,
                "mechanistic evidence unit is incompatible with its physical quantity"
            ),
            Self::InvalidHash(field) => write!(
                formatter,
                "mechanistic evidence {field} must be a sha256: prefixed 64-hex digest"
            ),
            Self::MissingCalculationContext => write!(
                formatter,
                "computed mechanistic evidence requires calculation context"
            ),
            Self::IncompleteComputedContext => write!(
                formatter,
                "computed mechanistic evidence requires method, state, and geometry provenance"
            ),
            Self::InvalidAxisSpec => write!(
                formatter,
                "mechanistic ranking axis specification is invalid"
            ),
            Self::IncomparableForRanking => write!(
                formatter,
                "mechanistic evidence is not comparable under one calculation contract"
            ),
        }
    }
}

impl std::error::Error for MechanisticEvidenceError {}

impl MechanisticEvidenceReceipt {
    pub fn validate(&self) -> Result<(), MechanisticEvidenceError> {
        if self.schema_version != MECHANISTIC_EVIDENCE_SCHEMA_VERSION {
            return Err(MechanisticEvidenceError::UnsupportedSchema(
                self.schema_version,
            ));
        }
        if self.route_id.trim().is_empty()
            || self.source_sha256.trim().is_empty()
            || self.reaction_mapping_sha256.trim().is_empty()
        {
            return Err(MechanisticEvidenceError::MissingIdentity);
        }
        validate_hash(&self.source_sha256, "source_sha256")?;
        validate_hash(&self.reaction_mapping_sha256, "reaction_mapping_sha256")?;
        if !self.value.is_finite() {
            return Err(MechanisticEvidenceError::InvalidValue);
        }
        if !valid_unit(self.quantity, &self.unit) {
            return Err(MechanisticEvidenceError::InvalidUnit);
        }
        match (self.origin, &self.calculation) {
            (EvidenceOrigin::Computed, Some(context)) => validate_computed_context(context)?,
            (EvidenceOrigin::Computed, None) => {
                return Err(MechanisticEvidenceError::MissingCalculationContext);
            }
            (_, Some(context)) => validate_optional_context(context)?,
            (_, None) => {}
        }
        Ok(())
    }
}

/// Project one exact, comparable measurement per route into ranking axes.
/// The projection fails rather than averaging different steps, units, origins,
/// or computational contexts.  It is intentionally post-audit data shaping;
/// it does not alter search or audit status.
pub fn project_comparable_axis(
    route_ids: &[String],
    evidence: &[MechanisticEvidenceReceipt],
    spec: &MechanisticAxisSpec,
) -> Result<Vec<(String, RankingAxis)>, MechanisticEvidenceError> {
    if spec.key.trim().is_empty()
        || spec.unit.trim().is_empty()
        || spec.basis.trim().is_empty()
        || !valid_unit(spec.quantity, &spec.unit)
    {
        return Err(MechanisticEvidenceError::InvalidAxisSpec);
    }
    let mut projected = Vec::with_capacity(route_ids.len());
    let mut origin = None;
    let mut computed_context = None;
    for route_id in route_ids {
        let matches = evidence
            .iter()
            .filter(|receipt| {
                receipt.route_id == *route_id
                    && receipt.step_index == spec.step_index
                    && receipt.quantity == spec.quantity
                    && receipt.unit == spec.unit
            })
            .collect::<Vec<_>>();
        let [receipt] = matches.as_slice() else {
            return Err(MechanisticEvidenceError::IncomparableForRanking);
        };
        receipt.validate()?;
        if origin
            .replace(receipt.origin)
            .is_some_and(|previous| previous != receipt.origin)
        {
            return Err(MechanisticEvidenceError::IncomparableForRanking);
        }
        if receipt.origin == EvidenceOrigin::Computed {
            let context = receipt
                .calculation
                .as_ref()
                .ok_or(MechanisticEvidenceError::IncomparableForRanking)?;
            let hash = crate::mcp::audit_receipt::sha256_value(
                &serde_json::to_value(context).expect("calculation context serializes"),
            );
            if computed_context
                .replace(hash.clone())
                .is_some_and(|prior| prior != hash)
            {
                return Err(MechanisticEvidenceError::IncomparableForRanking);
            }
        }
        projected.push((
            route_id.clone(),
            RankingAxis {
                key: spec.key.clone(),
                direction: spec.direction,
                value: Some(receipt.value),
                unit: spec.unit.clone(),
                basis: match &computed_context {
                    Some(hash) => format!("{}; calculation_context={hash}", spec.basis),
                    None => spec.basis.clone(),
                },
            },
        ));
    }
    Ok(projected)
}

fn valid_unit(quantity: MechanisticQuantity, unit: &str) -> bool {
    match quantity {
        MechanisticQuantity::ElectronicBarrier
        | MechanisticQuantity::EnthalpyActivation
        | MechanisticQuantity::GibbsActivation => matches!(unit, "kJ/mol" | "kcal/mol"),
        MechanisticQuantity::HomoLumoGap => matches!(unit, "eV" | "hartree"),
        MechanisticQuantity::FukuiIndex => matches!(unit, "1" | "dimensionless"),
    }
}

fn validate_computed_context(context: &CalculationContext) -> Result<(), MechanisticEvidenceError> {
    validate_optional_context(context)?;
    if context.method.trim().is_empty()
        || context.charge.is_none()
        || context.multiplicity.is_none()
        || context.geometry_sha256.is_none()
    {
        return Err(MechanisticEvidenceError::IncompleteComputedContext);
    }
    Ok(())
}

fn validate_optional_context(context: &CalculationContext) -> Result<(), MechanisticEvidenceError> {
    for hash in [
        context.geometry_sha256.as_deref(),
        context.transition_state_sha256.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        validate_hash(hash, "calculation geometry")?;
    }
    if context
        .temperature_kelvin
        .as_deref()
        .is_some_and(|temperature| temperature.trim().is_empty())
    {
        return Err(MechanisticEvidenceError::IncompleteComputedContext);
    }
    Ok(())
}

fn validate_hash(value: &str, field: &'static str) -> Result<(), MechanisticEvidenceError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(MechanisticEvidenceError::InvalidHash(field));
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(MechanisticEvidenceError::InvalidHash(field));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn computed() -> MechanisticEvidenceReceipt {
        MechanisticEvidenceReceipt {
            schema_version: MECHANISTIC_EVIDENCE_SCHEMA_VERSION,
            route_id: "sha256:route".into(),
            step_index: 0,
            quantity: MechanisticQuantity::GibbsActivation,
            value: 42.0,
            unit: "kJ/mol".into(),
            origin: EvidenceOrigin::Computed,
            source_sha256: HASH.into(),
            reaction_mapping_sha256: HASH.into(),
            calculation: Some(CalculationContext {
                method: "DFT".into(),
                functional: Some("PBE0".into()),
                basis_set: Some("def2-SVP".into()),
                solvent_model: Some("water".into()),
                temperature_kelvin: Some("298.15".into()),
                charge: Some(0),
                multiplicity: Some(1),
                geometry_sha256: Some(HASH.into()),
                transition_state_sha256: Some(HASH.into()),
                convergence_checked: Some(true),
                frequency_checked: Some(true),
                irc_checked: Some(true),
            }),
        }
    }

    #[test]
    fn validates_computed_context_without_ranking_it() {
        computed().validate().unwrap();
    }

    #[test]
    fn rejects_missing_context_and_mixed_quantity_unit() {
        let mut evidence = computed();
        evidence.calculation = None;
        assert_eq!(
            evidence.validate(),
            Err(MechanisticEvidenceError::MissingCalculationContext)
        );

        let mut evidence = computed();
        evidence.unit = "eV".into();
        assert_eq!(
            evidence.validate(),
            Err(MechanisticEvidenceError::InvalidUnit)
        );
    }

    #[test]
    fn projects_only_same_step_same_unit_same_calculation_context() {
        let first = computed();
        let mut second = computed();
        second.route_id = "sha256:route-2".into();
        let spec = MechanisticAxisSpec {
            key: "delta_g_dagger".into(),
            quantity: MechanisticQuantity::GibbsActivation,
            unit: "kJ/mol".into(),
            step_index: 0,
            direction: ObjectiveDirection::Minimize,
            basis: "PBE0 protocol v1".into(),
        };
        let axes = project_comparable_axis(
            &["sha256:route".into(), "sha256:route-2".into()],
            &[first, second],
            &spec,
        )
        .unwrap();
        assert_eq!(axes.len(), 2);
        assert!(axes[0].1.basis.contains("calculation_context=sha256:"));
    }

    #[test]
    fn rejects_cross_context_or_missing_route_evidence() {
        let first = computed();
        let mut second = computed();
        second.route_id = "sha256:route-2".into();
        second.calculation.as_mut().unwrap().functional = Some("B3LYP".into());
        let spec = MechanisticAxisSpec {
            key: "delta_g_dagger".into(),
            quantity: MechanisticQuantity::GibbsActivation,
            unit: "kJ/mol".into(),
            step_index: 0,
            direction: ObjectiveDirection::Minimize,
            basis: "protocol".into(),
        };
        assert_eq!(
            project_comparable_axis(
                &["sha256:route".into(), "sha256:route-2".into()],
                &[first, second],
                &spec
            ),
            Err(MechanisticEvidenceError::IncomparableForRanking)
        );
    }
}
