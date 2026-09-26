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

use chematic::smarts::parse_smarts;

use crate::bridge::route_graph::{ReactionEvidence, RouteDocument};
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

/// Exact formed-bond CDS for atom-mapped Bridge route documents.
///
/// This deliberately accepts [`RouteDocument`] rather than [`Route`]: native
/// search routes do not retain atom mapping. Every step must carry a mapped
/// reaction evidence record; if one step is unmapped or malformed, the result
/// is `None` rather than a proxy or a partial score. Bond order is intentionally
/// excluded from the identity, matching the formed-bond endpoint definition;
/// order changes are not new formed bonds.
pub fn atom_mapped_formed_bond_cds(routes: &[RouteDocument]) -> Option<f64> {
    let mut signatures = Vec::with_capacity(routes.len());
    for route in routes {
        let mut signature = BTreeSet::new();
        for step in route.steps() {
            let smirks = match step.reaction_evidence.as_ref()? {
                ReactionEvidence::RenkinTemplate {
                    declared_smirks: Some(smirks),
                    ..
                }
                | ReactionEvidence::AiZynthFinderTemplate { smirks, .. }
                | ReactionEvidence::SyntheseusReaction {
                    reaction_smiles: smirks,
                }
                | ReactionEvidence::SynPlannerReaction { smiles: smirks, .. } => smirks,
                ReactionEvidence::RenkinTemplate {
                    declared_smirks: None,
                    ..
                } => return None,
            };
            let (lhs, rhs) = smirks.split_once(">>")?;
            let lhs = mapped_bonds(lhs)?;
            let rhs = mapped_bonds(rhs)?;
            for bond in rhs.difference(&lhs) {
                signature.insert(*bond);
            }
        }
        if signature.is_empty() {
            return None;
        }
        signatures.push(signature);
    }
    Some(cds_from_signatures(signatures))
}

fn mapped_bonds(side: &str) -> Option<BTreeSet<(u16, u16)>> {
    let mut bonds = BTreeSet::new();
    for fragment in side.split('.') {
        let query = parse_smarts(fragment).ok()?;
        for bond in &query.bonds {
            let Some(a) = query.atoms.get(bond.atom1)?.atom_map else {
                continue;
            };
            let Some(b) = query.atoms.get(bond.atom2)?.atom_map else {
                continue;
            };
            bonds.insert(if a < b { (a, b) } else { (b, a) });
        }
    }
    Some(bonds)
}

fn cds_from_signatures(mut signatures: Vec<BTreeSet<(u16, u16)>>) -> f64 {
    if signatures.is_empty() {
        return 1.0;
    }
    signatures.sort();
    signatures.dedup();
    let core: Vec<_> = signatures
        .iter()
        .filter(|signature| {
            !signatures
                .iter()
                .any(|other| other.len() < signature.len() && other.is_subset(signature))
        })
        .collect();
    if core.len() < 2 {
        return 1.0;
    }
    let mut distance = 0.0;
    for i in 0..core.len() {
        for j in (i + 1)..core.len() {
            let union = core[i].union(core[j]).count();
            let intersection = core[i].intersection(core[j]).count();
            distance += 1.0 - intersection as f64 / union as f64;
        }
    }
    1.0 + 2.0 * distance / core.len() as f64
}

/// Reaction-level identity set of a route: one key per distinct step,
/// `sorted precursors>>target`, independent of step order.
pub fn reaction_set(route: &Route) -> BTreeSet<String> {
    route
        .steps
        .iter()
        .map(|step| {
            let mut precursors: Vec<&str> = step.precursors.iter().map(String::as_str).collect();
            precursors.sort_unstable();
            format!("{}>>{}", precursors.join("."), step.target)
        })
        .collect()
}

/// Jaccard distance between two routes' [`reaction_set`]s (Syntheseus
/// `reaction_jaccard_distance` parity). `0.0` for identical sets, including
/// two depth-0 routes; `1.0` when the routes share no reaction.
pub fn reaction_jaccard_distance(a: &Route, b: &Route) -> f64 {
    let sa = reaction_set(a);
    let sb = reaction_set(b);
    let union = sa.union(&sb).count();
    if union == 0 {
        return 0.0;
    }
    let intersection = sa.intersection(&sb).count();
    1.0 - intersection as f64 / union as f64
}

/// Default packing radius: routes must be reaction-disjoint (Jaccard
/// distance 1) to count as distinct.
pub const DEFAULT_PACKING_RADIUS: f64 = 0.999;

/// Recursion budget for the exact packing search before it reports its best
/// packing found so far as a lower bound.
const PACKING_SEARCH_BUDGET: u64 = 200_000;

/// Route-set packing number (Syntheseus "number of diverse routes" parity).
#[derive(Debug, Clone, serde::Serialize, PartialEq)]
pub struct PackingEstimate {
    pub schema_version: u32,
    pub distance: &'static str,
    /// Two routes are distinct when their distance is strictly greater than
    /// this radius.
    pub radius: f64,
    pub routes: usize,
    /// Size of the largest found subset of pairwise-distinct routes.
    pub packing_number: usize,
    /// `"exact"` when the search proved optimality, otherwise
    /// `"lower_bound"` (budget exhausted; the value is still achievable).
    pub method: &'static str,
    /// Indices (route order) of one maximum packing found.
    pub packing_route_indices: Vec<usize>,
}

