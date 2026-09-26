//! RENKIN Bridge PR6: normalizes real AiZynthFinder route output into the
//! same tool-neutral [`RouteDocument`] RENKIN's own routes produce, so
//! `bridge::audit::audit`/`bridge::forward::validate_step_forward` run
//! identically regardless of source -- no AiZynthFinder-specific audit
//! logic exists anywhere; this module's only job is the shape conversion.
//!
//! Confirmed against real `aizynthcli 4.4.1` output (not guessed -- see
//! `tests/fixtures/aizynthfinder/v4.4.1/PROVENANCE.md` for exact capture
//! provenance): a route is a recursive `mol` / `reaction` tree. A `mol`
//! node is either a leaf (`in_stock: bool` present, no children) or has
//! exactly one `reaction` child; a `reaction` node's own children are the
//! precursor `mol` nodes. `metadata.mapped_reaction_smiles` on a `reaction`
//! node, when present, is a complete atom-mapped reaction SMILES (not the
//! narrower template-match `smiles` field on the same node, which only
//! maps the atoms the template pattern itself touches) -- that field is
//! what feeds [`ReactionEvidence::AiZynthFinderTemplate`].
//!
//! Deliberately unparsed/unused here (forward-compatible, not a gap):
//! `smiles` on a `reaction` node, `template`/
//! `policy_probability`/etc. in `metadata`, and the root node's own
//! `scores`/`metadata` (iteration count, solved flag) -- none of it is
//! needed to build a [`RouteDocument`], and `#[derive(Deserialize)]`
//! without `deny_unknown_fields` means any of it (or fields from a future
//! `aizynthfinder` version) is silently ignored rather than a parse error.

use serde::Deserialize;

use crate::bridge::audit::AuditFindingCode;
use crate::bridge::route_graph::{
    ParseOutcome, ReactionEvidence, RouteDocument, RouteNode, RouteSource, count_edges,
};
use crate::chem_env::{mol_from_smiles, to_canonical};

/// One node of a real AiZynthFinder route tree (`type: "mol"` or
/// `"reaction"`). `in_stock`/`metadata` are only ever populated on the node
/// type they're meaningful for (`mol`/`reaction` respectively) -- reading
/// the wrong one just yields `None`, not an error, since both are
/// `Option`.
#[derive(Debug, Deserialize)]
pub struct AzfNode {
    #[serde(rename = "type")]
    pub node_type: String,
    pub smiles: String,
    #[serde(default)]
    pub in_stock: Option<bool>,
    #[serde(default)]
    pub metadata: Option<AzfMetadata>,
    #[serde(default)]
    pub children: Vec<AzfNode>,
}

#[derive(Debug, Deserialize)]
pub struct AzfMetadata {
    #[serde(default)]
    pub mapped_reaction_smiles: Option<String>,
    #[serde(default)]
    pub template_hash: Option<String>,
    #[serde(default)]
    pub classification: Option<String>,
}

fn canonicalize(smiles: &str) -> Option<String> {
    mol_from_smiles(smiles).ok().map(|m| to_canonical(&m))
}

