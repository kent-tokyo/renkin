//! Tool-neutral post-hoc route audit: promotes `scripts/compare_validation.py`
//! into Rust. Operates ONLY on an already-normalized [`RouteDocument`] --
//! never on a tool's native output shape directly -- so this logic runs
//! identically regardless of which tool produced the route. See
//! `crate::bridge` module docs for the parity contract with that file.
//!
//! Caveat that must accompany every metric this module produces (matching
//! `compare_validation.py`'s own `CAVEAT_TEXT`): atom-accounted does not
//! mean chemically correct; canonical-SMILES leaf matching does not
//! account for tautomers or differing stereochemistry conventions; no
//! route audited here has been reviewed by a human chemist.

use std::collections::{HashMap, HashSet};

use chematic::core::Element;
use serde::Serialize;

use crate::bridge::route_graph::{ParseOutcome, RouteDocument, RouteNode, RouteSource};
use crate::chem_env::mol_from_smiles;
use crate::synthesizability::heavy_atom_counts;

/// The finding taxonomy, closed set. Mirrors the constants defined across
/// `compare_route_graph.py` and `compare_validation.py` exactly (same
/// string spelling under `#[serde(rename_all = "snake_case")]`, so a JSON
/// report's `code` field reads identically to the Python harness's own
/// `defects`/`common_validation_warnings` lists).
///
/// `DisconnectedReference` and `StepArityMismatch` are ported for schema
/// completeness (the Python reference reserves both constants) but neither
/// is ever emitted there -- confirmed by grep, nothing in
/// `scripts/tests/test_compare_route_graph.py` exercises them either. Do
/// not add code here that emits them: doing so would break fixture parity
/// in the direction that's hardest to notice (a finding the oracle never
/// produces).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditFindingCode {
    RawOutputNotDecodable,
    MultipleOrZeroRoots,
    RootMismatch,
    CycleDetected,
    DisconnectedReference,
    UnparseableSmilesInRoute,
    ChildlessNonLeaf,
    AmbiguousLeafStatus,
    DegenerateSelfReferentialStep,
    StepArityMismatch,
    LeafClaimedStockNotMatched,
    LeafUnresolved,
    UnaccountedTargetElement,
    ChargeImbalance,
    StereoCenterCountMismatch,
}

/// Whether a finding can fail [`AuditStatus`] on its own, or is purely
/// informational. `ChargeImbalance`/`StereoCenterCountMismatch` are
/// `Informational` -- `compare_validation.py`'s own comment marks both
/// "informational only, never gates".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditSeverity {
    Gating,
    Informational,
}

