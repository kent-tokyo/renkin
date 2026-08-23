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
///
/// `pub(crate)`: also reused by `bridge::forward`'s reason-coded forward
/// validation (RENKIN Bridge PR4), which needs this same matching logic but
/// a richer outcome than this module's `bool`-only public API -- see that
/// module's doc comment for why the surrounding replay loop is duplicated
/// rather than shared.
pub(crate) fn matches_target(
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

    /// Frozen fixture, `co_aliphatic_cleavage` on
    /// `O=C(O[C@@H]1CCCNC1)N` <- `C1CCNCC1.NC(O)=O`: one of Finding #4's
    /// 6 pilot Invalid+balanced steps
    /// (`docs/validation/finding4-validator-pilot-2026-08-23.md`),
    /// individually investigated per that doc's own protocol rather than
    /// re-running search. Classified as **`source_step_underspecified`**,
    /// not a validator false negative and not a genuine template/search
    /// defect:
    ///
    /// - `co_aliphatic_cleavage`'s SMIRKS (`[C:1][O:2]>>[C:1].[O:2]`)
    ///   carries no stereo annotation at all -- fully generic aliphatic
    ///   C-O cleavage, by design (matches sibling rules like
    ///   `cn_aliphatic_cleavage`/`reductive_amination_retro`).
    /// - Reversed and applied forward to the declared precursors, it
    ///   produces exactly 3 distinct regiochemical outcomes (one per
    ///   piperidine's 3 distinct carbon environments: {2,6}, {3,5}, {4}).
    ///   One of them, `O=C(OC1CCCNC1)N`, has the *exact same connectivity*
    ///   as the target -- the regiochemistry (ring position 3) is
    ///   correctly reproduced -- but carries no `@`/`@@` marker at all,
    ///   because the rule never specified one.
    /// - The target's own canonical form, `O=C(O[C@@H]1CCCNC1)N`, carries
    ///   a real, defined stereocenter. `matches_target`'s VF2 structural
    ///   fallback is deliberately disabled whenever either side has a
    ///   stereo marker (this module's own doc comment: VF2 is confirmed
    ///   stereo-blind and would otherwise silently launder wrong
    ///   stereochemistry into a false `Valid`) -- so canonical-string
    ///   equality is the only check available here, and it correctly
    ///   fails: `"...OC1CCCNC1)N"` != `"...O[C@@H]1CCCNC1)N"`.
    ///
    /// This is not "the validator is wrong" (it's correctly refusing to
    /// confirm stereochemistry a stereo-blind rule can't determine) and
    /// not "the retro-step is chemically broken" (piperidine + carbamic
    /// acid really can form 3-substituted piperidinyl carbamate, and the
    /// *regiochemistry* is right) -- the declared step just doesn't carry
    /// enough information to verify *which enantiomer* forms, because the
    /// rule it claims is fundamentally achiral. A real, disclosed
    /// limitation of generic achiral-disconnection rules applied
    /// retrosynthetically to targets with a real stereocenter at the
    /// reaction site -- worth a dedicated look at how many of the other 5
    /// pilot findings (or the broader corpus) share this same shape, not
    /// fixed here.
    #[test]
    fn co_aliphatic_cleavage_piperidinyl_carbamate_is_source_step_underspecified() {
        let target = "O=C(O[C@@H]1CCCNC1)N";
        let precursors = vec!["C1CCNCC1".to_string(), "NC(O)=O".to_string()];
        let rule = co_aliphatic_cleavage();

        // The rule's own claimed step does NOT verify -- confirms this is
        // genuinely one of Finding #4's 6 Invalid+balanced steps, not
        // something that already silently passes.
        assert!(
            !rule_reproduces(target, &precursors, &rule),
            "expected this exact Finding #4 pilot step to still be Invalid"
        );

        // Reversed-SMIRKS replay: the correct regiochemistry IS among the
        // outcomes, just with no stereo marker -- confirms the mechanism,
        // not just the boolean verdict.
        let reactant_mols: Vec<Molecule> = precursors
            .iter()
            .map(|s| mol_from_smiles(s).unwrap())
            .collect();
        let reactant_refs: Vec<&Molecule> = reactant_mols.iter().collect();
        let (lhs, rhs) = rule.smirks.split_once(">>").unwrap();
        let fwd = format!("{rhs}>>{lhs}");
        let products: std::collections::BTreeSet<String> = run_reactants(&fwd, &reactant_refs)
            .unwrap()
            .into_iter()
            .flatten()
            .map(|m| canonical_smiles(&m))
            .collect();

        let stereo_free_correct_connectivity = "O=C(OC1CCCNC1)N";
        assert!(
            products.contains(stereo_free_correct_connectivity),
            "the rule's reversal must still find the right regiochemistry \
             (just without stereo): {products:?}"
        );
        assert!(
            !products.contains(canonical_smiles(&mol_from_smiles(target).unwrap()).as_str()),
            "the rule's reversal must never spontaneously produce the \
             exact stereo-defined target -- it has no stereo information \
             to do so from: {products:?}"
        );
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

    fn cc_single_cleavage() -> RetroRule {
        RetroRule {
            name: "cc_single_cleavage".to_string(),
            smirks: "[C:1][C:2]>>[C:1].[C:2]".to_string(),
            ..Default::default()
        }
    }

    /// Frozen fixture, `cc_single_cleavage` on `C1CC[C@H](C)NC[C@@H]1C`
    /// (2,7-dimethylazepane) <- `C1CCCC[C@H](N1)C.C` (2-methylazepane +
    /// methane): one of Finding #4's 6 pilot Invalid+balanced steps
    /// (`docs/validation/finding4-validator-pilot-2026-08-23.md`),
    /// individually investigated per that doc's own protocol. Classified as
    /// **`genuine_template_error`** -- a different failure shape than
    /// `co_aliphatic_cleavage_piperidinyl_carbamate_is_source_step_underspecified`,
    /// even though the graph-level mechanics look superficially similar:
    ///
    /// - `cc_single_cleavage`'s SMIRKS (`[C:1][C:2]>>[C:1].[C:2]`) is fully
    ///   generic aliphatic C-C cleavage, same style as `co_aliphatic_cleavage`
    ///   (`[C:1][O:2]>>...`) -- no stereo annotation, by design.
    /// - Reversed and applied forward to the declared precursors, it
    ///   produces 7 distinct regiochemical outcomes. One of them,
    ///   `C1CC[C@H](C)NCC1C`, has the *exact same connectivity* as the
    ///   target with the pre-existing stereocenter (from the precursor)
    ///   correctly retained -- only the newly-formed center carries no
    ///   `@`/`@@` marker, because the rule never specified one. Purely by
    ///   this graph-connectivity test, this looks identical in shape to
    ///   `co_aliphatic_cleavage`'s finding.
    /// - The difference: `co_aliphatic_cleavage` claims a real reaction
    ///   class (amine + carbamic-acid-equivalent -> carbamate is a genuine,
    ///   if achiral, disconnection). This step claims installing a methyl
    ///   group at an unactivated ring C-H position using **bare methane**
    ///   (`C`) as the second reagent -- `data/building_blocks.smi:8` lists
    ///   `C` as stock, literally named `methane`. There is no real
    ///   single-step reaction that methylates an alkyl C-H using free
    ///   methane as a stoichiometric reagent (methane C-H activation is a
    ///   specialized catalytic research topic, not a general-purpose
    ///   disconnection any retrosynthesis template should imply). This
    ///   also matches Phase 31's own baseline measurement of this exact
    ///   rule (92.3% Invalid) -- not a one-off.
    /// - So unlike `co_aliphatic_cleavage`, the defect isn't "missing
    ///   stereo info on an otherwise-real reaction" -- it's that the rule's
    ///   full generality lets it pair *any* C-C bond with a degenerate,
    ///   functional-group-free single-carbon "leaving group" that has no
    ///   real reagent behind it. Classified `genuine_template_error`
    ///   because the *template itself* (not just this one application) is
    ///   defective: it should not treat a bare-methyl / bare-methane
    ///   fragment as a legitimate disconnection partner. Not fixed here
    ///   (restricting the rule, or removing methane from the stock list,
    ///   is a rule-design decision) -- this test only freezes the
    ///   classification and the empirical mechanism behind it.
    #[test]
    fn cc_single_cleavage_azepane_methane_is_genuine_template_error() {
        let target = "C1CC[C@H](C)NC[C@@H]1C";
        let precursors = vec!["C1CCCC[C@H](N1)C".to_string(), "C".to_string()];
        let rule = cc_single_cleavage();

        // The rule's own claimed step does NOT verify -- confirms this is
        // genuinely one of Finding #4's 6 Invalid+balanced steps, not
        // something that already silently passes.
        assert!(
            !rule_reproduces(target, &precursors, &rule),
            "expected this exact Finding #4 pilot step to still be Invalid"
        );

        // Reversed-SMIRKS replay: the correct connectivity (right ring
        // position, pre-existing stereocenter retained) IS among the 7
        // outcomes, just with no stereo marker on the newly-formed center --
        // confirms the mechanism, not just the boolean verdict.
        let reactant_mols: Vec<Molecule> = precursors
            .iter()
            .map(|s| mol_from_smiles(s).unwrap())
            .collect();
        let reactant_refs: Vec<&Molecule> = reactant_mols.iter().collect();
        let (lhs, rhs) = rule.smirks.split_once(">>").unwrap();
        let fwd = format!("{rhs}>>{lhs}");
        let products: std::collections::BTreeSet<String> = run_reactants(&fwd, &reactant_refs)
            .unwrap()
            .into_iter()
            .flatten()
            .map(|m| canonical_smiles(&m))
            .collect();

        let stereo_free_correct_connectivity = "C1CC[C@H](C)NCC1C";
        assert!(
            products.contains(stereo_free_correct_connectivity),
            "the rule's reversal must still find the right regiochemistry \
             (just missing the newly-formed center's stereo): {products:?}"
        );
        assert!(
            !products.contains(canonical_smiles(&mol_from_smiles(target).unwrap()).as_str()),
            "the rule's reversal must never spontaneously produce the \
             exact stereo-defined target -- it has no stereo information \
             to do so from: {products:?}"
        );

        // The mechanism enabling this: methane really is a stock building
        // block, so the search can legitimately propose it as a "reagent"
        // for any C-C cleavage -- confirming the defect is in the rule's
        // scope, not a data-loading fluke.
        let building_blocks = std::fs::read_to_string("data/building_blocks.smi")
            .expect("data/building_blocks.smi must be readable from the crate root");
        assert!(
            building_blocks
                .lines()
                .any(|line| line.split_whitespace().next() == Some("C")),
            "methane ('C') must be present as its own stock entry in \
             data/building_blocks.smi for this to be the real mechanism, \
             not a stale assumption"
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
