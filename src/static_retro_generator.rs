//! Hash-pinned, file-backed `RetroGenerator` reference adapter.
//!
//! This adapter is intended for fixtures and local A/B plumbing. It carries
//! direct precursor proposals without pretending that they are accepted
//! routes; callers still need the normal RENKIN chemistry and stock gates.

use std::collections::HashMap;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;

#[cfg(not(target_arch = "wasm32"))]
use anyhow::{Context, Result, bail};
#[cfg(not(target_arch = "wasm32"))]
use serde::Deserialize;
#[cfg(not(target_arch = "wasm32"))]
use sha2::{Digest, Sha256};

#[cfg(not(target_arch = "wasm32"))]
use crate::model_manifest::{ModelKind, ModelManifest, load_model_manifest};
use crate::retro_generator::{
    RetroGenerationContext, RetroGenerator, RetroGeneratorDecision, RetroProposal,
};
#[cfg(not(target_arch = "wasm32"))]
use crate::retro_generator::{validate_chemistry, validate_decision};

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StaticRetroArtifact {
    schema_version: u32,
    /// Target SMILES -> ordered direct precursor proposals.
    proposals: HashMap<String, Vec<RetroProposal>>,
}

/// Deterministic direct-generator fixture with manifest and artifact hashes.
pub struct StaticRetroGenerator {
    proposals_by_target: HashMap<String, Vec<RetroProposal>>,
}

impl StaticRetroGenerator {
    /// Load and validate a hash-pinned direct-generator artifact.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn from_files(
        manifest_path: impl AsRef<Path>,
        artifact_path: impl AsRef<Path>,
    ) -> Result<(ModelManifest, Arc<Self>)> {
        let manifest = load_model_manifest(manifest_path)?;
        if manifest.model_kind != ModelKind::RetroGenerator {
            bail!("model manifest kind must be retro_generator");
        }
        let bytes = crate::io_limits::read_bounded_bytes_path(&artifact_path, "retro artifact")?;
        let actual_hash = format!("sha256:{}", crate::sha256_hex(Sha256::digest(&bytes)));
        if actual_hash != manifest.model_sha256 {
            bail!(
                "retro artifact SHA-256 mismatch: manifest={}, actual={actual_hash}",
                manifest.model_sha256
            );
        }
        let artifact: StaticRetroArtifact = serde_json::from_slice(&bytes)
            .context("invalid static retro-generator artifact JSON")?;
        if artifact.schema_version == 0 {
            bail!("static retro-generator artifact schema_version must be positive");
        }
        let mut proposals_by_target = HashMap::with_capacity(artifact.proposals.len());
        for (target, proposals) in artifact.proposals {
            let canonical_target = crate::chem_env::canonical_stock_identity_from_smiles(&target)
                .map_err(|error| {
                anyhow::anyhow!("invalid generator target {target:?}: {error}")
            })?;
            let proposals = proposals
                .into_iter()
                .map(|mut proposal| {
                    proposal.target = canonical_target.clone();
                    proposal
                })
                .collect();
            if proposals_by_target
                .insert(canonical_target.clone(), proposals)
                .is_some()
            {
                bail!("duplicate generator target after canonicalization: {canonical_target:?}");
            }
        }
        let generator = Self {
            proposals_by_target,
        };
        let unlimited = RetroGenerationContext::default();
        for (target, proposals) in &generator.proposals_by_target {
            validate_decision(
                target,
                &unlimited,
                &RetroGeneratorDecision {
                    proposals: proposals.clone(),
                    abstained: false,
                },
            )
            .map_err(|error| anyhow::anyhow!("invalid proposals for {target:?}: {error}"))?;
            validate_chemistry(
                target,
                &RetroGeneratorDecision {
                    proposals: proposals.clone(),
                    abstained: false,
                },
            )
            .map_err(|error| anyhow::anyhow!("invalid chemistry for {target:?}: {error}"))?;
        }
        Ok((manifest, Arc::new(generator)))
    }
}

impl RetroGenerator for StaticRetroGenerator {
    fn propose(&self, target: &str, _context: &RetroGenerationContext) -> RetroGeneratorDecision {
        let key = crate::chem_env::canonical_stock_identity_from_smiles(target).ok();
        match key
            .as_deref()
            .and_then(|key| self.proposals_by_target.get(key))
        {
            Some(proposals) => RetroGeneratorDecision {
                proposals: proposals
                    .iter()
                    .cloned()
                    .map(|mut proposal| {
                        // The transport contract compares the proposal target
                        // with the caller's spelling. Keep lookup canonical,
                        // but preserve the request spelling at the boundary.
                        proposal.target = target.to_owned();
                        proposal
                    })
                    .collect(),
                abstained: false,
            },
            None => RetroGeneratorDecision {
                proposals: Vec::new(),
                abstained: true,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retro_generator::GeneratorProvenance;

    #[test]
    fn unknown_target_abstains() {
        let generator = StaticRetroGenerator {
            proposals_by_target: HashMap::new(),
        };
        let decision = generator.propose("CCO", &RetroGenerationContext::default());
        assert!(decision.abstained);
        assert!(decision.proposals.is_empty());
    }

    #[test]
    fn equivalent_target_spelling_reuses_canonical_fixture_key() {
        let proposal = RetroProposal {
            candidate_id: "candidate-1".into(),
            target: "CCO".into(),
            precursors: vec!["CC".into(), "O".into()],
            atom_mapping: None,
            provenance: GeneratorProvenance {
                generator_id: "fixture".into(),
                generator_version: "1".into(),
                artifact_sha256: "sha256:fixture".into(),
                source_rank: 0,
                model_confidence: Some(0.8),
            },
        };
        let key = crate::chem_env::canonical_stock_identity_from_smiles("CCO").unwrap();
        let generator = StaticRetroGenerator {
            proposals_by_target: HashMap::from([(key, vec![proposal])]),
        };
        let decision = generator.propose("C(C)O", &RetroGenerationContext::default());
        assert_eq!(decision.proposals.len(), 1);
        assert_eq!(decision.proposals[0].target, "C(C)O");
    }

    #[test]
    fn fixture_proposal_is_traceable() {
        let proposal = RetroProposal {
            candidate_id: "candidate-1".into(),
            target: "CCO".into(),
            precursors: vec!["CC".into(), "O".into()],
            atom_mapping: None,
            provenance: GeneratorProvenance {
                generator_id: "fixture".into(),
                generator_version: "1".into(),
                artifact_sha256: "sha256:fixture".into(),
                source_rank: 0,
                model_confidence: Some(0.8),
            },
        };
        let generator = StaticRetroGenerator {
            proposals_by_target: HashMap::from([("CCO".into(), vec![proposal])]),
        };
        assert!(
            generator
                .propose_checked("CCO", &RetroGenerationContext::default())
                .is_ok()
        );
    }
}
