//! RENKIN Bridge PR4: declared-reaction-replay forward validation.
//!
//! Deliberately narrow, per the P0 scope decision: only ever replays the ONE
//! reaction a route step already claims to have used (via `template_id` for
//! a RENKIN-native step, or whatever reaction representation a competitor's
//! route metadata carries) -- never a brute-force scan over the whole rule
//! set, never a forward-candidate search, never network/DOI/condition/yield
//! prediction. Missing or ambiguous reaction evidence reports
//! `not_evaluable`, not a guessed pass/fail -- this is *expected*, not a
//! shortfall, for e.g. an AiZynthFinder route whose `ReactionTree.to_dict()`
//! export is documented lossy, and whose reaction-node metadata schema this
//! codebase has no adapter for yet (see `bridge::route_graph::ReactionEvidence`).
//!
//! Reuses `validation::forward::matches_target` (canonical-string equality +
//! stereo-gated VF2 structural fallback) rather than duplicating it. The
//! replay loop itself (SMIRKS-reversal + `run_reactants`) is intentionally
//! NOT shared with `validation::forward::rule_reverses_to`: that function is
//! live in `renkin-bench` with its own regression tests guarding its exact
//! bool-collapsing behavior, and this module needs a richer, reason-coded
//! outcome plus a two-orientation retry for AiZynthFinder evidence (see
//! [`validate_step_forward`]) that `rule_reverses_to` doesn't need. Reshaping
//! a live, tested function for a new caller is worse than a small,
//! well-documented duplication.

use std::collections::HashMap;

use chematic::rxn::{TransformError, run_reactants};
use chematic::smarts::{QueryMolecule, parse_smarts};
use chematic::smiles::canonical_smiles;
use serde::Serialize;

use crate::bridge::audit::CheckStatus;
use crate::bridge::route_graph::ReactionEvidence;
use crate::chem_env::{Molecule, RetroRule, mol_from_smiles};
use crate::validation::forward::matches_target;

/// Reason a step's forward validation couldn't reach a pass/fail verdict.
/// Minimum set required by the P0 spec -- distinguishes "no reaction
/// representation at all" from "had one but couldn't apply it" so a caller
/// can tell an AiZynthFinder metadata gap apart from a genuine replay error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ForwardNotEvaluableReason {
    MissingReactionRepresentation,
    MissingAtomMapping,
    UnsupportedReactionFormat,
    UnsupportedTemplateSyntax,
    ReactionApplicationError,
    AmbiguousExpectedProduct,
}

#[derive(Debug, Clone, Serialize)]
pub struct ForwardValidationResult {
    pub status: CheckStatus,
    pub method: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<ForwardNotEvaluableReason>,
}

const METHOD: &str = "declared_reaction_replay";

fn not_evaluable(reason: ForwardNotEvaluableReason) -> ForwardValidationResult {
    ForwardValidationResult {
        status: CheckStatus::NotEvaluable,
        method: METHOD,
        reason: Some(reason),
    }
}

fn pass() -> ForwardValidationResult {
    ForwardValidationResult {
        status: CheckStatus::Pass,
        method: METHOD,
        reason: None,
    }
}

fn fail() -> ForwardValidationResult {
    ForwardValidationResult {
        status: CheckStatus::Fail,
        method: METHOD,
        reason: None,
    }
}

/// Weak/permissive check: at least one atom-map marker (`:` + digit) is
/// present anywhere. Doesn't require every atom mapped -- RENKIN's own
/// hand-crafted rules only map the atoms they need to track (e.g.
/// `"[C:1][O:2]>>[C:1].[O:2]"`) -- only flags total absence, matching this
/// reason code's name.
fn has_atom_mapping(smirks: &str) -> bool {
    let bytes = smirks.as_bytes();
    bytes
        .windows(2)
        .any(|w| w[0] == b':' && w[1].is_ascii_digit())
}

