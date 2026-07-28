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
    CandidatePool, FEATURE_NAMES_V1, FEATURE_SCHEMA_VERSION, ProposalMode, UpstreamScoreStatus,
    extract_features, feature_schema_hash,
};
use crate::chem_env::{ChemEnv, Molecule, RetroRule};

/// Serializable summary of a [`ProposalMode`] for the manifest.
/// `ProposalMode` itself isn't `Serialize` (its `ScorerConditioned` variant
/// carries non-serializable scorer output types), so this captures only
/// what a pool consumer needs to know: which mode, its retrieval parameter,
/// and -- for `ScorerConditioned` -- the scorer provenance that lives on
/// `ScorerConditionedInput` (`rules_offset`, `scorer_identity`,
/// `scorer_model_sha256`, `status`), so a manifest can tell two
/// scorer-conditioned pools apart even when both otherwise look identical.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProposalModeSummary {
    pub mode: &'static str,
    pub top_k: Option<usize>,
    pub rules_offset: Option<usize>,
    pub scorer_identity: Option<String>,
    pub scorer_model_sha256: Option<String>,
    pub scorer_status: Option<UpstreamScoreStatus>,
}

impl ProposalModeSummary {
    pub fn from_mode(mode: &ProposalMode) -> Self {
        match mode {
            ProposalMode::Exhaustive => Self {
                mode: "exhaustive",
                top_k: None,
                rules_offset: None,
                scorer_identity: None,
                scorer_model_sha256: None,
                scorer_status: None,
            },
            ProposalMode::BondIndexed { top_k } => Self {
                mode: "bond_indexed",
                top_k: Some(*top_k),
                rules_offset: None,
                scorer_identity: None,
                scorer_model_sha256: None,
                scorer_status: None,
            },
            ProposalMode::ScorerConditioned { input, top_k } => Self {
                mode: "scorer_conditioned",
                top_k: Some(*top_k),
                rules_offset: Some(input.rules_offset),
                scorer_identity: Some(input.scorer_identity.clone()),
                scorer_model_sha256: Some(input.scorer_model_sha256.clone()),
                scorer_status: Some(input.status),
            },
        }
    }
}

/// One exported source's full provenance (one entry per distinct
/// contributing rule -- see `candidate::merge_duplicate_sources`).
#[derive(Debug, Clone, Serialize)]
pub struct SourceRow {
    pub template_id: String,
    pub rule_name: String,
    pub original_rank: usize,
    pub upstream_score: Option<f32>,
    pub upstream_score_status: UpstreamScoreStatus,
    pub template_log_frequency_raw: Option<f32>,
    pub base_step_cost: f64,
}

/// One exported candidate row (one JSONL line).
#[derive(Debug, Clone, Serialize)]
pub struct CandidateRow {
    pub group_id: String,
    pub target_id: String,
    pub target_smiles: String,
    pub candidate_id: String,
    pub precursor_smiles: Vec<String>,
    pub source_template_count: usize,
    pub best_upstream_rank: usize,
    pub sources: Vec<SourceRow>,
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
            let sources = c
                .sources
                .iter()
                .map(|s| SourceRow {
                    template_id: s.template_id.clone(),
                    rule_name: s.rule_name.clone(),
                    original_rank: s.original_rank,
                    upstream_score: s.upstream_score,
                    upstream_score_status: s.upstream_score_status,
                    template_log_frequency_raw: s.template_log_frequency_raw,
                    base_step_cost: s.base_step_cost,
                })
                .collect();
            CandidateRow {
                group_id: pool.group_id.clone(),
                target_id: pool.target_id.clone(),
                target_smiles: pool.target_smiles.clone(),
                candidate_id: c.candidate_id.clone(),
                precursor_smiles: c.precursor_smiles.clone(),
                source_template_count: c.source_template_count,
                best_upstream_rank: c.best_upstream_rank,
                sources,
                feature_schema_version: FEATURE_SCHEMA_VERSION,
                feature_values: features.values,
                feature_missing: features.missing,
            }
        })
        .collect()
}

/// Why a [`TargetPoolRecord`] has the candidate count it does. Kept distinct
/// from "zero candidates" so a consumer can tell "the target parsed fine and
/// genuinely has no one-step disconnections" apart from "proposal never ran
/// for this group at all" -- both produce `candidate_count == 0`, but only
/// the first is a real coverage gap; the second is a data-generation defect
/// that must not be silently counted as one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Ok,
    TargetParseFailed,
}

/// One record per (group_id, target) proposal attempt, exported alongside
/// (never derived from) the candidate JSONL -- a target with zero
/// candidates, or a target whose SMILES failed to parse at all, still gets
/// exactly one record here. A consumer builds its coverage denominator (how
/// many groups exist) from this file plus labels, never by counting distinct
/// `group_id`s that happen to appear in the candidate rows -- a group with
/// zero candidates would otherwise silently vanish from that count.
#[derive(Debug, Clone, Serialize)]
pub struct TargetPoolRecord {
    pub group_id: String,
    pub target_id: String,
    pub target_smiles: String,
    pub candidate_count: usize,
    pub proposal_status: ProposalStatus,
}

/// Build the record for a group whose proposal succeeded (`pool` may still
/// have zero candidates -- that's a real, reportable coverage gap, not an
/// error).
pub fn target_pool_record_for_pool(pool: &CandidatePool) -> TargetPoolRecord {
    TargetPoolRecord {
        group_id: pool.group_id.clone(),
        target_id: pool.target_id.clone(),
        target_smiles: pool.target_smiles.clone(),
        candidate_count: pool.candidates.len(),
        proposal_status: ProposalStatus::Ok,
    }
}

