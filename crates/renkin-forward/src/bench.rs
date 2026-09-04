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

use std::collections::{BTreeMap, HashMap};

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
/// Schema version of [`BenchRow`]/[`BenchReport`]. Bumped to 2:
/// `stereochemistry_ignored_hit: bool` became
/// `stereochemistry_ignored_outcome: StereoIgnoredOutcome` (an explicit
/// `Unsupported` state replaces a silent bool-`false` for the case where
/// the comparison couldn't be computed at all -- see that type's docs).
pub const FORWARD_BENCH_REPORT_SCHEMA_VERSION: u32 = 2;
/// Schema version of the compact benchmark contract manifest emitted beside
/// a report. This is intentionally separate from the larger report schema.
pub const FORWARD_BENCH_MANIFEST_SCHEMA_VERSION: u32 = 1;

/// Deterministic split-bucket cutoffs: buckets `[0, TRAIN_MAX_BUCKET)` ->
/// train, `[TRAIN_MAX_BUCKET, VAL_MAX_BUCKET)` -> val, the rest -> test.
/// SAME values as `scripts/train_reranker.py`'s `TRAIN_MAX_BUCKET`/
/// `VAL_MAX_BUCKET` (retrosynthesis reranker, PR #59) -- kept numerically
/// identical for cross-benchmark consistency, not because the two harnesses
/// share any code.
pub const TRAIN_MAX_BUCKET: u32 = 70;
pub const VAL_MAX_BUCKET: u32 = 85;

/// Version of the split ALGORITHM itself (bucket cutoffs + hash scheme
/// above), not of the schema. Bump only if `split_bucket`/`split_for_group`
/// or the cutoffs change -- a `--template-manifest` (once it exists, see
/// `TemplateSource::TrainExtracted`) can then assert it was built against a
/// compatible split protocol.
pub const SPLIT_PROTOCOL_VERSION: u32 = 1;

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
    /// Two rows sharing a reaction identity each supplied a different
    /// non-empty explicit `group_key` -- an unresolvable corpus data-quality
    /// problem: neither value can be trusted over the other, so guessing
    /// (even deterministically) would silently risk a leakage-safety
    /// violation. The affected reaction is REJECTED (moved out of the
    /// returned reactions into an `InvalidReactionAttempt` with reason
    /// `"conflicting_group_key"`), never kept under either candidate key.
    /// See the `conflicting_explicit_group_key` warning for both values.
    pub conflicting_group_keys: usize,
    /// The same corpus `reaction_id` was used for two rows that canonicalize
    /// to a DIFFERENT reaction identity (different reactants and/or accepted
    /// products) -- a corpus integrity problem distinct from the identity-
    /// level duplicate-merge above. The later row is rejected (`reason`
    /// `"conflicting_reaction_id"`), never silently accepted as if
    /// `reaction_id` were a reliable per-reaction key (it is documented as
    /// NOT used for identity/splitting precisely because corpora can't be
    /// trusted to keep it unique).
    pub conflicting_reaction_ids: usize,
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
    renkin::sha256_hex(hasher.finalize())
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
    renkin::sha256_hex(hasher.finalize())
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

/// Clears every atom's reaction atom-map number on a parsed molecule,
/// rebuilt via the public [`chematic::core::MoleculeBuilder`] API (there is
/// no in-place mutator for `Atom::atom_map` on `Molecule` itself). Atom and
/// bond order is preserved exactly (same indices in, same indices out), so
/// the `copy_*_from` convenience methods can carry over stereo groups,
/// stereo neighbor order, and bond directions verbatim -- this must NOT be
/// done by editing the canonical SMILES *text*: `canonical_smiles` decides
/// bracket-vs-organic-subset notation based on whether an atom needs
/// brackets at canonicalization time (atom_map present being one such
/// reason), so stripping only the `:<digits>` substring from already-
/// generated text leaves a redundant, non-canonical `[C]`/`[OH]`-style
/// bracket behind instead of collapsing to `C`/`O` -- silently producing a
/// DIFFERENT canonical string than the same molecule parsed without atom
/// maps in the first place (verified empirically while diagnosing this).
///
/// Without this, two corpus rows encoding the literal same reaction but
/// with different atom-map numbering would get different
/// `reaction_identity_hash`/`group_key` values, defeating dedup and
/// leakage-safe splitting.
fn clear_atom_maps(mol: &chematic::core::Molecule) -> chematic::core::Molecule {
    let mut builder = chematic::core::MoleculeBuilder::new();
    for (_, atom) in mol.atoms() {
        let mut a = atom.clone();
        a.atom_map = None;
        builder.add_atom(a);
    }
    for (_, bond) in mol.bonds() {
        let _ = builder.add_bond(bond.atom1, bond.atom2, bond.order);
    }
    builder.copy_stereo_groups_from(mol);
    builder.copy_stereo_from(mol);
    builder.copy_bond_directions_from(mol);
    builder.build()
}

/// Canonicalizes one SMILES, returning `None` (not an error) on failure --
/// callers decide how a single bad SMILES affects the enclosing record.
/// Clears any reaction atom-map annotation before canonicalizing (see
/// [`clear_atom_maps`]) -- this is corpus-side canonicalization only, used
/// for reaction identity, group-key derivation, and comparison against real
/// (always map-free) candidate products, never for `reactants_original`
/// (what's actually fed to `predict_products_detailed`).
fn try_canonicalize(smiles: &str) -> Option<String> {
    mol_from_smiles(smiles)
        .ok()
        .map(|mol| canonical_smiles(&clear_atom_maps(&mol)))
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
    let mut seen_identity: HashMap<String, usize> = HashMap::new();
    // Tracks, per reaction identity, the explicit (non-empty, corpus-
    // supplied) `group_key` seen so far, if any -- so a LATER duplicate row
    // that happens to carry the explicit key isn't silently ignored just
    // because an EARLIER, bare duplicate happened to be dedup-retained
    // first (see `duplicate_group_key_independent_of_row_order`).
    let mut explicit_group_key: HashMap<String, String> = HashMap::new();
    // Tracks the first-seen reaction identity for each corpus `reaction_id`
    // -- `reaction_id` is documented as not used for identity/splitting
    // precisely because corpora can't be trusted to keep it unique; a LATER
    // row reusing an already-seen `reaction_id` for a DIFFERENT identity is
    // a corpus integrity problem, rejected rather than silently accepted.
    let mut reaction_id_identity: HashMap<String, String> = HashMap::new();
    // Identities whose duplicate rows carried conflicting explicit
    // `group_key` values -- collected during the loop, then the whole
    // reaction is excluded from the returned `reactions` in a final pass
    // (see after the loop): neither candidate key can be trusted.
    let mut conflicted_identities: std::collections::HashSet<String> =
        std::collections::HashSet::new();

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
        // Two accepted-product entries that canonicalize to the identical
        // multiset (e.g. the same answer listed twice in the corpus, or two
        // spellings that happen to converge) must not count as two distinct
        // ground-truth outcomes -- that would inflate NDCG@10's ideal DCG
        // and double-count `accepted_product_count_min`/`_max`'s source.
        // Dedup the OUTER list only; multiplicity WITHIN one accepted
        // product multiset is untouched (each entry is already its own
        // sorted `Vec<String>`).
        accepted_products_canonical.dedup();

        let identity = reaction_identity_hash(&reactants_canonical, &accepted_products_canonical);
        let row_explicit_key = row.group_key.filter(|k| !k.is_empty());

        // `reaction_id` reused for a genuinely different reaction (different
        // reactants and/or accepted products) is a corpus integrity problem
        // -- reject this row rather than silently accepting a non-unique ID.
        if let Some(prior_identity) = reaction_id_identity.get(&row.reaction_id) {
            if *prior_identity != identity {
                stats.conflicting_reaction_ids += 1;
                push_warning(
                    &mut stats,
                    &mut warnings,
                    CorpusLoadWarning {
                        line_number,
                        code: "conflicting_reaction_id".to_string(),
                        message: format!(
                            "reaction_id {:?} was already used for a different reaction \
                             (different reactants/accepted products); rejecting this row",
                            row.reaction_id
                        ),
                    },
                );
                invalid.push(InvalidReactionAttempt {
                    reaction_id: row.reaction_id,
                    source_line: line_number,
                    reason: "conflicting_reaction_id".to_string(),
                });
                continue;
            }
        } else {
            reaction_id_identity.insert(row.reaction_id.clone(), identity.clone());
        }

        if let Some(&existing_idx) = seen_identity.get(&identity) {
            stats.duplicate_records_merged += 1;
            if let Some(new_key) = row_explicit_key {
                match explicit_group_key.get(&identity) {
                    Some(kept) if *kept != new_key => {
                        stats.conflicting_group_keys += 1;
                        conflicted_identities.insert(identity.clone());
                        push_warning(
                            &mut stats,
                            &mut warnings,
                            CorpusLoadWarning {
                                line_number,
                                code: "conflicting_explicit_group_key".to_string(),
                                message: format!(
                                    "reaction {:?} has conflicting explicit group_key values \
                                     across duplicate rows ({kept:?} vs {new_key:?}); rejecting \
                                     the whole reaction rather than guessing which is correct",
                                    reactions[existing_idx].reaction_id
                                ),
                            },
                        );
                    }
                    Some(_) => {}
                    None => {
                        // First explicit key seen for this identity: promote
                        // it onto the already-retained reaction, regardless
                        // of which occurrence happened to be dedup-retained.
                        explicit_group_key.insert(identity.clone(), new_key.clone());
                        reactions[existing_idx].group_key = new_key;
                    }
                }
            }
            continue;
        }

        let group_key = match row_explicit_key.clone() {
            Some(k) => {
                explicit_group_key.insert(identity.clone(), k.clone());
                k
            }
            None => fallback_group_key(&reactants_canonical),
        };
        let has_stereochemistry = reactants_canonical
            .iter()
            .chain(accepted_products_canonical.iter().flatten())
            .any(|s| s.contains('@') || s.contains('/') || s.contains('\\'));

        seen_identity.insert(identity, reactions.len());
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

    // Reject every reaction whose duplicate rows carried conflicting
    // explicit `group_key` values, rather than silently keeping whichever
    // key happened to be seen first -- moved out of `reactions` into
    // `invalid` here, once identity->index lookups from `seen_identity` are
    // stable (no more insertions happen after the loop above).
    if !conflicted_identities.is_empty() {
        let conflicted_indices: std::collections::HashSet<usize> = conflicted_identities
            .iter()
            .filter_map(|identity| seen_identity.get(identity).copied())
            .collect();
        let mut kept = Vec::with_capacity(reactions.len());
        for (i, reaction) in reactions.into_iter().enumerate() {
            if conflicted_indices.contains(&i) {
                invalid.push(InvalidReactionAttempt {
                    reaction_id: reaction.reaction_id,
                    source_line: reaction.source_line,
                    reason: "conflicting_group_key".to_string(),
                });
            } else {
                kept.push(reaction);
            }
        }
        reactions = kept;
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
        TemplateSource::File => {
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
        // ponytail: a manifest (templates_sha256 + source_corpus_sha256 +
        // split_protocol_version + included_split == "train", hard-
        // validated here) is the real unblock condition -- add
        // --template-manifest <path> and validate it before loading, then
        // this arm can load the file same as File mode above.
        TemplateSource::TrainExtracted => {
            bail!(
                "--template-source train-extracted is not usable yet: this harness has no way \
                 to verify that a --templates file was actually extracted from the train split \
                 only -- it would load the file exactly like --template-source file and merely \
                 stamp a different provenance label, which is a label, not a verified guarantee. \
                 Use --template-source file if you accept responsibility for that split \
                 boundary yourself (and inspect the per-split metric breakdown for suspiciously \
                 strong val/test results, which would be the only signal of a mislabeled file). \
                 A future version will accept --template-manifest <path> attesting \
                 {{templates_sha256, source_corpus_sha256, split_protocol_version, \
                 included_split: \"train\"}} and hard-validate it before loading."
            );
        }
    }
}

// ---------------------------------------------------------------------
// Stereochemistry-ignored comparison
// ---------------------------------------------------------------------

/// Structurally clears chirality and E/Z bond-direction stereo information
/// from a molecule, rebuilt via the public `chematic::core::MoleculeBuilder`
/// API (mirrors [`clear_atom_maps`]). Every atom's `chirality` is reset to
/// `Chirality::None`; every directional (`Up`/`Down`, i.e. `/`/`\`) bond is
/// normalized to `Single`. Unlike `clear_atom_maps`, enhanced stereo groups,
/// the SMILES stereo-neighbor order, and the auxiliary bond-direction map
/// are deliberately NOT copied over from the source molecule -- carrying
/// any of those forward would silently reintroduce the stereochemistry this
/// function exists to remove.
fn clear_stereochemistry(mol: &chematic::core::Molecule) -> chematic::core::Molecule {
    let mut builder = chematic::core::MoleculeBuilder::new();
    for (_, atom) in mol.atoms() {
        let mut a = atom.clone();
        a.chirality = chematic::core::Chirality::None;
        builder.add_atom(a);
    }
    for (_, bond) in mol.bonds() {
        let order = match bond.order {
            chematic::core::BondOrder::Up | chematic::core::BondOrder::Down => {
                chematic::core::BondOrder::Single
            }
            other => other,
        };
        let _ = builder.add_bond(bond.atom1, bond.atom2, order);
    }
    builder.build()
}

/// Best-effort stereo-flattened canonical form used only for the
/// "stereochemistry-ignored" comparison dimension. Structural (via
/// [`clear_stereochemistry`]), not a text strip-then-reparse -- verified
/// empirically (see this module's tests) to collapse both tetrahedral
/// (`@`/`@@`) and double-bond (`/`/`\`) stereo markers to the same
/// canonical form as the equivalent achiral input.
///
/// `s` is always an already-canonical SMILES string produced earlier in
/// this module, so re-parsing it should never fail in practice; `None` is
/// still returned rather than assumed-impossible, so a caller can mark the
/// whole comparison [`StereoIgnoredOutcome::Unsupported`] instead of
/// silently falling back to the stereo-aware value on the one input that
/// somehow doesn't re-parse.
fn stereo_ignored_canonical(s: &str) -> Option<String> {
    let mol = mol_from_smiles(s).ok()?;
    Some(canonical_smiles(&clear_stereochemistry(&mol)))
}

/// Stereo-ignored form of a whole accepted-product (or candidate-product)
/// multiset, sorted. `None` (not a silent fallback to the stereo-aware
/// form) if any member fails to convert.
fn stereo_ignored_set(products: &[String]) -> Option<Vec<String>> {
    let mut out = Vec::with_capacity(products.len());
    for s in products {
        out.push(stereo_ignored_canonical(s)?);
    }
    out.sort_unstable();
    Some(out)
}

/// Outcome of the stereochemistry-ignored comparison dimension for one row.
/// A dedicated tri-state, not a `bool`: collapsing "no hit" and "couldn't
/// compute this at all" into the same `false` would silently misreport an
/// unsupported case as a clean miss.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StereoIgnoredOutcome {
    /// A correct candidate was found within the top-10 under the
    /// stereochemistry-ignored comparison.
    Hit,
    /// The comparison was computed successfully; no correct candidate was
    /// found within the top-10.
    NoHit,
    /// At least one accepted-product or candidate-product SMILES for this
    /// reaction could not be converted by [`stereo_ignored_canonical`] --
    /// never silently reported as a hit or a miss.
    Unsupported,
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

/// Whether a row's input (reactants/accepted products) canonicalized at all.
/// Orthogonal to [`ProposalStatus`]/[`RankingStatus`]/[`StereoStatus`] below:
/// this alone answers "did we even attempt prediction", the others answer
/// "how did the attempt go".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputStatus {
    Valid,
    Invalid,
}

