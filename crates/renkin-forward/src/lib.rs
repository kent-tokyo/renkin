#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use anyhow::{Context, Result, bail};
use chematic::core::Element;
use chematic::smiles::canonical_smiles;
use renkin::chem_env::{Molecule, RetroRule, mol_from_smiles};
use renkin::search::Route;
use serde::Serialize;
use sha2::{Digest, Sha256};

/// Schema version of [`ForwardPredictionReport`]. Bump whenever a field is
/// added, removed, or its meaning changes, so downstream JSON consumers can
/// detect incompatible changes instead of silently misreading a report.
pub const FORWARD_REPORT_SCHEMA_VERSION: u32 = 1;

/// A predicted forward reaction outcome.
///
/// Legacy shape, kept for backward compatibility -- see
/// [`predict_products_detailed`] for the recommended detailed API.
#[derive(Debug, Clone, Serialize)]
pub struct ForwardPrediction {
    /// Rule name that produced this prediction.
    pub template: String,
    /// Predicted product SMILES (may be multiple per reaction outcome).
    pub products: Vec<String>,
    /// Template frequency weight (higher = more common in training data).
    pub weight: f64,
}

/// Forward-validation result for one step of a retrosynthetic route.
#[derive(Debug, Serialize)]
pub struct StepValidation {
    pub step_index: usize,
    /// The expected product (the step's target in the retro route).
    pub target: String,
    /// Whether the forward prediction reproduced the target (canonical SMILES match).
    pub verified: bool,
    /// Top forward predictions for this step's precursors.
    pub top_predictions: Vec<ForwardPrediction>,
}

/// A canonicalized input reactant, in the order the caller supplied it.
#[derive(Debug, Clone, Serialize)]
pub struct ForwardReactant {
    pub input_smiles: String,
    pub canonical_smiles: String,
    pub input_index: usize,
}

/// One rule application that contributed to a [`ForwardCandidate`].
#[derive(Debug, Clone, Serialize)]
pub struct ForwardCandidateSource {
    pub template_id: String,
    pub rule_name: String,
    pub template_weight: f64,
    /// 0-based index of this rule within the caller-supplied `rules` slice.
    /// A provenance/audit field (which input position produced this source),
    /// not a ranking metric.
    pub source_rank: usize,
}

/// A merged forward-prediction candidate: one canonical product multiset,
/// with every contributing template retained (see
/// [`predict_products_detailed`] for the merge rule).
#[derive(Debug, Clone, Serialize)]
pub struct ForwardCandidate {
    /// `sha256:<hex>` of the sorted canonical reactants and the sorted
    /// canonical product multiset -- stable across runs and process restarts.
    pub candidate_id: String,
    /// Canonical product SMILES, sorted, WITH multiplicity (a multiset, not
    /// a set -- `["CO", "CO"]` and `["CO"]` are different candidates).
    pub products: Vec<String>,
    /// 0-based rank within the final, truncated, deterministically-ordered
    /// candidate list.
    pub rank: usize,
    /// Ranking signal only -- the maximum contributing source's template
    /// weight. This is NOT a calibrated probability of reaction success.
    pub proposal_score: f64,
    /// Every contributing template, sorted deterministically (template
    /// weight descending, then template_id, then rule_name).
    pub sources: Vec<ForwardCandidateSource>,
}

/// Structured, deterministic accounting of one [`predict_products_detailed`]
/// call. Every count is independently incremented at the point in the
/// pipeline it describes (none are derived from the others after the fact),
/// so the invariants asserted in this crate's tests are genuine cross-checks:
///
/// `raw_outcomes == accepted_outcomes_before_merge + invalid_outcomes_rejected + no_op_outcomes_rejected`
/// `accepted_outcomes_before_merge - duplicate_candidates_merged == candidates_before_limit`
#[derive(Debug, Clone, Default, Serialize)]
pub struct ForwardStats {
    pub rules_loaded: usize,
    pub smirks_rules: usize,
    pub graph_rules_skipped: usize,
    pub templates_attempted: usize,
    pub templates_matched: usize,
    pub template_application_errors: usize,
    pub raw_outcomes: usize,
    pub accepted_outcomes_before_merge: usize,
    pub invalid_outcomes_rejected: usize,
    pub no_op_outcomes_rejected: usize,
    pub duplicate_candidates_merged: usize,
    pub candidates_before_limit: usize,
    pub candidates_returned: usize,
    pub truncated: bool,
}

