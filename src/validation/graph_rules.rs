#![forbid(unsafe_code)]

//! Structural validators for RENKIN's 7 graph-based retro rules.
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
//! A rule name with no case here (a future 8th graph rule, for instance)
//! falls through to `NotEvaluable` rather than silently passing — there is
//! deliberately no catch-all "trust it" arm.

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

/// Dispatch to the structural validator for one of the 7 graph-based rules.
/// Rule names not covered here return `NotEvaluable` — never a silent `Valid`.
pub fn validate_graph_step(
    rule_name: &str,
    target: &str,
    precursors: &[String],
) -> StepValidationStatus {
    match rule_name {
        "ester_cleavage" | "amide_cleavage" => {
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