impl InputStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Valid => "valid",
            Self::Invalid => "invalid",
        }
    }
}

/// Whether a correct candidate was present in the pool -- independent of
/// where it ranked (see [`RankingStatus`]) and independent of stereochemistry
/// (see [`StereoStatus`]). Unlike [`FailureReason`] (which conflates "did we
/// find it" with "how well ranked" into one flat enum), this and
/// `RankingStatus` are orthogonal: every `Covered` row has a concrete
/// `RankingStatus`, every non-`Covered` row has `RankingStatus::NotApplicable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    /// A correct candidate is present somewhere in the pool.
    Covered,
    /// No correct candidate, and the pool itself was empty.
    MissedEmptyPool,
    /// No correct candidate found, but the pool was non-empty.
    MissedNonemptyPool,
    /// The candidate pool was truncated (`ForwardStats::truncated`) before a
    /// correct candidate was confirmed absent -- absence can't be asserted
    /// when candidates past the cap were never examined. Wired for
    /// correctness but unreachable with this harness's own predict config
    /// (`max_results: usize::MAX` never truncates); kept as a real state
    /// rather than silently folded into `MissedNonemptyPool`, since a future
    /// caller that lowers `max_results` would otherwise get a silently wrong
    /// "confirmed absent" claim.
    CappedUnknown,
    /// The prediction call itself failed (see [`FailureReason::PredictionError`]).
    Error,
    /// Prediction was never attempted (input invalid -- see [`InputStatus::Invalid`]).
    NotAttempted,
}

impl ProposalStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Covered => "covered",
            Self::MissedEmptyPool => "missed_empty_pool",
            Self::MissedNonemptyPool => "missed_nonempty_pool",
            Self::CappedUnknown => "capped_unknown",
            Self::Error => "error",
            Self::NotAttempted => "not_attempted",
        }
    }
}

fn proposal_status_for(
    correct_candidate_present: bool,
    candidate_count: usize,
    truncated: bool,
) -> ProposalStatus {
    if correct_candidate_present {
        ProposalStatus::Covered
    } else if truncated {
        ProposalStatus::CappedUnknown
    } else if candidate_count == 0 {
        ProposalStatus::MissedEmptyPool
    } else {
        ProposalStatus::MissedNonemptyPool
    }
}

/// Where the best correct candidate ranked -- `NotApplicable` whenever
/// `proposal_status != Covered` (there is nothing to rank). See
/// [`ProposalStatus`]'s doc comment for the orthogonality contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RankingStatus {
    Top1,
    Top5,
    Top10,
    Beyond10,
    NotApplicable,
}

impl RankingStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Top1 => "top1",
            Self::Top5 => "top5",
            Self::Top10 => "top10",
            Self::Beyond10 => "beyond10",
            Self::NotApplicable => "not_applicable",
        }
    }
}

fn ranking_status_for(
    proposal_status: ProposalStatus,
    best_correct_rank: Option<usize>,
) -> RankingStatus {
    match (proposal_status, best_correct_rank) {
        (ProposalStatus::Covered, Some(rank)) if rank < 1 => RankingStatus::Top1,
        (ProposalStatus::Covered, Some(rank)) if rank < 5 => RankingStatus::Top5,
        (ProposalStatus::Covered, Some(rank)) if rank < 10 => RankingStatus::Top10,
        (ProposalStatus::Covered, Some(_)) => RankingStatus::Beyond10,
        _ => RankingStatus::NotApplicable,
    }
}

/// Stereochemistry comparison outcome, orthogonal to `ranking_status`/
/// `proposal_status` -- `NotApplicable` whenever prediction was never
/// attempted; otherwise mirrors [`StereoIgnoredOutcome`] plus the exact-hit
/// case. `Unsupported` is never silently collapsed to `NoHit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StereoStatus {
    /// Correct under the stereochemistry-AWARE comparison (implies correct
    /// under the ignored comparison too).
    ExactHit,
    /// Correct only once stereochemistry is ignored -- "constitution right,
    /// stereochemistry wrong".
    StereoOnlyHit,
    NoHit,
    Unsupported,
    NotApplicable,
}

impl StereoStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExactHit => "exact_hit",
            Self::StereoOnlyHit => "stereo_only_hit",
            Self::NoHit => "no_hit",
            Self::Unsupported => "unsupported",
            Self::NotApplicable => "not_applicable",
        }
    }
}

fn stereo_status_for(
    is_attempt: bool,
    stereochemistry_aware_hit: bool,
    stereochemistry_ignored_outcome: StereoIgnoredOutcome,
) -> StereoStatus {
    if !is_attempt {
        return StereoStatus::NotApplicable;
    }
    if stereochemistry_aware_hit {
        return StereoStatus::ExactHit;
    }
    match stereochemistry_ignored_outcome {
        StereoIgnoredOutcome::Hit => StereoStatus::StereoOnlyHit,
        StereoIgnoredOutcome::NoHit => StereoStatus::NoHit,
        StereoIgnoredOutcome::Unsupported => StereoStatus::Unsupported,
    }
}

/// Derives the legacy coarse [`FailureReason`] from the four orthogonal
/// status fields -- the single source of truth `compute_row`/`invalid_row`
/// call, so the two representations can never disagree (see
/// `failure_reason_is_uniquely_derivable_from_orthogonal_statuses`).
/// `FailureReason` predates `ProposalStatus::CappedUnknown` and has no
/// equivalent state for it; mapped to the closest legacy bucket
/// (`CorrectAbsentNonemptyPool`) since it's unreachable in practice anyway
/// (see [`ProposalStatus::CappedUnknown`]).
fn derive_failure_reason(
    input_status: InputStatus,
    proposal_status: ProposalStatus,
    ranking_status: RankingStatus,
) -> FailureReason {
    if input_status == InputStatus::Invalid {
        return FailureReason::InputInvalid;
    }
    match proposal_status {
        ProposalStatus::Error => FailureReason::PredictionError,
        ProposalStatus::Covered => match ranking_status {
            RankingStatus::Top1 => FailureReason::HitTop1,
            RankingStatus::Top5 => FailureReason::HitTop5,
            RankingStatus::Top10 => FailureReason::HitTop10,
            RankingStatus::Beyond10 | RankingStatus::NotApplicable => FailureReason::HitBeyond10,
        },
        ProposalStatus::MissedEmptyPool => FailureReason::CorrectAbsentEmptyPool,
        ProposalStatus::MissedNonemptyPool | ProposalStatus::CappedUnknown => {
            FailureReason::CorrectAbsentNonemptyPool
        }
        ProposalStatus::NotAttempted => FailureReason::InputInvalid,
    }
}

