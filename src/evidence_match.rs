//! Deterministic, exact-set batch matching of external reaction records
//! against RENKIN's stable `template_id`s (for importing evidence from
//! datasets such as ORD -- see `scripts/ord_evidence_audit.py`).
//!
//! Reuses the same canonicalization (`chem_env::mol_from_smiles`/
//! `to_canonical`, via `evidence::canonicalize`/`canonical_set`) and
//! single-step retro application (`chem_env::apply_retro`) that route search
//! and `evidence::match_example` already use, so this never disagrees with
//! authoring-time or route-display matching. No partial-structure,
//! fingerprint, or stereochemistry-blind normalization is applied -- exact
//! canonical-SMILES set equality only. A SMILES that fails to parse never
//! matches (see `TemplateMatchStatus::InvalidInput`).

use crate::chem_env::{self, RetroRule};
use crate::evidence::canonical_set;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemplateMatchStatus {
    /// Exactly one loaded rule's retro-application to the target produces a
    /// precursor set equal to the input precursor set.
    Unique,
    /// Two or more distinct `template_id`s each produce a matching precursor
    /// set (duplicate rule entries sharing one `template_id` don't count
    /// twice here -- ids are deduped before counting).
    Ambiguous,
    /// No loaded rule's retro-application produces a matching precursor set.
    NoMatch,
    /// The target SMILES failed to parse, the precursor list was empty, or
    /// any precursor SMILES failed to parse. Decided before any rule is
    /// applied -- a malformed record never gets to `NoMatch`.
    InvalidInput,
}

/// Result of matching one target/precursor pair against a set of
/// `RetroRule`s. `target_smiles`/`precursor_smiles` hold the canonical form
/// on success; on `InvalidInput` they echo the original (uncanonicalizable)
/// input unchanged.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateMatchResult {
    pub target_smiles: String,
    pub precursor_smiles: Vec<String>,
    /// Sorted, deduped `template_id`s of every rule whose retro-application
    /// exactly reproduces the input precursor set.
    pub matching_template_ids: Vec<String>,
    pub status: TemplateMatchStatus,
}