/// A non-fatal diagnostic recorded during prediction. Never implies a
/// candidate/outcome was silently dropped without also being reflected in
/// [`ForwardStats`]'s counters where applicable.
#[derive(Debug, Clone, Serialize)]
pub struct ForwardWarning {
    pub code: String,
    pub template_id: Option<String>,
    pub rule_name: Option<String>,
    pub message: String,
}

/// Configuration for [`predict_products_detailed`].
#[derive(Debug, Clone)]
pub struct ForwardPredictConfig {
    pub max_results: usize,
    /// If true, the first template parse/application error aborts the whole
    /// call. If false (default), template-level failures are reported as
    /// [`ForwardWarning`]s and processing continues with the remaining rules.
    pub strict_template_errors: bool,
    /// If true (default), reject an outcome whose canonical product
    /// multiset equals the canonical reactant multiset.
    pub reject_no_op: bool,
}

impl Default for ForwardPredictConfig {
    fn default() -> Self {
        Self {
            max_results: 5,
            strict_template_errors: false,
            reject_no_op: true,
        }
    }
}

/// A versioned, fully-detailed forward-prediction result.
#[derive(Debug, Clone, Serialize)]
pub struct ForwardPredictionReport {
    pub schema_version: u32,
    pub reactants: Vec<ForwardReactant>,
    pub candidates: Vec<ForwardCandidate>,
    pub stats: ForwardStats,
    pub warnings: Vec<ForwardWarning>,
}

/// Reverse a retro SMIRKS string to obtain a forward SMIRKS, validating it
/// along the way.
///
/// Retro direction:  `product_pattern >> precursor_pattern`
/// Forward direction: `precursor_pattern >> product_pattern`
///
/// This is a syntactic reversal of a SMIRKS-backed retro template, not a
/// guarantee that the underlying reaction is chemically reversible. It
/// rejects malformed input (not exactly one `>>`, an empty side) and, where
/// possible, verifies the resulting forward SMIRKS via chematic's own
/// reaction parser -- this catches malformed atom-mapping/bracket syntax
/// that a plain string swap cannot.
fn reverse_smirks_validated(smirks: &str) -> std::result::Result<String, String> {
    let parts: Vec<&str> = smirks.split(">>").collect();
    if parts.len() != 2 {
        return Err(format!(
            "expected exactly one '>>' separator, found {}",
            parts.len().saturating_sub(1)
        ));
    }
    let lhs = parts[0].trim();
    let rhs = parts[1].trim();
    if lhs.is_empty() {
        return Err("left-hand side is empty".to_string());
    }
    if rhs.is_empty() {
        return Err("right-hand side is empty".to_string());
    }
    let fwd = format!("{rhs}>>{lhs}");
    if let Err(e) = chematic::rxn::parse_reaction(&fwd) {
        return Err(format!(
            "forward SMIRKS failed to parse as a reaction: {e:?}"
        ));
    }
    Ok(fwd)
}

/// Backward-compatible alias kept for the existing unit test on plain
/// string-level reversal semantics; production code should go through
/// [`reverse_smirks_validated`].
#[cfg(test)]
fn reverse_smirks(smirks: &str) -> Option<String> {
    let (lhs, rhs) = smirks.split_once(">>")?;
    Some(format!("{rhs}>>{lhs}"))
}

/// Canonicalize and round-trip-validate one reaction outcome's products.
///
/// `run_reactants` returns one `Vec<Molecule>` per independent reaction
/// outcome; every molecule in that `Vec` must be kept together as a single
/// candidate. This function decides whether that outcome, as a whole, is
/// acceptable:
///
/// - Each product is canonicalized, then the canonical SMILES is re-parsed
///   (round-trip check). If *any* product in the outcome fails this, the
///   whole outcome is rejected -- a partially-valid outcome is not "fixed" by
///   dropping only the bad product, since that would silently change what
///   reaction actually happened.
/// - An outcome with zero products is rejected.
///
/// Ring-closure-digit string heuristics were deliberately removed: whether a
/// SMILES has a ring-closure digit is not a general test of chemical
/// validity. Round-tripping through the real parser is.
fn canonicalize_outcome(outcome: &[Molecule]) -> std::result::Result<Vec<String>, &'static str> {
    if outcome.is_empty() {
        return Err("empty_product_outcome");
    }
    let mut products = Vec::with_capacity(outcome.len());
    for mol in outcome {
        let canon = canonical_smiles(mol);
        if mol_from_smiles(&canon).is_err() {
            return Err("product_roundtrip_failed");
        }
        products.push(canon);
    }
    products.sort_unstable();
    Ok(products)
}

fn heavy_atom_count_and_charge(mol: &Molecule) -> (u32, i64) {
    let mut heavy = 0u32;
    let mut charge = 0i64;
    for (_, atom) in mol.atoms() {
        if atom.element != Element::H {
            heavy += 1;
        }
        charge += i64::from(atom.charge);
    }
    (heavy, charge)
}

