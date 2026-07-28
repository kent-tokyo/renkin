#![forbid(unsafe_code)]

use anyhow::Result;
use chematic::smiles::canonical_smiles;
use renkin::chem_env::{Molecule, RetroRule, mol_from_smiles};
use renkin::search::Route;
use serde::Serialize;

/// A predicted forward reaction outcome.
#[derive(Debug, Clone, Serialize)]
pub struct ForwardPrediction {
    /// Rule name that produced this prediction.
    pub template: String,
    /// Predicted product SMILES (may be multiple per reaction outcome).
    pub products: Vec<String>,
    /// Template frequency weight (higher = more common in training data).
    pub weight: f64,
}

/// Forward-validation result for one step of a retrosynthetic route.
#[derive(Debug, Serialize)]
pub struct StepValidation {
    pub step_index: usize,
    /// The expected product (the step's target in the retro route).
    pub target: String,
    /// Whether the forward prediction reproduced the target (canonical SMILES match).
    pub verified: bool,
    /// Top forward predictions for this step's precursors.
    pub top_predictions: Vec<ForwardPrediction>,
}

/// Reverse a retro SMIRKS string to obtain a forward SMIRKS.
///
/// Retro direction:  `product_pattern >> precursor_pattern`
/// Forward direction: `precursor_pattern >> product_pattern`
fn reverse_smirks(smirks: &str) -> Option<String> {
    let (lhs, rhs) = smirks.split_once(">>")?;
    Some(format!("{rhs}>>{lhs}"))
}

/// Validate and canonicalize one reaction outcome's products.
///
/// `run_reactants` returns one `Vec<Molecule>` per independent reaction
/// outcome; every molecule in that `Vec` must be kept together as a single
/// candidate (see [`predict_products`] docs). This function decides whether
/// that outcome, as a whole, is acceptable:
///
/// - Each product is canonicalized, then the canonical SMILES is re-parsed
///   (round-trip check). If *any* product in the outcome fails this, the
///   whole outcome is rejected — a partially-valid outcome is not "fixed" by
///   dropping only the bad product, since that would silently change what
///   reaction actually happened.
/// - An outcome with zero products is rejected.
/// - An outcome whose canonical product multiset equals the canonical
///   reactant multiset (a no-op transformation) is rejected.
///
/// Ring-closure-digit string heuristics were deliberately removed: whether a
/// SMILES has a ring-closure digit is not a general test of chemical
/// validity. Round-tripping through the real parser is.
fn validate_outcome(outcome: &[Molecule], reactant_canon: &[String]) -> Option<Vec<String>> {
    if outcome.is_empty() {
        return None;
    }
    let mut products = Vec::with_capacity(outcome.len());
    for mol in outcome {
        let canon = canonical_smiles(mol);
        mol_from_smiles(&canon).ok()?;
        products.push(canon);
    }
    products.sort_unstable();
    if products == reactant_canon {
        return None; // no-op transformation
    }
    Some(products)
}

