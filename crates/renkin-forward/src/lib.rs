#![forbid(unsafe_code)]

use anyhow::Result;
use chematic::smiles::canonical_smiles;
use renkin::chem_env::{RetroRule, mol_from_smiles};
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

/// Filter out chemically invalid SMILES fragments.
///
/// Rejects SMILES that contain aromatic atoms (lowercase c/n/o/s/p) without
/// any ring-closure digits — a signature of BFS-leakage artifacts from
/// chematic's run_reactants on certain templates.
fn filter_valid_smiles(smiles_list: Vec<String>) -> Vec<String> {
    smiles_list
        .into_iter()
        .filter(|s| {
            let has_aromatic = s
                .bytes()
                .any(|b| matches!(b, b'c' | b'n' | b'o' | b's' | b'p'));
            if !has_aromatic {
                return true;
            }
            s.bytes().any(|b| b.is_ascii_digit())
        })
        .collect()
}

/// Predict forward reaction products for a given set of reactants.
///
/// Only SMIRKS-based rules are used; graph-based rules (empty `smirks` field)
/// are skipped because they have no reversible template string.
///
/// Results are sorted by template weight descending and capped at `max_results`.
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

    let mut predictions: Vec<ForwardPrediction> = rules
        .iter()
        .filter(|r| !r.smirks.is_empty())
        .filter_map(|rule| {
            let fwd = reverse_smirks(&rule.smirks)?;
            let outcomes = chematic::rxn::run_reactants(&fwd, &mol_refs).ok()?;
            if outcomes.is_empty() {
                return None;
            }
            let products: Vec<String> = outcomes
                .into_iter()
                .flat_map(|mols| mols.iter().map(canonical_smiles).collect::<Vec<_>>())
                .collect();
            let products = filter_valid_smiles(products);
            if products.is_empty() {
                return None;
            }
            Some(ForwardPrediction {
                template: rule.name.clone(),
                products,
                weight: rule.weight,
            })
        })
        .collect();

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
    fn test_filter_valid_smiles() {
        let valid = "CC(=O)O".to_string();
        let invalid = "cccc".to_string(); // aromatic without ring closure
        let result = filter_valid_smiles(vec![valid.clone(), invalid]);
        assert_eq!(result, vec![valid]);
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
