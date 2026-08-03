//! Target-element accounting (design doc §4.5): a **directional, one-way**
//! per-element heavy-atom accounting check. Explicitly **not mass
//! conservation** -- never describe or name it that way anywhere in this
//! file. For each step, for each element present in the *target* (hydrogen
//! excluded), the target's heavy-atom count for that element must not
//! exceed the sum of that element's heavy-atom count over the step's
//! precursors. Precursors carrying *more* of an element than the target
//! needs is never a failure -- a leaving group, protecting group, or
//! reagent is expected to bring atoms the target doesn't keep. Only the
//! target needing more of an element than the precursors supply is a
//! problem.
//!
//! No existing Rust code implements these exact semantics (design doc §2 --
//! `atom_conservation` is MW-sum-only and clamp-hidden, `graph_rules.rs`'s
//! `element_counts` is H-inclusive and delta-equality-shaped, not a
//! one-directional inequality), so this is a fresh implementation.

use std::collections::HashMap;

use chematic::core::Element;

use crate::chem_env::mol_from_smiles;
use crate::synthesizability::schema::ElementAccountingStatus;

pub(crate) struct ElementAccountingResult {
    pub status: ElementAccountingStatus,
    /// Indices into `route.steps` with at least one element where the
    /// target's heavy-atom count exceeds the sum over that step's
    /// precursors. Empty unless `status == UnaccountedTargetElement`.
    pub failing_step_indices: Vec<usize>,
}

/// Heavy-atom (hydrogen-excluded) element counts for one SMILES string.
/// Matches `scripts/compare_validation.py`'s `_heavy_atom_counts`, which
/// explicitly does `if atom.GetSymbol() != "H"`: every atom present in the
/// parsed molecule is counted except hydrogen. Unlike
/// `validation::graph_rules::element_counts`, this deliberately does NOT
/// add back implicit hydrogens -- this check is heavy-atom-only by design,
/// not H-inclusive. `None` if the SMILES fails to parse under chematic.
fn heavy_atom_counts(smiles: &str) -> Option<HashMap<Element, usize>> {
    let mol = mol_from_smiles(smiles).ok()?;
    let mut counts: HashMap<Element, usize> = HashMap::new();
    for (_, atom) in mol.atoms() {
        if atom.element != Element::H {
            *counts.entry(atom.element).or_insert(0) += 1;
        }
    }
    Some(counts)
}

