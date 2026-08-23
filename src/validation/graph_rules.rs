#![forbid(unsafe_code)]

//! Structural validators for RENKIN's 8 graph-based retro rules.
//!
//! These rules cut a bond directly in the target's molecular graph (see
//! `apply_retro` in `chem_env.rs`) instead of matching a SMIRKS pattern, so
//! they can never be confirmed by SMIRKS-reversal forward validation
//! (`forward::smirks_reproduces` always returns `false` for their steps).
//! Rather than whitelist these rules as unconditionally `Valid` — which would
//! hide a real regression in the graph-cutting code — each rule is checked
//! against the exact atom-count delta its chemistry implies: e.g. ester
//! cleavage is a hydrolysis, so `Σ(precursor atoms) - target atoms` must be
//! exactly one H2O (2 H, 1 O), no more, no less. Deltas below were derived
//! from each rule's reaction equation and confirmed against `renkin --format
//! json` output for a concrete example (see PR test fixtures).
//!
//! A rule name with no case here falls through to `NotEvaluable` rather than
//! silently passing — there is deliberately no catch-all "trust it" arm. This
//! was a real, silent gap for a while: `aryl_ether_retro` became this
//! module's 8th graph-based rule when PR #171 converted it from a
//! SMIRKS-based rule (fixing a mislabeling bug -- see
//! `docs/design/retro-rule-precision-gaps-v0.md` #1) without a matching case
//! being added here, so every `aryl_ether_retro` step silently fell through
//! to `NotEvaluable` in this validator (used by `examples/inspect_validation`,
//! `src/bin/benchmark.rs`, and `synthesizability::assessment`) even though
//! the rule's own chemistry is straightforward to check -- caught and fixed
//! in a later pass, not part of PR #171 itself.

use std::collections::BTreeMap;

use chematic::core::Element;

use crate::chem_env::mol_from_smiles;

use super::StepValidationStatus;

/// Element-count delta (Σ precursors − target), as (Element, signed count) pairs.
/// Negative counts mean the target has MORE of that element than the precursors
/// (e.g. Boc/Cbz deprotection: the target carries the protecting group, the
/// single tracked precursor is the smaller deprotected amine).
type ElementDelta = &'static [(Element, i64)];

const ESTER_AMIDE_DELTA: ElementDelta = &[(Element::H, 2), (Element::O, 1)]; // + H2O
const SUZUKI_DELTA: ElementDelta = &[(Element::H, 1), (Element::BR, 1)]; // + HBr
const SULFONYL_DELTA: ElementDelta = &[(Element::H, 1), (Element::CL, 1)]; // + HCl
const BOC_DELTA: ElementDelta = &[(Element::C, -5), (Element::H, -8), (Element::O, -2)]; // - C5H8O2
const CBZ_DELTA: ElementDelta = &[(Element::C, -8), (Element::H, -6), (Element::O, -2)]; // - C8H6O2

/// Sum element counts (heavy atoms + implicit H) across one or more SMILES.
fn element_counts(smiles: &[&str]) -> Option<BTreeMap<Element, i64>> {
    let mut counts: BTreeMap<Element, i64> = BTreeMap::new();
    for s in smiles {
        let mol = mol_from_smiles(s).ok()?;
        for (_, atom) in mol.atoms() {
            *counts.entry(atom.element).or_insert(0) += 1;
        }
        for h in chematic::chem::implicit_hcount_per_atom(&mol) {
            if h > 0 {
                *counts.entry(Element::H).or_insert(0) += h as i64;
            }
        }
    }
    Some(counts)
}

/// True if `precursor_counts - target_counts == delta` exactly (elements absent
/// from either side are treated as zero).
fn delta_matches(
    target_counts: &BTreeMap<Element, i64>,
    precursor_counts: &BTreeMap<Element, i64>,
    delta: ElementDelta,
) -> bool {
    let mut all_elements: std::collections::BTreeSet<Element> = target_counts
        .keys()
        .chain(precursor_counts.keys())
        .copied()
        .collect();
    all_elements.extend(delta.iter().map(|(e, _)| *e));

    all_elements.iter().all(|e| {
        let t = *target_counts.get(e).unwrap_or(&0);
        let p = *precursor_counts.get(e).unwrap_or(&0);
        let expected = delta.iter().find(|(de, _)| de == e).map_or(0, |(_, d)| *d);
        p - t == expected
    })
}

fn validate_delta(
    target: &str,
    precursors: &[String],
    delta: ElementDelta,
) -> StepValidationStatus {
    let precursor_refs: Vec<&str> = precursors.iter().map(String::as_str).collect();
    let (Some(target_counts), Some(precursor_counts)) =
        (element_counts(&[target]), element_counts(&precursor_refs))
    else {
        return StepValidationStatus::NotEvaluable;
    };
    if delta_matches(&target_counts, &precursor_counts, delta) {
        StepValidationStatus::Valid
    } else {
        StepValidationStatus::Invalid
    }
}