/// Converts one `mol` node (leaf or with a single `reaction` child) into a
/// [`RouteNode`]. Fail-loud on anything not matching the confirmed real
/// shape: a non-`mol` node where a `mol` is expected, a non-leaf `mol` with
/// zero or more than one `reaction` child (a route -- as opposed to the raw
/// multi-alternative search tree -- has exactly one declared reaction per
/// step by construction), a `reaction` child whose own children aren't all
/// `mol` nodes, or a `reaction` with zero precursor children.
fn azf_mol_to_route_node(node: &AzfNode, defects: &mut Vec<AuditFindingCode>) -> RouteNode {
    if node.node_type != "mol" {
        defects.push(AuditFindingCode::RawOutputNotDecodable);
        return RouteNode {
            canonical_smiles: node.smiles.clone(),
            is_stock_leaf: None,
            reaction_evidence: None,
            children: vec![],
        };
    }
    let Some(canon) = canonicalize(&node.smiles) else {
        defects.push(AuditFindingCode::UnparseableSmilesInRoute);
        return RouteNode {
            canonical_smiles: node.smiles.clone(),
            is_stock_leaf: None,
            reaction_evidence: None,
            children: vec![],
        };
    };

    if node.children.is_empty() {
        if node.in_stock.is_none() {
            defects.push(AuditFindingCode::AmbiguousLeafStatus);
        }
        return RouteNode {
            canonical_smiles: canon,
            is_stock_leaf: node.in_stock,
            reaction_evidence: None,
            children: vec![],
        };
    }

    let [reaction] = node.children.as_slice() else {
        defects.push(AuditFindingCode::RawOutputNotDecodable);
        return RouteNode {
            canonical_smiles: canon,
            is_stock_leaf: Some(false),
            reaction_evidence: None,
            children: vec![],
        };
    };
    if reaction.node_type != "reaction" {
        defects.push(AuditFindingCode::RawOutputNotDecodable);
        return RouteNode {
            canonical_smiles: canon,
            is_stock_leaf: Some(false),
            reaction_evidence: None,
            children: vec![],
        };
    }

    let reaction_evidence = reaction.metadata.as_ref().and_then(|metadata| {
        metadata.mapped_reaction_smiles.clone().map(|smirks| {
            ReactionEvidence::AiZynthFinderTemplate {
                smirks,
                template_hash: metadata.template_hash.clone(),
                classification: metadata.classification.clone(),
            }
        })
    });

    let mut children = Vec::new();
    for precursor in &reaction.children {
        if precursor.node_type != "mol" {
            defects.push(AuditFindingCode::RawOutputNotDecodable);
            continue;
        }
        let Some(p_canon) = canonicalize(&precursor.smiles) else {
            defects.push(AuditFindingCode::UnparseableSmilesInRoute);
            continue;
        };
        if p_canon == canon {
            defects.push(AuditFindingCode::DegenerateSelfReferentialStep);
            continue;
        }
        children.push(azf_mol_to_route_node(precursor, defects));
    }
    if children.is_empty() {
        defects.push(AuditFindingCode::ChildlessNonLeaf);
    }
    RouteNode {
        canonical_smiles: canon,
        is_stock_leaf: Some(false),
        reaction_evidence,
        children,
    }
}

/// Normalizes one real AiZynthFinder route (one entry of a `trees` array,
/// either from `single_trees.json` or one target row's `trees` field in a
/// batch `output.json.gz`) into a tool-neutral [`ParseOutcome`]. No
/// separate "requested target" check exists here the way
/// `normalize_renkin_route` has one -- the root `mol` node *is* the target
/// unambiguously, by construction of the format itself.
pub fn normalize_aizynthfinder_route(node: &AzfNode) -> ParseOutcome {
    let mut defects = Vec::new();
    let root = azf_mol_to_route_node(node, &mut defects);
    let parseable = defects.is_empty();
    let document = parseable.then(|| RouteDocument {
        source: RouteSource::AiZynthFinder,
        step_count_collapsed_edges: count_edges(&root),
        root,
    });
    ParseOutcome {
        source: RouteSource::AiZynthFinder,
        document,
        parseable,
        defects,
    }
}

