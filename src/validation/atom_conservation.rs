#![forbid(unsafe_code)]

//! Molecular-weight atom-balance check.
//!
//! Moved out of `renkin-bench`/`renkin-mcp` verbatim (commit "centralize
//! shared validation helpers") — behavior is unchanged. This is a coarse
//! MW-only approximation, not an atom-composition-strict balance; making it
//! reagent-aware (several graph rules are intentionally single-fragment,
//! e.g. `boc_deprotection_retro` discarding the volatile isobutylene/CO2
//! byproduct) is deferred to a follow-up PR per the current hypothesis scope.

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

#[cfg(test)]
mod tests {
    use super::*;

    /// 31.11 regression: chlorobenzene -> [benzene] (the exact shape
    /// aryl_chloride_retro / aryl_iodide_retro / aryl_fluoride_snAr_retro
    /// used to produce before they were removed from default_rules() in
    /// chem_env.rs) must be flagged unbalanced — the Cl atom vanished with
    /// no tracked precursor accounting for it.
    #[test]
    fn halobenzene_to_bare_benzene_is_unbalanced() {
        assert!(!step_balanced("Clc1ccccc1", &["c1ccccc1".to_string()]));
        assert!(!step_balanced("Ic1ccccc1", &["c1ccccc1".to_string()]));
        assert!(!step_balanced("Fc1ccccc1", &["c1ccccc1".to_string()]));
    }

    /// Control: aryl_chloride_to_bromide's halogen swap (Cl -> Br) keeps
    /// the atom count non-decreasing and must still read as balanced.
    #[test]
    fn halogen_swap_stays_balanced() {
        assert!(step_balanced("Clc1ccccc1", &["Brc1ccccc1".to_string()]));
    }
}
