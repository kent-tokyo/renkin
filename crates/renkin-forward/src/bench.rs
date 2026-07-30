//! Deterministic forward-prediction benchmark harness.
//!
//! This module is "PR A" (Phase 0 + Phase 1) of
//! [issue #61](https://github.com/kent-tokyo/renkin/issues/61): a frozen
//! benchmark protocol plus the harness that measures it. It intentionally
//! does NOT attempt Phase 2 (proposal-coverage improvements), Phase 3 (a
//! forward-specific reranker), or Phase 5 (a generative-model decision) --
//! see `docs/guides/forward-benchmark.md` for the full frozen protocol,
//! including every judgment call made here and why.
//!
//! Two identifiers matter and are never conflated (mirroring
//! `scripts/train_reranker.py`'s `target_id`/`group_id` distinction on the
//! retrosynthesis side):
//!
//! - **reaction identity** (this module's dedup key): the full
//!   (canonical reactants, canonical accepted-product multisets) pair. Two
//!   corpus lines that agree on both are the same record and are merged.
//! - **group key** (the leakage-safe split key): the canonical reactant
//!   multiset alone, or an explicit corpus-supplied `group_key` (preferred
//!   when the corpus carries real patent-family/chronological metadata).
//!   Two reactions sharing reactants but reporting *different* accepted
//!   products (a genuinely ambiguous/multi-outcome literature reaction)
//!   still land in the SAME split -- letting them fall into different
//!   splits would leak the reactants' identity across train/val/test.
//!
//! Splitting itself is `SHA-256(group_key) mod 100`, bucketed at the SAME
//! cutoffs as `scripts/train_reranker.py`'s `TRAIN_MAX_BUCKET`/
//! `VAL_MAX_BUCKET` (70/85) -- the same algorithm, reimplemented here because
//! Rust and Python cannot share a constant across the language boundary. If
//! either changes, change both, and update both docs.

use std::collections::{BTreeMap, HashSet};

use anyhow::{Result, bail};
use chematic::smiles::canonical_smiles;
use renkin::chem_env::{RetroRule, default_rules, mol_from_smiles};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    ForwardPredictConfig, hash_string_sequence, load_templates_strict, predict_products_detailed,
    sha256_hex_of_file,
};

/// Schema version of one corpus input line. Bump whenever a field is added,
/// removed, or its meaning changes.
pub const FORWARD_BENCH_CORPUS_SCHEMA_VERSION: u32 = 1;
/// Schema version of [`BenchRow`]/[`BenchReport`].
pub const FORWARD_BENCH_REPORT_SCHEMA_VERSION: u32 = 1;

/// Deterministic split-bucket cutoffs: buckets `[0, TRAIN_MAX_BUCKET)` ->
/// train, `[TRAIN_MAX_BUCKET, VAL_MAX_BUCKET)` -> val, the rest -> test.
/// SAME values as `scripts/train_reranker.py`'s `TRAIN_MAX_BUCKET`/
/// `VAL_MAX_BUCKET` (retrosynthesis reranker, PR #59) -- kept numerically
/// identical for cross-benchmark consistency, not because the two harnesses
/// share any code.
pub const TRAIN_MAX_BUCKET: u32 = 70;
pub const VAL_MAX_BUCKET: u32 = 85;

/// Cap on detailed [`CorpusLoadWarning`] entries retained, so a badly
/// malformed corpus file can't inflate the report unboundedly.
/// `CorpusLoadStats`'s counters always report the true totals regardless
/// (see [`load_partners_strict`](crate::load_partners_strict) for the same
/// bounded-diagnostics convention used elsewhere in this crate).
const MAX_CORPUS_LOAD_WARNINGS: usize = 50;

// ---------------------------------------------------------------------
// Corpus input
// ---------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct CorpusRow {
    schema_version: u32,
    reaction_id: String,
    reactants: Vec<String>,
    accepted_products: Vec<Vec<String>>,
    #[serde(default)]
    reaction_class: Option<String>,
    /// Explicit patent-family/chronological grouping key, preferred over the
    /// deterministic reactant-hash fallback when the corpus supplies one
    /// (Phase 0: "prefer patent-family or chronological grouping where
    /// metadata permits").
    #[serde(default)]
    group_key: Option<String>,
}

/// One rejected corpus line -- a diagnostic, never a silent drop (every
/// rejection is also reflected in [`CorpusLoadStats`]'s counters).
#[derive(Debug, Clone, Serialize)]
pub struct CorpusLoadWarning {
    pub line_number: usize,
    pub code: String,
    pub message: String,
}

/// Structured, deterministic accounting of one [`load_corpus`] call. Every
/// count is independently incremented at the point it describes.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CorpusLoadStats {
    pub total_lines: usize,
    pub blank_lines_skipped: usize,
    /// Lines that are not even valid `CorpusRow` JSON (missing/wrong-typed
    /// required fields, or invalid JSON syntax) -- these can't reliably
    /// yield a `reaction_id`, so unlike every other rejection category they
    /// never become a [`BenchRow`], only a stats increment plus a
    /// [`CorpusLoadWarning`].
    pub malformed_json: usize,
    pub wrong_schema_version: usize,
    pub unparseable_smiles: usize,
    pub empty_reactants_or_products: usize,
    pub duplicate_records_merged: usize,
    pub reactions_loaded: usize,
    /// True once rejections exceed [`MAX_CORPUS_LOAD_WARNINGS`] -- the
    /// counters above always hold the true totals regardless.
    pub warnings_truncated: bool,
}

/// One canonicalized, deduplicated benchmark reaction, ready for prediction.
#[derive(Debug, Clone)]
pub struct BenchReaction {
    pub reaction_id: String,
    pub source_line: usize,
    /// The corpus's own reactant SMILES, in the corpus's own order --
    /// **this, not `reactants_canonical`, is what gets passed to
    /// `predict_products_detailed`.** A real caller of `renkin-forward
    /// predict` passes their own reactant text, not a pre-canonicalized,
    /// pre-sorted rewrite of it; `@`/`@@` tetrahedral spelling is relative
    /// to neighbor order, which a text rewrite can change even though the
    /// underlying molecule is identical (observed empirically while
    /// building this harness's fixture -- see
    /// `docs/guides/forward-benchmark.md`'s "Stereochemistry comparison"
    /// section). Feeding the harness's own normalized text instead would
    /// measure the harness's preprocessing, not the engine a caller
    /// actually invokes.
    pub reactants_original: Vec<String>,
    /// Sorted canonical reactant SMILES -- used for reaction identity,
    /// group key, `has_stereochemistry`, and the reported row fields.
    /// Never passed to `predict_products_detailed` (see
    /// `reactants_original`).
    pub reactants_canonical: Vec<String>,
    /// Every accepted correct product multiset, each inner list sorted, the
    /// outer list itself sorted (so declaration order in the corpus never
    /// affects the reaction-identity hash).
    pub accepted_products_canonical: Vec<Vec<String>>,
    pub reaction_class: Option<String>,
    pub group_key: String,
    /// Auto-detected from the presence of `@`/`/`/`\` in any canonical
    /// reactant or accepted-product SMILES -- never trusted from
    /// caller-supplied metadata, since it is fully derivable and therefore
    /// always self-consistent with the SMILES actually used for matching.
    pub has_stereochemistry: bool,
}