/// Resolve the declared SMIRKS for `evidence`, and whether the caller should
/// also try it in as-declared orientation (not just RENKIN's own
/// `target>>precursors` retro convention) -- see [`validate_step_forward`].
fn declared_smirks<'a>(
    evidence: &'a ReactionEvidence,
    rules_by_template_id: Option<&'a HashMap<String, &'a RetroRule>>,
) -> Result<(&'a str, bool), ForwardNotEvaluableReason> {
    use ForwardNotEvaluableReason::MissingReactionRepresentation;
    match evidence {
        ReactionEvidence::RenkinTemplate { template_id } => {
            let rule = rules_by_template_id
                .and_then(|m| m.get(template_id.as_str()))
                .ok_or(MissingReactionRepresentation)?;
            if rule.smirks.is_empty() {
                return Err(MissingReactionRepresentation);
            }
            Ok((rule.smirks.as_str(), false))
        }
        ReactionEvidence::AiZynthFinderTemplate { smirks } => {
            if smirks.is_empty() {
                return Err(MissingReactionRepresentation);
            }
            Ok((smirks.as_str(), true))
        }
        // Syntheseus's `reaction_smiles` is a plain `reactants>>product`
        // string (a computed property, never atom-mapped) -- same
        // as-declared orientation as AiZynthFinder's own metadata, so this
        // reuses the identical two-orientation retry. The downstream
        // `has_atom_mapping` gate then correctly reports `NotEvaluable
        // (MissingAtomMapping)` for every real Syntheseus route today: not a
        // gap, an honest reflection of what the source data actually
        // carries (no adapter here invents atom-mapping that isn't there).
        ReactionEvidence::SyntheseusReaction { reaction_smiles } => {
            if reaction_smiles.is_empty() {
                return Err(MissingReactionRepresentation);
            }
            Ok((reaction_smiles.as_str(), true))
        }
        // SynPlanner's reaction `smiles` is a `reactants>>product` SMIRKS
        // string, same as-declared orientation as AiZynthFinder/Syntheseus.
        // Confirmed against real MCTS-searched output (Phase 1 PR1.5): these
        // strings carry real, forward-replayable atom maps for the case
        // tested, so this is the one adapter today where the downstream
        // `has_atom_mapping` gate is expected to actually pass, not just
        // honestly report `MissingAtomMapping`.
        ReactionEvidence::SynPlannerReaction { smiles, .. } => {
            if smiles.is_empty() {
                return Err(MissingReactionRepresentation);
            }
            Ok((smiles.as_str(), true))
        }
    }
}

/// Try each candidate forward-oriented SMIRKS in turn against
/// `precursor_mols`, looking for a product matching the target. Any match ->
/// `Pass`, regardless of other non-matching candidates also produced (the
/// recorded reaction IS reproducible via a legitimate application). No match
/// and exactly one distinct non-matching product across every orientation
/// that ran -> a clean `Fail`. No match and more than one distinct product
/// -> `AmbiguousExpectedProduct`: there's no way to tell whether the
/// recorded reaction failed or the wrong multi-valued interpretation was
/// compared.
///
/// `orientations` is 1 element for a RENKIN step (the one direction its
/// retro-convention SMIRKS reverses to) or 2 for AiZynthFinder evidence,
/// whose storage direction this codebase has no confirmed schema for --
/// trying both is still replaying the single declared reaction, not a
/// search over alternatives.
/// `run_reactants` matches each `.`-separated SMIRKS reactant-side
/// component against the input reactant list *positionally*, not as an
/// unordered set -- confirmed empirically against a real AiZynthFinder
/// route (`tests/fixtures/aizynthfinder/v4.4.1/PROVENANCE.md`'s benzocaine
/// fixture): the exact same molecules, same SMIRKS, produced zero results
/// in one precursor order and the correct target in the other. A route's
/// own precursor list order is not semantically meaningful (it's a set),
/// so every orientation is tried against every precursor permutation, not
/// just the order the route happened to list them in -- this is still
/// replaying the one declared reaction, not searching alternatives: the
/// SMIRKS and the reactant set are both fixed, only their pairing varies.
/// Capped at `MAX_PERMUTE_LEN` reactants (6! = 720) to keep cost bounded;
/// a real per-step reactant count beyond that would be exceptional, and
/// only the as-given order is tried past the cap.
const MAX_PERMUTE_LEN: usize = 6;

