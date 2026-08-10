//! Issue #101: runtime candidate reranker, implementing the pre-existing
//! `crate::candidate::CandidateReranker` trait against a LightGBM model
//! frozen offline (Issue #101 Task 35, `data/phase3e_reranker_training/`).
//!
//! Native-only (`#[cfg(not(target_arch = "wasm32"))]`, matching
//! `src/scorer.rs`'s `nn` module): both `LightGbmModel::from_path` and
//! `RuntimeReranker::from_paths` load files via `std::fs`, unavailable on
//! `wasm32-unknown-unknown` (this project's WASM target, see
//! `.github/workflows/ci.yml`). No WASM/Python surface for the runtime
//! reranker in its first integration -- CLI-only, matching the staged
//! rollout the ring-context-policy flag also followed (see `src/main.rs`).
//!
//! Two pieces:
//!   - [`LightGbmModel`]: a from-scratch, pure-Rust reader/evaluator for
//!     LightGBM's plain-text `model.txt` dump format (`booster_type=tree`,
//!     numerical splits only). Deliberately NOT a binding to the LightGBM
//!     C++ library or a third-party inference crate: this codebase has no
//!     C/C++ dependency anywhere (`chematic` is pure Rust; `tract-onnx` was
//!     chosen over ONNX Runtime's C++ bindings for the same reason, see
//!     `src/scorer.rs`'s module doc) and every pure-Rust LightGBM-inference
//!     crate on crates.io as of this writing is unaudited (v0.0.x/v0.1.x,
//!     single-maintainer, one pulling in unrelated ONNX/CatBoost deps by
//!     default) -- a hand-rolled, narrowly-scoped, test-verified reader for
//!     a well-documented text format is the lower-risk choice here, and
//!     stays WASM-linkable in principle (no FFI), matching every other
//!     scoring dependency in this crate.
//!   - [`RuntimeReranker`]: loads a `LightGbmModel` + a
//!     [`TemplateFrequencyTable`] (the second required artifact -- see that
//!     type's doc) and implements `CandidateReranker::score_pool`,
//!     populating `ReactionCandidate.reranker_score` for candidates whose
//!     `.features` have already been computed via
//!     `crate::candidate::extract_features` (this module never computes
//!     features itself -- reusing the exact offline feature-extraction
//!     function is what makes the runtime score comparable to the offline
//!     one at all; see Issue #101 Task 35's PR discussion).

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use sha2::Digest;

use crate::candidate::{CandidateReranker, FEATURE_NAMES_V1, ReactionCandidate};

/// One LightGBM decision tree, flattened into parallel arrays exactly
/// mirroring the text dump's own field layout (`split_feature`,
/// `threshold`, `decision_type`, `left_child`, `right_child`,
/// `leaf_value`) -- node `0` is always the root. `left_child`/`right_child`
/// use LightGBM's own leaf encoding: a non-negative value is another
/// internal node index; a negative value `v` encodes leaf index `-(v)-1`.
#[derive(Debug, Clone)]
struct GbdtTree {
    split_feature: Vec<i32>,
    threshold: Vec<f64>,
    /// Bit 0 = categorical split (never set here -- `num_cat=0` is
    /// asserted per tree at parse time, see `parse_tree`), bit 1 =
    /// `default_left` (where a missing/NaN feature value routes).
    decision_type: Vec<u8>,
    left_child: Vec<i32>,
    right_child: Vec<i32>,
    leaf_value: Vec<f64>,
}

const CATEGORICAL_MASK: u8 = 1;
const DEFAULT_LEFT_MASK: u8 = 2;

impl GbdtTree {
    /// `features[i]` may be `f64::NAN` for a missing feature -- routed via
    /// `default_left` at whichever split reads it, exactly like LightGBM's
    /// own `use_missing=1` semantics (confirmed against this project's own
    /// frozen `model.txt`: `[use_missing: 1]`, `[zero_as_missing: 0]`, i.e.
    /// only real NaN counts as missing, never a literal `0.0`).
    fn predict_one(&self, features: &[f64]) -> f64 {
        if self.left_child.is_empty() {
            // A single-leaf tree (root is itself the only leaf) -- not
            // observed in the frozen model but a well-defined degenerate
            // case (num_leaves=1): LightGBM never emits split arrays for
            // it, so there is nothing to traverse.
            return self.leaf_value.first().copied().unwrap_or(0.0);
        }
        let mut node: usize = 0;
        loop {
            let feat_idx = self.split_feature[node] as usize;
            let value = features.get(feat_idx).copied().unwrap_or(f64::NAN);
            let dt = self.decision_type[node];
            debug_assert_eq!(
                dt & CATEGORICAL_MASK,
                0,
                "categorical splits are not supported (num_cat must be 0)"
            );
            let go_left = if value.is_nan() {
                dt & DEFAULT_LEFT_MASK != 0
            } else {
                value <= self.threshold[node]
            };
            let child = if go_left {
                self.left_child[node]
            } else {
                self.right_child[node]
            };
            if child < 0 {
                let leaf_idx = (-(child + 1)) as usize;
                return self.leaf_value[leaf_idx];
            }
            node = child as usize;
        }
    }
}

