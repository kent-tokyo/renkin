#![forbid(unsafe_code)]

//! SMIRKS-reversal forward validation.
//!
//! Moved out of `renkin-bench` verbatim (commit "centralize shared validation
//! helpers") — behavior is unchanged. Only SMIRKS-based rules are tried;
//! graph-based rules (empty `smirks`) are skipped here and covered by
//! [`super::graph_rules`] instead.

use chematic::rxn::run_reactants;
use chematic::smiles::canonical_smiles;

use crate::chem_env::{RetroRule, mol_from_smiles};

/// True if any SMIRKS-based rule, applied forward to `precursors`, reproduces `target`.
///
/// Tries every rule with a non-empty `smirks` (not just the one the step
/// actually used at retro time) — a forward match from a different rule still
/// confirms the transformation is chemically real.
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
    rules.iter().filter(|r| !r.smirks.is_empty()).any(|rule| {
        let Some((lhs, rhs)) = rule.smirks.split_once(">>") else {
            return false;
        };
        let fwd = format!("{rhs}>>{lhs}");
        run_reactants(&fwd, &mol_refs)
            .into_iter()
            .flatten()
            .flatten()
            .any(|m| canonical_smiles(&m) == target_canon)
    })
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
