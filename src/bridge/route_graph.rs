//! Tool-neutral route DAG: promotes `scripts/compare_route_graph.py`'s
//! `RouteNode`/`RouteGraph`/`normalize_renkin_route` into Rust. See
//! `crate::bridge` module docs for the parity contract with that file.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::bridge::audit::AuditFindingCode;
use crate::chem_env::{mol_from_smiles, to_canonical};
use crate::search::Route;

/// Which tool produced the route being audited. A closed, 2-variant set --
/// mirrors `compare_schema.py`'s `VALID_TOOLS` frozenset. RENKIN Bridge
/// PR4 adds the AiZynthFinder JSON adapter that actually constructs an
/// `AiZynthFinder`-sourced [`RouteDocument`]; this type exists now so
/// `AuditReport::source` has a home for it ahead of that.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteSource {
    Renkin,
    AiZynthFinder,
}

/// One node in the normalized, tool-neutral route tree. Mirrors
/// `compare_route_graph.py`'s `RouteNode` exactly: `is_stock_leaf` is
/// three-valued and never defaulted -- `None` (a leaf with no explicit
/// resolved/unresolved flag either way) is itself a defect
/// (`AmbiguousLeafStatus`), never silently treated as `true` or `false`.
#[derive(Debug, Clone, Serialize)]
pub struct RouteNode {
    pub canonical_smiles: String,
    pub is_stock_leaf: Option<bool>,
    pub children: Vec<RouteNode>,
}

/// One decomposition step, flattened out of the tree for reporting. Not
/// separately stored on [`RouteDocument`] -- derived on demand by
/// [`RouteDocument::steps`] from `root`, the document's single source of
/// truth, so there is nothing to keep in sync between a stored step list
/// and the tree.
#[derive(Debug, Clone, Serialize)]
pub struct RouteStep {
    pub target: String,
    pub precursors: Vec<String>,
}

/// A tool-neutral, normalized route -- the promoted form of
/// `compare_route_graph.py`'s `RouteGraph`.
#[derive(Debug, Clone, Serialize)]
pub struct RouteDocument {
    pub source: RouteSource,
    pub root: RouteNode,
    /// Number of parent->children edges after normalization (one per
    /// disconnection step) -- distinct from any tool-reported step count.
    pub step_count_collapsed_edges: usize,
}

impl RouteDocument {
    pub fn steps(&self) -> Vec<RouteStep> {
        fn walk(node: &RouteNode, out: &mut Vec<RouteStep>) {
            if !node.children.is_empty() {
                out.push(RouteStep {
                    target: node.canonical_smiles.clone(),
                    precursors: node
                        .children
                        .iter()
                        .map(|c| c.canonical_smiles.clone())
                        .collect(),
                });
            }
            for c in &node.children {
                walk(c, out);
            }
        }
        let mut out = Vec::new();
        walk(&self.root, &mut out);
        out
    }
}

/// Result of normalizing a raw route into a [`RouteDocument`]. Mirrors
/// `compare_route_graph.py`'s `ParseOutcome`: `document` is `Some` only
/// when `defects` is empty -- a route with any defect is never partially
/// trusted, even if a tree could technically be built around the defect.
#[derive(Debug, Default)]
pub struct ParseOutcome {
    pub document: Option<RouteDocument>,
    pub parseable: bool,
    pub defects: Vec<AuditFindingCode>,
}

fn canonicalize(smiles: &str) -> Option<String> {
    mol_from_smiles(smiles).ok().map(|m| to_canonical(&m))
}

fn count_edges(node: &RouteNode) -> usize {
    let mut total = usize::from(!node.children.is_empty());
    for c in &node.children {
        total += count_edges(c);
    }
    total
}