/// Matches one external reaction record against `rules`.
///
/// Matching: parse+canonicalize `target_smiles`; for each rule, apply it to
/// the target (`chem_env::apply_retro`, the same single-step retro
/// application route search uses -- one rule can produce several distinct
/// precursor sets at different sites, and the rule counts as matching if
/// *any* site's set equals the input); canonicalize+sort+dedup both the
/// site's precursor set and the input precursor set; compare for equality.
/// Matching `template_id`s are deduped and sorted before the status is
/// decided from how many distinct ids matched.
pub fn match_reaction_to_templates(
    target_smiles: &str,
    precursor_smiles: &[String],
    rules: &[RetroRule],
) -> TemplateMatchResult {
    let invalid = |target_smiles: String| TemplateMatchResult {
        target_smiles,
        precursor_smiles: precursor_smiles.to_vec(),
        matching_template_ids: Vec::new(),
        status: TemplateMatchStatus::InvalidInput,
    };

    if precursor_smiles.is_empty() {
        return invalid(target_smiles.to_string());
    }

    let target_mol = match chem_env::mol_from_smiles(target_smiles) {
        Ok(m) => m,
        Err(_) => return invalid(target_smiles.to_string()),
    };
    let canonical_target = chem_env::to_canonical(&target_mol);

    let input_precursor_set = match canonical_set(precursor_smiles) {
        Some(set) => set,
        None => return invalid(canonical_target),
    };

    let mut matching_template_ids: Vec<String> = rules
        .iter()
        .filter(|rule| {
            chem_env::apply_retro(&target_mol, rule)
                .into_iter()
                .any(|site| {
                    let mut set: Vec<String> = site.into_iter().map(|p| p.smiles).collect();
                    set.sort();
                    set.dedup();
                    set == input_precursor_set
                })
        })
        .map(|rule| rule.template_id.clone())
        .collect();
    matching_template_ids.sort();
    matching_template_ids.dedup();

    let status = match matching_template_ids.len() {
        0 => TemplateMatchStatus::NoMatch,
        1 => TemplateMatchStatus::Unique,
        _ => TemplateMatchStatus::Ambiguous,
    };

    TemplateMatchResult {
        target_smiles: canonical_target,
        precursor_smiles: input_precursor_set,
        matching_template_ids,
        status,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chem_env;

    fn rules() -> Vec<RetroRule> {
        chem_env::default_rules()
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn unique_match_regardless_of_precursor_order() {
        let a = match_reaction_to_templates("CC(=O)OCC", &s(&["CC(=O)O", "CCO"]), &rules());
        let b = match_reaction_to_templates("CC(=O)OCC", &s(&["CCO", "CC(=O)O"]), &rules());
        assert_eq!(a.status, TemplateMatchStatus::Unique);
        assert_eq!(a.status, b.status);
        assert_eq!(a.matching_template_ids, b.matching_template_ids);
        assert_eq!(a.matching_template_ids, vec!["rule:ester_cleavage"]);
    }

    #[test]
    fn different_target_gives_no_match() {
        // A target this matcher's rule set has no disconnection for at all.
        let result = match_reaction_to_templates("CCCCCC", &s(&["CCCCCC"]), &rules());
        assert_eq!(result.status, TemplateMatchStatus::NoMatch);
        assert!(result.matching_template_ids.is_empty());
    }

    #[test]
    fn different_precursors_give_no_match() {
        // Real target/rule pairing (ester_cleavage applies), but the supplied
        // precursor set doesn't match what the rule actually produces.
        let result =
            match_reaction_to_templates("CC(=O)OCC", &s(&["CC(=O)O", "CCCCCCCC"]), &rules());
        assert_eq!(result.status, TemplateMatchStatus::NoMatch);
    }

    #[test]
    fn ambiguous_when_multiple_rules_produce_same_precursor_set() {
        let mut two_identical = rules();
        let ester = two_identical
            .iter()
            .find(|r| r.template_id == "rule:ester_cleavage")
            .cloned()
            .expect("ester_cleavage present in default rules");
        let mut renamed = ester.clone();
        renamed.template_id = "rule:ester_cleavage_dup".to_string();
        two_identical.push(renamed);

        let result =
            match_reaction_to_templates("CC(=O)OCC", &s(&["CC(=O)O", "CCO"]), &two_identical);
        assert_eq!(result.status, TemplateMatchStatus::Ambiguous);
        assert_eq!(
            result.matching_template_ids,
            vec!["rule:ester_cleavage", "rule:ester_cleavage_dup"]
        );
    }

    #[test]
    fn duplicate_rule_entries_with_same_template_id_do_not_fake_ambiguity() {
        let mut doubled = rules();
        let ester = doubled
            .iter()
            .find(|r| r.template_id == "rule:ester_cleavage")
            .cloned()
            .expect("ester_cleavage present in default rules");
        doubled.push(ester); // same template_id pushed a second time

        let result = match_reaction_to_templates("CC(=O)OCC", &s(&["CC(=O)O", "CCO"]), &doubled);
        assert_eq!(result.status, TemplateMatchStatus::Unique);
        assert_eq!(result.matching_template_ids, vec!["rule:ester_cleavage"]);
    }

    #[test]
    fn malformed_target_is_invalid_input() {
        let result = match_reaction_to_templates("not(a smiles", &s(&["CCO"]), &rules());
        assert_eq!(result.status, TemplateMatchStatus::InvalidInput);
        assert!(result.matching_template_ids.is_empty());
        // Original (uncanonicalizable) input is echoed back, not fabricated.
        assert_eq!(result.target_smiles, "not(a smiles");
    }

    #[test]
    fn malformed_precursor_is_invalid_input() {
        let result =
            match_reaction_to_templates("CC(=O)OCC", &s(&["CC(=O)O", "not(a smiles"]), &rules());
        assert_eq!(result.status, TemplateMatchStatus::InvalidInput);
        assert!(result.matching_template_ids.is_empty());
    }

    #[test]
    fn empty_precursor_list_is_invalid_input() {
        let result = match_reaction_to_templates("CC(=O)OCC", &[], &rules());
        assert_eq!(result.status, TemplateMatchStatus::InvalidInput);
    }

    #[test]
    fn matching_template_ids_are_sorted() {
        let mut two_identical = rules();
        let ester = two_identical
            .iter()
            .find(|r| r.template_id == "rule:ester_cleavage")
            .cloned()
            .expect("ester_cleavage present in default rules");
        // Pushed *after* ester_cleavage in the rule list, but alphabetically
        // first -- if the matcher returned ids in match-order instead of
        // sorting, this would come back as [ester_cleavage, aaa_before_ester].
        let mut renamed = ester;
        renamed.template_id = "rule:aaa_before_ester".to_string();
        two_identical.push(renamed);

        let result =
            match_reaction_to_templates("CC(=O)OCC", &s(&["CC(=O)O", "CCO"]), &two_identical);
        assert_eq!(
            result.matching_template_ids,
            vec!["rule:aaa_before_ester", "rule:ester_cleavage"]
        );
    }

    #[test]
    fn stable_template_id_preserved_for_hand_crafted_and_extracted_rules() {
        // Hand-crafted rules use the "rule:<name>" id; extracted (SMIRKS-file)
        // rules use "smirks-sha256:<hex>" (chem_env::template_id_for_smirks).
        // The matcher must pass through whichever id the rule already carries
        // -- it never re-derives or renames a template_id. Reuses the
        // existing, already-verified friedel_crafts_acylation_retro SMIRKS
        // (see chem_env's friedel_crafts_retro_on_acetophenone test) under a
        // second, extracted-style id, so this test isn't betting on a
        // hand-rolled SMIRKS pattern actually firing.
        let friedel_crafts = rules()
            .into_iter()
            .find(|r| r.template_id == "rule:friedel_crafts_acylation_retro")
            .expect("friedel_crafts_acylation_retro present in default rules");
        let extracted = RetroRule {
            name: friedel_crafts.name.clone(),
            template_id: chem_env::template_id_for_smirks(&friedel_crafts.smirks),
            smirks: friedel_crafts.smirks.clone(),
            weight: 1.0,
            required_elements: friedel_crafts.required_elements,
        };
        let mut rule_set = vec![extracted.clone()];
        rule_set.extend(rules());

        let result =
            match_reaction_to_templates("CC(=O)c1ccccc1", &s(&["c1ccccc1", "CC(=O)Cl"]), &rule_set);
        assert_eq!(result.status, TemplateMatchStatus::Ambiguous);
        assert!(
            result
                .matching_template_ids
                .contains(&extracted.template_id)
        );
        assert!(
            result
                .matching_template_ids
                .contains(&"rule:friedel_crafts_acylation_retro".to_string())
        );
    }
}
