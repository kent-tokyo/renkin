//! Versioned, local process-mass receipts for audit-route.
//!
//! This module deliberately accepts measurements supplied by the caller. A
//! retrosynthetic route lists neither the actual solvent/work-up quantities nor
//! process yield, so it must not manufacture PMI or E-factor from SMILES.

use std::{collections::BTreeSet, fmt};

use serde::{Deserialize, Serialize};

pub const ROUTE_METRICS_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MassCategory {
    StartingMaterial,
    Reagent,
    Solvent,
    Catalyst,
    ProcessingAid,
    Water,
    Workup,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MassUnit {
    Gram,
    Kilogram,
}

impl MassUnit {
    fn grams(self, value: f64) -> f64 {
        match self {
            Self::Gram => value,
            Self::Kilogram => value * 1000.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MassAmount {
    pub value: f64,
    pub unit: MassUnit,
}

impl MassAmount {
    fn validated_grams(&self, field: &str) -> Result<f64, MetricError> {
        if !self.value.is_finite() || self.value <= 0.0 {
            return Err(MetricError::InvalidAmount {
                field: field.to_owned(),
            });
        }
        Ok(self.unit.grams(self.value))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcessMassInput {
    pub category: MassCategory,
    pub amount: MassAmount,
    /// Local, caller-defined identifier. It is not assumed to be a vendor,
    /// chemical identity, or public source.
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessBoundary {
    pub description: String,
    pub includes_water: bool,
    pub includes_workup: bool,
    pub recycling_policy: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricScope {
    Route,
    Step { step_id: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReportedMetric {
    pub value: f64,
    pub source_reference: String,
    pub method: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcessMassLedger {
    pub schema_version: u32,
    pub route_id: String,
    pub scope: MetricScope,
    pub boundary: ProcessBoundary,
    pub product: MassAmount,
    pub inputs: Vec<ProcessMassInput>,
    /// Categories whose mass must be supplied to recompute PMI. Empty means
    /// the ledger is incomplete rather than a zero-input process.
    pub required_categories: Vec<MassCategory>,
    /// Required for a recomputed E-factor. It is not inferred as `PMI - 1`.
    pub waste: Option<MassAmount>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reported_pmi: Option<ReportedMetric>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reported_e_factor: Option<ReportedMetric>,
    pub method: String,
    pub source_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricStatus {
    Evaluated,
    NotEvaluable,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MetricValue {
    pub status: MetricStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Coverage {
    pub required_categories: Vec<MassCategory>,
    pub present_categories: Vec<MassCategory>,
    pub missing_categories: Vec<MassCategory>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RouteMetricsReceipt {
    pub schema_version: u32,
    pub route_id: String,
    pub scope: MetricScope,
    pub boundary: ProcessBoundary,
    pub coverage: Coverage,
    pub process_mass_intensity: MetricValue,
    pub e_factor: MetricValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reported_pmi: Option<ReportedMetric>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reported_e_factor: Option<ReportedMetric>,
    pub method: String,
    pub source_sha256: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum MetricError {
    UnsupportedSchema(u32),
    MissingRouteId,
    MissingProvenance,
    MissingBoundary,
    InvalidAmount { field: String },
    InvalidRequiredCategories,
    BoundaryCategoryNotRequired { category: MassCategory },
    MissingInputLabel,
}

impl fmt::Display for MetricError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchema(version) => {
                write!(
                    formatter,
                    "unsupported route metrics schema_version {version}"
                )
            }
            Self::MissingRouteId => write!(formatter, "route metrics route_id is required"),
            Self::MissingProvenance => {
                write!(
                    formatter,
                    "route metrics method and source_sha256 are required"
                )
            }
            Self::MissingBoundary => {
                write!(
                    formatter,
                    "process boundary description and recycling_policy are required"
                )
            }
            Self::InvalidAmount { field } => write!(
                formatter,
                "process metric {field} must be finite and greater than zero"
            ),
            Self::InvalidRequiredCategories => write!(
                formatter,
                "process ledger required_categories must be non-empty and unique"
            ),
            Self::BoundaryCategoryNotRequired { category } => write!(
                formatter,
                "process boundary includes {category:?}, but it is not a required category"
            ),
            Self::MissingInputLabel => write!(formatter, "process mass input label is required"),
        }
    }
}

impl std::error::Error for MetricError {}

impl ProcessMassLedger {
    /// Validates supplied data and emits values only when every required mass
    /// category is present. `NotEvaluable` is a normal scientific result, not
    /// an error: it records that the ledger lacks the required evidence.
    pub fn evaluate(&self) -> Result<RouteMetricsReceipt, MetricError> {
        self.validate()?;
        let product_grams = self.product.validated_grams("product")?;
        let present = self
            .inputs
            .iter()
            .map(|input| input.category)
            .collect::<BTreeSet<_>>();
        let required = self
            .required_categories
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let missing = required.difference(&present).copied().collect::<Vec<_>>();
        let coverage = Coverage {
            required_categories: required.iter().copied().collect(),
            present_categories: present.iter().copied().collect(),
            missing_categories: missing.clone(),
        };
        let process_mass_intensity = if missing.is_empty() {
            let total_input_grams = self
                .inputs
                .iter()
                .map(|input| input.amount.validated_grams("input"))
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .sum::<f64>();
            evaluated(total_input_grams / product_grams)
        } else {
            not_evaluable("missing_required_mass_category")
        };
        let e_factor = if !missing.is_empty() {
            not_evaluable("missing_required_mass_category")
        } else if let Some(waste) = &self.waste {
            evaluated(waste.validated_grams("waste")? / product_grams)
        } else {
            not_evaluable("waste_mass_not_provided")
        };
        Ok(RouteMetricsReceipt {
            schema_version: ROUTE_METRICS_SCHEMA_VERSION,
            route_id: self.route_id.clone(),
            scope: self.scope.clone(),
            boundary: self.boundary.clone(),
            coverage,
            process_mass_intensity,
            e_factor,
            reported_pmi: self.reported_pmi.clone(),
            reported_e_factor: self.reported_e_factor.clone(),
            method: self.method.clone(),
            source_sha256: self.source_sha256.clone(),
        })
    }

    fn validate(&self) -> Result<(), MetricError> {
        if self.schema_version != ROUTE_METRICS_SCHEMA_VERSION {
            return Err(MetricError::UnsupportedSchema(self.schema_version));
        }
        if self.route_id.trim().is_empty() {
            return Err(MetricError::MissingRouteId);
        }
        if self.method.trim().is_empty() || self.source_sha256.trim().is_empty() {
            return Err(MetricError::MissingProvenance);
        }
        if self.boundary.description.trim().is_empty()
            || self.boundary.recycling_policy.trim().is_empty()
        {
            return Err(MetricError::MissingBoundary);
        }
        self.product.validated_grams("product")?;
        let required = self
            .required_categories
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        if required.is_empty() || required.len() != self.required_categories.len() {
            return Err(MetricError::InvalidRequiredCategories);
        }
        for (included, category) in [
            (self.boundary.includes_water, MassCategory::Water),
            (self.boundary.includes_workup, MassCategory::Workup),
        ] {
            if included && !required.contains(&category) {
                return Err(MetricError::BoundaryCategoryNotRequired { category });
            }
        }
        for input in &self.inputs {
            if input.label.trim().is_empty() {
                return Err(MetricError::MissingInputLabel);
            }
            input.amount.validated_grams("input")?;
        }
        if let Some(waste) = &self.waste {
            waste.validated_grams("waste")?;
        }
        for metric in [self.reported_pmi.as_ref(), self.reported_e_factor.as_ref()]
            .into_iter()
            .flatten()
        {
            if !metric.value.is_finite()
                || metric.value < 0.0
                || metric.source_reference.trim().is_empty()
                || metric.method.trim().is_empty()
            {
                return Err(MetricError::InvalidAmount {
                    field: "reported_metric".into(),
                });
            }
        }
        Ok(())
    }
}

fn evaluated(value: f64) -> MetricValue {
    MetricValue {
        status: MetricStatus::Evaluated,
        value: Some(value),
        reason_code: None,
    }
}

fn not_evaluable(reason_code: &'static str) -> MetricValue {
    MetricValue {
        status: MetricStatus::NotEvaluable,
        value: None,
        reason_code: Some(reason_code),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn amount(value: f64, unit: MassUnit) -> MassAmount {
        MassAmount { value, unit }
    }

    fn ledger() -> ProcessMassLedger {
        ProcessMassLedger {
            schema_version: ROUTE_METRICS_SCHEMA_VERSION,
            route_id: "sha256:route".into(),
            scope: MetricScope::Route,
            boundary: ProcessBoundary {
                description: "one isolated batch; all listed inputs counted".into(),
                includes_water: true,
                includes_workup: true,
                recycling_policy: "no recovery credited".into(),
            },
            product: amount(100.0, MassUnit::Gram),
            inputs: vec![
                ProcessMassInput {
                    category: MassCategory::StartingMaterial,
                    amount: amount(0.5, MassUnit::Kilogram),
                    label: "starting material".into(),
                },
                ProcessMassInput {
                    category: MassCategory::Solvent,
                    amount: amount(1.0, MassUnit::Kilogram),
                    label: "solvent".into(),
                },
                ProcessMassInput {
                    category: MassCategory::Water,
                    amount: amount(50.0, MassUnit::Gram),
                    label: "water".into(),
                },
                ProcessMassInput {
                    category: MassCategory::Workup,
                    amount: amount(50.0, MassUnit::Gram),
                    label: "workup".into(),
                },
            ],
            required_categories: vec![
                MassCategory::StartingMaterial,
                MassCategory::Solvent,
                MassCategory::Water,
                MassCategory::Workup,
            ],
            waste: Some(amount(1260.0, MassUnit::Gram)),
            reported_pmi: Some(ReportedMetric {
                value: 16.8,
                source_reference: "local procedure page 2".into(),
                method: "reported by source".into(),
            }),
            reported_e_factor: None,
            method: "mass ledger v1".into(),
            source_sha256: "sha256:source".into(),
        }
    }

    #[test]
    fn recomputes_pmi_and_e_factor_without_equating_them() {
        let receipt = ledger().evaluate().unwrap();
        assert_eq!(
            receipt.process_mass_intensity.status,
            MetricStatus::Evaluated
        );
        assert_eq!(receipt.process_mass_intensity.value, Some(16.0));
        assert_eq!(receipt.e_factor.value, Some(12.6));
        assert_eq!(receipt.reported_pmi.as_ref().unwrap().value, 16.8);
    }

    #[test]
    fn missing_required_water_is_not_evaluable_not_zero() {
        let mut input = ledger();
        input
            .inputs
            .retain(|entry| entry.category != MassCategory::Water);
        let receipt = input.evaluate().unwrap();
        assert_eq!(
            receipt.process_mass_intensity.status,
            MetricStatus::NotEvaluable
        );
        assert_eq!(receipt.process_mass_intensity.value, None);
        assert_eq!(
            receipt.e_factor.reason_code,
            Some("missing_required_mass_category")
        );
        assert_eq!(
            receipt.coverage.missing_categories,
            vec![MassCategory::Water]
        );
    }

    #[test]
    fn explicit_waste_is_required_for_e_factor() {
        let mut input = ledger();
        input.waste = None;
        let receipt = input.evaluate().unwrap();
        assert_eq!(receipt.process_mass_intensity.value, Some(16.0));
        assert_eq!(receipt.e_factor.status, MetricStatus::NotEvaluable);
        assert_eq!(
            receipt.e_factor.reason_code,
            Some("waste_mass_not_provided")
        );
    }

    #[test]
    fn rejects_zero_product_and_invalid_boundary_coverage() {
        let mut input = ledger();
        input.product.value = 0.0;
        assert!(matches!(
            input.evaluate(),
            Err(MetricError::InvalidAmount { .. })
        ));

        let mut input = ledger();
        input
            .required_categories
            .retain(|category| *category != MassCategory::Water);
        assert_eq!(
            input.evaluate(),
            Err(MetricError::BoundaryCategoryNotRequired {
                category: MassCategory::Water
            })
        );
    }
}