#[allow(clippy::too_many_arguments)]
fn build(
    canon_smiles: &str,
    steps_by_target: &HashMap<String, &[String]>,
    bb_canon: &HashSet<String>,
    on_stack: &mut HashSet<String>,
    defects: &mut Vec<AuditFindingCode>,
) -> RouteNode {
    if on_stack.contains(canon_smiles) {
        defects.push(AuditFindingCode::CycleDetected);
        return RouteNode {
            canonical_smiles: canon_smiles.to_string(),
            is_stock_leaf: None,
            children: vec![],
        };
    }
    let Some(precursors) = steps_by_target.get(canon_smiles) else {
        if !bb_canon.contains(canon_smiles) {
            // A precursor that's neither a step's target nor a declared
            // building block -- the source tool's own invariant is broken.
            defects.push(AuditFindingCode::AmbiguousLeafStatus);
            return RouteNode {
                canonical_smiles: canon_smiles.to_string(),
                is_stock_leaf: None,
                children: vec![],
            };
        }
        return RouteNode {
            canonical_smiles: canon_smiles.to_string(),
            is_stock_leaf: Some(true),
            children: vec![],
        };
    };

    on_stack.insert(canon_smiles.to_string());
    let mut children = Vec::new();
    for precursor_raw in *precursors {
        let Some(p_canon) = canonicalize(precursor_raw) else {
            defects.push(AuditFindingCode::UnparseableSmilesInRoute);
            continue;
        };
        if p_canon == canon_smiles {
            defects.push(AuditFindingCode::DegenerateSelfReferentialStep);
            continue;
        }
        children.push(build(
            &p_canon,
            steps_by_target,
            bb_canon,
            on_stack,
            defects,
        ));
    }
    on_stack.remove(canon_smiles);
    if children.is_empty() {
        defects.push(AuditFindingCode::ChildlessNonLeaf);
    }
    RouteNode {
        canonical_smiles: canon_smiles.to_string(),
        is_stock_leaf: Some(false),
        children,
    }
}

/// Tool-neutral normalization of RENKIN's own completed [`Route`] --
/// mirrors `compare_route_graph.py`'s `normalize_renkin_route` exactly
/// (same defect codes, same construction order); see that module and
/// `scripts/tests/test_compare_route_graph.py` for the fixture-parity
/// oracle this is verified against.
///
/// Divergence from RENKIN Bridge PR1's `search::route_integrity_defects`,
/// deliberate: a zero-step route (`route.steps` empty) is valid there --
/// the target itself is already a stock leaf, a legitimate search outcome
/// -- but is `MultipleOrZeroRoots` here, matching the Python reference.
/// PR1 gates *search output* (depth-0 is a real, common case); this
/// function audits *a submitted route document* (a route with no
/// decomposition steps at all is not decodable as a multi-node route to
/// audit). Do not "fix" one to match the other.
pub fn normalize_renkin_route(route: &Route, requested_target_smiles: &str) -> ParseOutcome {
    let mut defects = Vec::new();

    if route.steps.is_empty() {
        return ParseOutcome {
            document: None,
            parseable: false,
            defects: vec![AuditFindingCode::MultipleOrZeroRoots],
        };
    }

    let Some(requested_canon) = canonicalize(requested_target_smiles) else {
        return ParseOutcome {
            document: None,
            parseable: false,
            defects: vec![AuditFindingCode::UnparseableSmilesInRoute],
        };
    };

    let mut bb_canon: HashSet<String> = HashSet::new();
    for bb in &route.building_blocks {
        match canonicalize(bb) {
            Some(c) => {
                bb_canon.insert(c);
            }
            None => defects.push(AuditFindingCode::UnparseableSmilesInRoute),
        }
    }

    let mut steps_by_target: HashMap<String, &[String]> = HashMap::new();
    for step in &route.steps {
        match canonicalize(&step.target) {
            Some(canon) => {
                steps_by_target.insert(canon, step.precursors.as_slice());
            }
            None => defects.push(AuditFindingCode::UnparseableSmilesInRoute),
        }
    }

    let root_canon = canonicalize(&route.steps[0].target);
    if root_canon.as_deref() != Some(requested_canon.as_str()) {
        defects.push(AuditFindingCode::RootMismatch);
    }

    let mut on_stack: HashSet<String> = HashSet::new();
    let root_start = root_canon.unwrap_or(requested_canon);
    let root_node = build(
        &root_start,
        &steps_by_target,
        &bb_canon,
        &mut on_stack,
        &mut defects,
    );

    let parseable = defects.is_empty();
    let document = parseable.then(|| RouteDocument {
        source: RouteSource::Renkin,
        step_count_collapsed_edges: count_edges(&root_node),
        root: root_node,
    });
    ParseOutcome {
        document,
        parseable,
        defects,
    }
}

