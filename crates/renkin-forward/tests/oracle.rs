//! Oracle recall/soundness corpus for `renkin-forward hints`.
//!
//! For each fixture below, a *concrete* reaction is run via
//! `enumerate_products_detailed` (real reactants + a real, explicit
//! partner) and a *hints* report is generated for the known reactant(s)
//! alone (no partners). The check: every concrete success must be
//! represented by at least one hint -- the correct known-reactant slot,
//! the missing-partner slot (if any), a `product_query_smarts` that
//! structurally matches the real concrete product, and the same source
//! template. This is a **recall** check on `hints`, not a precision
//! check: `hints` may (and does, by design) surface hints with no
//! concrete partner available -- only concrete successes must be covered,
//! not the reverse.
//!
//! Two fixtures are intentionally *not* concrete-covered, for reasons
//! documented at each site rather than silently skipped:
//! - arity-3 templates: `enumerate` doesn't apply arity>=2-missing-partner
//!   templates at all (reported unsupported), so there is no concrete
//!   success to cross-check against; this is a hints-only structural test.
//! - isotope/stereochemistry constraints: covered at the feature-extraction
//!   level by `hints.rs`'s own unit tests
//!   (`isotope_and_charge_constraints_are_retained_in_required_features`),
//!   not re-verified here through a full concrete `run_reactants` pass in
//!   this round.
//!
//! The amine fixtures use `[NH2:2]` rather than a `[N;H1,H2:2]`-style
//! multi-condition OR: an earlier draft used the latter and both amine
//! fixtures failed at the "produced at least one concrete candidate"
//! precondition -- `enumerate_products_detailed` itself rejects that
//! SMIRKS (the same `reverse_smirks_validated`/`parse_reaction` limitation
//! documented in `hints.rs`'s regression-audit tests), so there was no
//! concrete run to cross-check at all. This is not a hints defect; the
//! OR-flattening logic that `[N;H1,H2:2]` would exercise is already
//! covered directly by `hints.rs`'s own feature-extraction fixtures.

use renkin::chem_env::{RetroRule, mol_from_smiles};
use renkin_forward::hints::{
    ForwardRetrievalHintReport, HintGenerationConfig, generate_retrieval_hints,
};
use renkin_forward::{
    ForwardEnumerationConfig, ForwardEnumerationReport, PartnerRecord, enumerate_products_detailed,
};

fn rule(name: &str, smirks: &str) -> RetroRule {
    RetroRule {
        name: name.to_string(),
        template_id: format!("rule:{name}"),
        smirks: smirks.to_string(),
        weight: 1.0,
        required_elements: 0,
    }
}

fn partner_record(row_index: usize, smiles: &str) -> PartnerRecord {
    let mol = mol_from_smiles(smiles).unwrap();
    PartnerRecord {
        row_index,
        label: None,
        input_smiles: smiles.to_string(),
        canonical_smiles: chematic::smiles::canonical_smiles(&mol),
    }
}

fn hints_config() -> HintGenerationConfig {
    HintGenerationConfig {
        max_hints: 50,
        max_matches_per_slot: 50,
        max_assignments_per_template: 100,
    }
}

/// The central oracle assertion: every (template, slot) pairing that
/// contributed to a real concrete candidate must be represented by at
/// least one hint from a partner-free `hints` run on the same known
/// reactant(s).
fn assert_every_concrete_success_is_covered(
    concrete: &ForwardEnumerationReport,
    hints: &ForwardRetrievalHintReport,
) {
    assert!(
        !concrete.candidates.is_empty(),
        "fixture must produce at least one concrete candidate to cross-check"
    );
    for candidate in &concrete.candidates {
        for source in &candidate.sources {
            let covered = hints.hints.iter().any(|hint| {
                let template_matches = hint
                    .sources
                    .iter()
                    .any(|s| s.template_id == source.template_id);
                if !template_matches {
                    return false;
                }
                let slot_matches = hint
                    .known_assignments
                    .iter()
                    .any(|ka| ka.slot_index == source.slot_index);
                if !slot_matches {
                    return false;
                }
                if source.partner.is_some() && hint.missing_partners.is_empty() {
                    return false;
                }
                candidate.products.iter().any(|product_smiles| {
                    let Ok(product_mol) = mol_from_smiles(product_smiles) else {
                        return false;
                    };
                    hint.product_query_smarts.iter().any(|pattern| {
                        chematic::smarts::parse_smarts(pattern)
                            .map(|q| !chematic::smarts::find_matches(&q, &product_mol).is_empty())
                            .unwrap_or(false)
                    })
                })
            });
            assert!(
                covered,
                "concrete success (template {:?}, slot {}, products {:?}) is not \
                 represented by any hint -- classify before loosening any matcher",
                source.template_id, source.slot_index, candidate.products
            );
        }
    }
}

#[test]
fn oracle_aryl_electrophile_plus_amine() {
    let r = rule("aryl_amination", "[c:1][NH2:2]>>[c:1][Br].[NH2:2]");
    let partners = vec![partner_record(1, "NCC")];
    let concrete = enumerate_products_detailed(
        "Brc1ccccc1",
        Some(&partners),
        std::slice::from_ref(&r),
        &ForwardEnumerationConfig::default(),
    )
    .unwrap();
    let hints = generate_retrieval_hints(&["Brc1ccccc1"], &[r], &hints_config()).unwrap();
    assert_every_concrete_success_is_covered(&concrete, &hints);
}

