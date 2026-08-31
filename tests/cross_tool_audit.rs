//! RENKIN Bridge PR6: cross-tool audit parity. Audits a real captured
//! AiZynthFinder route (`tests/fixtures/aizynthfinder/v4.4.1/single_trees.json`,
//! route index 0 -- see `PROVENANCE.md`) side by side with a hand-built
//! RENKIN-native route describing the *same* chemistry, and asserts the
//! two audits agree on everything tool-neutral: canonical root, leaf
//! multiset, step count, the set of structural finding codes, target
//! element accounting, and the configured-stock verdict.
//!
//! v0.30.0 Syntheseus Bridge, Phase 3 extends this to a third tool
//! (`syntheseus_route_agrees_structurally_with_renkin_and_aizynthfinder`,
//! `policy_verdict_invariance_holds_across_all_three_tools`) -- same
//! honest-degradation principle, same "tool-neutral structure must agree,
//! tool-specific evidence need not" contract.
//!
//! Phase 1 PR3 extends both of those tests to a fourth tool, SynPlanner
//! (renamed accordingly). One difference from AiZynthFinder/Syntheseus:
//! SynPlanner's reaction SMILES genuinely carries valid, forward-replayable
//! atom maps (confirmed against real MCTS-searched output, Phase 1 PR1.5)
//! -- unlike the other two adapters, whose steps always report
//! `not_evaluable` here, SynPlanner's `co_aliphatic_cleavage`-equivalent
//! step is expected to PASS forward validation, same as RENKIN's own side.
//!
//! Forward validation is deliberately NOT required to match step-for-step:
//! it's only required to agree when both sides have sufficient evidence to
//! reach a verdict at all. The RENKIN side is built with a `RetroRule`
//! whose SMIRKS is the fixture's own `mapped_reaction_smiles` (AiZynthFinder
//! writes it target>>precursors, the same retro convention `RetroRule::smirks`
//! uses), so both sides are expected to PASS forward validation here -- but
//! the assertion only requires "not FAIL when the other side reached a
//! verdict", not byte-identical reports, since that's the honest contract
//! (see `bridge` module docs on AiZynthFinder's own documented lossiness).

use std::collections::HashSet;

use renkin::bridge::syntheseus::{
    SyntheseusMoleculeMetadata, SyntheseusReactionMetadata, SyntheseusStep,
};
use renkin::bridge::{
    AuditFindingCode, AuditPolicy, AuditStatus, AzfNode, CheckStatus, ParseOutcome,
    ReactionEvidence, RouteDocument, RouteNode, RouteSource, SynPlannerNode, SyntheseusRouteV1,
    audit, audit_with_policy, normalize_aizynthfinder_route, normalize_renkin_route,
    normalize_synplanner_route, normalize_syntheseus_route,
};
use renkin::chem_env::{RetroRule, mol_from_smiles, to_canonical};
use renkin::search;

fn canon(smiles: &str) -> String {
    to_canonical(&mol_from_smiles(smiles).expect(smiles))
}