/// Inclusion-list hash: `(canonical_smiles, is_stock_leaf, n_children)` per
/// node, pre-order, children sorted ascending by canonical SMILES.
/// Excludes everything tool-specific (timing, scores, template ids, ...)
/// by construction -- only what's listed here is ever fed into the hash.
/// Mirrors `compare_route_graph.py`'s `normalized_route_sha256` bit for
/// bit, but is **not** asserted equal to that function's output anywhere:
/// the Python reference canonicalizes with RDKit, this canonicalizes with
/// chematic, and the two are a documented non-invariant (see
/// `docs/guides/open-source-retrosynthesis-comparison.md`, "Canonicalizer
/// choice"). Only this function's own stability/order-independence/
/// difference-detection properties are verified here, matching what
/// `test_compare_route_graph.py`'s
/// `TestNormalizedRouteHashCrossToolConsistency` checks within Python.
pub fn normalized_route_sha256(document: &RouteDocument) -> String {
    fn emit(node: &RouteNode, out: &mut String) {
        let mut sorted_children: Vec<&RouteNode> = node.children.iter().collect();
        sorted_children.sort_by(|a, b| a.canonical_smiles.cmp(&b.canonical_smiles));
        out.push('[');
        out.push_str(&serde_json::to_string(&node.canonical_smiles).unwrap());
        out.push(',');
        out.push_str(&serde_json::to_string(&node.is_stock_leaf).unwrap());
        out.push(',');
        out.push_str(&node.children.len().to_string());
        out.push_str(",[");
        for (i, c) in sorted_children.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            emit(c, out);
        }
        out.push_str("]]");
    }

    let mut tree = String::new();
    emit(&document.root, &mut tree);
    let payload = format!("{{\"schema\":\"renkin-issue66-route-hash-v1\",\"tree\":{tree}}}");
    crate::sha256_hex(payload.as_bytes())
}