/// Predict forward reaction products for a given set of reactants.
///
/// Only SMIRKS-based rules are used; graph-based rules (empty `smirks` field)
/// are skipped because they have no reversible template string.
///
/// `run_reactants` may return several independent reaction outcomes for one
/// template (e.g. it matches the reactants in more than one way); each
/// outcome becomes its own [`ForwardPrediction`] entry — outcomes are never
/// flattened together, so `products` on a given entry always reflects one
/// coherent set of products from one reaction event, and the same template
/// name may legitimately appear more than once in the result.
///
/// Results are sorted by template weight descending and capped at
/// `max_results` after outcomes have been separated.
pub fn predict_products(
    reactants: &[&str],
    rules: &[RetroRule],
    max_results: usize,
) -> Result<Vec<ForwardPrediction>> {
    let reactant_mols: Vec<_> = reactants
        .iter()
        .filter_map(|s| mol_from_smiles(s).ok())
        .collect();

    if reactant_mols.len() != reactants.len() {
        anyhow::bail!("one or more reactant SMILES failed to parse");
    }

    let mol_refs: Vec<_> = reactant_mols.iter().collect();

    let mut reactant_canon: Vec<String> = reactant_mols.iter().map(canonical_smiles).collect();
    reactant_canon.sort_unstable();

    let mut predictions: Vec<ForwardPrediction> = Vec::new();
    for rule in rules.iter().filter(|r| !r.smirks.is_empty()) {
        let Some(fwd) = reverse_smirks(&rule.smirks) else {
            continue;
        };
        let Ok(outcomes) = chematic::rxn::run_reactants(&fwd, &mol_refs) else {
            continue;
        };
        for outcome in &outcomes {
            let Some(products) = validate_outcome(outcome, &reactant_canon) else {
                continue;
            };
            predictions.push(ForwardPrediction {
                template: rule.name.clone(),
                products,
                weight: rule.weight,
            });
        }
    }

    predictions.sort_unstable_by(|a, b| {
        b.weight
            .partial_cmp(&a.weight)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    predictions.truncate(max_results);
    Ok(predictions)
}

/// Validate each step in a retrosynthetic route using forward reaction prediction.
///
/// For each step, applies forward prediction to the step's precursors and
/// checks whether the canonical SMILES of the step's target appears in the
/// predicted products.
pub fn validate_route(route: &Route, rules: &[RetroRule]) -> Result<Vec<StepValidation>> {
    let mut validations = Vec::with_capacity(route.steps.len());

    for (i, step) in route.steps.iter().enumerate() {
        let reactant_refs: Vec<&str> = step.precursors.iter().map(|s| s.as_str()).collect();
        let top_predictions = predict_products(&reactant_refs, rules, 5)?;

        let target_canon = mol_from_smiles(&step.target)
            .ok()
            .map(|m| canonical_smiles(&m))
            .unwrap_or_else(|| step.target.clone());

        let verified = top_predictions
            .iter()
            .any(|p| p.products.contains(&target_canon));

        validations.push(StepValidation {
            step_index: i,
            target: step.target.clone(),
            verified,
            top_predictions,
        });
    }

    Ok(validations)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reverse_smirks() {
        let retro = "[C:1](=[O:2])[O:3]>>[C:1](=[O:2])O.[O:3]";
        let fwd = reverse_smirks(retro).unwrap();
        assert!(fwd.starts_with("[C:1](=[O:2])O.[O:3]>>"));
    }

    #[test]
    fn validate_outcome_rejects_empty_outcome() {
        assert!(validate_outcome(&[], &[]).is_none());
    }

    #[test]
    fn validate_outcome_rejects_no_op_transformation() {
        let mol = mol_from_smiles("CCO").unwrap();
        let reactant_canon = vec![canonical_smiles(&mol)];
        assert!(validate_outcome(&[mol], &reactant_canon).is_none());
    }

    #[test]
    fn validate_outcome_accepts_and_sorts_real_transformation() {
        let a = mol_from_smiles("CCO").unwrap(); // ethanol
        let b = mol_from_smiles("CC(=O)O").unwrap(); // acetic acid (different from reactants)
        let reactant_canon = vec!["CN".to_string()]; // unrelated reactant, so this isn't a no-op
        let products = validate_outcome(&[b, a], &reactant_canon).unwrap();
        // sorted lexicographically regardless of input order
        let mut expected = vec![
            canonical_smiles(&mol_from_smiles("CCO").unwrap()),
            canonical_smiles(&mol_from_smiles("CC(=O)O").unwrap()),
        ];
        expected.sort_unstable();
        assert_eq!(products, expected);
    }

    /// Regression fixture for outcome separation, verified empirically against
    /// a real `chematic::rxn::run_reactants` call (not hypothesized): a
    /// hand-authored halide-metathesis SMIRKS applied to two dihalides with
    /// non-equivalent halogen sites returns 4 independent raw outcomes, each
    /// with 2 products. One of the 4 (the combination that reassigns each
    /// molecule's halogens back to its own starting arrangement) is a genuine
    /// no-op and is correctly filtered, leaving 3 -- confirmed by checking
    /// each raw outcome's product pair against the reactants' canonical forms
    /// directly (see the `main` probe this fixture was built from). The
    /// surviving outcomes' product *pairings* differ even though individual
    /// products repeat across them, which is exactly the information a flat
    /// `flat_map` would destroy by merging everything into one
    /// undifferentiated product bag.
    #[test]
    fn outcomes_are_never_flattened_together() {
        // Retro-direction smirks (predict_products reverses this internally).
        let retro_smirks = "[C:1][Br:4].[C:3][Cl:2]>>[C:1][Cl:2].[C:3][Br:4]";
        let rule = RetroRule {
            name: "synthetic_halide_metathesis".to_string(),
            template_id: "rule:synthetic_halide_metathesis".to_string(),
            smirks: retro_smirks.to_string(),
            weight: 1.0,
            required_elements: 0,
        };
        let result = predict_products(&["ClCC(Cl)CBr", "BrCC(Br)CCl"], &[rule], 10).unwrap();

        assert_eq!(
            result.len(),
            3,
            "expected 3 surviving outcomes (4 raw outcomes minus 1 genuine no-op), got {result:?}"
        );
        for entry in &result {
            assert_eq!(
                entry.products.len(),
                2,
                "each outcome from this fixture has exactly 2 products, got {entry:?}"
            );
        }

        // The surviving outcomes' product pairs, as observed empirically --
        // every pair is distinct even though individual products repeat
        // across pairs, which is exactly what would be lost by flattening.
        let mut pairs: Vec<Vec<String>> = result.iter().map(|p| p.products.clone()).collect();
        pairs.sort();
        pairs.dedup();
        assert_eq!(
            pairs.len(),
            3,
            "all 3 surviving outcomes must have distinct product pairings, got {pairs:?}"
        );

        // The no-op outcome (reactants' own canonical forms, paired back to
        // each other) must never appear among the results.
        let mut reactant_canon = vec![
            canonical_smiles(&mol_from_smiles("ClCC(Cl)CBr").unwrap()),
            canonical_smiles(&mol_from_smiles("BrCC(Br)CCl").unwrap()),
        ];
        reactant_canon.sort_unstable();
        assert!(
            !pairs.contains(&reactant_canon),
            "the no-op outcome must be filtered, not returned as a candidate"
        );
    }

    #[test]
    fn test_predict_products_does_not_panic() {
        let rules = renkin::chem_env::default_rules();
        // acetic acid + ethanol — ester_cleavage reverse may or may not match
        let result = predict_products(&["CC(=O)O", "CCO"], &rules, 5);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_route_smoke() {
        use renkin::chem_env::ChemEnv;
        use renkin::search::{SearchConfig, find_routes};

        let env = ChemEnv::in_memory(&["CC(=O)O", "Oc1ccccc1C(=O)O"]);
        let rules = renkin::chem_env::default_rules();
        let cfg = SearchConfig {
            max_depth: 2,
            max_routes: 1,
            ..Default::default()
        };
        let (routes, _) = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg).unwrap();
        if let Some(route) = routes.first() {
            let v = validate_route(route, &rules).unwrap();
            assert!(!v.is_empty());
        }
    }
}