/// Diagnostic-only atom/charge balance check -- never used to reject a
/// candidate. Reagents, leaving groups, and salts are commonly omitted from
/// a SMIRKS template, so a plain atom-count mismatch is expected and not
/// flagged; only "products gained heavy atoms from nowhere" (an actual
/// red flag) or a net formal charge change (which even an omitted neutral
/// byproduct cannot explain) are reported.
fn atom_charge_imbalance_diagnostic(
    reactants: &[&Molecule],
    outcome: &[Molecule],
) -> Option<String> {
    let (mut r_heavy, mut r_charge) = (0u32, 0i64);
    for mol in reactants {
        let (h, c) = heavy_atom_count_and_charge(mol);
        r_heavy += h;
        r_charge += c;
    }
    let (mut p_heavy, mut p_charge) = (0u32, 0i64);
    for mol in outcome {
        let (h, c) = heavy_atom_count_and_charge(mol);
        p_heavy += h;
        p_charge += c;
    }

    let mut notes = Vec::new();
    if p_heavy > r_heavy {
        notes.push(format!(
            "products have more heavy atoms ({p_heavy}) than reactants ({r_heavy})"
        ));
    }
    if p_charge != r_charge {
        notes.push(format!(
            "net formal charge changed ({r_charge} -> {p_charge})"
        ));
    }
    if notes.is_empty() {
        None
    } else {
        Some(format!(
            "{} (reagents/leaving groups may be legitimately untracked; diagnostic only, not rejected)",
            notes.join("; ")
        ))
    }
}

fn candidate_id_for(reactant_canon: &[String], products: &[String]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(reactant_canon.join("."));
    hasher.update(b"\0");
    hasher.update(products.join("."));
    format!("sha256:{:x}", hasher.finalize())
}

/// Strict wrapper around `renkin::chem_env::load_rules_from_file` for
/// explicitly-user-supplied template files.
///
/// The underlying loader returns an empty `Vec` and only logs a warning on a
/// missing/unreadable file or a file with zero valid templates -- a
/// reasonable default for an optional corpus, but not for a path the user
/// explicitly asked to use: silently proceeding with 0 templates from a
/// typo'd or empty path must not look like success.
pub fn load_templates_strict(path: &str) -> Result<Vec<RetroRule>> {
    std::fs::metadata(path)
        .with_context(|| format!("template file {path:?} does not exist or is not accessible"))?;
    std::fs::read_to_string(path)
        .with_context(|| format!("template file {path:?} could not be read"))?;
    let rules = renkin::chem_env::load_rules_from_file(path);
    if rules.is_empty() {
        bail!("template file {path:?} contains zero valid templates");
    }
    Ok(rules)
}