/// Rejects a [`BenchRow`] whose four orthogonal status fields contradict each
/// other or `best_correct_rank` -- called by every `BenchRow` constructor
/// (`invalid_row`, `compute_row`). Since both constructors derive every
/// status field from the same handful of underlying values, this should
/// never actually fire; it exists as a real hard-error guard rather than a
/// silently-trusted invariant, per the audit's explicit request.
fn check_status_consistency(row: &BenchRow) -> Result<()> {
    if row.input_status == InputStatus::Invalid
        && (row.proposal_status != ProposalStatus::NotAttempted
            || row.ranking_status != RankingStatus::NotApplicable
            || row.stereo_status != StereoStatus::NotApplicable)
    {
        bail!(
            "inconsistent BenchRow {:?}: input_status=invalid requires proposal_status= \
             not_attempted, ranking_status=not_applicable, stereo_status=not_applicable; got \
             {:?}/{:?}/{:?}",
            row.reaction_id,
            row.proposal_status,
            row.ranking_status,
            row.stereo_status
        );
    }
    let covered = row.proposal_status == ProposalStatus::Covered;
    let has_rank_status = row.ranking_status != RankingStatus::NotApplicable;
    if covered != has_rank_status {
        bail!(
            "inconsistent BenchRow {:?}: proposal_status=covered must have a concrete \
             ranking_status and vice versa; got {:?}/{:?}",
            row.reaction_id,
            row.proposal_status,
            row.ranking_status
        );
    }
    if covered != row.best_correct_rank.is_some() {
        bail!(
            "inconsistent BenchRow {:?}: proposal_status=covered must agree with \
             best_correct_rank.is_some(); got {:?}/{:?}",
            row.reaction_id,
            row.proposal_status,
            row.best_correct_rank
        );
    }
    if row.proposal_status == ProposalStatus::Error
        && row.stereo_status != StereoStatus::NotApplicable
    {
        bail!(
            "inconsistent BenchRow {:?}: proposal_status=error must have \
             stereo_status=not_applicable; got {:?}",
            row.reaction_id,
            row.stereo_status
        );
    }
    Ok(())
}

/// Min/max/mixed accepted-product arity across every entry in
/// `accepted_products_canonical` -- independent of the outer list's order
/// (min/max over a multiset don't care about order), and independent of
/// outer-list dedup (already applied in `load_corpus` -- see C4). `(0, 0,
/// false)` for an empty slice (only [`InvalidReactionAttempt`] rows ever
/// have one; see [`product_arity_bucket`]'s `"<missing>"` handling).
fn accepted_product_arity(accepted_products_canonical: &[Vec<String>]) -> (usize, usize, bool) {
    let lens = accepted_products_canonical.iter().map(Vec::len);
    let min = lens.clone().min().unwrap_or(0);
    let max = lens.max().unwrap_or(0);
    (min, max, min != max)
}

fn arity_bucket_label(n: usize) -> String {
    match n {
        1 => "1".to_string(),
        2 => "2".to_string(),
        _ => "3+".to_string(),
    }
}