/// A corpus line that parsed structurally (has a `reaction_id`) but whose
/// content is invalid -- wrong schema version, empty reactants/products, or
/// unparseable SMILES. Becomes a [`BenchRow`] with
/// [`FailureReason::InputInvalid`], never silently dropped.
#[derive(Debug, Clone)]
pub struct InvalidReactionAttempt {
    pub reaction_id: String,
    pub source_line: usize,
    pub reason: String,
}

/// Reaction identity: SHA-256 over a domain separator, the sorted canonical
/// reactants, then every accepted product multiset (each already sorted,
/// the outer list itself sorted) -- used only to dedupe exact-duplicate
/// corpus records, never as the split/group key (see module docs).
fn reaction_identity_hash(
    reactants_sorted: &[String],
    accepted_products_sorted: &[Vec<String>],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"renkin-forward-bench-reaction-v1\0");
    hash_string_sequence(&mut hasher, reactants_sorted);
    hasher.update(b"\0products\0");
    hasher.update((accepted_products_sorted.len() as u64).to_be_bytes());
    for set in accepted_products_sorted {
        hash_string_sequence(&mut hasher, set);
    }
    format!("{:x}", hasher.finalize())
}

/// Deterministic grouped-reaction-hash fallback (Phase 0: "otherwise use a
/// deterministic grouped reaction hash") -- SHA-256 over a domain separator
/// distinct from [`reaction_identity_hash`]'s, plus the sorted canonical
/// reactants alone (deliberately excluding accepted products -- see module
/// docs on why the split key must be reactant-only).
fn fallback_group_key(reactants_sorted: &[String]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"renkin-forward-bench-group-v1\0");
    hash_string_sequence(&mut hasher, reactants_sorted);
    format!("{:x}", hasher.finalize())
}

/// Deterministic bucket in `[0, 100)` for a group key, via SHA-256 -- not
/// Rust's `Hash`/`DefaultHasher` (unstable across processes) and not a
/// seeded PRNG. The same group key always maps to the same bucket, in this
/// process, any other process, or the Python reranker script's identical
/// `int.from_bytes(digest[:4], "big") % 100` convention.
pub fn split_bucket(group_key: &str) -> u32 {
    let mut hasher = Sha256::new();
    hasher.update(group_key.as_bytes());
    let digest = hasher.finalize();
    u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]]) % 100
}

/// Resolves a group key to its split name (`"train"`/`"val"`/`"test"`) via
/// [`split_bucket`] and the [`TRAIN_MAX_BUCKET`]/[`VAL_MAX_BUCKET`] cutoffs.
pub fn split_for_group(group_key: &str) -> &'static str {
    let bucket = split_bucket(group_key);
    if bucket < TRAIN_MAX_BUCKET {
        "train"
    } else if bucket < VAL_MAX_BUCKET {
        "val"
    } else {
        "test"
    }
}

/// Canonicalizes one SMILES, returning `None` (not an error) on failure --
/// callers decide how a single bad SMILES affects the enclosing record.
fn try_canonicalize(smiles: &str) -> Option<String> {
    mol_from_smiles(smiles)
        .ok()
        .map(|mol| canonical_smiles(&mol))
}

/// Return type of [`load_corpus`]: loaded reactions, invalid-but-attributable
/// attempts, load-time accounting, and bounded diagnostic warnings.
pub type CorpusLoadResult = Result<(
    Vec<BenchReaction>,
    Vec<InvalidReactionAttempt>,
    CorpusLoadStats,
    Vec<CorpusLoadWarning>,
)>;

/// Loads and canonicalizes a JSONL benchmark corpus (see
/// `docs/guides/forward-benchmark.md` for the full schema). Never silently
/// drops a line: every rejection increments a [`CorpusLoadStats`] counter,
/// and a line with content invalid enough to have a `reaction_id` but not
/// enough to predict from still becomes an [`InvalidReactionAttempt`] (see
/// that type's docs for exactly which rejections qualify).
pub fn load_corpus(path: &str) -> CorpusLoadResult {
    let content = std::fs::read_to_string(path).map_err(|e| {
        anyhow::anyhow!("corpus file {path:?} does not exist or is not readable: {e}")
    })?;

    let mut stats = CorpusLoadStats::default();
    let mut warnings: Vec<CorpusLoadWarning> = Vec::new();
    let mut reactions: Vec<BenchReaction> = Vec::new();
    let mut invalid: Vec<InvalidReactionAttempt> = Vec::new();
    let mut seen_identity: HashSet<String> = HashSet::new();

    let push_warning = |stats: &mut CorpusLoadStats,
                        warnings: &mut Vec<CorpusLoadWarning>,
                        w: CorpusLoadWarning| {
        if warnings.len() < MAX_CORPUS_LOAD_WARNINGS {
            warnings.push(w);
        } else {
            stats.warnings_truncated = true;
        }
    };

    for (idx, line) in content.lines().enumerate() {
        let line_number = idx + 1;
        stats.total_lines += 1;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            stats.blank_lines_skipped += 1;
            continue;
        }

        let row: CorpusRow = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(e) => {
                stats.malformed_json += 1;
                push_warning(
                    &mut stats,
                    &mut warnings,
                    CorpusLoadWarning {
                        line_number,
                        code: "malformed_json".to_string(),
                        message: e.to_string(),
                    },
                );
                continue;
            }
        };

        if row.schema_version != FORWARD_BENCH_CORPUS_SCHEMA_VERSION {
            stats.wrong_schema_version += 1;
            push_warning(
                &mut stats,
                &mut warnings,
                CorpusLoadWarning {
                    line_number,
                    code: "wrong_schema_version".to_string(),
                    message: format!(
                        "unsupported corpus schema_version {} (expected {})",
                        row.schema_version, FORWARD_BENCH_CORPUS_SCHEMA_VERSION
                    ),
                },
            );
            invalid.push(InvalidReactionAttempt {
                reaction_id: row.reaction_id,
                source_line: line_number,
                reason: "wrong_schema_version".to_string(),
            });
            continue;
        }

        if row.reactants.is_empty()
            || row.accepted_products.is_empty()
            || row.accepted_products.iter().any(Vec::is_empty)
        {
            stats.empty_reactants_or_products += 1;
            push_warning(
                &mut stats,
                &mut warnings,
                CorpusLoadWarning {
                    line_number,
                    code: "empty_reactants_or_products".to_string(),
                    message: "reactants or an accepted_products entry is empty".to_string(),
                },
            );
            invalid.push(InvalidReactionAttempt {
                reaction_id: row.reaction_id,
                source_line: line_number,
                reason: "empty_reactants_or_products".to_string(),
            });
            continue;
        }

        let mut reactants_canonical: Vec<String> = Vec::with_capacity(row.reactants.len());
        let mut ok = true;
        for smi in &row.reactants {
            match try_canonicalize(smi) {
                Some(c) => reactants_canonical.push(c),
                None => {
                    ok = false;
                    break;
                }
            }
        }
        let mut accepted_products_canonical: Vec<Vec<String>> =
            Vec::with_capacity(row.accepted_products.len());
        if ok {
            'outer: for set in &row.accepted_products {
                let mut canon_set = Vec::with_capacity(set.len());
                for smi in set {
                    match try_canonicalize(smi) {
                        Some(c) => canon_set.push(c),
                        None => {
                            ok = false;
                            break 'outer;
                        }
                    }
                }
                canon_set.sort_unstable();
                accepted_products_canonical.push(canon_set);
            }
        }

        if !ok {
            stats.unparseable_smiles += 1;
            push_warning(
                &mut stats,
                &mut warnings,
                CorpusLoadWarning {
                    line_number,
                    code: "unparseable_smiles".to_string(),
                    message: "a reactant or accepted-product SMILES failed to parse".to_string(),
                },
            );
            invalid.push(InvalidReactionAttempt {
                reaction_id: row.reaction_id,
                source_line: line_number,
                reason: "unparseable_smiles".to_string(),
            });
            continue;
        }

        reactants_canonical.sort_unstable();
        accepted_products_canonical.sort();

        let identity = reaction_identity_hash(&reactants_canonical, &accepted_products_canonical);
        if !seen_identity.insert(identity) {
            stats.duplicate_records_merged += 1;
            continue;
        }

        let group_key = row
            .group_key
            .filter(|k| !k.is_empty())
            .unwrap_or_else(|| fallback_group_key(&reactants_canonical));
        let has_stereochemistry = reactants_canonical
            .iter()
            .chain(accepted_products_canonical.iter().flatten())
            .any(|s| s.contains('@') || s.contains('/') || s.contains('\\'));

        reactions.push(BenchReaction {
            reaction_id: row.reaction_id,
            source_line: line_number,
            reactants_original: row.reactants,
            reactants_canonical,
            accepted_products_canonical,
            reaction_class: row.reaction_class,
            group_key,
            has_stereochemistry,
        });
    }

    stats.reactions_loaded = reactions.len();
    Ok((reactions, invalid, stats, warnings))
}

