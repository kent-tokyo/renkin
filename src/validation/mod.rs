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

/// Validate one route step against the rule it actually claims to have used
/// (`step.rule`), looked up by name in `rules` first:
///
/// - Not found (e.g. an extracted template RENKIN can't match back): `NotEvaluable`.
/// - Found, graph-based (empty `smirks`): routed to the dedicated structural
///   check for the 7 graph-based rules.
/// - Found, SMIRKS-based: `Valid` only if *that rule's own* reversed SMIRKS
///   reproduces the target from the precursors, `Invalid` otherwise.
///
/// Deliberately does NOT fall back to "does any other rule's SMIRKS happen to
/// reproduce this target" — a coincidental match from an unrelated rule
/// doesn't confirm the step's own claimed rule was chemically valid, it just
/// means two unrelated transformations connect the same two SMILES strings.
/// See `forward::smirks_reproduces` for that broader (non-provenance-bound)
/// check, still used elsewhere for "is this route chemically plausible at all".
pub fn validate_step(step: &ReactionStep, rules: &[RetroRule]) -> StepValidationStatus {
    match rules.iter().find(|r| r.name == step.rule) {
        Some(r) if r.smirks.is_empty() => {
            graph_rules::validate_graph_step(&step.rule, &step.target, &step.precursors)
        }
        Some(r) => {
            if forward::rule_reproduces(&step.target, &step.precursors, r) {
                StepValidationStatus::Valid
            } else {
                StepValidationStatus::Invalid
            }
        }
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
            template_id: format!("rule:{rule}"),
            target: target.to_string(),
            precursors: precursors.iter().map(|s| s.to_string()).collect(),
            conditions: None,
            atom_economy: None,
            step_confidence: 1.0,
            procedure_hint: None,
            reaction_family: None,
            metadata_source: None,
            metadata_scope: None,
            evidence: None,
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

    // ── unmatched rule name → NotEvaluable ───────────────────────────────────
    #[test]
    fn unmatched_rule_name_not_evaluable() {
        let rules = crate::chem_env::default_rules();
        let s = step("some_extracted_template_no_name_match", "CCO", &["CC", "O"]);
        assert_eq!(validate_step(&s, &rules), NotEvaluable);
    }

    // ── core fix: cross-rule corroboration must never upgrade a step ────────
    // Regression test for the real false positive found via
    // examples/inspect_validation.rs against a USPTO-50k re-measurement: 7 of
    // 49 routes marked `validated` contained a step using `aryl_chloride_retro`
    // ("[c:1][Cl]>>[c:1]", atom-imbalanced — it drops the leaving Cl with no
    // accounting for it) that was nonetheless marked `Valid` under the old
    // logic only because an unrelated rule's reversed SMIRKS happened to also
    // reproduce the target from the step's (wrong) precursor.
    //
    // Inline minimal rule set — deliberately NOT `crate::chem_env::default_rules()`,
    // per task constraints: `default_rules()` is being actively edited on a
    // parallel branch fixing this exact rule, so an inline set keeps this
    // regression stable regardless of that PR's outcome. `aryl_chloride_retro`'s
    // SMIRKS is copied verbatim from `chem_env.rs`'s real rule table so the
    // shape of the bug is faithfully reproduced; the corroborating rule is a
    // synthetic stand-in for "some other, unrelated rule" (not a real
    // `chem_env.rs` rule) chosen only to make its coincidental reverse-match
    // concrete and deterministic.
    #[test]
    fn cross_rule_corroboration_does_not_upgrade_to_valid() {
        let rules = vec![
            RetroRule {
                name: "aryl_chloride_retro".to_string(),
                smirks: "[c:1][Cl]>>[c:1]".to_string(),
                ..Default::default()
            },
            RetroRule {
                name: "unrelated_bromide_to_chloride_swap".to_string(),
                smirks: "[c:1]Cl>>[c:1][Br]".to_string(),
                ..Default::default()
            },
        ];
        // Step claims aryl_chloride_retro (Ar-Cl -> Ar-H), but its precursor
        // is bromobenzene, not benzene. aryl_chloride_retro's own reversed
        // SMIRKS (add Cl to any ring carbon) applied to bromobenzene keeps
        // the existing Br and adds a second halogen, so it does NOT reproduce
        // plain chlorobenzene — the step's own claimed rule does not confirm it.
        let s = step("aryl_chloride_retro", "Clc1ccccc1", &["Brc1ccccc1"]);

        // Sanity checks that this is a meaningful regression test: the step's
        // own claimed rule genuinely fails on its own...
        assert!(
            !forward::rule_reproduces("Clc1ccccc1", &["Brc1ccccc1".to_string()], &rules[0]),
            "sanity check failed: aryl_chloride_retro's own reversal unexpectedly reproduces the target"
        );
        // ...but an unrelated rule's reversed SMIRKS coincidentally does
        // reproduce the target from the same precursor, so the old "any rule"
        // scan (smirks_reproduces) would wrongly mark this step Valid.
        assert!(
            forward::smirks_reproduces("Clc1ccccc1", &["Brc1ccccc1".to_string()], &rules),
            "sanity check failed: fixture no longer exercises a cross-rule coincidental match"
        );

        // The fix: validate_step must bind to the step's own claimed rule and
        // ignore the unrelated corroboration.
        assert_eq!(validate_step(&s, &rules), Invalid);
    }

    // ── route-level rollup still holds end-to-end with the fix ──────────────
    #[test]
    fn route_with_one_invalid_step_is_invalid_end_to_end() {
        let rules = crate::chem_env::default_rules();
        let valid_step = step(
            "friedel_crafts_acylation_retro",
            "CC(=O)c1ccccc1",
            &["c1ccccc1", "CC(=O)Cl"],
        );
        let invalid_step = step("friedel_crafts_acylation_retro", "CC(=O)c1ccccc1", &["CCO"]);
        let (_, route_status) = validate_route_steps(&[valid_step, invalid_step], &rules);
        assert_eq!(route_status, RouteValidationStatus::Invalid);
    }

    #[test]
    fn route_with_valid_and_not_evaluable_is_partially_validated_end_to_end() {
        let rules = crate::chem_env::default_rules();
        let valid_step = step(
            "friedel_crafts_acylation_retro",
            "CC(=O)c1ccccc1",
            &["c1ccccc1", "CC(=O)Cl"],
        );
        let not_evaluable_step = step("no_such_rule", "CCO", &["CC", "O"]);
        let (_, route_status) = validate_route_steps(&[valid_step, not_evaluable_step], &rules);
        assert_eq!(route_status, RouteValidationStatus::PartiallyValidated);
    }
}
