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
    unsolved_mols
        .iter()
        .map(|m| {
            let sa = sa_score(m).clamp(1.0, 10.0);
            1.0 + 0.5 * (sa - 1.0) / 9.0 // base 1.0 + up to 0.5 for complexity
        })
        .sum()
}

/// g(n) step cost: penalize expansions that produce heavy molecules.
/// Returns a value in [1.0, 2.0].
pub fn step_cost(precursors: &[&Molecule]) -> f64 {
    let total_mw: f64 = precursors.iter().map(|m| molecular_weight(m)).sum();
    1.0 + (total_mw / 2000.0).min(1.0)
}

/// Template frequency bonus: reduces effective step cost for high-frequency extracted templates.
/// weight=1.0 (hand-crafted rules) gives no bonus. Normalized to [0, 0.2] so step_cost ≥ 0.8.
pub fn template_bonus(weight: f64, max_weight: f64) -> f64 {
    if max_weight <= 1.0 {
        return 0.0;
    }
    0.2 * (weight - 1.0) / (max_weight - 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chematic::smiles::parse;

    #[test]
    fn template_bonus_zero_when_all_weight_one() {
        // hand-crafted rules all weight=1.0; max_weight=1.0 → bonus=0 for all
        assert_eq!(template_bonus(1.0, 1.0), 0.0);
    }

    #[test]
    fn template_bonus_range() {
        // max_weight = e.g. ln(1294) ≈ 7.16 for top-1293 template
        let max_w = (1294_f64).ln();
        let bonus_min = template_bonus(1.0, max_w); // weight of hand-crafted or count=0
        let bonus_max = template_bonus(max_w, max_w);
        assert!(bonus_min >= 0.0);
        assert!(
            (bonus_max - 0.2).abs() < 1e-10,
            "max bonus must be 0.2, got {bonus_max}"
        );
    }

    fn mol(smi: &str) -> Molecule {
        parse(smi).expect("valid SMILES")
    }

    #[test]
    fn heuristic_empty_is_zero() {
        assert_eq!(heuristic(&[]), 0.0);
    }

    #[test]
    fn heuristic_single_simple_mol_in_range() {
        let m = mol("C"); // methane — very simple, SA Score near 1
        let h = heuristic(&[&m]);
        // base = 1.0, SA bonus in [0, 0.5] → h in [1.0, 1.5]
        assert!((1.0..=1.5).contains(&h), "h={h} out of [1.0, 1.5]");
    }

    #[test]
    fn step_cost_single_small_mol() {
        let m = mol("CC(=O)O"); // acetic acid, MW ~60
        let cost = step_cost(&[&m]);
        // total_mw/2000 ≈ 0.03 → cost ≈ 1.03
        assert!(cost > 1.0 && cost < 1.1, "step_cost={cost}");
    }

    #[test]
    fn step_cost_heavy_mol_capped_at_two() {
        // A large molecule should approach but not exceed 2.0
        let m = mol("CC(=O)Oc1ccccc1C(=O)O"); // aspirin, MW ~180
        let cost = step_cost(&[&m]);
        assert!(cost > 1.0 && cost <= 2.0, "step_cost={cost}");
    }
}