// ---------------------------------------------------------------------
// Template source (Phase 0's four required benchmark modes)
// ---------------------------------------------------------------------

/// Which rule set a benchmark run used. Phase 0 requires four modes to be
/// named by the protocol; only the first three are implemented in this PR
/// (`ScorerConditioned` has no scorer to condition on until Phase 3/4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateSource {
    Embedded,
    File,
    TrainExtracted,
}

impl TemplateSource {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "embedded" => Ok(Self::Embedded),
            "file" => Ok(Self::File),
            "train-extracted" => Ok(Self::TrainExtracted),
            "scorer-conditioned" => bail!(
                "--template-source scorer-conditioned is named by the frozen Phase 0 protocol as \
                 mode 4, but is not implemented until a scorer exists (issue #61 Phase 3/4) -- not \
                 usable in this PR"
            ),
            other => bail!(
                "unknown --template-source {other:?}; expected 'embedded', 'file', or \
                 'train-extracted' ('scorer-conditioned' is named by the protocol but not yet \
                 implemented)"
            ),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Embedded => "embedded",
            Self::File => "file",
            Self::TrainExtracted => "train_extracted",
        }
    }
}

/// Loads the rule set for exactly one template source. Modes never mix: an
/// explicit file REPLACES the embedded defaults, it does not extend them.
/// This is the opposite of `main.rs`'s `load_rules` (used by `predict`/
/// `enumerate`, which always extends the embedded set) -- Phase 0 is
/// explicit that the benchmark harness must never "silently substitute the
/// embedded fallback corpus for an intended external template set", which
/// cuts both ways: an explicit file must not be silently diluted by the
/// embedded set either.
pub fn load_rules_for_source(
    source: TemplateSource,
    templates_path: Option<&str>,
) -> Result<(Vec<RetroRule>, Option<String>)> {
    match source {
        TemplateSource::Embedded => {
            if templates_path.is_some() {
                bail!(
                    "--templates was given but --template-source is 'embedded' (the default); \
                     pass --template-source file or --template-source train-extracted to use it"
                );
            }
            Ok((default_rules(), None))
        }
        TemplateSource::File | TemplateSource::TrainExtracted => {
            let path = templates_path.ok_or_else(|| {
                anyhow::anyhow!(
                    "--template-source {:?} requires --templates <path>",
                    source.as_str()
                )
            })?;
            let rules = load_templates_strict(path)?;
            let sha = sha256_hex_of_file(path)?;
            Ok((rules, Some(sha)))
        }
    }
}

// ---------------------------------------------------------------------
// Stereochemistry-ignored comparison
// ---------------------------------------------------------------------

/// Strips OpenSMILES stereo markers (`@`, `@@` via repeated `@`, `/`, `\`)
/// at the text level. These four characters have no other meaning in
/// SMILES, so removing them is always syntactically safe; whether the
/// *result* is still valid/canonical is checked separately by re-parsing
/// (see [`stereo_ignored_canonical`]).
fn strip_stereo_markers(smiles: &str) -> String {
    smiles
        .chars()
        .filter(|&c| c != '@' && c != '/' && c != '\\')
        .collect()
}

/// Best-effort stereo-flattened canonical form used only for the
/// "stereochemistry-ignored" comparison dimension.
///
/// ponytail: this is a textual strip-then-reparse-then-recanonicalize, not a
/// structural chirality-clearing operation over chematic's `Atom::chirality`/
/// bond-direction APIs. Verified empirically (see this module's tests) to
/// collapse both tetrahedral (`@`/`@@`) and double-bond (`/`/`\`) stereo
/// markers to the same canonical form as the equivalent achiral input, for
/// every case checked. Ceiling: if the stripped text fails to re-parse (not
/// observed in testing, but not proven impossible for an exotic SMILES),
/// this falls back to the stereo-aware canonical string itself rather than
/// erroring -- a widening comparison silently not widening is safer than an
/// unrelated whole-reaction error. Upgrade to a structural
/// chirality/bond-direction clear if this heuristic is ever shown to
/// misclassify a real case.
fn stereo_ignored_canonical(canonical: &str) -> String {
    let stripped = strip_stereo_markers(canonical);
    match mol_from_smiles(&stripped) {
        Ok(mol) => canonical_smiles(&mol),
        Err(_) => canonical.to_string(),
    }
}

fn stereo_ignored_set(products: &[String]) -> Vec<String> {
    let mut out: Vec<String> = products
        .iter()
        .map(|s| stereo_ignored_canonical(s))
        .collect();
    out.sort_unstable();
    out
}

// ---------------------------------------------------------------------
// Per-reaction row
// ---------------------------------------------------------------------

