//! Reproducibility metadata for optional learned model adapters.
//!
//! A manifest is deliberately independent of the search implementation. It
//! records what a model consumes and emits, so an adapter cannot silently
//! compare scores produced for a different template vocabulary or feature
//! schema.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

/// Adapter boundary that a model implements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    TemplatePolicy,
    RetroGenerator,
    ValueModel,
    CandidateReranker,
}

/// Meaning of a model output. Adapters must convert this into their internal
/// ordering/cost convention explicitly rather than assuming a probability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputSemantics {
    Rank,
    Logit,
    Probability,
    Cost,
}

/// Versioned, hash-pinned metadata for a learned model adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelManifest {
    pub schema_version: u32,
    pub model_id: String,
    pub model_kind: ModelKind,
    pub model_version: String,
    /// Lower-case `sha256:<64 hex digits>` for the model artifact.
    pub model_sha256: String,
    pub input_schema: String,
    pub output_semantics: OutputSemantics,
    /// Required for template-based adapters; absent for direct generators.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_set_sha256: Option<String>,
    /// Stable description of model-template ID conversion, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_id_mapping: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub training_data: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split: Option<String>,
    pub license: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redistribution: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_inference_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_memory_bytes: Option<u64>,
    /// Whether the adapter may abstain when its input is out of distribution.
    pub ood_abstain: bool,
}

impl ModelManifest {
    /// Validate the fields that are load-bearing for reproducible evaluation.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version == 0 {
            bail!("model manifest schema_version must be positive");
        }
        for (field, value) in [
            ("model_id", self.model_id.as_str()),
            ("model_version", self.model_version.as_str()),
            ("input_schema", self.input_schema.as_str()),
            ("license", self.license.as_str()),
        ] {
            if value.trim().is_empty() {
                bail!("model manifest {field} must not be empty");
            }
        }
        validate_sha256("model_sha256", &self.model_sha256)?;
        if let Some(hash) = &self.template_set_sha256 {
            validate_sha256("template_set_sha256", hash)?;
        }
        if matches!(
            self.model_kind,
            ModelKind::TemplatePolicy | ModelKind::CandidateReranker
        ) && self.template_set_sha256.is_none()
        {
            bail!("template-based model manifests require template_set_sha256");
        }
        Ok(())
    }
}

/// Load and validate a manifest from a bounded local JSON file.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_model_manifest(path: impl AsRef<Path>) -> Result<ModelManifest> {
    let text = crate::io_limits::read_bounded_text_path(path, "model manifest")?;
    let manifest: ModelManifest = serde_json::from_str(&text)
        .map_err(|error| anyhow::anyhow!("invalid model manifest JSON: {error}"))?;
    manifest.validate()?;
    Ok(manifest)
}

fn validate_sha256(field: &str, value: &str) -> Result<()> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        bail!("model manifest {field} must use sha256:<64 hex digits>");
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("model manifest {field} must use sha256:<64 hex digits>");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(kind: ModelKind) -> ModelManifest {
        ModelManifest {
            schema_version: 1,
            model_id: "test-model".into(),
            model_kind: kind,
            model_version: "0.1.0".into(),
            model_sha256: format!("sha256:{}", "a".repeat(64)),
            input_schema: "renkin-fingerprint-v1".into(),
            output_semantics: OutputSemantics::Logit,
            template_set_sha256: Some(format!("sha256:{}", "b".repeat(64))),
            template_id_mapping: None,
            training_data: None,
            split: None,
            license: "MIT".into(),
            redistribution: None,
            max_inference_ms: Some(100),
            max_memory_bytes: None,
            ood_abstain: true,
        }
    }

    #[test]
    fn validates_template_policy_manifest() {
        assert!(manifest(ModelKind::TemplatePolicy).validate().is_ok());
    }

    #[test]
    fn rejects_template_model_without_template_hash() {
        let mut value = manifest(ModelKind::TemplatePolicy);
        value.template_set_sha256 = None;
        assert!(value.validate().is_err());
    }

    #[test]
    fn allows_direct_generator_without_template_hash() {
        let mut value = manifest(ModelKind::RetroGenerator);
        value.template_set_sha256 = None;
        assert!(value.validate().is_ok());
    }

    #[test]
    fn rejects_malformed_artifact_hash() {
        let mut value = manifest(ModelKind::ValueModel);
        value.model_sha256 = "sha256:not-a-hash".into();
        assert!(value.validate().is_err());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn serializes_and_loads_valid_manifest() {
        let value = manifest(ModelKind::TemplatePolicy);
        let path = std::env::temp_dir().join(format!(
            "renkin-model-manifest-{}-{}.json",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let loaded = load_model_manifest(&path).unwrap();
        let _ = std::fs::remove_file(path);
        assert_eq!(loaded, value);
    }
}