/// A parsed LightGBM `model.txt` (`booster=tree`, `objective=lambdarank` in
/// this project's case, but the reader itself doesn't assume any
/// particular objective -- it only sums leaf outputs across trees, which
/// is exactly what LightGBM's own `Booster.predict()` does for a raw
/// (non-probability) ranking score). No sigmoid/softmax is applied here:
/// lambdarank has no such transform on its native score, and applying one
/// would silently diverge from the offline evaluator's own
/// `booster.predict(X)` calls (see `scripts/train_reranker.py::
/// lightgbm_score_fn`), which never request one either.
#[derive(Debug, Clone)]
pub struct LightGbmModel {
    trees: Vec<GbdtTree>,
    max_feature_idx: usize,
}

fn parse_kv_line(line: &str) -> Option<(&str, &str)> {
    line.split_once('=')
}

fn parse_i32_list(s: &str) -> Result<Vec<i32>> {
    s.split_whitespace()
        .map(|tok| {
            tok.parse::<i32>()
                .with_context(|| format!("bad i32 {tok:?}"))
        })
        .collect()
}

fn parse_f64_list(s: &str) -> Result<Vec<f64>> {
    s.split_whitespace()
        .map(|tok| {
            tok.parse::<f64>()
                .with_context(|| format!("bad f64 {tok:?}"))
        })
        .collect()
}

fn parse_tree(block: &str) -> Result<GbdtTree> {
    let mut num_leaves: Option<usize> = None;
    let mut num_cat: Option<usize> = None;
    let mut split_feature = None;
    let mut threshold = None;
    let mut decision_type_raw: Option<Vec<i32>> = None;
    let mut left_child = None;
    let mut right_child = None;
    let mut leaf_value = None;

    for line in block.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("Tree=") {
            continue;
        }
        let Some((key, value)) = parse_kv_line(line) else {
            continue;
        };
        match key {
            "num_leaves" => num_leaves = Some(value.parse()?),
            "num_cat" => num_cat = Some(value.parse()?),
            "split_feature" => split_feature = Some(parse_i32_list(value)?),
            "threshold" => threshold = Some(parse_f64_list(value)?),
            "decision_type" => decision_type_raw = Some(parse_i32_list(value)?),
            "left_child" => left_child = Some(parse_i32_list(value)?),
            "right_child" => right_child = Some(parse_i32_list(value)?),
            "leaf_value" => leaf_value = Some(parse_f64_list(value)?),
            _ => {}
        }
    }

    let num_cat = num_cat.context("tree block missing num_cat")?;
    if num_cat != 0 {
        bail!(
            "tree has num_cat={num_cat} (categorical splits) -- this reader only \
             supports pure-numerical trees, matching the frozen reranker model's \
             own num_cat=0 for every tree"
        );
    }
    let num_leaves = num_leaves.context("tree block missing num_leaves")?;
    let leaf_value = leaf_value.context("tree block missing leaf_value")?;
    if leaf_value.len() != num_leaves {
        bail!(
            "tree declares num_leaves={num_leaves} but leaf_value has {} entries",
            leaf_value.len()
        );
    }

    if num_leaves <= 1 {
        // Degenerate single-leaf tree: no split arrays at all.
        return Ok(GbdtTree {
            split_feature: Vec::new(),
            threshold: Vec::new(),
            decision_type: Vec::new(),
            left_child: Vec::new(),
            right_child: Vec::new(),
            leaf_value,
        });
    }

    let split_feature = split_feature.context("tree block missing split_feature")?;
    let threshold = threshold.context("tree block missing threshold")?;
    let decision_type_raw = decision_type_raw.context("tree block missing decision_type")?;
    let left_child = left_child.context("tree block missing left_child")?;
    let right_child = right_child.context("tree block missing right_child")?;

    let n_internal = num_leaves - 1;
    for (name, len) in [
        ("split_feature", split_feature.len()),
        ("threshold", threshold.len()),
        ("decision_type", decision_type_raw.len()),
        ("left_child", left_child.len()),
        ("right_child", right_child.len()),
    ] {
        if len != n_internal {
            bail!(
                "tree has num_leaves={num_leaves} (expects {n_internal} internal nodes) but {name} has {len} entries"
            );
        }
    }

    let decision_type: Vec<u8> = decision_type_raw
        .iter()
        .map(|&v| {
            u8::try_from(v).with_context(|| format!("decision_type value {v} out of u8 range"))
        })
        .collect::<Result<_>>()?;

    Ok(GbdtTree {
        split_feature,
        threshold,
        decision_type,
        left_child,
        right_child,
        leaf_value,
    })
}