/// Why one reaction landed where it did -- Phase 1's "success/failure
/// reason" breakdown dimension. Deliberately coarse: classifying *why* a
/// template failed to apply (missing forward SMIRKS, atom-mapping mismatch,
/// etc.) is Phase 2 territory (issue #61), not this harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureReason {
    HitTop1,
    HitTop5,
    HitTop10,
    // Explicit rename: serde's `snake_case` conversion does not insert an
    // underscore before a trailing digit run, so the derive alone would
    // produce "hit_beyond10" here -- inconsistent with every other
    // variant's underscore-before-word convention, and with `as_str()`
    // below. Pinned explicitly so the derived `Serialize` impl (used for
    // `BenchRow::failure_reason`) and `as_str()` (used for breakdown bucket
    // keys) can never silently disagree.
    #[serde(rename = "hit_beyond_10")]
    HitBeyond10,
    CorrectAbsentEmptyPool,
    CorrectAbsentNonemptyPool,
    InputInvalid,
    PredictionError,
}

impl FailureReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HitTop1 => "hit_top1",
            Self::HitTop5 => "hit_top5",
            Self::HitTop10 => "hit_top10",
            Self::HitBeyond10 => "hit_beyond_10",
            Self::CorrectAbsentEmptyPool => "correct_absent_empty_pool",
            Self::CorrectAbsentNonemptyPool => "correct_absent_nonempty_pool",
            Self::InputInvalid => "input_invalid",
            Self::PredictionError => "prediction_error",
        }
    }

    /// True for a row that never received a real candidate pool (no
    /// `predict_products_detailed` call succeeded) -- excluded from every
    /// pool/latency/accuracy aggregate, but still counted in
    /// `valid_input_rate`'s denominator.
    fn is_non_attempt(self) -> bool {
        matches!(self, Self::InputInvalid | Self::PredictionError)
    }
}

/// Per-row provenance -- deliberately duplicated onto every row (not just
/// the report header) so a single row, taken out of context, still fully
/// describes what produced it. Exact chematic-version/rule-set provenance
/// beyond this is carried by the committed `Cargo.lock` (`renkin_forward_version`
/// pins the whole dependency tree transitively) plus `rules_file_sha256`.
#[derive(Debug, Clone, Serialize)]
pub struct RowProvenance {
    pub renkin_forward_version: String,
    pub template_source: String,
    pub rules_file_sha256: Option<String>,
}

/// One benchmark output row -- one per reaction, including reactions whose
/// input was invalid (see [`FailureReason::InputInvalid`]). See
/// `docs/guides/forward-benchmark.md` for the full field-by-field contract.
#[derive(Debug, Clone, Serialize)]
pub struct BenchRow {
    pub reaction_id: String,
    /// 1-based physical line number in the corpus file.
    pub source_line: usize,
    pub split: String,
    pub reaction_class: Option<String>,
    /// The corpus's own reactant text/order -- what was actually passed to
    /// `predict_products_detailed` (see [`BenchReaction::reactants_original`]).
    pub reactants_original: Vec<String>,
    pub reactants_canonical: Vec<String>,
    pub accepted_products_canonical: Vec<Vec<String>>,
    pub num_reactants: usize,
    /// Product count of `accepted_products_canonical[0]` (the primary
    /// accepted answer) -- see module docs for why a reaction with several
    /// accepted outcomes of different arity still gets one well-defined
    /// count.
    pub num_products: usize,
    pub has_stereochemistry: bool,
    pub candidate_count: usize,
    /// Total raw `run_reactants` outcomes attempted for this reaction
    /// (before validity/no-op filtering or merging) -- the denominator
    /// `invalid_product_rate`/`no_op_rate` are computed against.
    pub raw_outcomes: usize,
    pub correct_candidate_present: bool,
    pub best_correct_rank: Option<usize>,
    /// Best rank under the stereochemistry-ignored comparison (see
    /// [`stereo_ignored_canonical`]) -- always `<= best_correct_rank` when
    /// both are present, since the ignored comparison is strictly looser.
    pub best_correct_rank_stereo_ignored: Option<usize>,
    pub top1_hit: bool,
    pub top5_hit: bool,
    pub top10_hit: bool,
    /// `best_correct_rank < 10` under the exact (stereochemistry-aware)
    /// comparison -- identical to `top10_hit`, reported under its own name
    /// so it sits directly next to `stereochemistry_ignored_hit` for a
    /// same-row before/after comparison.
    pub stereochemistry_aware_hit: bool,
    /// `best_correct_rank_stereo_ignored < 10`. `true` while
    /// `stereochemistry_aware_hit` is `false` is the diagnostic signal for
    /// "constitution right, stereochemistry wrong".
    pub stereochemistry_ignored_hit: bool,
    pub invalid_candidate_count: usize,
    pub no_op_candidate_count: usize,
    pub application_warning_count: usize,
    pub application_error_count: usize,
    pub templates_attempted: usize,
    pub rules_loaded: usize,
    /// Wall-clock time for the one `predict_products_detailed` call this row
    /// required. NOT part of this harness's determinism guarantee -- see
    /// `docs/guides/forward-benchmark.md`'s determinism section for exactly
    /// which fields (this one, plus every `latency_ms` aggregate block) are
    /// expected to vary between otherwise-identical runs.
    pub elapsed_ms: f64,
    pub failure_reason: FailureReason,
    pub provenance: RowProvenance,
}

fn invalid_row(attempt: &InvalidReactionAttempt, provenance: &RowProvenance) -> BenchRow {
    BenchRow {
        reaction_id: attempt.reaction_id.clone(),
        source_line: attempt.source_line,
        // A group key (and therefore a split) cannot be computed reliably
        // once the reactants themselves failed to canonicalize -- "unknown"
        // is excluded from every split-based aggregate (see `aggregate`).
        split: "unknown".to_string(),
        reaction_class: None,
        reactants_original: Vec::new(),
        reactants_canonical: Vec::new(),
        accepted_products_canonical: Vec::new(),
        num_reactants: 0,
        num_products: 0,
        has_stereochemistry: false,
        candidate_count: 0,
        raw_outcomes: 0,
        correct_candidate_present: false,
        best_correct_rank: None,
        best_correct_rank_stereo_ignored: None,
        top1_hit: false,
        top5_hit: false,
        top10_hit: false,
        stereochemistry_aware_hit: false,
        stereochemistry_ignored_hit: false,
        invalid_candidate_count: 0,
        no_op_candidate_count: 0,
        application_warning_count: 0,
        application_error_count: 0,
        templates_attempted: 0,
        rules_loaded: 0,
        elapsed_ms: 0.0,
        failure_reason: FailureReason::InputInvalid,
        provenance: provenance.clone(),
    }
}

