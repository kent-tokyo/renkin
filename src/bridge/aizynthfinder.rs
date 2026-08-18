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
//! `smiles` on a `reaction` node, `template`/`template_hash`/
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

    let reaction_evidence = reaction
        .metadata
        .as_ref()
        .and_then(|m| m.mapped_reaction_smiles.clone())
        .map(|smirks| ReactionEvidence::AiZynthFinderTemplate { smirks });

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
#[cfg(test)]
mod tests {
    use super::*;

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