/// Breakdown bucket key for a row's accepted-product arity: `"1"`/`"2"`/
/// `"3+"` when every accepted outcome has the same product count, `"mixed:
/// {min}-{max}"` (each end bucketed the same way) when they differ, and
/// `"<missing>"` for a row with no accepted-products info at all (an
/// [`InvalidReactionAttempt`] row).
fn product_arity_bucket(row: &BenchRow) -> String {
    if row.accepted_products_canonical.is_empty() {
        return "<missing>".to_string();
    }
    if row.accepted_product_count_mixed {
        format!(
            "mixed:{}-{}",
            arity_bucket_label(row.accepted_product_count_min),
            arity_bucket_label(row.accepted_product_count_max)
        )
    } else {
        arity_bucket_label(row.accepted_product_count_max)
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
    /// The group key `split` was derived from (see [`split_for_group`]) --
    /// `None` only for [`InvalidReactionAttempt`] rows, where no group key
    /// could ever be computed (see `split`'s own `"unknown"` sentinel).
    /// Carried onto every row (not just used internally to compute `split`)
    /// so the leakage-safety guarantee is auditable from `rows.jsonl` alone,
    /// without cross-referencing the raw corpus file.
    pub leakage_group_id: Option<String>,
    pub reaction_class: Option<String>,
    /// The corpus's own reactant text/order -- what was actually passed to
    /// `predict_products_detailed` (see [`BenchReaction::reactants_original`]).
    pub reactants_original: Vec<String>,
    pub reactants_canonical: Vec<String>,
    pub accepted_products_canonical: Vec<Vec<String>>,
    pub num_reactants: usize,
    /// Smallest accepted-product count across every entry in
    /// `accepted_products_canonical` -- see [`accepted_product_arity`].
    pub accepted_product_count_min: usize,
    /// Largest accepted-product count across every entry in
    /// `accepted_products_canonical`.
    pub accepted_product_count_max: usize,
    /// `accepted_product_count_min != accepted_product_count_max` -- true
    /// when this reaction's accepted outcomes don't all have the same
    /// product count (e.g. a 1-product outcome and a 2-product outcome both
    /// accepted). Independent of `accepted_products_canonical`'s outer-list
    /// order (min/max don't care about order) and independent of the C4
    /// outer-list dedup already applied when this reaction was loaded.
    pub accepted_product_count_mixed: bool,
    pub has_stereochemistry: bool,
    pub candidate_count: usize,
    /// Total raw `run_reactants` outcomes attempted for this reaction
    /// (before validity/no-op filtering or merging) -- the denominator
    /// `invalid_product_rate`/`no_op_rate` are computed against.
    pub raw_outcomes: usize,
    pub correct_candidate_present: bool,
    pub best_correct_rank: Option<usize>,
    /// Every rank (0-based, ascending, `< 10`) whose candidate matches ANY
    /// entry in `accepted_products_canonical` -- not just the best one.
    /// `accepted_products_canonical` allows more than one correct outcome
    /// (e.g. competing literature reports), so a ranking that surfaces two
    /// of three accepted outcomes in the top 10 deserves more NDCG@10
    /// credit than one that surfaces only one; `best_correct_rank` alone
    /// can't express that. Used only for `ndcg_at_10`'s multi-positive
    /// binary-relevance computation -- `best_correct_rank`/`top1_hit`/
    /// `top5_hit`/`top10_hit`/mean/median rank are unaffected and still
    /// derived from the single best rank.
    pub correct_ranks_top10: Vec<usize>,
    /// Best rank under the stereochemistry-ignored comparison (see
    /// [`stereo_ignored_canonical`]) -- always `<= best_correct_rank` when
    /// both are present, since the ignored comparison is strictly looser.
    pub best_correct_rank_stereo_ignored: Option<usize>,
    pub top1_hit: bool,
    pub top5_hit: bool,
    pub top10_hit: bool,
    /// `best_correct_rank < 10` under the exact (stereochemistry-aware)
    /// comparison -- identical to `top10_hit`, reported under its own name
    /// so it sits directly next to `stereochemistry_ignored_outcome` for a
    /// same-row before/after comparison.
    pub stereochemistry_aware_hit: bool,
    /// `Hit` while `stereochemistry_aware_hit` is `false` is the diagnostic
    /// signal for "constitution right, stereochemistry wrong". See
    /// [`StereoIgnoredOutcome`] for why this isn't a plain `bool`.
    pub stereochemistry_ignored_outcome: StereoIgnoredOutcome,
    pub invalid_candidate_count: usize,
    pub no_op_candidate_count: usize,
    pub application_warning_count: usize,
    pub application_error_count: usize,
    pub templates_attempted: usize,
    /// From the underlying `ForwardStats` -- templates that had at least one
    /// slot match against a reactant (a subset of `templates_attempted`).
    pub templates_matched: usize,
    /// From the underlying `ForwardStats` -- graph-based rules (empty
    /// `smirks`) skipped, never counted as a parse failure.
    pub graph_rules_skipped: usize,
    pub rules_loaded: usize,
    /// Wall-clock time for the one `predict_products_detailed` call this row
    /// required. NOT part of this harness's determinism guarantee -- see
    /// `docs/guides/forward-benchmark.md`'s determinism section for exactly
    /// which fields (this one, plus every `latency_ms` aggregate block) are
    /// expected to vary between otherwise-identical runs.
    pub elapsed_ms: f64,
    pub failure_reason: FailureReason,
    /// Whether this row's input canonicalized at all -- see [`InputStatus`].
    pub input_status: InputStatus,
    /// Whether a correct candidate was present in the pool -- see
    /// [`ProposalStatus`]. Orthogonal to `ranking_status`.
    pub proposal_status: ProposalStatus,
    /// Where the best correct candidate ranked -- see [`RankingStatus`].
    /// `NotApplicable` iff `proposal_status != Covered`.
    pub ranking_status: RankingStatus,
    /// Stereochemistry comparison outcome -- see [`StereoStatus`].
    /// `NotApplicable` iff prediction was never attempted.
    pub stereo_status: StereoStatus,
    pub provenance: RowProvenance,
}

fn invalid_row(attempt: &InvalidReactionAttempt, provenance: &RowProvenance) -> Result<BenchRow> {
    let row = BenchRow {
        reaction_id: attempt.reaction_id.clone(),
        source_line: attempt.source_line,
        // A group key (and therefore a split) cannot be computed reliably
        // once the reactants themselves failed to canonicalize -- "unknown"
        // is excluded from every split-based aggregate (see `aggregate`).
        split: "unknown".to_string(),
        leakage_group_id: None,
        reaction_class: None,
        reactants_original: Vec::new(),
        reactants_canonical: Vec::new(),
        accepted_products_canonical: Vec::new(),
        num_reactants: 0,
        accepted_product_count_min: 0,
        accepted_product_count_max: 0,
        accepted_product_count_mixed: false,
        has_stereochemistry: false,
        candidate_count: 0,
        raw_outcomes: 0,
        correct_candidate_present: false,
        best_correct_rank: None,
        correct_ranks_top10: Vec::new(),
        best_correct_rank_stereo_ignored: None,
        top1_hit: false,
        top5_hit: false,
        top10_hit: false,
        stereochemistry_aware_hit: false,
        stereochemistry_ignored_outcome: StereoIgnoredOutcome::NoHit,
        invalid_candidate_count: 0,
        no_op_candidate_count: 0,
        application_warning_count: 0,
        application_error_count: 0,
        templates_attempted: 0,
        templates_matched: 0,
        graph_rules_skipped: 0,
        rules_loaded: 0,
        elapsed_ms: 0.0,
        failure_reason: FailureReason::InputInvalid,
        input_status: InputStatus::Invalid,
        proposal_status: ProposalStatus::NotAttempted,
        ranking_status: RankingStatus::NotApplicable,
        stereo_status: StereoStatus::NotApplicable,
        provenance: provenance.clone(),
    };
    check_status_consistency(&row)?;
    Ok(row)
}

fn compute_row(
    reaction: &BenchReaction,
    rules: &[RetroRule],
    provenance: &RowProvenance,
) -> Result<BenchRow> {
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
    let (accepted_product_count_min, accepted_product_count_max, accepted_product_count_mixed) =
        accepted_product_arity(&reaction.accepted_products_canonical);
    let split = split_for_group(&reaction.group_key).to_string();

    let report = match predict_result {
        Ok(r) => r,
        Err(_) => {
            let row = BenchRow {
                reaction_id: reaction.reaction_id.clone(),
                source_line: reaction.source_line,
                split,
                leakage_group_id: Some(reaction.group_key.clone()),
                reaction_class: reaction.reaction_class.clone(),
                reactants_original: reaction.reactants_original.clone(),
                reactants_canonical: reaction.reactants_canonical.clone(),
                accepted_products_canonical: reaction.accepted_products_canonical.clone(),
                num_reactants,
                accepted_product_count_min,
                accepted_product_count_max,
                accepted_product_count_mixed,
                has_stereochemistry: reaction.has_stereochemistry,
                candidate_count: 0,
                raw_outcomes: 0,
                correct_candidate_present: false,
                best_correct_rank: None,
                correct_ranks_top10: Vec::new(),
                best_correct_rank_stereo_ignored: None,
                top1_hit: false,
                top5_hit: false,
                top10_hit: false,
                stereochemistry_aware_hit: false,
                stereochemistry_ignored_outcome: StereoIgnoredOutcome::NoHit,
                invalid_candidate_count: 0,
                no_op_candidate_count: 0,
                application_warning_count: 0,
                application_error_count: 0,
                templates_attempted: 0,
                templates_matched: 0,
                graph_rules_skipped: 0,
                rules_loaded: rules.len(),
                elapsed_ms,
                failure_reason: FailureReason::PredictionError,
                input_status: InputStatus::Valid,
                proposal_status: ProposalStatus::Error,
                ranking_status: RankingStatus::NotApplicable,
                stereo_status: StereoStatus::NotApplicable,
                provenance: provenance.clone(),
            };
            check_status_consistency(&row)?;
            return Ok(row);
        }
    };

    // `None` (not a silent fallback) if any accepted product fails to
    // convert -- the whole ignored-comparison dimension becomes
    // `Unsupported` rather than mis-measuring against a partial ground
    // truth.
    let accepted_ignored: Option<Vec<Vec<String>>> = reaction
        .accepted_products_canonical
        .iter()
        .map(|set| stereo_ignored_set(set))
        .collect();
    let mut stereo_ignored_unsupported = accepted_ignored.is_none();

    let mut best_aware_rank: Option<usize> = None;
    let mut best_ignored_rank: Option<usize> = None;
    let mut correct_ranks_top10: Vec<usize> = Vec::new();
    for candidate in &report.candidates {
        if reaction
            .accepted_products_canonical
            .contains(&candidate.products)
        {
            if best_aware_rank.is_none() {
                best_aware_rank = Some(candidate.rank);
            }
            if candidate.rank < 10 {
                correct_ranks_top10.push(candidate.rank);
            }
        }
        if !stereo_ignored_unsupported && best_ignored_rank.is_none() {
            match stereo_ignored_set(&candidate.products) {
                Some(candidate_ignored) => {
                    if accepted_ignored
                        .as_ref()
                        .is_some_and(|sets| sets.contains(&candidate_ignored))
                    {
                        best_ignored_rank = Some(candidate.rank);
                    }
                }
                None => stereo_ignored_unsupported = true,
            }
        }
        // Candidates are already rank-ordered ascending. Once past rank 9,
        // no further candidate can add to `correct_ranks_top10`; safe to
        // stop once both the aware and ignored dimensions are also
        // resolved (nothing later can improve either).
        if candidate.rank >= 10
            && best_aware_rank.is_some()
            && (best_ignored_rank.is_some() || stereo_ignored_unsupported)
        {
            break;
        }
    }

    let candidate_count = report.candidates.len();
    let correct_candidate_present = best_aware_rank.is_some();
    let top1_hit = best_aware_rank == Some(0);
    let top5_hit = best_aware_rank.is_some_and(|r| r < 5);
    let top10_hit = best_aware_rank.is_some_and(|r| r < 10);
    let stereochemistry_ignored_outcome = if stereo_ignored_unsupported {
        StereoIgnoredOutcome::Unsupported
    } else if best_ignored_rank.is_some_and(|r| r < 10) {
        StereoIgnoredOutcome::Hit
    } else {
        StereoIgnoredOutcome::NoHit
    };

    let proposal_status = proposal_status_for(
        correct_candidate_present,
        candidate_count,
        report.stats.truncated,
    );
    let ranking_status = ranking_status_for(proposal_status, best_aware_rank);
    let stereo_status = stereo_status_for(true, top10_hit, stereochemistry_ignored_outcome);
    let failure_reason = derive_failure_reason(InputStatus::Valid, proposal_status, ranking_status);

    let row = BenchRow {
        reaction_id: reaction.reaction_id.clone(),
        source_line: reaction.source_line,
        split,
        leakage_group_id: Some(reaction.group_key.clone()),
        reaction_class: reaction.reaction_class.clone(),
        reactants_original: reaction.reactants_original.clone(),
        reactants_canonical: reaction.reactants_canonical.clone(),
        accepted_products_canonical: reaction.accepted_products_canonical.clone(),
        num_reactants,
        accepted_product_count_min,
        accepted_product_count_max,
        accepted_product_count_mixed,
        has_stereochemistry: reaction.has_stereochemistry,
        candidate_count,
        raw_outcomes: report.stats.raw_outcomes,
        correct_candidate_present,
        best_correct_rank: best_aware_rank,
        correct_ranks_top10,
        best_correct_rank_stereo_ignored: best_ignored_rank,
        top1_hit,
        top5_hit,
        top10_hit,
        stereochemistry_aware_hit: top10_hit,
        stereochemistry_ignored_outcome,
        invalid_candidate_count: report.stats.invalid_outcomes_rejected,
        no_op_candidate_count: report.stats.no_op_outcomes_rejected,
        application_warning_count: report.warnings.len(),
        application_error_count: report.stats.template_application_errors,
        templates_attempted: report.stats.templates_attempted,
        templates_matched: report.stats.templates_matched,
        graph_rules_skipped: report.stats.graph_rules_skipped,
        rules_loaded: report.stats.rules_loaded,
        elapsed_ms,
        failure_reason,
        input_status: InputStatus::Valid,
        proposal_status,
        ranking_status,
        stereo_status,
        provenance: provenance.clone(),
    };
    check_status_consistency(&row)?;
    Ok(row)
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
    /// Multi-positive, binary-relevance NDCG@10: every candidate (up to
    /// rank 9) matching ANY entry in `accepted_products_canonical` counts
    /// as relevant, not just the single best-ranked one -- the schema
    /// allows more than one accepted outcome (e.g. competing literature
    /// reports), so a ranking surfacing two of three accepted outcomes in
    /// the top 10 gets more credit than one surfacing only one. Ideal DCG
    /// is computed against `min(10, accepted_products_canonical.len())`,
    /// i.e. the true number of distinct accepted outcomes for that row
    /// (after outer-list dedup), not assumed to always be exactly 1.
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
        .map(|r| {
            let ideal_count = r.accepted_products_canonical.len().min(10);
            if ideal_count == 0 {
                // Never true for a row that reached `hit_rate_metrics`
                // (only valid, successfully-predicted rows do, and those
                // always carry >= 1 accepted product) -- guarded anyway
                // rather than dividing by zero if that invariant ever
                // breaks.
                return 0.0;
            }
            let dcg: f64 = r
                .correct_ranks_top10
                .iter()
                .map(|&rank| 1.0 / (rank as f64 + 2.0).log2())
                .sum();
            let idcg: f64 = (0..ideal_count)
                .map(|i| 1.0 / (i as f64 + 2.0).log2())
                .sum();
            dcg / idcg
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
    /// `sum(raw_outcomes)` over valid rows -- the explicit denominator for
    /// `invalid_product_rate`/`no_op_rate` below, so a reader never has to
    /// re-derive it from `rows.jsonl` to know what either rate is "out of".
    pub n_raw_outcomes: usize,
    /// `sum(invalid_candidate_count)` over valid rows -- the raw count behind
    /// `invalid_product_rate` below (the top-level report's
    /// `DiagnosticCounts` transcribes this value rather than re-summing rows
    /// independently).
    pub invalid_outcomes_rejected: usize,
    /// `sum(no_op_candidate_count)` over valid rows -- see
    /// `invalid_outcomes_rejected`.
    pub no_op_outcomes_rejected: usize,
    /// `invalid_outcomes_rejected / n_raw_outcomes` over valid rows.
    pub invalid_product_rate: Option<f64>,
    /// `no_op_outcomes_rejected / n_raw_outcomes` over valid rows.
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
        n_raw_outcomes: raw_outcomes_total,
        invalid_outcomes_rejected: invalid_total,
        no_op_outcomes_rejected: no_op_total,
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
    /// SHA-256 of the raw `--templates` file bytes -- `None` for `embedded`,
    /// where there is no file to hash (see `rules_content_sha256` for a
    /// mode-independent alternative).
    pub rules_file_sha256: Option<String>,
    /// SHA-256 over the canonical (`template_id`, `smirks`) sequence of the
    /// actually-loaded rule set, sorted for order-independence -- populated
    /// for EVERY `template_source` including `embedded`, unlike
    /// `rules_file_sha256`. Catches "the embedded default rule set changed
    /// between runs" (a rebuild against a different `renkin` version), which
    /// `rules_file_sha256: None` cannot.
    pub rules_content_sha256: String,
    pub rules_loaded: usize,
    pub corpus_path: String,
    pub corpus_sha256: String,
    pub train_max_bucket: u32,
    pub val_max_bucket: u32,
    pub split_protocol_version: u32,
    /// SHA-256 of the currently-running executable's own file on disk.
    /// `None` if `std::env::current_exe()` or reading it failed (never a
    /// hard error for the run). NOT part of `reproducibility_sha256` --
    /// Rust builds are not bit-reproducible, so rebuilding from identical
    /// source still changes this value; folding it into the reproducibility
    /// hash would make "same source, same corpus" register as different.
    pub binary_sha256: Option<String>,
    /// SHA-256 of the workspace `Cargo.lock` at compile time -- pins the
    /// entire resolved dependency graph (including chematic's exact
    /// version/source/checksum) as one number, cheaper and more reliable
    /// than hand-parsing individual package entries out of it.
    pub cargo_lock_sha256: String,
    /// SHA-256 over this run's own deterministic configuration (currently:
    /// `template_source`, `split_protocol_version`, `train_max_bucket`,
    /// `val_max_bucket`) -- deliberately EXCLUDES `corpus_path`, a
    /// filesystem path that can differ across machines/invocations even for
    /// byte-identical corpus *content* (`corpus_sha256` already captures
    /// content identity).
    pub config_sha256: String,
    /// SHA-256 over every row's fields EXCEPT `elapsed_ms` (see
    /// `reproducibility_excludes`), in `source_line` order. Two runs on the
    /// same corpus/rules/`--template-source` MUST produce the same value --
    /// `tests/bench.rs::benchmark_is_deterministic_modulo_timing_fields`
    /// asserts this directly.
    pub reproducibility_sha256: String,
    /// Field names never covered by `reproducibility_sha256` (wherever they
    /// appear in a row or in this provenance block), because they are
    /// expected to vary between two otherwise-identical runs (`elapsed_ms`,
    /// `latency_ms`) or because folding them in would make bit-identical
    /// rebuilds of the same source register as non-reproducible
    /// (`binary_sha256`) or make the same corpus content register as
    /// non-reproducible across machines (`corpus_path`).
    pub reproducibility_excludes: Vec<String>,
}

/// Report-level diagnostic totals, transcribed from already-computed sources
/// rather than re-derived independently (see each field's doc comment for
/// its exact source) -- so this struct can never silently disagree with the
/// numbers it's built from.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DiagnosticCounts {
    /// Corpus-load warning counts keyed by [`CorpusLoadWarning::code`] --
    /// transcribed directly from `corpus_warnings`.
    pub warning_counts_by_code: BTreeMap<String, usize>,
    /// Row-wise sum of `BenchRow::application_error_count`.
    pub template_application_errors: usize,
    /// Transcribed from `overall.invalid_outcomes_rejected` (the same value
    /// `invalid_product_rate`'s numerator already uses).
    pub invalid_outcomes_rejected: usize,
    /// Transcribed from `overall.no_op_outcomes_rejected`.
    pub no_op_outcomes_rejected: usize,
    /// Always empty in this harness: `load_templates_strict`/
    /// `load_rules_for_source` reject the whole rule set on any single
    /// rule's parse failure (a hard error before any row is computed), so a
    /// run that completes at all has zero PARTIAL template-parse rejections
    /// by construction. A non-empty map would require `ForwardStats`/rule
    /// loading to grow a genuine per-rule rejection-reason breakdown first
    /// -- not invented here (same reasoning as C1's `train-extracted` hard
    /// error: an honest empty map beats a populated-looking one with no
    /// real data behind it).
    pub template_parse_rejections_by_reason: BTreeMap<String, usize>,
    /// Row-wise sum of `BenchRow::graph_rules_skipped`.
    pub graph_rules_skipped: usize,
    /// Row-wise sum of `BenchRow::templates_attempted`.
    pub templates_attempted: usize,
    /// Row-wise sum of `BenchRow::templates_matched`.
    pub templates_matched: usize,
    /// Transcribed from `overall.n_raw_outcomes` (already the row-wise sum
    /// of `BenchRow::raw_outcomes` over valid rows).
    pub raw_outcomes: usize,
}

fn diagnostic_counts(
    rows: &[BenchRow],
    corpus_warnings: &[CorpusLoadWarning],
    overall: &BenchAggregate,
) -> DiagnosticCounts {
    let mut warning_counts_by_code: BTreeMap<String, usize> = BTreeMap::new();
    for w in corpus_warnings {
        *warning_counts_by_code.entry(w.code.clone()).or_insert(0) += 1;
    }
    DiagnosticCounts {
        warning_counts_by_code,
        template_application_errors: rows.iter().map(|r| r.application_error_count).sum(),
        invalid_outcomes_rejected: overall.invalid_outcomes_rejected,
        no_op_outcomes_rejected: overall.no_op_outcomes_rejected,
        template_parse_rejections_by_reason: BTreeMap::new(),
        graph_rules_skipped: rows.iter().map(|r| r.graph_rules_skipped).sum(),
        templates_attempted: rows.iter().map(|r| r.templates_attempted).sum(),
        templates_matched: rows.iter().map(|r| r.templates_matched).sum(),
        raw_outcomes: overall.n_raw_outcomes,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BenchReport {
    pub schema_version: u32,
    pub provenance: RunProvenance,
    pub corpus_stats: CorpusLoadStats,
    pub corpus_warnings: Vec<CorpusLoadWarning>,
    pub diagnostics: DiagnosticCounts,
    /// All rows, all splits combined.
    pub overall: BenchAggregate,
    /// Keyed `"train"`/`"val"`/`"test"` -- a row with `split == "unknown"`
    /// (an invalid-input row whose group key couldn't be computed) is
    /// counted in `overall` but not in any of these three.
    pub by_split: BTreeMap<String, BenchAggregate>,
    /// Keyed by dimension name: `reaction_class`, `num_reactants`,
    /// `accepted_product_arity`, `stereochemistry_presence`,
    /// `template_source`, `candidate_pool_size`, `failure_reason`,
    /// `input_status`, `proposal_status`, `ranking_status`, `stereo_status`.
    pub breakdowns: BTreeMap<String, Vec<BenchBreakdownBucket>>,
}

/// Portable, compact record of the benchmark inputs and protocol. Unlike
/// `RunProvenance`, this is intended to be shared as the comparison contract
/// without distributing the row-level benchmark output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkManifest {
    pub schema_version: u32,
    pub corpus_schema_version: u32,
    pub report_schema_version: u32,
    pub split_protocol_version: u32,
    pub train_max_bucket: u32,
    pub val_max_bucket: u32,
    pub renkin_forward_version: String,
    pub template_source: String,
    pub rules_content_sha256: String,
    pub rules_file_sha256: Option<String>,
    pub rules_loaded: usize,
    pub corpus_sha256: String,
    pub config_sha256: String,
    pub reproducibility_sha256: String,
}

impl BenchmarkManifest {
    pub fn from_report(report: &BenchReport) -> Self {
        let provenance = &report.provenance;
        Self {
            schema_version: FORWARD_BENCH_MANIFEST_SCHEMA_VERSION,
            corpus_schema_version: FORWARD_BENCH_CORPUS_SCHEMA_VERSION,
            report_schema_version: report.schema_version,
            split_protocol_version: provenance.split_protocol_version,
            train_max_bucket: provenance.train_max_bucket,
            val_max_bucket: provenance.val_max_bucket,
            renkin_forward_version: provenance.renkin_forward_version.clone(),
            template_source: provenance.template_source.clone(),
            rules_content_sha256: provenance.rules_content_sha256.clone(),
            rules_file_sha256: provenance.rules_file_sha256.clone(),
            rules_loaded: provenance.rules_loaded,
            corpus_sha256: provenance.corpus_sha256.clone(),
            config_sha256: provenance.config_sha256.clone(),
            reproducibility_sha256: provenance.reproducibility_sha256.clone(),
        }
    }

    pub fn verify_against_report(&self, report: &BenchReport) -> Result<()> {
        let expected = Self::from_report(report);
        let mut mismatches = Vec::new();

        macro_rules! compare {
            ($field:ident) => {
                if self.$field != expected.$field {
                    mismatches.push(format!(
                        "{}: manifest={:?}, current={:?}",
                        stringify!($field),
                        self.$field,
                        expected.$field
                    ));
                }
            };
        }

        compare!(schema_version);
        compare!(corpus_schema_version);
        compare!(report_schema_version);
        compare!(split_protocol_version);
        compare!(train_max_bucket);
        compare!(val_max_bucket);
        compare!(renkin_forward_version);
        compare!(template_source);
        compare!(rules_content_sha256);
        compare!(rules_file_sha256);
        compare!(rules_loaded);
        compare!(corpus_sha256);
        compare!(config_sha256);
        compare!(reproducibility_sha256);

        if mismatches.is_empty() {
            Ok(())
        } else {
            bail!(
                "benchmark manifest verification failed:\n{}",
                mismatches.join("\n")
            )
        }
    }
}

pub struct BenchOutcome {
    pub rows: Vec<BenchRow>,
    pub report: BenchReport,
}

/// SHA-256 over the canonical (`template_id`, `smirks`) sequence of `rules`,
/// sorted first so file/embedded declaration order never affects the
/// result. Populated for every `TemplateSource`, unlike `rules_file_sha256`
/// (which only exists for `File`/`TrainExtracted`).
fn rules_content_hash(rules: &[RetroRule]) -> String {
    let mut pairs: Vec<String> = rules
        .iter()
        .map(|r| format!("{}\0{}", r.template_id, r.smirks))
        .collect();
    pairs.sort_unstable();
    let mut hasher = Sha256::new();
    hasher.update(b"renkin-forward-bench-rules-content-v1\0");
    hash_string_sequence(&mut hasher, &pairs);
    renkin::sha256_hex(hasher.finalize())
}

/// SHA-256 of the currently-running executable's own bytes on disk. `None`
/// (never a hard error) if the current executable's path or contents can't
/// be read.
fn binary_sha256() -> Option<String> {
    let path = std::env::current_exe().ok()?;
    let bytes = std::fs::read(path).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Some(renkin::sha256_hex(hasher.finalize()))
}

/// SHA-256 of the workspace `Cargo.lock` at compile time -- pins the entire
/// resolved dependency graph (chematic's exact version/source/checksum
/// included) as one number. Embedded via `include_str!` rather than a
/// runtime read: an installed/copied binary has no reliable relative path
/// back to the source tree's `Cargo.lock`, but the compiled-in content is
/// always available. (`renkin-forward` has a `path`-only dependency on the
/// workspace root and is not independently packaged -- confirmed via
/// `cargo package -p renkin-forward`, which already fails for that reason
/// today -- so this reaching outside the crate directory does not affect
/// publishing.)
fn cargo_lock_sha256() -> String {
    const CARGO_LOCK: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.lock"));
    let mut hasher = Sha256::new();
    hasher.update(CARGO_LOCK.as_bytes());
    renkin::sha256_hex(hasher.finalize())
}

/// SHA-256 over this run's own deterministic configuration values --
/// deliberately excludes any filesystem path (see `RunProvenance::
/// config_sha256`'s doc comment).
fn config_hash(template_source: &str, train_max_bucket: u32, val_max_bucket: u32) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"renkin-forward-bench-config-v1\0");
    hash_string_sequence(&mut hasher, &[template_source.to_string()]);
    hasher.update(train_max_bucket.to_be_bytes());
    hasher.update(val_max_bucket.to_be_bytes());
    hasher.update(SPLIT_PROTOCOL_VERSION.to_be_bytes());
    renkin::sha256_hex(hasher.finalize())
}

/// Field names published as `RunProvenance::reproducibility_excludes` --
/// every one a reader should expect to legitimately differ between two
/// otherwise-identical runs (`elapsed_ms`/`latency_ms`: real wall-clock
/// timing; `binary_sha256`: Rust builds aren't bit-reproducible;
/// `corpus_path`: a filesystem path, not corpus content). Only `elapsed_ms`
/// is actually stripped by `reproducibility_hash` below -- the other three
/// never appear inside a `BenchRow` in the first place (they live in
/// `RunProvenance`/`BenchAggregate`, outside this hash's scope), so listing
/// them here is purely documentation of the wider "reproducible modulo
/// what" contract, not something this specific hash needs to strip.
const REPRODUCIBILITY_EXCLUDES: &[&str] =
    &["elapsed_ms", "latency_ms", "binary_sha256", "corpus_path"];

/// SHA-256 over every row's fields except `elapsed_ms`, in the rows'
/// existing (already source_line-sorted) order. Deliberately scoped to rows
/// only, not the full report: `overall`/`by_split`/`breakdowns` are pure,
/// deterministic functions of `rows` (no hidden per-run state), so hashing
/// rows alone already implies the whole report is reproducible -- and
/// avoids the self-reference problem of hashing a `BenchReport` that would
/// need to contain this very hash. Two runs against the same corpus/rules/
/// `--template-source` must produce byte-identical rows modulo `elapsed_ms`
/// (see `docs/guides/forward-benchmark.md`'s "Determinism" section) --
/// `tests/bench.rs::benchmark_is_deterministic_modulo_timing_fields`
/// asserts this hash matches across two independent runs directly.
fn reproducibility_hash(rows: &[BenchRow]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"renkin-forward-bench-reproducibility-v1\0");
    for row in rows {
        let mut value = serde_json::to_value(row).expect("BenchRow always serializes");
        if let Some(obj) = value.as_object_mut() {
            obj.remove("elapsed_ms");
        }
        let bytes = serde_json::to_vec(&value).expect("serde_json::Value always serializes");
        hasher.update((bytes.len() as u64).to_be_bytes());
        hasher.update(&bytes);
    }
    renkin::sha256_hex(hasher.finalize())
}

