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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RouteSource {
    #[default]
    Renkin,
    AiZynthFinder,
    /// v0.30.0 Syntheseus Bridge, Phase 2: a `syntheseus-route-v1` document
    /// (`bridge::syntheseus::normalize_syntheseus_route`) -- not Syntheseus
    /// output directly, since Syntheseus has no native route export (see
    /// `docs/design/syntheseus-bridge-v0.md`).
    Syntheseus,
}

/// Whatever reaction-identity evidence is available for a non-leaf node --
/// tool-neutral in shape, source-specific in content. `None` on
/// [`RouteNode::reaction_evidence`] means no evidence was attached (a leaf,
/// or a step whose source didn't supply enough to identify a reaction at
/// all -- e.g. an AiZynthFinder route whose reaction node metadata this
/// codebase has no confirmed schema for; see `bridge::forward` module docs).
/// Consumed by RENKIN Bridge PR4's forward-validation
/// (`bridge::forward::validate_step_forward`) to resolve which single
/// declared reaction to replay -- never a scan over alternatives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReactionEvidence {
    /// RENKIN-native: the `RetroRule` this step's search claimed to use.
    /// `ReactionStep::template_id` is "Always populated", so every
    /// RENKIN-sourced non-leaf node gets one of these.
    RenkinTemplate { template_id: String },
    /// AiZynthFinder-sourced: whatever reaction SMIRKS/SMILES the route
    /// metadata carried, if present. No adapter in this codebase
    /// constructs this from a real AiZynthFinder JSON export yet (RENKIN
    /// Bridge PR4 scope note) -- only hand-built fixtures do, until a real
    /// adapter is confirmed against actual `aizynthcli` output.
    AiZynthFinderTemplate { smirks: String },
    /// Syntheseus-sourced (v0.30.0 Phase 2): a step's `reaction_smiles`,
    /// always present on a real `syntheseus-route-v1` document (a computed
    /// property on every Syntheseus `Reaction` object -- see the schema doc,
    /// safe to treat as required rather than optional).
    SyntheseusReaction { reaction_smiles: String },
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
    pub reaction_evidence: Option<ReactionEvidence>,
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
    pub reaction_evidence: Option<ReactionEvidence>,
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
                    reaction_evidence: node.reaction_evidence.clone(),
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
///
/// `source` is carried here explicitly (set by whichever normalizer ran),
/// not derived from `document` -- `document` is `None` on every failure
/// path, which is exactly when a caller (e.g. `bridge::audit::audit`) most
/// needs to know which tool's route just failed to parse.
#[derive(Debug, Default)]
pub struct ParseOutcome {
    pub source: RouteSource,
    pub document: Option<RouteDocument>,
    pub parseable: bool,
    pub defects: Vec<AuditFindingCode>,
}

fn canonicalize(smiles: &str) -> Option<String> {
    mol_from_smiles(smiles).ok().map(|m| to_canonical(&m))
}

/// `pub(crate)`: also used by `bridge::aizynthfinder`'s normalizer, which
/// builds a [`RouteDocument`] from a different raw shape but needs the same
/// edge-count convention.
pub(crate) fn count_edges(node: &RouteNode) -> usize {
    let mut total = usize::from(!node.children.is_empty());
    for c in &node.children {
        total += count_edges(c);
    }
    total
}

/// A step's precursors plus its already-constructed [`ReactionEvidence`],
/// keyed by canonicalized target in [`build`]'s `steps_by_target` map.
/// `pub(crate)`: shared by every flat-step-list adapter (`normalize_renkin_route`,
/// `bridge::syntheseus::normalize_syntheseus_route`) -- evidence is
/// constructed by each adapter's own caller (source-specific), not by
/// [`build`] itself, which stays source-agnostic.
pub(crate) struct StepInfo<'a> {
    pub precursors: &'a [String],
    pub reaction_evidence: ReactionEvidence,
}

