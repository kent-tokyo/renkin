//! Route-set diversity metrics.
//!
//! Inspired by Mrugalla et al., *Generating diversity and securing
//! completeness in algorithmic retrosynthesis* (Journal of Cheminformatics,
//! 2025, https://doi.org/10.1186/s13321-025-00981-x; CC BY 4.0).  The paper's
//! CDS compares sets of bonds formed in the target.  A
//! [`Route`] in RENKIN deliberately does not retain atom mappings, so this
//! module uses the stable template IDs in a route as a conservative proxy for
//! disconnection ideas.  It must not be described as the exact paper CDS.
//! This is an independent Rust implementation of the general metric idea; no
//! paper text, figures, datasets, or upstream implementation code is included.

use std::collections::{BTreeSet, HashSet};

use crate::search::Route;

/// A route's chemical-idea signature: the set of distinct template IDs it
/// uses, independent of step order and precursor choice.
pub fn template_idea_signature(route: &Route) -> HashSet<&str> {
    route
        .steps
        .iter()
        .map(|step| step.template_id.as_str())
        .collect()
}

/// CDS-like diversity over a route set, using template/disconnection ideas.
///
/// Routes whose idea sets are strict supersets of another route are treated as
/// variations and removed from the core set. Equal signatures are represented
/// once. The score is `1 + (2 / core_count) * sum_unordered_pair_distances`,
/// the unordered-pair form of the cited paper's all-to-all normalization. It is
/// `1.0` for zero/one core route and is independent of duplicate or non-core
/// route variations.
pub fn template_disconnection_cds(routes: &[Route]) -> f64 {
    let mut signatures: Vec<BTreeSet<&str>> = routes
        .iter()
        .map(|route| {
            route
                .steps
                .iter()
                .map(|step| step.template_id.as_str())
                .collect()
        })
        .collect();
    if signatures.is_empty() {
        return 1.0;
    }

    // The core-route definition retains only one route when signatures are
    // equal. Deduplicate first so repeated serialization/search output cannot
    // change the metric merely by changing multiplicity. Sorting also fixes
    // floating-point accumulation order, making route-set input order
    // irrelevant down to the serialized f64 value.
    signatures.sort();
    signatures.dedup();

    let mut core = Vec::new();
    for signature in &signatures {
        let represented_by_shorter = signatures
            .iter()
            .any(|other| other.len() < signature.len() && other.is_subset(signature));
        if !represented_by_shorter {
            core.push(signature);
        }
    }
    if core.len() < 2 {
        return 1.0;
    }

    let mut total_distance = 0.0;
    for i in 0..core.len() {
        for j in (i + 1)..core.len() {
            let union = core[i].union(core[j]).count();
            let intersection = core[i].intersection(core[j]).count();
            total_distance += if union == 0 {
                0.0
            } else {
                1.0 - intersection as f64 / union as f64
            };
        }
    }
    1.0 + (2.0 * total_distance / core.len() as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::{AtomEconomyStatus, ReactionStep};

    fn route(ids: &[&str]) -> Route {
        Route {
            steps: ids
                .iter()
                .map(|id| ReactionStep {
                    rule: (*id).to_owned(),
                    template_id: (*id).to_owned(),
                    target: "C".to_owned(),
                    precursors: vec!["C".to_owned()],
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
                })
                .collect(),
            depth: ids.len() as u32,
            score: 0.0,
            building_blocks: Vec::new(),
            confidence: 0.0,
            convergency: 0.0,
            success_probability: 0.0,
            route_cost: 0.0,
        }
    }

    #[test]
    fn ignores_route_variations_that_add_ideas() {
        let routes = vec![route(&["a"]), route(&["a", "b"]), route(&["c"])];
        // Core routes are {a} and {c}; the superset {a,b} is a variation.
        assert!((template_disconnection_cds(&routes) - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn identical_ideas_have_minimum_score() {
        assert_eq!(
            template_disconnection_cds(&[route(&["a"]), route(&["a"])]),
            1.0
        );
        assert_eq!(template_disconnection_cds(&[]), 1.0);
    }

    #[test]
    fn duplicates_do_not_change_the_score() {
        let without_duplicate = template_disconnection_cds(&[route(&["a"]), route(&["b"])]);
        let with_duplicate =
            template_disconnection_cds(&[route(&["a"]), route(&["a"]), route(&["b"])]);
        assert_eq!(without_duplicate, 2.0);
        assert_eq!(with_duplicate, without_duplicate);
    }

    #[test]
    fn three_disjoint_core_ideas_score_three() {
        assert_eq!(
            template_disconnection_cds(&[route(&["a"]), route(&["b"]), route(&["c"])]),
            3.0
        );
    }

    #[test]
    fn strict_supersets_do_not_change_the_score() {
        let core_only = template_disconnection_cds(&[route(&["a"]), route(&["c"])]);
        let with_subset_variations = template_disconnection_cds(&[
            route(&["a"]),
            route(&["a", "b"]),
            route(&["c"]),
            route(&["c", "d"]),
        ]);
        assert_eq!(with_subset_variations, core_only);
    }

    #[test]
    fn route_order_does_not_change_the_serialized_value() {
        let forward = template_disconnection_cds(&[
            route(&["a", "b"]),
            route(&["c"]),
            route(&["d", "e", "f"]),
        ]);
        let reverse = template_disconnection_cds(&[
            route(&["d", "e", "f"]),
            route(&["c"]),
            route(&["a", "b"]),
        ]);
        assert_eq!(forward.to_bits(), reverse.to_bits());
    }
}
