#![forbid(unsafe_code)]

pub mod hints;

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use anyhow::{Context, Result, bail};
use chematic::core::Element;
use chematic::smiles::canonical_smiles;
use renkin::chem_env::{Molecule, RetroRule, aromaticity_integrity_violation, mol_from_smiles};
use renkin::search::Route;
use serde::Serialize;
use sha2::{Digest, Sha256};

pub mod bench;

/// Schema version of [`ForwardPredictionReport`]. Bump whenever a field is
/// added, removed, or its meaning changes, so downstream JSON consumers can
/// detect incompatible changes instead of silently misreading a report.
pub const FORWARD_REPORT_SCHEMA_VERSION: u32 = 1;

// Test-only instrumentation: counts `predict_products_detailed` calls, so
// tests can assert that callers with a single-execution contract
// (`validate_route`, the CLI) never make a redundant second pass over the
// (potentially large) rule set for the same step. Thread-local, not a
// process-wide static: `cargo test` runs tests concurrently on separate
// threads, and a shared counter would be polluted by unrelated tests
// calling `predict_products_detailed` at the same time.
#[cfg(test)]
thread_local! {
    static PREDICT_DETAILED_CALL_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

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
    let fwd = hints::reverse_smirks_shape_only(smirks)?;
    if let Err(e) = chematic::rxn::parse_reaction(&fwd) {
        return Err(format!(
            "forward SMIRKS failed to parse as a reaction: {e:?}"
        ));
    }
    Ok(fwd)
}

/// Every forward SMIRKS to actually attempt for `retro_smirks`. For a
/// `[#N]`-bearing template (Issue #88), `#N` doesn't say whether the atom
/// is aromatic, so every independently-validated retro-direction reading
/// (`renkin::chem_env::application_smirks_variants`) is reversed and
/// forward-validated in turn -- keeping every one that passes, not
/// guessing a single answer. For an ordinary SMIRKS, behaves exactly like
/// calling [`reverse_smirks_validated`] once: a one-element `Vec` on
/// success, empty on failure. `rule.smirks`/`rule.template_id` are never
/// touched by this -- callers still report the *original* template's
/// identity in every `ForwardCandidateSource`/`ForwardEnumerationSource`,
/// regardless of which variant a given outcome came from.
fn forward_smirks_variants(retro_smirks: &str) -> Vec<String> {
    if !retro_smirks.contains('#') {
        return match reverse_smirks_validated(retro_smirks) {
            Ok(fwd) => vec![fwd],
            Err(_) => Vec::new(),
        };
    }
    renkin::chem_env::application_smirks_variants(retro_smirks)
        .iter()
        .filter_map(|retro_variant| reverse_smirks_validated(retro_variant).ok())
        .collect()
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
        // Checked against the raw, just-constructed product molecule --
        // before this function's own canonical_smiles/mol_from_smiles
        // round-trip below, which (like an external tool's sanitizer) can
        // silently repair or reject the exact defect this catches, hiding
        // it here even when it's still semantically wrong (Issue #90).
        if let Some(violation) = aromaticity_integrity_violation(mol) {
            return Err(violation.reason_code());
        }
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

/// Hashes a sequence of strings into `hasher` with an unambiguous framing:
/// the element count, then each element as (length, bytes). A plain
/// `.join(".")` is not safe here -- a single canonical SMILES can itself
/// contain a `.` (e.g. a disconnected salt/ion-pair species), so
/// `["C.C", "N"]` and `["C", "C.N"]` would join to the identical string
/// `"C.C.N"` despite being different sequences. Length-prefixing makes the
/// encoding injective: the original sequence can always be reconstructed
/// from the byte stream, so two different sequences can never produce it.
pub(crate) fn hash_string_sequence(hasher: &mut Sha256, values: &[String]) {
    hasher.update((values.len() as u64).to_be_bytes());
    for value in values {
        let bytes = value.as_bytes();
        hasher.update((bytes.len() as u64).to_be_bytes());
        hasher.update(bytes);
    }
}

/// Candidate identity, documented so downstream consumers can independently
/// recompute it: SHA-256 over a domain separator (`renkin-forward-candidate-v1`,
/// pinning this to a specific framing so it can be revised later without
/// silently colliding with an older scheme), the sorted canonical reactants
/// ([`hash_string_sequence`]), an explicit section separator, then the
/// sorted canonical product multiset ([`hash_string_sequence`]).
fn candidate_id_for(reactant_canon: &[String], products: &[String]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"renkin-forward-candidate-v1\0");
    hash_string_sequence(&mut hasher, reactant_canon);
    hasher.update(b"\0products\0");
    hash_string_sequence(&mut hasher, products);
    format!("sha256:{}", renkin::sha256_hex(hasher.finalize()))
}

/// `chematic::rxn::run_reactants` binds `reactants[i]` to the i-th
/// left-hand-side SMIRKS component positionally -- it does not itself try
/// every assignment of the supplied molecules to template components. A
/// two-(or more-)reactant template can therefore match in one caller-given
/// order and silently fail to match in the reverse order, even though the
/// same molecules and template are involved. To make candidate discovery
/// independent of the order the caller happened to type `--reactants` in,
/// every distinct ordering is tried and their outcomes pooled -- outcomes
/// found this way still collapse into the same candidate, since
/// [`candidate_id_for`] keys off the *sorted* canonical reactants regardless
/// of which ordering produced them. The caller-visible order (`ForwardReactant`,
/// `input_index`) is never touched; only which orderings are attempted
/// against `run_reactants`.
///
/// Capped at [`MAX_PERMUTED_REACTANTS`] reactants (permutations grow
/// factorially; real templates rarely have more components) -- above the
/// cap only the caller's original order is tried, and `predict_products_detailed`
/// reports a `reactant_permutations_capped` warning rather than silently
/// reducing coverage. Below the cap, if every reactant already canonicalizes
/// to the same SMILES, permuting can't find anything new and is skipped.
const MAX_PERMUTED_REACTANTS: usize = 3;

fn reactant_orderings<'a>(
    mol_refs: &[&'a Molecule],
    reactant_canon: &[String],
) -> (Vec<Vec<&'a Molecule>>, bool) {
    if mol_refs.len() < 2 {
        return (vec![mol_refs.to_vec()], false);
    }
    if mol_refs.len() > MAX_PERMUTED_REACTANTS {
        return (vec![mol_refs.to_vec()], true);
    }

    let mut sorted_canon = reactant_canon.to_vec();
    sorted_canon.sort_unstable();
    if sorted_canon.windows(2).all(|w| w[0] == w[1]) {
        return (vec![mol_refs.to_vec()], false);
    }

    let mut indices: Vec<usize> = (0..mol_refs.len()).collect();
    let mut orderings = Vec::new();
    permute_indices(&mut indices, 0, &mut |perm| {
        orderings.push(perm.iter().map(|&i| mol_refs[i]).collect());
    });
    (orderings, false)
}

