//! Hash-pinned, file-backed value-model fixture.
//!
//! This is a deterministic reference adapter for calibration and paired A/B
//! plumbing. It does not claim that a table is a trained model, and unknown
//! targets abstain so callers cannot silently turn missing coverage into a
//! heuristic-looking prediction.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::model_manifest::{ModelKind, ModelManifest, OutputSemantics, load_model_manifest};
use crate::search::{ValueEstimate, ValueModel};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StaticValueArtifact {
    schema_version: u32,
    /// Canonical target SMILES -> structured remaining-cost estimate.
    estimates: HashMap<String, ValueEstimate>,
}

/// Deterministic file-backed `ValueModel` fixture with manifest and artifact
/// hash verification.
pub struct StaticValueModel {
    estimates_by_target: HashMap<String, ValueEstimate>,
}

impl StaticValueModel {
    /// Load a value table and verify its model manifest and artifact digest.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_files(
        manifest_path: impl AsRef<Path>,
        artifact_path: impl AsRef<Path>,
    ) -> Result<(ModelManifest, Arc<Self>)> {
        let manifest = load_model_manifest(manifest_path)?;
        if manifest.model_kind != ModelKind::ValueModel {
            bail!("model manifest kind must be value_model");
        }
        if manifest.output_semantics != OutputSemantics::Cost {
            bail!("value model manifest output_semantics must be cost");
        }
        let bytes = crate::io_limits::read_bounded_bytes_path(&artifact_path, "value artifact")?;
        let actual_hash = format!("sha256:{}", crate::sha256_hex(Sha256::digest(&bytes)));
        if actual_hash != manifest.model_sha256 {
            bail!(
                "value artifact SHA-256 mismatch: manifest={}, actual={actual_hash}",
                manifest.model_sha256
            );
        }
        let artifact: StaticValueArtifact =
            serde_json::from_slice(&bytes).context("invalid static value-model artifact JSON")?;
        if artifact.schema_version == 0 {
            bail!("static value-model artifact schema_version must be positive");
        }
        for (target, estimate) in &artifact.estimates {
            if target.trim().is_empty() {
                bail!("static value-model artifact contains an empty target");
            }
            if !estimate.value.is_finite()
                || estimate.value < 0.0
                || estimate.abstained
                || estimate.confidence.is_some_and(|confidence| {
                    !confidence.is_finite() || !(0.0..=1.0).contains(&confidence)
                })
            {
                bail!("invalid value estimate for target {target:?}");
            }
        }
        Ok((
            manifest,
            Arc::new(Self {
                estimates_by_target: artifact.estimates,
            }),
        ))
    }
}

impl ValueModel for StaticValueModel {
    fn estimate(&self, smiles: &str) -> ValueEstimate {
        self.estimates_by_target
            .get(smiles)
            .copied()
            .unwrap_or(ValueEstimate {
                value: 0.0,
                confidence: None,
                abstained: true,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_target_abstains() {
        let model = StaticValueModel {
            estimates_by_target: HashMap::new(),
        };
        assert!(model.estimate("CCO").abstained);
    }

    #[test]
    fn fixture_value_is_returned() {
        let model = StaticValueModel {
            estimates_by_target: HashMap::from([(
                "CCO".into(),
                ValueEstimate {
                    value: 2.0,
                    confidence: Some(0.75),
                    abstained: false,
                },
            )]),
        };
        assert_eq!(model.estimate("CCO").value, 2.0);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn loads_hash_pinned_fixture_and_rejects_invalid_manifest_or_estimate() {
        let dir = std::env::temp_dir();
        let stem = format!("renkin-static-value-{}", std::process::id());
        let artifact_path = dir.join(format!("{stem}.json"));
        let manifest_path = dir.join(format!("{stem}-manifest.json"));
        let artifact = serde_json::json!({
            "schema_version": 1,
            "estimates": {
                "CCO": {"value": 2.0, "confidence": 0.8, "abstained": false}
            }
        });
        let bytes = serde_json::to_vec(&artifact).unwrap();
        let hash = format!("sha256:{}", crate::sha256_hex(Sha256::digest(&bytes)));
        let manifest = serde_json::json!({
            "schema_version": 1,
            "model_id": "fixture",
            "model_kind": "value_model",
            "model_version": "0.1.0",
            "model_sha256": hash,
            "input_schema": "canonical-smiles-v1",
            "output_semantics": "cost",
            "license": "MIT",
            "ood_abstain": true
        });
        std::fs::write(&artifact_path, bytes).unwrap();
        std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let (_, model) = StaticValueModel::from_files(&manifest_path, &artifact_path).unwrap();
        assert_eq!(model.estimate("CCO").value, 2.0);

        let mut wrong_semantics = manifest.clone();
        wrong_semantics["output_semantics"] = serde_json::json!("logit");
        std::fs::write(
            &manifest_path,
            serde_json::to_vec(&wrong_semantics).unwrap(),
        )
        .unwrap();
        assert!(StaticValueModel::from_files(&manifest_path, &artifact_path).is_err());

        let invalid_artifact = serde_json::json!({
            "schema_version": 1,
            "estimates": {
                "CCO": {"value": -1.0, "confidence": 0.8, "abstained": false}
            }
        });
        let invalid_bytes = serde_json::to_vec(&invalid_artifact).unwrap();
        let invalid_hash = format!(
            "sha256:{}",
            crate::sha256_hex(Sha256::digest(&invalid_bytes))
        );
        let mut valid_semantics = manifest;
        valid_semantics["model_sha256"] = serde_json::json!(invalid_hash);
        std::fs::write(&artifact_path, invalid_bytes).unwrap();
        std::fs::write(
            &manifest_path,
            serde_json::to_vec(&valid_semantics).unwrap(),
        )
        .unwrap();
        assert!(StaticValueModel::from_files(&manifest_path, &artifact_path).is_err());

        let _ = std::fs::remove_file(artifact_path);
        let _ = std::fs::remove_file(manifest_path);
    }
}