fn index_permutations(n: usize) -> Vec<Vec<usize>> {
    if n > MAX_PERMUTE_LEN {
        return vec![(0..n).collect()];
    }
    fn permute(prefix: &mut Vec<usize>, remaining: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        if remaining.is_empty() {
            out.push(prefix.clone());
            return;
        }
        for i in 0..remaining.len() {
            let item = remaining.remove(i);
            prefix.push(item);
            permute(prefix, remaining, out);
            prefix.pop();
            remaining.insert(i, item);
        }
    }
    let mut out = Vec::new();
    permute(&mut Vec::new(), &mut (0..n).collect(), &mut out);
    out
}

fn replay_orientations(
    orientations: &[String],
    target_canon: &str,
    target_query: Option<&QueryMolecule>,
    target_atom_count: usize,
    precursor_mols: &[&Molecule],
) -> Result<bool, ForwardNotEvaluableReason> {
    let mut ran_successfully = false;
    let mut last_error = ForwardNotEvaluableReason::ReactionApplicationError;
    let mut distinct_products: Vec<String> = Vec::new();
    let permutations = index_permutations(precursor_mols.len());

    for fwd in orientations {
        for perm in &permutations {
            let ordered: Vec<&Molecule> = perm.iter().map(|&i| precursor_mols[i]).collect();
            match run_reactants(fwd, &ordered) {
                Err(TransformError::SmirksParse(_)) => {
                    last_error = ForwardNotEvaluableReason::UnsupportedTemplateSyntax;
                }
                Err(TransformError::ReactantCountMismatch { .. }) => {
                    last_error = ForwardNotEvaluableReason::ReactionApplicationError;
                }
                Ok(results) => {
                    ran_successfully = true;
                    for m in results.into_iter().flatten() {
                        if matches_target(&m, target_canon, target_query, target_atom_count) {
                            return Ok(true);
                        }
                        let canon = canonical_smiles(&m);
                        if !distinct_products.contains(&canon) {
                            distinct_products.push(canon);
                        }
                    }
                }
            }
        }
    }

    if !ran_successfully {
        return Err(last_error);
    }
    match distinct_products.len() {
        0 | 1 => Ok(false),
        _ => Err(ForwardNotEvaluableReason::AmbiguousExpectedProduct),
    }
}