impl LightGbmModel {
    /// Parses a LightGBM text-format model dump (`Booster.save_model()`
    /// output, `[boosting: gbdt]`/`[boosting: tree]`). Hard-errors on any
    /// categorical split (`num_cat != 0`) rather than silently mis-scoring
    /// one -- this project's frozen model has none (`FEATURE_NAMES_V1` is
    /// all-numerical), so encountering one means a mismatched/corrupt model
    /// file, not a case to route around.
    pub fn from_text(text: &str) -> Result<Self> {
        let header_end = text
            .find("\nTree=")
            .context("no Tree= block found in model text")?;
        let header = &text[..header_end];
        let max_feature_idx: usize = header
            .lines()
            .find_map(|l| parse_kv_line(l).filter(|(k, _)| *k == "max_feature_idx"))
            .map(|(_, v)| v.parse::<usize>())
            .transpose()?
            .context("model header missing max_feature_idx")?;

        let mut trees = Vec::new();
        // Split on "Tree=" so each block is self-contained; the header
        // (everything before the first Tree=) has already been consumed.
        for block in text.split("\nTree=").skip(1) {
            // Each tree block runs until the next blank-line-terminated
            // section boundary (LightGBM separates trees with a blank
            // line); trailing content (feature_importances, parameters,
            // pandas_categorical) never starts with a bare number line
            // this parser would misinterpret, since parse_tree only reads
            // recognized `key=value` lines and ignores the rest.
            trees.push(parse_tree(block)?);
        }
        if trees.is_empty() {
            bail!("model text contained no parseable Tree= blocks");
        }
        Ok(Self {
            trees,
            max_feature_idx,
        })
    }

    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let text = std::fs::read_to_string(path.as_ref())
            .with_context(|| format!("read model file {}", path.as_ref().display()))?;
        Self::from_text(&text)
    }

    /// Sum of every tree's leaf output for one row -- the raw lambdarank
    /// score, no post-transform. `features.len()` must be at least
    /// `max_feature_idx + 1`; a shorter vector is a caller bug, not
    /// silently zero-padded.
    pub fn predict(&self, features: &[f64]) -> Result<f64> {
        if features.len() <= self.max_feature_idx {
            bail!(
                "feature vector has {} entries, model needs at least {}",
                features.len(),
                self.max_feature_idx + 1
            );
        }
        Ok(self.trees.iter().map(|t| t.predict_one(features)).sum())
    }

    pub fn num_trees(&self) -> usize {
        self.trees.len()
    }
}

/// TRAIN-frozen `template_id -> log(count+1)` frequency table (Issue #101
/// Task 35's `data/phase3e_reranker_training/frequency_table.json`,
/// SHA-256-verified against the value baked into the model's own training
/// run). `FEATURE_NAMES_V1` indices 16/17 (`max`/`mean_template_log_
/// frequency`) are ALWAYS missing on a freshly-extracted candidate (see
/// `extract_features`'s own doc) -- the offline training pipeline
/// post-hoc-imputes them from exactly this table
/// (`impute_frequency_features`) before the model ever sees a row, so a
/// runtime scorer that skips this step feeds the model a feature
/// distribution it was never trained on for those two columns.
#[derive(Debug, Clone, Default)]
pub struct TemplateFrequencyTable(HashMap<String, f32>);

