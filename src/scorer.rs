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
        /// The rule set passed to `score_templates` has zero file templates
        /// at `[rules_offset, n_rules)` -- distinct from `ModelNotConfigured`
        /// (which means no model was loaded at all): here a model IS
        /// configured, there is simply nothing in this rule set for it to
        /// score.
        NoFileTemplates,
        TargetParseFailed,
        InferenceFailed,
        OutputShapeMismatch,
        /// The model ran and returned an output of the right shape, but one
        /// or more logits were non-finite (NaN/Inf) -- a corrupted or
        /// numerically unstable model output must never be silently ranked
        /// as if the values were meaningful.
        NonFiniteOutput,
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

    /// ONNX-independent validation and ranking of raw scorer logits --
    /// factored out of `TemplateScorer::score_templates` so this logic is
    /// directly unit-testable without a real ONNX model (see this module's
    /// `#[cfg(test)] mod tests`).
    ///
    /// `rules_offset` is deliberately NOT clamped to `n_rules`: a caller
    /// passing a `rules_offset` larger than the actual rule count is a
    /// config bug, and clamping it would make that bug indistinguishable
    /// from a real zero-file-template rule set (`NoFileTemplates`) --
    /// instead it's its own `OutputShapeMismatch` case.
    ///
    /// Rules: `n_file = n_rules - rules_offset`; `n_file == 0` ->
    /// `NoFileTemplates`; `raw_scores.len() != n_file` ->
    /// `OutputShapeMismatch`; any non-finite value -> `NonFiniteOutput`.
    /// Otherwise, ranks by `raw_logit` descending, tie-breaking exact ties
    /// by ascending absolute `rule_index` (`rules_offset + local index`);
    /// `rank` is a 0-based dense rank over the scored (local) indices. The
    /// returned `Vec<TemplateScore>` is stored by local (file-template)
    /// index, not by rank -- `scores[i]` is the `TemplateScore` for the
    /// `i`-th file template, matching `score_templates`'s pre-existing
    /// contract.
    fn validate_and_rank_logits(
        raw_scores: &[f32],
        rules_offset: usize,
        n_rules: usize,
    ) -> TemplateScoreOutput {
        let empty = |status: TemplateScoreStatus| TemplateScoreOutput {
            scores: Vec::new(),
            status,
        };

        if rules_offset > n_rules {
            return empty(TemplateScoreStatus::OutputShapeMismatch);
        }
        let n_file = n_rules - rules_offset;
        if n_file == 0 {
            return empty(TemplateScoreStatus::NoFileTemplates);
        }
        if raw_scores.len() != n_file {
            return empty(TemplateScoreStatus::OutputShapeMismatch);
        }
        if raw_scores.iter().any(|v| !v.is_finite()) {
            return empty(TemplateScoreStatus::NonFiniteOutput);
        }

        let mut order: Vec<usize> = (0..raw_scores.len()).collect();
        order.sort_by(|&a, &b| {
            raw_scores[b]
                .partial_cmp(&raw_scores[a])
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.cmp(&b))
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
                rule_index: rules_offset + i,
                raw_logit: raw_scores[i],
                rank,
            };
        }

        TemplateScoreOutput {
            scores,
            status: TemplateScoreStatus::Available,
        }
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
        ///
        /// Thin by design: (1) parse the target, (2) fingerprint it,
        /// (3) run ONNX inference, (4) extract the first output as
        /// `Vec<f32>`, (5) hand off to [`validate_and_rank_logits`] for
        /// every ONNX-independent validation/ranking rule -- that split
        /// lets the ranking logic be unit-tested directly, without a real
        /// ONNX model.
        pub fn score_templates(&self, target_smiles: &str, n_rules: usize) -> TemplateScoreOutput {
            let empty = |status: TemplateScoreStatus| TemplateScoreOutput {
                scores: Vec::new(),
                status,
            };

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

            let Some(first_output) = outputs.first() else {
                return empty(TemplateScoreStatus::OutputShapeMismatch);
            };
            let raw_scores: Vec<f32> = match first_output.to_plain_array_view::<f32>() {
                Ok(v) => v.iter().copied().collect(),
                Err(_) => return empty(TemplateScoreStatus::OutputShapeMismatch),
            };

            validate_and_rank_logits(&raw_scores, self.rules_offset, n_rules)
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

    #[cfg(test)]
    mod tests {
        use super::*;

        fn score_at(output: &TemplateScoreOutput, rule_index: usize) -> TemplateScore {
            output
                .scores
                .iter()
                .find(|s| s.rule_index == rule_index)
                .copied()
                .unwrap_or_else(|| panic!("no TemplateScore for rule_index {rule_index}"))
        }

        #[test]
        fn zero_file_templates() {
            let out = validate_and_rank_logits(&[], 5, 5);
            assert_eq!(out.status, TemplateScoreStatus::NoFileTemplates);
            assert!(out.scores.is_empty());
        }

        #[test]
        fn rules_offset_greater_than_n_rules() {
            let out = validate_and_rank_logits(&[], 6, 5);
            assert_eq!(out.status, TemplateScoreStatus::OutputShapeMismatch);
            assert!(out.scores.is_empty());
        }

        #[test]
        fn empty_output_when_one_file_template_expected() {
            let out = validate_and_rank_logits(&[], 0, 1);
            assert_eq!(out.status, TemplateScoreStatus::OutputShapeMismatch);
        }

        #[test]
        fn output_length_too_short() {
            let out = validate_and_rank_logits(&[0.1, 0.2], 0, 3);
            assert_eq!(out.status, TemplateScoreStatus::OutputShapeMismatch);
        }

        #[test]
        fn output_length_too_long() {
            let out = validate_and_rank_logits(&[0.1, 0.2, 0.3, 0.4], 0, 3);
            assert_eq!(out.status, TemplateScoreStatus::OutputShapeMismatch);
        }

        #[test]
        fn scorer_shape_contract_matches_real_extracted_corpus_rule_count() {
            // Regression guard for Issue #88: `n_rules` here is meant to be
            // exactly `rules.len()` from `chem_env::load_rules_from_file`
            // for the real extracted-template corpus. A model trained for
            // 500 outputs must still see `Available`, never
            // `OutputShapeMismatch` -- which is exactly what an earlier
            // draft of the #88 fix would have broken by inflating
            // `load_rules_from_file`'s return count (500 -> 865) without
            // retraining any model. This test doesn't load a real ONNX
            // model (none is checked in for CI); it locks in the pure
            // shape-validation contract against the corpus's real,
            // current rule count instead.
            let path = concat!(env!("CARGO_MANIFEST_DIR"), "/data/templates_extracted.smi");
            let n_rules = crate::chem_env::load_rules_from_file(path).len();
            assert_eq!(
                n_rules, 500,
                "extracted-template corpus rule count changed -- if intentional, a real \
                 template scorer trained against the old count needs retraining, and this \
                 constant should move with it"
            );
            let synthetic_logits: Vec<f32> = (0..n_rules).map(|i| i as f32 * 0.001).collect();
            let out = validate_and_rank_logits(&synthetic_logits, 0, n_rules);
            assert_eq!(out.status, TemplateScoreStatus::Available);
            assert_eq!(out.scores.len(), n_rules);
        }

        #[test]
        fn rejects_nan() {
            let out = validate_and_rank_logits(&[0.1, f32::NAN], 0, 2);
            assert_eq!(out.status, TemplateScoreStatus::NonFiniteOutput);
            assert!(out.scores.is_empty());
        }

        #[test]
        fn rejects_positive_infinity() {
            let out = validate_and_rank_logits(&[0.1, f32::INFINITY], 0, 2);
            assert_eq!(out.status, TemplateScoreStatus::NonFiniteOutput);
        }

        #[test]
        fn rejects_negative_infinity() {
            let out = validate_and_rank_logits(&[0.1, f32::NEG_INFINITY], 0, 2);
            assert_eq!(out.status, TemplateScoreStatus::NonFiniteOutput);
        }

        #[test]
        fn finite_scores_rank_descending() {
            let out = validate_and_rank_logits(&[0.1, 0.9, 0.5], 0, 3);
            assert_eq!(out.status, TemplateScoreStatus::Available);
            assert_eq!(score_at(&out, 1).rank, 0, "0.9 is the highest logit");
            assert_eq!(score_at(&out, 2).rank, 1, "0.5 is the middle logit");
            assert_eq!(score_at(&out, 0).rank, 2, "0.1 is the lowest logit");
        }

        #[test]
        fn exact_ties_use_absolute_rule_index_ascending() {
            // rule_index 11 and 12 (local indices 1 and 2, both offset by
            // rules_offset=10) tie at raw_logit 0.9 -- the lower absolute
            // rule_index (11) must rank ahead of the higher one (12).
            let out = validate_and_rank_logits(&[0.5, 0.9, 0.9, -1.0], 10, 14);
            assert_eq!(out.status, TemplateScoreStatus::Available);
            assert_eq!(score_at(&out, 11).rank, 0);
            assert_eq!(score_at(&out, 12).rank, 1);
            assert_eq!(score_at(&out, 10).rank, 2);
            assert_eq!(score_at(&out, 13).rank, 3);
        }

        #[test]
        fn rules_offset_is_applied_to_rule_index() {
            let out = validate_and_rank_logits(&[0.1, 0.2], 100, 102);
            assert_eq!(out.status, TemplateScoreStatus::Available);
            let rule_indices: std::collections::HashSet<usize> =
                out.scores.iter().map(|s| s.rule_index).collect();
            assert_eq!(rule_indices, std::collections::HashSet::from([100, 101]));
        }

        #[test]
        fn all_equal_logits_are_deterministic() {
            let out = validate_and_rank_logits(&[0.3, 0.3, 0.3], 0, 3);
            assert_eq!(out.status, TemplateScoreStatus::Available);
            assert_eq!(score_at(&out, 0).rank, 0);
            assert_eq!(score_at(&out, 1).rank, 1);
            assert_eq!(score_at(&out, 2).rank, 2);
        }

        #[test]
        fn repeated_calls_are_identical() {
            let raw_scores = [0.5, 0.9, 0.9, -1.0];
            let a = validate_and_rank_logits(&raw_scores, 10, 14);
            let b = validate_and_rank_logits(&raw_scores, 10, 14);
            for rule_index in [10, 11, 12, 13] {
                let sa = score_at(&a, rule_index);
                let sb = score_at(&b, rule_index);
                assert_eq!(sa.rank, sb.rank);
                assert_eq!(sa.raw_logit, sb.raw_logit);
            }
        }
    }
}

/// Stub for non-nn-scoring builds — scorer module is empty.
#[cfg(not(all(not(target_arch = "wasm32"), feature = "nn-scoring")))]
pub mod nn {}