// Fixture-parity oracle: `scripts/tests/test_compare_route_graph.py`'s
// `TestNormalizeRenkinRoute` -- same target/precursor SMILES literals, same
// expected defect codes on the same shapes. `test_malformed_shape_is_raw_
// output_not_decodable` has no Rust analog: that test exists only because
// Python's `route` argument is an untyped dict; `normalize_renkin_route`
// here takes a strongly-typed `&Route`, so "malformed shape" isn't a state
// this function's caller can construct in the first place.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::{AtomEconomyStatus, ReactionStep};

    const TARGET: &str = "CCOC(=O)c1ccccc1";
    const ETHANOL: &str = "CCO";
    const BENZOIC_ACID: &str = "O=C(O)c1ccccc1";

    fn step(target: &str, precursors: &[&str]) -> ReactionStep {
        ReactionStep {
            rule: "esterification".to_string(),
            template_id: "t1".to_string(),
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

    fn route(steps: Vec<ReactionStep>, building_blocks: &[&str]) -> Route {
        Route {
            steps,
            depth: 1,
            score: 1.0,
            building_blocks: building_blocks.iter().map(|s| s.to_string()).collect(),
            confidence: 1.0,
            convergency: 1.0,
            success_probability: 1.0,
            route_cost: 1.0,
        }
    }

    fn single_step_route() -> Route {
        route(
            vec![step(TARGET, &[ETHANOL, BENZOIC_ACID])],
            &[ETHANOL, BENZOIC_ACID],
        )
    }

    #[test]
    fn single_step_route_parses() {
        let outcome = normalize_renkin_route(&single_step_route(), TARGET);
        assert!(outcome.parseable, "{:?}", outcome.defects);
        let doc = outcome.document.unwrap();
        assert_eq!(doc.step_count_collapsed_edges, 1);
        assert_eq!(doc.root.children.len(), 2);
        assert!(
            doc.root
                .children
                .iter()
                .all(|c| c.is_stock_leaf == Some(true))
        );
    }

    #[test]
    fn root_mismatch_detected() {
        let outcome = normalize_renkin_route(&single_step_route(), "CCN");
        assert!(!outcome.parseable);
        assert!(outcome.defects.contains(&AuditFindingCode::RootMismatch));
    }

    #[test]
    fn cycle_detected() {
        let r = route(vec![step("CCO", &["CCN"]), step("CCN", &["CCO"])], &[]);
        let outcome = normalize_renkin_route(&r, "CCO");
        assert!(!outcome.parseable);
        assert!(outcome.defects.contains(&AuditFindingCode::CycleDetected));
    }

    #[test]
    fn childless_non_leaf_detected() {
        let r = route(vec![step(TARGET, &[])], &[]);
        let outcome = normalize_renkin_route(&r, TARGET);
        assert!(!outcome.parseable);
        assert!(
            outcome
                .defects
                .contains(&AuditFindingCode::ChildlessNonLeaf)
        );
    }

    #[test]
    fn ambiguous_leaf_status_when_precursor_not_declared_building_block() {
        // BENZOIC_ACID missing from declared leaves.
        let r = route(vec![step(TARGET, &[ETHANOL, BENZOIC_ACID])], &[ETHANOL]);
        let outcome = normalize_renkin_route(&r, TARGET);
        assert!(!outcome.parseable);
        assert!(
            outcome
                .defects
                .contains(&AuditFindingCode::AmbiguousLeafStatus)
        );
    }

    #[test]
    fn unparseable_smiles_in_route_detected() {
        let r = route(
            vec![step(TARGET, &[ETHANOL, "not_a_smiles((("])],
            &[ETHANOL, "not_a_smiles((("],
        );
        let outcome = normalize_renkin_route(&r, TARGET);
        assert!(!outcome.parseable);
        assert!(
            outcome
                .defects
                .contains(&AuditFindingCode::UnparseableSmilesInRoute)
        );
    }

    #[test]
    fn degenerate_self_referential_step_detected() {
        let r = route(vec![step(TARGET, &[TARGET])], &[]);
        let outcome = normalize_renkin_route(&r, TARGET);
        assert!(!outcome.parseable);
        assert!(
            outcome
                .defects
                .contains(&AuditFindingCode::DegenerateSelfReferentialStep)
        );
    }

    #[test]
    fn empty_steps_is_multiple_or_zero_roots() {
        let r = route(vec![], &[]);
        let outcome = normalize_renkin_route(&r, TARGET);
        assert!(!outcome.parseable);
        assert_eq!(outcome.defects, vec![AuditFindingCode::MultipleOrZeroRoots]);
    }

    // ── Hash properties (within-Rust only -- see `normalized_route_sha256`'s
    // doc comment for why no cross-language byte-equality is asserted). ──

    #[test]
    fn hash_stable_across_repeated_calls() {
        let outcome = normalize_renkin_route(&single_step_route(), TARGET);
        let doc = outcome.document.unwrap();
        assert_eq!(normalized_route_sha256(&doc), normalized_route_sha256(&doc));
    }

    #[test]
    fn hash_independent_of_input_precursor_order() {
        let a = normalize_renkin_route(
            &route(
                vec![step(TARGET, &[ETHANOL, BENZOIC_ACID])],
                &[ETHANOL, BENZOIC_ACID],
            ),
            TARGET,
        )
        .document
        .unwrap();
        let b = normalize_renkin_route(
            &route(
                vec![step(TARGET, &[BENZOIC_ACID, ETHANOL])],
                &[ETHANOL, BENZOIC_ACID],
            ),
            TARGET,
        )
        .document
        .unwrap();
        assert_eq!(normalized_route_sha256(&a), normalized_route_sha256(&b));
    }

    #[test]
    fn hash_differs_for_different_disconnections() {
        let a = normalize_renkin_route(&single_step_route(), TARGET)
            .document
            .unwrap();
        let b = normalize_renkin_route(
            &route(
                vec![step(TARGET, &[ETHANOL, "CC(=O)O"])],
                &[ETHANOL, "CC(=O)O"],
            ),
            TARGET,
        )
        .document
        .unwrap();
        assert_ne!(normalized_route_sha256(&a), normalized_route_sha256(&b));
    }

    #[test]
    fn steps_flattens_tree_back_to_step_list() {
        let doc = normalize_renkin_route(&single_step_route(), TARGET)
            .document
            .unwrap();
        let steps = doc.steps();
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].precursors.len(), 2);
    }
}
