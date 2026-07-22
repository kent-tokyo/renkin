#![forbid(unsafe_code)]

//! SMIRKS-reversal forward validation.
//!
//! Moved out of `renkin-bench` verbatim (commit "centralize shared validation
//! helpers") — behavior is unchanged for [`smirks_reproduces`]/[`route_forward_validated`].
//! Only SMIRKS-based rules are tried; graph-based rules (empty `smirks`) are
//! skipped here and covered by [`super::graph_rules`] instead.
//!
//! [`rule_reproduces`] is the provenance-bound sibling added for
//! `validate_step`: it checks exactly one rule's reversed SMIRKS rather than
//! scanning the whole rule set, so a step is only confirmed by the
//! transformation it actually claims to have used.
//!
//! ## Canonical-SMILES fallback (Track F, Phase 32)
//!
//! chematic's `canonical_smiles` is a stable fixed point but NOT invariant
//! under input atom order / bracket-atom notation (lessons.md L2, confirmed
//! still present in chematic 0.4.30 during the Phase 32 gold-set audit):
//! the same molecule, parsed from two differently-written SMILES, can
//! canonicalize to two different strings that never converge even after
//! repeated re-canonicalization. `ChemEnv::is_building_block` already works
//! around this exact issue with a VF2 structural-isomorphism fallback
//! (`chematic::smarts::find_matches`); [`rule_reverses_to`] mirrors that
//! precedent here, since a naive string-equality reversal check inherits the
//! same false-negative failure mode whenever a forward-reaction product
//! happens to serialize with different bracket-atom style than the target
//! (empirically the majority cause of "Invalid" verdicts on a gold-set
//! sample — see the Track F report for measurements).
//!
//! The VF2 fallback is skipped whenever either side's canonical SMILES
//! contains a `@`/`@@` tetrahedral stereo marker: `find_matches` was
//! confirmed empirically to be stereo-blind for tetrahedral centers (it
//! matches (R)- and (S)-2-butanol as the same structure) even though it
//! does respect E/Z double-bond geometry. Falling back to VF2 unconditionally
//! would silently launder a wrong-stereochemistry step into a Valid
//! verdict — worse than the false negative it would fix.

use chematic::rxn::run_reactants;
use chematic::smarts::{QueryMolecule, find_matches, parse_smarts};
use chematic::smiles::canonical_smiles;

use crate::chem_env::{Molecule, RetroRule, mol_from_smiles};

/// True if `target_canon`/`target_query` structurally match `candidate`,
/// either by canonical-string equality (fast path) or, failing that and
/// absent any tetrahedral stereo marker on either side, by VF2 graph
/// isomorphism against `target_query` (see module docs).
fn matches_target(
    candidate: &Molecule,
    target_canon: &str,
    target_query: Option<&QueryMolecule>,
    target_atom_count: usize,
) -> bool {
    let candidate_canon = canonical_smiles(candidate);
    if candidate_canon == target_canon {
        return true;
    }
    if target_canon.contains('@') || candidate_canon.contains('@') {
        return false;
    }
    let Some(query) = target_query else {
        return false;
    };
    candidate.atom_count() == target_atom_count
        && find_matches(query, candidate)
            .iter()
            .any(|m| m.len() == target_atom_count)
}

/// Core check: does `rule`'s reversed SMIRKS, applied to pre-parsed `precursor_mols`,
/// produce a molecule that structurally matches the target (see [`matches_target`])?
fn rule_reverses_to(
    target_canon: &str,
    target_query: Option<&QueryMolecule>,
    target_atom_count: usize,
    precursor_mols: &[&Molecule],
    rule: &RetroRule,
) -> bool {
    let Some((lhs, rhs)) = rule.smirks.split_once(">>") else {
        return false;
    };
    let fwd = format!("{rhs}>>{lhs}");
    run_reactants(&fwd, precursor_mols)
        .into_iter()
        .flatten()
        .flatten()
        .any(|m| matches_target(&m, target_canon, target_query, target_atom_count))
}

/// True if any SMIRKS-based rule, applied forward to `precursors`, reproduces `target`.
///
/// Tries every rule with a non-empty `smirks` (not just the one the step
/// actually used at retro time) — a forward match from a different rule still
/// confirms *some* chemically real transformation connects precursors to
/// target, but NOT that the step's claimed rule is itself correct. Callers
/// that need to confirm a step's own claimed rule should use
/// [`rule_reproduces`] instead — see `validate_step` in `super`.
pub fn smirks_reproduces(target: &str, precursors: &[String], rules: &[RetroRule]) -> bool {
    let Ok(reactant_mols): Result<Vec<_>, _> =
        precursors.iter().map(|s| mol_from_smiles(s)).collect()
    else {
        return false;
    };
    let Ok(target_mol) = mol_from_smiles(target) else {
        return false;
    };
    let target_canon = canonical_smiles(&target_mol);
    let target_query = parse_smarts(target).ok();
    let target_atom_count = target_mol.atom_count();
    let mol_refs: Vec<_> = reactant_mols.iter().collect();
    rules.iter().filter(|r| !r.smirks.is_empty()).any(|rule| {
        rule_reverses_to(
            &target_canon,
            target_query.as_ref(),
            target_atom_count,
            &mol_refs,
            rule,
        )
    })
}