/// Returns `(is_stock_leaf, defect_if_any)` for a canonical SMILES that
/// isn't any step's own target -- see [`build`]'s `resolve_leaf` parameter.
type LeafResolver<'a> = dyn Fn(&str) -> (Option<bool>, Option<AuditFindingCode>) + 'a;

/// Recursive flat-steps-to-tree builder, shared by every adapter whose raw
/// shape is a flat step list (unlike AiZynthFinder's already-nested tree,
/// which has its own walker in `bridge::aizynthfinder`). Cycle detection,
/// self-reference rejection, and childless-non-leaf detection are
/// source-agnostic and live here; leaf classification is not (RENKIN's own
/// `building_blocks` are an unconditional true/false claim, while
/// Syntheseus's `starting_molecules`/`molecule_metadata.is_purchasable`
/// carry a tri-state purchasability claim like AiZynthFinder's `in_stock`)
/// -- `resolve_leaf` is called for any canonical SMILES that isn't some
/// step's own target, and returns `(is_stock_leaf, defect_if_any)` for the
/// caller's own policy to decide.
pub(crate) fn build(
    canon_smiles: &str,
    steps_by_target: &HashMap<String, StepInfo>,
    resolve_leaf: &LeafResolver,
    on_stack: &mut HashSet<String>,
    defects: &mut Vec<AuditFindingCode>,
) -> RouteNode {
    if on_stack.contains(canon_smiles) {
        defects.push(AuditFindingCode::CycleDetected);
        return RouteNode {
            canonical_smiles: canon_smiles.to_string(),
            is_stock_leaf: None,
            reaction_evidence: None,
            children: vec![],
        };
    }
    let Some(step_info) = steps_by_target.get(canon_smiles) else {
        let (is_stock_leaf, defect) = resolve_leaf(canon_smiles);
        if let Some(d) = defect {
            defects.push(d);
        }
        return RouteNode {
            canonical_smiles: canon_smiles.to_string(),
            is_stock_leaf,
            reaction_evidence: None,
            children: vec![],
        };
    };

    on_stack.insert(canon_smiles.to_string());
    let mut children = Vec::new();
    for precursor_raw in step_info.precursors {
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
            resolve_leaf,
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
        reaction_evidence: Some(step_info.reaction_evidence.clone()),
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
            source: RouteSource::Renkin,
            document: None,
            parseable: false,
            defects: vec![AuditFindingCode::MultipleOrZeroRoots],
        };
    }

    let Some(requested_canon) = canonicalize(requested_target_smiles) else {
        return ParseOutcome {
            source: RouteSource::Renkin,
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

    let mut steps_by_target: HashMap<String, StepInfo> = HashMap::new();
    for step in &route.steps {
        match canonicalize(&step.target) {
            Some(canon) => {
                steps_by_target.insert(
                    canon,
                    StepInfo {
                        precursors: step.precursors.as_slice(),
                        reaction_evidence: ReactionEvidence::RenkinTemplate {
                            template_id: step.template_id.clone(),
                        },
                    },
                );
            }
            None => defects.push(AuditFindingCode::UnparseableSmilesInRoute),
        }
    }

    let root_canon = canonicalize(&route.steps[0].target);
    if root_canon.as_deref() != Some(requested_canon.as_str()) {
        defects.push(AuditFindingCode::RootMismatch);
    }

    // A precursor that's neither a step's target nor a declared building
    // block breaks RENKIN's own invariant -- unconditionally ambiguous,
    // never guessed.
    let resolve_leaf = |smi: &str| -> (Option<bool>, Option<AuditFindingCode>) {
        if bb_canon.contains(smi) {
            (Some(true), None)
        } else {
            (None, Some(AuditFindingCode::AmbiguousLeafStatus))
        }
    };

    let mut on_stack: HashSet<String> = HashSet::new();
    let root_start = root_canon.unwrap_or(requested_canon);
    let root_node = build(
        &root_start,
        &steps_by_target,
        &resolve_leaf,
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
        source: RouteSource::Renkin,
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
