//! Reference adapter for deterministic template-policy A/B evaluation.
//!
//! This is intentionally a static score-table adapter, not a learned model.
//! It lets callers validate the manifest, ordering-only boundary, and paired
//! benchmark plumbing before introducing an ONNX or external model runtime.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crate::model_manifest::{ModelKind, ModelManifest, load_model_manifest};
use crate::search::{TemplateInfo, TemplatePolicy, TemplatePolicyDecision, TemplatePolicyScore};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StaticPolicyArtifact {
    schema_version: u32,
    /// target SMILES → template ID → relevance score
    scores: HashMap<String, HashMap<String, f64>>,
}

/// Deterministic, file-backed `TemplatePolicy` for A/B tests and fixtures.
pub struct StaticTemplatePolicy {
    scores_by_target: HashMap<String, HashMap<String, f64>>,
}

impl StaticTemplatePolicy {
    /// Load a score table and verify it against a `TemplatePolicy` manifest.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_files(
        manifest_path: impl AsRef<Path>,
        artifact_path: impl AsRef<Path>,
    ) -> Result<(ModelManifest, Arc<Self>)> {
        let manifest = load_model_manifest(manifest_path)?;
        if manifest.model_kind != ModelKind::TemplatePolicy {
            bail!("model manifest kind must be template_policy");
        }
        let bytes = crate::io_limits::read_bounded_bytes_path(&artifact_path, "policy artifact")?;
        let actual_hash = format!("sha256:{}", crate::sha256_hex(Sha256::digest(&bytes)));
        if actual_hash != manifest.model_sha256 {
            bail!(
                "policy artifact SHA-256 mismatch: manifest={}, actual={actual_hash}",
                manifest.model_sha256
            );
        }
        let artifact: StaticPolicyArtifact = serde_json::from_slice(&bytes)
            .context("invalid static template policy artifact JSON")?;
        if artifact.schema_version == 0 {
            bail!("static policy artifact schema_version must be positive");
        }
        Ok((
            manifest,
            Arc::new(Self {
                scores_by_target: artifact.scores,
            }),
        ))
    }
}

impl TemplatePolicy for StaticTemplatePolicy {
    fn rank_templates(&self, target: &str, templates: &[TemplateInfo]) -> TemplatePolicyDecision {
        let Some(scores) = self.scores_by_target.get(target) else {
            return TemplatePolicyDecision {
                scores: Vec::new(),
                abstained: true,
            };
        };
        TemplatePolicyDecision {
            scores: templates
                .iter()
                .filter_map(|template| {
                    scores
                        .get(&template.template_id)
                        .copied()
                        .map(|score| TemplatePolicyScore {
                            template_id: template.template_id.clone(),
                            score,
                        })
                })
                .collect(),
            abstained: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_hash_pinned_static_policy_and_abstains_on_unknown_target() {
        let dir = std::env::temp_dir();
        let stem = format!("renkin-static-policy-{}", std::process::id());
        let artifact_path = dir.join(format!("{stem}.json"));
        let manifest_path = dir.join(format!("{stem}-manifest.json"));
        let artifact = serde_json::json!({
            "schema_version": 1,
            "scores": {"CCO": {"rule:a": 2.0, "rule:b": 1.0}}
        });
        let bytes = serde_json::to_vec(&artifact).unwrap();
        let hash = format!("sha256:{}", crate::sha256_hex(Sha256::digest(&bytes)));
        let manifest = serde_json::json!({
            "schema_version": 1,
            "model_id": "fixture",
            "model_kind": "template_policy",
            "model_version": "0.1.0",
            "model_sha256": hash,
            "input_schema": "canonical-smiles-v1",
            "output_semantics": "logit",
            "template_set_sha256": format!("sha256:{}", "b".repeat(64)),
            "license": "MIT",
            "ood_abstain": true
        });
        std::fs::write(&artifact_path, bytes).unwrap();
        std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let (_, policy) = StaticTemplatePolicy::from_files(&manifest_path, &artifact_path).unwrap();
        let rules = vec![
            TemplateInfo {
                template_id: "rule:a".into(),
                template_name: "a".into(),
            },
            TemplateInfo {
                template_id: "rule:b".into(),
                template_name: "b".into(),
            },
        ];
        let decision = policy.rank_templates("CCO", &rules);
        assert!(!decision.abstained);
        assert_eq!(decision.scores.len(), 2);
        assert!(policy.rank_templates("unknown", &rules).abstained);
        let _ = std::fs::remove_file(artifact_path);
        let _ = std::fs::remove_file(manifest_path);
    }
}