impl TemplateFrequencyTable {
    pub fn from_json_str(s: &str) -> Result<Self> {
        #[derive(serde::Deserialize)]
        struct Wire {
            table: HashMap<String, f32>,
        }
        let wire: Wire = serde_json::from_str(s).context("parse frequency table JSON")?;
        Ok(Self(wire.table))
    }

    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let text = std::fs::read_to_string(path.as_ref())
            .with_context(|| format!("read frequency table {}", path.as_ref().display()))?;
        Self::from_json_str(&text)
    }

    /// Mirrors `impute_frequency_features` exactly: max/mean over whichever
    /// of `template_ids` are present in the table; `None` if none are
    /// known (leave missing, never guess).
    fn max_mean(&self, template_ids: &[String]) -> Option<(f32, f32)> {
        let known: Vec<f32> = template_ids
            .iter()
            .filter_map(|t| self.0.get(t).copied())
            .collect();
        if known.is_empty() {
            return None;
        }
        let max = known.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let mean = known.iter().sum::<f32>() / known.len() as f32;
        Some((max, mean))
    }
}

fn feature_index(name: &str) -> usize {
    FEATURE_NAMES_V1
        .iter()
        .position(|&n| n == name)
        .unwrap_or_else(|| panic!("FEATURE_NAMES_V1 has no {name:?} entry"))
}

/// Runtime implementation of `CandidateReranker` -- ordering-only per
/// Issue #101's runtime-integration contract: this type only ever writes
/// `ReactionCandidate.reranker_score`, never touches `precursor_smiles`,
/// never removes a candidate, never changes `sources`. How that score
/// then adjusts search ordering (a rank-derived bonus on the existing
/// `template_bonus` scale, NOT the raw score -- seeArch `src/search.rs`'s
/// reranker-bonus wiring) is a separate, explicitly-runtime-only decision
/// this type has no opinion on.
pub struct RuntimeReranker {
    model: LightGbmModel,
    freq_table: TemplateFrequencyTable,
    model_sha256: String,
}

impl RuntimeReranker {
    pub fn from_paths(
        model_path: impl AsRef<Path>,
        freq_table_path: impl AsRef<Path>,
    ) -> Result<Self> {
        let model_bytes = std::fs::read(model_path.as_ref())
            .with_context(|| format!("read model file {}", model_path.as_ref().display()))?;
        let model_sha256 = format!(
            "sha256:{}",
            crate::sha256_hex(sha2::Sha256::digest(&model_bytes))
        );
        let model_text = String::from_utf8(model_bytes).with_context(|| {
            format!(
                "model file {} is not valid UTF-8",
                model_path.as_ref().display()
            )
        })?;
        let model = LightGbmModel::from_text(&model_text)?;
        let freq_table = TemplateFrequencyTable::from_path(freq_table_path)?;
        Ok(Self {
            model,
            freq_table,
            model_sha256,
        })
    }

    pub fn model_sha256(&self) -> &str {
        &self.model_sha256
    }
}