/// Predict forward reaction products for a given set of reactants, with full
/// candidate provenance, deterministic ranking, and structured stats/warnings.
///
/// Pipeline order: parse reactants (preserving caller order) -> for each
/// SMIRKS-backed rule, reverse+validate its template, apply it, and treat
/// each `run_reactants` outcome as one independent candidate -> merge
/// outcomes whose canonical product multiset is identical, across templates,
/// retaining full source provenance -> sort deterministically -> apply
/// `max_results`. `max_results` is applied only after outcome separation and
/// candidate merge, never before.
///
/// Graph-based rules (empty `smirks`) are skipped -- they have no reversible
/// template string -- and counted in `stats.graph_rules_skipped`, not
/// treated as an error.
pub fn predict_products_detailed(
    reactants: &[&str],
    rules: &[RetroRule],
    config: &ForwardPredictConfig,
) -> Result<ForwardPredictionReport> {
    if config.max_results == 0 {
        bail!("max_results must be greater than 0");
    }

    let mut reactant_mols = Vec::with_capacity(reactants.len());
    for (idx, smiles) in reactants.iter().enumerate() {
        let mol = mol_from_smiles(smiles)
            .with_context(|| format!("reactant {idx} ({smiles:?}) failed to parse"))?;
        reactant_mols.push(mol);
    }
    // Caller-supplied order is preserved for run_reactants; only the
    // candidate-identity fingerprint below uses a sorted copy.
    let mol_refs: Vec<&Molecule> = reactant_mols.iter().collect();

    let forward_reactants: Vec<ForwardReactant> = reactants
        .iter()
        .zip(reactant_mols.iter())
        .enumerate()
        .map(|(input_index, (smi, mol))| ForwardReactant {
            input_smiles: (*smi).to_string(),
            canonical_smiles: canonical_smiles(mol),
            input_index,
        })
        .collect();

    let mut reactant_canon: Vec<String> = forward_reactants
        .iter()
        .map(|r| r.canonical_smiles.clone())
        .collect();
    reactant_canon.sort_unstable();

    let mut stats = ForwardStats {
        rules_loaded: rules.len(),
        ..Default::default()
    };
    let mut warnings: Vec<ForwardWarning> = Vec::new();

    // Keyed by candidate_id -> (products, sources). BTreeMap keeps iteration
    // order deterministic (candidate_id lexicographic) independent of the
    // final explicit sort below.
    let mut candidates: BTreeMap<String, (Vec<String>, Vec<ForwardCandidateSource>)> =
        BTreeMap::new();

    for (source_rank, rule) in rules.iter().enumerate() {
        if rule.smirks.is_empty() {
            stats.graph_rules_skipped += 1;
            continue;
        }
        stats.smirks_rules += 1;

        if !rule.weight.is_finite() {
            let msg = format!(
                "template {:?} has a non-finite weight ({}), excluded from consideration",
                rule.template_id, rule.weight
            );
            if config.strict_template_errors {
                bail!(msg);
            }
            warnings.push(ForwardWarning {
                code: "invalid_template_weight".to_string(),
                template_id: Some(rule.template_id.clone()),
                rule_name: Some(rule.name.clone()),
                message: msg,
            });
            continue;
        }

        stats.templates_attempted += 1;

        let fwd = match reverse_smirks_validated(&rule.smirks) {
            Ok(s) => s,
            Err(reason) => {
                let msg = format!("template {:?}: {reason}", rule.template_id);
                if config.strict_template_errors {
                    bail!(msg);
                }
                warnings.push(ForwardWarning {
                    code: "invalid_forward_smirks".to_string(),
                    template_id: Some(rule.template_id.clone()),
                    rule_name: Some(rule.name.clone()),
                    message: msg,
                });
                continue;
            }
        };

        let outcomes = match chematic::rxn::run_reactants(&fwd, &mol_refs) {
            Ok(o) => o,
            Err(e) => {
                stats.template_application_errors += 1;
                let msg = format!(
                    "template {:?}: run_reactants failed: {e:?}",
                    rule.template_id
                );
                if config.strict_template_errors {
                    bail!(msg);
                }
                warnings.push(ForwardWarning {
                    code: "template_application_failed".to_string(),
                    template_id: Some(rule.template_id.clone()),
                    rule_name: Some(rule.name.clone()),
                    message: msg,
                });
                continue;
            }
        };

        if outcomes.is_empty() {
            continue;
        }
        stats.templates_matched += 1;

        for outcome in &outcomes {
            stats.raw_outcomes += 1;

            let products = match canonicalize_outcome(outcome) {
                Ok(p) => p,
                Err(code) => {
                    stats.invalid_outcomes_rejected += 1;
                    warnings.push(ForwardWarning {
                        code: code.to_string(),
                        template_id: Some(rule.template_id.clone()),
                        rule_name: Some(rule.name.clone()),
                        message: format!("outcome rejected: {code}"),
                    });
                    continue;
                }
            };

            if config.reject_no_op && products == reactant_canon {
                stats.no_op_outcomes_rejected += 1;
                continue;
            }

            if let Some(msg) = atom_charge_imbalance_diagnostic(&mol_refs, outcome) {
                warnings.push(ForwardWarning {
                    code: "atom_balance_diagnostic".to_string(),
                    template_id: Some(rule.template_id.clone()),
                    rule_name: Some(rule.name.clone()),
                    message: msg,
                });
            }

            stats.accepted_outcomes_before_merge += 1;

            let candidate_id = candidate_id_for(&reactant_canon, &products);
            let source = ForwardCandidateSource {
                template_id: rule.template_id.clone(),
                rule_name: rule.name.clone(),
                template_weight: rule.weight,
                source_rank,
            };

            match candidates.entry(candidate_id) {
                Entry::Vacant(e) => {
                    e.insert((products, vec![source]));
                }
                Entry::Occupied(mut e) => {
                    stats.duplicate_candidates_merged += 1;
                    let (_, sources) = e.get_mut();
                    let already_present = sources.iter().any(|s| {
                        s.template_id == source.template_id && s.rule_name == source.rule_name
                    });
                    if !already_present {
                        sources.push(source);
                    }
                }
            }
        }
    }

    stats.candidates_before_limit = candidates.len();

    let mut built: Vec<ForwardCandidate> = candidates
        .into_iter()
        .map(|(candidate_id, (products, mut sources))| {
            sources.sort_by(|a, b| {
                b.template_weight
                    .total_cmp(&a.template_weight)
                    .then_with(|| a.template_id.cmp(&b.template_id))
                    .then_with(|| a.rule_name.cmp(&b.rule_name))
            });
            let proposal_score = sources
                .iter()
                .map(|s| s.template_weight)
                .fold(f64::NEG_INFINITY, f64::max);
            ForwardCandidate {
                candidate_id,
                products,
                rank: 0,
                proposal_score,
                sources,
            }
        })
        .collect();

    built.sort_by(|a, b| {
        b.proposal_score
            .total_cmp(&a.proposal_score)
            .then_with(|| b.sources.len().cmp(&a.sources.len()))
            .then_with(|| a.products.cmp(&b.products))
            .then_with(|| a.candidate_id.cmp(&b.candidate_id))
    });

    let truncated = built.len() > config.max_results;
    built.truncate(config.max_results);
    for (i, c) in built.iter_mut().enumerate() {
        c.rank = i;
    }

    stats.candidates_returned = built.len();
    stats.truncated = truncated;

    Ok(ForwardPredictionReport {
        schema_version: FORWARD_REPORT_SCHEMA_VERSION,
        reactants: forward_reactants,
        candidates: built,
        stats,
        warnings,
    })
}

