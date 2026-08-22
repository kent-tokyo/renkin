//! v0.30.0 Syntheseus Bridge, Phase 2: normalizes a `syntheseus-route-v1`
//! JSON document into the same tool-neutral [`RouteDocument`] RENKIN's own
//! routes and AiZynthFinder's routes produce. Not a Syntheseus adapter in
//! the sense `bridge::aizynthfinder` is one -- Syntheseus itself has no
//! native route export, so `syntheseus-route-v1` is RENKIN's own
//! interchange schema, produced by the optional `renkin.syntheseus_exporter`
//! Python module (`python/renkin/syntheseus_exporter.py`, Phase 1). See
//! `docs/design/syntheseus-bridge-v0.md` for the schema and its rationale.
//!
//! `syntheseus-route-v1`'s `steps` is a flat list, deliberately shaped like
//! RENKIN's own native route format rather than AiZynthFinder's nested tree
//! (see the schema doc's own reasoning) -- specifically so this reuses
//! `route_graph::build`'s tree-flattening algorithm directly, rather than a
//! third, separately-maintained tree-walker.
//!
//! Leaf classification combines both existing adapters' own conventions,
//! since the schema carries both kinds of signal RENKIN already knows how
//! to read separately:
//! - `starting_molecules` plays the role of RENKIN's own `building_blocks`
//!   -- the structural claim "this precursor is a leaf, not some other
//!   step's target". A precursor absent from both is `AmbiguousLeafStatus`,
//!   same as `normalize_renkin_route`'s own "declared neither a step target
//!   nor a building block" case.
//! - `molecule_metadata[...].is_purchasable` plays the role of
//!   AiZynthFinder's `in_stock` -- a tri-state purchasability claim from the
//!   source tool. `None` (missing entirely, or present as JSON `null`) is
//!   the genuinely-ambiguous case and is never guessed at: same
//!   `AmbiguousLeafStatus` code AiZynthFinder's adapter uses when `in_stock`
//!   itself is absent.
//!
//! Convergent (non-tree) routes -- a molecule produced by one step and
//! consumed by two different downstream steps, proved real and reachable by
//! Phase 0's own `convergent_route.json` fixture -- are handled by simply
//! not special-casing them: [`build`] re-expands the shared molecule's own
//! sub-tree independently under each parent, exactly as it already does for
//! RENKIN's own routes. [`RouteNode`]'s tree shape has no way to represent a
//! DAG node with two parents, so duplication-on-flatten is the only
//! representable outcome without a bigger schema change -- this was an open
//! question in the Phase 0 design doc (§7.1), resolved this round by
//! inheriting the existing, already-tested behavior unchanged (smallest
//! diff, consistent with `normalize_renkin_route`'s own precedent).

use std::collections::{HashMap, HashSet};

use serde::Deserialize;

use crate::bridge::audit::AuditFindingCode;
use crate::bridge::route_graph::{
    ParseOutcome, ReactionEvidence, RouteDocument, RouteSource, StepInfo, build, count_edges,
};
use crate::chem_env::{mol_from_smiles, to_canonical};

/// Deserialized view of a `syntheseus-route-v1` document. Declares only the
/// fields this normalizer reads (`source_metadata` and per-step `identifier`/
/// `template`/`source`/`reaction_id` are deliberately unparsed here, same
/// forward-compatible convention as `bridge::aizynthfinder::AzfNode` --
/// silently ignored by serde without `deny_unknown_fields`, not an error).
#[derive(Debug, Deserialize)]
pub struct SyntheseusRouteV1 {
    #[serde(default)]
    pub schema_version: Option<u32>,
    pub target: String,
    #[serde(default)]
    pub steps: Vec<SyntheseusStep>,
    #[serde(default)]
    pub starting_molecules: Vec<String>,
    #[serde(default)]
    pub molecule_metadata: HashMap<String, SyntheseusMoleculeMetadata>,
}

#[derive(Debug, Deserialize)]
pub struct SyntheseusStep {
    pub product: String,
    pub reactants: Vec<String>,
    pub reaction_metadata: SyntheseusReactionMetadata,
}