fn permute_indices(indices: &mut [usize], k: usize, visit: &mut impl FnMut(&[usize])) {
    if k == indices.len() {
        visit(indices);
        return;
    }
    for i in k..indices.len() {
        indices.swap(k, i);
        permute_indices(indices, k + 1, visit);
        indices.swap(k, i);
    }
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
    if reactants.is_empty() {
        bail!("at least one reactant is required");
    }
    #[cfg(test)]
    PREDICT_DETAILED_CALL_COUNT.with(|c| c.set(c.get() + 1));

    let mut reactant_mols = Vec::with_capacity(reactants.len());
    for (idx, smiles) in reactants.iter().enumerate() {
        let mol = mol_from_smiles(smiles)
            .with_context(|| format!("reactant {idx} ({smiles:?}) failed to parse"))?;
        reactant_mols.push(mol);
    }
    // Caller-supplied order is never *sorted* -- it's reported verbatim in
    // `forward_reactants` below (`input_index`) and is always one of the
    // orderings tried against `run_reactants` (see `reactant_orderings`).
    // Additional orderings may also be tried, since `run_reactants` binds
    // reactant slots to SMIRKS components positionally; trying more
    // orderings is not the same as reordering the caller's input. The
    // candidate-identity fingerprint below separately uses a sorted copy.
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

    let (orderings, permutations_capped) = reactant_orderings(&mol_refs, &reactant_canon);
    if permutations_capped {
        warnings.push(ForwardWarning {
            code: "reactant_permutations_capped".to_string(),
            template_id: None,
            rule_name: None,
            message: format!(
                "{} reactants exceeds the {MAX_PERMUTED_REACTANTS}-reactant cap on trying \
                 every ordering against multi-component templates; only the supplied order \
                 was tried, so candidates that require a different ordering may be missing",
                mol_refs.len()
            ),
        });
    }

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

        let fwd_variants = forward_smirks_variants(&rule.smirks);
        if fwd_variants.is_empty() {
            let msg = format!(
                "template {:?}: no valid forward SMIRKS reading",
                rule.template_id
            );
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

        // `run_reactants` matches reactant slots to SMIRKS components
        // positionally, so every distinct ordering computed above is tried;
        // an ordering that doesn't match the template's arity/shape just
        // contributes no outcomes, not an error. Only report an error if
        // every (variant, ordering) pair failed. A [#N]-bearing template
        // may have multiple validated readings (Issue #88); trying all of
        // them can produce the same real outcome more than once, but the
        // candidate merge below dedupes by (candidate_id, template_id,
        // rule_name), so this never inflates the final candidate list.
        let mut outcomes: Vec<Vec<Molecule>> = Vec::new();
        let mut last_err = None;
        for fwd in &fwd_variants {
            for ordering in &orderings {
                match chematic::rxn::run_reactants(fwd, ordering) {
                    Ok(o) => outcomes.extend(o),
                    Err(e) => last_err = Some(e),
                }
            }
        }

        if outcomes.is_empty() {
            if let Some(e) = last_err {
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
            }
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
                    let (existing_products, sources) = e.get_mut();
                    // A candidate_id collision between two different product
                    // multisets would silently corrupt merging -- catch it
                    // immediately in debug/test builds rather than let it
                    // manifest as a mysteriously wrong `products` field.
                    debug_assert_eq!(
                        existing_products, &products,
                        "candidate_id collision: different product multisets hashed to the same ID"
                    );
                    match sources.iter_mut().find(|s| {
                        s.template_id == source.template_id && s.rule_name == source.rule_name
                    }) {
                        // The same (template_id, rule_name) reached this
                        // candidate again, possibly with a different weight
                        // or source_rank (e.g. a caller-supplied `rules`
                        // slice with near-duplicate entries) -- merge
                        // deterministically (max weight, min source_rank)
                        // rather than silently keeping whichever arrived
                        // first.
                        Some(existing) => {
                            if source.template_weight > existing.template_weight {
                                existing.template_weight = source.template_weight;
                            }
                            existing.source_rank = existing.source_rank.min(source.source_rank);
                        }
                        None => sources.push(source),
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

    // A symmetric multi-reactant template can raise the exact same
    // diagnostic from more than one tried reactant ordering (e.g. the same
    // invalid/no-op outcome found twice) -- dedupe by full content, keeping
    // first-seen order (via `Vec::retain`'s in-order traversal), so a
    // caller sees each distinct warning once rather than once per ordering
    // that happened to rediscover it.
    let mut seen_warnings = std::collections::HashSet::new();
    warnings.retain(|w| {
        seen_warnings.insert((
            w.code.clone(),
            w.template_id.clone(),
            w.rule_name.clone(),
            w.message.clone(),
        ))
    });

    Ok(ForwardPredictionReport {
        schema_version: FORWARD_REPORT_SCHEMA_VERSION,
        reactants: forward_reactants,
        candidates: built,
        stats,
        warnings,
    })
}

/// Expands a report's merged candidates back into the legacy
/// [`ForwardPrediction`] shape: candidates in their final rank order, each
/// candidate's sources in their own deterministic order, one legacy record
/// per source -- then truncates the resulting **flat record list** to
/// `max_results`. This is the only place that decides the final legacy
/// record count; a caller wanting `result.len() <= max_results` to hold
/// must pass a `report` built with an effectively unlimited
/// `ForwardPredictConfig::max_results` (see [`predict_products`]), since
/// truncating candidates first and expanding after could yield either more
/// or fewer than `max_results` records.
pub fn legacy_predictions_from_report(
    report: &ForwardPredictionReport,
    max_results: usize,
) -> Vec<ForwardPrediction> {
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
    out.truncate(max_results);
    out
}

/// Predict forward reaction products for a given set of reactants.
///
/// Backward-compatible wrapper over [`predict_products_detailed`]. The
/// legacy [`ForwardPrediction`] shape cannot express multiple templates
/// converging on the same product set, so each contributing source is
/// expanded back into its own record -- the same `template` name may
/// legitimately appear more than once. This is not deprecated: new
/// integrations should prefer [`predict_products_detailed`], which retains
/// full candidate/source provenance and structured stats/warnings --
/// notably including [`ForwardWarning`]s, which this function's signature
/// has no way to return; if you need visibility into template-level
/// failures, call [`predict_products_detailed`] directly.
///
/// `max_results` bounds the final flat record count
/// (`result.len() <= max_results` always holds): every candidate is
/// generated internally (not capped at the candidate level) before
/// expanding to legacy records and truncating, since one candidate can
/// expand into several records (one per source).
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
        max_results: usize::MAX,
        ..Default::default()
    };
    let report = predict_products_detailed(reactants, rules, &config)?;
    Ok(legacy_predictions_from_report(&report, max_results))
}

/// Validate each step in a retrosynthetic route using forward reaction prediction.
///
/// For each step, applies forward prediction to the step's precursors
/// **exactly once** (a single [`predict_products_detailed`] call, not one
/// for `verified` and a separate one for `top_predictions`) and checks
/// whether the canonical SMILES of the step's target appears among any
/// candidate's products. `verified` is computed over the full
/// (untruncated) candidate set -- so the outcome a target actually appears
/// in can never be hidden by an arbitrary display cap. This is a behavior
/// change from versions prior to this fix, which computed `verified` only
/// over an already-`--max-results`-truncated list; a route step that was
/// previously `verified: false` purely because its matching template fell
/// outside the top 5 will now correctly read `verified: true`.
/// `top_predictions` remains the same capped, legacy-shaped list as before.
pub fn validate_route(route: &Route, rules: &[RetroRule]) -> Result<Vec<StepValidation>> {
    let mut validations = Vec::with_capacity(route.steps.len());

    for (i, step) in route.steps.iter().enumerate() {
        let reactant_refs: Vec<&str> = step.precursors.iter().map(|s| s.as_str()).collect();

        let full_config = ForwardPredictConfig {
            max_results: usize::MAX,
            ..Default::default()
        };
        let report = predict_products_detailed(&reactant_refs, rules, &full_config)?;

        let target_canon = mol_from_smiles(&step.target)
            .ok()
            .map(|m| canonical_smiles(&m))
            .unwrap_or_else(|| step.target.clone());

        let verified = report
            .candidates
            .iter()
            .any(|c| c.products.contains(&target_canon));

        let top_predictions = legacy_predictions_from_report(&report, 5);

        validations.push(StepValidation {
            step_index: i,
            target: step.target.clone(),
            verified,
            top_predictions,
        });
    }

    Ok(validations)
}

/// SHA-256 hex digest of a file's raw bytes, for provenance recording (e.g.
/// [`ForwardEnumerationStats::templates_file_sha256`]/`partners_file_sha256`).
pub fn sha256_hex_of_file(path: &str) -> Result<String> {
    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read {path:?} for hashing"))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(renkin::sha256_hex(hasher.finalize()))
}

/// One row of an explicit `--partners` SMILES library, used by
/// [`enumerate_products_detailed`] to fill the missing reactant slot of a
/// binary forward template.
#[derive(Debug, Clone, Serialize)]
pub struct PartnerRecord {
    /// 1-based physical line number in the partners file -- always present,
    /// unambiguous, and independent of `label` (which may be missing or
    /// repeated).
    pub row_index: usize,
    /// Optional second whitespace-delimited token on the line (e.g. a
    /// human-readable name), present only when the file supplies one.
    pub label: Option<String>,
    pub input_smiles: String,
    pub canonical_smiles: String,
}

/// Maximum [`PartnerLoadWarning`] entries retained per [`PartnerLoadOutcome`]
/// -- bounded so a partners file with many malformed lines can't inflate the
/// enumeration report unboundedly. `skipped_malformed`/`diagnostics_truncated`
/// always report the true totals even once this cap is hit.
const MAX_PARTNER_LOAD_DIAGNOSTICS: usize = 20;

/// One malformed line encountered while loading a `--partners` file.
#[derive(Debug, Clone, Serialize)]
pub struct PartnerLoadWarning {
    /// 1-based physical line number, same numbering as [`PartnerRecord::row_index`].
    pub row_index: usize,
    pub code: String,
    /// The raw SMILES-position token that failed to parse (not the whole line).
    pub input: String,
    pub message: String,
}

/// Outcome of loading a `--partners` file: valid records, plus a count of
/// lines that failed to parse as SMILES. A malformed *line* is not a hard
/// error by itself -- only a missing/unreadable file or a file with zero
/// valid records is (see [`load_partners_strict`]).
#[derive(Debug, Clone, Default)]
pub struct PartnerLoadOutcome {
    pub records: Vec<PartnerRecord>,
    /// True total count of malformed lines -- never capped, unlike `diagnostics`.
    pub skipped_malformed: usize,
    /// Up to [`MAX_PARTNER_LOAD_DIAGNOSTICS`] per-line diagnostics, in file order.
    pub diagnostics: Vec<PartnerLoadWarning>,
    /// True once `skipped_malformed` exceeds `diagnostics.len()` -- i.e. more
    /// malformed lines existed than fit in the bounded `diagnostics` list.
    pub diagnostics_truncated: bool,
}

/// Strict loader for an explicit `--partners` SMILES file.
///
/// Same line shape as `data/building_blocks.smi` (`#`-prefixed and blank
/// lines skipped, first whitespace-delimited token is SMILES, optional
/// second token retained as `label`), but unlike
/// `renkin::chem_env::ChemEnv::load` this never deduplicates by canonical
/// SMILES: partner multiplicity and row identity must be preserved (two
/// lines with the same SMILES are two distinct partners).
pub fn load_partners_strict(path: &str) -> Result<PartnerLoadOutcome> {
    std::fs::metadata(path)
        .with_context(|| format!("partners file {path:?} does not exist or is not accessible"))?;
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("partners file {path:?} could not be read"))?;

    let mut records = Vec::new();
    let mut skipped_malformed = 0usize;
    let mut diagnostics = Vec::new();
    let mut diagnostics_truncated = false;
    for (line_idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let mut tokens = trimmed.split_whitespace();
        let Some(smiles) = tokens.next() else {
            continue;
        };
        let label = tokens.next().map(|s| s.to_string());
        match mol_from_smiles(smiles) {
            Ok(mol) => records.push(PartnerRecord {
                row_index: line_idx + 1,
                label,
                input_smiles: smiles.to_string(),
                canonical_smiles: canonical_smiles(&mol),
            }),
            Err(e) => {
                skipped_malformed += 1;
                if diagnostics.len() < MAX_PARTNER_LOAD_DIAGNOSTICS {
                    diagnostics.push(PartnerLoadWarning {
                        row_index: line_idx + 1,
                        code: "invalid_partner_smiles".to_string(),
                        input: smiles.to_string(),
                        message: e.to_string(),
                    });
                } else {
                    diagnostics_truncated = true;
                }
            }
        }
    }

    if records.is_empty() {
        bail!("partners file {path:?} contains zero valid partner records");
    }

    Ok(PartnerLoadOutcome {
        records,
        skipped_malformed,
        diagnostics,
        diagnostics_truncated,
    })
}

/// For a forward-direction reaction, determines for each left-hand-side
/// (reactant) component whether any of its atom-map numbers appear anywhere
/// among the right-hand-side (product) components' atom-map numbers.
///
/// `chematic::rxn::run_reactants`'s outcomes carry no atom-map information
/// (every output atom has its map cleared), so whether a molecule bound to a
/// given LHS slot could ever contribute an atom to the product cannot be
/// read off a real outcome after the fact -- it must be decided from the
/// template itself: a slot whose atom-map numbers share zero overlap with
/// the union of every RHS atom-map number can never contribute an atom to
/// any outcome, for any molecule bound to it, independent of which real
/// molecule is tried there.
fn contributing_lhs_slots(reaction: &chematic::rxn::Reaction) -> Vec<bool> {
    let product_maps: std::collections::HashSet<u16> = reaction
        .products
        .iter()
        .flat_map(|p| p.atoms())
        .filter_map(|(_, atom)| atom.atom_map)
        .collect();
    reaction
        .reactants
        .iter()
        .map(|component| {
            component
                .atoms()
                .any(|(_, atom)| atom.atom_map.is_some_and(|m| product_maps.contains(&m)))
        })
        .collect()
}

/// Schema version of [`ForwardEnumerationReport`]. Wholly separate from
/// [`FORWARD_REPORT_SCHEMA_VERSION`] -- bumping one never implies anything
/// about the other.
pub const FORWARD_ENUMERATION_REPORT_SCHEMA_VERSION: u32 = 1;

/// Configuration for [`enumerate_products_detailed`].
#[derive(Debug, Clone)]
pub struct ForwardEnumerationConfig {
    pub max_results: usize,
    /// Cap on partners tried per (template, slot) pair.
    pub max_partners_per_template: usize,
    /// Global cap on (template, slot, partner) combinations attempted across
    /// the whole call, independent of `max_partners_per_template`.
    pub max_combinations: usize,
    pub strict_template_errors: bool,
    pub reject_no_op: bool,
}

impl Default for ForwardEnumerationConfig {
    fn default() -> Self {
        Self {
            max_results: 5,
            max_partners_per_template: 50,
            max_combinations: 2000,
            strict_template_errors: false,
            reject_no_op: true,
        }
    }
}

/// Reference to the partner that filled the "other" slot of a binary match.
#[derive(Debug, Clone, Serialize)]
pub struct ForwardEnumerationPartnerRef {
    pub row_index: usize,
    pub label: Option<String>,
    pub canonical_smiles: String,
}

/// One rule application (at a specific slot, with a specific partner if any)
/// that contributed to a [`ForwardEnumerationCandidate`].
#[derive(Debug, Clone, Serialize)]
pub struct ForwardEnumerationSource {
    pub template_id: String,
    pub rule_name: String,
    pub template_weight: f64,
    pub source_rank: usize,
    /// 0-based LHS slot the known reactant was bound to (always 0 for a
    /// unary template).
    pub slot_index: usize,
    /// `None` for a unary template (no partner needed).
    pub partner: Option<ForwardEnumerationPartnerRef>,
}

/// A merged forward-enumeration candidate: one canonical product multiset,
/// with every contributing (template, slot, partner) retained.
#[derive(Debug, Clone, Serialize)]
pub struct ForwardEnumerationCandidate {
    /// `sha256:<hex>` of the known reactant's canonical SMILES and the
    /// sorted canonical product multiset -- deliberately excludes the
    /// partner, so different partners converging on the same products merge
    /// into one candidate (see [`ForwardEnumerationSource`] for per-partner
    /// provenance).
    pub candidate_id: String,
    pub products: Vec<String>,
    pub rank: usize,
    /// Ranking signal only -- the maximum contributing source's template
    /// weight. This is NOT a calibrated probability of reaction success.
    pub proposal_score: f64,
    pub sources: Vec<ForwardEnumerationSource>,
}

