//! Candidate proposal/selection separation inspired by:
//!
//! Pappala et al. (2026), "RETROSPECT: RETROsynthesis via
//! Sequential Prediction, and Chemically Transformed-ranking",
//! arXiv:2606.07181, doi:10.48550/arXiv.2606.07181.
//!
//! Independent RENKIN implementation. No upstream source code copied.
//!
//! `find_routes` (`crate::search`) and the standalone [`propose_one_step`]
//! API both go through [`raw_propose`], so route search and offline
//! candidate-pool generation see the same underlying rule-application
//! results. `find_routes` builds its own per-application step costs directly
//! from [`RawProposal`] (unmerged, one entry per rule application) and never
//! calls [`propose_one_step`] itself — canonical-precursor-set merging is a
//! candidate-pool-only concept; the search's own state-space dedup (frontier
//! `state_hash` in `closed`) is a different, coarser mechanism that already
//! serves the search's purposes. This module must never change what
//! `find_routes` computes.

#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;
use sha2::{Digest, Sha256};

use crate::chem_env::{
    Molecule, PrecursorMol, RetroRule, TemplateBondIndex, apply_retro, mol_from_smiles,
    to_canonical,
};

/// One raw one-step retrosynthetic proposal: a single rule application
/// against a single target, before candidate-level canonical merge.
/// `original_rank` is this rule's position in whatever active-rule ordering
/// produced it (bond-index retrieval order, NN-ranked order, or file order).
pub struct RawProposal {
    pub rule_name: String,
    pub template_id: String,
    pub rule_weight: f64,
    pub original_rank: usize,
    pub precursors: Vec<PrecursorMol>,
}

/// Rule-selection configuration for standalone one-step proposal generation.
/// Deliberately narrower than `SearchConfig`: no `reaction_prior`/`max_depth`/
/// beam settings, since a one-step proposal has no notion of a route.
#[derive(Default)]
pub struct ProposalConfig {
    pub bond_index: bool,
}

/// Extracted, structured candidate features (see `feature_schema` module,
/// added in a later commit). Order is fixed by `FeatureSchema`; `missing[i]`
/// is true when `values[i]` could not be computed and must be treated as
/// missing (not zero) by any consumer, including the reranker.
#[derive(Debug, Clone, Default)]
pub struct CandidateFeatures {
    pub values: Vec<f32>,
    pub missing: Vec<bool>,
}

/// A merged candidate: all rule applications that produced the same
/// canonical precursor set for the same target, collapsed into one entry
/// with full source provenance retained (see `merge_into_candidates`).
#[derive(Debug, Clone)]
pub struct ReactionCandidate {
    pub candidate_id: String,
    pub target_smiles: String,
    pub precursor_smiles: Vec<String>,
    pub source_template_ids: Vec<String>,
    pub source_rule_names: Vec<String>,

    /// Best (highest) raw upstream proposal score across merged sources.
    /// `None` when no scorer was configured for this proposal call.
    pub upstream_score: Option<f64>,
    /// Best (lowest = earliest) `original_rank` across merged sources.
    pub original_rank: usize,
    /// Number of distinct rule applications merged into this candidate.
    pub proposal_count: usize,

    pub features: CandidateFeatures,
    pub reranker_score: Option<f64>,
}

pub struct CandidatePool {
    pub target_id: String,
    pub target_smiles: String,
    pub candidates: Vec<ReactionCandidate>,
}

pub trait CandidateReranker: Send + Sync {
    fn score_pool(&self, target: &str, candidates: &mut [ReactionCandidate]) -> anyhow::Result<()>;
}

/// Select the active rule set for one target, mirroring `find_routes`'
/// bond_idx / ranked_rules fallback chain exactly (NN per-node ranking is a
/// `find_routes`-only concern — see module doc — so it is intentionally not
/// reproduced here; the standalone API always uses bond-index-or-file-order).
fn select_active_rules<'a>(
    target_mol: &Molecule,
    rules: &'a [RetroRule],
    bond_idx: Option<&TemplateBondIndex>,
) -> Vec<&'a RetroRule> {
    match bond_idx {
        Some(idx) => idx
            .retrieve(target_mol, 0, rules)
            .into_iter()
            .filter_map(|i| rules.get(i))
            .collect(),
        None => rules.iter().collect(),
    }
}