// Fixture-parity oracle: real `aizynthcli 4.4.1` output, not hand-authored --
// see `tests/fixtures/aizynthfinder/v4.4.1/PROVENANCE.md`.
/// Export a RENKIN [`crate::search::Route`] as an AiZynthFinder
/// `ReactionTree.to_dict()`-shaped route (the element type of
/// `aizynthcli`'s `trees` output), so RENKIN routes can be fed to
/// AiZynthFinder-ecosystem tooling (route-distances, visualisation,
/// notebooks) and back into `renkin audit-route --format aizynthfinder`.
///
/// Mapping, stated so nothing is over-read:
/// - `mol` nodes carry RENKIN's canonical SMILES. Leaves of a completed
///   route are stock terminals, so `in_stock: true`; intermediates and the
///   target of a multi-step route are `in_stock: false`. A depth-0 route is
///   a single in-stock `mol` node.
/// - `reaction.smiles` follows AiZynthFinder's retro direction
///   (`product>>reactants`) but is **not atom-mapped**, and
///   `metadata.mapped_reaction_smiles` is deliberately absent: RENKIN does
///   not fabricate a mapping. `metadata.template_hash`/`template_code`/
///   `policy_probability` are absent for the same reason; RENKIN's own
///   identity is carried as `renkin_template_id`/`renkin_rule`.
/// - `scores` uses AiZynthFinder's count-based score names where the value
///   is well defined, plus RENKIN-prefixed scores that have no AiZ analogue.
pub fn route_to_aizynthfinder_tree(
    route: &crate::search::Route,
    target: &str,
) -> serde_json::Value {
    use serde_json::json;
    use std::collections::HashMap;

    let by_target: HashMap<&str, &crate::search::ReactionStep> = route
        .steps
        .iter()
        .rev() // the first step for a target wins, matching display::build_tree
        .map(|s| (s.target.as_str(), s))
        .collect();
    let all_precursors: std::collections::HashSet<&str> = route
        .steps
        .iter()
        .flat_map(|s| s.precursors.iter().map(String::as_str))
        .collect();
    let root = route
        .steps
        .iter()
        .map(|s| s.target.as_str())
        .find(|t| !all_precursors.contains(t))
        .or_else(|| route.steps.first().map(|s| s.target.as_str()))
        .unwrap_or(target);

    fn mol(
        smiles: &str,
        by_target: &HashMap<&str, &crate::search::ReactionStep>,
        on_path: &mut Vec<String>,
        leaves: &mut usize,
    ) -> serde_json::Value {
        let step = by_target
            .get(smiles)
            .filter(|_| !on_path.iter().any(|s| s == smiles));
        let Some(step) = step else {
            *leaves += 1;
            return json!({
                "type": "mol",
                "hide": false,
                "smiles": smiles,
                "is_chemical": true,
                "in_stock": true,
            });
        };
        on_path.push(smiles.to_owned());
        let children: Vec<serde_json::Value> = step
            .precursors
            .iter()
            .map(|p| mol(p, by_target, on_path, leaves))
            .collect();
        on_path.pop();
        let mut metadata = serde_json::Map::new();
        metadata.insert("policy_name".into(), json!("renkin"));
        metadata.insert("renkin_template_id".into(), json!(step.template_id));
        metadata.insert("renkin_rule".into(), json!(step.rule));
        metadata.insert("renkin_step_confidence".into(), json!(step.step_confidence));
        if let Some(family) = step.reaction_family.as_deref() {
            metadata.insert("classification".into(), json!(family));
        }
        json!({
            "type": "mol",
            "hide": false,
            "smiles": smiles,
            "is_chemical": true,
            "in_stock": false,
            "children": [{
                "type": "reaction",
                "hide": false,
                "smiles": format!("{}>>{}", smiles, step.precursors.join(".")),
                "is_reaction": true,
                "metadata": metadata,
                "children": children,
            }],
        })
    }

    let mut leaves = 0usize;
    let mut tree = mol(root, &by_target, &mut Vec::new(), &mut leaves);
    if let Some(object) = tree.as_object_mut() {
        object.insert(
            "scores".into(),
            json!({
                "number of reactions": route.steps.len(),
                "number of pre-cursors": leaves,
                "number of pre-cursors in stock": leaves,
                "renkin score": route.score,
                "renkin route cost": route.route_cost,
                "renkin success probability": route.success_probability,
            }),
        );
        object.insert(
            "metadata".into(),
            json!({
                "is_solved": true,
                "exported_by": format!("renkin {}", env!("CARGO_PKG_VERSION")),
            }),
        );
    }
    tree
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exported_depth_zero_route_is_a_single_in_stock_molecule() {
        let route = crate::search::Route {
            steps: Vec::new(),
            depth: 0,
            score: 0.0,
            building_blocks: vec!["CCO".to_owned()],
            confidence: 1.0,
            convergency: 0.0,
            success_probability: 1.0,
            route_cost: 0.0,
        };
        let tree = route_to_aizynthfinder_tree(&route, "CCO");
        assert_eq!(tree["type"], "mol");
        assert_eq!(tree["smiles"], "CCO");
        assert_eq!(tree["in_stock"], true);
        assert!(tree.get("children").is_none());
        assert_eq!(tree["scores"]["number of reactions"], 0);
        // The exported shape must parse back through the importer.
        let node: AzfNode = serde_json::from_value(tree).unwrap();
        let outcome = normalize_aizynthfinder_route(&node);
        assert!(outcome.defects.is_empty(), "{:?}", outcome.defects);
    }

    fn load_fixture(name: &str) -> Vec<AzfNode> {
        let path = format!(
            "{}/tests/fixtures/aizynthfinder/v4.4.1/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        let content = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        serde_json::from_str(&content).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    #[test]
    fn real_single_step_route_normalizes_cleanly() {
        let routes = load_fixture("single_trees.json");
        let outcome = normalize_aizynthfinder_route(&routes[0]);
        assert!(outcome.parseable, "{:?}", outcome.defects);
        let doc = outcome.document.unwrap();
        assert_eq!(doc.source, RouteSource::AiZynthFinder);
        assert_eq!(doc.root.children.len(), 1);
        assert!(matches!(
            doc.root.reaction_evidence,
            Some(ReactionEvidence::AiZynthFinderTemplate { .. })
        ));
        match doc.root.reaction_evidence.as_ref() {
            Some(ReactionEvidence::AiZynthFinderTemplate {
                template_hash,
                classification,
                ..
            }) => {
                assert!(template_hash.is_some());
                assert_eq!(classification.as_deref(), Some("0.0 Unrecognized"));
            }
            other => panic!("expected AiZynthFinder provenance, got {other:?}"),
        }
    }

    #[test]
    fn real_two_step_route_normalizes_cleanly_with_evidence_at_both_steps() {
        let routes = load_fixture("single_trees.json");
        let outcome = normalize_aizynthfinder_route(&routes[1]);
        assert!(outcome.parseable, "{:?}", outcome.defects);
        let doc = outcome.document.unwrap();
        assert_eq!(doc.step_count_collapsed_edges, 2);
        let steps = doc.steps();
        assert_eq!(steps.len(), 2);
        for step in &steps {
            assert!(
                matches!(
                    step.reaction_evidence,
                    Some(ReactionEvidence::AiZynthFinderTemplate { .. })
                ),
                "expected reaction_evidence on every step, got {:?}",
                step.reaction_evidence
            );
        }
    }

    #[test]
    fn real_route_leaves_carry_the_source_tools_own_stock_claim() {
        let routes = load_fixture("single_trees.json");
        let outcome = normalize_aizynthfinder_route(&routes[1]);
        let doc = outcome.document.unwrap();
        fn leaves(node: &RouteNode, out: &mut Vec<bool>) {
            if node.children.is_empty() {
                out.push(
                    node.is_stock_leaf
                        .expect("real fixture leaves are unambiguous"),
                );
            }
            for c in &node.children {
                leaves(c, out);
            }
        }
        let mut claims = Vec::new();
        leaves(&doc.root, &mut claims);
        assert!(
            claims.iter().all(|&c| c),
            "real benzocaine route's leaves are all AiZynthFinder in_stock=true, got {claims:?}"
        );
    }

    #[test]
    fn missing_atom_mapping_fixture_has_no_reaction_evidence_at_the_mutated_step() {
        let routes = load_fixture("single_trees_missing_atom_mapping.json");
        let outcome = normalize_aizynthfinder_route(&routes[0]);
        assert!(outcome.parseable, "{:?}", outcome.defects);
        let doc = outcome.document.unwrap();
        assert!(
            doc.root.reaction_evidence.is_none(),
            "the mutated fixture's outer reaction node has mapped_reaction_smiles stripped -- \
             normalize_aizynthfinder_route must not invent evidence that isn't there"
        );
    }

    #[test]
    fn structurally_corrupt_route_fails_loud_not_silently() {
        // A non-leaf mol with two reaction children -- not a valid *route*
        // shape (only the raw multi-alternative search tree looks like
        // this), must be rejected rather than guessed at.
        let corrupt = AzfNode {
            node_type: "mol".to_string(),
            smiles: "CCO".to_string(),
            in_stock: None,
            metadata: None,
            children: vec![
                AzfNode {
                    node_type: "reaction".to_string(),
                    smiles: String::new(),
                    in_stock: None,
                    metadata: None,
                    children: vec![],
                },
                AzfNode {
                    node_type: "reaction".to_string(),
                    smiles: String::new(),
                    in_stock: None,
                    metadata: None,
                    children: vec![],
                },
            ],
        };
        let outcome = normalize_aizynthfinder_route(&corrupt);
        assert!(!outcome.parseable);
        assert!(
            outcome
                .defects
                .contains(&AuditFindingCode::RawOutputNotDecodable)
        );
    }
}