/// True if `rule`'s own reversed SMIRKS, applied to `precursors`, reproduces `target`.
///
/// Unlike [`smirks_reproduces`], this checks exactly the one rule passed in —
/// not the whole rule set — so a coincidental match from an unrelated rule
/// can never confirm a step. Returns `false` for graph-based rules (empty
/// `smirks`) or malformed SMILES/SMIRKS.
pub fn rule_reproduces(target: &str, precursors: &[String], rule: &RetroRule) -> bool {
    if rule.smirks.is_empty() {
        return false;
    }
    let Ok(reactant_mols): Result<Vec<_>, _> =
        precursors.iter().map(|s| mol_from_smiles(s)).collect()
    else {
        return false;
    };
    let Ok(target_mol) = mol_from_smiles(target) else {
        return false;
    };
    let target_canon = canonical_smiles(&target_mol);
    let target_query = parse_smarts(target).ok();
    let target_atom_count = target_mol.atom_count();
    let mol_refs: Vec<_> = reactant_mols.iter().collect();
    rule_reverses_to(
        &target_canon,
        target_query.as_ref(),
        target_atom_count,
        &mol_refs,
        rule,
    )
}

/// True if every step of the route passes SMIRKS-reversal forward validation.
/// Preserved for compatibility with call sites that only care about SMIRKS-based
/// rules (renkin-forward's per-step validator uses its own equivalent logic).
pub fn route_forward_validated(route: &crate::search::Route, rules: &[RetroRule]) -> bool {
    route
        .steps
        .iter()
        .all(|step| smirks_reproduces(&step.target, &step.precursors, rules))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chem_env::RetroRule;

    fn co_aliphatic_cleavage() -> RetroRule {
        RetroRule {
            name: "co_aliphatic_cleavage".to_string(),
            smirks: "[C:1][O:2]>>[C:1].[O:2]".to_string(),
            ..Default::default()
        }
    }

    /// Regression for the Track F gold-set finding: a real USPTO-50k
    /// benchmark step (co_aliphatic_cleavage on a diethyl-acetal-like
    /// amide, no ring, no stereo) whose precursors are chematic's own
    /// bracket-heavy fragment style. Before this fix: `canonical_smiles`
    /// on the reversed-SMIRKS forward product ("[NH]...") never equals
    /// `canonical_smiles` on the target's bracket-heavy parse
    /// ("[CH3][NH]..."), even though RDKit confirms they are the same
    /// molecule (chematic's canonical SMILES is order-dependent, not a
    /// true graph invariant — lessons.md L2). After this fix: the VF2
    /// structural fallback recognizes the match.
    #[test]
    fn co_aliphatic_cleavage_recognizes_bracket_style_reversal_match() {
        let rule = co_aliphatic_cleavage();
        let target = "[CH3][NH]C(=O)[CH](O[CH2][CH3])O[CH2][CH3]";
        let precursors = vec![
            "[CH3][NH]C(CO[CH2][CH3])=O".to_string(),
            "O[CH2][CH3]".to_string(),
        ];
        assert!(
            rule_reproduces(target, &precursors, &rule),
            "VF2 fallback must recognize this reversal match despite chematic's \
             canonical-string mismatch (lessons.md L2)"
        );
    }

    /// The VF2 fallback must never launder a wrong-stereochemistry match: if
    /// a forward-reaction candidate is the correct constitution but the
    /// wrong tetrahedral configuration, `matches_target` must still reject
    /// it. `find_matches` was confirmed empirically stereo-blind for
    /// tetrahedral centers (matches (R)- and (S)-2-butanol as the same
    /// structure), so the fallback is gated off whenever a `@`/`@@` marker
    /// is present on either side.
    #[test]
    fn vf2_fallback_does_not_launder_wrong_stereochemistry() {
        let target_mol = mol_from_smiles("CC[C@@H](C)O").unwrap(); // (S)-2-butanol
        let candidate_mol = mol_from_smiles("CC[C@H](C)O").unwrap(); // (R)-2-butanol
        let target_canon = canonical_smiles(&target_mol);
        let target_query = parse_smarts("CC[C@@H](C)O").ok();
        let target_atom_count = target_mol.atom_count();

        // Sanity check: this fixture is meaningful only if chematic's own
        // canonical strings actually differ between the two configurations.
        assert_ne!(
            target_canon,
            canonical_smiles(&candidate_mol),
            "sanity check failed: (R)/(S)-2-butanol must canonicalize differently \
             for this fixture to exercise the stereo guard"
        );

        assert!(
            !matches_target(
                &candidate_mol,
                &target_canon,
                target_query.as_ref(),
                target_atom_count
            ),
            "VF2 structural fallback must not ignore tetrahedral stereochemistry"
        );
    }
}
