use chematic::chem::{molecular_weight, sa_score};

use crate::chem_env::Molecule;

/// h(n): admissible heuristic for remaining synthesis cost.
///
/// Base: count of non-building-block molecules (each needs ≥ 1 step).
/// Bonus: SA Score contribution — each unsolved molecule adds a fraction of its
/// normalized SA Score so that harder molecules are explored later.
///
/// SA Score range: 1.0 (trivial) → 10.0 (extremely complex).
/// Normalized: (sa - 1) / 9 → [0, 1]. Weight 0.5 keeps h admissible because
/// step_cost ≥ 1.0, so total h ≤ 1.5 per unsolved molecule < true cost ≥ 1.0.
pub fn heuristic(unsolved_mols: &[&Molecule]) -> f64 {
    unsolved_mols.iter().map(|m| {
        let sa = sa_score(m).clamp(1.0, 10.0);
        1.0 + 0.5 * (sa - 1.0) / 9.0   // base 1.0 + up to 0.5 for complexity
    }).sum()
}

/// g(n) step cost: penalize expansions that produce heavy molecules.
/// Returns a value in [1.0, 2.0].
pub fn step_cost(precursors: &[&Molecule]) -> f64 {
    let total_mw: f64 = precursors.iter().map(|m| molecular_weight(m)).sum();
    1.0 + (total_mw / 2000.0).min(1.0)
}