#[test]
fn oracle_amine_plus_carbon_electrophile() {
    // Same template, known/partner roles reversed: the amine is now known,
    // the aryl bromide is the explicit partner.
    let r = rule("aryl_amination", "[c:1][NH2:2]>>[c:1][Br].[NH2:2]");
    let partners = vec![partner_record(1, "Brc1ccccc1")];
    let concrete = enumerate_products_detailed(
        "NCC",
        Some(&partners),
        std::slice::from_ref(&r),
        &ForwardEnumerationConfig::default(),
    )
    .unwrap();
    let hints = generate_retrieval_hints(&["NCC"], &[r], &hints_config()).unwrap();
    assert_every_concrete_success_is_covered(&concrete, &hints);
}

#[test]
fn oracle_c_c_bond_formation() {
    let r = rule("cc_coupling", "[C:1][C:2]>>[C:1][Br].[C:2][Br]");
    let partners = vec![partner_record(1, "CBr")];
    let concrete = enumerate_products_detailed(
        "CCBr",
        Some(&partners),
        std::slice::from_ref(&r),
        &ForwardEnumerationConfig::default(),
    )
    .unwrap();
    let hints = generate_retrieval_hints(&["CCBr"], &[r], &hints_config()).unwrap();
    assert_every_concrete_success_is_covered(&concrete, &hints);
}

#[test]
fn oracle_unary_transformation() {
    let r = rule("halide_swap", "[c:1][Cl]>>[c:1][Br]");
    let concrete = enumerate_products_detailed(
        "Brc1ccccc1",
        None,
        std::slice::from_ref(&r),
        &ForwardEnumerationConfig::default(),
    )
    .unwrap();
    let hints = generate_retrieval_hints(&["Brc1ccccc1"], &[r], &hints_config()).unwrap();
    assert_every_concrete_success_is_covered(&concrete, &hints);
    assert!(
        hints.hints[0].missing_partners.is_empty(),
        "a unary transformation must report zero missing partners"
    );
}

#[test]
fn oracle_same_known_molecule_multiple_reaction_sites() {
    // 1,4-dibromobenzene: two chemically-equivalent aromatic-Br sites.
    // enumerate/hints must each independently discover both, though
    // enumerate's own dedup collapses them into a single concrete
    // candidate (both sites produce the same product on this symmetric
    // molecule) -- the property under test is that hints' match_sites
    // reports both, not that enumerate returns two candidates.
    let r = rule("halide_swap", "[c:1][Cl]>>[c:1][Br]");
    let known = "Brc1ccc(Br)cc1";
    let concrete = enumerate_products_detailed(
        known,
        None,
        std::slice::from_ref(&r),
        &ForwardEnumerationConfig::default(),
    )
    .unwrap();
    let hints = generate_retrieval_hints(&[known], &[r], &hints_config()).unwrap();
    assert_every_concrete_success_is_covered(&concrete, &hints);
    assert_eq!(
        hints.hints[0].known_assignments[0].match_sites.len(),
        2,
        "both symmetric bromine sites must be reported as distinct match sites"
    );
}

#[test]
fn oracle_arity_3_has_no_concrete_counterpart_hints_only() {
    // enumerate never applies arity>=2-missing-partner templates (reports
    // them unsupported instead), so there is no concrete success to
    // cross-check here -- this is a structural, hints-only test verifying
    // two known reactants correctly leave the third slot missing.
    let r = rule(
        "triple_coupling",
        "[C:1][C:2][C:3]>>[C:1][Br].[C:2][Cl].[C:3][I]",
    );
    let enumerate_result = enumerate_products_detailed(
        "CBr",
        None,
        std::slice::from_ref(&r),
        &ForwardEnumerationConfig::default(),
    )
    .unwrap();
    assert_eq!(
        enumerate_result.stats.templates_unsupported_arity, 1,
        "sanity check: enumerate must still report this as unsupported, not silently apply it"
    );
    assert!(enumerate_result.candidates.is_empty());

    let hints = generate_retrieval_hints(&["CBr", "CCl"], &[r], &hints_config()).unwrap();
    assert_eq!(hints.hints.len(), 1);
    assert_eq!(hints.hints[0].known_assignments.len(), 2);
    assert_eq!(hints.hints[0].missing_partners.len(), 1);
}

#[test]
fn oracle_spectator_slot_produces_no_result_in_either_mode() {
    // A structural spectator: neither enumerate nor hints should ever
    // treat this known reactant as contributing to this template.
    let r = rule("disconnected", "[C:9]=[O:8]>>[C:1][Cl:2]");
    let concrete = enumerate_products_detailed(
        "CCl",
        None,
        std::slice::from_ref(&r),
        &ForwardEnumerationConfig::default(),
    )
    .unwrap();
    assert!(concrete.candidates.is_empty());
    assert_eq!(concrete.stats.spectator_slot_skips, 1);

    let hints = generate_retrieval_hints(&["CCl"], &[r], &hints_config()).unwrap();
    assert!(
        hints.hints.is_empty(),
        "a structural spectator must never produce a hint either"
    );
}

#[test]
fn oracle_charged_partner_constraint() {
    let r = rule("charged_amination", "[c:1][NH3+:2]>>[c:1][Br].[NH3+:2]");
    let partners = vec![partner_record(1, "C[NH3+]")];
    let concrete = enumerate_products_detailed(
        "Brc1ccccc1",
        Some(&partners),
        std::slice::from_ref(&r),
        &ForwardEnumerationConfig::default(),
    )
    .unwrap();
    let hints = generate_retrieval_hints(&["Brc1ccccc1"], &[r], &hints_config()).unwrap();
    assert_every_concrete_success_is_covered(&concrete, &hints);
    assert_eq!(
        hints.hints[0].missing_partners[0].required_features.charge,
        Some(1),
        "the charge constraint on the missing partner slot must be retained"
    );
}
