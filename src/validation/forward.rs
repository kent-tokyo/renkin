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

use chematic::rxn::run_reactants;
use chematic::smiles::canonical_smiles;

use crate::chem_env::{Molecule, RetroRule, mol_from_smiles};

/// Core check: does `rule`'s reversed SMIRKS, applied to pre-parsed `precursor_mols`,
/// produce a molecule that canonicalizes to `target_canon`?
fn rule_reverses_to(target_canon: &str, precursor_mols: &[&Molecule], rule: &RetroRule) -> bool {
    let Some((lhs, rhs)) = rule.smirks.split_once(">>") else {
        return false;
    };
    let fwd = format!("{rhs}>>{lhs}");
    run_reactants(&fwd, precursor_mols)
        .into_iter()
        .flatten()
        .flatten()
        .any(|m| canonical_smiles(&m) == target_canon)
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
    let mol_refs: Vec<_> = reactant_mols.iter().collect();
    rules
        .iter()
        .filter(|r| !r.smirks.is_empty())
        .any(|rule| rule_reverses_to(&target_canon, &mol_refs, rule))
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
    let mol_refs: Vec<_> = reactant_mols.iter().collect();
    rule_reverses_to(&target_canon, &mol_refs, rule)
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