/// Predict forward reaction products for a given set of reactants.
///
/// Backward-compatible wrapper over [`predict_products_detailed`]. The
/// legacy [`ForwardPrediction`] shape cannot express multiple templates
/// converging on the same product set, so each contributing source is
/// expanded back into its own record -- the same `template` name may
/// legitimately appear more than once. This is not deprecated: new
/// integrations should prefer [`predict_products_detailed`], which retains
/// full candidate/source provenance and structured stats/warnings.
///
/// `max_results == 0` returns an empty result here (matching this function's
/// pre-existing behavior), unlike [`predict_products_detailed`], which
/// treats it as an invalid argument -- see that function's docs.
pub fn predict_products(
    reactants: &[&str],
    rules: &[RetroRule],
    max_results: usize,
) -> Result<Vec<ForwardPrediction>> {
    if max_results == 0 {
        return Ok(Vec::new());
    }
    let config = ForwardPredictConfig {
        max_results,
        ..Default::default()
    };
    let report = predict_products_detailed(reactants, rules, &config)?;
    let mut out = Vec::new();
    for candidate in &report.candidates {
        for source in &candidate.sources {
            out.push(ForwardPrediction {
                template: source.rule_name.clone(),
                products: candidate.products.clone(),
                weight: source.template_weight,
            });
        }
    }
    Ok(out)
}

