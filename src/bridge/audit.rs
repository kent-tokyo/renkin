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
use crate::chem_env::{RetroRule, mol_from_smiles};
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
    /// RENKIN Bridge PR4: a step's declared-reaction replay ran but its
    /// product didn't reproduce the recorded parent molecule.
    ForwardReactionNotReproduced,
    /// RENKIN Bridge PR4: a step's forward validation couldn't reach a
    /// pass/fail verdict -- see the step's own `forward_validation.reason`
    /// on [`AuditedStep`] for which of the six not-evaluable causes applied;
    /// this finding code stays generic on purpose (the specific reason
    /// already lives there, not duplicated into six finding codes).
    ForwardValidationNotEvaluable,
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
            AuditFindingCode::ChargeImbalance
            | AuditFindingCode::StereoCenterCountMismatch
            // Informational: a not-evaluable forward result must only ever
            // contribute to `AuditStatus::Partial`, derived separately from
            // each step's own status (see `audit_document`) -- never to a
            // `Fail` via this finding merely being present, the same way
            // element-accounting's own `NotEvaluable` never pushes a gating
            // finding either.
            | AuditFindingCode::ForwardValidationNotEvaluable => AuditSeverity::Informational,
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

/// Non-negotiable three-valued route-level verdict -- never a boolean.
/// `Fail` when structural parsing, stock, or forward validation has a clear
/// FAIL anywhere. `Partial` when nothing failed outright but at least one
/// check (stock, element-accounting, or any step's forward validation)
/// remains `not_evaluable` -- e.g. no configured stock to verify leaves
/// against, or a step whose declared reaction couldn't be replayed. Never
/// force-passed to `Pass` on missing data: that would make "we couldn't
/// check" look identical to "we checked and it's fine". RENKIN Bridge PR4
/// renamed this from a `NotEvaluable` variant covering the whole report to
/// `Partial` covering the whole *route* -- see [`CheckStatus`] for the
/// equivalent per-check (not route-level) three-valued outcome that
/// individual checks like [`StockValidationResult`] and
/// `bridge::forward::ForwardValidationResult` now use instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditStatus {
    Pass,
    Fail,
    Partial,
}

/// Generic three-valued outcome for one independent audit check -- distinct
/// from [`AuditStatus`], which is the route-level *aggregate* verdict.
/// Shared between [`StockValidationResult`] and
/// `bridge::forward::ForwardValidationResult` so both report the same
/// pass/fail/not_evaluable vocabulary the user asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Fail,
    NotEvaluable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StockNotEvaluableReason {
    StockNotProvided,
}

/// Independent of forward validation's own not-evaluable reasons (kept in a
/// separate field on each [`AuditedStep`]) -- stock-audit ("can the leaves
/// be obtained") and forward-audit ("can the parent really be reconstructed
/// from those precursors") are orthogonal checks, so their not-evaluable
/// causes must never be conflated into one field.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct StockValidationResult {
    pub status: CheckStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<StockNotEvaluableReason>,
}

/// One decomposition step plus its forward-validation outcome, the shape
/// [`AuditReport::steps`] serializes -- matches the P0 spec's per-step JSON
/// exactly: `{"target": ..., "precursors": [...], "forward_validation":
/// {"status": ..., "method": ..., "reason": ...}}`.
#[derive(Debug, Clone, Serialize)]
pub struct AuditedStep {
    pub target: String,
    pub precursors: Vec<String>,
    pub forward_validation: crate::bridge::forward::ForwardValidationResult,
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
    /// `None` when `route_tree_parseable` is false -- there is no tree to
    /// walk. Independent of forward validation's own not-evaluable field on
    /// each [`AuditedStep`] (see [`StockValidationResult`]'s doc comment for
    /// why the two must never be conflated).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stock_validation: Option<StockValidationResult>,
    /// `"accounted"` | `"unaccounted_target_element"` | `"not_evaluable"`,
    /// reusing `synthesizability::ElementAccountingStatus`'s exact values.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_element_accounting_status: Option<crate::synthesizability::ElementAccountingStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normalized_route_sha256: Option<String>,
    /// Every decomposition step with its forward-validation outcome. Empty
    /// when `route_tree_parseable` is false -- there is no tree to walk.
    pub steps: Vec<AuditedStep>,
    /// Empty `findings` under `status: Partial` means "this check never
    /// ran" (e.g. no `configured_stock` was supplied, so
    /// `validate_stock_leaves` never executed and could not have produced
    /// a `LeafUnresolved`/`LeafClaimedStockNotMatched` finding either way),
    /// not "this check ran and found nothing" -- always read `status`
    /// before treating an empty list as a clean bill of health.
    pub findings: Vec<AuditFinding>,
}

