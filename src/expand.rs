//! Single-step retrosynthetic expansion (AiZynthFinder `AiZynthExpander`
//! parity).
//!
//! Answers "what are the one-step disconnections of this molecule?" without
//! running a multi-step search. It reuses the exact proposal path the search
//! and the offline candidate-pool exporter use
//! ([`crate::candidate::CandidateProposalContext::propose_one_step`]), so a
//! precursor set listed here is the same set the search would enqueue for
//! that molecule, merged across every rule that produced it.
//!
//! Ordering is deterministic: ascending chemistry-only step cost
//! (`min_base_step_cost`), then more in-stock precursors first, then
//! `candidate_id`. The cost is RENKIN's heuristic step cost, not a policy
//! probability and not a feasibility or yield claim.

use crate::candidate::{
    CandidatePoolStats, CandidateProposalContext, ProposalConfig, ProposalMode,
};
use crate::chem_env::{ChemEnv, RetroRule, mol_from_smiles, to_canonical};
use anyhow::Result;
use serde::Serialize;

/// Output schema version for [`ExpansionResult`].
pub const EXPANSION_SCHEMA_VERSION: u32 = 1;

/// Options for [`expand_one_step`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ExpansionOptions {
    /// Maximum candidates returned after ordering; `0` means all.
    pub max_candidates: usize,
    /// Use the reaction-center bond index to select rules (mirrors the
    /// search's `--bond-index`). `false` tries every rule.
    pub bond_index: bool,
}

/// One precursor of an expansion candidate.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ExpansionPrecursor {
    pub smiles: String,
    /// Exact standardized canonical-SMILES stock membership (the same
    /// identity policy the search uses).
    pub in_stock: bool,
}

/// One merged one-step disconnection.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ExpansionCandidate {
    /// 1-based position after deterministic ordering.
    pub rank: usize,
    pub candidate_id: String,
    /// `precursors>>target` (retro direction written forward).
    pub reaction_smiles: String,
    pub precursors: Vec<ExpansionPrecursor>,
    pub in_stock_count: usize,
    pub all_in_stock: bool,
    /// Lowest chemistry-only step cost among merged sources.
    pub step_cost: f64,
    /// Every rule/template that produced this precursor set, representative
    /// source first.
    pub template_ids: Vec<String>,
    pub rule_names: Vec<String>,
}

/// Result of [`expand_one_step`].
#[derive(Debug, Clone, Serialize)]
pub struct ExpansionResult {
    pub schema_version: u32,
    pub target: String,
    pub target_in_stock: bool,
    pub candidates_total: usize,
    pub candidates_returned: usize,
    pub candidates: Vec<ExpansionCandidate>,
    pub stats: CandidatePoolStats,
}