/// Build the record for a group whose proposal failed outright (e.g.
/// `propose_one_step`'s target SMILES did not parse) -- there is no
/// `CandidatePool` to draw a canonical `target_id` from, so the caller's
/// original (uncanonicalized) requested SMILES is recorded in both
/// `target_id` and `target_smiles` fields, and `candidate_count` is `0` with
/// `proposal_status: TargetParseFailed` rather than being indistinguishable
/// from a successful zero-candidate outcome.
pub fn target_pool_record_for_failure(
    group_id: &str,
    requested_target_smiles: &str,
) -> TargetPoolRecord {
    TargetPoolRecord {
        group_id: group_id.to_string(),
        target_id: requested_target_smiles.to_string(),
        target_smiles: requested_target_smiles.to_string(),
        candidate_count: 0,
        proposal_status: ProposalStatus::TargetParseFailed,
    }
}

/// Rejects a duplicate `group_id` within the group index -- each (group_id,
/// target) proposal attempt must be recorded exactly once.
fn validate_group_index(records: &[TargetPoolRecord]) -> anyhow::Result<()> {
    let mut seen = std::collections::HashSet::new();
    for record in records {
        if !seen.insert(record.group_id.as_str()) {
            anyhow::bail!(
                "duplicate group_id {:?} in the target/group index",
                record.group_id
            );
        }
    }
    Ok(())
}

/// Hard-validate every invariant a consumer relies on, before any bytes are
/// written -- a malformed row must never reach disk looking like a valid
/// export. Mirrors (and must stay in sync with) the equivalent checks
/// `scripts/train_reranker.py` performs on load: catching a bad row here,
/// at export time, is strictly better than a downstream loader silently
/// truncating a length mismatch (e.g. via an unchecked `zip()`) or training
/// on a non-finite feature value.
///
/// Duplicate `candidate_id` is checked *within one `group_id`*, not
/// globally -- `candidate_id` is a hash of (canonical target, precursor
/// set) alone, so the same candidate legitimately recurs under two
/// different `group_id`s when two dataset examples share a `target_id`
/// (see `CandidatePool`'s doc). Within a single group, though, a repeated
/// `candidate_id` means the same merged candidate was emitted twice, which
/// must never happen.
fn validate_candidate_rows(rows: &[CandidateRow]) -> anyhow::Result<()> {
    let mut seen_candidate_ids_by_group: HashMap<&str, std::collections::HashSet<&str>> =
        HashMap::new();
    let mut target_id_by_group: HashMap<&str, &str> = HashMap::new();
    let mut target_smiles_by_group: HashMap<&str, &str> = HashMap::new();

    for row in rows {
        if !seen_candidate_ids_by_group
            .entry(row.group_id.as_str())
            .or_default()
            .insert(row.candidate_id.as_str())
        {
            anyhow::bail!(
                "duplicate candidate_id {:?} within group_id {:?}",
                row.candidate_id,
                row.group_id
            );
        }
        match target_id_by_group.get(row.group_id.as_str()) {
            None => {
                target_id_by_group.insert(row.group_id.as_str(), row.target_id.as_str());
            }
            Some(&existing) if existing != row.target_id => {
                anyhow::bail!(
                    "group_id {:?} has rows with inconsistent target_id ({:?} vs {:?})",
                    row.group_id,
                    existing,
                    row.target_id
                );
            }
            _ => {}
        }
        match target_smiles_by_group.get(row.group_id.as_str()) {
            None => {
                target_smiles_by_group.insert(row.group_id.as_str(), row.target_smiles.as_str());
            }
            Some(&existing) if existing != row.target_smiles => {
                anyhow::bail!(
                    "group_id {:?} has rows with inconsistent target_smiles ({:?} vs {:?})",
                    row.group_id,
                    existing,
                    row.target_smiles
                );
            }
            _ => {}
        }

        if row.precursor_smiles.is_empty() {
            anyhow::bail!(
                "candidate {:?} (group_id {:?}) has an empty precursor_smiles list",
                row.candidate_id,
                row.group_id
            );
        }
        if row.sources.is_empty() {
            anyhow::bail!(
                "candidate {:?} (group_id {:?}) has an empty sources list",
                row.candidate_id,
                row.group_id
            );
        }
        if row.feature_values.len() != FEATURE_NAMES_V1.len()
            || row.feature_missing.len() != FEATURE_NAMES_V1.len()
        {
            anyhow::bail!(
                "candidate {:?} has feature_values.len()={} feature_missing.len()={}, \
                 expected {} (FEATURE_NAMES_V1.len())",
                row.candidate_id,
                row.feature_values.len(),
                row.feature_missing.len(),
                FEATURE_NAMES_V1.len()
            );
        }
        for (i, (&value, &missing)) in row
            .feature_values
            .iter()
            .zip(&row.feature_missing)
            .enumerate()
        {
            if !missing && !value.is_finite() {
                anyhow::bail!(
                    "candidate {:?} feature[{}] ({:?}) is non-finite ({}) but not marked missing",
                    row.candidate_id,
                    i,
                    FEATURE_NAMES_V1.get(i).copied().unwrap_or("?"),
                    value
                );
            }
        }
    }
    Ok(())
}

