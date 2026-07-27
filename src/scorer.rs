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

    /// One file template's raw scorer output. `rule_index` is the absolute
    /// index into the full `rules` slice passed to `score_templates`
    /// (i.e. already offset by `rules_offset`), not an index into the
    /// file-template-only subset. `rank` is this template's position among
    /// all *scored* file templates, sorted by `raw_logit` descending (0 =
    /// highest logit) -- ranks are dense over exactly the scored templates,
    /// never over hand-crafted rules (those are never scored at all).
    #[derive(Debug, Clone, Copy)]
    pub struct TemplateScore {
        pub rule_index: usize,
        pub raw_logit: f32,
        pub rank: usize,
    }

    /// Why `score_templates` did or did not produce scores. Distinct failure
    /// modes are kept distinct on purpose: a training/reranking consumer must
    /// be able to tell "no scorer was configured" apart from "a scorer was
    /// configured but this specific call failed", and must never treat a
    /// failure as if it were a valid (if uninformative) score.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum TemplateScoreStatus {
        Available,
        ModelNotConfigured,
        TargetParseFailed,
        InferenceFailed,
        OutputShapeMismatch,
    }

    /// `scores` is empty unless `status == Available`. On `Available`, it
    /// covers every file template (never truncated to top-K here -- that
    /// truncation is a caller-level concern, see `top_k_indices`), in
    /// arbitrary order (sort by `.rank` if a specific order is needed).
    #[derive(Debug, Clone)]
    pub struct TemplateScoreOutput {
        pub scores: Vec<TemplateScore>,
        pub status: TemplateScoreStatus,
    }

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
                .into_optimized()
                .context("failed to optimize ONNX model")?
                .into_runnable()
                .context("failed to create runnable ONNX plan")?;
            Ok(Self {
                model,
                top_k,
                rules_offset,
            })
        }

        /// Compute Morgan ECFP4 fingerprint as a flat Vec<f32> of length 2048.
        fn fingerprint(mol: &Molecule) -> Vec<f32> {
            let bv = ecfp(mol, &ECFP_CONFIG);
            (0..2048)
                .map(|i| if bv.get(i) { 1.0_f32 } else { 0.0_f32 })
                .collect()
        }

        /// Score every file template ([rules_offset, n_rules) of `rules`) by
        /// predicted relevance to `target_smiles`. Never truncates to top-K
        /// and never falls back to a fabricated score set on failure --
        /// `status` names exactly why `scores` is empty when it is. Hand-
        /// crafted rules ([0, rules_offset)) are never scored (the model is
        /// only trained over the file-template distribution), so they never
        /// appear in `scores` regardless of status.
        pub fn score_templates(&self, target_smiles: &str, n_rules: usize) -> TemplateScoreOutput {
            let empty = |status: TemplateScoreStatus| TemplateScoreOutput {
                scores: Vec::new(),
                status,
            };

            let offset = self.rules_offset.min(n_rules);
            let n_file = n_rules - offset;
            if n_file == 0 {
                return empty(TemplateScoreStatus::ModelNotConfigured);
            }

            let Ok(mol) = mol_from_smiles(target_smiles) else {
                return empty(TemplateScoreStatus::TargetParseFailed);
            };

            let bits = Self::fingerprint(&mol);
            let arr = match tract_ndarray::Array2::<f32>::from_shape_vec((1, 2048), bits) {
                Ok(a) => a,
                Err(_) => return empty(TemplateScoreStatus::OutputShapeMismatch),
            };
            let input: TVec<TValue> = tvec![arr.into_tvalue()];

            let outputs = match self.model.run(input) {
                Ok(o) => o,
                Err(_) => return empty(TemplateScoreStatus::InferenceFailed),
            };

            let raw_scores: Vec<f32> = match outputs[0].to_plain_array_view::<f32>() {
                Ok(v) => v.iter().copied().collect(),
                Err(_) => return empty(TemplateScoreStatus::OutputShapeMismatch),
            };
            if raw_scores.len() != n_file {
                return empty(TemplateScoreStatus::OutputShapeMismatch);
            }

            let mut order: Vec<usize> = (0..raw_scores.len()).collect();
            order.sort_by(|&a, &b| {
                raw_scores[b]
                    .partial_cmp(&raw_scores[a])
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let mut scores = vec![
                TemplateScore {
                    rule_index: 0,
                    raw_logit: 0.0,
                    rank: 0,
                };
                raw_scores.len()
            ];
            for (rank, i) in order.into_iter().enumerate() {
                scores[i] = TemplateScore {
                    rule_index: offset + i,
                    raw_logit: raw_scores[i],
                    rank,
                };
            }

            TemplateScoreOutput {
                scores,
                status: TemplateScoreStatus::Available,
            }
        }

        /// Return indices into `rules` (length `n_rules`) of the rules to try.
        ///
        /// Always includes [0, rules_offset) (the hand-crafted default rules).
        /// For [rules_offset, n_rules) (file templates), keeps only the top-K
        /// by predicted relevance. Falls back to all rules if scoring did not
        /// succeed (see `top_k_indices_with_status` for a variant that makes
        /// the fallback observable rather than silent) -- this exact fallback
        /// behavior is relied on by `search::nn_rank` and must not change.
        pub fn top_k_indices(&self, target_smiles: &str, n_rules: usize) -> Vec<usize> {
            self.top_k_indices_with_status(target_smiles, n_rules).0
        }

        /// Same as `top_k_indices`, but also returns the `TemplateScoreStatus`
        /// that produced the result -- `Available` when top-K filtering
        /// actually happened, anything else when it silently fell back to
        /// every rule. Callers that must not blur "narrowed by the model"
        /// with "fell back because the model/target/inference failed" (e.g.
        /// the candidate reranker's `ScorerConditioned` proposal mode) should
        /// use this instead of `top_k_indices`.
        pub fn top_k_indices_with_status(
            &self,
            target_smiles: &str,
            n_rules: usize,
        ) -> (Vec<usize>, TemplateScoreStatus) {
            let offset = self.rules_offset.min(n_rules);
            let output = self.score_templates(target_smiles, n_rules);
            if output.status != TemplateScoreStatus::Available {
                return ((0..n_rules).collect(), output.status);
            }

            let mut by_rank = output.scores;
            by_rank.sort_by_key(|s| s.rank);
            let k = self.top_k.min(by_rank.len());
            let mut result: Vec<usize> = (0..offset).collect();
            result.extend(by_rank.into_iter().take(k).map(|s| s.rule_index));
            (result, TemplateScoreStatus::Available)
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