#[derive(Debug, Deserialize)]
pub struct SyntheseusReactionMetadata {
    pub reaction_smiles: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct SyntheseusMoleculeMetadata {
    #[serde(default)]
    pub is_purchasable: Option<bool>,
}

fn canonicalize(smiles: &str) -> Option<String> {
    mol_from_smiles(smiles).ok().map(|m| to_canonical(&m))
}

/// Normalizes one `syntheseus-route-v1` document into a tool-neutral
/// [`ParseOutcome`]. `schema_version` must be exactly `1` (the only version
/// this schema has ever had) -- anything else is rejected outright
/// (`RawOutputNotDecodable`), never guessed at as forward-compatible.
pub fn normalize_syntheseus_route(input: &SyntheseusRouteV1) -> ParseOutcome {
    let mut defects = Vec::new();

    if input.schema_version != Some(1) {
        return ParseOutcome {
            source: RouteSource::Syntheseus,
            document: None,
            parseable: false,
            defects: vec![AuditFindingCode::RawOutputNotDecodable],
        };
    }
    if input.steps.is_empty() {
        return ParseOutcome {
            source: RouteSource::Syntheseus,
            document: None,
            parseable: false,
            defects: vec![AuditFindingCode::MultipleOrZeroRoots],
        };
    }
    let Some(target_canon) = canonicalize(&input.target) else {
        return ParseOutcome {
            source: RouteSource::Syntheseus,
            document: None,
            parseable: false,
            defects: vec![AuditFindingCode::UnparseableSmilesInRoute],
        };
    };

    let mut starting_canon: HashSet<String> = HashSet::new();
    for smi in &input.starting_molecules {
        match canonicalize(smi) {
            Some(c) => {
                starting_canon.insert(c);
            }
            None => defects.push(AuditFindingCode::UnparseableSmilesInRoute),
        }
    }

    let mut purchasable_by_canon: HashMap<String, Option<bool>> = HashMap::new();
    for (smi, meta) in &input.molecule_metadata {
        if let Some(c) = canonicalize(smi) {
            purchasable_by_canon.insert(c, meta.is_purchasable);
        }
    }

    let mut steps_by_target: HashMap<String, StepInfo> = HashMap::new();
    for step in &input.steps {
        match canonicalize(&step.product) {
            Some(canon) => {
                steps_by_target.insert(
                    canon,
                    StepInfo {
                        precursors: step.reactants.as_slice(),
                        reaction_evidence: ReactionEvidence::SyntheseusReaction {
                            reaction_smiles: step.reaction_metadata.reaction_smiles.clone(),
                        },
                    },
                );
            }
            None => defects.push(AuditFindingCode::UnparseableSmilesInRoute),
        }
    }

    let root_canon = canonicalize(&input.steps[0].product);
    if root_canon.as_deref() != Some(target_canon.as_str()) {
        defects.push(AuditFindingCode::RootMismatch);
    }

    // A precursor outside starting_molecules violates Syntheseus's own
    // get_starting_molecules() invariant -- unconditionally ambiguous, same
    // as normalize_renkin_route's "not a declared building block" case. A
    // precursor inside it but with no is_purchasable claim (missing
    // entirely, or present as null) is the genuinely-ambiguous case
    // AiZynthFinder's in_stock == None already models -- never guessed.
    let resolve_leaf = |smi: &str| -> (Option<bool>, Option<AuditFindingCode>) {
        if !starting_canon.contains(smi) {
            return (None, Some(AuditFindingCode::AmbiguousLeafStatus));
        }
        match purchasable_by_canon.get(smi).copied().flatten() {
            Some(p) => (Some(p), None),
            None => (None, Some(AuditFindingCode::AmbiguousLeafStatus)),
        }
    };

    let mut on_stack: HashSet<String> = HashSet::new();
    let root_start = root_canon.unwrap_or(target_canon);
    let root_node = build(
        &root_start,
        &steps_by_target,
        &resolve_leaf,
        &mut on_stack,
        &mut defects,
    );

    let parseable = defects.is_empty();
    let document = parseable.then(|| RouteDocument {
        source: RouteSource::Syntheseus,
        step_count_collapsed_edges: count_edges(&root_node),
        root: root_node,
    });
    ParseOutcome {
        source: RouteSource::Syntheseus,
        document,
        parseable,
        defects,
    }
}

// Fixture-parity oracle: the real, Phase-0-committed fixtures
// (`tests/fixtures/syntheseus/0.7.2/`), not hand-authored JSON -- see that
// directory's own PROVENANCE.md.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::route_graph::RouteNode;

