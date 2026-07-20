#![forbid(unsafe_code)]

//! Molecular-weight atom-balance check.
//!
//! Moved out of `renkin-bench`/`renkin-mcp` verbatim (commit "centralize
//! shared validation helpers") — behavior is unchanged. This is a coarse
//! MW-only approximation, not an atom-composition-strict balance; making it
//! reagent-aware (several graph rules are intentionally single-fragment,
//! e.g. `aryl_chloride_retro`) is deferred to a follow-up PR per the current
//! hypothesis scope.

use chematic::chem::molecular_weight;

use crate::chem_env::mol_from_smiles;
use crate::search::Route;

/// True if target_MW ≤ Σ precursor_MW (within 1% float tolerance).
/// In retrosynthesis the target is split from precursors; precursors must
/// carry at least as many atoms (by weight) as the target. Violation means
/// a template caused atoms to appear from nowhere — a CompleteRXN-style defect.
pub fn step_balanced(target: &str, precursors: &[String]) -> bool {
    let target_mw = mol_from_smiles(target)
        .ok()
        .map(|m| molecular_weight(&m))
        .unwrap_or(0.0);
    if target_mw == 0.0 {
        return true;
    }
    let precursor_mw: f64 = precursors
        .iter()
        .filter_map(|s| mol_from_smiles(s).ok())
        .map(|m| molecular_weight(&m))
        .sum();
    target_mw <= precursor_mw * 1.01
}

pub fn route_balanced(route: &Route) -> bool {
    route
        .steps
        .iter()
        .all(|s| step_balanced(&s.target, &s.precursors))
}