impl AuditFindingCode {
    fn severity(self) -> AuditSeverity {
        match self {
            AuditFindingCode::ChargeImbalance | AuditFindingCode::StereoCenterCountMismatch => {
                AuditSeverity::Informational
            }
            _ => AuditSeverity::Gating,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AuditFinding {
    pub code: AuditFindingCode,
    pub severity: AuditSeverity,
    /// Canonical SMILES of the node this finding is about, when
    /// applicable -- route-level findings (e.g. `RootMismatch`,
    /// `MultipleOrZeroRoots`) have none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
}

impl AuditFinding {
    fn new(code: AuditFindingCode) -> Self {
        Self {
            code,
            severity: code.severity(),
            node: None,
        }
    }

    fn at(code: AuditFindingCode, node: impl Into<String>) -> Self {
        Self {
            code,
            severity: code.severity(),
            node: Some(node.into()),
        }
    }
}

/// Non-negotiable three-valued verdict -- never a boolean. A route whose
/// audit couldn't fully run (e.g. no configured stock to verify leaves
/// against) reports `NotEvaluable`, never a force-passed `Pass`: passing
/// silently on missing data would make "we couldn't check" look identical
/// to "we checked and it's fine".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditStatus {
    Pass,
    Fail,
    NotEvaluable,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuditReport {
    pub source: RouteSource,
    pub status: AuditStatus,
    pub route_tree_parseable: bool,
    /// `None` when `route_tree_parseable` is false -- there is no tree to
    /// walk. `_TREE_DEPENDENT_FIELDS` in `compare_schema.py` enforces the
    /// identical nullability contract.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reaction_steps_parseable: Option<bool>,
    /// `None` when not parseable, or when no stock was supplied to check
    /// against (`configured_stock: None` in [`audit`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub all_leaves_in_configured_stock: Option<bool>,
    /// `"accounted"` | `"unaccounted_target_element"` | `"not_evaluable"`,
    /// reusing `synthesizability::ElementAccountingStatus`'s exact values.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_element_accounting_status: Option<crate::synthesizability::ElementAccountingStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normalized_route_sha256: Option<String>,
    pub findings: Vec<AuditFinding>,
}

fn parse_failure_report(source: RouteSource, defects: &[AuditFindingCode]) -> AuditReport {
    AuditReport {
        source,
        status: AuditStatus::Fail,
        route_tree_parseable: false,
        reaction_steps_parseable: None,
        all_leaves_in_configured_stock: None,
        target_element_accounting_status: None,
        normalized_route_sha256: None,
        findings: defects.iter().copied().map(AuditFinding::new).collect(),
    }
}

/// Per-edge check: no residual self-loop in the already-normalized tree.
/// Precondition (mirrors `compare_validation.py`'s
/// `check_reaction_steps_parseable` docstring): only meaningful once the
/// tree is already known to parse -- `normalize_renkin_route` already
/// rejects any self-loop it finds (`DegenerateSelfReferentialStep`), so
/// this can only ever observe `true` on a real, already-normalized
/// [`RouteDocument`]. Ported anyway for exact parity with the reference
/// and as a structural invariant check, not dead code by intent.
fn reaction_steps_parseable(root: &RouteNode) -> bool {
    fn walk(node: &RouteNode) -> bool {
        let mut ok = true;
        for child in &node.children {
            if child.canonical_smiles == node.canonical_smiles {
                ok = false;
            }
            ok = walk(child) && ok;
        }
        ok
    }
    walk(root)
}

fn validate_stock_leaves(
    root: &RouteNode,
    configured_stock: &HashSet<String>,
) -> (bool, Vec<AuditFinding>) {
    let mut findings = Vec::new();
    let mut all_ok = true;

    fn iter_leaves<'a>(node: &'a RouteNode, out: &mut Vec<&'a RouteNode>) {
        if node.children.is_empty() {
            out.push(node);
        } else {
            for c in &node.children {
                iter_leaves(c, out);
            }
        }
    }
    let mut leaves = Vec::new();
    iter_leaves(root, &mut leaves);

    for leaf in leaves {
        match leaf.is_stock_leaf {
            Some(true) => {
                if !configured_stock.contains(&leaf.canonical_smiles) {
                    findings.push(AuditFinding::at(
                        AuditFindingCode::LeafClaimedStockNotMatched,
                        leaf.canonical_smiles.clone(),
                    ));
                    all_ok = false;
                }
            }
            Some(false) | None => {
                // `None` (ambiguous leaf status) should already have
                // failed `route_tree_parseable` upstream; treated
                // defensively as unresolved if ever reached here anyway,
                // matching `compare_validation.py`'s own comment.
                findings.push(AuditFinding::at(
                    AuditFindingCode::LeafUnresolved,
                    leaf.canonical_smiles.clone(),
                ));
                all_ok = false;
            }
        }
    }
    (all_ok, findings)
}

fn net_charge(canonical_smiles: &str) -> Option<i32> {
    let mol = mol_from_smiles(canonical_smiles).ok()?;
    Some(mol.atoms().map(|(_, a)| i32::from(a.charge)).sum())
}

fn stereo_center_count(canonical_smiles: &str) -> Option<usize> {
    let mol = mol_from_smiles(canonical_smiles).ok()?;
    Some(
        mol.atoms()
            .filter(|(_, a)| a.chirality != chematic::core::Chirality::None)
            .count(),
    )
}

/// Directional per-element check plus two informational-only byproducts
/// (charge/stereo-center balance), ported from `compare_validation.py`'s
/// `check_target_element_accounting`. Walks every edge of the tree, not
/// just RENKIN's own per-step fields -- this is what makes the check
/// tool-neutral (an AiZynthFinder-sourced document has no `ReactionStep`
/// at all, only the tree PR3 built from it).
fn target_element_accounting(
    root: &RouteNode,
) -> (
    crate::synthesizability::ElementAccountingStatus,
    Vec<AuditFinding>,
) {
    use crate::synthesizability::ElementAccountingStatus;

    let mut findings = Vec::new();
    let mut any_evaluated = false;
    let mut unaccounted = false;

    fn walk(
        node: &RouteNode,
        any_evaluated: &mut bool,
        unaccounted: &mut bool,
        findings: &mut Vec<AuditFinding>,
    ) {
        if !node.children.is_empty()
            && let Some(target_counts) = heavy_atom_counts(&node.canonical_smiles)
        {
            let mut precursor_counts: HashMap<Element, usize> = HashMap::new();
            let mut countable = true;
            for c in &node.children {
                match heavy_atom_counts(&c.canonical_smiles) {
                    Some(counts) => {
                        for (el, n) in counts {
                            *precursor_counts.entry(el).or_insert(0) += n;
                        }
                    }
                    None => countable = false,
                }
            }
            if countable {
                *any_evaluated = true;
                let elements_in_excess = target_counts
                    .iter()
                    .any(|(el, n)| *n > precursor_counts.get(el).copied().unwrap_or(0));
                if elements_in_excess {
                    *unaccounted = true;
                    findings.push(AuditFinding::at(
                        AuditFindingCode::UnaccountedTargetElement,
                        node.canonical_smiles.clone(),
                    ));
                }

                let target_charge = net_charge(&node.canonical_smiles);
                let precursor_charge: i32 = node
                    .children
                    .iter()
                    .map(|c| net_charge(&c.canonical_smiles).unwrap_or(0))
                    .sum();
                if target_charge.is_some_and(|t| t != precursor_charge) {
                    findings.push(AuditFinding::at(
                        AuditFindingCode::ChargeImbalance,
                        node.canonical_smiles.clone(),
                    ));
                }

                let target_stereo = stereo_center_count(&node.canonical_smiles);
                let precursor_stereo: usize = node
                    .children
                    .iter()
                    .map(|c| stereo_center_count(&c.canonical_smiles).unwrap_or(0))
                    .sum();
                if target_stereo.is_some_and(|t| t != precursor_stereo) {
                    findings.push(AuditFinding::at(
                        AuditFindingCode::StereoCenterCountMismatch,
                        node.canonical_smiles.clone(),
                    ));
                }
            }
        }
        for c in &node.children {
            walk(c, any_evaluated, unaccounted, findings);
        }
    }
    walk(root, &mut any_evaluated, &mut unaccounted, &mut findings);

    let status = if !any_evaluated {
        ElementAccountingStatus::NotEvaluable
    } else if unaccounted {
        ElementAccountingStatus::UnaccountedTargetElement
    } else {
        ElementAccountingStatus::Accounted
    };
    (status, findings)
}

/// Audits an already-normalized route. Takes a [`ParseOutcome`] (not a raw
/// route) so this function can never see a tool's native output shape
/// directly -- normalization (`normalize_renkin_route`, and RENKIN Bridge
/// PR4's AiZynthFinder equivalent) is always a separate, prior step.
///
/// `configured_stock`: canonical SMILES of the stock actually configured
/// for this audit. `None` means "no stock to check against" -- leaf
/// resolution is left `NotEvaluable`, never force-passed as `Pass`, exactly
/// the "competitor output missing data" case this module's `AuditStatus`
/// exists to represent honestly.
pub fn audit(outcome: &ParseOutcome, configured_stock: Option<&HashSet<String>>) -> AuditReport {
    let (Some(document), true) = (&outcome.document, outcome.parseable) else {
        let source = outcome
            .document
            .as_ref()
            .map(|d| d.source)
            .unwrap_or(RouteSource::Renkin);
        return parse_failure_report(source, &outcome.defects);
    };

    audit_document(document, configured_stock)
}

/// Same as [`audit`], but for a [`RouteDocument`] the caller already has in
/// hand (e.g. built directly by RENKIN Bridge PR4's AiZynthFinder adapter,
/// which may not always route through a [`ParseOutcome`]).
pub fn audit_document(
    document: &RouteDocument,
    configured_stock: Option<&HashSet<String>>,
) -> AuditReport {
    let mut findings = Vec::new();

    let steps_ok = reaction_steps_parseable(&document.root);
    if !steps_ok {
        findings.push(AuditFinding::new(
            AuditFindingCode::DegenerateSelfReferentialStep,
        ));
    }

    let (all_leaves_ok, stock_findings) = match configured_stock {
        Some(stock) => {
            let (ok, f) = validate_stock_leaves(&document.root, stock);
            (Some(ok), f)
        }
        None => (None, Vec::new()),
    };
    findings.extend(stock_findings);

    let (element_status, element_findings) = target_element_accounting(&document.root);
    findings.extend(element_findings);

    let has_gating_finding = findings.iter().any(|f| f.severity == AuditSeverity::Gating);
    let status = if !steps_ok || has_gating_finding {
        AuditStatus::Fail
    } else if all_leaves_ok.is_none()
        || element_status == crate::synthesizability::ElementAccountingStatus::NotEvaluable
    {
        AuditStatus::NotEvaluable
    } else {
        AuditStatus::Pass
    };

    AuditReport {
        source: document.source,
        status,
        route_tree_parseable: true,
        reaction_steps_parseable: Some(steps_ok),
        all_leaves_in_configured_stock: all_leaves_ok,
        target_element_accounting_status: Some(element_status),
        normalized_route_sha256: Some(crate::bridge::route_graph::normalized_route_sha256(
            document,
        )),
        findings,
    }
}

// Fixture-parity oracle: `scripts/tests/test_compare_validation.py`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::route_graph::normalize_renkin_route;
    use crate::search::{AtomEconomyStatus, ReactionStep, Route};
    use crate::synthesizability::ElementAccountingStatus;

    const TARGET: &str = "CCOC(=O)c1ccccc1";
    const ETHANOL: &str = "CCO";
    const BENZOIC_ACID: &str = "O=C(O)c1ccccc1";

    fn canon(smiles: &str) -> String {
        crate::chem_env::to_canonical(&mol_from_smiles(smiles).unwrap())
    }

    fn step(target: &str, precursors: &[&str]) -> ReactionStep {
        ReactionStep {
            rule: "r".to_string(),
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

    fn stock(smiles: &[&str]) -> HashSet<String> {
        smiles.iter().map(|s| canon(s)).collect()
    }

    #[test]
    fn all_leaves_matched_and_element_accounted_is_pass() {
        let outcome = normalize_renkin_route(&single_step_route(), TARGET);
        let report = audit(&outcome, Some(&stock(&[ETHANOL, BENZOIC_ACID, "CCN"])));
        assert_eq!(report.status, AuditStatus::Pass);
        assert_eq!(report.all_leaves_in_configured_stock, Some(true));
        assert_eq!(
            report.target_element_accounting_status,
            Some(ElementAccountingStatus::Accounted)
        );
    }

    #[test]
    fn leaf_claimed_stock_but_not_configured_fails() {
        let outcome = normalize_renkin_route(&single_step_route(), TARGET);
        // Configured stock (e.g. shared_stock mode) doesn't include benzoic acid.
        let report = audit(&outcome, Some(&stock(&[ETHANOL])));
        assert_eq!(report.status, AuditStatus::Fail);
        assert_eq!(report.all_leaves_in_configured_stock, Some(false));
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.code == AuditFindingCode::LeafClaimedStockNotMatched
                    && f.node.as_deref() == Some(canon(BENZOIC_ACID).as_str()))
        );
    }

    #[test]
    fn no_configured_stock_is_not_evaluable_not_a_silent_pass() {
        let outcome = normalize_renkin_route(&single_step_route(), TARGET);
        let report = audit(&outcome, None);
        assert_eq!(report.status, AuditStatus::NotEvaluable);
        assert_eq!(report.all_leaves_in_configured_stock, None);
    }

    #[test]
    fn atom_materializing_from_nowhere_is_unaccounted_and_fails() {
        // Chlorobenzene "from" bromobenzene: Cl appears in the product with
        // no precursor accounting for it. MW-based checks pass this
        // (bromobenzene is heavier); the per-element check must not.
        let r = route(vec![step("Clc1ccccc1", &["Brc1ccccc1"])], &["Brc1ccccc1"]);
        let outcome = normalize_renkin_route(&r, "Clc1ccccc1");
        assert!(outcome.parseable, "{:?}", outcome.defects);
        let report = audit(&outcome, Some(&stock(&["Brc1ccccc1"])));
        assert_eq!(report.status, AuditStatus::Fail);
        assert_eq!(
            report.target_element_accounting_status,
            Some(ElementAccountingStatus::UnaccountedTargetElement)
        );
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.code == AuditFindingCode::UnaccountedTargetElement)
        );
    }

    #[test]
    fn leaf_only_route_element_accounting_is_not_evaluable() {
        // A route consisting of the target itself as a stock leaf has no
        // steps at all -- mirror the Python fixture by constructing a
        // single-node tree directly rather than through normalization
        // (which requires >=1 step; see `MultipleOrZeroRoots`).
        let leaf = RouteNode {
            canonical_smiles: canon(TARGET),
            is_stock_leaf: Some(true),
            children: vec![],
        };
        let (status, findings) = target_element_accounting(&leaf);
        assert_eq!(status, ElementAccountingStatus::NotEvaluable);
        assert!(findings.is_empty());
    }

    #[test]
    fn informational_findings_never_gate_status() {
        // Constructed directly: a target/precursor pair with the same
        // heavy-atom counts (so element-accounting passes) but a net
        // charge difference, to isolate that ChargeImbalance alone must
        // not fail the route.
        let target = "[NH4+]";
        let precursor = "N";
        let root = RouteNode {
            canonical_smiles: canon(target),
            is_stock_leaf: Some(false),
            children: vec![RouteNode {
                canonical_smiles: canon(precursor),
                is_stock_leaf: Some(true),
                children: vec![],
            }],
        };
        let document = RouteDocument {
            source: RouteSource::Renkin,
            step_count_collapsed_edges: 1,
            root,
        };
        let report = audit_document(&document, Some(&stock(&[precursor])));
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.code == AuditFindingCode::ChargeImbalance),
            "expected a ChargeImbalance finding, got {:?}",
            report.findings
        );
        assert_eq!(
            report.status,
            AuditStatus::Pass,
            "an Informational-severity finding must never gate AuditStatus, got {:?}",
            report
        );
    }

    #[test]
    fn unparseable_route_reports_fail_with_tree_not_parseable() {
        let r = route(vec![], &[]);
        let outcome = normalize_renkin_route(&r, TARGET);
        let report = audit(&outcome, None);
        assert_eq!(report.status, AuditStatus::Fail);
        assert!(!report.route_tree_parseable);
        assert_eq!(report.reaction_steps_parseable, None);
        assert_eq!(report.normalized_route_sha256, None);
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.code == AuditFindingCode::MultipleOrZeroRoots)
        );
    }

    #[test]
    fn normal_route_reaction_steps_are_parseable() {
        let outcome = normalize_renkin_route(&single_step_route(), TARGET);
        let document = outcome.document.unwrap();
        assert!(reaction_steps_parseable(&document.root));
    }
}