/// Runs the full Phase 1 harness end to end: load + canonicalize + dedupe
/// the corpus, load exactly one rule set for `template_source`, predict and
/// score every reaction, then aggregate. Never panics on malformed corpus
/// content -- only a missing/unreadable corpus file, a missing/invalid
/// `--templates` file, an invalid `template_source` name, or (when `strict`)
/// one of the conditions named in [`run_benchmark`]'s `strict` parameter doc
/// is a hard error.
///
/// `strict`: when true, promotes every one of the following from a
/// counted-and-continued data-quality issue to a whole-run hard error:
/// malformed corpus JSON, wrong corpus schema version, an unparseable
/// reactant/product, empty reactants/accepted products, a conflicting
/// explicit `group_key` or reused `reaction_id`, a per-row prediction
/// failure, `proposal_status = capped_unknown`, or a missing
/// `binary_sha256` (incomplete reproducibility provenance). Template load/
/// manifest/provenance failure is already an unconditional hard error (see
/// [`load_rules_for_source`]) -- `strict` doesn't change that. Deliberately
/// does NOT fail on a legitimate proposal miss, ranking miss, stereo
/// mismatch, or a genuinely empty candidate pool -- those are real
/// benchmark outcomes, not data-quality problems. In non-strict mode (the
/// default), every one of these still lands in `corpus_stats`/
/// `corpus_warnings`/`diagnostics`/per-row fields -- nothing is ever
/// silently dropped regardless of `strict`.
pub fn run_benchmark(
    corpus_path: &str,
    template_source: TemplateSource,
    templates_path: Option<&str>,
    strict: bool,
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
        rows.push(compute_row(reaction, &rules, &row_provenance)?);
    }
    for attempt in &invalid_attempts {
        rows.push(invalid_row(attempt, &row_provenance)?);
    }
    // Deterministic row order independent of the two loops above: by
    // source_line, so output never depends on internal push order.
    rows.sort_by_key(|r| r.source_line);

    let run_provenance = RunProvenance {
        renkin_forward_version: env!("CARGO_PKG_VERSION").to_string(),
        template_source: template_source.as_str().to_string(),
        rules_file_sha256,
        rules_content_sha256: rules_content_hash(&rules),
        rules_loaded: rules.len(),
        corpus_path: corpus_path.to_string(),
        corpus_sha256,
        train_max_bucket: TRAIN_MAX_BUCKET,
        val_max_bucket: VAL_MAX_BUCKET,
        split_protocol_version: SPLIT_PROTOCOL_VERSION,
        binary_sha256: binary_sha256(),
        cargo_lock_sha256: cargo_lock_sha256(),
        config_sha256: config_hash(template_source.as_str(), TRAIN_MAX_BUCKET, VAL_MAX_BUCKET),
        reproducibility_sha256: reproducibility_hash(&rows),
        reproducibility_excludes: REPRODUCIBILITY_EXCLUDES
            .iter()
            .map(|s| s.to_string())
            .collect(),
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
            // "<missing>" (not "unknown") -- a real corpus-supplied
            // reaction_class value of literally "unknown" must not collide
            // with "no reaction_class was given at all".
            r.reaction_class
                .clone()
                .unwrap_or_else(|| "<missing>".to_string())
        }),
    );
    breakdowns.insert(
        "num_reactants".to_string(),
        breakdown_by(rows.iter(), |r| r.num_reactants.to_string()),
    );
    breakdowns.insert(
        "accepted_product_arity".to_string(),
        breakdown_by(rows.iter(), product_arity_bucket),
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
    breakdowns.insert(
        "input_status".to_string(),
        breakdown_by(rows.iter(), |r| r.input_status.as_str().to_string()),
    );
    breakdowns.insert(
        "proposal_status".to_string(),
        breakdown_by(rows.iter(), |r| r.proposal_status.as_str().to_string()),
    );
    breakdowns.insert(
        "ranking_status".to_string(),
        breakdown_by(rows.iter(), |r| r.ranking_status.as_str().to_string()),
    );
    breakdowns.insert(
        "stereo_status".to_string(),
        breakdown_by(rows.iter(), |r| r.stereo_status.as_str().to_string()),
    );

    let diagnostics = diagnostic_counts(&rows, &corpus_warnings, &overall);

    if strict {
        let mut violations: Vec<String> = Vec::new();
        if corpus_stats.malformed_json > 0 {
            violations.push(format!(
                "{} corpus line(s) with malformed JSON",
                corpus_stats.malformed_json
            ));
        }
        if corpus_stats.wrong_schema_version > 0 {
            violations.push(format!(
                "{} corpus line(s) with the wrong schema_version",
                corpus_stats.wrong_schema_version
            ));
        }
        if corpus_stats.unparseable_smiles > 0 {
            violations.push(format!(
                "{} corpus line(s) with an unparseable reactant/product SMILES",
                corpus_stats.unparseable_smiles
            ));
        }
        if corpus_stats.empty_reactants_or_products > 0 {
            violations.push(format!(
                "{} corpus line(s) with empty reactants/accepted products",
                corpus_stats.empty_reactants_or_products
            ));
        }
        if corpus_stats.conflicting_group_keys > 0 {
            violations.push(format!(
                "{} reaction(s) with a conflicting explicit group_key",
                corpus_stats.conflicting_group_keys
            ));
        }
        if corpus_stats.conflicting_reaction_ids > 0 {
            violations.push(format!(
                "{} reaction(s) with a reused reaction_id for different chemistry",
                corpus_stats.conflicting_reaction_ids
            ));
        }
        let prediction_errors = rows
            .iter()
            .filter(|r| r.proposal_status == ProposalStatus::Error)
            .count();
        if prediction_errors > 0 {
            violations.push(format!(
                "{prediction_errors} row(s) where prediction itself failed"
            ));
        }
        let capped_unknown = rows
            .iter()
            .filter(|r| r.proposal_status == ProposalStatus::CappedUnknown)
            .count();
        if capped_unknown > 0 {
            violations.push(format!(
                "{capped_unknown} row(s) where the candidate cap left correctness unknown"
            ));
        }
        if run_provenance.binary_sha256.is_none() {
            violations.push(
                "binary_sha256 could not be computed (incomplete reproducibility provenance)"
                    .to_string(),
            );
        }
        if !violations.is_empty() {
            bail!(
                "--strict: benchmark run failed {} check(s):\n- {}",
                violations.len(),
                violations.join("\n- ")
            );
        }
    }

    let report = BenchReport {
        schema_version: FORWARD_BENCH_REPORT_SCHEMA_VERSION,
        provenance: run_provenance,
        corpus_stats,
        corpus_warnings,
        diagnostics,
        overall,
        by_split,
        breakdowns,
    };

    Ok(BenchOutcome { rows, report })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

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

    fn probe_rule(name: &str, smirks: &str) -> RetroRule {
        RetroRule {
            name: name.to_string(),
            template_id: format!("rule:{name}"),
            smirks: smirks.to_string(),
            weight: 1.0,
            required_elements: 0,
        }
    }

    #[test]
    fn rules_content_hash_differs_for_different_rule_sets_and_is_order_independent() {
        let a = [probe_rule("swap", "[c:1][Cl]>>[c:1][Br]")];
        let b = [probe_rule("swap", "[c:1][F]>>[c:1][Br]")];
        assert_ne!(rules_content_hash(&a), rules_content_hash(&b));

        let c = [
            probe_rule("swap", "[c:1][Cl]>>[c:1][Br]"),
            probe_rule("other", "[C:1][O]>>[C:1][N]"),
        ];
        let d = [
            probe_rule("other", "[C:1][O]>>[C:1][N]"),
            probe_rule("swap", "[c:1][Cl]>>[c:1][Br]"),
        ];
        assert_eq!(
            rules_content_hash(&c),
            rules_content_hash(&d),
            "declaration order must not affect the hash"
        );
    }

    #[test]
    fn config_hash_differs_by_template_source_only() {
        let a = config_hash("embedded", TRAIN_MAX_BUCKET, VAL_MAX_BUCKET);
        let b = config_hash("file", TRAIN_MAX_BUCKET, VAL_MAX_BUCKET);
        assert_ne!(a, b);
        assert_eq!(a, config_hash("embedded", TRAIN_MAX_BUCKET, VAL_MAX_BUCKET));
    }

    #[test]
    fn cargo_lock_sha256_is_stable_and_looks_like_a_sha256_hex_digest() {
        let a = cargo_lock_sha256();
        let b = cargo_lock_sha256();
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn reproducibility_hash_ignores_elapsed_ms_but_reacts_to_other_row_content() {
        let base = ndcg_probe_row(1, vec![0]);
        let mut same_chemistry_different_timing = base.clone();
        same_chemistry_different_timing.elapsed_ms = base.elapsed_ms + 500.0;
        assert_eq!(
            reproducibility_hash(std::slice::from_ref(&base)),
            reproducibility_hash(&[same_chemistry_different_timing]),
            "elapsed_ms alone must not change the reproducibility hash"
        );

        let mut different_chemistry = base.clone();
        different_chemistry.best_correct_rank = Some(3);
        assert_ne!(
            reproducibility_hash(&[base]),
            reproducibility_hash(&[different_chemistry]),
            "a genuinely different row must change the reproducibility hash"
        );
    }

    #[test]
    fn stereo_ignored_canonical_collapses_tetrahedral_and_ez() {
        let achiral_amino_acid = try_canonicalize("CC(N)C(=O)O").unwrap();
        for stereo in ["C[C@H](N)C(=O)O", "C[C@@H](N)C(=O)O"] {
            let canon = try_canonicalize(stereo).unwrap();
            assert_eq!(
                stereo_ignored_canonical(&canon).as_deref(),
                Some(achiral_amino_acid.as_str())
            );
        }

        let achiral_butene = try_canonicalize("CC=CC").unwrap();
        for stereo in ["C/C=C/C", "C/C=C\\C"] {
            let canon = try_canonicalize(stereo).unwrap();
            assert_eq!(
                stereo_ignored_canonical(&canon).as_deref(),
                Some(achiral_butene.as_str())
            );
        }
    }

    #[test]
    fn stereo_ignored_canonical_never_falls_back_to_the_stereo_aware_string() {
        // Regression guard: an independent audit found the previous
        // text-strip-then-reparse implementation fell back to the
        // stereo-AWARE canonical string on any reparse failure, which would
        // silently make `stereochemistry_ignored_outcome` an artifact of
        // that fallback rather than an honest comparison. The structural
        // implementation has no such fallback path -- confirm the isotope/
        // charge-preserving case still round-trips correctly (a case where
        // a naive fallback would be tempting to reach for).
        let canon = try_canonicalize("[13C@H](N)(C)C(=O)O").unwrap();
        let ignored = stereo_ignored_canonical(&canon).unwrap();
        assert!(ignored.contains("13C"), "isotope must survive: {ignored}");
        assert!(
            !ignored.contains('@'),
            "chirality must be cleared: {ignored}"
        );
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

    /// `train-extracted` is a recognized mode name (the frozen Phase 0
    /// protocol names it), but until a manifest can verify the train-only
    /// boundary, loading it exactly like `file` would silently accept an
    /// unverified split-safety claim. Must hard error, not fall through to
    /// `File`'s loading behavior with a different label stamped on top.
    #[test]
    fn load_rules_for_source_train_extracted_is_a_hard_error_without_a_manifest() {
        let err = load_rules_for_source(TemplateSource::TrainExtracted, Some("some/path.smi"))
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("train-extracted"));
        assert!(msg.contains("manifest"));
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
    fn atom_map_numbering_does_not_change_group_key_or_identity() {
        // Regression guard: an independent audit found that atom-map numbers
        // (`:1`, `:2`, ...) survive `canonical_smiles` unchanged, so two
        // corpus rows encoding the literal same reaction but with different
        // atom-map numbering previously got DIFFERENT `reaction_identity_hash`
        // (not deduped) and different `group_key` (different splits) --
        // silently defeating the leakage-safety guarantee this module exists
        // to provide.
        let dir = std::env::temp_dir().join(format!(
            "renkin-forward-bench-atom-map-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("corpus.jsonl");
        let content = r#"
{"schema_version": 1, "reaction_id": "unmapped", "reactants": ["Oc1ccccc1C(=O)O", "CCO"], "accepted_products": [["CCOC(=O)c1ccccc1O"]]}
{"schema_version": 1, "reaction_id": "mapped-1", "reactants": ["[OH:1]c1ccccc1[C:2](=[O:3])[OH:4]", "[CH3:5][CH2:6][OH:7]"], "accepted_products": [["CCOC(=O)c1ccccc1O"]]}
{"schema_version": 1, "reaction_id": "mapped-2", "reactants": ["[OH:21]c1ccccc1[C:22](=[O:23])[OH:24]", "[CH3:25][CH2:26][OH:27]"], "accepted_products": [["CCOC(=O)c1ccccc1O"]]}
"#;
        std::fs::write(&path, content).unwrap();

        let (reactions, _invalid, stats, _warnings) = load_corpus(path.to_str().unwrap()).unwrap();
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(
            stats.duplicate_records_merged, 2,
            "all 3 rows encode the same reaction and must dedupe to 1"
        );
        assert_eq!(reactions.len(), 1);
        assert!(
            !reactions[0]
                .reactants_canonical
                .iter()
                .any(|s| s.contains(':')),
            "canonical reactant SMILES must never carry a surviving atom-map \
             annotation: {:?}",
            reactions[0].reactants_canonical
        );
    }

    #[test]
    fn duplicate_accepted_product_entries_are_deduped_in_the_outer_list() {
        // Regression guard: a row's own `accepted_products` listing the same
        // multiset twice (verbatim, or under two SMILES spellings that
        // canonicalize identically) must not count as two distinct
        // ground-truth outcomes -- that would inflate `ndcg_at_10`'s ideal
        // DCG and double-count this reaction's accepted-outcome count.
        // Multiplicity WITHIN one accepted product multiset must still be
        // preserved (untouched by this dedup, which only applies to the
        // outer list).
        let dir = std::env::temp_dir().join(format!(
            "renkin-forward-bench-dup-accepted-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("corpus.jsonl");
        let content = r#"
{"schema_version": 1, "reaction_id": "dup-accepted", "reactants": ["CCO"], "accepted_products": [["CC=O"], ["CC=O"], ["C(C)=O"]]}
"#;
        std::fs::write(&path, content).unwrap();

        let (reactions, _invalid, _stats, _warnings) = load_corpus(path.to_str().unwrap()).unwrap();
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(reactions.len(), 1);
        assert_eq!(
            reactions[0].accepted_products_canonical.len(),
            1,
            "all 3 accepted-products entries canonicalize to the same multiset \
             and must dedupe to 1: {:?}",
            reactions[0].accepted_products_canonical
        );
    }

    #[test]
    fn duplicate_group_key_independent_of_row_order() {
        // Regression guard: an independent audit found that dedup-by-identity
        // happened BEFORE group-key resolution, keeping only the
        // first-encountered occurrence's group_key -- so whether an explicit
        // corpus-supplied group_key won over the deterministic fallback
        // depended on file row order, not on the metadata itself.
        let dir = std::env::temp_dir().join(format!(
            "renkin-forward-bench-group-key-order-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let bare_first = r#"
{"schema_version": 1, "reaction_id": "bare", "reactants": ["CCO"], "accepted_products": [["CC=O"]]}
{"schema_version": 1, "reaction_id": "keyed", "reactants": ["CCO"], "accepted_products": [["CC=O"]], "group_key": "explicit-patent-family-1"}
"#;
        let keyed_first = r#"
{"schema_version": 1, "reaction_id": "keyed", "reactants": ["CCO"], "accepted_products": [["CC=O"]], "group_key": "explicit-patent-family-1"}
{"schema_version": 1, "reaction_id": "bare", "reactants": ["CCO"], "accepted_products": [["CC=O"]]}
"#;

        let path_a = dir.join("bare_first.jsonl");
        std::fs::write(&path_a, bare_first).unwrap();
        let (reactions_a, _invalid, _stats, _warnings) =
            load_corpus(path_a.to_str().unwrap()).unwrap();

        let path_b = dir.join("keyed_first.jsonl");
        std::fs::write(&path_b, keyed_first).unwrap();
        let (reactions_b, _invalid, _stats, _warnings) =
            load_corpus(path_b.to_str().unwrap()).unwrap();

        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(reactions_a.len(), 1);
        assert_eq!(reactions_b.len(), 1);
        assert_eq!(
            reactions_a[0].group_key, "explicit-patent-family-1",
            "the explicit group_key must win regardless of which row it \
             appeared on first"
        );
        assert_eq!(
            reactions_a[0].group_key, reactions_b[0].group_key,
            "group_key must be independent of row order"
        );
    }

    #[test]
    fn genuinely_conflicting_explicit_group_keys_reject_the_whole_reaction() {
        // Unlike the same-key-different-order case above, two duplicate
        // rows with two DIFFERENT non-empty explicit group_key values can't
        // be resolved by picking one -- neither is more trustworthy than
        // the other, and guessing risks a real leakage-safety violation.
        // The whole reaction must be rejected, not silently kept under
        // either candidate key.
        let dir = std::env::temp_dir().join(format!(
            "renkin-forward-bench-conflicting-group-key-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("corpus.jsonl");
        let content = r#"
{"schema_version": 1, "reaction_id": "a", "reactants": ["CCO"], "accepted_products": [["CC=O"]], "group_key": "patent-family-1"}
{"schema_version": 1, "reaction_id": "b", "reactants": ["CCO"], "accepted_products": [["CC=O"]], "group_key": "patent-family-2"}
"#;
        std::fs::write(&path, content).unwrap();

        let (reactions, invalid, stats, _warnings) = load_corpus(path.to_str().unwrap()).unwrap();
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(
            reactions.len(),
            0,
            "the conflicted reaction must not appear in the returned reactions"
        );
        assert_eq!(stats.conflicting_group_keys, 1);
        assert_eq!(stats.reactions_loaded, 0);
        assert!(
            invalid.iter().any(|a| a.reason == "conflicting_group_key"),
            "must be reported as an InvalidReactionAttempt, not silently dropped: {invalid:?}"
        );
    }

    #[test]
    fn reused_reaction_id_for_a_different_reaction_is_rejected() {
        // `reaction_id` is documented as not used for identity/splitting
        // precisely because corpora can't be trusted to keep it unique --
        // reusing it for a genuinely different reaction must reject the
        // later row, not silently accept a non-unique key.
        let dir = std::env::temp_dir().join(format!(
            "renkin-forward-bench-conflicting-reaction-id-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("corpus.jsonl");
        let content = r#"
{"schema_version": 1, "reaction_id": "dup-id", "reactants": ["CCO"], "accepted_products": [["CC=O"]]}
{"schema_version": 1, "reaction_id": "dup-id", "reactants": ["CC(C)O"], "accepted_products": [["CC(C)=O"]]}
"#;
        std::fs::write(&path, content).unwrap();

        let (reactions, invalid, stats, _warnings) = load_corpus(path.to_str().unwrap()).unwrap();
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(
            reactions.len(),
            1,
            "the first-seen reaction under this reaction_id is kept"
        );
        assert_eq!(stats.conflicting_reaction_ids, 1);
        assert!(
            invalid
                .iter()
                .any(|a| a.reason == "conflicting_reaction_id"),
            "the second, conflicting row must be reported, not silently dropped: {invalid:?}"
        );
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

        // This exact rank is sensitive to default_rules()'s composition
        // (candidate generation order shifts whenever a rule is added or
        // removed) -- was 6 before v0.36.0's negishi_retro/
        // grignard_addition_retro removal dropped it to 4. Re-derive, don't
        // guess, if this fails after a future rule-set change.
        let row = compute_row(&reaction, &rules, &provenance).unwrap();
        assert_eq!(row.best_correct_rank, Some(4));

        // The actual invariant this test protects: feeding the canonical
        // (pre-sorted) rewrite instead of the original text must give a
        // *different* result -- here, losing the correct candidate
        // entirely -- proving `compute_row` genuinely depends on which
        // reactant spelling/order it's given, not that it happens to not
        // matter for this fixture.
        let mut canonical_reaction = reaction.clone();
        canonical_reaction.reactants_original = canonical_reaction.reactants_canonical.clone();
        let canonical_row = compute_row(&canonical_reaction, &rules, &provenance).unwrap();
        assert_ne!(canonical_row.best_correct_rank, row.best_correct_rank);
    }

    fn ndcg_probe_row(accepted_count: usize, correct_ranks_top10: Vec<usize>) -> BenchRow {
        let provenance = RowProvenance {
            renkin_forward_version: "test".to_string(),
            template_source: "embedded".to_string(),
            rules_file_sha256: None,
        };
        let accepted_products_canonical: Vec<Vec<String>> = (0..accepted_count)
            .map(|i| vec![format!("dummy-{i}")])
            .collect();
        let (accepted_product_count_min, accepted_product_count_max, accepted_product_count_mixed) =
            accepted_product_arity(&accepted_products_canonical);
        let correct_candidate_present = !correct_ranks_top10.is_empty();
        let best_correct_rank = correct_ranks_top10.first().copied();
        let candidate_count = 10;
        let proposal_status =
            proposal_status_for(correct_candidate_present, candidate_count, false);
        let ranking_status = ranking_status_for(proposal_status, best_correct_rank);
        let stereo_status = stereo_status_for(true, false, StereoIgnoredOutcome::NoHit);
        let failure_reason =
            derive_failure_reason(InputStatus::Valid, proposal_status, ranking_status);
        let row = BenchRow {
            reaction_id: "r".to_string(),
            source_line: 1,
            split: "train".to_string(),
            leakage_group_id: Some("g".to_string()),
            reaction_class: None,
            reactants_original: vec![],
            reactants_canonical: vec![],
            accepted_products_canonical,
            num_reactants: 1,
            accepted_product_count_min,
            accepted_product_count_max,
            accepted_product_count_mixed,
            has_stereochemistry: false,
            candidate_count,
            raw_outcomes: 10,
            correct_candidate_present,
            best_correct_rank,
            correct_ranks_top10,
            best_correct_rank_stereo_ignored: None,
            top1_hit: false,
            top5_hit: false,
            top10_hit: false,
            stereochemistry_aware_hit: false,
            stereochemistry_ignored_outcome: StereoIgnoredOutcome::NoHit,
            invalid_candidate_count: 0,
            no_op_candidate_count: 0,
            application_warning_count: 0,
            application_error_count: 0,
            templates_attempted: 1,
            templates_matched: 1,
            graph_rules_skipped: 0,
            rules_loaded: 1,
            elapsed_ms: 1.0,
            failure_reason,
            input_status: InputStatus::Valid,
            proposal_status,
            ranking_status,
            stereo_status,
            provenance,
        };
        check_status_consistency(&row).expect("ndcg_probe_row must build a consistent BenchRow");
        row
    }

    /// A ranking that surfaces BOTH accepted outcomes in the top 2 must
    /// score the same as a perfect single-outcome ranking (NDCG = 1.0):
    /// DCG matches IDCG exactly when every ideal slot is filled.
    #[test]
    fn ndcg_at_10_is_perfect_when_every_accepted_outcome_is_found_at_the_top() {
        let row = ndcg_probe_row(2, vec![0, 1]);
        let rows = [&row];
        let metrics = hit_rate_metrics(&rows, false);
        assert!((metrics.ndcg_at_10.unwrap() - 1.0).abs() < 1e-9);
    }

    /// Regression for the single-relevant-item bug: with 2 accepted
    /// outcomes but only 1 found (at rank 1), single-relevant-item NDCG
    /// would have reported `1/log2(3) ~= 0.631` (assuming exactly one
    /// outcome was ever possible). The correct multi-positive NDCG divides
    /// by the true ideal DCG for 2 accepted outcomes, giving a lower,
    /// honestly-worse score for a ranking that only found half the ground
    /// truth.
    #[test]
    fn ndcg_at_10_scores_lower_when_only_some_accepted_outcomes_are_found() {
        let row = ndcg_probe_row(2, vec![1]);
        let rows = [&row];
        let metrics = hit_rate_metrics(&rows, false);
        let dcg = 1.0 / (1.0f64 + 2.0).log2();
        let idcg = 1.0 / 2.0f64.log2() + 1.0 / 3.0f64.log2();
        let expected = dcg / idcg;
        assert!((metrics.ndcg_at_10.unwrap() - expected).abs() < 1e-9);
        assert!(
            expected < 1.0 / 3.0f64.log2(),
            "must score lower than the old single-relevant-item formula would have"
        );
    }

    #[test]
    fn aggregate_computes_conditional_and_end_to_end_separately() {
        let provenance = RowProvenance {
            renkin_forward_version: "test".to_string(),
            template_source: "embedded".to_string(),
            rules_file_sha256: None,
        };
        let mk = |rank: Option<usize>, candidate_count: usize| {
            let correct_candidate_present = rank.is_some();
            let stereochemistry_ignored_outcome = if rank.is_some_and(|r| r < 10) {
                StereoIgnoredOutcome::Hit
            } else {
                StereoIgnoredOutcome::NoHit
            };
            let proposal_status =
                proposal_status_for(correct_candidate_present, candidate_count, false);
            let ranking_status = ranking_status_for(proposal_status, rank);
            let stereo_status = stereo_status_for(
                true,
                rank.is_some_and(|r| r < 10),
                stereochemistry_ignored_outcome,
            );
            let failure_reason =
                derive_failure_reason(InputStatus::Valid, proposal_status, ranking_status);
            let row = BenchRow {
                reaction_id: "r".to_string(),
                source_line: 1,
                split: "train".to_string(),
                leakage_group_id: Some("g".to_string()),
                reaction_class: None,
                reactants_original: vec![],
                reactants_canonical: vec![],
                // A valid row (this fixture is never `input_invalid`) always has
                // at least one accepted product multiset -- `ndcg_at_10`'s ideal
                // DCG divides by this count, so it must not be empty.
                accepted_products_canonical: vec![vec!["dummy".to_string()]],
                num_reactants: 1,
                accepted_product_count_min: 1,
                accepted_product_count_max: 1,
                accepted_product_count_mixed: false,
                has_stereochemistry: false,
                candidate_count,
                raw_outcomes: candidate_count,
                correct_candidate_present,
                best_correct_rank: rank,
                correct_ranks_top10: rank.filter(|&r| r < 10).into_iter().collect(),
                best_correct_rank_stereo_ignored: rank,
                top1_hit: rank == Some(0),
                top5_hit: rank.is_some_and(|r| r < 5),
                top10_hit: rank.is_some_and(|r| r < 10),
                stereochemistry_aware_hit: rank.is_some_and(|r| r < 10),
                stereochemistry_ignored_outcome,
                invalid_candidate_count: 0,
                no_op_candidate_count: 0,
                application_warning_count: 0,
                application_error_count: 0,
                templates_attempted: 1,
                templates_matched: 1,
                graph_rules_skipped: 0,
                rules_loaded: 1,
                elapsed_ms: 1.0,
                failure_reason,
                input_status: InputStatus::Valid,
                proposal_status,
                ranking_status,
                stereo_status,
                provenance: provenance.clone(),
            };
            check_status_consistency(&row).expect("mk() must build a consistent BenchRow");
            row
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

    #[test]
    fn accepted_product_arity_single_outcome() {
        let (min, max, mixed) = accepted_product_arity(&[vec!["a".to_string()]]);
        assert_eq!((min, max, mixed), (1, 1, false));
    }

    #[test]
    fn accepted_product_arity_multiple_outcomes_same_arity() {
        let (min, max, mixed) = accepted_product_arity(&[
            vec!["a".to_string(), "b".to_string()],
            vec!["c".to_string(), "d".to_string()],
        ]);
        assert_eq!((min, max, mixed), (2, 2, false));
    }

    #[test]
    fn accepted_product_arity_multiple_outcomes_different_arity() {
        let (min, max, mixed) = accepted_product_arity(&[
            vec!["a".to_string()],
            vec!["b".to_string(), "c".to_string()],
        ]);
        assert_eq!((min, max, mixed), (1, 2, true));
    }

    #[test]
    fn accepted_product_arity_is_independent_of_outer_list_order() {
        let forward = vec![
            vec!["a".to_string()],
            vec!["b".to_string(), "c".to_string()],
            vec!["d".to_string(), "e".to_string(), "f".to_string()],
        ];
        let mut reversed = forward.clone();
        reversed.reverse();
        assert_eq!(
            accepted_product_arity(&forward),
            accepted_product_arity(&reversed)
        );
    }

    #[test]
    fn accepted_product_arity_empty_is_zero_zero_false() {
        // Only an InvalidReactionAttempt row (no accepted-products info at
        // all) ever has an empty slice -- see `product_arity_bucket`'s
        // "<missing>" handling.
        assert_eq!(accepted_product_arity(&[]), (0, 0, false));
    }

    #[test]
    fn product_arity_bucket_labels() {
        let row = ndcg_probe_row(1, vec![0]);

        let mut single = row.clone();
        single.accepted_products_canonical = vec![vec!["a".to_string()]];
        single.accepted_product_count_min = 1;
        single.accepted_product_count_max = 1;
        single.accepted_product_count_mixed = false;
        assert_eq!(product_arity_bucket(&single), "1");

        let mut triple_plus = single.clone();
        triple_plus.accepted_product_count_min = 4;
        triple_plus.accepted_product_count_max = 4;
        assert_eq!(product_arity_bucket(&triple_plus), "3+");

        let mut mixed = single.clone();
        mixed.accepted_product_count_min = 1;
        mixed.accepted_product_count_max = 2;
        mixed.accepted_product_count_mixed = true;
        assert_eq!(product_arity_bucket(&mixed), "mixed:1-2");

        let mut mixed_big = mixed.clone();
        mixed_big.accepted_product_count_max = 5;
        assert_eq!(product_arity_bucket(&mixed_big), "mixed:1-3+");

        let mut missing = single.clone();
        missing.accepted_products_canonical = Vec::new();
        assert_eq!(product_arity_bucket(&missing), "<missing>");
    }

    #[test]
    fn duplicate_accepted_product_dedup_preserves_within_multiset_multiplicity_for_arity() {
        // Regression guard tying C4's outer-list dedup to C6's arity fields:
        // a 2-product outcome listed twice must dedupe to ONE accepted
        // outcome of arity 2, not silently collapse multiplicity within
        // that outcome itself.
        let dir = std::env::temp_dir().join(format!(
            "renkin-forward-bench-dup-arity-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("corpus.jsonl");
        let content = r#"
{"schema_version": 1, "reaction_id": "dup-two-product", "reactants": ["CCO"], "accepted_products": [["CC=O", "O"], ["C(C)=O", "O"]]}
"#;
        std::fs::write(&path, content).unwrap();

        let (reactions, _invalid, _stats, _warnings) = load_corpus(path.to_str().unwrap()).unwrap();
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(reactions.len(), 1);
        assert_eq!(reactions[0].accepted_products_canonical.len(), 1);
        assert_eq!(reactions[0].accepted_products_canonical[0].len(), 2);
        let (min, max, mixed) = accepted_product_arity(&reactions[0].accepted_products_canonical);
        assert_eq!((min, max, mixed), (2, 2, false));
    }

    #[test]
    fn derive_failure_reason_is_uniquely_derivable_from_orthogonal_statuses() {
        let cases = [
            (
                InputStatus::Invalid,
                ProposalStatus::NotAttempted,
                RankingStatus::NotApplicable,
                FailureReason::InputInvalid,
            ),
            (
                InputStatus::Valid,
                ProposalStatus::Error,
                RankingStatus::NotApplicable,
                FailureReason::PredictionError,
            ),
            (
                InputStatus::Valid,
                ProposalStatus::Covered,
                RankingStatus::Top1,
                FailureReason::HitTop1,
            ),
            (
                InputStatus::Valid,
                ProposalStatus::Covered,
                RankingStatus::Top5,
                FailureReason::HitTop5,
            ),
            (
                InputStatus::Valid,
                ProposalStatus::Covered,
                RankingStatus::Top10,
                FailureReason::HitTop10,
            ),
            (
                InputStatus::Valid,
                ProposalStatus::Covered,
                RankingStatus::Beyond10,
                FailureReason::HitBeyond10,
            ),
            (
                InputStatus::Valid,
                ProposalStatus::MissedEmptyPool,
                RankingStatus::NotApplicable,
                FailureReason::CorrectAbsentEmptyPool,
            ),
            (
                InputStatus::Valid,
                ProposalStatus::MissedNonemptyPool,
                RankingStatus::NotApplicable,
                FailureReason::CorrectAbsentNonemptyPool,
            ),
        ];
        for (input_status, proposal_status, ranking_status, expected) in cases {
            assert_eq!(
                derive_failure_reason(input_status, proposal_status, ranking_status),
                expected,
                "input_status={input_status:?} proposal_status={proposal_status:?} \
                 ranking_status={ranking_status:?}"
            );
        }
    }

    #[test]
    fn proposal_status_and_ranking_status_are_orthogonal_to_stereo_status() {
        // A row with proposal_status=covered/ranking_status=top1 can still
        // have any stereo_status -- ranking and stereochemistry are
        // independent dimensions.
        let correct_candidate_present = true;
        let proposal_status = proposal_status_for(correct_candidate_present, 3, false);
        let ranking_status = ranking_status_for(proposal_status, Some(0));
        assert_eq!(proposal_status, ProposalStatus::Covered);
        assert_eq!(ranking_status, RankingStatus::Top1);

        for (aware_hit, ignored_outcome, expected) in [
            (true, StereoIgnoredOutcome::Hit, StereoStatus::ExactHit),
            (
                false,
                StereoIgnoredOutcome::Hit,
                StereoStatus::StereoOnlyHit,
            ),
            (false, StereoIgnoredOutcome::NoHit, StereoStatus::NoHit),
            (
                false,
                StereoIgnoredOutcome::Unsupported,
                StereoStatus::Unsupported,
            ),
        ] {
            assert_eq!(
                stereo_status_for(true, aware_hit, ignored_outcome),
                expected
            );
        }
    }

    #[test]
    fn check_status_consistency_rejects_input_invalid_with_a_ranked_proposal() {
        let mut row = ndcg_probe_row(1, vec![0]);
        row.input_status = InputStatus::Invalid;
        // proposal_status/ranking_status/stereo_status still say "covered,
        // top1" -- contradicts input_status=invalid.
        assert!(check_status_consistency(&row).is_err());
    }

    #[test]
    fn check_status_consistency_rejects_covered_without_a_rank() {
        let mut row = ndcg_probe_row(1, vec![0]);
        row.ranking_status = RankingStatus::NotApplicable;
        assert!(check_status_consistency(&row).is_err());
    }

    #[test]
    fn check_status_consistency_rejects_covered_disagreeing_with_best_correct_rank() {
        let mut row = ndcg_probe_row(1, vec![]);
        // proposal_status/ranking_status still say "covered, top1" from the
        // helper's own construction path is bypassed here -- force the
        // mismatch directly.
        row.proposal_status = ProposalStatus::Covered;
        row.ranking_status = RankingStatus::Top1;
        assert!(row.best_correct_rank.is_none());
        assert!(check_status_consistency(&row).is_err());
    }

    #[test]
    fn check_status_consistency_accepts_every_fixture_this_module_builds() {
        // Cross-status-consistency sweep: every fixture-building helper in
        // this test module must produce a self-consistent BenchRow.
        for row in [
            ndcg_probe_row(1, vec![0]),
            ndcg_probe_row(2, vec![0, 1]),
            ndcg_probe_row(2, vec![1]),
            ndcg_probe_row(0, vec![]),
        ] {
            check_status_consistency(&row).unwrap();
        }
    }
}
