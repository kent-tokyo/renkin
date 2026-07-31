//! Deterministic hashing/provenance helpers for the Synthesizability Kernel
//! (`docs/design/synthesizability-kernel-v0.md` §5, §6). Every function
//! here is a pure function over already-in-memory data: no I/O, no
//! wall-clock reads, nothing that needs `#[cfg(not(target_arch =
//! "wasm32"))]` gating (design doc §5.1).
//!
//! Hashing convention (matches `ChemEnv::content_sha256` in `chem_env.rs`
//! and `renkin-forward`'s `candidate_id_for`/`hash_string_sequence`): a
//! fixed domain-separator byte string unique to each hash's preimage shape
//! (so two different hash schemes can never collide with each other), an
//! explicit element count, and length-prefixed bytes for every
//! variable-length field -- never a plain `.join(...)`, since a canonical
//! SMILES can itself contain any given join separator (e.g. `.` for a
//! disconnected salt).

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::chem_env::{self, RetroRule};
use crate::search::Route;
use crate::synthesizability::schema::{RouteAssessment, SynthesizabilityConfig};

// ---------------------------------------------------------------------
// Shared low-level hashing helpers
// ---------------------------------------------------------------------

fn hash_str(hasher: &mut Sha256, s: &str) {
    let bytes = s.as_bytes();
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

fn hash_string_seq<S: AsRef<str>>(hasher: &mut Sha256, values: &[S]) {
    hasher.update((values.len() as u64).to_be_bytes());
    for v in values {
        hash_str(hasher, v.as_ref());
    }
}

fn hash_bool(hasher: &mut Sha256, b: bool) {
    hasher.update([b as u8]);
}

fn hash_usize(hasher: &mut Sha256, n: usize) {
    hasher.update((n as u64).to_be_bytes());
}

/// Canonical, stable string tag for any `Serialize` value governed by
/// `#[serde(rename_all = "snake_case")]` (every enum in `schema.rs`) --
/// reused for hashing so the hash's notion of "which variant is this" can
/// never drift from the JSON the caller actually sees. Every such enum is a
/// plain derived `Serialize` with no fallible custom logic, so this never
/// realistically fails; `unwrap_or_default()` keeps the function itself
/// infallible (an empty tag would just be less debuggable, never a panic)
/// rather than threading a `Result` through every hash helper for a case
/// that isn't reachable today.
fn tag_of<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

// ---------------------------------------------------------------------
// Route-structure canonicalization (shared with assessment.rs)
// ---------------------------------------------------------------------

/// Best-effort re-canonicalization of a route-step SMILES string, via the
/// kernel's own pipeline (`chem_env::mol_from_smiles` +
/// `chem_env::to_canonical`) -- the same "independent re-verification"
/// philosophy design doc §4.2 applies to stock identity, applied here to
/// route-step SMILES too, rather than trusting `search::Route`'s strings
/// are already canonical. Returns `None` on parse failure; `assessment.rs`
/// uses that to populate `HardFailure::RouteStructureUnparseable`.
///
/// Like the stock-identity check (design doc §4.2, §11 limitation 3),
/// `route_id`'s stability across two otherwise-identical runs rests on
/// `chematic::smiles::canonical_smiles` (via `chem_env::to_canonical`)
/// being construction-path-invariant -- documented as true of chematic
/// ≥0.8.1 in `CHANGELOG.md`, a pinned assumption this module does not
/// independently re-verify.
pub(crate) fn try_canonicalize(smiles: &str) -> Option<String> {
    chem_env::mol_from_smiles(smiles)
        .ok()
        .map(|mol| chem_env::to_canonical(&mol))
}

/// `try_canonicalize`, falling back to the raw input string on parse
/// failure so hash computation (`compute_route_id`) is always total --
/// the parse failure itself is recorded separately, as a hard failure, by
/// `assessment.rs`'s own (re-)parse pass, not by making this function
/// fallible.
fn canonicalize_or_raw(smiles: &str) -> String {
    try_canonicalize(smiles).unwrap_or_else(|| smiles.to_string())
}

// ---------------------------------------------------------------------
// compute_rules_hash
// ---------------------------------------------------------------------

/// sha256 over every rule's `(template_id, smirks)` pair, sorted first so
/// the result is independent of rule-file/load order (design doc §5: "same
/// style as `ChemEnv::content_sha256`" -- sorted-then-hashed content, not a
/// caller-supplied label).
pub(crate) fn compute_rules_hash(rules: &[RetroRule]) -> String {
    let mut pairs: Vec<(&str, &str)> = rules
        .iter()
        .map(|r| (r.template_id.as_str(), r.smirks.as_str()))
        .collect();
    pairs.sort_unstable();

    let mut hasher = Sha256::new();
    hasher.update(b"renkin-synthesizability-rules-v1\0");
    hash_usize(&mut hasher, pairs.len());
    for (template_id, smirks) in pairs {
        hash_str(&mut hasher, template_id);
        hash_str(&mut hasher, smirks);
    }
    format!("sha256:{:x}", hasher.finalize())
}

// ---------------------------------------------------------------------
// compute_assessment_config_hash
// ---------------------------------------------------------------------

/// sha256 over every `SynthesizabilityConfig` field, deterministic
/// regardless of `reagent_omission_template_allowlist`'s input order (it is
/// sorted before hashing).
pub(crate) fn compute_assessment_config_hash(config: &SynthesizabilityConfig) -> String {
    let mut allowlist: Vec<&str> = config
        .reagent_omission_template_allowlist
        .iter()
        .map(String::as_str)
        .collect();
    allowlist.sort_unstable();

    let mut hasher = Sha256::new();
    hasher.update(b"renkin-synthesizability-config-v1\0");
    hash_bool(&mut hasher, config.require_verified_stock_terminal);
    hash_bool(&mut hasher, config.require_target_element_accounting);
    hash_str(&mut hasher, &tag_of(&config.forward_validation_policy));
    hash_str(&mut hasher, &tag_of(&config.evidence_policy));
    hash_string_seq(&mut hasher, &allowlist);
    hash_str(&mut hasher, &tag_of(&config.accounting_failure_policy));
    hash_usize(&mut hasher, config.max_routes_to_assess);
    hash_bool(&mut hasher, config.include_all_route_diagnostics);
    format!("sha256:{:x}", hasher.finalize())
}

// ---------------------------------------------------------------------
// compute_route_id
// ---------------------------------------------------------------------

/// Deterministic route identifier (design doc §6): `sha256:<hex>` over a
/// fixed domain separator in its own namespace (distinct from
/// `renkin-forward`'s `renkin-forward-candidate-v1`/
/// `renkin-forward-enumeration-candidate-v1`), the canonical target, and
/// the sorted, canonicalized `(template_id, target, precursors)` tuple for
/// every step. Sorting both each step's precursor list and the overall
/// step-tuple list makes the result independent of `Route.steps`'s
/// discovery/collection order and of precursor list order within a step.
pub(crate) fn compute_route_id(canonical_target: &str, route: &Route) -> String {
    let mut step_tuples: Vec<(String, String, Vec<String>)> = route
        .steps
        .iter()
        .map(|step| {
            let target = canonicalize_or_raw(&step.target);
            let mut precursors: Vec<String> = step
                .precursors
                .iter()
                .map(|p| canonicalize_or_raw(p))
                .collect();
            precursors.sort_unstable();
            (step.template_id.clone(), target, precursors)
        })
        .collect();
    step_tuples.sort_unstable();

    let mut hasher = Sha256::new();
    hasher.update(b"renkin-synthesizability-route-v1\0");
    hash_str(&mut hasher, canonical_target);
    hash_usize(&mut hasher, step_tuples.len());
    for (template_id, target, precursors) in &step_tuples {
        hash_str(&mut hasher, template_id);
        hash_str(&mut hasher, target);
        hash_string_seq(&mut hasher, precursors);
    }
    format!("sha256:{:x}", hasher.finalize())
}

// ---------------------------------------------------------------------
// compute_reproducibility_hash
// ---------------------------------------------------------------------

/// sha256 combining `rules_hash`, `stock_hash`, `assessment_config_hash`,
/// `canonical_target`, and every route's `route_id` plus its per-route
/// status/failure signals (design doc §6).
///
/// `RouteAssessment` carries no separate top-level "status" field -- the
/// design doc's route-level `AssessmentStatus` contribution is itself a
/// pure function of `hard_failures`/`validation_gaps` (empty/non-empty),
/// which are already in this preimage. What's hashed here instead, in
/// their place, are the three per-route status enums that *are* on
/// `RouteAssessment` (`stock_termination_status`,
/// `target_element_accounting_status`, `forward_validation_status`) --
/// nothing that determines reproducibility is lost by this substitution.
///
/// Hashed in `route_assessments`'s *stored* (slice) order -- the caller
/// (`assessment::assess_routes`) must pass routes already in their final
/// design-doc-§4.8-sorted order, since re-sorting only for hashing here
/// would silently decouple the hash from the actual serialized output
/// (defeating §6's "byte-identical output" property).
///
/// Never includes timing or wall-clock fields: nothing time-based is
/// computed anywhere in this module (see
/// `AssessmentProvenance::reproducibility_exclusions`).
pub(crate) fn compute_reproducibility_hash(
    rules_hash: &str,
    stock_hash: &str,
    assessment_config_hash: &str,
    canonical_target: &str,
    route_assessments: &[RouteAssessment],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"renkin-synthesizability-repro-v1\0");
    hash_str(&mut hasher, rules_hash);
    hash_str(&mut hasher, stock_hash);
    hash_str(&mut hasher, assessment_config_hash);
    hash_str(&mut hasher, canonical_target);
    hash_usize(&mut hasher, route_assessments.len());
    for ra in route_assessments {
        hash_str(&mut hasher, &ra.route_id);
        hash_str(&mut hasher, &tag_of(&ra.stock_termination_status));
        hash_str(&mut hasher, &tag_of(&ra.target_element_accounting_status));
        hash_str(&mut hasher, &tag_of(&ra.forward_validation_status));
        hash_usize(&mut hasher, ra.hard_failures.len());
        for hf in &ra.hard_failures {
            hash_str(&mut hasher, &tag_of(hf));
        }
        hash_usize(&mut hasher, ra.validation_gaps.len());
        for vg in &ra.validation_gaps {
            hash_str(&mut hasher, &tag_of(vg));
        }
    }
    format!("sha256:{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::ReactionStep;
    use crate::synthesizability::schema::{
        ElementAccountingStatus, EvidenceCoverage, ForwardValidationStatus, HardFailure,
        StockTerminationStatus,
    };

    fn rule(template_id: &str, smirks: &str) -> RetroRule {
        RetroRule {
            name: template_id.to_string(),
            template_id: template_id.to_string(),
            smirks: smirks.to_string(),
            weight: 1.0,
            required_elements: 0,
        }
    }

    #[test]
    fn rules_hash_is_independent_of_input_order() {
        let a = vec![rule("rule:a", "A>>B"), rule("rule:b", "C>>D")];
        let b = vec![rule("rule:b", "C>>D"), rule("rule:a", "A>>B")];
        assert_eq!(compute_rules_hash(&a), compute_rules_hash(&b));
    }

    #[test]
    fn rules_hash_changes_when_smirks_changes() {
        let a = vec![rule("rule:a", "A>>B")];
        let b = vec![rule("rule:a", "A>>C")];
        assert_ne!(compute_rules_hash(&a), compute_rules_hash(&b));
    }

    #[test]
    fn rules_hash_is_not_confused_by_join_ambiguity() {
        // ["ab", "c"] vs ["a", "bc"] must not collide despite naive
        // concatenation producing the same "abc" either way.
        let a = vec![rule("ab", "c")];
        let b = vec![rule("a", "bc")];
        assert_ne!(compute_rules_hash(&a), compute_rules_hash(&b));
    }

    fn base_config() -> SynthesizabilityConfig {
        SynthesizabilityConfig::conservative()
    }

    #[test]
    fn config_hash_is_independent_of_allowlist_order() {
        let mut a = base_config();
        a.reagent_omission_template_allowlist = vec!["x".to_string(), "y".to_string()];
        let mut b = base_config();
        b.reagent_omission_template_allowlist = vec!["y".to_string(), "x".to_string()];
        assert_eq!(
            compute_assessment_config_hash(&a),
            compute_assessment_config_hash(&b)
        );
    }

    #[test]
    fn config_hash_differs_between_conservative_and_diagnostic() {
        let conservative = SynthesizabilityConfig::conservative();
        let diagnostic = SynthesizabilityConfig::diagnostic();
        assert_ne!(
            compute_assessment_config_hash(&conservative),
            compute_assessment_config_hash(&diagnostic)
        );
    }

    #[test]
    fn config_hash_changes_with_max_routes_to_assess() {
        let mut a = base_config();
        a.max_routes_to_assess = 5;
        let mut b = base_config();
        b.max_routes_to_assess = 6;
        assert_ne!(
            compute_assessment_config_hash(&a),
            compute_assessment_config_hash(&b)
        );
    }

    fn step(template_id: &str, target: &str, precursors: &[&str]) -> ReactionStep {
        ReactionStep {
            rule: template_id.to_string(),
            template_id: template_id.to_string(),
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

    fn route(steps: Vec<ReactionStep>) -> Route {
        Route {
            depth: steps.len() as u32,
            steps,
            score: 0.0,
            building_blocks: Vec::new(),
            confidence: 1.0,
            convergency: 1.0,
            success_probability: 1.0,
            route_cost: 1.0,
        }
    }

    #[test]
    fn route_id_is_independent_of_precursor_order_within_a_step() {
        let r_a = route(vec![step("rule:x", "CCO", &["CC=O", "O"])]);
        let r_b = route(vec![step("rule:x", "CCO", &["O", "CC=O"])]);
        // "CCO"/"CC=O"/"O" are not guaranteed parseable by a minimal test
        // build, so this only asserts order-independence of the raw-string
        // fallback path, not real chematic canonicalization.
        assert_eq!(compute_route_id("CCO", &r_a), compute_route_id("CCO", &r_b));
    }

    #[test]
    fn route_id_is_independent_of_step_order() {
        let r_a = route(vec![
            step("rule:x", "A", &["B"]),
            step("rule:y", "B", &["C"]),
        ]);
        let r_b = route(vec![
            step("rule:y", "B", &["C"]),
            step("rule:x", "A", &["B"]),
        ]);
        assert_eq!(compute_route_id("A", &r_a), compute_route_id("A", &r_b));
    }

    #[test]
    fn route_id_changes_when_template_id_changes() {
        let r_a = route(vec![step("rule:x", "A", &["B"])]);
        let r_b = route(vec![step("rule:z", "A", &["B"])]);
        assert_ne!(compute_route_id("A", &r_a), compute_route_id("A", &r_b));
    }

    #[test]
    fn route_id_changes_when_canonical_target_changes() {
        let r = route(vec![step("rule:x", "A", &["B"])]);
        assert_ne!(compute_route_id("A", &r), compute_route_id("Z", &r));
    }

    fn sample_route_assessment(route_id: &str, hard_failures: Vec<HardFailure>) -> RouteAssessment {
        RouteAssessment {
            route_id: route_id.to_string(),
            route_depth: 1,
            route_cost: 1.0,
            stock_termination_status: StockTerminationStatus::AllLeavesVerifiedInConfiguredStock,
            target_element_accounting_status: ElementAccountingStatus::Accounted,
            forward_validation_status: ForwardValidationStatus::NotEvaluated,
            evidence_coverage: EvidenceCoverage::default(),
            hard_failures,
            validation_gaps: vec![],
            warnings: vec![],
        }
    }

    #[test]
    fn reproducibility_hash_is_deterministic_for_identical_inputs() {
        let ras = vec![sample_route_assessment("sha256:aaa", vec![])];
        let h1 = compute_reproducibility_hash("rh", "sh", "ch", "CCO", &ras);
        let h2 = compute_reproducibility_hash("rh", "sh", "ch", "CCO", &ras);
        assert_eq!(h1, h2);
    }

    #[test]
    fn reproducibility_hash_changes_when_a_hard_failure_is_added() {
        let clean = vec![sample_route_assessment("sha256:aaa", vec![])];
        let rejected = vec![sample_route_assessment(
            "sha256:aaa",
            vec![HardFailure::RouteGraphInconsistent],
        )];
        assert_ne!(
            compute_reproducibility_hash("rh", "sh", "ch", "CCO", &clean),
            compute_reproducibility_hash("rh", "sh", "ch", "CCO", &rejected)
        );
    }

    #[test]
    fn reproducibility_hash_changes_when_route_order_changes() {
        let a = vec![
            sample_route_assessment("sha256:aaa", vec![]),
            sample_route_assessment("sha256:bbb", vec![]),
        ];
        let b = vec![
            sample_route_assessment("sha256:bbb", vec![]),
            sample_route_assessment("sha256:aaa", vec![]),
        ];
        // Deliberately NOT equal: compute_reproducibility_hash hashes
        // stored order as-is (see doc comment) -- it is the caller's job
        // to have already sorted per §4.8 before calling this.
        assert_ne!(
            compute_reproducibility_hash("rh", "sh", "ch", "CCO", &a),
            compute_reproducibility_hash("rh", "sh", "ch", "CCO", &b)
        );
    }
}