/// Structured, deterministic accounting of one
/// [`enumerate_products_detailed`] call.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ForwardEnumerationStats {
    pub rules_loaded: usize,
    pub smirks_rules: usize,
    pub graph_rules_skipped: usize,
    pub templates_inspected: usize,
    pub templates_unary: usize,
    /// Arity-2 templates with a partners file given and at least one
    /// contributing slot -- i.e. actually attempted, not merely eligible.
    pub templates_binary_supported: usize,
    pub templates_binary_skipped_no_partners: usize,
    /// Arity >= 3: always counted here and reported unsupported via a
    /// warning, never silently skipped.
    pub templates_unsupported_arity: usize,
    /// Count of (template, known-reactant slot) assignments for which at
    /// least one attempted unary/binary application produced an accepted
    /// outcome. Not a substructure-match count: since this PR has no
    /// partner-side pre-filter, this is a byproduct of combinations
    /// actually attempted, not an independent structural pre-check -- if
    /// `--partners` is omitted, or no partner in the file happens to
    /// produce an accepted outcome for a given binary slot, that slot is
    /// simply not counted here (its match status is undetermined, not
    /// zero).
    pub slot_assignments_with_accepted_outcome: usize,
    pub partners_scanned: usize,
    pub partners_matched: usize,
    /// True total count of malformed partner-file lines (never capped).
    pub partner_records_skipped_malformed: usize,
    /// Count of per-line diagnostics actually retained in
    /// `ForwardEnumerationReport::partner_load_warnings` (capped at
    /// [`MAX_PARTNER_LOAD_DIAGNOSTICS`]).
    pub partner_diagnostics_returned: usize,
    /// True when `partner_records_skipped_malformed` exceeds
    /// `partner_diagnostics_returned` -- more malformed lines existed than
    /// were retained as diagnostics.
    pub partner_diagnostics_truncated: bool,
    pub combinations_attempted: usize,
    pub raw_outcomes: usize,
    pub accepted_outcomes_before_merge: usize,
    pub invalid_outcomes_rejected: usize,
    pub no_op_outcomes_rejected: usize,
    /// (template, slot) assignments skipped before ever calling
    /// `run_reactants`, because the slot's atom-map numbers share no overlap
    /// with any product's -- counted separately from `raw_outcomes`.
    pub spectator_slot_skips: usize,
    pub duplicate_candidates_merged: usize,
    pub candidates_before_limit: usize,
    pub candidates_returned: usize,
    pub partners_per_template_capped: bool,
    pub combinations_capped: bool,
    pub results_capped: bool,
    /// OR of `partners_per_template_capped`, `combinations_capped`, `results_capped`.
    pub truncated: bool,
    pub templates_file_sha256: Option<String>,
    pub partners_file_sha256: Option<String>,
}

/// A versioned, fully-detailed single-known-reactant forward-enumeration
/// result. See [`FORWARD_ENUMERATION_REPORT_SCHEMA_VERSION`].
#[derive(Debug, Clone, Serialize)]
pub struct ForwardEnumerationReport {
    pub schema_version: u32,
    pub known_reactant: ForwardReactant,
    pub candidates: Vec<ForwardEnumerationCandidate>,
    pub stats: ForwardEnumerationStats,
    pub warnings: Vec<ForwardWarning>,
    /// Bounded per-line diagnostics for malformed `--partners` lines (see
    /// [`MAX_PARTNER_LOAD_DIAGNOSTICS`]); empty when `--partners` was
    /// omitted or every line parsed cleanly. Cross-check against
    /// `stats.partner_records_skipped_malformed`/`partner_diagnostics_truncated`
    /// for the true total when this list was capped.
    pub partner_load_warnings: Vec<PartnerLoadWarning>,
}

/// Candidate identity for enumeration: SHA-256 over a domain separator
/// distinct from [`candidate_id_for`]'s (so the two hash schemes can never
/// collide), the known reactant's canonical SMILES, an explicit section
/// separator, then the sorted canonical product multiset
/// ([`hash_string_sequence`]). Deliberately excludes the partner -- see
/// [`ForwardEnumerationCandidate::candidate_id`].
fn enumeration_candidate_id_for(known_reactant_canon: &str, products: &[String]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"renkin-forward-enumeration-candidate-v1\0");
    hasher.update((known_reactant_canon.len() as u64).to_be_bytes());
    hasher.update(known_reactant_canon.as_bytes());
    hasher.update(b"\0products\0");
    hash_string_sequence(&mut hasher, products);
    format!("sha256:{}", renkin::sha256_hex(hasher.finalize()))
}

/// Applies one (template, slot, optional-partner) combination -- trying
/// every forward SMIRKS variant in `fwd_variants` (see
/// `forward_smirks_variants`; a single element for an ordinary template)
/// via `run_reactants` -- folding every accepted outcome into `candidates`
/// and every diagnostic into `warnings`/`stats`. Returns whether at least
/// one outcome was accepted (used by the caller to track
/// `stats.slot_assignments_with_accepted_outcome`/`stats.partners_matched`).
/// Two variants that happen to produce the same real outcome merge into
/// one source below (keyed on `(template_id, rule_name, slot_index,
/// partner.row_index)`, not on which variant matched), so trying multiple
/// variants never inflates the candidate list.
#[allow(clippy::too_many_arguments)]
fn apply_combination(
    fwd_variants: &[String],
    ordering: &[&Molecule],
    known_canon: &str,
    rule: &RetroRule,
    source_rank: usize,
    slot_index: usize,
    partner: Option<&PartnerRecord>,
    config: &ForwardEnumerationConfig,
    stats: &mut ForwardEnumerationStats,
    warnings: &mut Vec<ForwardWarning>,
    candidates: &mut BTreeMap<String, (Vec<String>, Vec<ForwardEnumerationSource>)>,
) -> bool {
    let mut matched = false;
    let other_canon = partner.map(|p| p.canonical_smiles.as_str());
    // [#N]-bearing templates (Issue #88) may have multiple validated
    // forward readings; try every one. Two readings producing the same
    // real outcome merge below (keyed on template_id/rule_name/slot_index/
    // partner, not on which reading matched), so this never inflates the
    // candidate list -- only `stats.duplicate_candidates_merged`, which is
    // accurate: it genuinely matched more than once.
    let mut any_ok = false;
    let mut last_err = None;
    for fwd_smirks in fwd_variants {
        let outcomes = match chematic::rxn::run_reactants(fwd_smirks, ordering) {
            Ok(o) => {
                any_ok = true;
                o
            }
            Err(e) => {
                last_err = Some(e);
                continue;
            }
        };
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

            if config.reject_no_op {
                let mut reactant_set: Vec<String> = match other_canon {
                    Some(o) => vec![known_canon.to_string(), o.to_string()],
                    None => vec![known_canon.to_string()],
                };
                reactant_set.sort_unstable();
                if products == reactant_set {
                    stats.no_op_outcomes_rejected += 1;
                    continue;
                }
            }

            if let Some(msg) = atom_charge_imbalance_diagnostic(ordering, outcome) {
                warnings.push(ForwardWarning {
                    code: "atom_balance_diagnostic".to_string(),
                    template_id: Some(rule.template_id.clone()),
                    rule_name: Some(rule.name.clone()),
                    message: msg,
                });
            }

            stats.accepted_outcomes_before_merge += 1;
            matched = true;

            let candidate_id = enumeration_candidate_id_for(known_canon, &products);
            let partner_ref = partner.map(|p| ForwardEnumerationPartnerRef {
                row_index: p.row_index,
                label: p.label.clone(),
                canonical_smiles: p.canonical_smiles.clone(),
            });
            let source = ForwardEnumerationSource {
                template_id: rule.template_id.clone(),
                rule_name: rule.name.clone(),
                template_weight: rule.weight,
                source_rank,
                slot_index,
                partner: partner_ref,
            };

            match candidates.entry(candidate_id) {
                Entry::Vacant(e) => {
                    e.insert((products, vec![source]));
                }
                Entry::Occupied(mut e) => {
                    stats.duplicate_candidates_merged += 1;
                    let (existing_products, sources) = e.get_mut();
                    debug_assert_eq!(
                        existing_products, &products,
                        "candidate_id collision: different product multisets hashed to the same ID"
                    );
                    let dup = sources.iter_mut().find(|s| {
                        s.template_id == source.template_id
                            && s.rule_name == source.rule_name
                            && s.slot_index == source.slot_index
                            && s.partner.as_ref().map(|p| p.row_index)
                                == source.partner.as_ref().map(|p| p.row_index)
                    });
                    match dup {
                        Some(existing) => {
                            existing.source_rank = existing.source_rank.min(source.source_rank);
                        }
                        None => sources.push(source),
                    }
                }
            }
        }
    }
    if !any_ok && let Some(e) = last_err {
        warnings.push(ForwardWarning {
            code: "combination_application_failed".to_string(),
            template_id: Some(rule.template_id.clone()),
            rule_name: Some(rule.name.clone()),
            message: format!("run_reactants failed: {e:?}"),
        });
    }
    matched
}