/// Dispatch to the structural validator for one of the 8 graph-based rules.
/// Rule names not covered here return `NotEvaluable` — never a silent `Valid`.
pub fn validate_graph_step(
    rule_name: &str,
    target: &str,
    precursors: &[String],
) -> StepValidationStatus {
    match rule_name {
        // aryl_ether_retro: Ar-O-R -> Ar-OH + R-OH -- the O atom the target
        // already has stays with the R fragment (picks up one new H to fill
        // its valence), and the aromatic fragment gains a brand-new O-H.
        // Net delta: +1 O, +2 H -- formally the same hydrolysis-shaped delta
        // as ester/amide cleavage, confirmed by direct atom counting against
        // `aryl_ether_cleavage` in chem_env.rs (a diaryl ether like
        // `c1ccccc1Oc1ccccc1` -> two phenols is +2H+1O end to end).
        "ester_cleavage" | "amide_cleavage" | "aryl_ether_retro" => {
            validate_delta(target, precursors, ESTER_AMIDE_DELTA)
        }
        "suzuki_retro" => validate_delta(target, precursors, SUZUKI_DELTA),
        "sulfonamide_retro" | "diaryl_sulfone_retro" => {
            validate_delta(target, precursors, SULFONYL_DELTA)
        }
        "boc_deprotection_retro" => validate_delta(target, precursors, BOC_DELTA),
        "cbz_deprotection_retro" => validate_delta(target, precursors, CBZ_DELTA),
        _ => StepValidationStatus::NotEvaluable,
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn precs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    // ── ester_cleavage ──────────────────────────────────────────────────────
    #[test]
    fn ester_cleavage_valid() {
        // phenyl acetate → acetic acid + phenol
        let status = validate_graph_step(
            "ester_cleavage",
            "CC(=O)Oc1ccccc1",
            &precs(&["CC(=O)O", "Oc1ccccc1"]),
        );
        assert_eq!(status, StepValidationStatus::Valid);
    }

    #[test]
    fn ester_cleavage_invalid_wrong_precursors() {
        // precursors don't correspond to the target at all
        let status = validate_graph_step("ester_cleavage", "CC(=O)Oc1ccccc1", &precs(&["CCO"]));
        assert_eq!(status, StepValidationStatus::Invalid);
    }

    // ── amide_cleavage ──────────────────────────────────────────────────────
    #[test]
    fn amide_cleavage_valid() {
        // acetanilide → acetic acid + aniline
        let status = validate_graph_step(
            "amide_cleavage",
            "CC(=O)Nc1ccccc1",
            &precs(&["CC(=O)O", "Nc1ccccc1"]),
        );
        assert_eq!(status, StepValidationStatus::Valid);
    }

    #[test]
    fn amide_cleavage_invalid_missing_precursor() {
        let status = validate_graph_step("amide_cleavage", "CC(=O)Nc1ccccc1", &precs(&["CC(=O)O"]));
        assert_eq!(status, StepValidationStatus::Invalid);
    }

    // ── aryl_ether_retro ─────────────────────────────────────────────────────
    #[test]
    fn aryl_ether_retro_valid() {
        // diphenyl ether -> phenol + phenol
        let status = validate_graph_step(
            "aryl_ether_retro",
            "c1ccc(Oc2ccccc2)cc1",
            &precs(&["Oc1ccccc1", "Oc1ccccc1"]),
        );
        assert_eq!(status, StepValidationStatus::Valid);
    }

    #[test]
    fn aryl_ether_retro_invalid_missing_precursor() {
        let status = validate_graph_step(
            "aryl_ether_retro",
            "c1ccc(Oc2ccccc2)cc1",
            &precs(&["Oc1ccccc1"]),
        );
        assert_eq!(status, StepValidationStatus::Invalid);
    }

    // ── suzuki_retro ─────────────────────────────────────────────────────────
    #[test]
    fn suzuki_retro_valid() {
        // biphenyl → bromobenzene + benzene
        let status = validate_graph_step(
            "suzuki_retro",
            "c1ccc(-c2ccccc2)cc1",
            &precs(&["Brc1ccccc1", "c1ccccc1"]),
        );
        assert_eq!(status, StepValidationStatus::Valid);
    }

    #[test]
    fn suzuki_retro_invalid_wrong_halide() {
        // chlorobenzene instead of bromobenzene — wrong leaving-group element
        let status = validate_graph_step(
            "suzuki_retro",
            "c1ccc(-c2ccccc2)cc1",
            &precs(&["Clc1ccccc1", "c1ccccc1"]),
        );
        assert_eq!(status, StepValidationStatus::Invalid);
    }

    // ── sulfonamide_retro ────────────────────────────────────────────────────
    #[test]
    fn sulfonamide_retro_valid() {
        // PhSO2NHPh → PhSO2Cl + aniline
        let status = validate_graph_step(
            "sulfonamide_retro",
            "O=S(=O)(Nc1ccccc1)c1ccccc1",
            &precs(&["O=S(=O)(Cl)c1ccccc1", "Nc1ccccc1"]),
        );
        assert_eq!(status, StepValidationStatus::Valid);
    }

    #[test]
    fn sulfonamide_retro_invalid() {
        let status = validate_graph_step(
            "sulfonamide_retro",
            "O=S(=O)(Nc1ccccc1)c1ccccc1",
            &precs(&["Nc1ccccc1"]),
        );
        assert_eq!(status, StepValidationStatus::Invalid);
    }

    // ── diaryl_sulfone_retro ─────────────────────────────────────────────────
    #[test]
    fn diaryl_sulfone_retro_valid() {
        // PhSO2Ph → benzene + PhSO2Cl
        let status = validate_graph_step(
            "diaryl_sulfone_retro",
            "O=S(=O)(c1ccccc1)c1ccccc1",
            &precs(&["c1ccccc1", "O=S(=O)(Cl)c1ccccc1"]),
        );
        assert_eq!(status, StepValidationStatus::Valid);
    }

    #[test]
    fn diaryl_sulfone_retro_invalid() {
        let status = validate_graph_step(
            "diaryl_sulfone_retro",
            "O=S(=O)(c1ccccc1)c1ccccc1",
            &precs(&["c1ccccc1"]),
        );
        assert_eq!(status, StepValidationStatus::Invalid);
    }

    // ── boc_deprotection_retro ───────────────────────────────────────────────
    #[test]
    fn boc_deprotection_valid() {
        // N-Boc-piperidine → piperidine
        let status = validate_graph_step(
            "boc_deprotection_retro",
            "CC(C)(C)OC(=O)N1CCCCC1",
            &precs(&["C1CCNCC1"]),
        );
        assert_eq!(status, StepValidationStatus::Valid);
    }

    #[test]
    fn boc_deprotection_invalid_wrong_amine() {
        // precursor amine doesn't match the target's ring size
        let status = validate_graph_step(
            "boc_deprotection_retro",
            "CC(C)(C)OC(=O)N1CCCCC1",
            &precs(&["C1CCNC1"]),
        );
        assert_eq!(status, StepValidationStatus::Invalid);
    }

    // ── cbz_deprotection_retro ───────────────────────────────────────────────
    #[test]
    fn cbz_deprotection_valid() {
        // N-Cbz-piperidine → piperidine
        let status = validate_graph_step(
            "cbz_deprotection_retro",
            "O=C(OCc1ccccc1)N1CCCCC1",
            &precs(&["C1CCNCC1"]),
        );
        assert_eq!(status, StepValidationStatus::Valid);
    }

    #[test]
    fn cbz_deprotection_invalid_wrong_amine() {
        let status = validate_graph_step(
            "cbz_deprotection_retro",
            "O=C(OCc1ccccc1)N1CCCCC1",
            &precs(&["C1CCNC1"]),
        );
        assert_eq!(status, StepValidationStatus::Invalid);
    }

    // ── uncovered rule name ──────────────────────────────────────────────────
    #[test]
    fn unknown_graph_rule_not_evaluable() {
        let status = validate_graph_step("some_future_graph_rule", "C", &precs(&["C"]));
        assert_eq!(status, StepValidationStatus::NotEvaluable);
    }

    #[test]
    fn unparseable_smiles_not_evaluable() {
        let status = validate_graph_step("ester_cleavage", "", &precs(&["CCO"]));
        assert_eq!(status, StepValidationStatus::NotEvaluable);
    }

    // ── closed-set coverage: catches exactly the class of bug this module's
    // own doc comment warns about (a graph-based rule silently falling
    // through to the NotEvaluable catch-all because no case was added here
    // when it was introduced elsewhere) -- this is precisely what happened
    // to aryl_ether_retro after PR #171 converted it from SMIRKS-based to
    // graph-based. Every rule with an empty `smirks` in `default_rules()`
    // must have a real (non-catch-all) case in `validate_graph_step`.
    #[test]
    fn every_graph_based_default_rule_has_a_validate_graph_step_case() {
        let covered: std::collections::BTreeSet<&str> = [
            "ester_cleavage",
            "amide_cleavage",
            "aryl_ether_retro",
            "suzuki_retro",
            "sulfonamide_retro",
            "diaryl_sulfone_retro",
            "boc_deprotection_retro",
            "cbz_deprotection_retro",
        ]
        .into_iter()
        .collect();
        let graph_based: std::collections::BTreeSet<String> = crate::chem_env::default_rules()
            .iter()
            .filter(|r| r.smirks.is_empty())
            .map(|r| r.name.clone())
            .collect();
        let covered_owned: std::collections::BTreeSet<String> =
            covered.iter().map(|s| s.to_string()).collect();
        assert_eq!(
            graph_based, covered_owned,
            "default_rules()'s graph-based rule set and validate_graph_step's \
             covered names have drifted apart -- a rule on either side with no \
             match on the other silently degrades to NotEvaluable everywhere \
             this validator is used (examples/inspect_validation, \
             src/bin/benchmark.rs, synthesizability::assessment)"
        );
    }
}
