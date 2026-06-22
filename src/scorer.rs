/// Phase B: ONNX-based template relevance scoring.
///
/// Given a target molecule, predicts the probability that each template
/// in the rule set is applicable. Used in beam search to prefer high-probability
/// templates before attempting SMARTS matching.
///
/// Only compiled when the `nn-scoring` feature is enabled (CLI/Python bindings).
/// WASM builds always use frequency-only scoring (Phase A).
///
/// Uses tract-onnx (Pure Rust, no C/C++ dependency) for inference.
/// `TypedSimplePlan` is `Send + Sync`, so no `Mutex` is needed.
#[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
pub mod nn {
    use std::sync::Arc;

    use anyhow::{Context, Result};
    use chematic::fp::{EcfpConfig, ecfp};
    use tract_onnx::prelude::*;

    use crate::chem_env::{Molecule, RetroRule, mol_from_smiles};

    /// ECFP4 fingerprint: radius=2, 2048 bits (standard for template relevance).
    const ECFP_CONFIG: EcfpConfig = EcfpConfig {
        radius: 2,
        nbits: 2048,
        use_chirality: false,
        use_double_fold: false,
    };

    /// Template relevance scorer backed by an ONNX model (Pure Rust inference).
    ///
    /// The model takes a 2048-bit Morgan fingerprint and outputs logits over
    /// the file templates. Trained by `scripts/train_template_scorer.py`.
    ///
    /// `TypedSimplePlan` implements `Send + Sync`, so the model can be shared
    /// via `Arc<TemplateScorer>` without a `Mutex`. `run` takes `&Arc<Self>`
    /// and creates per-call execution state internally.
    pub struct TemplateScorer {
        model: Arc<TypedSimplePlan>,
        /// Number of top file templates to retain per molecule.
        pub top_k: usize,
        /// Number of rules at the start of the `rules` slice that are hand-crafted
        /// (default_rules) and always tried regardless of scorer output.
        pub rules_offset: usize,
    }

    impl TemplateScorer {
        /// Load a scorer from an ONNX model file.
        ///
        /// `rules_offset` is the count of default (hand-crafted) rules prepended
        /// before file templates in the rules slice. These are always included;
        /// the scorer pre-filters only the file templates.
        pub fn from_path(path: &str, top_k: usize, rules_offset: usize) -> Result<Self> {
            let model = tract_onnx::onnx()
                .model_for_path(path)
                .with_context(|| format!("failed to load ONNX model from {path}"))?
                .with_input_fact(
                    0,
                    InferenceFact::dt_shape(f32::datum_type(), [1usize, 2048usize]),
                )?
                .into_optimized()
                .context("failed to optimize ONNX model")?
                .into_runnable()
                .context("failed to create runnable ONNX plan")?;
            Ok(Self { model, top_k, rules_offset })
        }

        /// Compute Morgan ECFP4 fingerprint as a flat Vec<f32> of length 2048.
        fn fingerprint(mol: &Molecule) -> Vec<f32> {
            let bv = ecfp(mol, &ECFP_CONFIG);
            (0..2048).map(|i| if bv.get(i) { 1.0_f32 } else { 0.0_f32 }).collect()
        }

        /// Return indices into `rules` (length `n_rules`) of the rules to try.
        ///
        /// Always includes [0, rules_offset) (the hand-crafted default rules).
        /// For [rules_offset, n_rules) (file templates), keeps only the top-K
        /// by predicted relevance. Falls back to all rules if inference fails.
        pub fn top_k_indices(&self, target_smiles: &str, n_rules: usize) -> Vec<usize> {
            let offset = self.rules_offset.min(n_rules);
            let n_file = n_rules - offset;
            let fallback = || (0..n_rules).collect::<Vec<_>>();

            if n_file == 0 {
                return fallback();
            }

            let Ok(mol) = mol_from_smiles(target_smiles) else {
                return fallback();
            };

            let bits = Self::fingerprint(&mol);

            // Build [1, 2048] input tensor and convert to TValue.
            let arr = match tract_ndarray::Array2::<f32>::from_shape_vec((1, 2048), bits) {
                Ok(a) => a,
                Err(_) => return fallback(),
            };
            let input: TVec<TValue> = tvec![arr.into_tvalue()];

            // Run inference — `run` takes `&Arc<Self>` internally, no lock needed.
            let outputs = match self.model.run(input) {
                Ok(o) => o,
                Err(_) => return fallback(),
            };

            // Extract logits; TValue derefs to Tensor via Deref.
            let scores: Vec<f32> = match outputs[0].to_plain_array_view::<f32>() {
                Ok(v) => v.iter().copied().collect(),
                Err(_) => return fallback(),
            };

            // Keep top-K file templates by logit (descending).
            let k = self.top_k.min(n_file).min(scores.len());
            let mut file_indices: Vec<usize> = (0..scores.len().min(n_file)).collect();
            file_indices.sort_by(|&a, &b| {
                scores[b].partial_cmp(&scores[a]).unwrap_or(std::cmp::Ordering::Equal)
            });
            file_indices.truncate(k);

            // Prepend hand-crafted rule indices, then offset file template indices.
            let mut result: Vec<usize> = (0..offset).collect();
            result.extend(file_indices.iter().map(|&i| offset + i));
            result
        }

        /// Filter and reorder `rules` by predicted relevance for `target_smiles`.
        pub fn filter_rules<'a>(
            &self,
            rules: &'a [RetroRule],
            target_smiles: &str,
        ) -> Vec<&'a RetroRule> {
            self.top_k_indices(target_smiles, rules.len())
                .into_iter()
                .filter_map(|i| rules.get(i))
                .collect()
        }
    }
}

/// Stub for non-nn-scoring builds — scorer module is empty.
#[cfg(not(all(not(target_arch = "wasm32"), feature = "nn-scoring")))]
pub mod nn {}