/// Validate each step in a retrosynthetic route using forward reaction prediction.
///
/// For each step, applies forward prediction to the step's precursors and
/// checks whether the canonical SMILES of the step's target appears among
/// any candidate's products. `verified` is computed over the full
/// (untruncated) candidate set -- so the outcome a target actually appears
/// in can never be hidden by an arbitrary display cap -- while
/// `top_predictions` remains the same capped, legacy-shaped list as before.
pub fn validate_route(route: &Route, rules: &[RetroRule]) -> Result<Vec<StepValidation>> {
    let mut validations = Vec::with_capacity(route.steps.len());

    for (i, step) in route.steps.iter().enumerate() {
        let reactant_refs: Vec<&str> = step.precursors.iter().map(|s| s.as_str()).collect();

        let full_config = ForwardPredictConfig {
            max_results: usize::MAX,
            ..Default::default()
        };
        let full_report = predict_products_detailed(&reactant_refs, rules, &full_config)?;

        let target_canon = mol_from_smiles(&step.target)
            .ok()
            .map(|m| canonical_smiles(&m))
            .unwrap_or_else(|| step.target.clone());

        let verified = full_report
            .candidates
            .iter()
            .any(|c| c.products.contains(&target_canon));

        let top_predictions = predict_products(&reactant_refs, rules, 5)?;

        validations.push(StepValidation {
            step_index: i,
            target: step.target.clone(),
            verified,
            top_predictions,
        });
    }

    Ok(validations)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reverse_smirks() {
        let retro = "[C:1](=[O:2])[O:3]>>[C:1](=[O:2])O.[O:3]";
        let fwd = reverse_smirks(retro).unwrap();
        assert!(fwd.starts_with("[C:1](=[O:2])O.[O:3]>>"));
    }

    #[test]
    fn reverse_smirks_validated_accepts_real_default_rule() {
        let retro = "[c:1][N:2]>>[c:1].[N:2]";
        let fwd = reverse_smirks_validated(retro).unwrap();
        assert_eq!(fwd, "[c:1].[N:2]>>[c:1][N:2]");
    }

    #[test]
    fn reverse_smirks_validated_rejects_zero_arrows() {
        assert!(reverse_smirks_validated("[C:1][C:2]").is_err());
    }

    #[test]
    fn reverse_smirks_validated_rejects_multiple_arrows() {
        assert!(reverse_smirks_validated("[C:1]>>[C:2]>>[C:3]").is_err());
    }

    #[test]
    fn reverse_smirks_validated_rejects_empty_lhs() {
        assert!(reverse_smirks_validated(">>[C:1]").is_err());
    }

    #[test]
    fn reverse_smirks_validated_rejects_empty_rhs() {
        assert!(reverse_smirks_validated("[C:1]>>").is_err());
    }

    #[test]
    fn canonicalize_outcome_rejects_empty_outcome() {
        assert!(canonicalize_outcome(&[]).is_err());
    }

    #[test]
    fn canonicalize_outcome_sorts_products() {
        let a = mol_from_smiles("CCO").unwrap();
        let b = mol_from_smiles("CC(=O)O").unwrap();
        let products = canonicalize_outcome(&[b, a]).unwrap();
        let mut expected = vec![
            canonical_smiles(&mol_from_smiles("CCO").unwrap()),
            canonical_smiles(&mol_from_smiles("CC(=O)O").unwrap()),
        ];
        expected.sort_unstable();
        assert_eq!(products, expected);
    }

    fn synthetic_metathesis_rule() -> RetroRule {
        RetroRule {
            name: "synthetic_halide_metathesis".to_string(),
            template_id: "rule:synthetic_halide_metathesis".to_string(),
            smirks: "[C:1][Br:4].[C:3][Cl:2]>>[C:1][Cl:2].[C:3][Br:4]".to_string(),
            weight: 1.0,
            required_elements: 0,
        }
    }

    /// Regression fixture for outcome separation, verified empirically against
    /// a real `chematic::rxn::run_reactants` call (not hypothesized): a
    /// hand-authored halide-metathesis SMIRKS applied to two dihalides with
    /// non-equivalent halogen sites returns 4 independent raw outcomes, each
    /// with 2 products. One of the 4 (the combination that reassigns each
    /// molecule's halogens back to its own starting arrangement) is a genuine
    /// no-op and is correctly filtered, leaving 3 -- confirmed by checking
    /// each raw outcome's product pair against the reactants' canonical forms
    /// directly. The surviving outcomes' product *pairings* differ even
    /// though individual products repeat across them, which is exactly the
    /// information a flat `flat_map` would destroy by merging everything
    /// into one undifferentiated product bag.
    #[test]
    fn outcomes_are_never_flattened_together() {
        let result = predict_products(
            &["ClCC(Cl)CBr", "BrCC(Br)CCl"],
            &[synthetic_metathesis_rule()],
            10,
        )
        .unwrap();

        assert_eq!(
            result.len(),
            3,
            "expected 3 surviving outcomes (4 raw outcomes minus 1 genuine no-op), got {result:?}"
        );
        for entry in &result {
            assert_eq!(
                entry.products.len(),
                2,
                "each outcome from this fixture has exactly 2 products, got {entry:?}"
            );
        }

        let mut pairs: Vec<Vec<String>> = result.iter().map(|p| p.products.clone()).collect();
        pairs.sort();
        pairs.dedup();
        assert_eq!(
            pairs.len(),
            3,
            "all 3 surviving outcomes must have distinct product pairings, got {pairs:?}"
        );

        let mut reactant_canon = vec![
            canonical_smiles(&mol_from_smiles("ClCC(Cl)CBr").unwrap()),
            canonical_smiles(&mol_from_smiles("BrCC(Br)CCl").unwrap()),
        ];
        reactant_canon.sort_unstable();
        assert!(
            !pairs.contains(&reactant_canon),
            "the no-op outcome must be filtered, not returned as a candidate"
        );
    }

    #[test]
    fn max_results_applied_after_merge_not_before() {
        // Requesting only 1 result must still reflect the *merged, ranked*
        // candidate set, not an arbitrary prefix of raw outcomes.
        let full = predict_products_detailed(
            &["ClCC(Cl)CBr", "BrCC(Br)CCl"],
            &[synthetic_metathesis_rule()],
            &ForwardPredictConfig {
                max_results: usize::MAX,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(full.candidates.len(), 3);
        assert!(!full.stats.truncated);

        let capped = predict_products_detailed(
            &["ClCC(Cl)CBr", "BrCC(Br)CCl"],
            &[synthetic_metathesis_rule()],
            &ForwardPredictConfig {
                max_results: 1,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(capped.candidates.len(), 1);
        assert!(capped.stats.truncated);
        assert_eq!(
            capped.candidates[0].candidate_id,
            full.candidates[0].candidate_id
        );
    }

    #[test]
    fn duplicate_candidate_merged_across_two_templates_retains_both_sources() {
        // Two differently-named/weighted templates sharing the exact same
        // SMIRKS text necessarily produce byte-identical canonical products
        // from the same reactants (same transform, same molecule object
        // shape) -- this isolates the cross-template merge behavior from any
        // bracket-atom/explicit-hydrogen notation differences between two
        // textually-different-but-chemically-equivalent SMIRKS strings
        // (which would otherwise canonicalize to different strings and never
        // merge, a real subtlety hit while designing this fixture).
        let mut rule_a = synthetic_metathesis_rule();
        rule_a.name = "rule_a".to_string();
        rule_a.template_id = "rule:rule_a".to_string();
        rule_a.weight = 2.0;
        let mut rule_b = synthetic_metathesis_rule();
        rule_b.name = "rule_b".to_string();
        rule_b.template_id = "rule:rule_b".to_string();
        rule_b.weight = 5.0;

        let config = ForwardPredictConfig {
            max_results: 10,
            ..Default::default()
        };
        let report =
            predict_products_detailed(&["ClCC(Cl)CBr", "BrCC(Br)CCl"], &[rule_a, rule_b], &config)
                .unwrap();
        let merged = report
            .candidates
            .iter()
            .find(|c| c.sources.len() > 1)
            .expect("expected at least one candidate merged from both templates");
        assert_eq!(merged.sources.len(), 2);
        let mut names: Vec<&str> = merged
            .sources
            .iter()
            .map(|s| s.rule_name.as_str())
            .collect();
        names.sort_unstable();
        assert_eq!(names, vec!["rule_a", "rule_b"]);
        // Sources sorted by template_weight descending: rule_b (5.0) first.
        assert_eq!(merged.sources[0].rule_name, "rule_b");
        assert_eq!(merged.proposal_score, 5.0);
    }

    #[test]
    fn same_template_two_outcomes_same_multiset_dedupes_to_one_source() {
        // The synthetic metathesis fixture's own reactants, run through the
        // SAME rule twice via two independent predict_products_detailed
        // calls whose results are merged manually, would double the source
        // -- assert the in-process de-dup guard directly instead by
        // checking that no candidate in a single real call ever has two
        // sources with the same (template_id, rule_name).
        let report = predict_products_detailed(
            &["ClCC(Cl)CBr", "BrCC(Br)CCl"],
            &[synthetic_metathesis_rule()],
            &ForwardPredictConfig {
                max_results: usize::MAX,
                ..Default::default()
            },
        )
        .unwrap();
        for candidate in &report.candidates {
            let mut seen = std::collections::HashSet::new();
            for source in &candidate.sources {
                assert!(
                    seen.insert((source.template_id.clone(), source.rule_name.clone())),
                    "duplicate source in one candidate: {candidate:?}"
                );
            }
        }
    }

    #[test]
    fn nan_template_weight_is_excluded_not_silently_equal() {
        let bad_rule = RetroRule {
            name: "nan_weight_rule".to_string(),
            template_id: "rule:nan_weight_rule".to_string(),
            smirks: "[C:1][Br:4].[C:3][Cl:2]>>[C:1][Cl:2].[C:3][Br:4]".to_string(),
            weight: f64::NAN,
            required_elements: 0,
        };
        let report = predict_products_detailed(
            &["ClCC(Cl)CBr", "BrCC(Br)CCl"],
            &[bad_rule],
            &ForwardPredictConfig {
                max_results: 10,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            report.candidates.is_empty(),
            "NaN-weight rule must be excluded entirely"
        );
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.code == "invalid_template_weight"),
            "expected an invalid_template_weight warning, got {:?}",
            report.warnings
        );
    }

    #[test]
    fn nan_template_weight_is_hard_error_in_strict_mode() {
        let bad_rule = RetroRule {
            name: "nan_weight_rule".to_string(),
            template_id: "rule:nan_weight_rule".to_string(),
            smirks: "[C:1][Br:4].[C:3][Cl:2]>>[C:1][Cl:2].[C:3][Br:4]".to_string(),
            weight: f64::NAN,
            required_elements: 0,
        };
        let config = ForwardPredictConfig {
            max_results: 10,
            strict_template_errors: true,
            ..Default::default()
        };
        assert!(
            predict_products_detailed(&["ClCC(Cl)CBr", "BrCC(Br)CCl"], &[bad_rule], &config)
                .is_err()
        );
    }

    #[test]
    fn graph_based_rule_is_skipped_and_counted_not_errored() {
        let graph_rule = RetroRule {
            name: "graph_rule".to_string(),
            template_id: "rule:graph_rule".to_string(),
            smirks: String::new(),
            weight: 1.0,
            required_elements: 0,
        };
        let report = predict_products_detailed(
            &["CC(=O)O"],
            &[graph_rule],
            &ForwardPredictConfig::default(),
        )
        .unwrap();
        assert_eq!(report.stats.graph_rules_skipped, 1);
        assert_eq!(report.stats.smirks_rules, 0);
        assert!(report.candidates.is_empty());
    }

    #[test]
    fn stats_accounting_invariants_hold() {
        let report = predict_products_detailed(
            &["ClCC(Cl)CBr", "BrCC(Br)CCl"],
            &[synthetic_metathesis_rule()],
            &ForwardPredictConfig {
                max_results: usize::MAX,
                ..Default::default()
            },
        )
        .unwrap();
        let s = &report.stats;
        assert_eq!(
            s.raw_outcomes,
            s.accepted_outcomes_before_merge
                + s.invalid_outcomes_rejected
                + s.no_op_outcomes_rejected,
            "raw_outcomes invariant violated: {s:?}"
        );
        assert_eq!(
            s.accepted_outcomes_before_merge - s.duplicate_candidates_merged,
            s.candidates_before_limit,
            "candidate merge invariant violated: {s:?}"
        );
        assert_eq!(s.candidates_before_limit, s.candidates_returned);
        assert!(!s.truncated);
    }

    #[test]
    fn invalid_reactant_reports_index_and_smiles() {
        let err = predict_products_detailed(
            &["CCO", "not a smiles("],
            &[synthetic_metathesis_rule()],
            &ForwardPredictConfig::default(),
        )
        .unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains('1'),
            "expected reactant index 1 in error: {msg}"
        );
        assert!(
            msg.contains("not a smiles("),
            "expected the original SMILES in error: {msg}"
        );
    }

    #[test]
    fn reactant_order_and_duplicates_preserved() {
        let report =
            predict_products_detailed(&["CCO", "CCO"], &[], &ForwardPredictConfig::default())
                .unwrap();
        assert_eq!(report.reactants.len(), 2);
        assert_eq!(report.reactants[0].input_index, 0);
        assert_eq!(report.reactants[1].input_index, 1);
        assert_eq!(
            report.reactants[0].canonical_smiles,
            report.reactants[1].canonical_smiles
        );
    }

    #[test]
    fn max_results_zero_is_invalid_argument_for_detailed_api() {
        let config = ForwardPredictConfig {
            max_results: 0,
            ..Default::default()
        };
        assert!(predict_products_detailed(&["CCO"], &[], &config).is_err());
    }

    #[test]
    fn max_results_zero_returns_empty_for_legacy_api() {
        // Preserves predict_products' pre-existing behavior for this edge case.
        let result = predict_products(&["CCO"], &[], 0).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn candidate_id_is_stable_sha256_prefixed() {
        let id_a = candidate_id_for(&["CCO".to_string()], &["CC(=O)O".to_string()]);
        let id_b = candidate_id_for(&["CCO".to_string()], &["CC(=O)O".to_string()]);
        assert_eq!(id_a, id_b);
        assert!(id_a.starts_with("sha256:"));
        let id_c = candidate_id_for(
            &["CCO".to_string()],
            &["CC(=O)O".to_string(), "CC(=O)O".to_string()],
        );
        assert_ne!(
            id_a, id_c,
            "multiset multiplicity must change the candidate ID"
        );
    }

    #[test]
    fn test_predict_products_does_not_panic() {
        let rules = renkin::chem_env::default_rules();
        // acetic acid + ethanol — ester_cleavage reverse may or may not match
        let result = predict_products(&["CC(=O)O", "CCO"], &rules, 5);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_route_smoke() {
        use renkin::chem_env::ChemEnv;
        use renkin::search::{SearchConfig, find_routes};

        let env = ChemEnv::in_memory(&["CC(=O)O", "Oc1ccccc1C(=O)O"]);
        let rules = renkin::chem_env::default_rules();
        let cfg = SearchConfig {
            max_depth: 2,
            max_routes: 1,
            ..Default::default()
        };
        let (routes, _) = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg).unwrap();
        if let Some(route) = routes.first() {
            let v = validate_route(route, &rules).unwrap();
            assert!(!v.is_empty());
        }
    }
}