fn compute_row(
    reaction: &BenchReaction,
    rules: &[RetroRule],
    provenance: &RowProvenance,
) -> BenchRow {
    // The corpus's own reactant text, in the corpus's own order -- NOT
    // `reactants_canonical` (see that field's docs on `BenchReaction`: a
    // pre-canonicalized, pre-sorted rewrite can change candidate
    // stereochemistry spelling even though the underlying molecule is
    // identical, which would make this harness measure its own
    // preprocessing rather than the engine a real `predict` caller invokes).
    let reactant_refs: Vec<&str> = reaction
        .reactants_original
        .iter()
        .map(String::as_str)
        .collect();
    let config = ForwardPredictConfig {
        max_results: usize::MAX,
        ..Default::default()
    };

    let start = std::time::Instant::now();
    let predict_result = predict_products_detailed(&reactant_refs, rules, &config);
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;

    let num_reactants = reaction.reactants_canonical.len();
    let num_products = reaction
        .accepted_products_canonical
        .first()
        .map_or(0, Vec::len);
    let split = split_for_group(&reaction.group_key).to_string();

    let report = match predict_result {
        Ok(r) => r,
        Err(_) => {
            return BenchRow {
                reaction_id: reaction.reaction_id.clone(),
                source_line: reaction.source_line,
                split,
                reaction_class: reaction.reaction_class.clone(),
                reactants_original: reaction.reactants_original.clone(),
                reactants_canonical: reaction.reactants_canonical.clone(),
                accepted_products_canonical: reaction.accepted_products_canonical.clone(),
                num_reactants,
                num_products,
                has_stereochemistry: reaction.has_stereochemistry,
                candidate_count: 0,
                raw_outcomes: 0,
                correct_candidate_present: false,
                best_correct_rank: None,
                best_correct_rank_stereo_ignored: None,
                top1_hit: false,
                top5_hit: false,
                top10_hit: false,
                stereochemistry_aware_hit: false,
                stereochemistry_ignored_hit: false,
                invalid_candidate_count: 0,
                no_op_candidate_count: 0,
                application_warning_count: 0,
                application_error_count: 0,
                templates_attempted: 0,
                rules_loaded: rules.len(),
                elapsed_ms,
                failure_reason: FailureReason::PredictionError,
                provenance: provenance.clone(),
            };
        }
    };

    let accepted_ignored: Vec<Vec<String>> = reaction
        .accepted_products_canonical
        .iter()
        .map(|set| stereo_ignored_set(set))
        .collect();

    let mut best_aware_rank: Option<usize> = None;
    let mut best_ignored_rank: Option<usize> = None;
    for candidate in &report.candidates {
        if best_aware_rank.is_none()
            && reaction
                .accepted_products_canonical
                .contains(&candidate.products)
        {
            best_aware_rank = Some(candidate.rank);
        }
        if best_ignored_rank.is_none() {
            let candidate_ignored = stereo_ignored_set(&candidate.products);
            if accepted_ignored.contains(&candidate_ignored) {
                best_ignored_rank = Some(candidate.rank);
            }
        }
        if best_aware_rank.is_some() && best_ignored_rank.is_some() {
            // Candidates are already rank-ordered ascending; nothing later
            // can improve either rank once both are found.
            break;
        }
    }

    let candidate_count = report.candidates.len();
    let correct_candidate_present = best_aware_rank.is_some();
    let top1_hit = best_aware_rank == Some(0);
    let top5_hit = best_aware_rank.is_some_and(|r| r < 5);
    let top10_hit = best_aware_rank.is_some_and(|r| r < 10);
    let stereochemistry_ignored_hit = best_ignored_rank.is_some_and(|r| r < 10);

    let failure_reason = if top1_hit {
        FailureReason::HitTop1
    } else if top5_hit {
        FailureReason::HitTop5
    } else if top10_hit {
        FailureReason::HitTop10
    } else if correct_candidate_present {
        FailureReason::HitBeyond10
    } else if candidate_count == 0 {
        FailureReason::CorrectAbsentEmptyPool
    } else {
        FailureReason::CorrectAbsentNonemptyPool
    };

    BenchRow {
        reaction_id: reaction.reaction_id.clone(),
        source_line: reaction.source_line,
        split,
        reaction_class: reaction.reaction_class.clone(),
        reactants_original: reaction.reactants_original.clone(),
        reactants_canonical: reaction.reactants_canonical.clone(),
        accepted_products_canonical: reaction.accepted_products_canonical.clone(),
        num_reactants,
        num_products,
        has_stereochemistry: reaction.has_stereochemistry,
        candidate_count,
        raw_outcomes: report.stats.raw_outcomes,
        correct_candidate_present,
        best_correct_rank: best_aware_rank,
        best_correct_rank_stereo_ignored: best_ignored_rank,
        top1_hit,
        top5_hit,
        top10_hit,
        stereochemistry_aware_hit: top10_hit,
        stereochemistry_ignored_hit,
        invalid_candidate_count: report.stats.invalid_outcomes_rejected,
        no_op_candidate_count: report.stats.no_op_outcomes_rejected,
        application_warning_count: report.warnings.len(),
        application_error_count: report.stats.template_application_errors,
        templates_attempted: report.stats.templates_attempted,
        rules_loaded: report.stats.rules_loaded,
        elapsed_ms,
        failure_reason,
        provenance: provenance.clone(),
    }
}

// ---------------------------------------------------------------------
// Aggregate metrics
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize)]
pub struct PercentileStats {
    pub min: f64,
    pub p50: f64,
    pub p90: f64,
    pub p95: f64,
    pub max: f64,
    pub mean: f64,
}

/// Nearest-rank percentile over `values` (mutated in place to sort it).
/// `p` is in `[0, 100]`. `None` for an empty input.
fn percentile_stats(mut values: Vec<f64>) -> Option<PercentileStats> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.total_cmp(b));
    let n = values.len();
    let pct = |p: f64| -> f64 {
        let idx = ((p / 100.0) * (n as f64 - 1.0)).round() as usize;
        values[idx.min(n - 1)]
    };
    let mean = values.iter().sum::<f64>() / n as f64;
    Some(PercentileStats {
        min: values[0],
        p50: pct(50.0),
        p90: pct(90.0),
        p95: pct(95.0),
        max: values[n - 1],
        mean,
    })
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct HitRateMetrics {
    pub n: usize,
    pub top1_hit_rate: Option<f64>,
    pub top5_hit_rate: Option<f64>,
    pub top10_hit_rate: Option<f64>,
    pub mrr: Option<f64>,
    /// Single-relevant-item NDCG@10 (ideal DCG = 1.0, i.e. exactly one
    /// accepted product multiset expected at rank 0).
    ///
    /// ponytail: this does not implement full graded relevance across every
    /// candidate that happens to match one of several accepted outcomes
    /// (`scripts/train_reranker.py`'s NDCG does, since a labeled group can
    /// have several positives). Issue #61 asks for one NDCG@10 number, not
    /// multi-outcome credit assignment, so only the best-ranked accepted
    /// outcome contributes here. Upgrade if multi-outcome credit assignment
    /// becomes a real question.
    pub ndcg_at_10: Option<f64>,
    pub mean_best_correct_rank: Option<f64>,
    pub median_best_correct_rank: Option<f64>,
}

