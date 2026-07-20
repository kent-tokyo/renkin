#![forbid(unsafe_code)]

//! Route/step plausibility checks, shared by `renkin-bench` and `renkin-mcp`.
//!
//! Forward validation historically returned a single bool per step, computed
//! by reverse-applying every rule's SMIRKS to the step's precursors. Seven
//! hand-crafted rules (ester/amide/Suzuki/sulfonamide/sulfone/Boc/Cbz
//! cleavage) are graph-based — they cut bonds directly in the target's
//! molecular graph instead of matching a SMIRKS pattern — so they carry an
//! empty `smirks` string and could never pass that check. That conflated
//! "this route step is chemically wrong" with "the validator has no method
//! for this rule family", silently collapsing both to `false`. The
//! [`StepValidationStatus`] three-way split keeps them apart.

pub mod atom_conservation;
pub mod forward;
pub mod graph_rules;

use serde::Serialize;

use crate::chem_env::RetroRule;
use crate::search::ReactionStep;

pub use atom_conservation::{route_balanced, step_balanced};
pub use forward::route_forward_validated;

/// Per-step forward-validation outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepValidationStatus {
    /// A validation method (SMIRKS reversal or graph-rule structural check) confirmed the step.
    Valid,
    /// A validation method ran and did not confirm the step.
    Invalid,
    /// No validation method covers this step's rule or its SMILES failed to parse.
    NotEvaluable,
}

/// Route-level rollup of its steps' [`StepValidationStatus`] values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteValidationStatus {
    /// Every step is `Valid`.
    Validated,
    /// At least one step is `Invalid` (regardless of the rest).
    Invalid,
    /// No `Invalid` steps, but a mix of `Valid` and `NotEvaluable`.
    PartiallyValidated,
    /// Every step is `NotEvaluable`.
    NotEvaluable,
}

/// Validate one route step: try SMIRKS-reversal forward validation first
/// (unchanged behavior for the ~24 SMIRKS-based rules), then fall back to a
/// rule-specific graph structural check for the 7 graph-based rules.
pub fn validate_step(step: &ReactionStep, rules: &[RetroRule]) -> StepValidationStatus {
    if forward::smirks_reproduces(&step.target, &step.precursors, rules) {
        return StepValidationStatus::Valid;
    }
    match rules.iter().find(|r| r.name == step.rule) {
        Some(r) if r.smirks.is_empty() => {
            graph_rules::validate_graph_step(&step.rule, &step.target, &step.precursors)
        }
        // Rule has SMIRKS but reversal didn't reproduce the target — a real mismatch.
        Some(_) => StepValidationStatus::Invalid,
        // Rule name not found (e.g. extracted template with no name match) — can't evaluate.
        None => StepValidationStatus::NotEvaluable,
    }
}

/// Roll up a route's step statuses. Invalid dominates (don't trust a route with
/// even one confirmed-wrong step); otherwise all-Valid → Validated, all-NotEvaluable
/// → NotEvaluable, and a Valid/NotEvaluable mix → PartiallyValidated.
pub fn aggregate_route(statuses: &[StepValidationStatus]) -> RouteValidationStatus {
    if statuses.contains(&StepValidationStatus::Invalid) {
        return RouteValidationStatus::Invalid;
    }
    if statuses.iter().all(|s| *s == StepValidationStatus::Valid) {
        return RouteValidationStatus::Validated;
    }
    if statuses
        .iter()
        .all(|s| *s == StepValidationStatus::NotEvaluable)
    {
        return RouteValidationStatus::NotEvaluable;
    }
    RouteValidationStatus::PartiallyValidated
}

/// Validate every step of a route and return its rolled-up status.
pub fn validate_route_steps(
    steps: &[ReactionStep],
    rules: &[RetroRule],
) -> (Vec<StepValidationStatus>, RouteValidationStatus) {
    let statuses: Vec<StepValidationStatus> =
        steps.iter().map(|s| validate_step(s, rules)).collect();
    let route_status = aggregate_route(&statuses);
    (statuses, route_status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use StepValidationStatus::{Invalid, NotEvaluable, Valid};

    // ── aggregate_route rollup rules ─────────────────────────────────────────
    #[test]
    fn aggregate_all_valid_is_validated() {
        assert_eq!(
            aggregate_route(&[Valid, Valid]),
            RouteValidationStatus::Validated
        );
    }

    #[test]
    fn aggregate_one_invalid_dominates() {
        // Invalid wins even with Valid steps present — one confirmed-wrong step
        // sinks the whole route.
        assert_eq!(
            aggregate_route(&[Valid, Invalid, Valid]),
            RouteValidationStatus::Invalid
        );
    }

    #[test]
    fn aggregate_all_not_evaluable() {
        assert_eq!(
            aggregate_route(&[NotEvaluable, NotEvaluable]),
            RouteValidationStatus::NotEvaluable
        );
    }

    #[test]
    fn aggregate_valid_and_not_evaluable_mix_is_partial() {
        assert_eq!(
            aggregate_route(&[Valid, NotEvaluable]),
            RouteValidationStatus::PartiallyValidated
        );
    }

    #[test]
    fn aggregate_empty_is_validated() {
        // Vacuously true: no steps, nothing failed to validate.
        assert_eq!(aggregate_route(&[]), RouteValidationStatus::Validated);
    }

    // ── SMIRKS-based rule path is unchanged by this refactor ────────────────
    fn step(rule: &str, target: &str, precursors: &[&str]) -> ReactionStep {
        ReactionStep {
            rule: rule.to_string(),
            target: target.to_string(),
            precursors: precursors.iter().map(|s| s.to_string()).collect(),
            conditions: None,
            atom_economy: None,
            step_confidence: 1.0,
            procedure_hint: None,
            reaction_family: None,
        }
    }

    #[test]
    fn smirks_rule_step_valid_on_forward_match() {
        // Friedel-Crafts acylation retro (SMIRKS-based, not one of the 7 graph
        // rules): acetophenone → benzene + acetyl chloride.
        let rules = crate::chem_env::default_rules();
        let s = step(
            "friedel_crafts_acylation_retro",
            "CC(=O)c1ccccc1",
            &["c1ccccc1", "CC(=O)Cl"],
        );
        assert_eq!(validate_step(&s, &rules), Valid);
    }

    #[test]
    fn smirks_rule_step_invalid_on_forward_mismatch() {
        let rules = crate::chem_env::default_rules();
        let s = step("friedel_crafts_acylation_retro", "CC(=O)c1ccccc1", &["CCO"]);
        assert_eq!(validate_step(&s, &rules), Invalid);
    }

    // ── graph-rule step routes through graph_rules::validate_graph_step ─────
    #[test]
    fn graph_rule_step_valid() {
        let rules = crate::chem_env::default_rules();
        let s = step(
            "ester_cleavage",
            "CC(=O)Oc1ccccc1",
            &["CC(=O)O", "Oc1ccccc1"],
        );
        assert_eq!(validate_step(&s, &rules), Valid);
    }

    #[test]
    fn graph_rule_step_invalid() {
        let rules = crate::chem_env::default_rules();
        let s = step("ester_cleavage", "CC(=O)Oc1ccccc1", &["CCO"]);
        assert_eq!(validate_step(&s, &rules), Invalid);
    }
}
