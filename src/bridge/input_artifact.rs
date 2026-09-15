//! Local-by-default provenance for structures supplied from non-SMILES input.
//!
//! This is a receipt format, not an OCR client. It never fetches a URL,
//! renders a PDF/SVG, or invokes a recognition model. Raw predictions and
//! locators stay in the caller-supplied local sidecar; the audit report uses a
//! deliberately redacted view.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::chem_env::{mol_from_smiles, to_canonical};

pub const INPUT_ARTIFACT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputSourceKind {
    Image,
    Svg,
    Pdf,
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactTransform {
    pub kind: String,
    pub input_sha256: String,
    pub output_sha256: String,
    pub tool: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OcsrProvenance {
    pub tool: String,
    pub model: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_sha256: Option<String>,
    /// The model's original output. It is deliberately omitted from the
    /// public receipt because it can reveal input data and is not an audit
    /// verdict by itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_prediction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence_semantics: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructureReviewStatus {
    NeedsReview,
    Confirmed,
    Corrected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructureReview {
    pub status: StructureReviewStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reviewer_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Full local receipt. `source_locator` and raw prediction are intentionally
/// available only to the local caller; use [`InputArtifactReceipt::redacted`]
/// for any audit report or interchange export.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputArtifactReceipt {
    pub schema_version: u32,
    pub source_kind: InputSourceKind,
    pub content_sha256: String,
    pub content_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_locator: Option<String>,
    pub transforms: Vec<ArtifactTransform>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ocsr: Option<OcsrProvenance>,
    /// Canonical SMILES if a structure was parsed and normalized. A valid
    /// SMILES does not prove that it represents the source drawing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normalized_smiles: Option<String>,
    pub normalization_changed: bool,
    pub review: StructureReview,
}

/// Safe-by-default public view: it binds content and conversion provenance
/// but omits source locations, raw model output, the normalized structure, and
/// reviewer identity/reason. Hashes alone do not guarantee confidentiality.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RedactedInputArtifactReceipt {
    pub schema_version: u32,
    pub source_kind: InputSourceKind,
    pub content_sha256: String,
    pub content_type: String,
    pub transforms: Vec<ArtifactTransform>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ocsr: Option<RedactedOcsrProvenance>,
    pub normalization_changed: bool,
    pub review_status: StructureReviewStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RedactedOcsrProvenance {
    pub tool: String,
    pub model: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence_semantics: Option<String>,
}

#[derive(Debug)]
pub enum InputArtifactError {
    UnsupportedSchema(u32),
    InvalidHash { field: &'static str },
    MissingContentType,
    MissingTransformField { field: &'static str },
    IncompleteOcsr,
    InvalidConfidence,
    NonCanonicalSmiles,
    NormalizationMismatch,
    MissingCorrectionReason,
    TargetMismatch,
}

impl fmt::Display for InputArtifactError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchema(version) => {
                write!(
                    formatter,
                    "unsupported input artifact schema_version {version}"
                )
            }
            Self::InvalidHash { field } => write!(
                formatter,
                "input artifact {field} must be a sha256: prefixed 64-hex digest"
            ),
            Self::MissingContentType => {
                write!(formatter, "input artifact content_type is required")
            }
            Self::MissingTransformField { field } => {
                write!(formatter, "input artifact transform {field} is required")
            }
            Self::IncompleteOcsr => {
                write!(formatter, "input artifact OCR provenance is incomplete")
            }
            Self::InvalidConfidence => write!(
                formatter,
                "input artifact confidence requires finite value and non-empty semantics"
            ),
            Self::NonCanonicalSmiles => write!(
                formatter,
                "input artifact normalized_smiles is not RENKIN canonical SMILES"
            ),
            Self::NormalizationMismatch => write!(
                formatter,
                "input artifact normalization_changed conflicts with raw prediction"
            ),
            Self::MissingCorrectionReason => {
                write!(
                    formatter,
                    "input artifact review correction requires a reason"
                )
            }
            Self::TargetMismatch => write!(
                formatter,
                "input artifact normalized structure does not match audited route target"
            ),
        }
    }
}

impl std::error::Error for InputArtifactError {}

impl InputArtifactReceipt {
    pub fn validate(&self) -> Result<(), InputArtifactError> {
        if self.schema_version != INPUT_ARTIFACT_SCHEMA_VERSION {
            return Err(InputArtifactError::UnsupportedSchema(self.schema_version));
        }
        validate_hash(&self.content_sha256, "content_sha256")?;
        if self.content_type.trim().is_empty() {
            return Err(InputArtifactError::MissingContentType);
        }
        for transform in &self.transforms {
            if transform.kind.trim().is_empty()
                || transform.tool.trim().is_empty()
                || transform.version.trim().is_empty()
            {
                return Err(InputArtifactError::MissingTransformField { field: "metadata" });
            }
            validate_hash(&transform.input_sha256, "transform.input_sha256")?;
            validate_hash(&transform.output_sha256, "transform.output_sha256")?;
        }
        if let Some(ocsr) = &self.ocsr {
            if ocsr.tool.trim().is_empty()
                || ocsr.model.trim().is_empty()
                || ocsr.version.trim().is_empty()
            {
                return Err(InputArtifactError::IncompleteOcsr);
            }
            if let Some(hash) = &ocsr.model_sha256 {
                validate_hash(hash, "ocsr.model_sha256")?;
            }
            match (ocsr.confidence, ocsr.confidence_semantics.as_deref()) {
                (Some(value), Some(semantics))
                    if value.is_finite() && !semantics.trim().is_empty() => {}
                (None, None) => {}
                _ => return Err(InputArtifactError::InvalidConfidence),
            }
        }
        if let Some(smiles) = &self.normalized_smiles {
            let canonical = to_canonical(
                &mol_from_smiles(smiles).map_err(|_| InputArtifactError::NonCanonicalSmiles)?,
            );
            if canonical != *smiles {
                return Err(InputArtifactError::NonCanonicalSmiles);
            }
            if let Some(raw) = self
                .ocsr
                .as_ref()
                .and_then(|ocsr| ocsr.raw_prediction.as_deref())
            {
                let raw_canonical = to_canonical(
                    &mol_from_smiles(raw).map_err(|_| InputArtifactError::NormalizationMismatch)?,
                );
                if self.normalization_changed != (raw != smiles) || raw_canonical != canonical {
                    return Err(InputArtifactError::NormalizationMismatch);
                }
            }
        } else if self.normalization_changed {
            return Err(InputArtifactError::NormalizationMismatch);
        }
        if self.review.status == StructureReviewStatus::Corrected
            && self
                .review
                .reason
                .as_deref()
                .is_none_or(|reason| reason.trim().is_empty())
        {
            return Err(InputArtifactError::MissingCorrectionReason);
        }
        Ok(())
    }

    pub fn validate_for_target(&self, target: &str) -> Result<(), InputArtifactError> {
        self.validate()?;
        if self.normalized_smiles.as_deref() != Some(target) {
            return Err(InputArtifactError::TargetMismatch);
        }
        Ok(())
    }

    pub fn redacted(&self) -> RedactedInputArtifactReceipt {
        RedactedInputArtifactReceipt {
            schema_version: self.schema_version,
            source_kind: self.source_kind,
            content_sha256: self.content_sha256.clone(),
            content_type: self.content_type.clone(),
            transforms: self.transforms.clone(),
            ocsr: self.ocsr.as_ref().map(|ocsr| RedactedOcsrProvenance {
                tool: ocsr.tool.clone(),
                model: ocsr.model.clone(),
                version: ocsr.version.clone(),
                model_sha256: ocsr.model_sha256.clone(),
                confidence: ocsr.confidence,
                confidence_semantics: ocsr.confidence_semantics.clone(),
            }),
            normalization_changed: self.normalization_changed,
            review_status: self.review.status,
        }
    }
}

fn validate_hash(value: &str, field: &'static str) -> Result<(), InputArtifactError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(InputArtifactError::InvalidHash { field });
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(InputArtifactError::InvalidHash { field });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn receipt() -> InputArtifactReceipt {
        InputArtifactReceipt {
            schema_version: INPUT_ARTIFACT_SCHEMA_VERSION,
            source_kind: InputSourceKind::Image,
            content_sha256: HASH.into(),
            content_type: "image/png".into(),
            source_locator: Some("https://private.example/drawing.png?token=secret".into()),
            transforms: vec![],
            ocsr: Some(OcsrProvenance {
                tool: "local-ocr".into(),
                model: "model".into(),
                version: "1".into(),
                model_sha256: Some(HASH.into()),
                raw_prediction: Some("CCO".into()),
                confidence: Some(0.9),
                confidence_semantics: Some("uncalibrated model score".into()),
            }),
            normalized_smiles: Some(to_canonical(&mol_from_smiles("CCO").unwrap())),
            normalization_changed: true,
            review: StructureReview {
                status: StructureReviewStatus::NeedsReview,
                reviewer_id: Some("private-user".into()),
                reason: None,
            },
        }
    }

    #[test]
    fn validates_local_receipt_and_redacts_sensitive_values() {
        let artifact = receipt();
        artifact.validate().unwrap();
        let public = serde_json::to_string(&artifact.redacted()).unwrap();
        assert!(!public.contains("private.example"));
        assert!(!public.contains("CCO"));
        assert!(!public.contains("private-user"));
        assert!(public.contains("uncalibrated model score"));
    }

    #[test]
    fn rejects_bad_normalization_and_requires_target_binding() {
        let mut artifact = receipt();
        artifact.normalized_smiles = Some("CCO".into());
        assert!(matches!(
            artifact.validate(),
            Err(InputArtifactError::NonCanonicalSmiles)
        ));

        let artifact = receipt();
        assert!(matches!(
            artifact.validate_for_target("C"),
            Err(InputArtifactError::TargetMismatch)
        ));
    }

    #[test]
    fn rejects_confidence_without_semantics_or_invalid_hash() {
        let mut artifact = receipt();
        artifact.ocsr.as_mut().unwrap().confidence_semantics = None;
        assert!(matches!(
            artifact.validate(),
            Err(InputArtifactError::InvalidConfidence)
        ));

        let mut artifact = receipt();
        artifact.content_sha256 = "not-a-hash".into();
        assert!(matches!(
            artifact.validate(),
            Err(InputArtifactError::InvalidHash { .. })
        ));
    }
}