fn hit_rate_metrics(rows: &[&BenchRow], with_rank_summary: bool) -> HitRateMetrics {
    let n = rows.len();
    if n == 0 {
        return HitRateMetrics::default();
    }
    let n_f = n as f64;
    let top1_hit_rate = rows.iter().filter(|r| r.top1_hit).count() as f64 / n_f;
    let top5_hit_rate = rows.iter().filter(|r| r.top5_hit).count() as f64 / n_f;
    let top10_hit_rate = rows.iter().filter(|r| r.top10_hit).count() as f64 / n_f;
    let mrr = rows
        .iter()
        .map(|r| {
            r.best_correct_rank
                .map_or(0.0, |rank| 1.0 / (rank as f64 + 1.0))
        })
        .sum::<f64>()
        / n_f;
    let ndcg_at_10 = rows
        .iter()
        .map(|r| match r.best_correct_rank {
            Some(rank) if rank < 10 => 1.0 / (rank as f64 + 2.0).log2(),
            _ => 0.0,
        })
        .sum::<f64>()
        / n_f;

    let (mean_best_correct_rank, median_best_correct_rank) = if with_rank_summary {
        let mut ranks: Vec<f64> = rows
            .iter()
            .filter_map(|r| r.best_correct_rank)
            .map(|r| r as f64)
            .collect();
        if ranks.is_empty() {
            (None, None)
        } else {
            ranks.sort_by(|a, b| a.total_cmp(b));
            let mean = ranks.iter().sum::<f64>() / ranks.len() as f64;
            let mid = ranks.len() / 2;
            let median = if ranks.len().is_multiple_of(2) {
                (ranks[mid - 1] + ranks[mid]) / 2.0
            } else {
                ranks[mid]
            };
            (Some(mean), Some(median))
        }
    } else {
        (None, None)
    };

    HitRateMetrics {
        n,
        top1_hit_rate: Some(top1_hit_rate),
        top5_hit_rate: Some(top5_hit_rate),
        top10_hit_rate: Some(top10_hit_rate),
        mrr: Some(mrr),
        ndcg_at_10: Some(ndcg_at_10),
        mean_best_correct_rank,
        median_best_correct_rank,
    }
}

/// Two denominators over the SAME rows, per Phase 1: "Conditional and
/// end-to-end metrics must never be conflated."
///
/// - `conditional`: only rows with a correct candidate somewhere in their
///   own pool -- "given that ranking is possible at all, how good is it".
/// - `end_to_end`: every row that received a real candidate pool (excludes
///   [`FailureReason::InputInvalid`]/[`FailureReason::PredictionError`]
///   non-attempts, which `valid_input_rate` already accounts for
///   separately); a coverage miss contributes 0, it is never excluded.
#[derive(Debug, Clone, Default, Serialize)]
pub struct BenchAggregate {
    pub n_total_rows: usize,
    pub n_valid_rows: usize,
    pub valid_input_rate: Option<f64>,
    pub proposal_coverage: Option<f64>,
    pub conditional: HitRateMetrics,
    pub end_to_end: HitRateMetrics,
    /// `sum(invalid_candidate_count) / sum(raw_outcomes)` over valid rows.
    pub invalid_product_rate: Option<f64>,
    /// `sum(no_op_candidate_count) / sum(raw_outcomes)` over valid rows.
    pub no_op_rate: Option<f64>,
    pub candidate_count_distribution: Option<PercentileStats>,
    pub latency_ms: Option<PercentileStats>,
}

