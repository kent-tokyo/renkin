//! Phase 1 PR2: normalizes a real SynPlanner `write_routes_json` export into
//! the same tool-neutral [`RouteDocument`] every other adapter builds into.
//! Design: `docs/design/synplanner-adapter-v1.md`. Confirmed against real
//! SynPlanner 1.6.0 output twice -- Phase 0 (hand-constructed `chython`
//! reactions run through SynPlanner's own real exporter) and Phase 1 PR1.5
//! (a real, CPU-only MCTS-searched planning run through SynPlanner's own
//! `synplan planning` CLI end to end) -- see
//! `tests/fixtures/synplanner/v1.6.0/PROVENANCE.md` and
//! `real_planning_route.PROVENANCE.md` for exact capture provenance.
//!
//! Structurally identical in shape to AiZynthFinder's `mol`/`reaction`
//! alternating tree (this module ports [`crate::bridge::aizynthfinder`]'s
//! recursive-walker pattern, not `route_graph::build`'s flat-step-list one),
//! with one difference: SynPlanner's reaction-only fields (`rule_id`/
//! `rule_source`/`rule_key`) are direct siblings of `type`/`smiles`/
//! `children` on the same node object, not nested under a `metadata`
//! sub-object the way AiZynthFinder's are.
//!
//! Deliberately unparsed/unused here, per `docs/design/synplanner-adapter-v1.md`
//! §7's resolved design decisions -- not a gap:
//! - `meta`/`step_id`/`tree_node_id`: read-and-discard. Accepted as unknown
//!   fields (no `#[serde(deny_unknown_fields)]` anywhere in this codebase's
//!   adapters), never copied into the report as opaque JSON, never confused
//!   with RENKIN's own internal node identifiers.
//! - The top-level `{route_id: RouteNode}` wrapper's `route_id` itself is
//!   retained by the audit-route interchange layer as `source_route_id`, but
//!   is not threaded onto [`RouteDocument`]/`AuditReport`'s legacy schema.
//! - The separate, explicitly versioned `--export_routes` "public contract"
//!   wrapper (`manifest.json` + `results.json.gz`, `{target_smiles:
//!   [RouteNode, ...]}`, confirmed in Phase 1 PR1.5): not parsed by this
//!   module. A user following that path decompresses `results.json.gz`
//!   themselves; only the internal `{route_id: RouteNode}` shape (the same
//!   shape every committed fixture uses) is recognized as SynPlanner input
//!   today. Widening detection to the wrapped shape is a real, tracked
//!   follow-up, not silently unsupported.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::bridge::audit::AuditFindingCode;
use crate::bridge::route_graph::{
    ParseOutcome, ReactionEvidence, RouteDocument, RouteNode, RouteSource,
    SynPlannerRuleProvenance, count_edges,
};
use crate::chem_env::{mol_from_smiles, to_canonical};

/// One node of a real SynPlanner `RouteNode` tree (`type: "mol"` or
/// `"reaction"`). `in_stock` is only ever populated on a `mol` node;
/// `rule_id`/`rule_source`/`rule_key` only ever on a `reaction` node --
/// reading the wrong one just yields `None`, not an error, since all are
/// `Option`. `rule_id` is deserialized as a raw [`serde_json::Value`]
/// because real SynPlanner output emits it as a JSON *number*
/// (`tests/fixtures/synplanner/v1.6.0/route_3_full_fields.json`), not a
/// string -- `scalar_to_string` normalizes either representation.
#[derive(Debug, Deserialize)]
pub struct SynPlannerNode {
    #[serde(rename = "type")]
    pub node_type: String,
    pub smiles: String,
    #[serde(default)]
    pub in_stock: Option<bool>,
    #[serde(default)]
    pub rule_id: Option<serde_json::Value>,
    #[serde(default)]
    pub rule_source: Option<String>,
    #[serde(default)]
    pub rule_key: Option<String>,
    #[serde(default)]
    pub children: Vec<SynPlannerNode>,
}

fn scalar_to_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn rule_provenance(node: &SynPlannerNode) -> Option<SynPlannerRuleProvenance> {
    let rule_id = node.rule_id.as_ref().and_then(scalar_to_string);
    if rule_id.is_none() && node.rule_source.is_none() && node.rule_key.is_none() {
        return None;
    }
    Some(SynPlannerRuleProvenance {
        rule_id,
        rule_source: node.rule_source.clone(),
        rule_key: node.rule_key.clone(),
    })
}

fn canonicalize(smiles: &str) -> Option<String> {
    mol_from_smiles(smiles).ok().map(|m| to_canonical(&m))
}