/// Propose every one-step disconnection of `target_smiles` under `rules`.
pub fn expand_one_step(
    target_smiles: &str,
    env: &ChemEnv,
    rules: &[RetroRule],
    options: &ExpansionOptions,
) -> Result<ExpansionResult> {
    let target_mol = mol_from_smiles(target_smiles)?;
    let target_canonical = to_canonical(&target_mol);
    let target_in_stock = env.is_building_block(&target_mol);

    let mode = if options.bond_index {
        ProposalMode::BondIndexed { top_k: 0 }
    } else {
        ProposalMode::Exhaustive
    };
    let pool = CandidateProposalContext::new(rules, options.bond_index).propose_one_step(
        "expand",
        &target_canonical,
        &ProposalConfig { mode },
    )?;

    let mut candidates: Vec<ExpansionCandidate> = pool
        .candidates
        .iter()
        .map(|candidate| {
            let precursors: Vec<ExpansionPrecursor> = candidate
                .precursor_smiles
                .iter()
                .map(|smiles| ExpansionPrecursor {
                    in_stock: env.is_building_block_smiles(smiles)
                        || mol_from_smiles(smiles).is_ok_and(|m| env.is_building_block(&m)),
                    smiles: smiles.clone(),
                })
                .collect();
            let in_stock_count = precursors.iter().filter(|p| p.in_stock).count();
            ExpansionCandidate {
                rank: 0,
                candidate_id: candidate.candidate_id.clone(),
                reaction_smiles: format!(
                    "{}>>{}",
                    candidate.precursor_smiles.join("."),
                    pool.target_smiles
                ),
                all_in_stock: !precursors.is_empty() && in_stock_count == precursors.len(),
                in_stock_count,
                precursors,
                step_cost: candidate.min_base_step_cost,
                template_ids: candidate
                    .sources
                    .iter()
                    .map(|s| s.template_id.clone())
                    .collect(),
                rule_names: candidate
                    .sources
                    .iter()
                    .map(|s| s.rule_name.clone())
                    .collect(),
            }
        })
        .collect();

    candidates.sort_by(|a, b| {
        a.step_cost
            .total_cmp(&b.step_cost)
            .then_with(|| b.in_stock_count.cmp(&a.in_stock_count))
            .then_with(|| a.candidate_id.cmp(&b.candidate_id))
    });
    let candidates_total = candidates.len();
    if options.max_candidates > 0 {
        candidates.truncate(options.max_candidates);
    }
    for (index, candidate) in candidates.iter_mut().enumerate() {
        candidate.rank = index + 1;
    }

    Ok(ExpansionResult {
        schema_version: EXPANSION_SCHEMA_VERSION,
        target: target_canonical,
        target_in_stock,
        candidates_total,
        candidates_returned: candidates.len(),
        candidates,
        stats: pool.stats,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chem_env::default_rules;

    const ASPIRIN: &str = "CC(=O)Oc1ccccc1C(=O)O";

    fn env() -> ChemEnv {
        ChemEnv::in_memory(&["CC(=O)O", "Oc1ccccc1C(=O)O"])
    }

    #[test]
    fn aspirin_has_an_all_in_stock_ester_disconnection() {
        let result = expand_one_step(ASPIRIN, &env(), &default_rules(), &Default::default())
            .expect("aspirin parses");
        assert_eq!(result.schema_version, EXPANSION_SCHEMA_VERSION);
        assert!(!result.target_in_stock);
        assert!(result.candidates_total > 0);
        assert_eq!(result.candidates_total, result.candidates_returned);
        let ester = result
            .candidates
            .iter()
            .find(|c| c.all_in_stock)
            .expect("salicylic acid + acetic acid disconnection");
        assert_eq!(ester.in_stock_count, ester.precursors.len());
        assert!(
            ester
                .reaction_smiles
                .ends_with(&format!(">>{}", result.target))
        );
        assert!(!ester.template_ids.is_empty());
        assert_eq!(ester.template_ids.len(), ester.rule_names.len());
    }

    #[test]
    fn ordering_is_deterministic_and_ranked() {
        let rules = default_rules();
        let a = expand_one_step(ASPIRIN, &env(), &rules, &Default::default()).unwrap();
        let b = expand_one_step(ASPIRIN, &env(), &rules, &Default::default()).unwrap();
        assert_eq!(a.candidates, b.candidates);
        for (index, candidate) in a.candidates.iter().enumerate() {
            assert_eq!(candidate.rank, index + 1);
        }
        for pair in a.candidates.windows(2) {
            assert!(pair[0].step_cost <= pair[1].step_cost);
        }
    }

    #[test]
    fn max_candidates_truncates_but_reports_total() {
        let options = ExpansionOptions {
            max_candidates: 1,
            ..Default::default()
        };
        let result = expand_one_step(ASPIRIN, &env(), &default_rules(), &options).unwrap();
        assert_eq!(result.candidates_returned, 1);
        assert!(result.candidates_total >= 1);
    }

    #[test]
    fn invalid_smiles_is_an_error() {
        assert!(
            expand_one_step(
                "not-a-smiles((",
                &env(),
                &default_rules(),
                &Default::default()
            )
            .is_err()
        );
    }
}