/// Validate one step's declared reaction by replaying it forward and
/// checking whether the result reproduces `target_canonical`. `evidence` is
/// `None` when the route carries no reaction-identity information for this
/// step at all -- reported `not_evaluable`, never guessed.
///
/// `rules_by_template_id`: RENKIN's own rule corpus, indexed once by the
/// caller (`candidate::index_rules_by_template_id`) and shared across every
/// step's call so the audit's total cost stays proportional to step count,
/// not `steps * rules`. Only consulted for [`ReactionEvidence::RenkinTemplate`]
/// evidence -- `None` here means "no corpus was supplied", which resolves to
/// `not_evaluable` exactly like an unresolvable `template_id` would.
pub fn validate_step_forward(
    target_canonical: &str,
    precursor_canonical: &[String],
    evidence: Option<&ReactionEvidence>,
    rules_by_template_id: Option<&HashMap<String, &RetroRule>>,
) -> ForwardValidationResult {
    let Some(evidence) = evidence else {
        return not_evaluable(ForwardNotEvaluableReason::MissingReactionRepresentation);
    };
    let (smirks, try_both_orientations) = match declared_smirks(evidence, rules_by_template_id) {
        Ok(v) => v,
        Err(reason) => return not_evaluable(reason),
    };

    if !has_atom_mapping(smirks) {
        return not_evaluable(ForwardNotEvaluableReason::MissingAtomMapping);
    }
    let Some((lhs, rhs)) = smirks.split_once(">>") else {
        return not_evaluable(ForwardNotEvaluableReason::UnsupportedReactionFormat);
    };

    let mut orientations = vec![format!("{rhs}>>{lhs}")];
    if try_both_orientations {
        orientations.push(smirks.to_string());
    }

    let Ok(target_mol) = mol_from_smiles(target_canonical) else {
        return not_evaluable(ForwardNotEvaluableReason::ReactionApplicationError);
    };
    let Ok(precursor_mols): Result<Vec<_>, _> = precursor_canonical
        .iter()
        .map(|s| mol_from_smiles(s))
        .collect()
    else {
        return not_evaluable(ForwardNotEvaluableReason::ReactionApplicationError);
    };
    let mol_refs: Vec<&Molecule> = precursor_mols.iter().collect();
    let target_canon = canonical_smiles(&target_mol);
    let target_query = parse_smarts(target_canonical).ok();
    let target_atom_count = target_mol.atom_count();

    match replay_orientations(
        &orientations,
        &target_canon,
        target_query.as_ref(),
        target_atom_count,
        &mol_refs,
    ) {
        Err(reason) => not_evaluable(reason),
        Ok(true) => pass(),
        Ok(false) => fail(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::candidate::index_rules_by_template_id;

    fn co_aliphatic_cleavage() -> RetroRule {
        RetroRule {
            name: "co_aliphatic_cleavage".to_string(),
            template_id: "t1".to_string(),
            smirks: "[C:1][O:2]>>[C:1].[O:2]".to_string(),
            ..Default::default()
        }
    }

    // Methane + water, each with exactly one atom the rule's pattern can
    // match (one C, one O) -- deterministic, single-product replay, so the
    // fail case below can't collide with the ambiguous-product path (see
    // `replay_orientations`'s doc comment for why a multi-carbon precursor
    // would produce more than one distinct candidate here).
    const METHANE: &str = "C";
    const WATER: &str = "O";
    const METHANOL: &str = "CO";

    /// Regression for a real bug found while building the AiZynthFinder
    /// adapter (RENKIN Bridge PR6): `run_reactants` matches SMIRKS reactant
    /// components positionally, not as a set. This exact target/precursor
    /// pair (order: ethanol, then the amino-acid) comes verbatim from a
    /// real `aizynthcli 4.4.1` route
    /// (`tests/fixtures/aizynthfinder/v4.4.1/single_trees.json`, route
    /// index 1's outer step) and, before the permutation fix, failed to
    /// reproduce the target purely because of precursor list order -- a
    /// spurious FAIL on a route AiZynthFinder itself confirms is valid.
    /// Swapping the precursor order below must not change the verdict:
    /// this affects RENKIN-native routes too, not just AiZynthFinder ones,
    /// since `replay_orientations` is shared by both.
    #[test]
    fn precursor_order_does_not_affect_the_verdict() {
        let evidence = ReactionEvidence::AiZynthFinderTemplate {
            smirks: "[CH3:1][CH2:2][O:3][C:4](=[O:5])[c:6]1[cH:7][cH:8][c:9]([NH2:10])[cH:11][cH:12]1>>[C:4](=[O:5])([c:6]1[cH:7][cH:8][c:9]([NH2:10])[cH:11][cH:12]1)[OH:13].[CH3:1][CH2:2][OH:3]".to_string(),
        };
        let target = "CCOC(=O)c1ccc(N)cc1";
        let ethanol = "CCO".to_string();
        let amino_acid = "Nc1ccc(C(=O)O)cc1".to_string();

        let as_listed_by_aizynthfinder = vec![ethanol.clone(), amino_acid.clone()];
        let reversed = vec![amino_acid, ethanol];

        for precursors in [as_listed_by_aizynthfinder, reversed] {
            let result = validate_step_forward(target, &precursors, Some(&evidence), None);
            assert_eq!(
                result.status,
                CheckStatus::Pass,
                "precursor order must not change the verdict: {result:?} for {precursors:?}"
            );
        }
    }

    #[test]
    fn renkin_native_step_that_replays_correctly_passes() {
        let rule = co_aliphatic_cleavage();
        let rules = vec![rule];
        let by_id = index_rules_by_template_id(&rules).unwrap();
        let evidence = ReactionEvidence::RenkinTemplate {
            template_id: "t1".to_string(),
        };
        let precursors = vec![METHANE.to_string(), WATER.to_string()];
        let result = validate_step_forward(METHANOL, &precursors, Some(&evidence), Some(&by_id));
        assert_eq!(result.status, CheckStatus::Pass, "{result:?}");
        assert_eq!(result.method, "declared_reaction_replay");
        assert!(result.reason.is_none());
    }

    #[test]
    fn renkin_native_step_producing_a_different_parent_fails() {
        let rule = co_aliphatic_cleavage();
        let rules = vec![rule];
        let by_id = index_rules_by_template_id(&rules).unwrap();
        let evidence = ReactionEvidence::RenkinTemplate {
            template_id: "t1".to_string(),
        };
        // A route that (wrongly) claims methane + water give ethane --
        // co_aliphatic_cleavage's reversal deterministically produces
        // methanol from these precursors, never ethane.
        let wrong_target = "CC";
        let precursors = vec![METHANE.to_string(), WATER.to_string()];
        let result =
            validate_step_forward(wrong_target, &precursors, Some(&evidence), Some(&by_id));
        assert_eq!(result.status, CheckStatus::Fail, "{result:?}");
        assert!(result.reason.is_none());
    }

    /// A multi-carbon precursor gives the rule's reversal more than one
    /// place to attach the oxygen, so it can't reach a single deterministic
    /// product to compare against a non-matching target -- discovered
    /// empirically while building the fixtures above (this exact
    /// precursor/target pair originally stood in for the plain Fail case
    /// until it turned out to be ambiguous instead).
    #[test]
    fn multiple_distinct_non_matching_products_is_ambiguous_not_fail() {
        let rule = co_aliphatic_cleavage();
        let rules = vec![rule];
        let by_id = index_rules_by_template_id(&rules).unwrap();
        let evidence = ReactionEvidence::RenkinTemplate {
            template_id: "t1".to_string(),
        };
        let precursors = vec![
            "[CH3][NH]C(CO[CH2][CH3])=O".to_string(),
            "O[CH2][CH3]".to_string(),
        ];
        let result = validate_step_forward("CCC", &precursors, Some(&evidence), Some(&by_id));
        assert_eq!(result.status, CheckStatus::NotEvaluable, "{result:?}");
        assert_eq!(
            result.reason,
            Some(ForwardNotEvaluableReason::AmbiguousExpectedProduct)
        );
    }

    #[test]
    fn aizynthfinder_step_with_no_evidence_is_not_evaluable() {
        let result = validate_step_forward("CCOC", &["CCO".to_string()], None, None);
        assert_eq!(result.status, CheckStatus::NotEvaluable);
        assert_eq!(
            result.reason,
            Some(ForwardNotEvaluableReason::MissingReactionRepresentation)
        );
    }

    #[test]
    fn aizynthfinder_step_with_sufficient_metadata_passes() {
        // Declared in AiZynthFinder's as-exported orientation
        // (precursors>>target) rather than RENKIN's retro convention -- the
        // two-orientation retry must still find it.
        let evidence = ReactionEvidence::AiZynthFinderTemplate {
            smirks: "[C:1][O:2].[C:3]>>[C:1][O:2][C:3]".to_string(),
        };
        let target = "CCOC";
        let precursors = vec!["CCO".to_string(), "C".to_string()];
        let result = validate_step_forward(target, &precursors, Some(&evidence), None);
        assert_eq!(result.status, CheckStatus::Pass, "{result:?}");
    }

    #[test]
    fn missing_atom_mapping_is_distinguished_from_missing_representation() {
        let evidence = ReactionEvidence::AiZynthFinderTemplate {
            smirks: "CCO.C>>CCOC".to_string(),
        };
        let result = validate_step_forward(
            "CCOC",
            &["CCO".to_string(), "C".to_string()],
            Some(&evidence),
            None,
        );
        assert_eq!(result.status, CheckStatus::NotEvaluable);
        assert_eq!(
            result.reason,
            Some(ForwardNotEvaluableReason::MissingAtomMapping)
        );
    }
}
