//! RENKIN Bridge PR6: cross-tool audit parity. Audits a real captured
//! AiZynthFinder route (`tests/fixtures/aizynthfinder/v4.4.1/single_trees.json`,
//! route index 0 -- see `PROVENANCE.md`) side by side with a hand-built
//! RENKIN-native route describing the *same* chemistry, and asserts the
//! two audits agree on everything tool-neutral: canonical root, leaf
//! multiset, step count, the set of structural finding codes, target
//! element accounting, and the configured-stock verdict.
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

use renkin::bridge::{
    AuditFindingCode, AuditStatus, AzfNode, CheckStatus, ParseOutcome, ReactionEvidence,
    RouteDocument, RouteNode, RouteSource, audit, normalize_aizynthfinder_route,
};
use renkin::chem_env::{RetroRule, mol_from_smiles, to_canonical};

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