fn parse_failure_report(source: RouteSource, defects: &[AuditFindingCode]) -> AuditReport {
    AuditReport {
        source,
        status: AuditStatus::Fail,
        route_tree_parseable: false,
        reaction_steps_parseable: None,
        stock_validation: None,
        target_element_accounting_status: None,
        normalized_route_sha256: None,
        steps: Vec::new(),
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
/// for this audit. `None` means "no stock to check against" -- stock
/// validation is left `NotEvaluable: stock_not_provided`, never force-passed
/// as `Pass`, exactly the "competitor output missing data" case this
/// module's `AuditStatus` exists to represent honestly.
///
/// `rules`: RENKIN's own rule corpus, needed to resolve a RENKIN-sourced
/// step's `template_id` to its SMIRKS for forward validation
/// (`bridge::forward::validate_step_forward`). `None` (e.g. auditing an
/// AiZynthFinder route in isolation) leaves every RENKIN-evidenced step
/// `not_evaluable: missing_reaction_representation`; AiZynthFinder-evidenced
/// steps are unaffected either way.
pub fn audit(
    outcome: &ParseOutcome,
    configured_stock: Option<&HashSet<String>>,
    rules: Option<&[RetroRule]>,
) -> AuditReport {
    let (Some(document), true) = (&outcome.document, outcome.parseable) else {
        return parse_failure_report(outcome.source, &outcome.defects);
    };

    audit_document(document, configured_stock, rules)
}

/// Same as [`audit`], but for a [`RouteDocument`] the caller already has in
/// hand (e.g. built directly by RENKIN Bridge PR4's AiZynthFinder adapter,
/// which may not always route through a [`ParseOutcome`]).
pub fn audit_document(
    document: &RouteDocument,
    configured_stock: Option<&HashSet<String>>,
    rules: Option<&[RetroRule]>,
) -> AuditReport {
    let mut findings = Vec::new();

    let steps_ok = reaction_steps_parseable(&document.root);
    if !steps_ok {
        findings.push(AuditFinding::new(
            AuditFindingCode::DegenerateSelfReferentialStep,
        ));
    }

    let stock_validation = match configured_stock {
        Some(stock) => {
            let (ok, f) = validate_stock_leaves(&document.root, stock);
            findings.extend(f);
            StockValidationResult {
                status: if ok {
                    CheckStatus::Pass
                } else {
                    CheckStatus::Fail
                },
                reason: None,
            }
        }
        None => StockValidationResult {
            status: CheckStatus::NotEvaluable,
            reason: Some(StockNotEvaluableReason::StockNotProvided),
        },
    };

    let (element_status, element_findings) = target_element_accounting(&document.root);
    findings.extend(element_findings);

    // Indexed once, reused for every step -- keeps forward-validation's cost
    // proportional to step count, not `steps * rules`.
    let rules_by_template_id =
        rules.and_then(|rs| crate::candidate::index_rules_by_template_id(rs).ok());
    let steps: Vec<AuditedStep> = document
        .steps()
        .into_iter()
        .map(|step| {
            let forward_validation = crate::bridge::forward::validate_step_forward(
                &step.target,
                &step.precursors,
                step.reaction_evidence.as_ref(),
                rules_by_template_id.as_ref(),
            );
            match forward_validation.status {
                CheckStatus::Fail => findings.push(AuditFinding::at(
                    AuditFindingCode::ForwardReactionNotReproduced,
                    step.target.clone(),
                )),
                CheckStatus::NotEvaluable => findings.push(AuditFinding::at(
                    AuditFindingCode::ForwardValidationNotEvaluable,
                    step.target.clone(),
                )),
                CheckStatus::Pass => {}
            }
            AuditedStep {
                target: step.target,
                precursors: step.precursors,
                forward_validation,
            }
        })
        .collect();

    // Deliberately redundant with the Gating-findings check: stock/forward
    // Fail already push a Gating finding (`LeafClaimedStockNotMatched`/
    // `LeafUnresolved`/`ForwardReactionNotReproduced`), so this clause is
    // belt-and-braces, not dead code -- the verdict is meant to be
    // derivable directly from each check's own status, independent of
    // whether findings-severity classification ever changes.
    let any_fail = !steps_ok
        || findings.iter().any(|f| f.severity == AuditSeverity::Gating)
        || stock_validation.status == CheckStatus::Fail
        || steps
            .iter()
            .any(|s| s.forward_validation.status == CheckStatus::Fail);
    let any_not_evaluable = stock_validation.status == CheckStatus::NotEvaluable
        || element_status == crate::synthesizability::ElementAccountingStatus::NotEvaluable
        || steps
            .iter()
            .any(|s| s.forward_validation.status == CheckStatus::NotEvaluable);
    let status = if any_fail {
        AuditStatus::Fail
    } else if any_not_evaluable {
        AuditStatus::Partial
    } else {
        AuditStatus::Pass
    };

    AuditReport {
        source: document.source,
        status,
        route_tree_parseable: true,
        reaction_steps_parseable: Some(steps_ok),
        stock_validation: Some(stock_validation),
        target_element_accounting_status: Some(element_status),
        normalized_route_sha256: Some(crate::bridge::route_graph::normalized_route_sha256(
            document,
        )),
        steps,
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
    fn all_leaves_matched_and_element_accounted_is_partial_without_forward_rules() {
        let outcome = normalize_renkin_route(&single_step_route(), TARGET);
        // RENKIN Bridge PR4: with no rule corpus supplied, the step's
        // forward validation can't be evaluated (its `template_id` can't be
        // resolved to a SMIRKS) -- stock and element-accounting alone are no
        // longer sufficient for a route-level `Pass`, exactly the
        // independence the forward-validation axis exists to enforce (see
        // `bridge::forward` module docs).
        let report = audit(
            &outcome,
            Some(&stock(&[ETHANOL, BENZOIC_ACID, "CCN"])),
            None,
        );
        assert_eq!(report.status, AuditStatus::Partial);
        assert_eq!(
            report.stock_validation.as_ref().map(|s| s.status),
            Some(CheckStatus::Pass)
        );
        assert_eq!(
            report.target_element_accounting_status,
            Some(ElementAccountingStatus::Accounted)
        );
        assert_eq!(
            report.steps[0].forward_validation.status,
            CheckStatus::NotEvaluable
        );
    }

    #[test]
    fn leaf_claimed_stock_but_not_configured_fails() {
        let outcome = normalize_renkin_route(&single_step_route(), TARGET);
        // Configured stock (e.g. shared_stock mode) doesn't include benzoic acid.
        let report = audit(&outcome, Some(&stock(&[ETHANOL])), None);
        assert_eq!(report.status, AuditStatus::Fail);
        assert_eq!(
            report.stock_validation.as_ref().map(|s| s.status),
            Some(CheckStatus::Fail)
        );
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.code == AuditFindingCode::LeafClaimedStockNotMatched
                    && f.node.as_deref() == Some(canon(BENZOIC_ACID).as_str()))
        );
    }

    #[test]
    fn no_configured_stock_is_partial_not_a_silent_pass() {
        let outcome = normalize_renkin_route(&single_step_route(), TARGET);
        let report = audit(&outcome, None, None);
        assert_eq!(report.status, AuditStatus::Partial);
        let stock_validation = report.stock_validation.expect("stock check always runs");
        assert_eq!(stock_validation.status, CheckStatus::NotEvaluable);
        assert_eq!(
            stock_validation.reason,
            Some(StockNotEvaluableReason::StockNotProvided)
        );
    }

    #[test]
    fn atom_materializing_from_nowhere_is_unaccounted_and_fails() {
        // Chlorobenzene "from" bromobenzene: Cl appears in the product with
        // no precursor accounting for it. MW-based checks pass this
        // (bromobenzene is heavier); the per-element check must not.
        let r = route(vec![step("Clc1ccccc1", &["Brc1ccccc1"])], &["Brc1ccccc1"]);
        let outcome = normalize_renkin_route(&r, "Clc1ccccc1");
        assert!(outcome.parseable, "{:?}", outcome.defects);
        let report = audit(&outcome, Some(&stock(&["Brc1ccccc1"])), None);
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
            reaction_evidence: None,
            children: vec![],
        };
        let (status, findings) = target_element_accounting(&leaf);
        assert_eq!(status, ElementAccountingStatus::NotEvaluable);
        assert!(findings.is_empty());
    }

    #[test]
    fn informational_findings_never_gate_status_to_fail() {
        // Constructed directly: a target/precursor pair with the same
        // heavy-atom counts (so element-accounting passes) but a net
        // charge difference, to isolate that ChargeImbalance alone must
        // not fail the route. This node carries no `reaction_evidence`
        // (hand-built, not from `normalize_renkin_route`), so forward
        // validation is legitimately `not_evaluable` here too -- the route
        // lands on `Partial`, not `Pass`, but the assertion that matters is
        // that an Informational finding alone never pushes it to `Fail`.
        let target = "[NH4+]";
        let precursor = "N";
        let root = RouteNode {
            canonical_smiles: canon(target),
            is_stock_leaf: Some(false),
            reaction_evidence: None,
            children: vec![RouteNode {
                canonical_smiles: canon(precursor),
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
        let report = audit_document(&document, Some(&stock(&[precursor])), None);
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.code == AuditFindingCode::ChargeImbalance),
            "expected a ChargeImbalance finding, got {:?}",
            report.findings
        );
        assert_ne!(
            report.status,
            AuditStatus::Fail,
            "an Informational-severity finding must never gate AuditStatus to Fail, got {:?}",
            report
        );
    }

    #[test]
    fn unparseable_route_reports_fail_with_tree_not_parseable() {
        let r = route(vec![], &[]);
        let outcome = normalize_renkin_route(&r, TARGET);
        let report = audit(&outcome, None, None);
        assert_eq!(report.status, AuditStatus::Fail);
        assert!(!report.route_tree_parseable);
        assert_eq!(report.reaction_steps_parseable, None);
        assert_eq!(report.normalized_route_sha256, None);
        assert!(report.steps.is_empty());
        assert!(report.stock_validation.is_none());
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.code == AuditFindingCode::MultipleOrZeroRoots)
        );
        assert_eq!(
            report.source,
            RouteSource::Renkin,
            "a real normalize_renkin_route() failure must report its actual source"
        );
    }

    /// Regression: `audit()`'s failure path used to derive `source` from
    /// `outcome.document` (`.map(|d| d.source).unwrap_or(Renkin)`) --
    /// always `None` on this exact path by `ParseOutcome`'s own contract
    /// (`document` is `Some` only when parseable), so every failed audit
    /// silently reported `source: "renkin"` regardless of which tool the
    /// route actually came from. `ParseOutcome::source` is now set by
    /// whichever normalizer ran, independent of `document`; this
    /// constructs a `ParseOutcome` directly (simulating what RENKIN Bridge
    /// PR4's AiZynthFinder normalizer will eventually produce) to prove a
    /// failed audit doesn't relabel a competitor's broken route as
    /// RENKIN's own.
    #[test]
    fn failed_audit_preserves_a_non_renkin_source() {
        let outcome = ParseOutcome {
            source: RouteSource::AiZynthFinder,
            document: None,
            parseable: false,
            defects: vec![AuditFindingCode::RawOutputNotDecodable],
        };
        let report = audit(&outcome, None, None);
        assert_eq!(report.status, AuditStatus::Fail);
        assert_eq!(report.source, RouteSource::AiZynthFinder);
    }

    #[test]
    fn normal_route_reaction_steps_are_parseable() {
        let outcome = normalize_renkin_route(&single_step_route(), TARGET);
        let document = outcome.document.unwrap();
        assert!(reaction_steps_parseable(&document.root));
    }

    // ── RENKIN Bridge PR4: forward validation folded into the route audit ──

    fn co_aliphatic_cleavage_rule() -> RetroRule {
        RetroRule {
            name: "co_aliphatic_cleavage".to_string(),
            template_id: "t1".to_string(),
            smirks: "[C:1][O:2]>>[C:1].[O:2]".to_string(),
            ..Default::default()
        }
    }

    /// Structural audit PASS + stock PASS + forward FAIL must coexist:
    /// proves the three axes are genuinely independent, not one gating the
    /// others. Formaldehyde ("C=O") has the same heavy-atom counts as
    /// methanol ("CO"), so element-accounting is satisfied even though
    /// `co_aliphatic_cleavage`'s forward replay of methane + water
    /// deterministically produces methanol, never formaldehyde.
    #[test]
    fn independence_structural_and_stock_pass_while_forward_fails() {
        let methane = canon("C");
        let water = canon("O");
        let root = RouteNode {
            canonical_smiles: canon("C=O"),
            is_stock_leaf: Some(false),
            reaction_evidence: Some(
                crate::bridge::route_graph::ReactionEvidence::RenkinTemplate {
                    template_id: "t1".to_string(),
                },
            ),
            children: vec![
                RouteNode {
                    canonical_smiles: methane.clone(),
                    is_stock_leaf: Some(true),
                    reaction_evidence: None,
                    children: vec![],
                },
                RouteNode {
                    canonical_smiles: water.clone(),
                    is_stock_leaf: Some(true),
                    reaction_evidence: None,
                    children: vec![],
                },
            ],
        };
        let document = RouteDocument {
            source: RouteSource::Renkin,
            step_count_collapsed_edges: 1,
            root,
        };
        let rules = vec![co_aliphatic_cleavage_rule()];
        let stock: HashSet<String> = [methane, water].into_iter().collect();
        let report = audit_document(&document, Some(&stock), Some(&rules));

        assert_eq!(
            report.stock_validation.as_ref().map(|s| s.status),
            Some(CheckStatus::Pass),
            "{report:?}"
        );
        assert_eq!(
            report.target_element_accounting_status,
            Some(ElementAccountingStatus::Accounted),
            "{report:?}"
        );
        assert_eq!(report.steps[0].forward_validation.status, CheckStatus::Fail);
        assert_eq!(
            report.status,
            AuditStatus::Fail,
            "a clear forward FAIL must fail the route even though stock and \
             structural checks independently pass"
        );
    }

    /// Forward validation being `not_evaluable` must never lose or corrupt
    /// the structural/stock/element-accounting parts of the report -- a
    /// regression this codebase's own design principle (three-valued,
    /// never silently collapsed) is meant to prevent, but only a direct
    /// assertion on every field catches an accidental early-return.
    #[test]
    fn forward_not_evaluable_does_not_corrupt_other_audit_fields() {
        let outcome = normalize_renkin_route(&single_step_route(), TARGET);
        // No rule corpus supplied -> every step's forward_validation is
        // not_evaluable, but nothing else about the audit should change.
        let report = audit(
            &outcome,
            Some(&stock(&[ETHANOL, BENZOIC_ACID, "CCN"])),
            None,
        );

        assert!(report.route_tree_parseable);
        assert_eq!(report.reaction_steps_parseable, Some(true));
        assert_eq!(
            report.stock_validation.as_ref().map(|s| s.status),
            Some(CheckStatus::Pass)
        );
        assert_eq!(
            report.target_element_accounting_status,
            Some(ElementAccountingStatus::Accounted)
        );
        assert!(report.normalized_route_sha256.is_some());
        assert_eq!(report.steps.len(), 1);
        assert_eq!(
            report.steps[0].forward_validation.status,
            CheckStatus::NotEvaluable
        );
        assert_eq!(
            report.status,
            AuditStatus::Partial,
            "not_evaluable forward validation alone must yield Partial, \
             never silently Pass and never Fail"
        );
    }
}