    fn load_fixture(name: &str) -> SyntheseusRouteV1 {
        let path = format!(
            "{}/tests/fixtures/syntheseus/0.7.2/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        let content = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        serde_json::from_str(&content).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    #[test]
    fn real_linear_route_normalizes_cleanly() {
        let input = load_fixture("linear_two_leaf_route.json");
        let outcome = normalize_syntheseus_route(&input);
        assert!(outcome.parseable, "{:?}", outcome.defects);
        let doc = outcome.document.unwrap();
        assert_eq!(doc.source, RouteSource::Syntheseus);
        assert_eq!(doc.root.children.len(), 2);
        assert!(
            doc.root
                .children
                .iter()
                .all(|c| c.is_stock_leaf == Some(true))
        );
        assert!(matches!(
            doc.root.reaction_evidence,
            Some(ReactionEvidence::SyntheseusReaction { .. })
        ));
    }

    #[test]
    fn real_convergent_route_normalizes_by_duplicating_the_shared_subtree() {
        // Same convergent structure as the committed fixture, but with a
        // resolvable is_purchasable claim on the CC leaf -- the committed
        // fixture's own CC is deliberately ambiguous (see
        // `convergent_fixture_ambiguous_leaf_is_a_gating_finding`), which
        // makes `document` always None there; this variant isolates the
        // duplication-on-flatten *tree shape* on an otherwise-clean parse.
        let mut input = load_fixture("convergent_route.json");
        input
            .molecule_metadata
            .get_mut("CC")
            .expect("fixture has a CC entry")
            .is_purchasable = Some(true);
        let outcome = normalize_syntheseus_route(&input);
        assert!(outcome.parseable, "{:?}", outcome.defects);
        let doc = outcome.document.unwrap();
        // CO (methanol) is a child of both the root (cocs_from_co_cs) and
        // cs_from_co -- RouteNode can't represent a shared node, so its own
        // CO->CC decomposition step is expanded independently under each
        // parent: 4 total non-leaf nodes (root, root's CO child, CS, CS's
        // own CO child), not 3.
        assert_eq!(doc.step_count_collapsed_edges, 4);
        let leaves: Vec<&RouteNode> = {
            fn collect<'a>(node: &'a RouteNode, out: &mut Vec<&'a RouteNode>) {
                if node.children.is_empty() {
                    out.push(node);
                } else {
                    for c in &node.children {
                        collect(c, out);
                    }
                }
            }
            let mut out = Vec::new();
            collect(&doc.root, &mut out);
            out
        };
        assert_eq!(leaves.len(), 2, "CC appears as a leaf under both parents");
        assert!(leaves.iter().all(|l| l.canonical_smiles == "CC"));
        assert!(
            leaves.iter().all(|l| l.is_stock_leaf == Some(true)),
            "both duplicated CC leaves carry the same resolved purchasability claim"
        );
    }

    #[test]
    fn convergent_fixture_ambiguous_leaf_is_a_gating_finding() {
        let input = load_fixture("convergent_route.json");
        let outcome = normalize_syntheseus_route(&input);
        // Fixture B's own leaf genuinely has no purchasability claim, so
        // this document is never `parseable` -- confirming the schema's
        // "null is real, not a bug" case still surfaces as a real defect,
        // not silently swallowed.
        assert!(!outcome.parseable);
        assert!(
            outcome
                .defects
                .contains(&AuditFindingCode::AmbiguousLeafStatus)
        );
    }

    #[test]
    fn wrong_schema_version_is_rejected() {
        let mut input = load_fixture("linear_two_leaf_route.json");
        input.schema_version = Some(2);
        let outcome = normalize_syntheseus_route(&input);
        assert!(!outcome.parseable);
        assert_eq!(
            outcome.defects,
            vec![AuditFindingCode::RawOutputNotDecodable]
        );
    }

    #[test]
    fn empty_steps_is_multiple_or_zero_roots() {
        let mut input = load_fixture("linear_two_leaf_route.json");
        input.steps.clear();
        let outcome = normalize_syntheseus_route(&input);
        assert!(!outcome.parseable);
        assert_eq!(outcome.defects, vec![AuditFindingCode::MultipleOrZeroRoots]);
    }

    #[test]
    fn root_mismatch_detected() {
        let mut input = load_fixture("linear_two_leaf_route.json");
        input.target = "CCN".to_string();
        let outcome = normalize_syntheseus_route(&input);
        assert!(!outcome.parseable);
        assert!(outcome.defects.contains(&AuditFindingCode::RootMismatch));
    }

    #[test]
    fn leaf_outside_starting_molecules_is_ambiguous() {
        let mut input = load_fixture("linear_two_leaf_route.json");
        input.starting_molecules.clear();
        let outcome = normalize_syntheseus_route(&input);
        assert!(!outcome.parseable);
        assert!(
            outcome
                .defects
                .contains(&AuditFindingCode::AmbiguousLeafStatus)
        );
    }
}