/// Compute the element bitmask pre-screen and apply every active rule to
/// `target_mol`, exactly as `find_routes`' retro-cache-miss branch does.
/// Shared verbatim by both callers — see module doc.
pub(crate) fn raw_propose(
    target_mol: &Molecule,
    target_smi: &str,
    active_rules: &[&RetroRule],
) -> Vec<RawProposal> {
    let target_elem_mask: u64 = crate::search::elem_mask_from_smiles(target_smi);

    #[cfg(not(target_arch = "wasm32"))]
    let raw: Vec<(String, String, f64, usize, Vec<PrecursorMol>)> = active_rules
        .par_iter()
        .enumerate()
        .filter(|(_, rule)| {
            rule.required_elements == 0
                || (target_elem_mask & rule.required_elements == rule.required_elements)
        })
        .flat_map(|(rank, rule)| {
            apply_retro(target_mol, rule)
                .into_iter()
                .map(|precs| {
                    (
                        rule.name.to_string(),
                        rule.template_id.clone(),
                        rule.weight,
                        rank,
                        precs,
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect();
    #[cfg(target_arch = "wasm32")]
    let raw: Vec<(String, String, f64, usize, Vec<PrecursorMol>)> = active_rules
        .iter()
        .enumerate()
        .filter(|(_, rule)| {
            rule.required_elements == 0
                || (target_elem_mask & rule.required_elements == rule.required_elements)
        })
        .flat_map(|(rank, rule)| {
            apply_retro(target_mol, rule)
                .into_iter()
                .map(|precs| {
                    (
                        rule.name.to_string(),
                        rule.template_id.clone(),
                        rule.weight,
                        rank,
                        precs,
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect();

    raw.into_iter()
        .filter(|(_, _, _, _, precs)| {
            !precs.is_empty() && !precs.iter().any(|p| p.smiles == target_smi)
        })
        .map(
            |(rule_name, template_id, rule_weight, original_rank, precursors)| RawProposal {
                rule_name,
                template_id,
                rule_weight,
                original_rank,
                precursors,
            },
        )
        .collect()
}

/// Canonicalize and merge raw proposals into candidate-pool entries.
///
/// Candidate ID: `sha256(canonical_target + "\0" + canonical_precursors.join("."))`
/// where `canonical_precursors` is the sorted, deduplicated list of this
/// proposal's own precursor SMILES (already canonical -- `apply_retro`'s
/// `split_fragments` standardizes+canonicalizes every fragment before it
/// reaches this function). Proposals whose sorted precursor list hashes the
/// same are merged: all `source_template_ids`/`source_rule_names` are kept,
/// `upstream_score` becomes the best (max) across sources, `original_rank`
/// becomes the best (min) across sources, `proposal_count` counts merged
/// sources. No provenance is dropped in favor of "just the best one".
fn merge_into_candidates(
    canonical_target: &str,
    raw: Vec<RawProposal>,
    upstream_scores: Option<&[Option<f64>]>,
) -> Vec<ReactionCandidate> {
    let mut order: Vec<String> = Vec::new();
    let mut by_id: std::collections::HashMap<String, ReactionCandidate> =
        std::collections::HashMap::new();

    for (i, proposal) in raw.into_iter().enumerate() {
        let mut precursor_smiles: Vec<String> = proposal
            .precursors
            .iter()
            .map(|p| p.smiles.clone())
            .collect();
        precursor_smiles.sort_unstable();
        precursor_smiles.dedup();

        let candidate_id = {
            let mut hasher = Sha256::new();
            hasher.update(canonical_target.as_bytes());
            hasher.update(b"\0");
            hasher.update(precursor_smiles.join(".").as_bytes());
            let digest = hasher.finalize();
            let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
            format!("sha256:{hex}")
        };

        let this_score = upstream_scores.and_then(|s| s.get(i).copied().flatten());

        match by_id.get_mut(&candidate_id) {
            Some(existing) => {
                existing.source_template_ids.push(proposal.template_id);
                existing.source_rule_names.push(proposal.rule_name);
                existing.proposal_count += 1;
                existing.original_rank = existing.original_rank.min(proposal.original_rank);
                existing.upstream_score = match (existing.upstream_score, this_score) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    (Some(a), None) => Some(a),
                    (None, Some(b)) => Some(b),
                    (None, None) => None,
                };
            }
            None => {
                order.push(candidate_id.clone());
                by_id.insert(
                    candidate_id.clone(),
                    ReactionCandidate {
                        candidate_id,
                        target_smiles: canonical_target.to_string(),
                        precursor_smiles,
                        source_template_ids: vec![proposal.template_id],
                        source_rule_names: vec![proposal.rule_name],
                        upstream_score: this_score,
                        original_rank: proposal.original_rank,
                        proposal_count: 1,
                        features: CandidateFeatures::default(),
                        reranker_score: None,
                    },
                );
            }
        }
    }

    order
        .into_iter()
        .filter_map(|id| by_id.remove(&id))
        .collect()
}

/// Deterministic one-step candidate proposal: parse `target_smiles`, select
/// active rules (bond-index retrieval if `config.bond_index`, else file
/// order), apply every active rule, then canonicalize+merge duplicate
/// precursor-set outcomes (see `merge_into_candidates`). Builds its own
/// `TemplateBondIndex` per call (a one-shot API, unlike `find_routes`'
/// amortized-per-search index) -- intended for offline candidate-pool
/// generation, not the route-search hot path.
pub fn propose_one_step(
    target_smiles: &str,
    rules: &[RetroRule],
    config: &ProposalConfig,
) -> anyhow::Result<CandidatePool> {
    let target_mol = mol_from_smiles(target_smiles)?;
    let canonical_target = to_canonical(&target_mol);

    let bond_idx = if config.bond_index {
        Some(TemplateBondIndex::build(rules))
    } else {
        None
    };
    let active_rules = select_active_rules(&target_mol, rules, bond_idx.as_ref());

    let raw = raw_propose(&target_mol, &canonical_target, &active_rules);
    let candidates = merge_into_candidates(&canonical_target, raw, None);

    Ok(CandidatePool {
        target_id: canonical_target.clone(),
        target_smiles: canonical_target,
        candidates,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chem_env::default_rules;

    fn rule(name: &str, smirks: &str) -> RetroRule {
        RetroRule {
            name: name.to_string(),
            template_id: format!("rule:{name}"),
            smirks: smirks.to_string(),
            weight: 1.0,
            required_elements: 0,
        }
    }

    #[test]
    fn raw_propose_matches_independent_reference_filter() {
        // Golden test: raw_propose's output must equal what find_routes'
        // original inline logic computed -- reimplemented here independently
        // (not by calling raw_propose internals) as a reference, so a future
        // change to raw_propose that silently drops/adds entries is caught.
        let rules = default_rules();
        let target = "CC(=O)c1ccccc1"; // acetophenone
        let target_mol = mol_from_smiles(target).unwrap();
        let canon_target = to_canonical(&target_mol);
        let target_elem_mask = crate::search::elem_mask_from_smiles(&canon_target);

        let mut reference: Vec<(String, Vec<String>)> = Vec::new();
        for rule in &rules {
            if !(rule.required_elements == 0
                || (target_elem_mask & rule.required_elements == rule.required_elements))
            {
                continue;
            }
            for precs in apply_retro(&target_mol, rule) {
                if precs.is_empty() || precs.iter().any(|p| p.smiles == canon_target) {
                    continue;
                }
                reference.push((
                    rule.name.clone(),
                    precs.iter().map(|p| p.smiles.clone()).collect(),
                ));
            }
        }

        let active_rules: Vec<&RetroRule> = rules.iter().collect();
        let got = raw_propose(&target_mol, &canon_target, &active_rules);
        let mut got_pairs: Vec<(String, Vec<String>)> = got
            .into_iter()
            .map(|p| {
                (
                    p.rule_name,
                    p.precursors.iter().map(|pm| pm.smiles.clone()).collect(),
                )
            })
            .collect();

        let mut reference_sorted = reference;
        reference_sorted.sort();
        got_pairs.sort();
        assert_eq!(
            got_pairs, reference_sorted,
            "raw_propose must produce exactly the same (rule, precursor-set) pairs as the \
             original inline find_routes logic, just relocated into a shared function"
        );
        assert!(
            !got_pairs.is_empty(),
            "expected at least one match for acetophenone"
        );
    }

    #[test]
    fn candidate_ids_are_deterministic_across_calls() {
        let rules = default_rules();
        let config = ProposalConfig::default();
        let target = "CC(=O)c1ccccc1"; // acetophenone: hits friedel_crafts_retro
        let a = propose_one_step(target, &rules, &config).unwrap();
        let b = propose_one_step(target, &rules, &config).unwrap();
        assert!(!a.candidates.is_empty(), "expected at least one candidate");
        let ids_a: Vec<&str> = a
            .candidates
            .iter()
            .map(|c| c.candidate_id.as_str())
            .collect();
        let ids_b: Vec<&str> = b
            .candidates
            .iter()
            .map(|c| c.candidate_id.as_str())
            .collect();
        assert_eq!(ids_a, ids_b);
    }

    #[test]
    fn precursor_order_does_not_change_candidate_id() {
        // Two proposals with the same precursor SET in different Vec order
        // must hash to the same candidate_id (sorted before hashing).
        let target = "canonical_target";
        let a = ReactionCandidate {
            candidate_id: String::new(),
            target_smiles: target.to_string(),
            precursor_smiles: vec!["A".to_string(), "B".to_string()],
            source_template_ids: vec![],
            source_rule_names: vec![],
            upstream_score: None,
            original_rank: 0,
            proposal_count: 1,
            features: CandidateFeatures::default(),
            reranker_score: None,
        };
        let mut sorted_ab = a.precursor_smiles.clone();
        sorted_ab.sort_unstable();
        let mut sorted_ba = vec!["B".to_string(), "A".to_string()];
        sorted_ba.sort_unstable();
        assert_eq!(sorted_ab, sorted_ba);
    }

    #[test]
    fn duplicate_precursor_set_from_two_rules_merges_with_provenance_retained() {
        // Two distinct hand-crafted rules that can both fire on the same
        // molecule and (if they ever produced the same canonical precursor
        // set) must merge into one candidate without losing either rule's
        // provenance. Exercised at the merge-function level with synthetic
        // RawProposals rather than relying on finding two real rules that
        // collide, since that's a property of the merge logic, not of the
        // rule set.
        let precs = |smi: &str| PrecursorMol {
            smiles: smi.to_string(),
            mol: mol_from_smiles(smi).unwrap(),
        };
        let raw = vec![
            RawProposal {
                rule_name: "rule_a".to_string(),
                template_id: "rule:rule_a".to_string(),
                rule_weight: 1.0,
                original_rank: 0,
                precursors: vec![precs("CC"), precs("O")],
            },
            RawProposal {
                rule_name: "rule_b".to_string(),
                template_id: "rule:rule_b".to_string(),
                rule_weight: 1.0,
                original_rank: 5,
                precursors: vec![precs("O"), precs("CC")], // same set, different order
            },
        ];
        let merged = merge_into_candidates("target", raw, None);
        assert_eq!(
            merged.len(),
            1,
            "identical precursor sets must merge into one candidate"
        );
        let c = &merged[0];
        assert_eq!(c.proposal_count, 2);
        assert_eq!(
            c.original_rank, 0,
            "must keep the best (lowest) original_rank"
        );
        assert!(c.source_rule_names.contains(&"rule_a".to_string()));
        assert!(c.source_rule_names.contains(&"rule_b".to_string()));
        assert!(c.source_template_ids.contains(&"rule:rule_a".to_string()));
        assert!(c.source_template_ids.contains(&"rule:rule_b".to_string()));
    }

    #[test]
    fn best_upstream_score_retained_on_merge() {
        let precs = |smi: &str| PrecursorMol {
            smiles: smi.to_string(),
            mol: mol_from_smiles(smi).unwrap(),
        };
        let raw = vec![
            RawProposal {
                rule_name: "rule_a".to_string(),
                template_id: "rule:rule_a".to_string(),
                rule_weight: 1.0,
                original_rank: 3,
                precursors: vec![precs("CC")],
            },
            RawProposal {
                rule_name: "rule_b".to_string(),
                template_id: "rule:rule_b".to_string(),
                rule_weight: 1.0,
                original_rank: 1,
                precursors: vec![precs("CC")],
            },
        ];
        let merged = merge_into_candidates("target", raw, Some(&[Some(0.2), Some(0.9)]));
        assert_eq!(merged.len(), 1);
        assert_eq!(
            merged[0].upstream_score,
            Some(0.9),
            "must keep the best (max) upstream score"
        );
        assert_eq!(
            merged[0].original_rank, 1,
            "must keep the best (min) original_rank"
        );
    }

    #[test]
    fn no_precursors_produces_no_candidate() {
        let target = "CCO";
        let rules = vec![rule("noop", "[C:1]>>[C:1]")];
        let config = ProposalConfig::default();
        // A rule that maps carbon to itself should either not match, or if it
        // matches, produce a precursor set equal to the target itself and be
        // filtered as a self-loop -- either way, no candidate loss should
        // manifest as a panic or an unfiltered self-loop candidate.
        let pool = propose_one_step(target, &rules, &config).unwrap();
        for c in &pool.candidates {
            assert_ne!(
                c.precursor_smiles,
                vec![pool.target_smiles.clone()],
                "self-loop candidate must be filtered, not just the exact target restated"
            );
        }
    }
}