fn aggregate(rows: &[&BenchRow]) -> BenchAggregate {
    let n_total_rows = rows.len();
    let valid_rows: Vec<&BenchRow> = rows
        .iter()
        .copied()
        .filter(|r| !r.failure_reason.is_non_attempt())
        .collect();
    let n_valid_rows = valid_rows.len();
    let valid_input_rate = (n_total_rows > 0).then(|| n_valid_rows as f64 / n_total_rows as f64);

    let with_positive: Vec<&BenchRow> = valid_rows
        .iter()
        .copied()
        .filter(|r| r.correct_candidate_present)
        .collect();
    let proposal_coverage =
        (n_valid_rows > 0).then(|| with_positive.len() as f64 / n_valid_rows as f64);

    let conditional = hit_rate_metrics(&with_positive, true);
    let end_to_end = hit_rate_metrics(&valid_rows, false);

    let raw_outcomes_total: usize = valid_rows.iter().map(|r| r.raw_outcomes).sum();
    let invalid_total: usize = valid_rows.iter().map(|r| r.invalid_candidate_count).sum();
    let no_op_total: usize = valid_rows.iter().map(|r| r.no_op_candidate_count).sum();
    let invalid_product_rate =
        (raw_outcomes_total > 0).then(|| invalid_total as f64 / raw_outcomes_total as f64);
    let no_op_rate =
        (raw_outcomes_total > 0).then(|| no_op_total as f64 / raw_outcomes_total as f64);

    let candidate_count_distribution = percentile_stats(
        valid_rows
            .iter()
            .map(|r| r.candidate_count as f64)
            .collect(),
    );
    let latency_ms = percentile_stats(valid_rows.iter().map(|r| r.elapsed_ms).collect());

    BenchAggregate {
        n_total_rows,
        n_valid_rows,
        valid_input_rate,
        proposal_coverage,
        conditional,
        end_to_end,
        invalid_product_rate,
        no_op_rate,
        candidate_count_distribution,
        latency_ms,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BenchBreakdownBucket {
    pub bucket: String,
    pub aggregate: BenchAggregate,
}

fn breakdown_by<'a>(
    rows: impl Iterator<Item = &'a BenchRow>,
    key_fn: impl Fn(&BenchRow) -> String,
) -> Vec<BenchBreakdownBucket> {
    let mut groups: BTreeMap<String, Vec<&BenchRow>> = BTreeMap::new();
    for row in rows {
        groups.entry(key_fn(row)).or_default().push(row);
    }
    groups
        .into_iter()
        .map(|(bucket, rows)| BenchBreakdownBucket {
            aggregate: aggregate(&rows),
            bucket,
        })
        .collect()
}

fn pool_size_bucket(n: usize) -> String {
    match n {
        0 => "0",
        1..=5 => "1-5",
        6..=20 => "6-20",
        21..=50 => "21-50",
        _ => "51+",
    }
    .to_string()
}

// ---------------------------------------------------------------------
// Top-level report
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct RunProvenance {
    pub renkin_forward_version: String,
    pub template_source: String,
    pub rules_file_sha256: Option<String>,
    pub rules_loaded: usize,
    pub corpus_path: String,
    pub corpus_sha256: String,
    pub train_max_bucket: u32,
    pub val_max_bucket: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct BenchReport {
    pub schema_version: u32,
    pub provenance: RunProvenance,
    pub corpus_stats: CorpusLoadStats,
    pub corpus_warnings: Vec<CorpusLoadWarning>,
    /// All rows, all splits combined.
    pub overall: BenchAggregate,
    /// Keyed `"train"`/`"val"`/`"test"` -- a row with `split == "unknown"`
    /// (an invalid-input row whose group key couldn't be computed) is
    /// counted in `overall` but not in any of these three.
    pub by_split: BTreeMap<String, BenchAggregate>,
    /// Keyed by dimension name: `reaction_class`, `num_reactants`,
    /// `num_products`, `stereochemistry_presence`, `template_source`,
    /// `candidate_pool_size`, `failure_reason`.
    pub breakdowns: BTreeMap<String, Vec<BenchBreakdownBucket>>,
}

pub struct BenchOutcome {
    pub rows: Vec<BenchRow>,
    pub report: BenchReport,
}

/// Runs the full Phase 1 harness end to end: load + canonicalize + dedupe
/// the corpus, load exactly one rule set for `template_source`, predict and
/// score every reaction, then aggregate. Never panics on malformed corpus
/// content -- only a missing/unreadable corpus file, a missing/invalid
/// `--templates` file, or an invalid `template_source` name is a hard error.
pub fn run_benchmark(
    corpus_path: &str,
    template_source: TemplateSource,
    templates_path: Option<&str>,
) -> Result<BenchOutcome> {
    let (reactions, invalid_attempts, corpus_stats, corpus_warnings) = load_corpus(corpus_path)?;
    let corpus_sha256 = sha256_hex_of_file(corpus_path)?;
    let (rules, rules_file_sha256) = load_rules_for_source(template_source, templates_path)?;

    let row_provenance = RowProvenance {
        renkin_forward_version: env!("CARGO_PKG_VERSION").to_string(),
        template_source: template_source.as_str().to_string(),
        rules_file_sha256: rules_file_sha256.clone(),
    };

    let mut rows: Vec<BenchRow> = Vec::with_capacity(reactions.len() + invalid_attempts.len());
    for reaction in &reactions {
        rows.push(compute_row(reaction, &rules, &row_provenance));
    }
    for attempt in &invalid_attempts {
        rows.push(invalid_row(attempt, &row_provenance));
    }
    // Deterministic row order independent of the two loops above: by
    // source_line, so output never depends on internal push order.
    rows.sort_by_key(|r| r.source_line);

    let run_provenance = RunProvenance {
        renkin_forward_version: env!("CARGO_PKG_VERSION").to_string(),
        template_source: template_source.as_str().to_string(),
        rules_file_sha256,
        rules_loaded: rules.len(),
        corpus_path: corpus_path.to_string(),
        corpus_sha256,
        train_max_bucket: TRAIN_MAX_BUCKET,
        val_max_bucket: VAL_MAX_BUCKET,
    };

    let all_refs: Vec<&BenchRow> = rows.iter().collect();
    let overall = aggregate(&all_refs);

    let mut by_split: BTreeMap<String, BenchAggregate> = BTreeMap::new();
    for split in ["train", "val", "test"] {
        let split_rows: Vec<&BenchRow> = rows.iter().filter(|r| r.split == split).collect();
        by_split.insert(split.to_string(), aggregate(&split_rows));
    }

    let mut breakdowns: BTreeMap<String, Vec<BenchBreakdownBucket>> = BTreeMap::new();
    breakdowns.insert(
        "reaction_class".to_string(),
        breakdown_by(rows.iter(), |r| {
            r.reaction_class
                .clone()
                .unwrap_or_else(|| "unknown".to_string())
        }),
    );
    breakdowns.insert(
        "num_reactants".to_string(),
        breakdown_by(rows.iter(), |r| r.num_reactants.to_string()),
    );
    breakdowns.insert(
        "num_products".to_string(),
        breakdown_by(rows.iter(), |r| r.num_products.to_string()),
    );
    breakdowns.insert(
        "stereochemistry_presence".to_string(),
        breakdown_by(rows.iter(), |r| {
            if r.has_stereochemistry {
                "present"
            } else {
                "absent"
            }
            .to_string()
        }),
    );
    breakdowns.insert(
        "template_source".to_string(),
        breakdown_by(rows.iter(), |r| r.provenance.template_source.clone()),
    );
    breakdowns.insert(
        "failure_reason".to_string(),
        breakdown_by(rows.iter(), |r| r.failure_reason.as_str().to_string()),
    );
    breakdowns.insert(
        "candidate_pool_size".to_string(),
        breakdown_by(
            rows.iter().filter(|r| !r.failure_reason.is_non_attempt()),
            |r| pool_size_bucket(r.candidate_count),
        ),
    );

    let report = BenchReport {
        schema_version: FORWARD_BENCH_REPORT_SCHEMA_VERSION,
        provenance: run_provenance,
        corpus_stats,
        corpus_warnings,
        overall,
        by_split,
        breakdowns,
    };

    Ok(BenchOutcome { rows, report })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_bucket_is_deterministic_and_in_range() {
        let a = split_bucket("some-group-key");
        let b = split_bucket("some-group-key");
        assert_eq!(a, b);
        assert!(a < 100);
        assert_ne!(split_bucket("a"), split_bucket("b").wrapping_add(1000)); // sanity: not a constant fn
    }

    #[test]
    fn split_for_group_respects_cutoffs() {
        // Construct keys that hash into each bucket range by brute force
        // over a small deterministic search space -- avoids hardcoding a
        // magic string per split while staying fully deterministic.
        let mut seen = HashSet::new();
        for i in 0..2000u32 {
            let key = format!("probe-{i}");
            let split = split_for_group(&key);
            seen.insert(split);
            if seen.len() == 3 {
                break;
            }
        }
        assert_eq!(seen, HashSet::from(["train", "val", "test"]));
    }

    #[test]
    fn stereo_ignored_canonical_collapses_tetrahedral_and_ez() {
        let achiral_amino_acid = try_canonicalize("CC(N)C(=O)O").unwrap();
        for stereo in ["C[C@H](N)C(=O)O", "C[C@@H](N)C(=O)O"] {
            let canon = try_canonicalize(stereo).unwrap();
            assert_eq!(stereo_ignored_canonical(&canon), achiral_amino_acid);
        }

        let achiral_butene = try_canonicalize("CC=CC").unwrap();
        for stereo in ["C/C=C/C", "C/C=C\\C"] {
            let canon = try_canonicalize(stereo).unwrap();
            assert_eq!(stereo_ignored_canonical(&canon), achiral_butene);
        }
    }

    #[test]
    fn failure_reason_serde_output_matches_as_str_for_every_variant() {
        // Regression guard: `BenchRow::failure_reason` is serialized via
        // `#[derive(Serialize)]` directly, while breakdown bucket keys use
        // `as_str()` -- these two must never disagree (see `HitBeyond10`'s
        // explicit `#[serde(rename)]`).
        let all = [
            FailureReason::HitTop1,
            FailureReason::HitTop5,
            FailureReason::HitTop10,
            FailureReason::HitBeyond10,
            FailureReason::CorrectAbsentEmptyPool,
            FailureReason::CorrectAbsentNonemptyPool,
            FailureReason::InputInvalid,
            FailureReason::PredictionError,
        ];
        for reason in all {
            let serialized = serde_json::to_value(reason).unwrap();
            assert_eq!(
                serialized.as_str().unwrap(),
                reason.as_str(),
                "mismatch for {reason:?}"
            );
        }
    }

    #[test]
    fn failure_reason_round_trips_through_as_str() {
        let all = [
            FailureReason::HitTop1,
            FailureReason::HitTop5,
            FailureReason::HitTop10,
            FailureReason::HitBeyond10,
            FailureReason::CorrectAbsentEmptyPool,
            FailureReason::CorrectAbsentNonemptyPool,
            FailureReason::InputInvalid,
            FailureReason::PredictionError,
        ];
        let mut seen = HashSet::new();
        for reason in all {
            assert!(
                seen.insert(reason.as_str()),
                "duplicate as_str for {reason:?}"
            );
        }
    }

    #[test]
    fn template_source_parse_rejects_scorer_conditioned_and_unknown() {
        assert!(TemplateSource::parse("embedded").is_ok());
        assert!(TemplateSource::parse("file").is_ok());
        assert!(TemplateSource::parse("train-extracted").is_ok());
        assert!(TemplateSource::parse("scorer-conditioned").is_err());
        assert!(TemplateSource::parse("bogus").is_err());
    }

    #[test]
    fn load_rules_for_source_embedded_rejects_stray_templates_path() {
        let err =
            load_rules_for_source(TemplateSource::Embedded, Some("some/path.smi")).unwrap_err();
        assert!(err.to_string().contains("embedded"));
    }

    #[test]
    fn load_rules_for_source_file_requires_path() {
        let err = load_rules_for_source(TemplateSource::File, None).unwrap_err();
        assert!(err.to_string().contains("--templates"));
    }

    #[test]
    fn load_corpus_counts_every_rejection_category_and_dedupes() {
        let dir =
            std::env::temp_dir().join(format!("renkin-forward-bench-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("corpus.jsonl");
        let content = r#"
not json at all
{"schema_version": 99, "reaction_id": "bad-version", "reactants": ["CCO"], "accepted_products": [["CC=O"]]}
{"schema_version": 1, "reaction_id": "empty", "reactants": [], "accepted_products": [["CC=O"]]}
{"schema_version": 1, "reaction_id": "bad-smiles", "reactants": ["not_a_smiles("], "accepted_products": [["CC=O"]]}
{"schema_version": 1, "reaction_id": "ok-1", "reactants": ["CCO"], "accepted_products": [["CC=O"]]}
{"schema_version": 1, "reaction_id": "ok-1-dup", "reactants": ["CCO"], "accepted_products": [["CC=O"]]}
"#;
        std::fs::write(&path, content).unwrap();

        let (reactions, invalid, stats, _warnings) = load_corpus(path.to_str().unwrap()).unwrap();
        assert_eq!(stats.malformed_json, 1);
        assert_eq!(stats.wrong_schema_version, 1);
        assert_eq!(stats.empty_reactants_or_products, 1);
        assert_eq!(stats.unparseable_smiles, 1);
        assert_eq!(stats.duplicate_records_merged, 1);
        assert_eq!(reactions.len(), 1);
        assert_eq!(invalid.len(), 3);
        assert_eq!(stats.reactions_loaded, 1);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn compute_row_predicts_from_reactants_original_not_reactants_canonical() {
        // Regression guard: compute_row must feed `reactants_original` (the
        // corpus's own text/order, matching what a direct `predict` CLI
        // call would receive) to `predict_products_detailed`, never
        // `reactants_canonical` (a pre-canonicalized, pre-sorted rewrite).
        // Feeding the rewrite silently changed which candidate
        // stereochemistry-tag spelling was produced for this exact
        // reaction -- which would make `stereochemistry_aware_hit` an
        // artifact of this harness's own preprocessing rather than a fact
        // about the engine. See docs/guides/forward-benchmark.md's
        // "Stereochemistry comparison" / "Fixture corpus" sections.
        let rules = default_rules();
        let provenance = RowProvenance {
            renkin_forward_version: "test".to_string(),
            template_source: "embedded".to_string(),
            rules_file_sha256: None,
        };

        // `reactants_canonical` is deliberately a DIFFERENT (but chemically
        // equivalent) representation/order than `reactants_original`, so
        // a regression back to using it would change the result below.
        let reaction = BenchReaction {
            reaction_id: "stereo-regression".to_string(),
            source_line: 1,
            reactants_original: vec!["C[C@H](N)C(=O)O".to_string(), "CCO".to_string()],
            reactants_canonical: vec!["C(C)O".to_string(), "[C@@H](C(O)=O)(C)N".to_string()],
            accepted_products_canonical: vec![vec!["C([C@H](C(=O)O)N)OCC".to_string()]],
            reaction_class: None,
            group_key: "g".to_string(),
            has_stereochemistry: true,
        };

        let row = compute_row(&reaction, &rules, &provenance);
        assert_eq!(row.best_correct_rank, Some(5));
    }

    #[test]
    fn aggregate_computes_conditional_and_end_to_end_separately() {
        let provenance = RowProvenance {
            renkin_forward_version: "test".to_string(),
            template_source: "embedded".to_string(),
            rules_file_sha256: None,
        };
        let mk = |rank: Option<usize>, candidate_count: usize| BenchRow {
            reaction_id: "r".to_string(),
            source_line: 1,
            split: "train".to_string(),
            reaction_class: None,
            reactants_original: vec![],
            reactants_canonical: vec![],
            accepted_products_canonical: vec![],
            num_reactants: 1,
            num_products: 1,
            has_stereochemistry: false,
            candidate_count,
            raw_outcomes: candidate_count,
            correct_candidate_present: rank.is_some(),
            best_correct_rank: rank,
            best_correct_rank_stereo_ignored: rank,
            top1_hit: rank == Some(0),
            top5_hit: rank.is_some_and(|r| r < 5),
            top10_hit: rank.is_some_and(|r| r < 10),
            stereochemistry_aware_hit: rank.is_some_and(|r| r < 10),
            stereochemistry_ignored_hit: rank.is_some_and(|r| r < 10),
            invalid_candidate_count: 0,
            no_op_candidate_count: 0,
            application_warning_count: 0,
            application_error_count: 0,
            templates_attempted: 1,
            rules_loaded: 1,
            elapsed_ms: 1.0,
            failure_reason: match rank {
                Some(0) => FailureReason::HitTop1,
                Some(_) => FailureReason::HitBeyond10,
                None if candidate_count == 0 => FailureReason::CorrectAbsentEmptyPool,
                None => FailureReason::CorrectAbsentNonemptyPool,
            },
            provenance: provenance.clone(),
        };
        // 2 hits at rank 0, 1 miss with a nonempty pool, 1 miss with an
        // empty pool -- 4 rows total, 3 with a correct candidate present...
        // wait: only rank Some(_) counts as present. Build explicitly:
        let rows_owned = [mk(Some(0), 3), mk(Some(0), 5), mk(None, 4), mk(None, 0)];
        let rows: Vec<&BenchRow> = rows_owned.iter().collect();

        let agg = aggregate(&rows);
        assert_eq!(agg.n_total_rows, 4);
        assert_eq!(agg.n_valid_rows, 4);
        assert_eq!(agg.proposal_coverage, Some(0.5)); // 2 of 4 have a correct candidate present
        assert_eq!(agg.conditional.n, 2);
        assert_eq!(agg.conditional.top1_hit_rate, Some(1.0)); // both present rows are rank 0
        assert_eq!(agg.end_to_end.n, 4);
        assert_eq!(agg.end_to_end.top1_hit_rate, Some(0.5)); // 2 of 4 rows overall
    }
}