/// Cross-check that every `group_id` appearing in `rows` has a
/// corresponding entry in `target_pool_records`, with matching `target_id`
/// and `target_smiles` -- catches an exporter (or a caller assembling these
/// two files independently) that wrote candidate rows for a group whose
/// group-index record was never written, or that disagrees with it.
fn validate_rows_consistent_with_group_index(
    rows: &[CandidateRow],
    target_pool_records: &[TargetPoolRecord],
) -> anyhow::Result<()> {
    let index: HashMap<&str, &TargetPoolRecord> = target_pool_records
        .iter()
        .map(|r| (r.group_id.as_str(), r))
        .collect();
    let mut row_count_by_group: HashMap<&str, usize> = HashMap::new();
    for row in rows {
        match index.get(row.group_id.as_str()) {
            None => anyhow::bail!(
                "candidate row's group_id {:?} has no entry in the target/group index",
                row.group_id
            ),
            Some(record) => {
                if record.target_id != row.target_id {
                    anyhow::bail!(
                        "group_id {:?}: candidate row target_id {:?} does not match \
                         group index target_id {:?}",
                        row.group_id,
                        row.target_id,
                        record.target_id
                    );
                }
                if record.target_smiles != row.target_smiles {
                    anyhow::bail!(
                        "group_id {:?}: candidate row target_smiles {:?} does not match \
                         group index target_smiles {:?}",
                        row.group_id,
                        row.target_smiles,
                        record.target_smiles
                    );
                }
            }
        }
        *row_count_by_group.entry(row.group_id.as_str()).or_insert(0) += 1;
    }
    // Every group's `candidate_count` is a claim about how many candidate
    // rows exist for it -- checked against the rows actually present, not
    // just recorded and trusted. A record claiming a nonzero count with no
    // matching rows (or vice versa) is caught here too, since a missing
    // group_id contributes 0 to `row_count_by_group`.
    for record in target_pool_records {
        let actual = row_count_by_group
            .get(record.group_id.as_str())
            .copied()
            .unwrap_or(0);
        if actual != record.candidate_count {
            anyhow::bail!(
                "group_id {:?}: group index claims candidate_count={}, but {} candidate \
                 row(s) were actually found",
                record.group_id,
                record.candidate_count,
                actual
            );
        }
    }
    Ok(())
}