/// Largest subset of `routes` whose pairwise [`reaction_jaccard_distance`]
/// exceeds `radius`. Exact branch-and-bound (maximum independent set on the
/// "too-similar" conflict graph) within a fixed deterministic budget; if the
/// budget runs out the best packing found is reported as a lower bound.
/// Ties are broken toward earlier (better-ranked) routes.
pub fn route_packing_number(routes: &[Route], radius: f64) -> PackingEstimate {
    let n = routes.len();
    let sets: Vec<BTreeSet<String>> = routes.iter().map(reaction_set).collect();
    let distance = |i: usize, j: usize| -> f64 {
        let union = sets[i].union(&sets[j]).count();
        if union == 0 {
            0.0
        } else {
            1.0 - sets[i].intersection(&sets[j]).count() as f64 / union as f64
        }
    };
    let conflicts: Vec<Vec<bool>> = (0..n)
        .map(|i| {
            (0..n)
                .map(|j| i != j && distance(i.min(j), i.max(j)) <= radius)
                .collect()
        })
        .collect();

    struct Search<'a> {
        conflicts: &'a [Vec<bool>],
        best: Vec<usize>,
        steps: u64,
        exhausted: bool,
    }
    impl Search<'_> {
        fn run(&mut self, chosen: &mut Vec<usize>, candidates: &[usize]) {
            self.steps += 1;
            if self.steps > PACKING_SEARCH_BUDGET {
                self.exhausted = true;
                return;
            }
            if candidates.is_empty() {
                if chosen.len() > self.best.len() {
                    self.best = chosen.clone();
                }
                return;
            }
            if chosen.len() + candidates.len() <= self.best.len() {
                return; // cannot beat the incumbent
            }
            let (&first, rest) = candidates.split_first().expect("non-empty");
            // Include `first` (earlier routes preferred on ties).
            let compatible: Vec<usize> = rest
                .iter()
                .copied()
                .filter(|&c| !self.conflicts[first][c])
                .collect();
            chosen.push(first);
            self.run(chosen, &compatible);
            chosen.pop();
            if self.exhausted {
                return;
            }
            // Exclude `first`.
            self.run(chosen, rest);
        }
    }

    let mut search = Search {
        conflicts: &conflicts,
        best: Vec::new(),
        steps: 0,
        exhausted: false,
    };
    let all: Vec<usize> = (0..n).collect();
    search.run(&mut Vec::new(), &all);
    let mut best = search.best;
    best.sort_unstable();
    PackingEstimate {
        schema_version: 1,
        distance: "reaction_jaccard",
        radius,
        routes: n,
        packing_number: best.len(),
        method: if search.exhausted {
            "lower_bound"
        } else {
            "exact"
        },
        packing_route_indices: best,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::{AtomEconomyStatus, ReactionStep};

    fn step(target: &str, precursors: &[&str]) -> ReactionStep {
        ReactionStep {
            rule: "r".to_owned(),
            template_id: "rule:r".to_owned(),
            target: target.to_owned(),
            precursors: precursors.iter().map(|p| (*p).to_owned()).collect(),
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

    fn steps_route(steps: Vec<ReactionStep>) -> Route {
        Route {
            depth: steps.len() as u32,
            steps,
            score: 0.0,
            building_blocks: Vec::new(),
            confidence: 0.0,
            convergency: 0.0,
            success_probability: 0.0,
            route_cost: 0.0,
        }
    }

    #[test]
    fn reaction_jaccard_distance_ignores_precursor_order() {
        let a = steps_route(vec![step("T", &["A", "B"]), step("A", &["C"])]);
        let b = steps_route(vec![step("T", &["B", "A"]), step("A", &["D"])]);
        let c = steps_route(vec![step("T", &["X", "Y"])]);
        assert_eq!(reaction_jaccard_distance(&a, &a), 0.0);
        // One shared reaction of three distinct ones.
        assert!((reaction_jaccard_distance(&a, &b) - (1.0 - 1.0 / 3.0)).abs() < 1e-12);
        assert_eq!(reaction_jaccard_distance(&a, &c), 1.0);
        let buy = steps_route(Vec::new());
        assert_eq!(reaction_jaccard_distance(&buy, &buy), 0.0);
    }

    #[test]
    fn packing_number_counts_reaction_disjoint_routes_exactly() {
        let routes = vec![
            steps_route(vec![step("T", &["A", "B"]), step("A", &["C"])]),
            steps_route(vec![step("T", &["A", "B"]), step("A", &["D"])]), // shares T>>A.B
            steps_route(vec![step("T", &["X", "Y"])]),
            steps_route(vec![step("T", &["P", "Q"])]),
            steps_route(vec![step("T", &["X", "Y"]), step("X", &["Z"])]), // shares T>>X.Y
        ];
        let estimate = route_packing_number(&routes, DEFAULT_PACKING_RADIUS);
        assert_eq!(estimate.method, "exact");
        assert_eq!(estimate.packing_number, 3);
        assert_eq!(estimate.packing_route_indices, vec![0, 2, 3]);

        // radius < 0 makes every pair distinct.
        assert_eq!(route_packing_number(&routes, -1.0).packing_number, 5);
        assert_eq!(
            route_packing_number(&[], DEFAULT_PACKING_RADIUS).packing_number,
            0
        );
    }

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

    #[test]
    fn mapped_bond_delta_ignores_unmapped_reagent_bonds() {
        let lhs = mapped_bonds("[C:1]-[O:2].O").unwrap();
        let rhs = mapped_bonds("[C:1]-[O:2]-[CH3:3].O").unwrap();
        assert_eq!(lhs, BTreeSet::from([(1, 2)]));
        assert_eq!(rhs, BTreeSet::from([(1, 2), (2, 3)]));
    }

    #[test]
    fn exact_cds_uses_formed_bond_endpoint_sets() {
        let a = BTreeSet::from([(1, 2)]);
        let b = BTreeSet::from([(3, 4)]);
        assert_eq!(cds_from_signatures(vec![a, b]), 2.0);
    }
}