impl CandidateReranker for RuntimeReranker {
    fn score_pool(&self, _target: &str, candidates: &mut [ReactionCandidate]) -> Result<()> {
        let max_i = feature_index("max_template_log_frequency");
        let mean_i = feature_index("mean_template_log_frequency");

        for candidate in candidates.iter_mut() {
            if candidate.features.values.len() != FEATURE_NAMES_V1.len() {
                bail!(
                    "candidate {:?} has {} features, expected {} (extract_features must run \
                     before RuntimeReranker::score_pool)",
                    candidate.candidate_id,
                    candidate.features.values.len(),
                    FEATURE_NAMES_V1.len()
                );
            }

            let template_ids: Vec<String> = candidate
                .sources
                .iter()
                .map(|s| s.template_id.clone())
                .collect();
            let mut values = candidate.features.values.clone();
            let mut missing = candidate.features.missing.clone();
            if let Some((max, mean)) = self.freq_table.max_mean(&template_ids) {
                values[max_i] = max;
                values[mean_i] = mean;
                missing[max_i] = false;
                missing[mean_i] = false;
            }

            let feat_vec: Vec<f64> = values
                .iter()
                .zip(missing.iter())
                .map(|(&v, &m)| if m { f64::NAN } else { v as f64 })
                .collect();
            let score = self.model.predict(&feat_vec)?;
            candidate.reranker_score = Some(score);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hand-built, minimal single-split tree: feature 0 <= 5.0 -> leaf 0
    /// (value -1.0), else leaf 1 (value 2.0); missing -> default_left (goes
    /// to leaf 0). Two such trees summed = the model's predict output.
    fn one_split_model_text(default_left: bool, num_features: usize) -> String {
        let decision_type = if default_left { 2 } else { 0 }; // bit1=default_left, bit0=categorical(never)
        format!(
            "tree\nversion=v4\nnum_class=1\nnum_tree_per_iteration=1\nlabel_index=0\n\
             max_feature_idx={}\nobjective=regression\nfeature_names=f0\nfeature_infos=[0:10]\n\
             tree_sizes=1\n\n\
             Tree=0\nnum_leaves=2\nnum_cat=0\nsplit_feature=0\nsplit_gain=1\nthreshold=5.0\n\
             decision_type={}\nleft_child=-1\nright_child=-2\nleaf_value=-1.0 2.0\n\
             leaf_weight=1 1\nleaf_count=1 1\ninternal_value=0\ninternal_weight=1\n\
             internal_count=2\nis_linear=0\nshrinkage=1\n\n\
             end of trees\n\nfeature_importances:\nf0=1\n\nparameters:\n[boosting: gbdt]\n\n\
             end of parameters\n\npandas_categorical:null\n",
            num_features - 1,
            decision_type
        )
    }

    #[test]
    fn single_split_routes_left_and_right_by_threshold() {
        let model = LightGbmModel::from_text(&one_split_model_text(true, 1)).unwrap();
        assert_eq!(model.predict(&[0.0]).unwrap(), -1.0); // 0.0 <= 5.0 -> left leaf
        assert_eq!(model.predict(&[5.0]).unwrap(), -1.0); // boundary: <= is left
        assert_eq!(model.predict(&[5.0001]).unwrap(), 2.0); // > 5.0 -> right leaf
        assert_eq!(model.predict(&[100.0]).unwrap(), 2.0);
    }

    #[test]
    fn missing_value_routes_by_default_left_bit() {
        let default_left = LightGbmModel::from_text(&one_split_model_text(true, 1)).unwrap();
        assert_eq!(default_left.predict(&[f64::NAN]).unwrap(), -1.0);

        let default_right = LightGbmModel::from_text(&one_split_model_text(false, 1)).unwrap();
        assert_eq!(default_right.predict(&[f64::NAN]).unwrap(), 2.0);
    }

    #[test]
    fn multiple_trees_sum_leaf_outputs() {
        // Two independent single-split trees back to back, same shape as
        // the real model.txt's tree separation (blank line, "Tree=N").
        let text = "tree\nversion=v4\nmax_feature_idx=0\nobjective=regression\n\
             feature_names=f0\nfeature_infos=[0:10]\ntree_sizes=1 1\n\n\
             Tree=0\nnum_leaves=2\nnum_cat=0\nsplit_feature=0\nthreshold=5.0\n\
             decision_type=2\nleft_child=-1\nright_child=-2\nleaf_value=-1.0 2.0\n\
             is_linear=0\nshrinkage=1\n\n\
             Tree=1\nnum_leaves=2\nnum_cat=0\nsplit_feature=0\nthreshold=1.0\n\
             decision_type=2\nleft_child=-1\nright_child=-2\nleaf_value=10.0 20.0\n\
             is_linear=0\nshrinkage=1\n\n\
             end of trees\n\nfeature_importances:\nf0=2\n\nparameters:\n[boosting: gbdt]\n\n\
             end of parameters\n\npandas_categorical:null\n";
        let model = LightGbmModel::from_text(text).unwrap();
        assert_eq!(model.num_trees(), 2);
        // x=0.0: tree0 left(-1.0) + tree1 left(10.0) = 9.0
        assert_eq!(model.predict(&[0.0]).unwrap(), 9.0);
        // x=3.0: tree0 left(-1.0, 3<=5) + tree1 right(20.0, 3>1) = 19.0
        assert_eq!(model.predict(&[3.0]).unwrap(), 19.0);
        // x=100.0: tree0 right(2.0) + tree1 right(20.0) = 22.0
        assert_eq!(model.predict(&[100.0]).unwrap(), 22.0);
    }

    #[test]
    fn rejects_categorical_tree() {
        let text = "tree\nmax_feature_idx=0\nobjective=regression\n\n\
             Tree=0\nnum_leaves=2\nnum_cat=1\nsplit_feature=0\nthreshold=5.0\n\
             decision_type=1\nleft_child=-1\nright_child=-2\nleaf_value=-1.0 2.0\n\n";
        assert!(LightGbmModel::from_text(text).is_err());
    }

    #[test]
    fn frequency_table_max_mean_matches_python_semantics() {
        let json = r#"{"table": {"t1": 1.0, "t2": 3.0}}"#;
        let table = TemplateFrequencyTable::from_json_str(json).unwrap();
        let (max, mean) = table
            .max_mean(&["t1".to_string(), "t2".to_string()])
            .unwrap();
        assert_eq!(max, 3.0);
        assert_eq!(mean, 2.0);
        assert!(table.max_mean(&["unknown".to_string()]).is_none());
    }
}