fn load_azf_route_0() -> AzfNode {
    let path = format!(
        "{}/tests/fixtures/aizynthfinder/v4.4.1/single_trees.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let content = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut routes: Vec<AzfNode> =
        serde_json::from_str(&content).unwrap_or_else(|e| panic!("{path}: {e}"));
    routes.remove(0)
}

// The real fixture's own `metadata.mapped_reaction_smiles` for route 0, in
// AiZynthFinder's target>>precursors convention -- identical to
// `RetroRule::smirks`'s own "reactant>>product1.product2 (retro direction)"
// convention, so this exact string is reused verbatim as the RENKIN side's
// rule rather than re-derived, guaranteeing the two sides describe the same
// reaction, not just similarly-shaped SMILES.
const NITRO_REDUCTION_RETRO_SMIRKS: &str = "[CH3:1][CH2:2][O:3][C:4](=[O:5])[c:6]1[cH:7][cH:8][c:9]([NH2:10])[cH:11][cH:12]1>>[CH3:1][CH2:2][O:3][C:4](=[O:5])[c:6]1[cH:7][cH:8][c:9]([N+:10](=[O:13])[O-:14])[cH:11][cH:12]1";

fn renkin_equivalent_route() -> (RouteDocument, Vec<RetroRule>) {
    let target = canon("CCOC(=O)c1ccc(N)cc1");
    let precursor = canon("CCOC(=O)c1ccc([N+](=O)[O-])cc1");
    let root = RouteNode {
        canonical_smiles: target,
        is_stock_leaf: Some(false),
        reaction_evidence: Some(ReactionEvidence::RenkinTemplate {
            template_id: "nitro_to_amine_retro".to_string(),
            declared_smirks: None,
        }),
        children: vec![RouteNode {
            canonical_smiles: precursor,
            is_stock_leaf: Some(true),
            reaction_evidence: None,
            children: vec![],
        }],
    };
    let document = RouteDocument {
        source: RouteSource::Renkin,
        step_count_collapsed_edges: 1,
        root,
    };
    let rules = vec![RetroRule {
        name: "nitro_to_amine_retro".to_string(),
        template_id: "nitro_to_amine_retro".to_string(),
        smirks: NITRO_REDUCTION_RETRO_SMIRKS.to_string(),
        weight: 1.0,
        required_elements: 0,
    }];
    (document, rules)
}

fn leaf_multiset(node: &RouteNode, out: &mut Vec<String>) {
    if node.children.is_empty() {
        out.push(node.canonical_smiles.clone());
    }
    for c in &node.children {
        leaf_multiset(c, out);
    }
}

#[test]
fn renkin_and_aizynthfinder_audits_of_the_same_reaction_agree_structurally() {
    let stock: HashSet<String> = [canon("CCOC(=O)c1ccc([N+](=O)[O-])cc1")]
        .into_iter()
        .collect();

    let (renkin_document, renkin_rules) = renkin_equivalent_route();
    let renkin_outcome = ParseOutcome {
        source: RouteSource::Renkin,
        document: Some(renkin_document.clone()),
        parseable: true,
        defects: Vec::new(),
    };
    let renkin_report = audit(&renkin_outcome, Some(&stock), Some(&renkin_rules));

    let azf_outcome = normalize_aizynthfinder_route(&load_azf_route_0());
    assert!(
        azf_outcome.parseable,
        "real fixture must normalize cleanly: {:?}",
        azf_outcome.defects
    );
    let azf_report = audit(&azf_outcome, Some(&stock), None);

    // Canonical root.
    assert_eq!(
        renkin_document.root.canonical_smiles,
        azf_outcome.document.as_ref().unwrap().root.canonical_smiles,
        "both sides describe the same target molecule"
    );

    // Leaf multiset.
    let mut renkin_leaves = Vec::new();
    leaf_multiset(&renkin_document.root, &mut renkin_leaves);
    let mut azf_leaves = Vec::new();
    leaf_multiset(
        &azf_outcome.document.as_ref().unwrap().root,
        &mut azf_leaves,
    );
    renkin_leaves.sort();
    azf_leaves.sort();
    assert_eq!(
        renkin_leaves, azf_leaves,
        "both sides bottom out on the same precursor"
    );

    // Step count.
    assert_eq!(
        renkin_document.step_count_collapsed_edges,
        azf_outcome
            .document
            .as_ref()
            .unwrap()
            .step_count_collapsed_edges
    );

    // Structural finding code set (gating findings only -- forward-related
    // codes are excluded on purpose, see module docs). `AuditFindingCode`
    // doesn't derive `Hash`, so "set" here means a sorted, deduped `Vec`
    // compared by value rather than an actual `HashSet`.
    fn structural_codes(findings: &[renkin::bridge::AuditFinding]) -> Vec<AuditFindingCode> {
        let mut codes: Vec<AuditFindingCode> = findings
            .iter()
            .map(|f| f.code)
            .filter(|c| {
                !matches!(
                    c,
                    AuditFindingCode::ForwardReactionNotReproduced
                        | AuditFindingCode::ForwardValidationNotEvaluable
                )
            })
            .collect();
        codes.sort_by_key(|c| *c as u8);
        codes.dedup();
        codes
    }
    assert_eq!(
        structural_codes(&renkin_report.findings),
        structural_codes(&azf_report.findings),
        "no structural defect on either side for this clean, equivalent route"
    );
    assert!(structural_codes(&renkin_report.findings).is_empty());

    // Target element accounting.
    assert_eq!(
        renkin_report.target_element_accounting_status,
        azf_report.target_element_accounting_status
    );

    // Configured stock result.
    assert_eq!(
        renkin_report.stock_validation.as_ref().map(|s| s.status),
        azf_report.stock_validation.as_ref().map(|s| s.status)
    );
    assert_eq!(
        renkin_report.stock_validation.as_ref().map(|s| s.status),
        Some(CheckStatus::Pass)
    );

    // Forward validation: only required to agree when both sides actually
    // reached a verdict. Here both have full evidence (a RenkinTemplate rule
    // with matching SMIRKS on one side, an AiZynthFinderTemplate with the
    // fixture's own mapped_reaction_smiles on the other), so both are
    // expected to PASS -- but the assertion is written to tolerate the
    // documented honest-degradation case (one side not_evaluable) as a
    // valid, non-failing outcome too.
    let renkin_step = &renkin_report.steps[0];
    let azf_step = &azf_report.steps[0];
    assert_ne!(renkin_step.forward_validation.status, CheckStatus::Fail);
    assert_ne!(azf_step.forward_validation.status, CheckStatus::Fail);
    if renkin_step.forward_validation.status == CheckStatus::Pass
        && azf_step.forward_validation.status == CheckStatus::Pass
    {
        // The expected, best case for this fixture: both sides have enough
        // evidence and agree the declared reaction reproduces the target.
    } else {
        assert!(
            renkin_step.forward_validation.status == CheckStatus::NotEvaluable
                || azf_step.forward_validation.status == CheckStatus::NotEvaluable,
            "a non-pass, non-fail forward result must be not_evaluable, never silently something else"
        );
    }

    assert_eq!(renkin_report.status, AuditStatus::Pass);
    assert_eq!(azf_report.status, AuditStatus::Pass);
}

// ── v0.27.0 "Reproducible Route Audit" P0 item 2: adapter conformance ──
//
// Extends the single hand-picked equivalence check above with targeted
// scenarios from the shared fixture contract (linear route is already
// covered above). Per this file's own established honest-degradation
// principle: scenarios about audit *outcomes* (branched, malformed
// hierarchy) assert genuine cross-tool agreement on a chemically
// equivalent input; scenarios about *missing evidence* assert both sides
// reach the same NotEvaluable/never-silently-Pass contract even though the
// two tools express "missing" through different, tool-specific reasons
// (RENKIN: an unresolved template_id; AiZynthFinder: an absent
// mapped_reaction_smiles) -- forcing byte-identical reasons here would
// misrepresent a real, documented difference as a bug. "No stock given" is
// already covered by `partial_without_stock_reports_stock_not_provided_not_a_silent_pass`
// in `tests/audit_route_cli.rs` (CLI layer) and not duplicated here.

/// Minimal `search::Route` builder mirroring `main.rs`'s own
/// `route_from_audit_input` -- every field `normalize_renkin_route` doesn't
/// read is defaulted, since this file tests normalization/audit logic, not
/// the search algorithm that would normally populate them.
fn renkin_route(steps: &[(&str, &[&str], &str)], building_blocks: &[&str]) -> search::Route {
    search::Route {
        steps: steps
            .iter()
            .map(|(target, precursors, template_id)| search::ReactionStep {
                rule: String::new(),
                template_id: template_id.to_string(),
                target: target.to_string(),
                precursors: precursors.iter().map(|s| s.to_string()).collect(),
                conditions: None,
                atom_economy: None,
                atom_economy_raw_percent: None,
                atom_economy_status: search::AtomEconomyStatus::NotEvaluable,
                step_confidence: 1.0,
                procedure_hint: None,
                reaction_family: None,
                metadata_source: None,
                metadata_scope: None,
                evidence: None,
            })
            .collect(),
        depth: 0,
        score: 0.0,
        building_blocks: building_blocks.iter().map(|s| s.to_string()).collect(),
        confidence: 0.0,
        convergency: 0.0,
        success_probability: 0.0,
        route_cost: 0.0,
    }
}

/// A generic aliphatic C-O cleavage, reused verbatim from
/// `bridge::audit`'s own already-proven test rule (`co_aliphatic_cleavage`
/// there) -- deliberately not inventing new chemistry per this program's
/// own "reuse what exists" constraint for this release.
fn co_aliphatic_cleavage_rule() -> RetroRule {
    RetroRule {
        name: "co_aliphatic_cleavage".to_string(),
        template_id: "co_aliphatic_cleavage".to_string(),
        smirks: "[C:1][O:2]>>[C:1].[O:2]".to_string(),
        ..Default::default()
    }
}

fn azf_mol(smiles: &str, in_stock: Option<bool>, children: Vec<AzfNode>) -> AzfNode {
    AzfNode {
        node_type: "mol".to_string(),
        smiles: smiles.to_string(),
        in_stock,
        metadata: None,
        children,
    }
}

fn azf_reaction(mapped_reaction_smiles: Option<&str>, precursors: Vec<AzfNode>) -> AzfNode {
    AzfNode {
        node_type: "reaction".to_string(),
        smiles: String::new(),
        in_stock: None,
        metadata: Some(renkin::bridge::AzfMetadata {
            mapped_reaction_smiles: mapped_reaction_smiles.map(str::to_string),
        }),
        children: precursors,
    }
}

/// Branched route: one step producing 2 non-trivial precursors (methanol
/// -> methane + water), not the linear single-child shape the test above
/// already covers. Both sides use the exact same reaction (retro-direction
/// SMIRKS on the RENKIN side, forward-direction `mapped_reaction_smiles` on
/// the AiZynthFinder side, same convention this file's module docs already
/// establish), so a genuine cross-tool structural comparison is meaningful
/// here, not a strained pairing.
#[test]
fn branched_route_agrees_structurally_across_tools() {
    let stock: HashSet<String> = [canon("C"), canon("O")].into_iter().collect();

    let renkin_input = renkin_route(&[("CO", &["C", "O"], "co_aliphatic_cleavage")], &["C", "O"]);
    let renkin_outcome = normalize_renkin_route(&renkin_input, "CO");
    assert!(renkin_outcome.parseable, "{:?}", renkin_outcome.defects);
    let renkin_report = audit(
        &renkin_outcome,
        Some(&stock),
        Some(&[co_aliphatic_cleavage_rule()]),
    );

    let azf_node = azf_mol(
        "CO",
        Some(false),
        vec![azf_reaction(
            // Atom-mapped, matching `co_aliphatic_cleavage_rule`'s own
            // SMIRKS exactly -- an unmapped SMILES here triggers AiZynthFinder
            // forward validation's own `MissingAtomMapping` not_evaluable
            // path, which isn't what this test is exercising.
            Some("[C:1][O:2]>>[C:1].[O:2]"),
            vec![
                azf_mol("C", Some(true), vec![]),
                azf_mol("O", Some(true), vec![]),
            ],
        )],
    );
    let azf_outcome = normalize_aizynthfinder_route(&azf_node);
    assert!(azf_outcome.parseable, "{:?}", azf_outcome.defects);
    let azf_report = audit(&azf_outcome, Some(&stock), None);

    let renkin_doc = renkin_outcome.document.as_ref().unwrap();
    let azf_doc = azf_outcome.document.as_ref().unwrap();

    assert_eq!(
        renkin_doc.root.children.len(),
        2,
        "genuinely branched, not linear"
    );
    assert_eq!(renkin_doc.root.children.len(), azf_doc.root.children.len());

    let mut renkin_leaves = Vec::new();
    leaf_multiset(&renkin_doc.root, &mut renkin_leaves);
    let mut azf_leaves = Vec::new();
    leaf_multiset(&azf_doc.root, &mut azf_leaves);
    renkin_leaves.sort();
    azf_leaves.sort();
    assert_eq!(renkin_leaves, azf_leaves);

    assert_eq!(renkin_report.status, AuditStatus::Pass, "{renkin_report:?}");
    assert_eq!(azf_report.status, AuditStatus::Pass, "{azf_report:?}");
}

/// Malformed hierarchy: a precursor identical to its own product (a
/// self-loop). Both adapters detect this via the exact same
/// `AuditFindingCode::DegenerateSelfReferentialStep` code (confirmed by
/// reading both `normalize_renkin_route` and `normalize_aizynthfinder_route`
/// before writing this test, not assumed) -- a real shared vocabulary, not
/// a coincidence this test invents.
#[test]
fn self_referential_hierarchy_fails_identically_across_tools() {
    let renkin_input = renkin_route(&[("CO", &["CO"], "self_loop")], &[]);
    let renkin_outcome = normalize_renkin_route(&renkin_input, "CO");
    assert!(!renkin_outcome.parseable);
    assert!(
        renkin_outcome
            .defects
            .contains(&AuditFindingCode::DegenerateSelfReferentialStep),
        "{:?}",
        renkin_outcome.defects
    );

    let azf_node = azf_mol(
        "CO",
        Some(false),
        vec![azf_reaction(
            Some("CO>>CO"),
            vec![azf_mol("CO", None, vec![])],
        )],
    );
    let azf_outcome = normalize_aizynthfinder_route(&azf_node);
    assert!(!azf_outcome.parseable);
    assert!(
        azf_outcome
            .defects
            .contains(&AuditFindingCode::DegenerateSelfReferentialStep),
        "{:?}",
        azf_outcome.defects
    );

    let renkin_report = audit(&renkin_outcome, None, None);
    let azf_report = audit(&azf_outcome, None, None);
    assert_eq!(renkin_report.status, AuditStatus::Fail);
    assert_eq!(azf_report.status, AuditStatus::Fail);
}

/// Missing reaction evidence: RENKIN's mechanism (a `template_id` with no
/// matching `RetroRule` supplied to `audit`) and AiZynthFinder's mechanism
/// (a `reaction` node with no `mapped_reaction_smiles`, using the real
/// `single_trees_missing_atom_mapping.json` fixture -- see that file's own
/// `PROVENANCE.md` entry) are different by construction, so this
/// deliberately does NOT assert identical reasons. What both tools must
/// agree on: forward validation reports `not_evaluable`, never silently
/// `pass` and never `fail` -- the shared contract, not a shared cause.
#[test]
fn missing_reaction_evidence_is_not_evaluable_never_silently_resolved_on_either_tool() {
    let renkin_input = renkin_route(
        &[("CO", &["C", "O"], "template_not_in_ruleset")],
        &["C", "O"],
    );
    let renkin_outcome = normalize_renkin_route(&renkin_input, "CO");
    assert!(renkin_outcome.parseable, "{:?}", renkin_outcome.defects);
    let renkin_report = audit(&renkin_outcome, None, Some(&[])); // no rules supplied at all
    assert_eq!(
        renkin_report.steps[0].forward_validation.status,
        CheckStatus::NotEvaluable,
        "{renkin_report:?}"
    );

    let path = format!(
        "{}/tests/fixtures/aizynthfinder/v4.4.1/single_trees_missing_atom_mapping.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let content = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut routes: Vec<AzfNode> =
        serde_json::from_str(&content).unwrap_or_else(|e| panic!("{path}: {e}"));
    let azf_outcome = normalize_aizynthfinder_route(&routes.remove(0));
    assert!(azf_outcome.parseable, "{:?}", azf_outcome.defects);
    let azf_report = audit(&azf_outcome, None, None);
    assert!(
        azf_report
            .steps
            .iter()
            .any(|s| s.forward_validation.status == CheckStatus::NotEvaluable),
        "the step with the deliberately-removed mapped_reaction_smiles must be not_evaluable: {azf_report:?}"
    );
    assert!(
        azf_report
            .steps
            .iter()
            .all(|s| s.forward_validation.status != CheckStatus::Fail),
        "missing evidence must never be misreported as a forward FAIL: {azf_report:?}"
    );
}

// ── v0.30.0 Syntheseus Bridge, Phase 3: extends the above to a third tool ──

/// Builds a single-step `SyntheseusRouteV1` -- `starting_leaves` is
/// `(smiles, is_purchasable)`, mirroring `azf_mol`'s `in_stock` parameter
/// (`None` is the genuinely-ambiguous case, never guessed).
fn syntheseus_route(
    target: &str,
    reactants: &[&str],
    starting_leaves: &[(&str, Option<bool>)],
) -> SyntheseusRouteV1 {
    SyntheseusRouteV1 {
        schema_version: Some(1),
        source_version: None,
        target: target.to_string(),
        steps: vec![SyntheseusStep {
            product: target.to_string(),
            reactants: reactants.iter().map(|s| s.to_string()).collect(),
            reaction_metadata: SyntheseusReactionMetadata {
                reaction_smiles: format!("{}>>{target}", reactants.join(".")),
                identifier: None,
            },
        }],
        starting_molecules: starting_leaves.iter().map(|(s, _)| s.to_string()).collect(),
        molecule_metadata: starting_leaves
            .iter()
            .map(|(s, purchasable)| {
                (
                    s.to_string(),
                    SyntheseusMoleculeMetadata {
                        is_purchasable: *purchasable,
                    },
                )
            })
            .collect(),
    }
}

/// `SynPlannerNode` builders mirroring `azf_mol`/`azf_reaction` -- SynPlanner's
/// reaction-only fields (`rule_id`/`rule_source`/`rule_key`) are unset here
/// since this test isn't exercising rule provenance.
fn synplanner_mol(
    smiles: &str,
    in_stock: Option<bool>,
    children: Vec<SynPlannerNode>,
) -> SynPlannerNode {
    SynPlannerNode {
        node_type: "mol".to_string(),
        smiles: smiles.to_string(),
        in_stock,
        rule_id: None,
        rule_source: None,
        rule_key: None,
        step_id: None,
        tree_node_id: None,
        children,
    }
}

fn synplanner_reaction(smiles: &str, precursors: Vec<SynPlannerNode>) -> SynPlannerNode {
    SynPlannerNode {
        node_type: "reaction".to_string(),
        smiles: smiles.to_string(),
        in_stock: None,
        rule_id: None,
        rule_source: None,
        rule_key: None,
        step_id: None,
        tree_node_id: None,
        children: precursors,
    }
}

/// Same "CO -> C + O" reaction as `branched_route_agrees_structurally_across_tools`,
/// described a fourth way -- a genuine 4-way structural comparison, not a
/// strained pairing, since all four sides describe the identical chemistry.
#[test]
fn four_tools_agree_structurally_renkin_aizynthfinder_syntheseus_synplanner() {
    let stock: HashSet<String> = [canon("C"), canon("O")].into_iter().collect();

    let renkin_input = renkin_route(&[("CO", &["C", "O"], "co_aliphatic_cleavage")], &["C", "O"]);
    let renkin_outcome = normalize_renkin_route(&renkin_input, "CO");
    assert!(renkin_outcome.parseable, "{:?}", renkin_outcome.defects);
    let renkin_report = audit(
        &renkin_outcome,
        Some(&stock),
        Some(&[co_aliphatic_cleavage_rule()]),
    );

    let azf_node = azf_mol(
        "CO",
        Some(false),
        vec![azf_reaction(
            Some("[C:1][O:2]>>[C:1].[O:2]"),
            vec![
                azf_mol("C", Some(true), vec![]),
                azf_mol("O", Some(true), vec![]),
            ],
        )],
    );
    let azf_outcome = normalize_aizynthfinder_route(&azf_node);
    assert!(azf_outcome.parseable, "{:?}", azf_outcome.defects);
    let azf_report = audit(&azf_outcome, Some(&stock), None);

    let syn_input = syntheseus_route("CO", &["C", "O"], &[("C", Some(true)), ("O", Some(true))]);
    let syn_outcome = normalize_syntheseus_route(&syn_input);
    assert!(syn_outcome.parseable, "{:?}", syn_outcome.defects);
    let syn_report = audit(&syn_outcome, Some(&stock), None);

    let sp_node = synplanner_mol(
        "CO",
        Some(false),
        vec![synplanner_reaction(
            "[C:1][O:2]>>[C:1].[O:2]",
            vec![
                synplanner_mol("C", Some(true), vec![]),
                synplanner_mol("O", Some(true), vec![]),
            ],
        )],
    );
    let sp_outcome = normalize_synplanner_route(&sp_node);
    assert!(sp_outcome.parseable, "{:?}", sp_outcome.defects);
    let sp_report = audit(&sp_outcome, Some(&stock), None);

    let renkin_doc = renkin_outcome.document.as_ref().unwrap();
    let azf_doc = azf_outcome.document.as_ref().unwrap();
    let syn_doc = syn_outcome.document.as_ref().unwrap();
    let sp_doc = sp_outcome.document.as_ref().unwrap();

    // Canonical root, agreed by all four.
    for doc in [azf_doc, syn_doc, sp_doc] {
        assert_eq!(renkin_doc.root.canonical_smiles, doc.root.canonical_smiles);
    }

    // Leaf multiset, agreed by all four.
    let leaves_of = |doc: &RouteDocument| {
        let mut out = Vec::new();
        leaf_multiset(&doc.root, &mut out);
        out.sort();
        out
    };
    let renkin_leaves = leaves_of(renkin_doc);
    assert_eq!(renkin_leaves, leaves_of(azf_doc));
    assert_eq!(renkin_leaves, leaves_of(syn_doc));
    assert_eq!(renkin_leaves, leaves_of(sp_doc));

    // Step count, agreed by all four.
    for doc in [azf_doc, syn_doc, sp_doc] {
        assert_eq!(
            renkin_doc.step_count_collapsed_edges,
            doc.step_count_collapsed_edges
        );
    }

    // Structural findings: empty on every side for this clean, equivalent
    // route (forward-related codes excluded, same convention as the
    // 2-way test above).
    fn structural_codes(findings: &[renkin::bridge::AuditFinding]) -> Vec<AuditFindingCode> {
        let mut codes: Vec<AuditFindingCode> = findings
            .iter()
            .map(|f| f.code)
            .filter(|c| {
                !matches!(
                    c,
                    AuditFindingCode::ForwardReactionNotReproduced
                        | AuditFindingCode::ForwardValidationNotEvaluable
                )
            })
            .collect();
        codes.sort_by_key(|c| *c as u8);
        codes.dedup();
        codes
    }
    for report in [&azf_report, &syn_report, &sp_report] {
        assert_eq!(
            structural_codes(&renkin_report.findings),
            structural_codes(&report.findings)
        );
    }
    assert!(structural_codes(&renkin_report.findings).is_empty());

    // Target element accounting, agreed by all four.
    for report in [&azf_report, &syn_report, &sp_report] {
        assert_eq!(
            renkin_report.target_element_accounting_status,
            report.target_element_accounting_status
        );
    }

    // Configured stock result, agreed by all four -- all leaves are in
    // `stock`, so every side passes.
    for report in [&renkin_report, &azf_report, &syn_report, &sp_report] {
        assert_eq!(
            report.stock_validation.as_ref().map(|s| s.status),
            Some(CheckStatus::Pass),
            "{report:?}"
        );
    }

    // Forward validation: unlike AiZynthFinder/Syntheseus (whose steps here
    // are only required to reach not_evaluable/pass, never fail -- see the
    // 2-way and 3-way tests above), SynPlanner's real, atom-mapped reaction
    // SMILES is expected to genuinely PASS here, matching RENKIN's own side
    // (Phase 1 PR1.5's central finding, confirmed end to end through this
    // adapter in PR2's own unit tests -- reconfirmed here in a genuine
    // cross-tool comparison, not just in isolation).
    assert_eq!(
        renkin_report.steps[0].forward_validation.status,
        CheckStatus::Pass
    );
    assert_eq!(
        sp_report.steps[0].forward_validation.status,
        CheckStatus::Pass,
        "{sp_report:?}"
    );
}

/// The `derive_status` policy table (`AuditPolicy::{Informational,Standard,
/// Strict}` under a genuine gating finding: `informational` softens to
/// `partial`, `standard`/`strict` stay `fail`) already has an exhaustive,
/// adapter-agnostic proof in `bridge::audit`'s own tests. What's new here:
/// confirming that table holds for real, adapter-specific *input* on all
/// four tools at once, from the same underlying "one leaf's purchasability
/// is genuinely unknown" condition expressed four different ways
/// (RENKIN: absent from `building_blocks`; AiZynthFinder: `in_stock: None`;
/// Syntheseus: `is_purchasable: None`; SynPlanner: `in_stock: None`) -- the
/// same finding-set-invariance property v0.29.0 proved per-adapter, now
/// confirmed to generalize across every adapter uniformly, not just each
/// one in isolation.
#[test]
fn policy_verdict_invariance_holds_across_all_four_tools() {
    let renkin_input = renkin_route(&[("CO", &["C", "O"], "co_aliphatic_cleavage")], &["C"]);
    let renkin_outcome = normalize_renkin_route(&renkin_input, "CO");
    assert!(!renkin_outcome.parseable);
    assert!(
        renkin_outcome
            .defects
            .contains(&AuditFindingCode::AmbiguousLeafStatus)
    );

    let azf_node = azf_mol(
        "CO",
        Some(false),
        vec![azf_reaction(
            Some("[C:1][O:2]>>[C:1].[O:2]"),
            vec![azf_mol("C", Some(true), vec![]), azf_mol("O", None, vec![])],
        )],
    );
    let azf_outcome = normalize_aizynthfinder_route(&azf_node);
    assert!(!azf_outcome.parseable);
    assert!(
        azf_outcome
            .defects
            .contains(&AuditFindingCode::AmbiguousLeafStatus)
    );

    let syn_input = syntheseus_route("CO", &["C", "O"], &[("C", Some(true)), ("O", None)]);
    let syn_outcome = normalize_syntheseus_route(&syn_input);
    assert!(!syn_outcome.parseable);
    assert!(
        syn_outcome
            .defects
            .contains(&AuditFindingCode::AmbiguousLeafStatus)
    );

    let sp_node = synplanner_mol(
        "CO",
        Some(false),
        vec![synplanner_reaction(
            "[C:1][O:2]>>[C:1].[O:2]",
            vec![
                synplanner_mol("C", Some(true), vec![]),
                synplanner_mol("O", None, vec![]),
            ],
        )],
    );
    let sp_outcome = normalize_synplanner_route(&sp_node);
    assert!(!sp_outcome.parseable);
    assert!(
        sp_outcome
            .defects
            .contains(&AuditFindingCode::AmbiguousLeafStatus)
    );

    for (policy, expected) in [
        (AuditPolicy::Informational, AuditStatus::Partial),
        (AuditPolicy::Standard, AuditStatus::Fail),
        (AuditPolicy::Strict, AuditStatus::Fail),
    ] {
        let renkin_report = audit_with_policy(&renkin_outcome, None, None, policy);
        let azf_report = audit_with_policy(&azf_outcome, None, None, policy);
        let syn_report = audit_with_policy(&syn_outcome, None, None, policy);
        let sp_report = audit_with_policy(&sp_outcome, None, None, policy);
        assert_eq!(renkin_report.status, expected, "renkin, {policy:?}");
        assert_eq!(azf_report.status, expected, "aizynthfinder, {policy:?}");
        assert_eq!(syn_report.status, expected, "syntheseus, {policy:?}");
        assert_eq!(sp_report.status, expected, "synplanner, {policy:?}");
    }
}