/// Converts one `mol` node (leaf or with a single `reaction` child) into a
/// [`RouteNode`]. Fail-loud on anything not matching the confirmed real
/// shape -- identical defect vocabulary and structural expectations to
/// [`crate::bridge::aizynthfinder::azf_mol_to_route_node`], since real
/// SynPlanner route trees have the exact same shape constraints (a route,
/// as opposed to a raw multi-alternative search tree, has exactly one
/// declared reaction per step).
fn synplanner_mol_to_route_node(
    node: &SynPlannerNode,
    defects: &mut Vec<AuditFindingCode>,
) -> RouteNode {
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

    // Real SynPlanner reaction nodes always carry `smiles` (required, not
    // optional -- see docs/design/synplanner-adapter-v1.md §3.1), so
    // evidence is always constructed here, unlike AiZynthFinder's
    // conditional `Option::map` over an optional metadata field.
    let reaction_evidence = Some(ReactionEvidence::SynPlannerReaction {
        smiles: reaction.smiles.clone(),
        rule_provenance: rule_provenance(reaction),
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
        children.push(synplanner_mol_to_route_node(precursor, defects));
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

/// Normalizes one real SynPlanner route (one value of the top-level
/// `{route_id: RouteNode}` object) into a tool-neutral [`ParseOutcome`]. The
/// source tool's own `route_id` string key is retained by the caller when
/// iterating [`parse_synplanner_routes`]; the normalized document remains
/// tool-neutral.
pub fn normalize_synplanner_route(node: &SynPlannerNode) -> ParseOutcome {
    let mut defects = Vec::new();
    let root = synplanner_mol_to_route_node(node, &mut defects);
    let parseable = defects.is_empty();
    let document = parseable.then(|| RouteDocument {
        source: RouteSource::SynPlanner,
        step_count_collapsed_edges: count_edges(&root),
        root,
    });
    ParseOutcome {
        source: RouteSource::SynPlanner,
        document,
        parseable,
        defects,
    }
}

/// Parses a real `write_routes_json` top-level object
/// (`{route_id: RouteNode}`) into a deterministically-ordered map (by
/// route-ID string, not real-world numeric order -- ordering only matters
/// for reproducible report iteration, not per-route correctness).
pub fn parse_synplanner_routes(
    value: serde_json::Value,
) -> Result<BTreeMap<String, SynPlannerNode>, serde_json::Error> {
    serde_json::from_value(value)
}

// Fixture-parity oracle: real SynPlanner 1.6.0 output (both hand-constructed
// chython input run through the real exporter, and a real MCTS-searched
// planning run), not hand-authored -- see
// tests/fixtures/synplanner/v1.6.0/PROVENANCE.md and
// real_planning_route.PROVENANCE.md.
#[cfg(test)]
mod tests {
    use super::*;

    fn load_fixture(name: &str) -> BTreeMap<String, SynPlannerNode> {
        let path = format!(
            "{}/tests/fixtures/synplanner/v1.6.0/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        let content = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let value: serde_json::Value =
            serde_json::from_str(&content).unwrap_or_else(|e| panic!("{path}: {e}"));
        parse_synplanner_routes(value).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    #[test]
    fn real_two_step_hand_built_route_normalizes_cleanly_with_rule_provenance() {
        let routes = load_fixture("route_1_two_step.json");
        let (_, node) = routes.iter().next().unwrap();
        let outcome = normalize_synplanner_route(node);
        assert!(outcome.parseable, "{:?}", outcome.defects);
        let doc = outcome.document.unwrap();
        assert_eq!(doc.source, RouteSource::SynPlanner);
        assert_eq!(doc.step_count_collapsed_edges, 2);
        let steps = doc.steps();
        assert_eq!(steps.len(), 2);
        for step in &steps {
            match &step.reaction_evidence {
                Some(ReactionEvidence::SynPlannerReaction {
                    smiles,
                    rule_provenance,
                }) => {
                    assert!(!smiles.is_empty());
                    let rp = rule_provenance
                        .as_ref()
                        .expect("this fixture's route_metadata populates rule provenance");
                    assert!(rp.rule_id.is_some());
                }
                other => panic!("expected SynPlannerReaction evidence, got {other:?}"),
            }
        }
    }

    #[test]
    fn full_fields_fixture_carries_rule_id_as_a_string_despite_json_number_source() {
        let routes = load_fixture("route_3_full_fields.json");
        let (route_id, node) = routes.iter().next().unwrap();
        assert_eq!(
            route_id, "7",
            "non-sequential route ID must round-trip as-is"
        );
        let outcome = normalize_synplanner_route(node);
        assert!(outcome.parseable, "{:?}", outcome.defects);
        let doc = outcome.document.unwrap();
        let reaction_evidence = doc.root.reaction_evidence.as_ref().unwrap();
        let ReactionEvidence::SynPlannerReaction {
            rule_provenance, ..
        } = reaction_evidence
        else {
            panic!("expected SynPlannerReaction evidence");
        };
        let rp = rule_provenance.as_ref().unwrap();
        assert_eq!(rp.rule_id.as_deref(), Some("17"));
        assert_eq!(rp.rule_source.as_deref(), Some("handcrafted"));
        assert_eq!(rp.rule_key.as_deref(), Some("chy:0017"));
    }

    #[test]
    fn real_planning_route_has_no_rule_provenance_since_cli_never_passes_route_metadata() {
        let routes = load_fixture("real_planning_route_2step.json");
        let (_, node) = routes.iter().next().unwrap();
        let outcome = normalize_synplanner_route(node);
        assert!(outcome.parseable, "{:?}", outcome.defects);
        let doc = outcome.document.unwrap();
        for step in doc.steps() {
            match step.reaction_evidence {
                Some(ReactionEvidence::SynPlannerReaction {
                    smiles,
                    rule_provenance,
                }) => {
                    assert!(!smiles.is_empty());
                    assert!(
                        rule_provenance.is_none(),
                        "real synplan planning CLI output never passes route_metadata"
                    );
                }
                other => panic!("expected SynPlannerReaction evidence, got {other:?}"),
            }
        }
    }

    #[test]
    fn real_planning_route_atom_mapped_smiles_actually_passes_forward_validation() {
        // Confirms the design doc's central Phase 1 PR1.5 finding end to
        // end through this adapter, not just via the standalone Python
        // diagnostic: a real planning reaction's smiles reaches
        // forward.rs's has_atom_mapping gate unchanged and is accepted
        // (not MissingAtomMapping) -- the opposite of AiZynthFinder's/
        // Syntheseus's always-not_evaluable outcome.
        let routes = load_fixture("real_planning_route_1step.json");
        let (_, node) = routes.iter().next().unwrap();
        let outcome = normalize_synplanner_route(node);
        let doc = outcome.document.unwrap();
        let step = &doc.steps()[0];
        let result = crate::bridge::forward::validate_step_forward(
            &step.target,
            &step.precursors,
            step.reaction_evidence.as_ref(),
            None,
        );
        assert_ne!(
            result.reason,
            Some(crate::bridge::forward::ForwardNotEvaluableReason::MissingAtomMapping),
            "real planning output's atom maps must reach the gate unchanged: {result:?}"
        );
    }

    #[test]
    fn stock_leaf_without_in_stock_field_is_ambiguous_not_guessed() {
        let corrupt = SynPlannerNode {
            node_type: "mol".to_string(),
            smiles: "CCO".to_string(),
            in_stock: None,
            rule_id: None,
            rule_source: None,
            rule_key: None,
            children: vec![],
        };
        let outcome = normalize_synplanner_route(&corrupt);
        assert!(!outcome.parseable);
        assert!(
            outcome
                .defects
                .contains(&AuditFindingCode::AmbiguousLeafStatus)
        );
    }

    #[test]
    fn structurally_corrupt_route_fails_loud_not_silently() {
        // A non-leaf mol with two reaction children -- not a valid *route*
        // shape, must be rejected rather than guessed at (mirrors
        // aizynthfinder's identical test, same underlying shape contract).
        let corrupt = SynPlannerNode {
            node_type: "mol".to_string(),
            smiles: "CCO".to_string(),
            in_stock: None,
            rule_id: None,
            rule_source: None,
            rule_key: None,
            children: vec![
                SynPlannerNode {
                    node_type: "reaction".to_string(),
                    smiles: String::new(),
                    in_stock: None,
                    rule_id: None,
                    rule_source: None,
                    rule_key: None,
                    children: vec![],
                },
                SynPlannerNode {
                    node_type: "reaction".to_string(),
                    smiles: String::new(),
                    in_stock: None,
                    rule_id: None,
                    rule_source: None,
                    rule_key: None,
                    children: vec![],
                },
            ],
        };
        let outcome = normalize_synplanner_route(&corrupt);
        assert!(!outcome.parseable);
        assert!(
            outcome
                .defects
                .contains(&AuditFindingCode::RawOutputNotDecodable)
        );
    }
}
