//! JSONL candidate-pool export for offline reranker training/evaluation.
//!
//! One JSON object per line (JSONL), one line per merged candidate, feature
//! values already extracted per [`crate::candidate::FEATURE_SCHEMA_VERSION`].
//! A sidecar [`PoolManifest`] records exactly what this pool *is* --
//! [`ProposalMode`](crate::candidate::ProposalMode) and its params, the
//! rules set's content hash, and the stock identity/count -- so a consumer
//! never has to guess or assume. See `crate::candidate`'s module doc for why
//! this matters: different `ProposalMode`s produce different candidate
//! *sets*, not just different orderings, so a pool that doesn't record its
//! mode could silently get trained on one mode and evaluated as if it were
//! another.
//!
//! This module writes rows/manifests; it does not decide *which* targets to
//! run or generate a pool at any particular scale -- that is a driver's
//! responsibility, kept out of this crate deliberately (see the repo's
//! staged 100/500/full-corpus gate).

use std::collections::HashMap;
use std::io::Write;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::candidate::{
    CandidatePool, FEATURE_NAMES_V1, FEATURE_SCHEMA_VERSION, ProposalMode, extract_features,
};
use crate::chem_env::{ChemEnv, Molecule, RetroRule};

/// Serializable summary of a [`ProposalMode`] for the manifest.
/// `ProposalMode` itself isn't `Serialize` (its `ScorerConditioned` variant
/// carries non-serializable scorer output types), so this captures only
/// what a pool consumer needs to know: which mode, and its retrieval
/// parameter.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProposalModeSummary {
    pub mode: &'static str,
    pub top_k: Option<usize>,
}

impl ProposalModeSummary {
    pub fn from_mode(mode: &ProposalMode) -> Self {
        match mode {
            ProposalMode::Exhaustive => Self {
                mode: "exhaustive",
                top_k: None,
            },
            ProposalMode::BondIndexed { top_k } => Self {
                mode: "bond_indexed",
                top_k: Some(*top_k),
            },
            ProposalMode::ScorerConditioned { top_k, .. } => Self {
                mode: "scorer_conditioned",
                top_k: Some(*top_k),
            },
        }
    }
}

/// One exported candidate row (one JSONL line).
#[derive(Debug, Clone, Serialize)]
pub struct CandidateRow {
    pub target_id: String,
    pub target_smiles: String,
    pub candidate_id: String,
    pub precursor_smiles: Vec<String>,
    pub source_template_count: usize,
    pub best_upstream_rank: usize,
    pub feature_schema_version: u32,
    pub feature_values: Vec<f32>,
    pub feature_missing: Vec<bool>,
}

/// Build a `template_id -> &RetroRule` index once, then extract features
/// and build one [`CandidateRow`] per candidate in `pool`.
///
/// Candidates are sorted by `candidate_id` (lexicographic) before export --
/// `merge_into_candidates`' insertion order happens to be deterministic
/// today, but that's an implementation detail of a `rayon`-parallel
/// collect, not a contract; sorting here makes byte-identical JSONL output
/// an explicit property of the exporter itself.
pub fn candidate_rows_for_pool(
    pool: &CandidatePool,
    target_mol: &Molecule,
    templates_by_id: &HashMap<String, &RetroRule>,
    stock: Option<&ChemEnv>,
) -> Vec<CandidateRow> {
    let mut candidates: Vec<&crate::candidate::ReactionCandidate> =
        pool.candidates.iter().collect();
    candidates.sort_by(|a, b| a.candidate_id.cmp(&b.candidate_id));

    candidates
        .into_iter()
        .map(|c| {
            let features = extract_features(c, target_mol, templates_by_id, stock);
            CandidateRow {
                target_id: pool.target_id.clone(),
                target_smiles: pool.target_smiles.clone(),
                candidate_id: c.candidate_id.clone(),
                precursor_smiles: c.precursor_smiles.clone(),
                source_template_count: c.source_template_count,
                best_upstream_rank: c.best_upstream_rank,
                feature_schema_version: FEATURE_SCHEMA_VERSION,
                feature_values: features.values,
                feature_missing: features.missing,
            }
        })
        .collect()
}

/// Write `rows` as JSONL (one compact JSON object per line) to `writer`.
pub fn write_jsonl<W: Write>(rows: &[CandidateRow], mut writer: W) -> anyhow::Result<()> {
    for row in rows {
        serde_json::to_writer(&mut writer, row)?;
        writer.write_all(b"\n")?;
    }
    Ok(())
}