/// Write `records` as JSONL, one object per line -- same framing as
/// [`write_jsonl`], kept as a separate function since the two files are
/// written to different paths by a driver and must never be interleaved.
/// Returns the SHA-256 digest of exactly the bytes written (not a
/// re-serialization computed separately), so a manifest's
/// `target_group_index_sha256` can never drift from what's actually on
/// disk.
pub fn write_target_pool_jsonl<W: Write>(
    records: &[TargetPoolRecord],
    mut writer: W,
) -> anyhow::Result<String> {
    validate_group_index(records)?;
    let mut hasher = Sha256::new();
    for record in records {
        let line = serde_json::to_vec(record)?;
        hasher.update(&line);
        hasher.update(b"\n");
        writer.write_all(&line)?;
        writer.write_all(b"\n")?;
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

/// Write `rows` as JSONL (one compact JSON object per line) to `writer`.
/// Validates every row first (see [`validate_candidate_rows`]) -- a bad row
/// is a hard error, never a partial write. Returns the SHA-256 digest of
/// exactly the bytes written, for the same reason as
/// [`write_target_pool_jsonl`].
pub fn write_jsonl<W: Write>(rows: &[CandidateRow], mut writer: W) -> anyhow::Result<String> {
    validate_candidate_rows(rows)?;
    let mut hasher = Sha256::new();
    for row in rows {
        let line = serde_json::to_vec(row)?;
        hasher.update(&line);
        hasher.update(b"\n");
        writer.write_all(&line)?;
        writer.write_all(b"\n")?;
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

/// Content hash over a rules set: sorted by `template_id`, each rule framed
/// as (template_id, name, smirks, weight-bits, required_elements) with
/// length-prefixed fields -- the same unambiguous framing used for
/// candidate identity (see `candidate::hash_string_sequence`), so two
/// different rule sets can never collide onto the same hash. `name` is
/// included (not just `template_id`/`smirks`) so a rename alone still
/// changes the hash -- a manifest's whole purpose is to let a consumer
/// detect "this isn't the rule set I think it is", and a silent rename
/// would be exactly the kind of drift that should be visible. `weight` is
/// hashed via its IEEE-754 bit pattern (`to_bits`), not its `Display`
/// string, so the hash is exact rather than dependent on float-formatting
/// precision.
pub fn rules_content_hash(rules: &[RetroRule]) -> String {
    let mut sorted: Vec<&RetroRule> = rules.iter().collect();
    sorted.sort_by(|a, b| a.template_id.cmp(&b.template_id));

    let mut hasher = Sha256::new();
    hasher.update(b"renkin-retrospect-rules-v2\0");
    hasher.update((sorted.len() as u64).to_be_bytes());
    for rule in sorted {
        for field in [
            rule.template_id.as_str(),
            rule.name.as_str(),
            rule.smirks.as_str(),
        ] {
            hasher.update((field.len() as u64).to_be_bytes());
            hasher.update(field.as_bytes());
        }
        hasher.update(rule.weight.to_bits().to_be_bytes());
        hasher.update(rule.required_elements.to_be_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}

/// Provenance this crate has no way to derive itself -- git/build state,
/// dependency versions, and the driver's own original input are all outside
/// this library's boundary (see `stock_identity`'s doc: this module never
/// guesses or shells out for provenance it wasn't handed). The caller (a
/// driver script or binary) is responsible for supplying real values before
/// a pool is used for anything beyond a local smoke test; `Default`
/// produces obviously-placeholder values (empty strings, `false`,
/// `Value::Null`) rather than anything that could be mistaken for real
/// provenance.
#[derive(Debug, Clone, Serialize)]
pub struct PoolProvenance {
    pub renkin_git_commit: String,
    pub cargo_lock_sha256: String,
    pub chematic_version: String,
    /// Hash of the driver's own original target-list input (e.g. a dataset
    /// file of group_id/target_smiles pairs), independent of this crate's
    /// own group-index/candidate-JSONL output -- this module never sees
    /// that original input, so it can't hash it itself.
    pub target_input_sha256: String,
    /// Where the stock came from (e.g. a file path, or `"embedded_default"`)
    /// -- `None` when no stock was supplied. Distinct from
    /// `PoolManifest::stock_content_sha256`, which hashes what's actually IN
    /// the stock regardless of where it came from.
    pub stock_source: Option<String>,
    /// True if stock loading fell back to a bundled/embedded default
    /// compound list rather than using a caller-requested external one --
    /// this crate's own stock loading (`ChemEnv::load`/`in_memory`) has no
    /// concept of "fallback" itself, so only the driver that chose to fall
    /// back can report this.
    pub embedded_fallback_used: bool,
    /// Opaque summary of whatever export configuration the driver used
    /// (e.g. its own CLI arguments) -- this module doesn't know the shape
    /// of its caller's configuration surface, so it never tries to
    /// reconstruct one.
    pub export_config: serde_json::Value,
}

impl Default for PoolProvenance {
    fn default() -> Self {
        Self {
            renkin_git_commit: String::new(),
            cargo_lock_sha256: String::new(),
            chematic_version: String::new(),
            target_input_sha256: String::new(),
            stock_source: None,
            embedded_fallback_used: false,
            export_config: serde_json::Value::Null,
        }
    }
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
    /// SHA-256 over `feature_schema_version` + `feature_names` (see
    /// `candidate::feature_schema_hash`) -- catches a same-length rename or
    /// reorder that a plain `feature_names` comparison could still miss if
    /// a consumer only checked its length.
    pub feature_schema_hash: String,
    pub proposal_mode: ProposalModeSummary,
    pub rules_content_hash: String,
    pub rules_count: usize,
    /// `None` when no stock was supplied to feature extraction (every
    /// stock-dependent feature is `missing` in every row in that case).
    pub stock_identity: Option<String>,
    pub stock_compound_count: Option<usize>,
    /// Hash of the stock's actual compound content (see
    /// `ChemEnv::content_sha256`) -- distinct from `stock_identity`, which
    /// is just a caller-supplied label and can't itself detect a silently
    /// swapped or truncated stock under an unchanged label.
    pub stock_content_sha256: Option<String>,
    /// Derived from (and asserted consistent with) the target/group index,
    /// never taken unchecked from caller input -- see `build_manifest`.
    pub target_count: usize,
    pub group_count: usize,
    pub candidate_count: usize,
    /// SHA-256 of the exact candidate JSONL bytes written by [`write_jsonl`]
    /// for this export.
    pub candidate_jsonl_sha256: String,
    /// SHA-256 of the exact target/group-index JSONL bytes written by
    /// [`write_target_pool_jsonl`] for this export.
    pub target_group_index_sha256: String,
    pub provenance: PoolProvenance,
}

/// Bumped to 2 for the Commit-4 manifest shape change (five new required
/// fields, plus `rules_content_hash`'s algorithm change) -- a v1 and a v2
/// manifest must never be silently treated as the same shape by a consumer.
pub const MANIFEST_SCHEMA_VERSION: u32 = 2;

/// Build the manifest for an export spanning `rows`, cross-validated
/// against `target_pool_records` (the same group/target index written by
/// [`write_target_pool_jsonl`]) -- `target_count`/`group_count` are derived
/// from this index, never taken unchecked from a caller-supplied number,
/// and every `group_id` in `rows` must have a consistent entry in the
/// index (see `validate_rows_consistent_with_group_index`). `stock_identity`
/// is a caller-supplied label (e.g. a file path, or `"in_memory"` for a
/// synthetic/test stock) -- this module never guesses one, since a wrong or
/// silently-substituted stock identity is exactly the kind of provenance
/// gap this manifest exists to close (see this repo's own history of a
/// benchmark run silently falling back to a smaller stock than intended);
/// `stock_content_sha256` is computed here from the stock itself, so a
/// swapped stock under an unchanged label is still detectable.
///
/// `candidate_jsonl_sha256`/`target_group_index_sha256` must be the digests
/// [`write_jsonl`]/[`write_target_pool_jsonl`] actually returned for this
/// export, not independently recomputed -- passing anything else would
/// defeat the point of hashing the bytes that were actually written.
#[allow(clippy::too_many_arguments)]
pub fn build_manifest(
    rows: &[CandidateRow],
    candidate_jsonl_sha256: &str,
    target_pool_records: &[TargetPoolRecord],
    target_group_index_sha256: &str,
    rules: &[RetroRule],
    mode: &ProposalMode,
    stock: Option<(&str, &ChemEnv)>,
    provenance: PoolProvenance,
) -> anyhow::Result<PoolManifest> {
    validate_group_index(target_pool_records)?;
    validate_rows_consistent_with_group_index(rows, target_pool_records)?;

    let target_ids: std::collections::HashSet<&str> = target_pool_records
        .iter()
        .map(|r| r.target_id.as_str())
        .collect();

    Ok(PoolManifest {
        manifest_schema_version: MANIFEST_SCHEMA_VERSION,
        feature_schema_version: FEATURE_SCHEMA_VERSION,
        feature_names: FEATURE_NAMES_V1.to_vec(),
        feature_schema_hash: feature_schema_hash(),
        proposal_mode: ProposalModeSummary::from_mode(mode),
        rules_content_hash: rules_content_hash(rules),
        rules_count: rules.len(),
        stock_identity: stock.map(|(id, _)| id.to_string()),
        stock_compound_count: stock.map(|(_, env)| env.bb_count()),
        stock_content_sha256: stock.map(|(_, env)| env.content_sha256()),
        target_count: target_ids.len(),
        group_count: target_pool_records.len(),
        candidate_count: rows.len(),
        candidate_jsonl_sha256: candidate_jsonl_sha256.to_string(),
        target_group_index_sha256: target_group_index_sha256.to_string(),
        provenance,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::{ProposalConfig, index_rules_by_template_id, propose_one_step};
    use crate::chem_env::{default_rules, mol_from_smiles};

    #[test]
    fn candidate_rows_carry_group_id_from_pool() {
        let rules = default_rules();
        let target = "CC(=O)c1ccccc1";
        let target_mol = mol_from_smiles(target).unwrap();
        let pool =
            propose_one_step("rxn-example-42", target, &rules, &ProposalConfig::default()).unwrap();
        let templates_by_id = index_rules_by_template_id(&rules).unwrap();
        let rows = candidate_rows_for_pool(&pool, &target_mol, &templates_by_id, None);
        assert!(!rows.is_empty());
        for row in &rows {
            assert_eq!(row.group_id, "rxn-example-42");
            assert_eq!(row.target_id, pool.target_id);
        }
    }

    #[test]
    fn target_pool_record_for_pool_reports_real_candidate_count() {
        let rules = default_rules();
        let target = "CC(=O)c1ccccc1";
        let pool =
            propose_one_step("rxn-example-1", target, &rules, &ProposalConfig::default()).unwrap();
        let record = target_pool_record_for_pool(&pool);
        assert_eq!(record.group_id, "rxn-example-1");
        assert_eq!(record.target_id, pool.target_id);
        assert_eq!(record.candidate_count, pool.candidates.len());
        assert_eq!(record.proposal_status, ProposalStatus::Ok);
    }

    #[test]
    fn target_pool_record_for_pool_still_emits_a_record_with_zero_candidates() {
        // A target that genuinely has no one-step disconnections under this
        // rule set is a real coverage gap, not an error -- it still gets
        // exactly one record, with candidate_count == 0 and status Ok (not
        // TargetParseFailed, which is reserved for parse failure).
        let rules = vec![RetroRule {
            name: "unreachable".to_string(),
            template_id: "rule:unreachable".to_string(),
            smirks: "[Xe:1]>>[Xe:1]".to_string(),
            weight: 1.0,
            required_elements: 0,
        }];
        let pool =
            propose_one_step("rxn-example-2", "CCO", &rules, &ProposalConfig::default()).unwrap();
        assert_eq!(pool.candidates.len(), 0);
        let record = target_pool_record_for_pool(&pool);
        assert_eq!(record.candidate_count, 0);
        assert_eq!(record.proposal_status, ProposalStatus::Ok);
    }

    #[test]
    fn target_pool_record_for_failure_is_distinguishable_from_a_real_zero_candidate_outcome() {
        let record = target_pool_record_for_failure("rxn-example-3", "not-a-valid-smiles(((");
        assert_eq!(record.group_id, "rxn-example-3");
        assert_eq!(record.candidate_count, 0);
        assert_eq!(record.proposal_status, ProposalStatus::TargetParseFailed);
        assert_ne!(
            record.proposal_status,
            ProposalStatus::Ok,
            "a parse failure must never be reported as a real zero-candidate outcome"
        );
    }

    #[test]
    fn write_target_pool_jsonl_is_one_valid_json_object_per_line() {
        let records = vec![
            TargetPoolRecord {
                group_id: "g1".to_string(),
                target_id: "t1".to_string(),
                target_smiles: "t1".to_string(),
                candidate_count: 3,
                proposal_status: ProposalStatus::Ok,
            },
            target_pool_record_for_failure("g2", "bad-smiles"),
        ];
        let mut buf: Vec<u8> = Vec::new();
        write_target_pool_jsonl(&records, &mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        for line in &lines {
            let parsed: serde_json::Value =
                serde_json::from_str(line).expect("each line must be valid JSON");
            assert!(parsed.is_object());
        }
        let second: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["proposal_status"], "target_parse_failed");
    }

    #[test]
    fn candidate_rows_are_sorted_by_candidate_id() {
        let rules = default_rules();
        let target = "CC(=O)c1ccccc1";
        let target_mol = mol_from_smiles(target).unwrap();
        let pool = propose_one_step("group:1", target, &rules, &ProposalConfig::default()).unwrap();
        let templates_by_id = index_rules_by_template_id(&rules).unwrap();

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
        let pool = propose_one_step("group:1", target, &rules, &ProposalConfig::default()).unwrap();
        let templates_by_id = index_rules_by_template_id(&rules).unwrap();

        let rows = candidate_rows_for_pool(&pool, &target_mol, &templates_by_id, None);
        for row in &rows {
            assert_eq!(row.feature_schema_version, FEATURE_SCHEMA_VERSION);
            assert_eq!(row.feature_values.len(), FEATURE_NAMES_V1.len());
            assert_eq!(row.feature_missing.len(), FEATURE_NAMES_V1.len());
        }
    }

    #[test]
    fn candidate_rows_export_full_source_provenance_including_scorer_status() {
        let rules = default_rules();
        let target = "CC(=O)c1ccccc1";
        let target_mol = mol_from_smiles(target).unwrap();
        let pool = propose_one_step("group:1", target, &rules, &ProposalConfig::default()).unwrap();
        let templates_by_id = index_rules_by_template_id(&rules).unwrap();

        let rows = candidate_rows_for_pool(&pool, &target_mol, &templates_by_id, None);
        assert!(!rows.is_empty());
        for row in &rows {
            assert_eq!(row.sources.len(), row.source_template_count);
            for source in &row.sources {
                assert!(!source.template_id.is_empty());
                assert!(!source.rule_name.is_empty());
                // Exhaustive mode: every source's scorer status must be
                // exported and must be NotApplicable (no scorer involved).
                assert_eq!(
                    source.upstream_score_status,
                    UpstreamScoreStatus::NotApplicable
                );
            }
        }
    }

    #[test]
    fn write_jsonl_is_one_valid_json_object_per_line() {
        let rules = default_rules();
        let target = "CC(=O)c1ccccc1";
        let target_mol = mol_from_smiles(target).unwrap();
        let pool = propose_one_step("group:1", target, &rules, &ProposalConfig::default()).unwrap();
        let templates_by_id = index_rules_by_template_id(&rules).unwrap();
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
        let templates_by_id = index_rules_by_template_id(&rules).unwrap();

        let mut outputs = Vec::new();
        for _ in 0..2 {
            let pool =
                propose_one_step("group:1", target, &rules, &ProposalConfig::default()).unwrap();
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

    fn export_pool_with_group_index(
        pool: &CandidatePool,
        target_mol: &Molecule,
        templates_by_id: &HashMap<String, &RetroRule>,
    ) -> (Vec<CandidateRow>, String, Vec<TargetPoolRecord>, String) {
        let rows = candidate_rows_for_pool(pool, target_mol, templates_by_id, None);
        let records = vec![target_pool_record_for_pool(pool)];
        let mut pool_buf: Vec<u8> = Vec::new();
        let candidate_hash = write_jsonl(&rows, &mut pool_buf).unwrap();
        let mut group_buf: Vec<u8> = Vec::new();
        let group_hash = write_target_pool_jsonl(&records, &mut group_buf).unwrap();
        (rows, candidate_hash, records, group_hash)
    }

    #[test]
    fn manifest_records_mode_rules_and_stock_identity() {
        let rules = default_rules();
        let target = "CC(=O)c1ccccc1";
        let target_mol = mol_from_smiles(target).unwrap();
        let pool = propose_one_step("group:1", target, &rules, &ProposalConfig::default()).unwrap();
        let templates_by_id = index_rules_by_template_id(&rules).unwrap();
        let (rows, candidate_hash, records, group_hash) =
            export_pool_with_group_index(&pool, &target_mol, &templates_by_id);

        let manifest = build_manifest(
            &rows,
            &candidate_hash,
            &records,
            &group_hash,
            &rules,
            &ProposalMode::Exhaustive,
            None,
            PoolProvenance::default(),
        )
        .unwrap();
        assert_eq!(manifest.manifest_schema_version, MANIFEST_SCHEMA_VERSION);
        assert_eq!(manifest.feature_schema_version, FEATURE_SCHEMA_VERSION);
        assert_eq!(manifest.feature_names.len(), FEATURE_NAMES_V1.len());
        assert_eq!(manifest.feature_schema_hash, feature_schema_hash());
        assert_eq!(
            manifest.proposal_mode,
            ProposalModeSummary {
                mode: "exhaustive",
                top_k: None,
                rules_offset: None,
                scorer_identity: None,
                scorer_model_sha256: None,
                scorer_status: None,
            }
        );
        assert_eq!(manifest.rules_count, rules.len());
        assert_eq!(manifest.rules_content_hash, rules_content_hash(&rules));
        assert!(manifest.stock_identity.is_none());
        assert!(manifest.stock_compound_count.is_none());
        assert!(manifest.stock_content_sha256.is_none());
        assert_eq!(manifest.target_count, 1);
        assert_eq!(manifest.group_count, 1);
        assert_eq!(manifest.candidate_count, rows.len());
        assert_eq!(manifest.candidate_jsonl_sha256, candidate_hash);
        assert_eq!(manifest.target_group_index_sha256, group_hash);

        let stock_a = ChemEnv::in_memory(&["CCO", "CC(=O)O"]);
        let manifest_with_stock = build_manifest(
            &rows,
            &candidate_hash,
            &records,
            &group_hash,
            &rules,
            &ProposalMode::BondIndexed { top_k: 5 },
            Some(("in_memory:test", &stock_a)),
            PoolProvenance::default(),
        )
        .unwrap();
        assert_eq!(
            manifest_with_stock.stock_identity,
            Some("in_memory:test".to_string())
        );
        assert_eq!(manifest_with_stock.stock_compound_count, Some(2));
        assert_eq!(
            manifest_with_stock.stock_content_sha256,
            Some(stock_a.content_sha256())
        );
        assert_eq!(
            manifest_with_stock.proposal_mode,
            ProposalModeSummary {
                mode: "bond_indexed",
                top_k: Some(5),
                rules_offset: None,
                scorer_identity: None,
                scorer_model_sha256: None,
                scorer_status: None,
            }
        );

        // A different stock's content under the SAME caller-supplied label
        // must still be distinguishable -- `stock_identity` alone can't
        // catch a silent swap.
        let stock_b = ChemEnv::in_memory(&["CCN"]);
        let manifest_with_swapped_stock = build_manifest(
            &rows,
            &candidate_hash,
            &records,
            &group_hash,
            &rules,
            &ProposalMode::BondIndexed { top_k: 5 },
            Some(("in_memory:test", &stock_b)),
            PoolProvenance::default(),
        )
        .unwrap();
        assert_ne!(
            manifest_with_stock.stock_content_sha256,
            manifest_with_swapped_stock.stock_content_sha256,
            "a swapped stock under an unchanged label must still change the content hash"
        );
    }

    #[test]
    fn manifest_serializes_to_json() {
        let rules = default_rules();
        let manifest = build_manifest(
            &[],
            "sha256:empty",
            &[],
            "sha256:empty",
            &rules,
            &ProposalMode::Exhaustive,
            None,
            PoolProvenance::default(),
        )
        .unwrap();
        let json = serde_json::to_string(&manifest).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["feature_names"].is_array());
        assert_eq!(parsed["proposal_mode"]["mode"], "exhaustive");
    }

    #[test]
    fn manifest_records_scorer_conditioned_provenance() {
        let mut rules = default_rules();
        let n_handcrafted = rules.len();
        rules.push(RetroRule {
            name: "extracted_0".to_string(),
            template_id: "smirks-sha256:fake0".to_string(),
            smirks: "[C:1][C:2]>>[C:1].[C:2]".to_string(),
            weight: 3.0,
            required_elements: 0,
        });
        let mode = ProposalMode::ScorerConditioned {
            input: crate::candidate::ScorerConditionedInput {
                scores: vec![(n_handcrafted, 0.9, 0)],
                status: UpstreamScoreStatus::Available,
                rules_offset: n_handcrafted,
                scorer_identity: "test-scorer-v1".to_string(),
                scorer_model_sha256: "sha256:modelbytes".to_string(),
            },
            top_k: 1,
        };
        let manifest = build_manifest(
            &[],
            "sha256:empty",
            &[],
            "sha256:empty",
            &rules,
            &mode,
            None,
            PoolProvenance::default(),
        )
        .unwrap();
        assert_eq!(manifest.proposal_mode.mode, "scorer_conditioned");
        assert_eq!(manifest.proposal_mode.rules_offset, Some(n_handcrafted));
        assert_eq!(
            manifest.proposal_mode.scorer_identity,
            Some("test-scorer-v1".to_string())
        );
        assert_eq!(
            manifest.proposal_mode.scorer_model_sha256,
            Some("sha256:modelbytes".to_string())
        );
        assert_eq!(
            manifest.proposal_mode.scorer_status,
            Some(UpstreamScoreStatus::Available)
        );
    }

    #[test]
    fn manifest_records_embedded_fallback_used_provenance() {
        // PoolProvenance::default() (used by every other manifest test in
        // this file) leaves embedded_fallback_used at its default (false),
        // which alone can't distinguish "correctly recorded false" from
        // "field never wired through at all". Set it true explicitly and
        // confirm it survives build_manifest and JSON serialization.
        let rules = default_rules();
        let provenance = PoolProvenance {
            embedded_fallback_used: true,
            ..PoolProvenance::default()
        };
        let manifest = build_manifest(
            &[],
            "sha256:empty",
            &[],
            "sha256:empty",
            &rules,
            &ProposalMode::Exhaustive,
            None,
            provenance,
        )
        .unwrap();
        assert!(manifest.provenance.embedded_fallback_used);

        let json = serde_json::to_string(&manifest).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["provenance"]["embedded_fallback_used"], true);
    }

    #[test]
    fn build_manifest_rejects_group_id_missing_from_group_index() {
        let rules = default_rules();
        let target = "CC(=O)c1ccccc1";
        let target_mol = mol_from_smiles(target).unwrap();
        let pool = propose_one_step("group:1", target, &rules, &ProposalConfig::default()).unwrap();
        let templates_by_id = index_rules_by_template_id(&rules).unwrap();
        let rows = candidate_rows_for_pool(&pool, &target_mol, &templates_by_id, None);
        assert!(!rows.is_empty());

        // Group index deliberately omits "group:1".
        let result = build_manifest(
            &rows,
            "sha256:whatever",
            &[],
            "sha256:whatever",
            &rules,
            &ProposalMode::Exhaustive,
            None,
            PoolProvenance::default(),
        );
        assert!(
            result.is_err(),
            "a group_id present in rows but absent from the group index must be a hard error"
        );
    }

    #[test]
    fn build_manifest_rejects_duplicate_group_id_in_group_index() {
        let rules = default_rules();
        let records = vec![
            TargetPoolRecord {
                group_id: "g1".to_string(),
                target_id: "t1".to_string(),
                target_smiles: "t1".to_string(),
                candidate_count: 0,
                proposal_status: ProposalStatus::Ok,
            },
            TargetPoolRecord {
                group_id: "g1".to_string(),
                target_id: "t2".to_string(),
                target_smiles: "t2".to_string(),
                candidate_count: 0,
                proposal_status: ProposalStatus::Ok,
            },
        ];
        let result = build_manifest(
            &[],
            "sha256:empty",
            &records,
            "sha256:whatever",
            &rules,
            &ProposalMode::Exhaustive,
            None,
            PoolProvenance::default(),
        );
        assert!(
            result.is_err(),
            "a duplicate group_id in the group index must be a hard error"
        );
    }

    #[test]
    fn build_manifest_rejects_candidate_count_mismatch() {
        let rules = default_rules();
        let target = "CC(=O)c1ccccc1";
        let target_mol = mol_from_smiles(target).unwrap();
        let pool = propose_one_step("group:1", target, &rules, &ProposalConfig::default()).unwrap();
        let templates_by_id = index_rules_by_template_id(&rules).unwrap();
        let rows = candidate_rows_for_pool(&pool, &target_mol, &templates_by_id, None);
        assert!(
            rows.len() > 1,
            "fixture must have more than one candidate for this check to bite"
        );

        let mut record = target_pool_record_for_pool(&pool);
        record.candidate_count += 1; // claims one more row than actually exists
        let result = build_manifest(
            &rows,
            "sha256:whatever",
            &[record],
            "sha256:whatever",
            &rules,
            &ProposalMode::Exhaustive,
            None,
            PoolProvenance::default(),
        );
        assert!(
            result.is_err(),
            "a group index candidate_count that disagrees with the actual row count must be rejected"
        );
    }

    #[test]
    fn write_jsonl_rejects_duplicate_candidate_id_within_one_group() {
        let base = CandidateRow {
            group_id: "g1".to_string(),
            target_id: "t1".to_string(),
            target_smiles: "t1".to_string(),
            candidate_id: "same-id".to_string(),
            precursor_smiles: vec!["CCO".to_string()],
            source_template_count: 1,
            best_upstream_rank: 0,
            sources: vec![SourceRow {
                template_id: "rule:x".to_string(),
                rule_name: "x".to_string(),
                original_rank: 0,
                upstream_score: None,
                upstream_score_status: UpstreamScoreStatus::NotApplicable,
                template_log_frequency_raw: None,
                base_step_cost: 1.0,
            }],
            feature_schema_version: FEATURE_SCHEMA_VERSION,
            feature_values: vec![0.0; FEATURE_NAMES_V1.len()],
            feature_missing: vec![true; FEATURE_NAMES_V1.len()],
        };
        let rows = vec![base.clone(), base];
        let mut buf: Vec<u8> = Vec::new();
        assert!(write_jsonl(&rows, &mut buf).is_err());
    }

    #[test]
    fn write_jsonl_allows_same_candidate_id_across_different_groups() {
        // Same target_id, different group_id -- a legitimate case (two
        // dataset examples sharing a product), must not be rejected.
        let mut row_a = CandidateRow {
            group_id: "g1".to_string(),
            target_id: "t1".to_string(),
            target_smiles: "t1".to_string(),
            candidate_id: "same-id".to_string(),
            precursor_smiles: vec!["CCO".to_string()],
            source_template_count: 1,
            best_upstream_rank: 0,
            sources: vec![SourceRow {
                template_id: "rule:x".to_string(),
                rule_name: "x".to_string(),
                original_rank: 0,
                upstream_score: None,
                upstream_score_status: UpstreamScoreStatus::NotApplicable,
                template_log_frequency_raw: None,
                base_step_cost: 1.0,
            }],
            feature_schema_version: FEATURE_SCHEMA_VERSION,
            feature_values: vec![0.0; FEATURE_NAMES_V1.len()],
            feature_missing: vec![true; FEATURE_NAMES_V1.len()],
        };
        let mut row_b = row_a.clone();
        row_b.group_id = "g2".to_string();
        row_a.candidate_id = "same-id".to_string();
        row_b.candidate_id = "same-id".to_string();

        let mut buf: Vec<u8> = Vec::new();
        assert!(write_jsonl(&[row_a, row_b], &mut buf).is_ok());
    }

    #[test]
    fn write_jsonl_rejects_malformed_feature_vector_lengths() {
        let mut row = CandidateRow {
            group_id: "g1".to_string(),
            target_id: "t1".to_string(),
            target_smiles: "t1".to_string(),
            candidate_id: "id-1".to_string(),
            precursor_smiles: vec!["CCO".to_string()],
            source_template_count: 1,
            best_upstream_rank: 0,
            sources: vec![SourceRow {
                template_id: "rule:x".to_string(),
                rule_name: "x".to_string(),
                original_rank: 0,
                upstream_score: None,
                upstream_score_status: UpstreamScoreStatus::NotApplicable,
                template_log_frequency_raw: None,
                base_step_cost: 1.0,
            }],
            feature_schema_version: FEATURE_SCHEMA_VERSION,
            feature_values: vec![0.0; FEATURE_NAMES_V1.len() - 1],
            feature_missing: vec![true; FEATURE_NAMES_V1.len()],
        };
        let mut buf: Vec<u8> = Vec::new();
        assert!(write_jsonl(std::slice::from_ref(&row), &mut buf).is_err());

        row.feature_values = vec![0.0; FEATURE_NAMES_V1.len()];
        row.feature_missing = vec![false; FEATURE_NAMES_V1.len()];
        row.feature_values[0] = f32::NAN;
        let mut buf2: Vec<u8> = Vec::new();
        assert!(
            write_jsonl(&[row], &mut buf2).is_err(),
            "a non-finite value not marked missing must be rejected"
        );
    }

    #[test]
    fn write_jsonl_rejects_empty_precursor_or_source_lists() {
        let mut row = CandidateRow {
            group_id: "g1".to_string(),
            target_id: "t1".to_string(),
            target_smiles: "t1".to_string(),
            candidate_id: "id-1".to_string(),
            precursor_smiles: vec![],
            source_template_count: 1,
            best_upstream_rank: 0,
            sources: vec![SourceRow {
                template_id: "rule:x".to_string(),
                rule_name: "x".to_string(),
                original_rank: 0,
                upstream_score: None,
                upstream_score_status: UpstreamScoreStatus::NotApplicable,
                template_log_frequency_raw: None,
                base_step_cost: 1.0,
            }],
            feature_schema_version: FEATURE_SCHEMA_VERSION,
            feature_values: vec![0.0; FEATURE_NAMES_V1.len()],
            feature_missing: vec![true; FEATURE_NAMES_V1.len()],
        };
        let mut buf: Vec<u8> = Vec::new();
        assert!(write_jsonl(std::slice::from_ref(&row), &mut buf).is_err());

        row.precursor_smiles = vec!["CCO".to_string()];
        row.sources = vec![];
        let mut buf2: Vec<u8> = Vec::new();
        assert!(write_jsonl(&[row], &mut buf2).is_err());
    }

    #[test]
    fn write_functions_return_the_digest_of_exactly_what_they_wrote() {
        let rules = default_rules();
        let target = "CC(=O)c1ccccc1";
        let target_mol = mol_from_smiles(target).unwrap();
        let pool = propose_one_step("group:1", target, &rules, &ProposalConfig::default()).unwrap();
        let templates_by_id = index_rules_by_template_id(&rules).unwrap();
        let rows = candidate_rows_for_pool(&pool, &target_mol, &templates_by_id, None);

        let mut buf: Vec<u8> = Vec::new();
        let returned_hash = write_jsonl(&rows, &mut buf).unwrap();
        let mut hasher = Sha256::new();
        hasher.update(&buf);
        let recomputed = format!("sha256:{:x}", hasher.finalize());
        assert_eq!(returned_hash, recomputed);
    }
}