/// Directional, one-way per-element heavy-atom accounting (design doc
/// §4.5) -- **NOT mass conservation**.
///
/// Cross-language parity note, deliberate design choice matching
/// `compare_validation.py`'s `check_target_element_accounting` exactly (the
/// design doc calls out this exact-parity requirement for Agent D's later
/// cross-language test, and flags that the *only* underspecified point is
/// this any-evaluated-across-the-whole-route rule): a step is skipped
/// entirely (not evaluated, not counted as failing) if its target or ANY of
/// its precursors fails to parse. `NotEvaluable` is returned for the whole
/// route only when literally **no step** could be evaluated at all -- if
/// some steps parse and evaluate cleanly while others don't, the cleanly-
/// evaluated ones still determine `Accounted` vs.
/// `UnaccountedTargetElement`; a route is only ever `NotEvaluable` as a
/// whole when its evaluable-step count is exactly zero. This mirrors the
/// Python reference's `any_evaluated` flag precisely (it is set inside the
/// per-node walk only when that node's target and all its children parse).
///
/// One consequence worth documenting explicitly: a zero-step route (the
/// target itself is already a stock leaf -- see `search::find_routes`'s
/// documented depth-0 case) is `NotEvaluable`, not `Accounted`, because
/// there is no step to walk and `any_evaluated` never becomes `true`. This
/// looks surprising for a trivially-"nothing to check" route, but it is
/// exactly what the Python reference does for an empty walk, and matching
/// that reference takes priority per the design doc's parity requirement.
pub(crate) fn compute_element_accounting(route: &crate::search::Route) -> ElementAccountingResult {
    let mut any_evaluated = false;
    let mut failing_step_indices = Vec::new();

    for (idx, step) in route.steps.iter().enumerate() {
        let Some(target_counts) = heavy_atom_counts(&step.target) else {
            continue;
        };

        let mut precursor_counts: HashMap<Element, usize> = HashMap::new();
        let mut all_precursors_parsed = true;
        for precursor in &step.precursors {
            match heavy_atom_counts(precursor) {
                Some(counts) => {
                    for (element, n) in counts {
                        *precursor_counts.entry(element).or_insert(0) += n;
                    }
                }
                None => {
                    all_precursors_parsed = false;
                    break;
                }
            }
        }
        if !all_precursors_parsed {
            continue;
        }

        any_evaluated = true;
        let step_fails = target_counts
            .iter()
            .any(|(element, n)| *n > precursor_counts.get(element).copied().unwrap_or(0));
        if step_fails {
            failing_step_indices.push(idx);
        }
    }

    if !any_evaluated {
        return ElementAccountingResult {
            status: ElementAccountingStatus::NotEvaluable,
            failing_step_indices: Vec::new(),
        };
    }

    if failing_step_indices.is_empty() {
        ElementAccountingResult {
            status: ElementAccountingStatus::Accounted,
            failing_step_indices: Vec::new(),
        }
    } else {
        ElementAccountingResult {
            status: ElementAccountingStatus::UnaccountedTargetElement,
            failing_step_indices,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::{AtomEconomyStatus, ReactionStep, Route};

    fn step(rule: &str, target: &str, precursors: &[&str]) -> ReactionStep {
        ReactionStep {
            rule: rule.to_string(),
            template_id: format!("rule:{rule}"),
            target: target.to_string(),
            precursors: precursors.iter().map(|s| s.to_string()).collect(),
            conditions: None,
            atom_economy: None,
            atom_economy_raw_percent: None,
            atom_economy_status: AtomEconomyStatus::NotEvaluable,
            step_confidence: 1.0,
            procedure_hint: None,
            reaction_family: None,
            metadata_source: None,
            metadata_scope: None,
            evidence: None,
        }
    }

    fn route(steps: Vec<ReactionStep>) -> Route {
        Route {
            steps,
            depth: 1,
            score: 0.0,
            building_blocks: vec![],
            confidence: 1.0,
            convergency: 1.0,
            success_probability: 1.0,
            route_cost: 1.0,
        }
    }

    #[test]
    fn clean_case_is_accounted() {
        // Ester hydrolysis: phenyl acetate -> acetic acid + phenol. Every
        // target element (C, H, O) is covered by the precursor pool (the
        // extra water-derived H/O the precursors bring is fine -- excess
        // supply is never a failure).
        let r = route(vec![step(
            "ester_cleavage",
            "CC(=O)Oc1ccccc1",
            &["CC(=O)O", "Oc1ccccc1"],
        )]);
        let result = compute_element_accounting(&r);
        assert_eq!(result.status, ElementAccountingStatus::Accounted);
        assert!(result.failing_step_indices.is_empty());
    }

    #[test]
    fn clear_violation_is_unaccounted() {
        // Target has a bromine the single precursor (benzene, no Br at
        // all) cannot supply -- a genuine unaccounted-element case.
        let r = route(vec![step("suzuki_retro", "Brc1ccccc1", &["c1ccccc1"])]);
        let result = compute_element_accounting(&r);
        assert_eq!(
            result.status,
            ElementAccountingStatus::UnaccountedTargetElement
        );
        assert_eq!(result.failing_step_indices, vec![0]);
    }

    #[test]
    fn precursor_excess_is_not_a_failure() {
        // Precursors collectively supply MORE carbon than the target needs
        // (a dropped/untracked byproduct) -- must not be flagged.
        let r = route(vec![step("amide_cleavage", "CC(N)=O", &["CC(=O)O", "CCN"])]);
        let result = compute_element_accounting(&r);
        assert_eq!(result.status, ElementAccountingStatus::Accounted);
    }

    #[test]
    fn unparseable_target_makes_step_not_evaluable_but_other_steps_still_count() {
        // First step's target fails to parse -> skipped, not counted as a
        // failure. Second step is a genuine violation -> the route-level
        // status must still be UnaccountedTargetElement (parity with
        // Python's any_evaluated semantics), not NotEvaluable.
        let r = route(vec![
            // Unclosed bracket is guaranteed to be rejected by the SMILES
            // parser (same convention as `search.rs`'s
            // `invalid_smiles_returns_err` test).
            step("bogus_rule", "[C(", &["CCO"]),
            step("suzuki_retro", "Brc1ccccc1", &["c1ccccc1"]),
        ]);
        let result = compute_element_accounting(&r);
        assert_eq!(
            result.status,
            ElementAccountingStatus::UnaccountedTargetElement
        );
        assert_eq!(result.failing_step_indices, vec![1]);
    }

    #[test]
    fn all_steps_unparseable_is_not_evaluable() {
        let r = route(vec![step("bogus_rule", "[C(", &["[N("])]);
        let result = compute_element_accounting(&r);
        assert_eq!(result.status, ElementAccountingStatus::NotEvaluable);
        assert!(result.failing_step_indices.is_empty());
    }

    #[test]
    fn zero_step_route_is_not_evaluable() {
        // Matches Python's `any_evaluated` semantics for an empty walk --
        // see this function's doc comment.
        let r = route(vec![]);
        let result = compute_element_accounting(&r);
        assert_eq!(result.status, ElementAccountingStatus::NotEvaluable);
    }
}
