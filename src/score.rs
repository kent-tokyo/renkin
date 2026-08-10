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

/// Reranker rank bonus (Issue #101 Task 35): the same [0, 0.2] scale as
/// [`template_bonus`], but keyed by a candidate's rank within its own
/// same-target merged pool (`0` = best, ranked by reranker score
/// descending, `candidate_id` ascending as a total tie-break) rather than
/// by template weight. `rank=0` of `count` gets the full 0.2 bonus,
/// `rank=count-1` gets 0.0, linearly in between. A pool of one candidate
/// (or fewer) always gets 0.0 -- there is nothing to rank it above.
/// Deliberately a REPLACEMENT for `template_bonus`/`ReactionPrior::prior`
/// at its call site, not an addition to it: summing both would push the
/// effective step cost bonus outside the calibrated [0, 0.2] range the
/// A*/beam-prune g/h split assumes.
pub fn rank_bonus(rank: usize, count: usize) -> f64 {
    if count <= 1 {
        return 0.0;
    }
    0.2 * (count - 1 - rank) as f64 / (count - 1) as f64
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

    #[test]
    fn rank_bonus_spans_full_scale_and_is_monotonic() {
        assert_eq!(rank_bonus(0, 5), 0.2);
        assert_eq!(rank_bonus(4, 5), 0.0);
        assert!((rank_bonus(2, 5) - 0.1).abs() < 1e-12);
        assert!(rank_bonus(0, 5) > rank_bonus(1, 5));
    }

    #[test]
    fn rank_bonus_degenerate_pool_is_zero() {
        assert_eq!(rank_bonus(0, 1), 0.0);
        assert_eq!(rank_bonus(0, 0), 0.0);
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