/// Enumerate forward reaction products reachable from a single known
/// reactant, with full candidate provenance, deterministic ranking, and
/// structured stats/warnings.
///
/// Bounded, template-guided enumeration -- not an open-ended generative
/// predictor. Unary templates apply directly. Binary (two-reactant)
/// templates try the known reactant in each compatible LHS slot and search
/// `partners` for the other slot; `partners` may be omitted, in which case
/// binary templates are skipped (counted + warned), not attempted with an
/// invented partner. Templates requiring two or more missing partners are
/// always counted and reported as unsupported, never silently skipped.
pub fn enumerate_products_detailed(
    known_reactant: &str,
    partners: Option<&[PartnerRecord]>,
    rules: &[RetroRule],
    config: &ForwardEnumerationConfig,
) -> Result<ForwardEnumerationReport> {
    if config.max_results == 0 {
        bail!("max_results must be greater than 0");
    }
    if config.max_partners_per_template == 0 {
        bail!("max_partners_per_template must be greater than 0");
    }
    if config.max_combinations == 0 {
        bail!("max_combinations must be greater than 0");
    }

    let known_mol = mol_from_smiles(known_reactant)
        .with_context(|| format!("known reactant {known_reactant:?} failed to parse"))?;
    let known_canon = canonical_smiles(&known_mol);
    let known_reactant_report = ForwardReactant {
        input_smiles: known_reactant.to_string(),
        canonical_smiles: known_canon.clone(),
        input_index: 0,
    };

    let partner_records: &[PartnerRecord] = partners.unwrap_or(&[]);
    let mut partner_mols: Vec<Molecule> = Vec::with_capacity(partner_records.len());
    for record in partner_records {
        let mol = mol_from_smiles(&record.canonical_smiles).with_context(|| {
            format!(
                "partner row {} ({:?}) failed to re-parse from its own canonical SMILES",
                record.row_index, record.canonical_smiles
            )
        })?;
        partner_mols.push(mol);
    }

    let mut stats = ForwardEnumerationStats {
        rules_loaded: rules.len(),
        partners_scanned: partner_records.len(),
        ..Default::default()
    };
    let mut warnings: Vec<ForwardWarning> = Vec::new();
    let mut candidates: BTreeMap<String, (Vec<String>, Vec<ForwardEnumerationSource>)> =
        BTreeMap::new();
    let mut matched_partner_rows: std::collections::HashSet<usize> =
        std::collections::HashSet::new();

    'templates: for (source_rank, rule) in rules.iter().enumerate() {
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

        stats.templates_inspected += 1;

        let fwd_variants = forward_smirks_variants(&rule.smirks);
        if fwd_variants.is_empty() {
            let msg = format!(
                "template {:?}: no valid forward SMIRKS reading",
                rule.template_id
            );
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

        // Re-parses the first variant (already validated once inside
        // `forward_smirks_variants`) to get structural access to per-slot
        // atom maps for arity detection and spectator-slot analysis --
        // cheap for a short SMIRKS string, and this is the only place in
        // the crate that needs the parsed `Reaction` shape rather than just
        // knowing it parses. Every variant of a `[#N]`-bearing template
        // shares identical atom-map/connectivity structure by construction
        // (only the element/aromaticity annotation at existing positions
        // differs -- see `chem_env::expand_hash_atom_variants`), so arity
        // and spectator-slot classification are the same for all of them;
        // only the actual `run_reactants` call needs to try every variant
        // (see `apply_combination` below).
        let reaction = match chematic::rxn::parse_reaction(&fwd_variants[0]) {
            Ok(r) => r,
            Err(e) => {
                let msg = format!(
                    "template {:?}: forward SMIRKS failed to re-parse for arity/slot analysis: {e:?}",
                    rule.template_id
                );
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

        let arity = reaction.reactants.len();
        let contributing = contributing_lhs_slots(&reaction);

        match arity {
            1 => {
                stats.templates_unary += 1;
                if !contributing[0] {
                    stats.spectator_slot_skips += 1;
                    continue;
                }
                if stats.combinations_attempted >= config.max_combinations {
                    stats.combinations_capped = true;
                    break 'templates;
                }
                stats.combinations_attempted += 1;
                let matched = apply_combination(
                    &fwd_variants,
                    &[&known_mol],
                    &known_canon,
                    rule,
                    source_rank,
                    0,
                    None,
                    config,
                    &mut stats,
                    &mut warnings,
                    &mut candidates,
                );
                if matched {
                    stats.slot_assignments_with_accepted_outcome += 1;
                }
            }
            2 => {
                if partner_records.is_empty() {
                    stats.templates_binary_skipped_no_partners += 1;
                    warnings.push(ForwardWarning {
                        code: "binary_template_skipped_no_partners".to_string(),
                        template_id: Some(rule.template_id.clone()),
                        rule_name: Some(rule.name.clone()),
                        message: "binary template requires --partners to enumerate its \
                                  missing reactant slot"
                            .to_string(),
                    });
                    continue;
                }
                if contributing.iter().any(|&c| c) {
                    stats.templates_binary_supported += 1;
                }

                for (slot_index, &is_contributing) in contributing.iter().enumerate() {
                    if !is_contributing {
                        stats.spectator_slot_skips += 1;
                        continue;
                    }
                    let mut slot_matched = false;
                    for (tried, (partner, partner_mol)) in
                        partner_records.iter().zip(partner_mols.iter()).enumerate()
                    {
                        if tried >= config.max_partners_per_template {
                            stats.partners_per_template_capped = true;
                            break;
                        }
                        if stats.combinations_attempted >= config.max_combinations {
                            stats.combinations_capped = true;
                            break 'templates;
                        }
                        stats.combinations_attempted += 1;

                        let ordering: [&Molecule; 2] = if slot_index == 0 {
                            [&known_mol, partner_mol]
                        } else {
                            [partner_mol, &known_mol]
                        };

                        let matched = apply_combination(
                            &fwd_variants,
                            &ordering,
                            &known_canon,
                            rule,
                            source_rank,
                            slot_index,
                            Some(partner),
                            config,
                            &mut stats,
                            &mut warnings,
                            &mut candidates,
                        );
                        if matched {
                            slot_matched = true;
                            matched_partner_rows.insert(partner.row_index);
                        }
                    }
                    if slot_matched {
                        stats.slot_assignments_with_accepted_outcome += 1;
                    }
                }
            }
            _ => {
                stats.templates_unsupported_arity += 1;
                warnings.push(ForwardWarning {
                    code: "template_arity_unsupported".to_string(),
                    template_id: Some(rule.template_id.clone()),
                    rule_name: Some(rule.name.clone()),
                    message: format!(
                        "template requires {arity} reactant slots; enumerate currently supports \
                         at most one missing partner (arity <= 2), reported as unsupported \
                         rather than silently skipped"
                    ),
                });
            }
        }
    }

    stats.partners_matched = matched_partner_rows.len();
    stats.candidates_before_limit = candidates.len();

    let mut built: Vec<ForwardEnumerationCandidate> = candidates
        .into_iter()
        .map(|(candidate_id, (products, mut sources))| {
            sources.sort_by(|a, b| {
                b.template_weight
                    .total_cmp(&a.template_weight)
                    .then_with(|| a.template_id.cmp(&b.template_id))
                    .then_with(|| a.rule_name.cmp(&b.rule_name))
                    .then_with(|| a.slot_index.cmp(&b.slot_index))
                    .then_with(|| {
                        a.partner
                            .as_ref()
                            .map(|p| p.row_index)
                            .cmp(&b.partner.as_ref().map(|p| p.row_index))
                    })
            });
            let proposal_score = sources
                .iter()
                .map(|s| s.template_weight)
                .fold(f64::NEG_INFINITY, f64::max);
            ForwardEnumerationCandidate {
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

    let results_capped = built.len() > config.max_results;
    built.truncate(config.max_results);
    for (i, c) in built.iter_mut().enumerate() {
        c.rank = i;
    }

    stats.candidates_returned = built.len();
    stats.results_capped = results_capped;
    stats.truncated =
        stats.partners_per_template_capped || stats.combinations_capped || stats.results_capped;

    let mut seen_warnings = std::collections::HashSet::new();
    warnings.retain(|w| {
        seen_warnings.insert((
            w.code.clone(),
            w.template_id.clone(),
            w.rule_name.clone(),
            w.message.clone(),
        ))
    });

    Ok(ForwardEnumerationReport {
        schema_version: FORWARD_ENUMERATION_REPORT_SCHEMA_VERSION,
        known_reactant: known_reactant_report,
        candidates: built,
        stats,
        warnings,
        // Populated by the caller from `PartnerLoadOutcome` -- this function
        // only receives already-loaded `PartnerRecord`s, not raw file lines.
        partner_load_warnings: Vec::new(),
    })
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

    // Regression audit for `hints::reverse_smirks_shape_only` extraction
    // (see hints.rs's own doc comment): `predict`/`enumerate`'s accept/reject
    // partition via `reverse_smirks_validated` must be byte-for-byte
    // unchanged by that refactor, since its only change was moving the
    // shape check into a shared function -- the subsequent
    // `chematic::rxn::parse_reaction` sanity gate is still applied exactly
    // as before for these callers. These fixtures lock in the CURRENT
    // (deliberately more restrictive than `hints`) behavior.
    #[test]
    fn reverse_smirks_validated_still_rejects_multi_condition_smarts() {
        // Legitimate SMARTS logical-OR (`;`/`,`), but not valid SMILES --
        // `parse_reaction`'s `parse_smiles` call rejects it. This is the
        // exact fixture that motivated `hints` to bypass this gate; predict/
        // enumerate's own behavior toward it is unchanged and intentional.
        assert!(reverse_smirks_validated("[c:1][N;H1,H2:2]>>[c:1][Br].[N;H1,H2:2]").is_err());
    }

    #[test]
    fn reverse_smirks_validated_still_rejects_recursive_smarts() {
        assert!(reverse_smirks_validated("[c:1][C:2]>>[c:1][Br].[C;$(C=O):2]").is_err());
    }

    #[test]
    fn reverse_smirks_validated_rejects_unbalanced_bracket() {
        assert!(reverse_smirks_validated("[c:1][N:2>>[c:1].[N:2]").is_err());
    }

    #[test]
    fn reverse_smirks_validated_rejects_invalid_atom_map_token() {
        assert!(reverse_smirks_validated("[c:1][N:xyz]>>[c:1].[N:xyz]").is_err());
    }

    #[test]
    fn reverse_smirks_validated_accepts_well_formed_ordinary_smirks_unchanged() {
        // Sanity anchor: plain, unremarkable templates (the vast majority of
        // this crate's default/extracted rules) must still be accepted --
        // the refactor must not have narrowed acceptance for the common case.
        assert!(reverse_smirks_validated("[c:1][Cl]>>[c:1][Br]").is_ok());
        assert!(reverse_smirks_validated("[c:1][N:2]>>[c:1].[N:2]").is_ok());
    }

    /// Snapshot-style regression guard: locks in the exact accept/reject
    /// partition of `reverse_smirks_validated` over the real embedded
    /// default-rule corpus. A pure extract-method refactor (moving the
    /// shape-only check into `hints::reverse_smirks_shape_only`) must not
    /// change this count. If this test's expected numbers ever need to
    /// change, that is itself the signal to double check nothing regressed.
    #[test]
    fn reverse_smirks_validated_default_rules_accept_reject_partition_is_stable() {
        let rules = renkin::chem_env::default_rules();
        let mut smirks_based = 0usize;
        let mut graph_based = 0usize;
        let mut accepted = 0usize;
        let mut rejected = 0usize;
        for rule in &rules {
            if rule.smirks.is_empty() {
                graph_based += 1;
                continue;
            }
            smirks_based += 1;
            match reverse_smirks_validated(&rule.smirks) {
                Ok(_) => accepted += 1,
                Err(_) => rejected += 1,
            }
        }
        assert_eq!(graph_based, 9, "graph-based default rule count changed");
        assert_eq!(smirks_based, rules.len() - 9);
        assert_eq!(
            rejected, 0,
            "every SMIRKS-backed default rule must remain accepted by predict/enumerate's validator"
        );
        assert_eq!(accepted, smirks_based);
    }

    /// Same audit as
    /// `reverse_smirks_validated_default_rules_accept_reject_partition_is_stable`,
    /// extended to the full extracted-template corpus (the workspace-root
    /// `data/templates_extracted.smi` file, ~500 USPTO-derived templates) --
    /// the larger, more chemically-varied corpus this audit is meant to
    /// protect. `load_rules_from_file` already pre-filters on the product
    /// side at load time, but never validates the precursor
    /// (forward-LHS) side, so this is not a redundant check.
    #[test]
    fn reverse_smirks_validated_extracted_templates_accept_reject_partition_is_stable() {
        // History: this test originally locked in a 217/500-rejected
        // baseline caused by `parse_reaction`'s SMILES-based parser
        // rejecting every `[#N]`/`[#N:map]` bare-atomic-number SMARTS
        // primitive (e.g. `[#7:2]`, meaning "any nitrogen, aromaticity
        // unspecified") -- confirmed the sole failure class across all 217
        // (no multi-condition `;`/`,` or recursive SMARTS actually occur in
        // this corpus; see the two fixtures above that still lock those in
        // as *unsupported*, proving this fix stayed narrow).
        //
        // `load_rules_from_file` is unchanged: exactly 500 `RetroRule`s,
        // one per raw line, `smirks` byte-identical to the file (including
        // any `[#N]` atoms) -- ONNX template-scorer `n_rules` and
        // candidate-pool `template_id` uniqueness both depend on this and
        // are exercised by dedicated tests elsewhere (Issue #88's fix
        // deliberately does not touch this loader). What changed is what
        // `predict`/`enumerate` do with a `[#N]`-bearing rule at *apply*
        // time: `forward_smirks_variants` tries every independently-
        // validated concrete-element reading (via
        // `renkin::chem_env::application_smirks_variants`) instead of
        // calling `reverse_smirks_validated` on the raw SMIRKS once and
        // giving up -- `[#N]` doesn't say which reading is correct, so
        // every one that actually parses is tried, nothing is guessed.
        // This test now audits `forward_smirks_variants`, which is what
        // `predict_products_detailed` actually calls, not
        // `reverse_smirks_validated` in isolation (that one-shot function
        // is unchanged and still rejects raw `[#N]` SMIRKS on its own --
        // see the fixture two tests below).
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../data/templates_extracted.smi"
        );
        let rules = renkin::chem_env::load_rules_from_file(path);
        assert_eq!(
            rules.len(),
            500,
            "extracted-template corpus size changed -- update this test's baseline intentionally"
        );
        let distinct_template_ids: std::collections::HashSet<&str> =
            rules.iter().map(|r| r.template_id.as_str()).collect();
        assert_eq!(
            distinct_template_ids.len(),
            500,
            "template_id must be unique across the loaded rule set (candidate-pool export relies \
             on this)"
        );

        let mut rejected = 0usize;
        let mut accepted = 0usize;
        for rule in &rules {
            if forward_smirks_variants(&rule.smirks).is_empty() {
                rejected += 1;
            } else {
                accepted += 1;
            }
        }
        assert_eq!(
            (accepted, rejected),
            (500, 0),
            "predict/enumerate's accept/reject partition over the extracted corpus changed -- \
             if this is a deliberate loosening/tightening of forward_smirks_variants, update \
             this baseline with an explanation; if unintended, it's a real regression"
        );
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

    #[test]
    fn canonicalize_outcome_rejects_aromaticity_integrity_violation() {
        // Issue #90's exact known-bad hash-atom variant, applied directly
        // via chematic::rxn::run_reactants (bypassing chem_env's now-fixed
        // spectator grouping entirely) to prove canonicalize_outcome's own
        // wiring of aromaticity_integrity_violation -- not just chem_env's
        // -- rejects the raw product with the right reason code, before its
        // own canonical_smiles/mol_from_smiles round-trip below gets a
        // chance to (possibly) hide the same defect.
        let bad_variant = "[N:2]-[CH2:1]-[C:3]>>O=[C:1](-[n:2])-[C:3]";
        let target = mol_from_smiles("c1ccccc1CCCNCC").unwrap();
        let results = chematic::rxn::run_reactants(bad_variant, &[&target]).unwrap_or_default();
        let group = results
            .first()
            .expect("the bad variant must still match the acyclic amine");
        let err = canonicalize_outcome(group)
            .expect_err("must reject an aromaticity-integrity violation");
        assert_eq!(err, "aromatic_atom_not_in_ring");
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
    /// non-equivalent halogen sites. `run_reactants` binds reactant slots to
    /// SMIRKS components positionally, so the caller's order alone changes
    /// which combinations it finds: the given order yields 4 raw outcomes
    /// (one a genuine no-op, reassigning each molecule's halogens back to its
    /// own starting arrangement), while the reversed order yields 1 further
    /// raw outcome absent from the first ordering entirely (confirmed via a
    /// scratch probe against the reversed forward SMIRKS). Since
    /// `predict_products` tries every reactant ordering up to
    /// [`MAX_PERMUTED_REACTANTS`] (see [`reactant_orderings`]), all 5 raw
    /// outcomes are pooled, leaving 4 survivors after the 1 no-op is
    /// filtered -- confirmed by checking each raw outcome's product pair
    /// against the reactants' canonical forms directly. The surviving
    /// outcomes' product *pairings* differ even though individual products
    /// repeat across them, which is exactly the information a flat
    /// `flat_map` would destroy by merging everything into one
    /// undifferentiated product bag.
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
            4,
            "expected 4 surviving outcomes (5 raw outcomes across both reactant \
             orderings minus 1 genuine no-op), got {result:?}"
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
            4,
            "all 4 surviving outcomes must have distinct product pairings, got {pairs:?}"
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
        assert_eq!(full.candidates.len(), 4);
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

    fn dummy_report(candidates: Vec<ForwardCandidate>) -> ForwardPredictionReport {
        ForwardPredictionReport {
            schema_version: FORWARD_REPORT_SCHEMA_VERSION,
            reactants: Vec::new(),
            candidates,
            stats: ForwardStats::default(),
            warnings: Vec::new(),
        }
    }

    fn dummy_source(name: &str, weight: f64) -> ForwardCandidateSource {
        ForwardCandidateSource {
            template_id: format!("rule:{name}"),
            rule_name: name.to_string(),
            template_weight: weight,
            source_rank: 0,
        }
    }

    #[test]
    fn legacy_helper_truncates_flat_records_one_candidate_many_sources() {
        let sources: Vec<_> = (0..10)
            .map(|i| dummy_source(&format!("rule_{i}"), f64::from(i)))
            .collect();
        let candidate = ForwardCandidate {
            candidate_id: "sha256:dummy".to_string(),
            products: vec!["X".to_string()],
            rank: 0,
            proposal_score: 9.0,
            sources,
        };
        let report = dummy_report(vec![candidate]);
        assert_eq!(legacy_predictions_from_report(&report, 5).len(), 5);
    }

    #[test]
    fn legacy_helper_truncates_flat_records_many_candidates_one_source_each() {
        let candidates: Vec<_> = (0..10)
            .map(|i| ForwardCandidate {
                candidate_id: format!("sha256:dummy{i}"),
                products: vec![format!("X{i}")],
                rank: i,
                proposal_score: 1.0,
                sources: vec![dummy_source(&format!("rule_{i}"), 1.0)],
            })
            .collect();
        let report = dummy_report(candidates);
        assert_eq!(legacy_predictions_from_report(&report, 5).len(), 5);
    }

    #[test]
    fn legacy_helper_max_results_one() {
        let candidates: Vec<_> = (0..3)
            .map(|i| ForwardCandidate {
                candidate_id: format!("sha256:dummy{i}"),
                products: vec![format!("X{i}")],
                rank: i,
                proposal_score: 1.0,
                sources: vec![dummy_source(&format!("rule_{i}"), 1.0)],
            })
            .collect();
        let report = dummy_report(candidates);
        assert_eq!(legacy_predictions_from_report(&report, 1).len(), 1);
    }

    #[test]
    fn legacy_predict_products_respects_max_results_across_candidate_merges() {
        // Two rules with distinct names sharing the same SMIRKS converge on
        // the same 3 candidates from the dihalide fixture, each candidate
        // ending up with 2 sources -- 6 legacy records total if unbounded.
        // max_results=1 must still cap the FINAL flat list at 1, not at 1
        // candidate's worth of sources (which would be 2 here).
        let mut rule_a = synthetic_metathesis_rule();
        rule_a.name = "rule_a".to_string();
        rule_a.template_id = "rule:rule_a".to_string();
        let mut rule_b = synthetic_metathesis_rule();
        rule_b.name = "rule_b".to_string();
        rule_b.template_id = "rule:rule_b".to_string();

        let result =
            predict_products(&["ClCC(Cl)CBr", "BrCC(Br)CCl"], &[rule_a, rule_b], 1).unwrap();
        assert!(
            result.len() <= 1,
            "expected at most 1 record, got {result:?}"
        );
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
    fn candidate_id_products_join_ambiguity_is_resolved() {
        // A naive `.join(".")` would make these two DIFFERENT product
        // sequences collide: ["C.C", "N"].join(".") == "C.C.N"
        //                    ["C", "C.N"].join(".") == "C.C.N"
        let id_a = candidate_id_for(&["X".to_string()], &["C.C".to_string(), "N".to_string()]);
        let id_b = candidate_id_for(&["X".to_string()], &["C".to_string(), "C.N".to_string()]);
        assert_ne!(id_a, id_b, "product-side join ambiguity must not collide");
    }

    #[test]
    fn candidate_id_reactants_join_ambiguity_is_resolved() {
        let id_a = candidate_id_for(&["C.C".to_string(), "N".to_string()], &["X".to_string()]);
        let id_b = candidate_id_for(&["C".to_string(), "C.N".to_string()], &["X".to_string()]);
        assert_ne!(id_a, id_b, "reactant-side join ambiguity must not collide");
    }

    #[test]
    fn candidate_id_is_order_independent_given_presorted_input() {
        // candidate_id_for itself just hashes whatever order it's given --
        // order-independence is a property of the CALLER always sorting
        // first (canonicalize_outcome, predict_products_detailed's
        // reactant_canon). This test pins that contract: the same multiset,
        // sorted, gives the same ID regardless of the order it started in.
        let mut products_a = vec!["b".to_string(), "a".to_string()];
        products_a.sort_unstable();
        let mut products_b = vec!["a".to_string(), "b".to_string()];
        products_b.sort_unstable();
        assert_eq!(products_a, products_b);
        let id_a = candidate_id_for(&["X".to_string()], &products_a);
        let id_b = candidate_id_for(&["X".to_string()], &products_b);
        assert_eq!(id_a, id_b);
    }

    #[test]
    fn candidate_id_differs_by_product_multiplicity() {
        // ["CO"] and ["CO", "CO"] are different candidates (a multiset, not
        // a set) -- the ID must reflect that, not just the set of distinct
        // products.
        let one = candidate_id_for(&["X".to_string()], &["CO".to_string()]);
        let two = candidate_id_for(&["X".to_string()], &["CO".to_string(), "CO".to_string()]);
        assert_ne!(one, two, "differing product multiplicity must not collide");
    }

    #[test]
    fn same_input_produces_byte_identical_report_json() {
        let rules = renkin::chem_env::default_rules();
        let config = ForwardPredictConfig::default();
        let report_a = predict_products_detailed(&["CC(=O)O", "CCO"], &rules, &config).unwrap();
        let report_b = predict_products_detailed(&["CC(=O)O", "CCO"], &rules, &config).unwrap();
        let json_a = serde_json::to_string(&report_a).unwrap();
        let json_b = serde_json::to_string(&report_b).unwrap();
        assert_eq!(json_a, json_b);
    }

    #[test]
    fn reactant_input_order_alone_does_not_change_candidate_ids() {
        let rules = renkin::chem_env::default_rules();
        let config = ForwardPredictConfig::default();
        let report_a =
            predict_products_detailed(&["Oc1ccccc1C(=O)O", "CCO"], &rules, &config).unwrap();
        let report_b =
            predict_products_detailed(&["CCO", "Oc1ccccc1C(=O)O"], &rules, &config).unwrap();
        let mut ids_a: Vec<&str> = report_a
            .candidates
            .iter()
            .map(|c| c.candidate_id.as_str())
            .collect();
        let mut ids_b: Vec<&str> = report_b
            .candidates
            .iter()
            .map(|c| c.candidate_id.as_str())
            .collect();
        ids_a.sort_unstable();
        ids_b.sort_unstable();
        assert_eq!(
            ids_a, ids_b,
            "swapping reactant input order must not change the resulting candidate ID set"
        );
    }

    #[test]
    fn reactant_permutations_beyond_cap_emit_a_warning() {
        let rules = renkin::chem_env::default_rules();
        let config = ForwardPredictConfig::default();
        // MAX_PERMUTED_REACTANTS + 1 distinct reactants: too many for every
        // ordering to be tried, so only the caller's order is attempted and
        // the caller must be told coverage was reduced, not left to assume
        // the full order-independence guarantee silently held anyway.
        let report =
            predict_products_detailed(&["CCO", "CC(=O)O", "c1ccccc1", "CCN"], &rules, &config)
                .unwrap();
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.code == "reactant_permutations_capped"),
            "expected a reactant_permutations_capped warning for {} reactants, got {:?}",
            4,
            report.warnings
        );
    }

    #[test]
    fn duplicate_source_metadata_conflict_merges_to_max_weight_min_rank() {
        // Two entries in the caller's `rules` slice share the same
        // (template_id, rule_name) but differ in weight/position -- this
        // must merge deterministically, not silently keep whichever the
        // loop visited first.
        let mut first = synthetic_metathesis_rule();
        first.weight = 3.0;
        let mut second = synthetic_metathesis_rule(); // same name/template_id
        second.weight = 9.0;

        let report = predict_products_detailed(
            &["ClCC(Cl)CBr", "BrCC(Br)CCl"],
            &[first, second],
            &ForwardPredictConfig {
                max_results: usize::MAX,
                ..Default::default()
            },
        )
        .unwrap();
        for candidate in &report.candidates {
            let matching: Vec<_> = candidate
                .sources
                .iter()
                .filter(|s| s.rule_name == "synthetic_halide_metathesis")
                .collect();
            assert_eq!(
                matching.len(),
                1,
                "duplicate (template_id, rule_name) must merge to one source entry"
            );
            assert_eq!(matching[0].template_weight, 9.0, "must keep the max weight");
            assert_eq!(matching[0].source_rank, 0, "must keep the min source_rank");
        }
    }

    #[test]
    fn canonicalize_outcome_preserves_product_multiplicity() {
        // Two entries in one outcome that happen to be the SAME molecule
        // must survive as two entries, not be deduplicated into one.
        let a = mol_from_smiles("CO").unwrap();
        let b = mol_from_smiles("CO").unwrap();
        let products = canonicalize_outcome(&[a, b]).unwrap();
        assert_eq!(products.len(), 2);
        assert_eq!(products[0], products[1]);
    }

    #[test]
    fn template_application_error_reported_as_warning_in_non_strict_mode() {
        // A real default rule that requires exactly 1 reactant, applied to 2,
        // fails inside run_reactants (ReactantCountMismatch) rather than at
        // SMIRKS-reversal time.
        let rules = renkin::chem_env::default_rules();
        let bad_rule = rules
            .iter()
            .find(|r| r.name == "aryl_chloride_to_bromide")
            .expect("expected aryl_chloride_to_bromide in default_rules()")
            .clone();
        let report = predict_products_detailed(
            &["c1ccccc1Cl", "CCO"],
            &[bad_rule],
            &ForwardPredictConfig::default(),
        )
        .unwrap();
        assert_eq!(report.stats.template_application_errors, 1);
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.code == "template_application_failed"),
            "expected a template_application_failed warning, got {:?}",
            report.warnings
        );
    }

    #[test]
    fn template_application_error_is_hard_error_in_strict_mode() {
        let rules = renkin::chem_env::default_rules();
        let bad_rule = rules
            .iter()
            .find(|r| r.name == "aryl_chloride_to_bromide")
            .expect("expected aryl_chloride_to_bromide in default_rules()")
            .clone();
        let config = ForwardPredictConfig {
            strict_template_errors: true,
            ..Default::default()
        };
        assert!(predict_products_detailed(&["c1ccccc1Cl", "CCO"], &[bad_rule], &config).is_err());
    }

    #[test]
    fn candidate_ranking_is_deterministic_across_score_source_count_and_id() {
        // Three synthetic single-source candidates with distinct proposal
        // scores must come back sorted strictly by score descending.
        let mut high = synthetic_metathesis_rule();
        high.name = "high_weight".to_string();
        high.template_id = "rule:high_weight".to_string();
        high.weight = 9.0;
        let mut mid = synthetic_metathesis_rule();
        mid.name = "mid_weight".to_string();
        mid.template_id = "rule:mid_weight".to_string();
        mid.weight = 4.0;
        let mut low = synthetic_metathesis_rule();
        low.name = "low_weight".to_string();
        low.template_id = "rule:low_weight".to_string();
        low.weight = 1.0;

        let report = predict_products_detailed(
            &["ClCC(Cl)CBr", "BrCC(Br)CCl"],
            &[low, high, mid],
            &ForwardPredictConfig {
                max_results: usize::MAX,
                ..Default::default()
            },
        )
        .unwrap();
        // Every candidate here is single-sourced from one of the 3 identical
        // synthetic rules (same SMIRKS, different weights) applied to the
        // same reactants, so they all converge into the SAME 3 candidates,
        // each merging all 3 sources -- ranking must still be deterministic
        // by proposal_score (== max source weight == 9.0 for all of them
        // here since every candidate is reachable by all 3 rules), then by
        // product multiset lexicographically as the tiebreaker.
        let scores: Vec<f64> = report.candidates.iter().map(|c| c.proposal_score).collect();
        for w in scores.windows(2) {
            assert!(
                w[0] >= w[1],
                "candidates must be sorted score-descending: {scores:?}"
            );
        }
        let products: Vec<&Vec<String>> = report.candidates.iter().map(|c| &c.products).collect();
        let mut sorted_products = products.clone();
        sorted_products.sort();
        assert_eq!(
            products, sorted_products,
            "equal-score candidates must be ordered by product multiset lexicographically"
        );
    }

    /// Compatibility regression: pins `validate_route`'s `verified` outcome
    /// for a route step built on a real default rule
    /// (`sonogashira_retro`, bromobenzene + propyne -> 1-phenylpropyne), so
    /// this specific, previously-working case can never silently flip
    /// during future refactors of the candidate/merge pipeline. Was
    /// `aryl_ether_retro` (salicylic acid + ethanol -> 2-ethoxybenzoic
    /// acid) until that rule was converted to a graph-based Rust function
    /// (empty smirks -- see docs/design/retro-rule-precision-gaps-v0.md
    /// #1), then `buchwald_hartwig_retro` (2-bromobenzoic acid + ethylamine
    /// -> 2-(ethylamino)benzoic acid) until that rule was itself removed
    /// from `default_rules()` (issue #77: ring-fused-nitrogen atom loss,
    /// plus a corrupted surviving fragment) -- each time substituted with a
    /// structurally analogous SMIRKS-backed rule, re-verified with a
    /// scratch probe to still hold.
    #[test]
    fn validate_route_golden_fixture_verified_true() {
        use renkin::search::ReactionStep;

        let rules = renkin::chem_env::default_rules();
        let step = ReactionStep {
            rule: "sonogashira_retro".to_string(),
            template_id: "rule:sonogashira_retro".to_string(),
            target: "c1ccccc1C#CC".to_string(),
            precursors: vec!["Brc1ccccc1".to_string(), "C#CC".to_string()],
            conditions: None,
            atom_economy: None,
            atom_economy_raw_percent: None,
            atom_economy_status: renkin::search::AtomEconomyStatus::NotEvaluable,
            step_confidence: 1.0,
            procedure_hint: None,
            reaction_family: None,
            metadata_source: None,
            metadata_scope: None,
            evidence: None,
        };
        let route = Route {
            steps: vec![step],
            depth: 1,
            score: 1.0,
            building_blocks: vec!["Brc1ccccc1".to_string(), "C#CC".to_string()],
            confidence: 1.0,
            convergency: 1.0,
            success_probability: 1.0,
            route_cost: 1.0,
        };

        let validations = validate_route(&route, &rules).unwrap();
        assert_eq!(validations.len(), 1);
        assert!(
            validations[0].verified,
            "expected sonogashira_retro forward application to verify the target, got {:?}",
            validations[0]
        );
    }

    /// `chematic::rxn::run_reactants` binds precursor slots to SMIRKS
    /// components positionally, so a route search emitting precursors in
    /// either order for the same underlying chemistry must not flip
    /// `verified` -- this is the same reactant-ordering fix as
    /// [`reactant_input_order_alone_does_not_change_candidate_ids`], exercised
    /// through `validate_route` instead of `predict_products_detailed`
    /// directly. Re-verified with a scratch probe (both precursor orders
    /// for this rule/pair genuinely produce `verified: true` here) after
    /// substituting `sonogashira_retro` for the removed
    /// `buchwald_hartwig_retro` fixture -- see the sibling
    /// `validate_route_golden_fixture_verified_true`'s doc comment for why.
    #[test]
    fn validate_route_verified_is_independent_of_precursor_order() {
        use renkin::search::ReactionStep;

        let rules = renkin::chem_env::default_rules();
        let step = ReactionStep {
            rule: "sonogashira_retro".to_string(),
            template_id: "rule:sonogashira_retro".to_string(),
            target: "c1ccccc1C#CC".to_string(),
            precursors: vec!["C#CC".to_string(), "Brc1ccccc1".to_string()],
            conditions: None,
            atom_economy: None,
            atom_economy_raw_percent: None,
            atom_economy_status: renkin::search::AtomEconomyStatus::NotEvaluable,
            step_confidence: 1.0,
            procedure_hint: None,
            reaction_family: None,
            metadata_source: None,
            metadata_scope: None,
            evidence: None,
        };
        let route = Route {
            steps: vec![step],
            depth: 1,
            score: 1.0,
            building_blocks: vec!["C#CC".to_string(), "Brc1ccccc1".to_string()],
            confidence: 1.0,
            convergency: 1.0,
            success_probability: 1.0,
            route_cost: 1.0,
        };

        let validations = validate_route(&route, &rules).unwrap();
        assert_eq!(validations.len(), 1);
        assert!(
            validations[0].verified,
            "reversing precursor order alone must not flip verified to false, got {:?}",
            validations[0]
        );
    }

    #[test]
    fn validate_route_calls_predict_products_detailed_exactly_once_per_step() {
        use renkin::search::ReactionStep;

        PREDICT_DETAILED_CALL_COUNT.with(|c| c.set(0));

        let rules = renkin::chem_env::default_rules();
        let step = ReactionStep {
            rule: "aryl_ether_retro".to_string(),
            template_id: "rule:aryl_ether_retro".to_string(),
            target: "CCOc1ccccc1C(=O)O".to_string(),
            precursors: vec!["Oc1ccccc1C(=O)O".to_string(), "CCO".to_string()],
            conditions: None,
            atom_economy: None,
            atom_economy_raw_percent: None,
            atom_economy_status: renkin::search::AtomEconomyStatus::NotEvaluable,
            step_confidence: 1.0,
            procedure_hint: None,
            reaction_family: None,
            metadata_source: None,
            metadata_scope: None,
            evidence: None,
        };
        let route = Route {
            steps: vec![step],
            depth: 1,
            score: 1.0,
            building_blocks: vec!["Oc1ccccc1C(=O)O".to_string(), "CCO".to_string()],
            confidence: 1.0,
            convergency: 1.0,
            success_probability: 1.0,
            route_cost: 1.0,
        };

        validate_route(&route, &rules).unwrap();
        assert_eq!(
            PREDICT_DETAILED_CALL_COUNT.with(|c| c.get()),
            1,
            "validate_route must call predict_products_detailed exactly once per step, not once for `verified` and again for `top_predictions`"
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

    // -- enumerate_products_detailed ------------------------------------

    /// Real default rule (`renkin::chem_env::default_rules`'s
    /// `aryl_chloride_to_bromide`), reversed forward SMIRKS is
    /// `[c:1][Br]>>[c:1][Cl]` -- a genuine arity-1 (unary) forward
    /// template, empirically confirmed to actually swap the halogen (not a
    /// no-op) rather than assumed from the SMIRKS text alone.
    fn unary_halide_swap_rule() -> RetroRule {
        RetroRule {
            name: "aryl_chloride_to_bromide".to_string(),
            template_id: "rule:aryl_chloride_to_bromide".to_string(),
            smirks: "[c:1][Cl]>>[c:1][Br]".to_string(),
            weight: 1.0,
            required_elements: 0,
        }
    }

    /// Purely synthetic rule whose forward product atom-map numbers ({8, 9})
    /// share zero overlap with its single forward reactant slot's atom-map
    /// numbers ({1, 2}) -- i.e. the reactant is a structural spectator: it
    /// can match the LHS pattern, but the "product" is built from entirely
    /// different mapped atoms with no atom actually carried over. Used only
    /// to exercise `contributing_lhs_slots`/spectator-skip logic; not
    /// meant to represent a real reaction.
    fn synthetic_disconnected_rule() -> RetroRule {
        RetroRule {
            name: "synthetic_disconnected".to_string(),
            template_id: "rule:synthetic_disconnected".to_string(),
            smirks: "[C:9]=[O:8]>>[C:1][Cl:2]".to_string(),
            weight: 1.0,
            required_elements: 0,
        }
    }

    /// Purely synthetic arity-3 rule (three separate single-carbon forward
    /// reactant slots), used only to exercise the "unsupported arity"
    /// counting/warning path -- never actually applied via `run_reactants`.
    fn synthetic_triple_rule() -> RetroRule {
        RetroRule {
            name: "synthetic_triple".to_string(),
            template_id: "rule:synthetic_triple".to_string(),
            smirks: "[C:1][C:2][C:3]>>[C:1].[C:2].[C:3]".to_string(),
            weight: 1.0,
            required_elements: 0,
        }
    }

    fn partner(row_index: usize, smiles: &str) -> PartnerRecord {
        let mol = mol_from_smiles(smiles).unwrap();
        PartnerRecord {
            row_index,
            label: None,
            input_smiles: smiles.to_string(),
            canonical_smiles: canonical_smiles(&mol),
        }
    }

    #[test]
    fn contributing_lhs_slots_detects_both_slots_contributing() {
        let fwd = reverse_smirks_validated(&synthetic_metathesis_rule().smirks).unwrap();
        let reaction = chematic::rxn::parse_reaction(&fwd).unwrap();
        assert_eq!(contributing_lhs_slots(&reaction), vec![true, true]);
    }

    #[test]
    fn contributing_lhs_slots_detects_spectator_slot() {
        let fwd = reverse_smirks_validated(&synthetic_disconnected_rule().smirks).unwrap();
        let reaction = chematic::rxn::parse_reaction(&fwd).unwrap();
        assert_eq!(contributing_lhs_slots(&reaction), vec![false]);
    }

    #[test]
    fn arity_detection_unary_binary_and_unsupported_via_parse_reaction() {
        let unary = reverse_smirks_validated(&unary_halide_swap_rule().smirks).unwrap();
        assert_eq!(
            chematic::rxn::parse_reaction(&unary)
                .unwrap()
                .reactants
                .len(),
            1
        );

        let binary = reverse_smirks_validated(&synthetic_metathesis_rule().smirks).unwrap();
        assert_eq!(
            chematic::rxn::parse_reaction(&binary)
                .unwrap()
                .reactants
                .len(),
            2
        );

        let triple = reverse_smirks_validated(&synthetic_triple_rule().smirks).unwrap();
        assert_eq!(
            chematic::rxn::parse_reaction(&triple)
                .unwrap()
                .reactants
                .len(),
            3
        );
    }

    #[test]
    fn enumerate_unary_template_applies_directly_to_known_reactant() {
        let report = enumerate_products_detailed(
            "Brc1ccccc1",
            None,
            &[unary_halide_swap_rule()],
            &ForwardEnumerationConfig::default(),
        )
        .unwrap();

        assert_eq!(report.candidates.len(), 1);
        // chematic >=0.8.1 (kent-tokyo/chematic#205/#206) unifies explicit-
        // vs-implicit-hydrogen canonicalization, so the reaction-derived
        // form now matches direct parsing of the same compound.
        assert_eq!(
            report.candidates[0].products,
            vec![canonical_smiles(&mol_from_smiles("Clc1ccccc1").unwrap())]
        );
        assert_eq!(report.candidates[0].sources.len(), 1);
        assert_eq!(report.candidates[0].sources[0].slot_index, 0);
        assert!(report.candidates[0].sources[0].partner.is_none());
        assert_eq!(report.stats.templates_unary, 1);
        assert_eq!(report.stats.slot_assignments_with_accepted_outcome, 1);
    }

    #[test]
    fn enumerate_binary_template_known_reactant_matches_either_slot() {
        // Same fixture as `outcomes_are_never_flattened_together`, already
        // empirically verified there: 5 raw outcomes across both reactant
        // orderings, 1 genuine no-op, 4 surviving candidates.
        let partners = vec![partner(1, "BrCC(Br)CCl")];
        let report = enumerate_products_detailed(
            "ClCC(Cl)CBr",
            Some(&partners),
            &[synthetic_metathesis_rule()],
            &ForwardEnumerationConfig {
                max_results: usize::MAX,
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(report.stats.raw_outcomes, 5);
        assert_eq!(report.stats.no_op_outcomes_rejected, 1);
        assert_eq!(report.candidates.len(), 4);
        for c in &report.candidates {
            assert_eq!(c.products.len(), 2);
        }
        assert!(
            [0, 1].iter().all(|slot| report
                .candidates
                .iter()
                .any(|c| c.sources.iter().any(|s| s.slot_index == *slot))),
            "known reactant must have been tried in both slots"
        );
    }

    #[test]
    fn enumerate_multiple_partners_produce_distinct_candidates() {
        let partners = vec![partner(1, "CCBr"), partner(2, "CCCBr")];
        let report = enumerate_products_detailed(
            "CCCCCl",
            Some(&partners),
            &[synthetic_metathesis_rule()],
            &ForwardEnumerationConfig::default(),
        )
        .unwrap();

        assert_eq!(report.candidates.len(), 2);
        let mut product_sets: Vec<Vec<String>> = report
            .candidates
            .iter()
            .map(|c| c.products.clone())
            .collect();
        product_sets.sort();
        product_sets.dedup();
        assert_eq!(
            product_sets.len(),
            2,
            "both partners must yield distinct products"
        );
        assert_eq!(report.stats.no_op_outcomes_rejected, 0);
        assert_eq!(report.stats.partners_matched, 2);
    }

    #[test]
    fn enumerate_different_partners_converging_on_same_products_merge_into_one_candidate() {
        // Two partner rows with the identical SMILES must merge into one
        // candidate but retain two distinct sources (regression test for
        // candidate identity excluding the partner).
        let partners = vec![partner(1, "CCBr"), partner(2, "CCBr")];
        let report = enumerate_products_detailed(
            "CCCCCl",
            Some(&partners),
            &[synthetic_metathesis_rule()],
            &ForwardEnumerationConfig::default(),
        )
        .unwrap();

        assert_eq!(report.candidates.len(), 1);
        assert_eq!(report.stats.duplicate_candidates_merged, 1);
        let sources = &report.candidates[0].sources;
        assert_eq!(
            sources.len(),
            2,
            "both partner rows must be retained as distinct sources"
        );
        let mut row_indices: Vec<usize> = sources
            .iter()
            .filter_map(|s| s.partner.as_ref().map(|p| p.row_index))
            .collect();
        row_indices.sort_unstable();
        assert_eq!(row_indices, vec![1, 2]);
    }

    #[test]
    fn enumerate_cross_notation_partner_rows_converge_into_one_candidate() {
        // Row 1 and row 2 spell the *identical* molecule (ethyl bromide) via
        // genuinely different construction paths -- organic-subset vs
        // bracket notation with a redundant explicit H count -- rather than
        // literally duplicate SMILES text (already covered by
        // `enumerate_different_partners_converging_on_same_products_merge_into_one_candidate`).
        // Before chematic 0.8.1 (kent-tokyo/chematic#205/#206),
        // `canonical_smiles` was not construction-path invariant for this
        // exact shape (a bracket atom whose explicit H count is redundant
        // with valence inference), so these two rows could canonicalize to
        // different strings and the resulting candidates would not merge.
        let partners = vec![partner(1, "CCBr"), partner(2, "CC[Br]")];
        assert_eq!(
            partners[0].canonical_smiles, partners[1].canonical_smiles,
            "precondition: both notations must canonicalize identically"
        );
        let report = enumerate_products_detailed(
            "CCCCCl",
            Some(&partners),
            &[synthetic_metathesis_rule()],
            &ForwardEnumerationConfig::default(),
        )
        .unwrap();

        assert_eq!(
            report.candidates.len(),
            1,
            "differently-spelled-but-identical partners must merge, got {:?}",
            report.candidates
        );
        assert_eq!(report.stats.duplicate_candidates_merged, 1);
        let sources = &report.candidates[0].sources;
        assert_eq!(
            sources.len(),
            2,
            "both partner rows must be retained as distinct sources"
        );
        let mut row_indices: Vec<usize> = sources
            .iter()
            .filter_map(|s| s.partner.as_ref().map(|p| p.row_index))
            .collect();
        row_indices.sort_unstable();
        assert_eq!(row_indices, vec![1, 2]);
    }

    /// Retro SMIRKS (product>>reactants): an aromatic chloride decomposes
    /// into an aromatic bromide plus sodium chloride. Forward direction
    /// (after `reverse_smirks_validated`) is a genuine arity-2 template that
    /// installs Cl onto an aromatic bromide using NaCl as the chloride
    /// source, discarding the displaced Br (an unmapped leaving atom, same
    /// pattern `synthetic_disconnected_rule` exercises for spectator
    /// detection) -- empirically confirmed to actually run and to produce
    /// the same single product `unary_halide_swap_rule` produces directly
    /// from the same known reactant, letting the two independent template
    /// paths (one unary, one binary) converge on one candidate.
    fn synthetic_binary_halide_install_rule() -> RetroRule {
        RetroRule {
            name: "synthetic_binary_halide_install".to_string(),
            template_id: "rule:synthetic_binary_halide_install".to_string(),
            smirks: "[c:1][Cl:2]>>[c:1][Br].[Na][Cl:2]".to_string(),
            weight: 1.0,
            required_elements: 0,
        }
    }

    #[test]
    fn enumerate_unary_and_binary_template_paths_converge_to_one_candidate() {
        let partners = vec![partner(1, "[Na]Cl")];
        let report = enumerate_products_detailed(
            "Brc1ccccc1",
            Some(&partners),
            &[
                unary_halide_swap_rule(),
                synthetic_binary_halide_install_rule(),
            ],
            &ForwardEnumerationConfig::default(),
        )
        .unwrap();

        assert_eq!(
            report.candidates.len(),
            1,
            "unary and binary template paths reaching the same product must \
             merge into one candidate, got {:?}",
            report.candidates
        );
        let candidate = &report.candidates[0];
        assert_eq!(
            candidate.products,
            vec![canonical_smiles(&mol_from_smiles("Clc1ccccc1").unwrap())],
            "must match direct parsing of the same compound (construction-path invariance)"
        );

        // `candidate_id` is a pure function of (known reactant canonical
        // SMILES, products) -- recomputing it independently proves it is
        // identical regardless of which template path produced the merge.
        let known_canon = canonical_smiles(&mol_from_smiles("Brc1ccccc1").unwrap());
        assert_eq!(
            candidate.candidate_id,
            enumeration_candidate_id_for(&known_canon, &candidate.products)
        );

        assert_eq!(
            candidate.sources.len(),
            2,
            "both the unary and binary template contributions must be retained, got {:?}",
            candidate.sources
        );
        let mut template_ids: Vec<&str> = candidate
            .sources
            .iter()
            .map(|s| s.template_id.as_str())
            .collect();
        template_ids.sort_unstable();
        assert_eq!(
            template_ids,
            vec![
                "rule:aryl_chloride_to_bromide",
                "rule:synthetic_binary_halide_install"
            ]
        );
        let unary_source = candidate
            .sources
            .iter()
            .find(|s| s.template_id == "rule:aryl_chloride_to_bromide")
            .unwrap();
        assert!(unary_source.partner.is_none());
        let binary_source = candidate
            .sources
            .iter()
            .find(|s| s.template_id == "rule:synthetic_binary_halide_install")
            .unwrap();
        assert_eq!(binary_source.partner.as_ref().unwrap().row_index, 1);
    }

    #[test]
    fn enumerate_unary_and_binary_convergence_is_byte_identical_across_runs() {
        let partners = vec![partner(1, "[Na]Cl")];
        let rules = [
            unary_halide_swap_rule(),
            synthetic_binary_halide_install_rule(),
        ];
        let a = enumerate_products_detailed(
            "Brc1ccccc1",
            Some(&partners),
            &rules,
            &ForwardEnumerationConfig::default(),
        )
        .unwrap();
        let b = enumerate_products_detailed(
            "Brc1ccccc1",
            Some(&partners),
            &rules,
            &ForwardEnumerationConfig::default(),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
    }

    #[test]
    fn enumerate_no_op_rejection_is_independent_of_reactant_notation() {
        // Same fixture as
        // `enumerate_rejects_no_op_outcome_scoped_to_known_plus_partner_pair`,
        // but written with a redundant-explicit-H bracket atom (`CC[Cl]`
        // instead of `CCCl`) on the known reactant. Both spellings parse to
        // the identical molecule; no-op detection must reject this exactly
        // as it does for the organic-subset spelling, which only holds
        // because chematic >=0.8.1 canonicalizes both notations identically
        // (kent-tokyo/chematic#205/#206).
        let partners = vec![partner(1, "CC[Br]")];
        let report = enumerate_products_detailed(
            "CC[Cl]",
            Some(&partners),
            &[synthetic_metathesis_rule()],
            &ForwardEnumerationConfig::default(),
        )
        .unwrap();

        assert_eq!(report.stats.raw_outcomes, 1);
        assert_eq!(report.stats.no_op_outcomes_rejected, 1);
        assert_eq!(report.stats.accepted_outcomes_before_merge, 0);
        assert!(report.candidates.is_empty());
    }

    #[test]
    fn enumerate_source_dedupe_keys_on_template_slot_and_partner_row() {
        // Same rule, same slot, two different partner rows: sources must
        // stay distinct (not collapse the way `predict`'s
        // (template_id, rule_name)-only dedupe key would).
        let partners = vec![partner(7, "CCBr"), partner(8, "CCBr")];
        let report = enumerate_products_detailed(
            "CCCCCl",
            Some(&partners),
            &[synthetic_metathesis_rule()],
            &ForwardEnumerationConfig::default(),
        )
        .unwrap();
        assert_eq!(report.candidates[0].sources.len(), 2);
    }

    #[test]
    fn enumerate_rejects_no_op_outcome_scoped_to_known_plus_partner_pair() {
        let partners = vec![partner(1, "CCBr")];
        let report = enumerate_products_detailed(
            "CCCl",
            Some(&partners),
            &[synthetic_metathesis_rule()],
            &ForwardEnumerationConfig::default(),
        )
        .unwrap();

        assert_eq!(report.stats.raw_outcomes, 1);
        assert_eq!(report.stats.no_op_outcomes_rejected, 1);
        assert_eq!(report.stats.accepted_outcomes_before_merge, 0);
        assert!(report.candidates.is_empty());
    }

    #[test]
    fn enumerate_rejects_spectator_only_known_reactant_match() {
        let report = enumerate_products_detailed(
            "CCCl",
            None,
            &[synthetic_disconnected_rule()],
            &ForwardEnumerationConfig::default(),
        )
        .unwrap();

        assert_eq!(report.stats.templates_unary, 1);
        assert_eq!(report.stats.spectator_slot_skips, 1);
        assert_eq!(report.stats.combinations_attempted, 0);
        assert!(report.candidates.is_empty());
    }

    #[test]
    fn enumerate_arity_3_plus_reported_unsupported_not_silently_skipped() {
        let report = enumerate_products_detailed(
            "C",
            None,
            &[synthetic_triple_rule()],
            &ForwardEnumerationConfig::default(),
        )
        .unwrap();

        assert_eq!(report.stats.templates_unsupported_arity, 1);
        assert_eq!(report.stats.combinations_attempted, 0);
        assert!(report.candidates.is_empty());
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.code == "template_arity_unsupported")
        );
    }

    #[test]
    fn enumerate_binary_template_skipped_without_partners_file() {
        let report = enumerate_products_detailed(
            "ClCC(Cl)CBr",
            None,
            &[synthetic_metathesis_rule()],
            &ForwardEnumerationConfig::default(),
        )
        .unwrap();

        assert_eq!(report.stats.templates_binary_skipped_no_partners, 1);
        assert_eq!(report.stats.templates_binary_supported, 0);
        assert!(report.candidates.is_empty());
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.code == "binary_template_skipped_no_partners")
        );
    }

    #[test]
    fn enumerate_max_partners_per_template_caps_and_warns() {
        let partners: Vec<PartnerRecord> = (1..=5).map(|i| partner(i, "CCBr")).collect();
        let report = enumerate_products_detailed(
            "CCCCCl",
            Some(&partners),
            &[synthetic_metathesis_rule()],
            &ForwardEnumerationConfig {
                max_partners_per_template: 2,
                ..Default::default()
            },
        )
        .unwrap();

        assert!(report.stats.partners_per_template_capped);
        assert!(report.stats.truncated);
        // 2 contributing slots x 2 partners tried each = 4 combinations.
        assert_eq!(report.stats.combinations_attempted, 4);
    }

    #[test]
    fn enumerate_max_combinations_caps_globally_and_warns() {
        let partners: Vec<PartnerRecord> = (1..=5).map(|i| partner(i, "CCBr")).collect();
        let report = enumerate_products_detailed(
            "CCCCCl",
            Some(&partners),
            &[synthetic_metathesis_rule()],
            &ForwardEnumerationConfig {
                max_combinations: 1,
                ..Default::default()
            },
        )
        .unwrap();

        assert!(report.stats.combinations_capped);
        assert!(report.stats.truncated);
        assert_eq!(report.stats.combinations_attempted, 1);
    }

    #[test]
    fn enumerate_max_results_applied_after_merge_not_before() {
        let partners = vec![partner(1, "CCBr"), partner(2, "CCCBr")];
        let full = enumerate_products_detailed(
            "CCCCCl",
            Some(&partners),
            &[synthetic_metathesis_rule()],
            &ForwardEnumerationConfig {
                max_results: usize::MAX,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(full.candidates.len(), 2);
        assert!(!full.stats.truncated);

        let capped = enumerate_products_detailed(
            "CCCCCl",
            Some(&partners),
            &[synthetic_metathesis_rule()],
            &ForwardEnumerationConfig {
                max_results: 1,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(capped.candidates.len(), 1);
        assert!(capped.stats.results_capped);
        assert!(capped.stats.truncated);
    }

    #[test]
    fn enumerate_candidate_ranking_is_deterministic() {
        let partners = vec![partner(1, "CCBr"), partner(2, "CCCBr")];
        let a = enumerate_products_detailed(
            "CCCCCl",
            Some(&partners),
            &[synthetic_metathesis_rule()],
            &ForwardEnumerationConfig::default(),
        )
        .unwrap();
        let b = enumerate_products_detailed(
            "CCCCCl",
            Some(&partners),
            &[synthetic_metathesis_rule()],
            &ForwardEnumerationConfig::default(),
        )
        .unwrap();
        let ids_a: Vec<String> = a
            .candidates
            .iter()
            .map(|c| c.candidate_id.clone())
            .collect();
        let ids_b: Vec<String> = b
            .candidates
            .iter()
            .map(|c| c.candidate_id.clone())
            .collect();
        assert_eq!(ids_a, ids_b);
        for (i, c) in a.candidates.iter().enumerate() {
            assert_eq!(c.rank, i);
        }
    }

    #[test]
    fn enumerate_stats_accounting_invariants_hold() {
        let partners = vec![partner(1, "CCBr"), partner(2, "CCCBr")];
        let report = enumerate_products_detailed(
            "CCCCCl",
            Some(&partners),
            &[synthetic_metathesis_rule(), unary_halide_swap_rule()],
            &ForwardEnumerationConfig::default(),
        )
        .unwrap();

        assert_eq!(
            report.stats.raw_outcomes,
            report.stats.accepted_outcomes_before_merge
                + report.stats.invalid_outcomes_rejected
                + report.stats.no_op_outcomes_rejected
        );
        assert_eq!(
            report.stats.accepted_outcomes_before_merge - report.stats.duplicate_candidates_merged,
            report.stats.candidates_before_limit
        );
    }

    #[test]
    fn enumerate_same_input_produces_byte_identical_report_json() {
        let partners = vec![partner(1, "CCBr"), partner(2, "CCCBr")];
        let a = enumerate_products_detailed(
            "CCCCCl",
            Some(&partners),
            &[synthetic_metathesis_rule()],
            &ForwardEnumerationConfig::default(),
        )
        .unwrap();
        let b = enumerate_products_detailed(
            "CCCCCl",
            Some(&partners),
            &[synthetic_metathesis_rule()],
            &ForwardEnumerationConfig::default(),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
    }

    #[test]
    fn enumerate_zero_max_results_is_invalid_argument() {
        let err = enumerate_products_detailed(
            "CCCCCl",
            None,
            &[unary_halide_swap_rule()],
            &ForwardEnumerationConfig {
                max_results: 0,
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("max_results"));
    }

    #[test]
    fn enumerate_zero_max_combinations_is_invalid_argument() {
        let err = enumerate_products_detailed(
            "CCCCCl",
            None,
            &[unary_halide_swap_rule()],
            &ForwardEnumerationConfig {
                max_combinations: 0,
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("max_combinations"));
    }

    // -- partner loading -------------------------------------------------

    #[test]
    fn partner_record_parses_row_index_and_optional_label() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "renkin_forward_test_partners_{}.smi",
            std::process::id()
        ));
        std::fs::write(&path, "# comment\n\nCCO ethanol\nCCBr\n").unwrap();

        let outcome = load_partners_strict(path.to_str().unwrap()).unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(outcome.records.len(), 2);
        assert_eq!(outcome.records[0].row_index, 3);
        assert_eq!(outcome.records[0].label.as_deref(), Some("ethanol"));
        assert_eq!(outcome.records[1].row_index, 4);
        assert_eq!(outcome.records[1].label, None);
        assert_eq!(outcome.skipped_malformed, 0);
    }

    #[test]
    fn partner_record_duplicate_smiles_rows_both_retained() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "renkin_forward_test_partners_dup_{}.smi",
            std::process::id()
        ));
        std::fs::write(&path, "CCBr\nCCBr\n").unwrap();

        let outcome = load_partners_strict(path.to_str().unwrap()).unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(outcome.records.len(), 2);
        assert_eq!(outcome.records[0].row_index, 1);
        assert_eq!(outcome.records[1].row_index, 2);
        assert_eq!(
            outcome.records[0].canonical_smiles,
            outcome.records[1].canonical_smiles
        );
    }

    #[test]
    fn load_partners_strict_skips_malformed_lines_and_counts_them() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "renkin_forward_test_partners_malformed_{}.smi",
            std::process::id()
        ));
        std::fs::write(&path, "CCBr\nnot(a smiles\nCCO\n").unwrap();

        let outcome = load_partners_strict(path.to_str().unwrap()).unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(outcome.records.len(), 2);
        assert_eq!(outcome.skipped_malformed, 1);
        assert_eq!(outcome.diagnostics.len(), 1);
        assert_eq!(outcome.diagnostics[0].row_index, 2);
        assert_eq!(outcome.diagnostics[0].code, "invalid_partner_smiles");
        assert_eq!(outcome.diagnostics[0].input, "not(a");
        assert!(!outcome.diagnostics[0].message.is_empty());
        assert!(!outcome.diagnostics_truncated);
    }

    #[test]
    fn load_partners_strict_caps_diagnostics_but_not_the_true_count() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "renkin_forward_test_partners_many_malformed_{}.smi",
            std::process::id()
        ));
        let mut content = String::from("CCBr\n");
        for _ in 0..(MAX_PARTNER_LOAD_DIAGNOSTICS + 5) {
            content.push_str("not(valid\n");
        }
        std::fs::write(&path, &content).unwrap();

        let outcome = load_partners_strict(path.to_str().unwrap()).unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(outcome.records.len(), 1);
        assert_eq!(outcome.skipped_malformed, MAX_PARTNER_LOAD_DIAGNOSTICS + 5);
        assert_eq!(outcome.diagnostics.len(), MAX_PARTNER_LOAD_DIAGNOSTICS);
        assert!(outcome.diagnostics_truncated);
    }

    #[test]
    fn load_partners_strict_hard_errors_on_missing_file() {
        assert!(load_partners_strict("/nonexistent/renkin_forward_partners.smi").is_err());
    }

    #[test]
    fn load_partners_strict_hard_errors_on_zero_valid_records() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "renkin_forward_test_partners_empty_{}.smi",
            std::process::id()
        ));
        std::fs::write(&path, "# only comments\n\n").unwrap();

        let result = load_partners_strict(path.to_str().unwrap());
        std::fs::remove_file(&path).ok();
        assert!(result.is_err());
    }
}