/// Content hash over a rules set: sorted by `template_id`, each rule framed
/// as (template_id, smirks, weight-bits, required_elements) with
/// length-prefixed fields -- the same unambiguous framing used for
/// candidate identity (see `candidate::hash_string_sequence`), so two
/// different rule sets can never collide onto the same hash. `weight` is
/// hashed via its IEEE-754 bit pattern (`to_bits`), not its `Display`
/// string, so the hash is exact rather than dependent on float-formatting
/// precision.
pub fn rules_content_hash(rules: &[RetroRule]) -> String {
    let mut sorted: Vec<&RetroRule> = rules.iter().collect();
    sorted.sort_by(|a, b| a.template_id.cmp(&b.template_id));

    let mut hasher = Sha256::new();
    hasher.update(b"renkin-retrospect-rules-v1\0");
    hasher.update((sorted.len() as u64).to_be_bytes());
    for rule in sorted {
        for field in [rule.template_id.as_str(), rule.smirks.as_str()] {
            hasher.update((field.len() as u64).to_be_bytes());
            hasher.update(field.as_bytes());
        }
        hasher.update(rule.weight.to_bits().to_be_bytes());
        hasher.update(rule.required_elements.to_be_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}

/// Manifest describing one candidate-pool export in full: everything a
/// consumer (a training/evaluation script, or a human debugging a
/// surprising result) needs to know about what this pool *is*, without
/// re-deriving it from the JSONL rows themselves.
#[derive(Debug, Clone, Serialize)]
pub struct PoolManifest {
    pub manifest_schema_version: u32,
    pub feature_schema_version: u32,
    pub feature_names: Vec<&'static str>,
    pub proposal_mode: ProposalModeSummary,
    pub rules_content_hash: String,
    pub rules_count: usize,
    /// `None` when no stock was supplied to feature extraction (every
    /// stock-dependent feature is `missing` in every row in that case).
    pub stock_identity: Option<String>,
    pub stock_compound_count: Option<usize>,
    pub target_count: usize,
    pub candidate_count: usize,
}

pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

/// Build the manifest for an export spanning `rows` drawn from
/// `target_count` distinct targets. `stock_identity` is a caller-supplied
/// label (e.g. a file path, or `"in_memory"` for a synthetic/test stock) --
/// this module never guesses one, since a wrong or silently-substituted
/// stock identity is exactly the kind of provenance gap this manifest
/// exists to close (see this repo's own history of a benchmark run
/// silently falling back to a smaller stock than intended).
pub fn build_manifest(
    rows: &[CandidateRow],
    target_count: usize,
    rules: &[RetroRule],
    mode: &ProposalMode,
    stock: Option<(&str, &ChemEnv)>,
) -> PoolManifest {
    PoolManifest {
        manifest_schema_version: MANIFEST_SCHEMA_VERSION,
        feature_schema_version: FEATURE_SCHEMA_VERSION,
        feature_names: FEATURE_NAMES_V1.to_vec(),
        proposal_mode: ProposalModeSummary::from_mode(mode),
        rules_content_hash: rules_content_hash(rules),
        rules_count: rules.len(),
        stock_identity: stock.map(|(id, _)| id.to_string()),
        stock_compound_count: stock.map(|(_, env)| env.bb_count()),
        target_count,
        candidate_count: rows.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::{ProposalConfig, index_rules_by_template_id, propose_one_step};
    use crate::chem_env::{default_rules, mol_from_smiles};

    #[test]
    fn candidate_rows_are_sorted_by_candidate_id() {
        let rules = default_rules();
        let target = "CC(=O)c1ccccc1";
        let target_mol = mol_from_smiles(target).unwrap();
        let pool = propose_one_step(target, &rules, &ProposalConfig::default()).unwrap();
        let templates_by_id = index_rules_by_template_id(&rules);

        let rows = candidate_rows_for_pool(&pool, &target_mol, &templates_by_id, None);
        assert!(!rows.is_empty());
        let ids: Vec<&str> = rows.iter().map(|r| r.candidate_id.as_str()).collect();
        let mut sorted_ids = ids.clone();
        sorted_ids.sort_unstable();
        assert_eq!(ids, sorted_ids);
    }

    #[test]
    fn candidate_rows_carry_feature_schema_version_and_matching_lengths() {
        let rules = default_rules();
        let target = "CC(=O)c1ccccc1";
        let target_mol = mol_from_smiles(target).unwrap();
        let pool = propose_one_step(target, &rules, &ProposalConfig::default()).unwrap();
        let templates_by_id = index_rules_by_template_id(&rules);

        let rows = candidate_rows_for_pool(&pool, &target_mol, &templates_by_id, None);
        for row in &rows {
            assert_eq!(row.feature_schema_version, FEATURE_SCHEMA_VERSION);
            assert_eq!(row.feature_values.len(), FEATURE_NAMES_V1.len());
            assert_eq!(row.feature_missing.len(), FEATURE_NAMES_V1.len());
        }
    }

    #[test]
    fn write_jsonl_is_one_valid_json_object_per_line() {
        let rules = default_rules();
        let target = "CC(=O)c1ccccc1";
        let target_mol = mol_from_smiles(target).unwrap();
        let pool = propose_one_step(target, &rules, &ProposalConfig::default()).unwrap();
        let templates_by_id = index_rules_by_template_id(&rules);
        let rows = candidate_rows_for_pool(&pool, &target_mol, &templates_by_id, None);

        let mut buf: Vec<u8> = Vec::new();
        write_jsonl(&rows, &mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), rows.len());
        for line in &lines {
            let parsed: serde_json::Value =
                serde_json::from_str(line).expect("each line must be valid JSON");
            assert!(parsed.is_object());
        }
    }

    #[test]
    fn two_export_runs_produce_byte_identical_jsonl() {
        let rules = default_rules();
        let target = "CC(=O)c1ccccc1";
        let target_mol = mol_from_smiles(target).unwrap();
        let templates_by_id = index_rules_by_template_id(&rules);

        let mut outputs = Vec::new();
        for _ in 0..2 {
            let pool = propose_one_step(target, &rules, &ProposalConfig::default()).unwrap();
            let rows = candidate_rows_for_pool(&pool, &target_mol, &templates_by_id, None);
            let mut buf: Vec<u8> = Vec::new();
            write_jsonl(&rows, &mut buf).unwrap();
            outputs.push(buf);
        }
        assert_eq!(outputs[0], outputs[1]);
    }

    #[test]
    fn rules_content_hash_differs_on_smirks_change_stable_otherwise() {
        let rules_a = default_rules();
        let mut rules_b = default_rules();
        rules_b[0].smirks = format!("{}X", rules_b[0].smirks);

        assert_ne!(rules_content_hash(&rules_a), rules_content_hash(&rules_b));
        assert_eq!(
            rules_content_hash(&rules_a),
            rules_content_hash(&default_rules())
        );
    }

    #[test]
    fn rules_content_hash_is_order_independent() {
        let mut rules_a = default_rules();
        let mut rules_b = rules_a.clone();
        rules_b.reverse();
        rules_a.sort_by(|a, b| a.name.cmp(&b.name));
        rules_b.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(rules_content_hash(&rules_a), rules_content_hash(&rules_b));
    }

    #[test]
    fn manifest_records_mode_rules_and_stock_identity() {
        let rules = default_rules();
        let target = "CC(=O)c1ccccc1";
        let target_mol = mol_from_smiles(target).unwrap();
        let pool = propose_one_step(target, &rules, &ProposalConfig::default()).unwrap();
        let templates_by_id = index_rules_by_template_id(&rules);
        let rows = candidate_rows_for_pool(&pool, &target_mol, &templates_by_id, None);

        let manifest = build_manifest(&rows, 1, &rules, &ProposalMode::Exhaustive, None);
        assert_eq!(manifest.manifest_schema_version, MANIFEST_SCHEMA_VERSION);
        assert_eq!(manifest.feature_schema_version, FEATURE_SCHEMA_VERSION);
        assert_eq!(manifest.feature_names.len(), FEATURE_NAMES_V1.len());
        assert_eq!(
            manifest.proposal_mode,
            ProposalModeSummary {
                mode: "exhaustive",
                top_k: None,
            }
        );
        assert_eq!(manifest.rules_count, rules.len());
        assert_eq!(manifest.rules_content_hash, rules_content_hash(&rules));
        assert!(manifest.stock_identity.is_none());
        assert!(manifest.stock_compound_count.is_none());
        assert_eq!(manifest.target_count, 1);
        assert_eq!(manifest.candidate_count, rows.len());

        let stock = ChemEnv::in_memory(&["CCO", "CC(=O)O"]);
        let manifest_with_stock = build_manifest(
            &rows,
            1,
            &rules,
            &ProposalMode::BondIndexed { top_k: 5 },
            Some(("in_memory:test", &stock)),
        );
        assert_eq!(
            manifest_with_stock.stock_identity,
            Some("in_memory:test".to_string())
        );
        assert_eq!(manifest_with_stock.stock_compound_count, Some(2));
        assert_eq!(
            manifest_with_stock.proposal_mode,
            ProposalModeSummary {
                mode: "bond_indexed",
                top_k: Some(5),
            }
        );
    }

    #[test]
    fn manifest_serializes_to_json() {
        let rules = default_rules();
        let manifest = build_manifest(&[], 0, &rules, &ProposalMode::Exhaustive, None);
        let json = serde_json::to_string(&manifest).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["feature_names"].is_array());
        assert_eq!(parsed["proposal_mode"]["mode"], "exhaustive");
    }
}
