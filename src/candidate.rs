//! Candidate proposal/selection separation inspired by:
//!
//! Pappala et al. (2026), "RETROSPECT: RETROsynthesis via
//! Sequential Prediction, and Chemically Transformed-ranking",
//! arXiv:2606.07181, doi:10.48550/arXiv.2606.07181.
//!
//! Independent RENKIN implementation. No upstream source code copied.
//!
//! `find_routes` (`crate::search`) and the standalone [`propose_one_step`]
//! API both go through [`raw_propose`], so route search and offline
//! candidate-pool generation see the same underlying rule-application
//! results *for whatever active-rule set they were each given* -- see the
//! important caveat below.
//!
//! **`propose_one_step` does NOT always see the same candidate set as
//! `find_routes`.** When an NN template scorer is configured,
//! `TemplateScorer::top_k_indices` does not just reorder rules, it *reduces*
//! the file-template set to the top-K by predicted relevance, and
//! `find_routes` only ever calls `apply_retro` on that reduced set. So with
//! a scorer active, `find_routes`'s candidate set is a strict subset of what
//! `propose_one_step(..., ProposalMode::Exhaustive)` would produce -- not
//! merely a reordering of the same set. [`ProposalMode`] makes this explicit
//! instead of leaving it implicit: `Exhaustive` (all rules, offline-only,
//! maximum coverage for reranker training), `BondIndexed` (mirrors
//! `--bond-index` retrieval), and `ScorerConditioned` (mirrors an active NN
//! scorer, using caller-supplied scores so this module never has to own a
//! `TemplateScorer` itself). Evaluating a reranker on an `Exhaustive` pool
//! answers "how good is the reranker at selection, given everything to
//! select from"; it does NOT by itself demonstrate that hooking the
//! reranker into the current NN-scorer-gated runtime search would reproduce
//! that improvement -- that requires a separate `ScorerConditioned`
//! evaluation (see `docs/guides/reranker-candidate-pools.md`).
//!
//! `find_routes` builds its own per-application step costs directly from
//! [`RawCandidate`] (unmerged, one entry per rule application) and never
//! calls [`propose_one_step`] itself -- canonical-precursor-set merging is a
//! candidate-pool-only concept; the search's own state-space dedup (frontier
//! `state_hash` in `closed`) is a different, coarser mechanism that already
//! serves the search's purposes. This module must never change what
//! `find_routes` computes.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::chem_env::{
    Molecule, PrecursorMol, RetroRule, TemplateBondIndex, apply_retro, mol_from_smiles,
    to_canonical,
};
use crate::score::step_cost;
#[cfg(test)]
use crate::search::is_extracted_template;

/// Why a `ScoredRuleRef`'s `upstream_score` is `Some`/`None`. Distinct from
/// `TemplateScoreStatus` (a whole-scoring-call status): this is attached to
/// *each rule*, since `Exhaustive`/`BondIndexed` modes and hand-crafted rules
/// within `ScorerConditioned` mode never go through a scorer at all --
/// that's a different situation from a scorer being configured and failing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpstreamScoreStatus {
    /// A real scorer output is attached.
    Available,
    /// This proposal mode does not use a scorer (`Exhaustive`, `BondIndexed`,
    /// or a hand-crafted rule that a scorer never scores in the first place).
    NotApplicable,
    ModelNotConfigured,
    TargetParseFailed,
    InferenceFailed,
    OutputShapeMismatch,
}

/// One active rule selected for proposal, with its upstream scoring
/// provenance intact (never collapsed to just an index or just an order).
pub struct ScoredRuleRef<'a> {
    pub rule: &'a RetroRule,
    /// This rule's position within *this proposal mode's* selection order
    /// (0 = first considered). Defined per-mode, not globally -- see
    /// `ProposalMode` doc.
    pub source_rank: usize,
    pub upstream_score: Option<f32>,
    pub upstream_score_status: UpstreamScoreStatus,
}

/// Rule-selection mode for one-step proposal generation. `original_rank` on
/// the resulting candidates is always "rank within this mode's own
/// selection", not a global rank -- comparing `original_rank` across two
/// pools generated under different modes is not meaningful.
pub enum ProposalMode {
    /// Every rule is tried. Offline-only: maximum-coverage pool for
    /// evaluating a reranker's own selection ability, independent of
    /// whatever the runtime search's active retrieval narrows to.
    Exhaustive,
    /// Mirrors `--bond-index`: `TemplateBondIndex::retrieve` selects active
    /// rules for this target molecule. `top_k` is forwarded to `retrieve`
    /// unchanged (0 = no truncation, matching `find_routes`' own usage).
    BondIndexed { top_k: usize },
    /// Mirrors an active NN template scorer. `input` must be pre-computed
    /// by the caller (e.g. from `TemplateScorer::score_templates`'s output)
    /// -- this module never owns a scorer, so it can't silently fail to
    /// configure one. Hand-crafted rules ([0, `input.rules_offset`)) are
    /// always included regardless of `input.scores`, matching
    /// `TemplateScorer`'s own `rules_offset` convention exactly --
    /// classification is by POSITION in `rules`, never by rule-name prefix.
    ScorerConditioned {
        input: ScorerConditionedInput,
        top_k: usize,
    },
}

/// Caller-supplied scorer output for [`ProposalMode::ScorerConditioned`].
/// Deliberately NOT gated behind the `nn-scoring` feature -- this module
/// never owns a `TemplateScorer` (see module doc), so it only needs the
/// *shape* of a scorer's output, not the scorer implementation itself. A
/// caller with a real `TemplateScorer` (behind `nn-scoring`) converts its
/// `TemplateScoreOutput` into this shape. Keeping one ungated shape (rather
/// than the previous `#[cfg(...)]`-duplicated `scores` field) also means
/// this mode's selection logic and its tests exercise exactly one code
/// path regardless of which features are compiled in, instead of leaving
/// whichever branch isn't the default-build's silently untested.
#[derive(Debug, Clone)]
pub struct ScorerConditionedInput {
    /// (rule_index, raw_logit, rank) per scored file template. `rule_index`
    /// is an absolute index into the `rules` slice passed to
    /// `propose_one_step`. Empty unless `status ==
    /// UpstreamScoreStatus::Available`.
    pub scores: Vec<(usize, f32, usize)>,
    /// Never `NotApplicable` here -- that variant means "no scorer was used
    /// at all" (`Exhaustive`/`BondIndexed`), which does not describe a
    /// caller that explicitly chose `ScorerConditioned` mode. A `status`
    /// other than `Available` makes `propose_one_step` fail closed (return
    /// `Err`) rather than silently narrowing to zero file templates as if
    /// the scorer had succeeded with nothing relevant to offer.
    pub status: UpstreamScoreStatus,
    /// Count of rules at the start of `rules` that are hand-crafted and
    /// always included, mirroring `TemplateScorer::rules_offset` exactly.
    pub rules_offset: usize,
    /// Free-form identity for the scorer that produced `scores` (e.g. a
    /// model file path or name), so a pool manifest can tell two
    /// scorer-conditioned pools apart even when both have `status:
    /// Available`.
    pub scorer_identity: String,
    /// SHA-256 of the scorer model file's bytes, so a manifest can detect a
    /// silently-swapped model between training and evaluation.
    pub scorer_model_sha256: String,
}

pub struct ProposalConfig {
    pub mode: ProposalMode,
}

impl Default for ProposalConfig {
    fn default() -> Self {
        Self {
            mode: ProposalMode::Exhaustive,
        }
    }
}

/// Extracted, structured candidate features (see `feature_schema` module,
/// added in a later commit). Order is fixed by `FeatureSchema`; `missing[i]`
/// is true when `values[i]` could not be computed and must be treated as
/// missing (not zero) by any consumer, including the reranker.
#[derive(Debug, Clone, Default)]
pub struct CandidateFeatures {
    pub values: Vec<f32>,
    pub missing: Vec<bool>,
}

/// One rule application's provenance, retained in full even after its
/// precursor set is merged with other applications that produced the same
/// canonical outcome. `base_step_cost` is the plain chemistry-only step cost
/// (`crate::score::step_cost`, no `reaction_prior`/`template_bonus` applied)
/// -- an intrinsic property of the precursor set, not of whatever prior a
/// particular search run happened to configure.
#[derive(Debug, Clone)]
pub struct CandidateSource {
    pub template_id: String,
    pub rule_name: String,
    pub original_rank: usize,
    pub upstream_score: Option<f32>,
    /// Why `upstream_score` is `Some`/`None` -- kept alongside the score
    /// itself (not just on `ScoredRuleRef`, which is consumed before this
    /// struct is built) so exported provenance can distinguish "no scorer
    /// involved" from "a scorer was involved and produced this score".
    pub upstream_score_status: UpstreamScoreStatus,
    /// `RetroRule.weight` (`ln(count+1)` for extracted templates, 1.0 for
    /// hand-crafted rules), named `_raw` because it is NOT train-split-frozen
    /// yet -- that requires split-aware recomputation, added with the full
    /// feature schema/training-pipeline commit. Do not treat this as a
    /// leakage-safe feature on its own.
    pub template_log_frequency_raw: Option<f32>,
    pub base_step_cost: f64,
}

/// A merged candidate: all rule applications that produced the same
/// canonical precursor set for the same target, collapsed into one entry
/// with full source provenance retained (see `merge_into_candidates`).
/// `sources` is sorted deterministically (upstream score descending, then
/// base step cost ascending, then original rank ascending, then template_id
/// lexicographic) -- `sources[0]` after this sort is "the" representative
/// source when exactly one must be picked (e.g. for display), so which
/// source was chosen is never ambiguous.
#[derive(Debug, Clone)]
pub struct ReactionCandidate {
    pub candidate_id: String,
    pub target_smiles: String,
    pub precursor_smiles: Vec<String>,

    pub sources: Vec<CandidateSource>,
    pub source_template_count: usize,
    pub best_upstream_score: Option<f32>,
    pub best_upstream_rank: usize,
    pub min_base_step_cost: f64,
    pub max_template_frequency: Option<f32>,
    pub mean_template_frequency: Option<f32>,

    pub features: CandidateFeatures,
    pub reranker_score: Option<f64>,
}

/// `group_id` is the caller-supplied dataset reaction/example identifier --
/// one LightGBM ranking group. `target_id` is the canonical target
/// structure, used only as the leakage-safe train/val/test split key. The
/// same target structure can appear in multiple dataset examples (e.g. two
/// different literature reactions producing the same product): those share
/// `target_id` (same split) but must each get their own `group_id` (separate
/// ranking groups) -- this struct keeps the two deliberately distinct so a
/// caller can never conflate "the same molecule" with "the same example".
pub struct CandidatePool {
    pub group_id: String,
    pub target_id: String,
    pub target_smiles: String,
    pub candidates: Vec<ReactionCandidate>,
}

pub trait CandidateReranker: Send + Sync {
    fn score_pool(&self, target: &str, candidates: &mut [ReactionCandidate]) -> anyhow::Result<()>;
}

/// Template-level reaction-center features, computed once per template
/// SMIRKS from the template alone (LHS vs RHS atom-mapped bond diff) and
/// cached -- never inferred from an actual (target, precursors) molecule
/// instance, since `apply_retro`'s output molecules have no atom_map left
/// (chematic clears it on the product side during transform), so there is
/// no reliable per-instance atom mapping to diff. Graph-based rules (empty
/// SMIRKS) are never extractable -- their transformation isn't expressed as
/// a SMIRKS bond diff at all, and `extractable = false` must not be papered
/// over by guessing values from the rule name or from what a candidate's
/// molecules happen to look like.
#[derive(Debug, Clone, Copy, Default)]
pub struct TemplateTransformationFeatures {
    pub mapped_atom_count: u32,
    pub unmapped_atom_count: u32,
    pub deleted_bond_count: u32,
    pub added_bond_count: u32,
    pub changed_bond_order_count: u32,
    pub reaction_center_atom_count: u32,
    pub extractable: bool,
}

/// Keyed by `(template_id, sha256(smirks))`, not `template_id` alone --
/// `template_id` is meant to be stable, but keying on it alone would let a
/// caller that (incorrectly) reuses the same `template_id` for two
/// different SMIRKS strings silently read back whichever one was cached
/// first, across calls or even across parallel test threads sharing this
/// process-global cache.
type TransformationCacheKey = (String, String);

fn transformation_cache()
-> &'static Mutex<HashMap<TransformationCacheKey, TemplateTransformationFeatures>> {
    static CACHE: OnceLock<Mutex<HashMap<TransformationCacheKey, TemplateTransformationFeatures>>> =
        OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn smirks_hash(smirks: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(smirks.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// atom_map -> (mol_idx, atom_idx) for one side (reactants or products) of a
/// `Reaction`. `duplicate` is true if the same atom_map number appears on
/// more than one atom within this side -- an ambiguous mapping that must
/// never be silently resolved to "whichever one was inserted last".
struct AtomMapIndex {
    by_map: rustc_hash::FxHashMap<u16, (usize, chematic::core::AtomIdx)>,
    duplicate: bool,
}

fn index_atoms_by_map(mols: &[Molecule]) -> AtomMapIndex {
    let mut by_map = rustc_hash::FxHashMap::default();
    let mut duplicate = false;
    for (mol_idx, mol) in mols.iter().enumerate() {
        for (atom_idx, atom) in mol.atoms() {
            if let Some(map_num) = atom.atom_map
                && by_map.insert(map_num, (mol_idx, atom_idx)).is_some()
            {
                duplicate = true;
            }
        }
    }
    AtomMapIndex { by_map, duplicate }
}

/// Bond key = (min, max) of the two endpoints' atom_map numbers -- globally
/// unique across a multi-component side, unlike `chematic::core::AtomIdx`
/// (which is only unique *within* one molecule; the same `AtomIdx(0)` can
/// name a different atom in each precursor fragment). Only bonds where BOTH
/// endpoints are mapped are included; a bond touching an unmapped atom isn't
/// part of the mapped-skeleton diff.
fn bond_orders_by_atom_map(
    mols: &[Molecule],
    index: &rustc_hash::FxHashMap<u16, (usize, chematic::core::AtomIdx)>,
) -> rustc_hash::FxHashMap<(u16, u16), chematic::core::BondOrder> {
    let mut bonds = rustc_hash::FxHashMap::default();
    for (&map_a, &(mol_idx, atom_idx)) in index {
        let mol = &mols[mol_idx];
        for (neighbor_idx, bond_idx) in mol.neighbors(atom_idx) {
            let Some(map_b) = mol.atom(neighbor_idx).atom_map else {
                continue;
            };
            if map_b <= map_a {
                continue; // visit each edge once, from its lower-numbered endpoint
            }
            bonds.insert((map_a, map_b), mol.bond(bond_idx).order);
        }
    }
    bonds
}

/// A template's reaction-center diff, computed independently of
/// `chematic::rxn::find_reaction_center` -- that function's `broken_bonds`/
/// `formed_bonds`/`changed_atoms` are `AtomIdx` pairs scoped to a single
/// molecule (reactant-side `AtomIdx`s and product-side `AtomIdx`s are
/// different numbering spaces even in the single-component case, and a
/// multi-component precursor side has yet another collision risk *within*
/// that space: `AtomIdx(0)` in precursor fragment 0 is a different atom
/// from `AtomIdx(0)` in precursor fragment 1). Pooling those into one
/// `HashSet<AtomIdx>` (the previous implementation) would silently
/// undercount `reaction_center_atom_count` for any multi-component SMIRKS --
/// which is the common case for a disconnection template. Every quantity
/// here is instead keyed by atom_map number, which is globally unique
/// across the whole reaction by construction.
#[derive(Debug, Clone, Copy, Default)]
struct ReactionCenterDiff {
    deleted_bond_count: u32,
    added_bond_count: u32,
    changed_bond_order_count: u32,
    reaction_center_atom_count: u32,
    extractable: bool,
}

fn compute_reaction_center(rxn: &chematic::rxn::Reaction) -> ReactionCenterDiff {
    let reactant_index = index_atoms_by_map(&rxn.reactants);
    let product_index = index_atoms_by_map(&rxn.products);
    if reactant_index.by_map.is_empty() || reactant_index.duplicate || product_index.duplicate {
        return ReactionCenterDiff::default();
    }

    let reactant_bonds = bond_orders_by_atom_map(&rxn.reactants, &reactant_index.by_map);
    let product_bonds = bond_orders_by_atom_map(&rxn.products, &product_index.by_map);

    let mut deleted = 0u32;
    let mut changed_order = 0u32;
    let mut center_maps: std::collections::HashSet<u16> = std::collections::HashSet::new();
    for (&key, r_order) in &reactant_bonds {
        match product_bonds.get(&key) {
            None => {
                deleted += 1;
                center_maps.insert(key.0);
                center_maps.insert(key.1);
            }
            Some(p_order) if p_order != r_order => {
                changed_order += 1;
                center_maps.insert(key.0);
                center_maps.insert(key.1);
            }
            _ => {}
        }
    }
    let mut added = 0u32;
    for &key in product_bonds.keys() {
        if !reactant_bonds.contains_key(&key) {
            added += 1;
            center_maps.insert(key.0);
            center_maps.insert(key.1);
        }
    }

    for (&map_num, &(r_mol_idx, r_atom_idx)) in &reactant_index.by_map {
        if let Some(&(p_mol_idx, p_atom_idx)) = product_index.by_map.get(&map_num) {
            let r_atom = rxn.reactants[r_mol_idx].atom(r_atom_idx);
            let p_atom = rxn.products[p_mol_idx].atom(p_atom_idx);
            if r_atom.element != p_atom.element
                || r_atom.charge != p_atom.charge
                || r_atom.aromatic != p_atom.aromatic
            {
                center_maps.insert(map_num);
            }
        }
    }

    ReactionCenterDiff {
        deleted_bond_count: deleted,
        added_bond_count: added,
        changed_bond_order_count: changed_order,
        reaction_center_atom_count: center_maps.len() as u32,
        extractable: true,
    }
}

/// Compute (and cache) template-level transformation features for one rule.
/// Keyed by `(template_id, sha256(smirks))` -- see
/// [`TransformationCacheKey`]'s doc for why `template_id` alone isn't safe.
pub fn template_transformation_features(rule: &RetroRule) -> TemplateTransformationFeatures {
    let cache_key = (rule.template_id.clone(), smirks_hash(&rule.smirks));
    if let Some(cached) = transformation_cache().lock().unwrap().get(&cache_key) {
        return *cached;
    }

    let features = if rule.smirks.is_empty() {
        TemplateTransformationFeatures {
            extractable: false,
            ..Default::default()
        }
    } else {
        match chematic::rxn::parse_reaction(&rule.smirks) {
            Ok(rxn) => {
                let has_atom_map = rxn
                    .reactants
                    .iter()
                    .any(|m| m.atoms().any(|(_, a)| a.atom_map.is_some()));
                if !has_atom_map {
                    TemplateTransformationFeatures {
                        extractable: false,
                        ..Default::default()
                    }
                } else {
                    let mapped_atom_count = rxn
                        .reactants
                        .iter()
                        .flat_map(|m| m.atoms())
                        .filter(|(_, a)| a.atom_map.is_some())
                        .count() as u32;
                    let unmapped_atom_count = rxn
                        .reactants
                        .iter()
                        .flat_map(|m| m.atoms())
                        .filter(|(_, a)| a.atom_map.is_none())
                        .count() as u32;
                    let center = compute_reaction_center(&rxn);
                    if !center.extractable {
                        // Ambiguous duplicate atom_map numbers on one side --
                        // never guess which occurrence was intended.
                        TemplateTransformationFeatures {
                            mapped_atom_count,
                            unmapped_atom_count,
                            extractable: false,
                            ..Default::default()
                        }
                    } else {
                        TemplateTransformationFeatures {
                            mapped_atom_count,
                            unmapped_atom_count,
                            deleted_bond_count: center.deleted_bond_count,
                            added_bond_count: center.added_bond_count,
                            changed_bond_order_count: center.changed_bond_order_count,
                            reaction_center_atom_count: center.reaction_center_atom_count,
                            extractable: true,
                        }
                    }
                }
            }
            Err(_) => TemplateTransformationFeatures {
                extractable: false,
                ..Default::default()
            },
        }
    };

    transformation_cache()
        .lock()
        .unwrap()
        .insert(cache_key, features);
    features
}

/// Deterministic aggregate of per-source template-transformation features
/// for a merged candidate with multiple sources. `_fraction` and `_mean`
/// divide by `features.len()`, which is always >= 1 for a real candidate
/// (a candidate always has at least one source) -- never NaN/Inf.
#[derive(Debug, Clone, Copy, Default)]
pub struct TransformationFeatureAggregate {
    pub reaction_center_atom_count_min: u32,
    pub reaction_center_atom_count_max: u32,
    pub reaction_center_atom_count_mean: f32,
    pub reaction_center_extractable_fraction: f32,
}

pub fn aggregate_transformation_features(
    features: &[TemplateTransformationFeatures],
) -> TransformationFeatureAggregate {
    if features.is_empty() {
        return TransformationFeatureAggregate::default();
    }
    let extractable: Vec<&TemplateTransformationFeatures> =
        features.iter().filter(|f| f.extractable).collect();
    let (min, max, mean) = if extractable.is_empty() {
        (0, 0, 0.0)
    } else {
        let counts: Vec<u32> = extractable
            .iter()
            .map(|f| f.reaction_center_atom_count)
            .collect();
        let min = *counts.iter().min().unwrap();
        let max = *counts.iter().max().unwrap();
        let mean = counts.iter().sum::<u32>() as f32 / counts.len() as f32;
        (min, max, mean)
    };
    TransformationFeatureAggregate {
        reaction_center_atom_count_min: min,
        reaction_center_atom_count_max: max,
        reaction_center_atom_count_mean: mean,
        reaction_center_extractable_fraction: extractable.len() as f32 / features.len() as f32,
    }
}

/// Version 1 of the candidate feature schema. Every feature has a stable
/// name and a fixed position in [`CandidateFeatures`]'s `values`/`missing`
/// -- look up a position with [`feature_index`], never hardcode a raw index
/// in a consumer, since a later schema version may add or reorder features
/// under a new version number.
///
/// Two groups, split by leakage exposure -- this is the reason
/// [`CandidateFeatures::missing`] exists, not an incidental detail:
///
/// - **Group 1** (`FEATURE_NAMES_V1[..FEATURE_GROUP1_LEN]`): structural,
///   chemistry-integrity, and reaction-center features. Computable from
///   (target, precursors, template) alone, so [`extract_features`] always
///   attempts these regardless of what else the caller supplies.
/// - **Group 2** (the remainder): availability (depends on a stock/building
///   -block library) and template-frequency (depends on which train split
///   a given template's count was observed in -- see
///   [`CandidateSource::template_log_frequency_raw`]'s doc). These stay
///   `missing` until the caller supplies the corpus-dependent input
///   (`stock`) they need; a pool exported before stock/split-freezing lands
///   must never silently ship a leakage-contaminated or wrong-stock value
///   as if it were real.
pub const FEATURE_SCHEMA_VERSION: u32 = 1;

pub const FEATURE_NAMES_V1: &[&str] = &[
    // -- structural (group 1) --
    "num_precursors",
    "target_heavy_atom_count",
    "precursor_heavy_atom_count_sum",
    "precursor_heavy_atom_count_max",
    // Not the chemistry "atom economy" (MW(product)/ΣMW(reagents), see
    // `search.rs::RouteStep::atom_economy`) -- this is a heavy-atom-COUNT
    // ratio, a cheaper, MW-free proxy. Named accordingly so the two are
    // never confused in a feature-importance report or a doc.
    "heavy_atom_retention_ratio",
    // -- chemistry-integrity (group 1) --
    "net_charge_balanced",
    "no_heavy_atom_gain",
    // -- reaction-center / provenance (group 1) --
    "source_template_count",
    "reaction_center_atom_count_min",
    "reaction_center_atom_count_max",
    "reaction_center_atom_count_mean",
    "reaction_center_extractable_fraction",
    "min_base_step_cost",
    // Missing whenever no source has an upstream score -- always the case
    // under ProposalMode::Exhaustive/BondIndexed (no scorer involved at
    // all). This is a mode-dependent absence, not a leakage concern, so it
    // stays group 1: unlike the stock/frequency features below, there is no
    // "supply the missing input and it becomes available" story for it.
    "best_upstream_score",
    // -- availability (group 2 -- stock-dependent) --
    "fraction_precursors_in_stock",
    "all_precursors_in_stock",
    // -- frequency (group 2 -- train-split-dependent) --
    "max_template_log_frequency",
    "mean_template_log_frequency",
];

/// Number of group-1 (always-attempted) features at the front of
/// `FEATURE_NAMES_V1`. Everything from this index onward is group 2.
pub const FEATURE_GROUP1_LEN: usize = 14;

/// Stable index of a named feature within [`FEATURE_NAMES_V1`], or `None` if
/// the name isn't in this schema version.
pub fn feature_index(name: &str) -> Option<usize> {
    FEATURE_NAMES_V1.iter().position(|&n| n == name)
}

/// SHA-256 over `FEATURE_SCHEMA_VERSION` and the exact ordered
/// `FEATURE_NAMES_V1` list, length-prefixed so no separator ambiguity is
/// possible. A consumer (e.g. `scripts/train_reranker.py`, which mirrors
/// this schema in Python since it has no way to import this crate) recomputes
/// this hash from its own copy of the feature names and compares it against
/// a pool manifest's `feature_schema_hash` -- if the two languages' feature
/// lists ever silently drift apart, this is the check that catches it,
/// rather than a length-only comparison that would miss a same-length
/// reorder or rename.
pub fn feature_schema_hash() -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"renkin-retrospect-feature-schema-v1\0");
    hasher.update(FEATURE_SCHEMA_VERSION.to_be_bytes());
    hasher.update((FEATURE_NAMES_V1.len() as u64).to_be_bytes());
    for name in FEATURE_NAMES_V1 {
        hasher.update((name.len() as u64).to_be_bytes());
        hasher.update(name.as_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}

fn heavy_atom_count_and_charge(mol: &Molecule) -> (u32, i64) {
    let mut heavy = 0u32;
    let mut charge = 0i64;
    for (_, atom) in mol.atoms() {
        if atom.element != chematic::core::Element::H {
            heavy += 1;
        }
        charge += i64::from(atom.charge);
    }
    (heavy, charge)
}

/// Extract schema-v1 features for one merged candidate.
///
/// `target_mol` is the already-parsed target molecule (a caller building a
/// pool already has it -- [`propose_one_step`] parses it once per target,
/// not once per candidate).
///
/// `templates_by_id` looks up each source's [`RetroRule`] by `template_id`
/// so [`template_transformation_features`] can be computed (it needs the
/// rule's SMIRKS, which [`CandidateSource`] doesn't itself carry). Build it
/// once per pool-export call with [`index_rules_by_template_id`], not once
/// per candidate.
///
/// `stock` is optional: `None` means "no stock available for this
/// extraction," and every stock-dependent (group 2) feature stays
/// `missing`, never computed against an empty/wrong stock as if that were a
/// real answer.
pub fn extract_features(
    candidate: &ReactionCandidate,
    target_mol: &Molecule,
    templates_by_id: &HashMap<String, &RetroRule>,
    stock: Option<&crate::chem_env::ChemEnv>,
) -> CandidateFeatures {
    let n = FEATURE_NAMES_V1.len();
    let mut values = vec![0.0f32; n];
    let mut missing = vec![false; n];

    // -- structural / chemistry-integrity --
    values[0] = candidate.precursor_smiles.len() as f32;

    let mut precursor_mols: Vec<Molecule> = Vec::with_capacity(candidate.precursor_smiles.len());
    let mut reparse_failed = false;
    for smi in &candidate.precursor_smiles {
        match mol_from_smiles(smi) {
            Ok(m) => precursor_mols.push(m),
            Err(_) => reparse_failed = true,
        }
    }

    if reparse_failed || precursor_mols.is_empty() {
        // Every structural/chemistry-integrity feature past num_precursors
        // depends on re-parsing every precursor SMILES back into a
        // Molecule -- mark them missing rather than computing a partial
        // aggregate over only the precursors that happened to re-parse.
        for m in missing.iter_mut().take(7).skip(1) {
            *m = true;
        }
    } else {
        let (target_heavy, target_charge) = heavy_atom_count_and_charge(target_mol);
        let per_precursor: Vec<(u32, i64)> = precursor_mols
            .iter()
            .map(heavy_atom_count_and_charge)
            .collect();
        let precursor_heavy_sum: u32 = per_precursor.iter().map(|(h, _)| h).sum();
        let precursor_charge_sum: i64 = per_precursor.iter().map(|(_, c)| c).sum();
        let precursor_heavy_max = per_precursor.iter().map(|(h, _)| *h).max().unwrap_or(0);

        values[1] = target_heavy as f32;
        values[2] = precursor_heavy_sum as f32;
        values[3] = precursor_heavy_max as f32;
        if precursor_heavy_sum > 0 {
            values[4] = target_heavy as f32 / precursor_heavy_sum as f32;
        } else {
            missing[4] = true;
        }
        values[5] = if precursor_charge_sum == target_charge {
            1.0
        } else {
            0.0
        };
        // "no_heavy_atom_gain": this is the RETRO direction -- the target is
        // the forward reaction's product, so it must never have MORE heavy
        // atoms than its precursors combined. precursor_heavy_sum >
        // target_heavy is expected and common (reagents/leaving groups) and
        // is not a violation.
        values[6] = if precursor_heavy_sum >= target_heavy {
            1.0
        } else {
            0.0
        };
    }

    // -- reaction-center / provenance (group 1, always attempted) --
    values[7] = candidate.source_template_count as f32;
    let per_source_template_features: Vec<TemplateTransformationFeatures> = candidate
        .sources
        .iter()
        .filter_map(|s| templates_by_id.get(&s.template_id).copied())
        .map(template_transformation_features)
        .collect();
    let agg = aggregate_transformation_features(&per_source_template_features);
    values[8] = agg.reaction_center_atom_count_min as f32;
    values[9] = agg.reaction_center_atom_count_max as f32;
    values[10] = agg.reaction_center_atom_count_mean;
    values[11] = agg.reaction_center_extractable_fraction;
    values[12] = candidate.min_base_step_cost as f32;
    match candidate.best_upstream_score {
        Some(s) => values[13] = s,
        None => missing[13] = true,
    }

    // -- availability (group 2 -- stock-dependent) --
    match stock {
        Some(stock) => {
            let in_stock: Vec<bool> = candidate
                .precursor_smiles
                .iter()
                .map(|smi| stock.is_building_block_smiles(smi))
                .collect();
            let n_in_stock = in_stock.iter().filter(|b| **b).count();
            values[14] = n_in_stock as f32 / in_stock.len().max(1) as f32;
            values[15] = if in_stock.iter().all(|b| *b) {
                1.0
            } else {
                0.0
            };
        }
        None => {
            missing[14] = true;
            missing[15] = true;
        }
    }

    // -- frequency (group 2 -- train-split-dependent) --
    // `template_log_frequency_raw` is populated from `RetroRule.weight` as
    // computed over WHATEVER rule set the caller passed to
    // `propose_one_step`, not necessarily a train-split-frozen recomputation
    // -- always missing until split-aware recomputation lands, per
    // `CandidateSource::template_log_frequency_raw`'s doc.
    missing[16] = true;
    missing[17] = true;

    CandidateFeatures { values, missing }
}

/// Build a `template_id -> &RetroRule` index once per pool-export call, for
/// [`extract_features`] to look up each candidate source's rule by id
/// without an O(rules) scan per source.
///
/// Rejects a `template_id` that appears on two rules with different
/// `name`/`smirks`/`weight`/`required_elements` -- a hard error, since
/// `template_id` is meant to be a stable, unambiguous identity (see
/// `RetroRule::template_id`'s doc); silently keeping "whichever rule was
/// seen first" would let a caller's rule set be internally inconsistent
/// without ever finding out. An exact duplicate (identical on every field)
/// is tolerated, since it names the same rule twice, not two different
/// ones.
pub fn index_rules_by_template_id(
    rules: &[RetroRule],
) -> anyhow::Result<HashMap<String, &RetroRule>> {
    let mut by_id: HashMap<String, &RetroRule> = HashMap::new();
    for rule in rules {
        if let Some(existing) = by_id.get(&rule.template_id) {
            let conflicting = existing.name != rule.name
                || existing.smirks != rule.smirks
                || existing.weight != rule.weight
                || existing.required_elements != rule.required_elements;
            if conflicting {
                anyhow::bail!(
                    "template_id {:?} maps to two different rules: \
                     {{name: {:?}, smirks: {:?}, weight: {}}} vs \
                     {{name: {:?}, smirks: {:?}, weight: {}}}",
                    rule.template_id,
                    existing.name,
                    existing.smirks,
                    existing.weight,
                    rule.name,
                    rule.smirks,
                    rule.weight
                );
            }
            continue;
        }
        by_id.insert(rule.template_id.clone(), rule);
    }
    Ok(by_id)
}

/// Select the active rule set for one target under `mode`, mirroring
/// `find_routes`' bond_idx / scorer fallback chains exactly for the modes
/// that have a `find_routes` analog.
///
/// Fallible: `ScorerConditioned` fails closed (returns `Err`) when
/// `input.status != Available` or `input.scores` contains any entry that
/// fails validation (out-of-bounds/duplicate `rule_index`, duplicate
/// `rank`, non-finite `raw_logit`) -- an invalid or failed scorer input
/// must never silently produce a plausible-looking but wrong active-rule
/// set, or look identical to "the scorer legitimately selected zero file
/// templates".
fn select_active_rules<'a>(
    target_mol: &Molecule,
    rules: &'a [RetroRule],
    mode: &ProposalMode,
) -> anyhow::Result<Vec<ScoredRuleRef<'a>>> {
    match mode {
        ProposalMode::Exhaustive => Ok(rules
            .iter()
            .enumerate()
            .map(|(i, rule)| ScoredRuleRef {
                rule,
                source_rank: i,
                upstream_score: None,
                upstream_score_status: UpstreamScoreStatus::NotApplicable,
            })
            .collect()),
        ProposalMode::BondIndexed { top_k } => {
            let idx = TemplateBondIndex::build(rules);
            Ok(idx
                .retrieve(target_mol, *top_k, rules)
                .into_iter()
                .enumerate()
                .filter_map(|(rank, i)| {
                    rules.get(i).map(|rule| ScoredRuleRef {
                        rule,
                        source_rank: rank,
                        upstream_score: None,
                        upstream_score_status: UpstreamScoreStatus::NotApplicable,
                    })
                })
                .collect())
        }
        ProposalMode::ScorerConditioned { input, top_k } => {
            if input.status != UpstreamScoreStatus::Available {
                anyhow::bail!(
                    "ScorerConditioned proposal mode requires a successful \
                     scorer (status: Available), got {:?} -- failing closed \
                     rather than silently narrowing to zero file templates \
                     as if the scorer had succeeded",
                    input.status
                );
            }
            let offset = input.rules_offset.min(rules.len());
            let mut result: Vec<ScoredRuleRef> = rules
                .iter()
                .enumerate()
                .take(offset)
                .map(|(i, rule)| ScoredRuleRef {
                    rule,
                    source_rank: i,
                    upstream_score: None,
                    upstream_score_status: UpstreamScoreStatus::NotApplicable,
                })
                .collect();

            let mut seen_indices = std::collections::HashSet::new();
            let mut seen_ranks = std::collections::HashSet::new();
            for &(rule_index, raw_logit, rank) in &input.scores {
                if rule_index < offset || rule_index >= rules.len() {
                    anyhow::bail!(
                        "ScorerConditioned scores entry has rule_index {rule_index} \
                         out of bounds [{offset}, {})",
                        rules.len()
                    );
                }
                if !seen_indices.insert(rule_index) {
                    anyhow::bail!(
                        "ScorerConditioned scores contain duplicate rule_index {rule_index}"
                    );
                }
                if !seen_ranks.insert(rank) {
                    anyhow::bail!("ScorerConditioned scores contain duplicate rank {rank}");
                }
                if !raw_logit.is_finite() {
                    anyhow::bail!(
                        "ScorerConditioned scores entry for rule_index {rule_index} \
                         has a non-finite raw_logit ({raw_logit})"
                    );
                }
            }

            let mut by_rank: Vec<&(usize, f32, usize)> = input.scores.iter().collect();
            by_rank.sort_by_key(|s| s.2);
            let mut rank_counter = offset;
            for &(rule_index, raw_logit, _) in by_rank.into_iter().take(*top_k) {
                if let Some(rule) = rules.get(rule_index) {
                    result.push(ScoredRuleRef {
                        rule,
                        source_rank: rank_counter,
                        upstream_score: Some(raw_logit),
                        upstream_score_status: UpstreamScoreStatus::Available,
                    });
                    rank_counter += 1;
                }
            }
            Ok(result)
        }
    }
}

/// One raw one-step retrosynthetic proposal: a single rule application
/// against a single target, before candidate-level canonical merge.
pub struct RawCandidate {
    pub rule_name: String,
    pub template_id: String,
    pub rule_weight: f64,
    pub original_rank: usize,
    pub upstream_score: Option<f32>,
    pub upstream_score_status: UpstreamScoreStatus,
    pub precursors: Vec<PrecursorMol>,
}

/// Apply every rule in `active_rules` to `target_mol`, exactly as
/// `find_routes`' retro-cache-miss branch does. Shared verbatim by both
/// callers -- see module doc. `active_rules` is the caller's own selection
/// (already mode-specific); this function does not choose rules itself.
pub(crate) fn raw_propose(
    target_mol: &Molecule,
    target_smi: &str,
    active_rules: &[ScoredRuleRef<'_>],
) -> Vec<RawCandidate> {
    let target_elem_mask: u64 = crate::search::elem_mask_from_smiles(target_smi);

    #[cfg(not(target_arch = "wasm32"))]
    let iter = active_rules.par_iter();
    #[cfg(target_arch = "wasm32")]
    let iter = active_rules.iter();

    let raw: Vec<RawCandidate> = iter
        .filter(|r| {
            r.rule.required_elements == 0
                || (target_elem_mask & r.rule.required_elements == r.rule.required_elements)
        })
        .flat_map(|r| {
            apply_retro(target_mol, r.rule)
                .into_iter()
                .filter(|precs| !precs.is_empty() && !precs.iter().any(|p| p.smiles == target_smi))
                .map(|precs| RawCandidate {
                    rule_name: r.rule.name.to_string(),
                    template_id: r.rule.template_id.clone(),
                    rule_weight: r.rule.weight,
                    original_rank: r.source_rank,
                    upstream_score: r.upstream_score,
                    upstream_score_status: r.upstream_score_status,
                    precursors: precs,
                })
                .collect::<Vec<_>>()
        })
        .collect();
    raw
}

/// Hashes a sequence of strings with an unambiguous, length-prefixed
/// framing: the count, then each element as (length, bytes). A plain
/// `.join(".")` is not safe here -- a canonical SMILES can itself contain a
/// `.` (a disconnected salt/ion-pair fragment), so `["C.C", "N"]` and `["C",
/// "C.N"]` would join to the identical string `"C.C.N"` despite being
/// different precursor sets.
fn hash_string_sequence(hasher: &mut Sha256, values: &[String]) {
    hasher.update((values.len() as u64).to_be_bytes());
    for value in values {
        let bytes = value.as_bytes();
        hasher.update((bytes.len() as u64).to_be_bytes());
        hasher.update(bytes);
    }
}

/// Candidate identity: SHA-256 over a domain separator
/// (`renkin-retrospect-candidate-v1`, so a later framing revision can never
/// silently collide with this one), the canonical target, an explicit
/// section separator, then the sorted (not deduplicated -- see call site)
/// precursor multiset.
fn candidate_id_for(canonical_target: &str, precursor_smiles: &[String]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"renkin-retrospect-candidate-v1\0");
    hash_string_sequence(&mut hasher, &[canonical_target.to_string()]);
    hasher.update(b"\0precursors\0");
    hash_string_sequence(&mut hasher, precursor_smiles);
    format!("sha256:{:x}", hasher.finalize())
}

/// Merge sources that share `(template_id, rule_name)` within one
/// candidate's `sources` into a single entry: the same rule can legitimately
/// reach the same merged candidate through more than one distinct
/// application (e.g. a symmetric rule matching at two equivalent sites of a
/// symmetric molecule that happen to produce the identical sorted precursor
/// set) -- without this, `source_template_count` would over-count how many
/// *distinct* rules actually contributed.
///
/// `template_log_frequency_raw` and `upstream_score_status` are required to
/// agree across duplicates of the same `(template_id, rule_name)` -- both
/// are properties of the *rule*, not of a particular application, so by
/// construction every application of one active rule within one
/// `propose_one_step` call carries the same values. A mismatch means the
/// input was already inconsistent (e.g. two different `RetroRule` entries
/// sharing an id/name with different weights); this is a hard error rather
/// than a silent pick of whichever value happened to be seen first.
fn merge_duplicate_sources(sources: Vec<CandidateSource>) -> anyhow::Result<Vec<CandidateSource>> {
    let mut order: Vec<(String, String)> = Vec::new();
    let mut by_key: HashMap<(String, String), CandidateSource> = HashMap::new();

    for s in sources {
        let key = (s.template_id.clone(), s.rule_name.clone());
        match by_key.get_mut(&key) {
            None => {
                order.push(key.clone());
                by_key.insert(key, s);
            }
            Some(existing) => {
                if existing.template_log_frequency_raw != s.template_log_frequency_raw {
                    anyhow::bail!(
                        "duplicate source (template_id={:?}, rule_name={:?}) reports \
                         inconsistent template_log_frequency_raw ({:?} vs {:?}) for what \
                         must be the same rule",
                        key.0,
                        key.1,
                        existing.template_log_frequency_raw,
                        s.template_log_frequency_raw
                    );
                }
                if existing.upstream_score_status != s.upstream_score_status {
                    anyhow::bail!(
                        "duplicate source (template_id={:?}, rule_name={:?}) reports \
                         inconsistent upstream_score_status ({:?} vs {:?}) for what must \
                         be the same rule",
                        key.0,
                        key.1,
                        existing.upstream_score_status,
                        s.upstream_score_status
                    );
                }
                existing.upstream_score = match (existing.upstream_score, s.upstream_score) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    (Some(a), None) | (None, Some(a)) => Some(a),
                    (None, None) => None,
                };
                existing.original_rank = existing.original_rank.min(s.original_rank);
                existing.base_step_cost = existing.base_step_cost.min(s.base_step_cost);
            }
        }
    }

    Ok(order
        .into_iter()
        .map(|key| by_key.remove(&key).expect("key was just inserted above"))
        .collect())
}

/// Canonicalize and merge raw proposals into candidate-pool entries.
///
/// Candidate ID: see [`candidate_id_for`]. Proposals whose sorted precursor
/// list hashes the same are merged: every distinct-rule source's full
/// provenance is kept in `sources` (see [`merge_duplicate_sources`] for
/// same-rule duplicates), `sources` is sorted deterministically (see
/// `ReactionCandidate` doc) so the "representative" source is never
/// ambiguous, and `best_*`/`min_*`/`max_*`/`mean_*` aggregates are computed
/// over all sources -- no provenance is dropped in favor of "just the best
/// one".
fn merge_into_candidates(
    canonical_target: &str,
    raw: Vec<RawCandidate>,
) -> anyhow::Result<Vec<ReactionCandidate>> {
    let mut order: Vec<String> = Vec::new();
    let mut precursors_by_id: HashMap<String, Vec<String>> = HashMap::new();
    let mut sources_by_id: HashMap<String, Vec<CandidateSource>> = HashMap::new();

    for proposal in raw {
        // Sorted but NOT deduplicated: a symmetric target can split into two
        // copies of the same fragment (e.g. bond-breaking down the middle
        // of a symmetric molecule), and that multiplicity is real
        // stoichiometry, not noise -- deduplicating would silently claim
        // one equivalent reconstitutes the target when two are actually
        // needed. This mirrors renkin-forward's product-multiset handling
        // (`["CO","CO"]` and `["CO"]` are different candidates there for
        // the same reason).
        let mut precursor_smiles: Vec<String> = proposal
            .precursors
            .iter()
            .map(|p| p.smiles.clone())
            .collect();
        precursor_smiles.sort_unstable();

        let candidate_id = candidate_id_for(canonical_target, &precursor_smiles);

        let base_step_cost = step_cost(
            &proposal
                .precursors
                .iter()
                .map(|p| &p.mol)
                .collect::<Vec<_>>(),
        );

        let source = CandidateSource {
            template_id: proposal.template_id,
            rule_name: proposal.rule_name,
            original_rank: proposal.original_rank,
            upstream_score: proposal.upstream_score,
            upstream_score_status: proposal.upstream_score_status,
            template_log_frequency_raw: Some(proposal.rule_weight as f32),
            base_step_cost,
        };

        if !sources_by_id.contains_key(&candidate_id) {
            order.push(candidate_id.clone());
            precursors_by_id.insert(candidate_id.clone(), precursor_smiles);
        }
        sources_by_id.entry(candidate_id).or_default().push(source);
    }

    order
        .into_iter()
        .map(|id| {
            let raw_sources = sources_by_id
                .remove(&id)
                .expect("id was just inserted above");
            let precursor_smiles = precursors_by_id
                .remove(&id)
                .expect("id was just inserted above");
            let mut sources = merge_duplicate_sources(raw_sources)?;

            sources.sort_by(|a, b| {
                b.upstream_score
                    .partial_cmp(&a.upstream_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(
                        a.base_step_cost
                            .partial_cmp(&b.base_step_cost)
                            .unwrap_or(std::cmp::Ordering::Equal),
                    )
                    .then(a.original_rank.cmp(&b.original_rank))
                    .then(a.template_id.cmp(&b.template_id))
                    .then(a.rule_name.cmp(&b.rule_name))
            });

            let best_upstream_score = sources
                .iter()
                .filter_map(|s| s.upstream_score)
                .fold(None, |acc: Option<f32>, v| {
                    Some(acc.map_or(v, |a| a.max(v)))
                });
            // The rank of whichever source achieved `best_upstream_score` --
            // NOT the plain minimum rank across all sources, which could
            // belong to a different, lower-scoring source. When no source
            // has a score at all (Exhaustive/BondIndexed), there is no
            // "best-scoring source" to correlate with, so this falls back to
            // the plain minimum rank, matching this field's only meaning in
            // that case.
            let best_upstream_rank = match best_upstream_score {
                Some(best) => sources
                    .iter()
                    .filter(|s| s.upstream_score == Some(best))
                    .map(|s| s.original_rank)
                    .min()
                    .unwrap_or(0),
                None => sources.iter().map(|s| s.original_rank).min().unwrap_or(0),
            };
            let min_base_step_cost = sources
                .iter()
                .map(|s| s.base_step_cost)
                .fold(f64::INFINITY, f64::min);
            let freqs: Vec<f32> = sources
                .iter()
                .filter_map(|s| s.template_log_frequency_raw)
                .collect();
            let max_template_frequency = freqs.iter().copied().fold(None, |acc: Option<f32>, v| {
                Some(acc.map_or(v, |a| a.max(v)))
            });
            let mean_template_frequency = if freqs.is_empty() {
                None
            } else {
                Some(freqs.iter().sum::<f32>() / freqs.len() as f32)
            };

            Ok(ReactionCandidate {
                candidate_id: id,
                target_smiles: canonical_target.to_string(),
                precursor_smiles,
                source_template_count: sources.len(),
                sources,
                best_upstream_score,
                best_upstream_rank,
                min_base_step_cost,
                max_template_frequency,
                mean_template_frequency,
                features: CandidateFeatures::default(),
                reranker_score: None,
            })
        })
        .collect()
}

/// Deterministic one-step candidate proposal: parse `target_smiles`, select
/// active rules per `config.mode`, apply every active rule, then
/// canonicalize+merge duplicate precursor-set outcomes. See module doc for
/// the important caveat that different `ProposalMode`s produce different
/// candidate *sets*, not just different orderings.
///
/// `group_id` is stamped onto the returned pool unchanged (see
/// [`CandidatePool`]'s doc for why it's kept distinct from `target_id`) --
/// this function never derives or validates it, since a dataset's grouping
/// scheme is entirely the caller's concern.
pub fn propose_one_step(
    group_id: &str,
    target_smiles: &str,
    rules: &[RetroRule],
    config: &ProposalConfig,
) -> anyhow::Result<CandidatePool> {
    let target_mol = mol_from_smiles(target_smiles)?;
    let canonical_target = to_canonical(&target_mol);

    let active_rules = select_active_rules(&target_mol, rules, &config.mode)?;
    let raw = raw_propose(&target_mol, &canonical_target, &active_rules);
    let candidates = merge_into_candidates(&canonical_target, raw)?;

    Ok(CandidatePool {
        group_id: group_id.to_string(),
        target_id: canonical_target.clone(),
        target_smiles: canonical_target,
        candidates,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chem_env::default_rules;

    fn rule(name: &str, smirks: &str) -> RetroRule {
        RetroRule {
            name: name.to_string(),
            template_id: format!("rule:{name}"),
            smirks: smirks.to_string(),
            weight: 1.0,
            required_elements: 0,
        }
    }

    fn extracted_rule(idx: usize, smirks: &str, weight: f64) -> RetroRule {
        RetroRule {
            name: format!("extracted_{idx}"),
            template_id: format!("smirks-sha256:fake{idx}"),
            smirks: smirks.to_string(),
            weight,
            required_elements: 0,
        }
    }

    // ---- ProposalMode / selection ----

    fn scorer_input(
        scores: Vec<(usize, f32, usize)>,
        rules_offset: usize,
    ) -> ScorerConditionedInput {
        ScorerConditionedInput {
            scores,
            status: UpstreamScoreStatus::Available,
            rules_offset,
            scorer_identity: "test-scorer".to_string(),
            scorer_model_sha256: "sha256:test".to_string(),
        }
    }

    #[test]
    fn exhaustive_mode_tries_all_rules() {
        let rules = default_rules();
        let target = "CC(=O)c1ccccc1";
        let target_mol = mol_from_smiles(target).unwrap();
        let active = select_active_rules(&target_mol, &rules, &ProposalMode::Exhaustive).unwrap();
        assert_eq!(active.len(), rules.len());
        for r in &active {
            assert_eq!(r.upstream_score_status, UpstreamScoreStatus::NotApplicable);
            assert!(r.upstream_score.is_none());
        }
    }

    #[test]
    fn scorer_conditioned_mode_tries_only_top_k_file_templates() {
        let mut rules = default_rules();
        let n_handcrafted = rules.len();
        rules.push(extracted_rule(0, "[C:1][C:2]>>[C:1].[C:2]", 3.0));
        rules.push(extracted_rule(1, "[C:1][C:2]>>[C:1].[C:2]", 2.0));
        rules.push(extracted_rule(2, "[C:1][C:2]>>[C:1].[C:2]", 1.0));

        // Only file template index (n_handcrafted) scores highest; top_k=1.
        let scores = vec![
            (n_handcrafted, 0.9, 0),
            (n_handcrafted + 1, 0.5, 1),
            (n_handcrafted + 2, 0.1, 2),
        ];
        let mode = ProposalMode::ScorerConditioned {
            input: scorer_input(scores, n_handcrafted),
            top_k: 1,
        };
        let target = "CCCC";
        let target_mol = mol_from_smiles(target).unwrap();
        let active = select_active_rules(&target_mol, &rules, &mode).unwrap();

        // All hand-crafted rules + exactly 1 file template.
        assert_eq!(active.len(), n_handcrafted + 1);
        let extracted_in_active: Vec<&ScoredRuleRef> = active
            .iter()
            .filter(|r| is_extracted_template(&r.rule.name))
            .collect();
        assert_eq!(extracted_in_active.len(), 1);
        assert_eq!(extracted_in_active[0].rule.name, "extracted_0");
        assert_eq!(extracted_in_active[0].upstream_score, Some(0.9));
        assert_eq!(
            extracted_in_active[0].upstream_score_status,
            UpstreamScoreStatus::Available
        );
    }

    #[test]
    fn handcrafted_rules_always_included_regardless_of_scorer() {
        let mut rules = default_rules();
        let n_handcrafted = rules.len();
        rules.push(extracted_rule(0, "[C:1][C:2]>>[C:1].[C:2]", 1.0));

        let mode_zero_k = ProposalMode::ScorerConditioned {
            input: scorer_input(vec![(n_handcrafted, 0.9, 0)], n_handcrafted),
            top_k: 0, // no file templates selected at all
        };
        let target_mol = mol_from_smiles("CCCC").unwrap();
        let active = select_active_rules(&target_mol, &rules, &mode_zero_k).unwrap();
        assert_eq!(
            active.len(),
            n_handcrafted,
            "hand-crafted rules must all still be present"
        );
        assert!(active.iter().all(|r| !is_extracted_template(&r.rule.name)));
    }

    #[test]
    fn scorer_conditioned_classifies_by_rules_offset_position_not_name_prefix() {
        // A "handcrafted-looking" rule name placed AFTER rules_offset must
        // be treated as a scoreable file template (position rules), and a
        // rule without the extracted_ prefix placed BEFORE rules_offset
        // must still be treated as always-included hand-crafted -- name
        // prefix must play no role in the classification.
        let handcrafted = rule("totally_handcrafted", "[C:1][C:2]>>[C:1].[C:2]");
        let file_template_with_plain_name = rule("not_prefixed_at_all", "[C:1][C:2]>>[C:1].[C:2]");
        let rules = vec![handcrafted, file_template_with_plain_name];
        let rules_offset = 1; // only index 0 is hand-crafted by position

        let mode = ProposalMode::ScorerConditioned {
            input: scorer_input(vec![(1, 0.5, 0)], rules_offset),
            top_k: 1,
        };
        let target_mol = mol_from_smiles("CCCC").unwrap();
        let active = select_active_rules(&target_mol, &rules, &mode).unwrap();
        assert_eq!(
            active.len(),
            2,
            "handcrafted-by-position + 1 scored file template"
        );
        let scored: Vec<&ScoredRuleRef> = active
            .iter()
            .filter(|r| r.upstream_score_status == UpstreamScoreStatus::Available)
            .collect();
        assert_eq!(scored.len(), 1);
        assert_eq!(scored[0].rule.name, "not_prefixed_at_all");
    }

    #[test]
    fn exhaustive_and_scorer_conditioned_candidate_sets_can_differ() {
        // Not just order -- an actual set difference, per the corrected
        // module-doc claim.
        let mut rules = default_rules();
        let n_handcrafted = rules.len();
        rules.push(extracted_rule(0, "[C:1][C:2]>>[C:1].[C:2]", 1.0));
        rules.push(extracted_rule(1, "[C:1][C:2]>>[C:1].[C:2]", 1.0));
        let target = "CCCC";
        let target_mol = mol_from_smiles(target).unwrap();

        let exhaustive =
            select_active_rules(&target_mol, &rules, &ProposalMode::Exhaustive).unwrap();
        let conditioned = select_active_rules(
            &target_mol,
            &rules,
            &ProposalMode::ScorerConditioned {
                input: scorer_input(
                    vec![(n_handcrafted, 0.9, 0), (n_handcrafted + 1, 0.1, 1)],
                    n_handcrafted,
                ),
                top_k: 1,
            },
        )
        .unwrap();
        assert!(
            conditioned.len() < exhaustive.len(),
            "scorer-conditioned selection must be a strict subset here, not just reordered"
        );
    }

    #[test]
    fn original_rank_matches_within_mode_rank() {
        let rules = default_rules();
        let target_mol = mol_from_smiles("CC(=O)c1ccccc1").unwrap();
        let active = select_active_rules(&target_mol, &rules, &ProposalMode::Exhaustive).unwrap();
        for (i, r) in active.iter().enumerate() {
            assert_eq!(r.source_rank, i);
        }
    }

    #[test]
    fn scorer_failure_status_is_not_applicable_not_frequency() {
        // A rule selected without any scorer involvement (Exhaustive) must
        // report NotApplicable, and must never carry a frequency-derived
        // value in upstream_score -- upstream_score stays None.
        let rules = default_rules();
        let target_mol = mol_from_smiles("CC(=O)c1ccccc1").unwrap();
        let active = select_active_rules(&target_mol, &rules, &ProposalMode::Exhaustive).unwrap();
        for r in &active {
            assert!(r.upstream_score.is_none());
            assert_eq!(r.upstream_score_status, UpstreamScoreStatus::NotApplicable);
        }
    }

    #[test]
    fn scorer_conditioned_with_empty_scores_but_available_status_selects_only_handcrafted() {
        // A scorer that legitimately succeeded (status: Available) but
        // found zero relevant file templates must select exactly the
        // hand-crafted rules -- distinct from the failure case below, which
        // must be a hard error rather than looking the same as this.
        let mut rules = default_rules();
        let n_handcrafted = rules.len();
        rules.push(extracted_rule(0, "[C:1][C:2]>>[C:1].[C:2]", 3.0));
        rules.push(extracted_rule(1, "[C:1][C:2]>>[C:1].[C:2]", 2.0));

        let mode = ProposalMode::ScorerConditioned {
            input: scorer_input(Vec::new(), n_handcrafted),
            top_k: 10,
        };
        let target_mol = mol_from_smiles("CCCC").unwrap();
        let active = select_active_rules(&target_mol, &rules, &mode).unwrap();

        assert_eq!(
            active.len(),
            n_handcrafted,
            "must select only hand-crafted rules, zero file templates"
        );
        assert!(active.iter().all(|r| !is_extracted_template(&r.rule.name)));
        assert!(
            active.iter().all(|r| r.upstream_score.is_none()),
            "no frequency-derived value may appear in upstream_score when the scorer produced no scores"
        );
    }

    #[test]
    fn scorer_conditioned_fails_closed_when_status_is_not_available() {
        // A scorer FAILURE must never be silently indistinguishable from
        // "the scorer succeeded and found nothing relevant" -- it must fail
        // the whole proposal call instead of quietly narrowing to zero file
        // templates as if that were a normal, successful outcome.
        let rules = default_rules();
        let n_handcrafted = rules.len();
        for status in [
            UpstreamScoreStatus::ModelNotConfigured,
            UpstreamScoreStatus::TargetParseFailed,
            UpstreamScoreStatus::InferenceFailed,
            UpstreamScoreStatus::OutputShapeMismatch,
        ] {
            let mode = ProposalMode::ScorerConditioned {
                input: ScorerConditionedInput {
                    scores: Vec::new(),
                    status,
                    rules_offset: n_handcrafted,
                    scorer_identity: "test-scorer".to_string(),
                    scorer_model_sha256: "sha256:test".to_string(),
                },
                top_k: 10,
            };
            let target_mol = mol_from_smiles("CCCC").unwrap();
            assert!(
                select_active_rules(&target_mol, &rules, &mode).is_err(),
                "status {status:?} must fail closed, not silently succeed with zero file templates"
            );
        }
    }

    #[test]
    fn scorer_conditioned_rejects_out_of_bounds_rule_index() {
        let rules = default_rules();
        let n_handcrafted = rules.len();
        let mode = ProposalMode::ScorerConditioned {
            input: scorer_input(vec![(rules.len() + 5, 0.5, 0)], n_handcrafted),
            top_k: 10,
        };
        let target_mol = mol_from_smiles("CCCC").unwrap();
        assert!(select_active_rules(&target_mol, &rules, &mode).is_err());
    }

    #[test]
    fn scorer_conditioned_rejects_rule_index_inside_handcrafted_prefix() {
        let rules = default_rules();
        let n_handcrafted = rules.len();
        assert!(n_handcrafted > 0, "fixture assumption");
        let mode = ProposalMode::ScorerConditioned {
            input: scorer_input(vec![(0, 0.5, 0)], n_handcrafted),
            top_k: 10,
        };
        let target_mol = mol_from_smiles("CCCC").unwrap();
        assert!(
            select_active_rules(&target_mol, &rules, &mode).is_err(),
            "a scored rule_index inside [0, rules_offset) must be rejected"
        );
    }

    #[test]
    fn scorer_conditioned_rejects_duplicate_rule_index() {
        let mut rules = default_rules();
        let n_handcrafted = rules.len();
        rules.push(extracted_rule(0, "[C:1][C:2]>>[C:1].[C:2]", 1.0));
        let mode = ProposalMode::ScorerConditioned {
            input: scorer_input(
                vec![(n_handcrafted, 0.5, 0), (n_handcrafted, 0.6, 1)],
                n_handcrafted,
            ),
            top_k: 10,
        };
        let target_mol = mol_from_smiles("CCCC").unwrap();
        assert!(select_active_rules(&target_mol, &rules, &mode).is_err());
    }

    #[test]
    fn scorer_conditioned_rejects_duplicate_rank() {
        let mut rules = default_rules();
        let n_handcrafted = rules.len();
        rules.push(extracted_rule(0, "[C:1][C:2]>>[C:1].[C:2]", 1.0));
        rules.push(extracted_rule(1, "[C:1][C:2]>>[C:1].[C:2]", 1.0));
        let mode = ProposalMode::ScorerConditioned {
            input: scorer_input(
                vec![(n_handcrafted, 0.5, 0), (n_handcrafted + 1, 0.6, 0)],
                n_handcrafted,
            ),
            top_k: 10,
        };
        let target_mol = mol_from_smiles("CCCC").unwrap();
        assert!(select_active_rules(&target_mol, &rules, &mode).is_err());
    }

    #[test]
    fn scorer_conditioned_rejects_non_finite_raw_logit() {
        let mut rules = default_rules();
        let n_handcrafted = rules.len();
        rules.push(extracted_rule(0, "[C:1][C:2]>>[C:1].[C:2]", 1.0));
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mode = ProposalMode::ScorerConditioned {
                input: scorer_input(vec![(n_handcrafted, bad, 0)], n_handcrafted),
                top_k: 10,
            };
            let target_mol = mol_from_smiles("CCCC").unwrap();
            assert!(
                select_active_rules(&target_mol, &rules, &mode).is_err(),
                "raw_logit {bad} must be rejected"
            );
        }
    }

    #[test]
    fn scorer_conditioned_tie_break_is_rank_ascending() {
        let mut rules = default_rules();
        let n_handcrafted = rules.len();
        rules.push(extracted_rule(0, "[C:1][C:2]>>[C:1].[C:2]", 1.0));
        rules.push(extracted_rule(1, "[C:1][C:2]>>[C:1].[C:2]", 1.0));
        let mode = ProposalMode::ScorerConditioned {
            input: scorer_input(
                vec![(n_handcrafted + 1, 0.1, 1), (n_handcrafted, 0.9, 0)],
                n_handcrafted,
            ),
            top_k: 1,
        };
        let target_mol = mol_from_smiles("CCCC").unwrap();
        let active = select_active_rules(&target_mol, &rules, &mode).unwrap();
        let scored: Vec<&ScoredRuleRef> = active
            .iter()
            .filter(|r| r.upstream_score_status == UpstreamScoreStatus::Available)
            .collect();
        assert_eq!(scored.len(), 1);
        assert_eq!(
            scored[0].upstream_score,
            Some(0.9),
            "rank 0 (lowest rank) must be selected by top_k=1, not insertion order"
        );
    }

    // ---- raw_propose / golden equivalence ----

    #[test]
    fn raw_propose_matches_independent_reference_filter() {
        let rules = default_rules();
        let target = "CC(=O)c1ccccc1"; // acetophenone
        let target_mol = mol_from_smiles(target).unwrap();
        let canon_target = to_canonical(&target_mol);
        let target_elem_mask = crate::search::elem_mask_from_smiles(&canon_target);

        let mut reference: Vec<(String, Vec<String>)> = Vec::new();
        for rule in &rules {
            if !(rule.required_elements == 0
                || (target_elem_mask & rule.required_elements == rule.required_elements))
            {
                continue;
            }
            for precs in apply_retro(&target_mol, rule) {
                if precs.is_empty() || precs.iter().any(|p| p.smiles == canon_target) {
                    continue;
                }
                reference.push((
                    rule.name.clone(),
                    precs.iter().map(|p| p.smiles.clone()).collect(),
                ));
            }
        }

        let active_rules =
            select_active_rules(&target_mol, &rules, &ProposalMode::Exhaustive).unwrap();
        let got = raw_propose(&target_mol, &canon_target, &active_rules);
        let mut got_pairs: Vec<(String, Vec<String>)> = got
            .into_iter()
            .map(|p| {
                (
                    p.rule_name,
                    p.precursors.iter().map(|pm| pm.smiles.clone()).collect(),
                )
            })
            .collect();

        let mut reference_sorted = reference;
        reference_sorted.sort();
        got_pairs.sort();
        assert_eq!(got_pairs, reference_sorted);
        assert!(
            !got_pairs.is_empty(),
            "expected at least one match for acetophenone"
        );
    }

    // ---- merge / provenance ----

    fn precs(smi: &str) -> PrecursorMol {
        PrecursorMol {
            smiles: smi.to_string(),
            mol: mol_from_smiles(smi).unwrap(),
        }
    }

    #[test]
    fn candidate_id_join_ambiguity_is_resolved() {
        // A naive `.join(".")` would make these two DIFFERENT precursor
        // sequences collide: ["C.C", "N"].join(".") == "C.C.N"
        //                    ["C", "C.N"].join(".") == "C.C.N"
        let id_a = candidate_id_for("target", &["C.C".to_string(), "N".to_string()]);
        let id_b = candidate_id_for("target", &["C".to_string(), "C.N".to_string()]);
        assert_ne!(id_a, id_b, "join ambiguity must not collide");
    }

    #[test]
    fn candidate_id_is_stable_sha256_prefixed() {
        let id = candidate_id_for("target", &["CC".to_string()]);
        assert!(id.starts_with("sha256:"));
        assert_eq!(candidate_id_for("target", &["CC".to_string()]), id);
    }

    #[test]
    fn duplicate_precursor_fragment_within_one_split_is_not_collapsed() {
        // A symmetric target splitting into two copies of the same fragment
        // is real stoichiometry (two equivalents needed to reconstitute the
        // target), not noise -- merge_into_candidates must not silently
        // collapse it to one.
        let raw = vec![RawCandidate {
            rule_name: "symmetric_split".to_string(),
            template_id: "rule:symmetric_split".to_string(),
            rule_weight: 1.0,
            original_rank: 0,
            upstream_score: None,
            upstream_score_status: UpstreamScoreStatus::NotApplicable,
            precursors: vec![precs("CC"), precs("CC")],
        }];
        let merged = merge_into_candidates("target", raw).unwrap();
        assert_eq!(merged.len(), 1);
        assert_eq!(
            merged[0].precursor_smiles,
            vec!["CC".to_string(), "CC".to_string()],
            "duplicate precursor fragment multiplicity must be preserved, not deduplicated"
        );
    }

    #[test]
    fn duplicate_precursor_set_from_two_rules_merges_with_provenance_retained() {
        let raw = vec![
            RawCandidate {
                rule_name: "rule_a".to_string(),
                template_id: "rule:rule_a".to_string(),
                rule_weight: 1.0,
                original_rank: 0,
                upstream_score: None,
                upstream_score_status: UpstreamScoreStatus::NotApplicable,
                precursors: vec![precs("CC"), precs("O")],
            },
            RawCandidate {
                rule_name: "rule_b".to_string(),
                template_id: "rule:rule_b".to_string(),
                rule_weight: 1.0,
                original_rank: 5,
                upstream_score: None,
                upstream_score_status: UpstreamScoreStatus::NotApplicable,
                precursors: vec![precs("O"), precs("CC")], // same set, different order
            },
        ];
        let merged = merge_into_candidates("target", raw).unwrap();
        assert_eq!(
            merged.len(),
            1,
            "identical precursor sets must merge into one candidate"
        );
        let c = &merged[0];
        assert_eq!(c.source_template_count, 2);
        assert_eq!(c.sources.len(), 2);
        assert_eq!(
            c.best_upstream_rank, 0,
            "must keep the best (lowest) original_rank"
        );
        let names: Vec<&str> = c.sources.iter().map(|s| s.rule_name.as_str()).collect();
        assert!(names.contains(&"rule_a"));
        assert!(names.contains(&"rule_b"));
    }

    #[test]
    fn duplicate_same_template_outcomes_do_not_inflate_source_count() {
        // The SAME (template_id, rule_name) reaching the same merged
        // candidate twice (e.g. a symmetric rule matching two equivalent
        // sites that happen to produce the identical sorted precursor set)
        // must collapse into one source, not two -- `source_template_count`
        // counts distinct contributing rules, not raw applications.
        let raw = vec![
            RawCandidate {
                rule_name: "symmetric_rule".to_string(),
                template_id: "rule:symmetric_rule".to_string(),
                rule_weight: 1.0,
                original_rank: 3,
                upstream_score: Some(0.4),
                upstream_score_status: UpstreamScoreStatus::Available,
                precursors: vec![precs("CC"), precs("O")],
            },
            RawCandidate {
                rule_name: "symmetric_rule".to_string(),
                template_id: "rule:symmetric_rule".to_string(),
                rule_weight: 1.0,
                original_rank: 1,
                upstream_score: Some(0.4),
                upstream_score_status: UpstreamScoreStatus::Available,
                precursors: vec![precs("O"), precs("CC")], // same set, different match site
            },
        ];
        let merged = merge_into_candidates("target", raw).unwrap();
        assert_eq!(merged.len(), 1);
        assert_eq!(
            merged[0].source_template_count, 1,
            "two applications of the same rule must merge into one source"
        );
        assert_eq!(merged[0].sources.len(), 1);
        assert_eq!(
            merged[0].sources[0].original_rank, 1,
            "merged source must keep the min original_rank across duplicates"
        );
    }

    #[test]
    fn duplicate_same_template_outcomes_reject_inconsistent_frequency() {
        // Two applications of what claims to be the same (template_id,
        // rule_name) but with different rule_weight (hence different
        // template_log_frequency_raw) is an internally inconsistent input --
        // a hard error, not a silent pick of one value.
        let raw = vec![
            RawCandidate {
                rule_name: "r".to_string(),
                template_id: "rule:r".to_string(),
                rule_weight: 1.0,
                original_rank: 0,
                upstream_score: None,
                upstream_score_status: UpstreamScoreStatus::NotApplicable,
                precursors: vec![precs("CC")],
            },
            RawCandidate {
                rule_name: "r".to_string(),
                template_id: "rule:r".to_string(),
                rule_weight: 2.0, // inconsistent with the first
                original_rank: 1,
                upstream_score: None,
                upstream_score_status: UpstreamScoreStatus::NotApplicable,
                precursors: vec![precs("CC")],
            },
        ];
        assert!(merge_into_candidates("target", raw).is_err());
    }

    #[test]
    fn best_upstream_score_and_min_cost_retained_on_merge() {
        let mut a = RawCandidate {
            rule_name: "rule_a".to_string(),
            template_id: "rule:rule_a".to_string(),
            rule_weight: 1.0,
            original_rank: 3,
            upstream_score: Some(0.2),
            upstream_score_status: UpstreamScoreStatus::Available,
            precursors: vec![precs("CC")],
        };
        let mut b = RawCandidate {
            rule_name: "rule_b".to_string(),
            template_id: "rule:rule_b".to_string(),
            rule_weight: 1.0,
            original_rank: 1,
            upstream_score: Some(0.9),
            upstream_score_status: UpstreamScoreStatus::Available,
            precursors: vec![precs("CC")],
        };
        // Distinguish base_step_cost between the two sources by giving them
        // different (but same-canonical-set) precursor lists is not
        // possible without changing the merge key, so just check via the
        // raw fields directly after merge instead.
        a.precursors = vec![precs("CC")];
        b.precursors = vec![precs("CC")];

        let merged = merge_into_candidates("target", vec![a, b]).unwrap();
        assert_eq!(merged.len(), 1);
        assert_eq!(
            merged[0].best_upstream_score,
            Some(0.9),
            "must keep the best (max) upstream score"
        );
        assert_eq!(
            merged[0].best_upstream_rank, 1,
            "must keep the best (min) original_rank"
        );
        assert!(merged[0].min_base_step_cost.is_finite());
    }

    #[test]
    fn best_upstream_rank_is_the_best_scoring_sources_rank_not_the_global_minimum() {
        // rule_low_rank has the lowest original_rank (0) but a mediocre
        // score; rule_best_score has a worse (higher) rank but the best
        // score. best_upstream_rank must report rule_best_score's rank (2),
        // NOT the global minimum rank (0) which belongs to a different,
        // lower-scoring source.
        let raw = vec![
            RawCandidate {
                rule_name: "rule_low_rank".to_string(),
                template_id: "rule:rule_low_rank".to_string(),
                rule_weight: 1.0,
                original_rank: 0,
                upstream_score: Some(0.1),
                upstream_score_status: UpstreamScoreStatus::Available,
                precursors: vec![precs("CC")],
            },
            RawCandidate {
                rule_name: "rule_best_score".to_string(),
                template_id: "rule:rule_best_score".to_string(),
                rule_weight: 1.0,
                original_rank: 2,
                upstream_score: Some(0.9),
                upstream_score_status: UpstreamScoreStatus::Available,
                precursors: vec![precs("CC")],
            },
        ];
        let merged = merge_into_candidates("target", raw).unwrap();
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].best_upstream_score, Some(0.9));
        assert_eq!(
            merged[0].best_upstream_rank, 2,
            "must be the rank of the source that achieved best_upstream_score, not the global minimum rank (0)"
        );
    }

    #[test]
    fn sources_sorted_deterministically_representative_unambiguous() {
        let raw = vec![
            RawCandidate {
                rule_name: "z_rule".to_string(),
                template_id: "z".to_string(),
                rule_weight: 1.0,
                original_rank: 0,
                upstream_score: Some(0.5),
                upstream_score_status: UpstreamScoreStatus::Available,
                precursors: vec![precs("CC")],
            },
            RawCandidate {
                rule_name: "a_rule".to_string(),
                template_id: "a".to_string(),
                rule_weight: 1.0,
                original_rank: 0,
                upstream_score: Some(0.5),
                upstream_score_status: UpstreamScoreStatus::Available,
                precursors: vec![precs("CC")],
            },
        ];
        let merged = merge_into_candidates("target", raw).unwrap();
        assert_eq!(merged.len(), 1);
        // Tied on upstream_score and base_step_cost and original_rank -->
        // tie-break by template_id lexicographic: "a" before "z".
        assert_eq!(merged[0].sources[0].template_id, "a");
    }

    #[test]
    fn merge_into_candidates_output_is_independent_of_input_order() {
        // candidate_id/grouping is keyed by canonical target + sorted
        // precursors (order-independent by construction), and every
        // aggregated field (best_upstream_score/rank, min_base_step_cost,
        // frequencies, sources' own sort) is computed via commutative
        // min/max/fold/sort -- never "whichever RawCandidate arrived
        // first/last". The only order-DEPENDENT thing is the returned
        // Vec's overall position (first-seen insertion order), which a
        // caller must sort by candidate_id before comparing (exactly as
        // `pool_export`'s exporter already does).
        fn one(rule_name: &str, template_id: &str, rank: usize, precursor: &str) -> RawCandidate {
            RawCandidate {
                rule_name: rule_name.to_string(),
                template_id: template_id.to_string(),
                rule_weight: 1.0,
                original_rank: rank,
                upstream_score: None,
                upstream_score_status: UpstreamScoreStatus::NotApplicable,
                precursors: vec![precs(precursor)],
            }
        }
        fn summarize(candidates: Vec<ReactionCandidate>) -> Vec<(String, Vec<String>)> {
            let mut summary: Vec<(String, Vec<String>)> = candidates
                .into_iter()
                .map(|c| {
                    let mut rule_names: Vec<String> =
                        c.sources.iter().map(|s| s.rule_name.clone()).collect();
                    rule_names.sort();
                    (c.candidate_id, rule_names)
                })
                .collect();
            summary.sort_by(|a, b| a.0.cmp(&b.0));
            summary
        }

        // rule_a and rule_c both propose the same precursor ("CC") -> merge
        // into one candidate with two sources; rule_b proposes a distinct
        // precursor ("CCO") -> its own separate candidate.
        let forward = vec![
            one("rule_a", "rule:a", 0, "CC"),
            one("rule_b", "rule:b", 1, "CCO"),
            one("rule_c", "rule:c", 2, "CC"),
        ];
        let reversed = vec![
            one("rule_c", "rule:c", 2, "CC"),
            one("rule_b", "rule:b", 1, "CCO"),
            one("rule_a", "rule:a", 0, "CC"),
        ];

        let forward_summary = summarize(merge_into_candidates("target", forward).unwrap());
        let reversed_summary = summarize(merge_into_candidates("target", reversed).unwrap());
        assert_eq!(
            forward_summary, reversed_summary,
            "merged candidate set/content must not depend on RawCandidate input order"
        );
        assert_eq!(
            forward_summary.len(),
            2,
            "CC merges rule_a+rule_c; CCO stays separate"
        );
    }

    #[test]
    fn no_precursors_produces_no_self_loop_candidate() {
        let target = "CCO";
        let rules = vec![rule("noop", "[C:1]>>[C:1]")];
        let config = ProposalConfig::default();
        let pool = propose_one_step("group:1", target, &rules, &config).unwrap();
        for c in &pool.candidates {
            assert_ne!(c.precursor_smiles, vec![pool.target_smiles.clone()]);
        }
    }

    #[test]
    fn same_target_different_group_shares_target_id_not_group_id() {
        // Two dataset examples (e.g. two different literature reactions)
        // producing the same product molecule must share `target_id` (the
        // leakage-safe split key) while keeping distinct `group_id`s (each
        // its own LightGBM ranking group) -- this module never conflates
        // "same molecule" with "same example".
        let rules = default_rules();
        let target = "CC(=O)c1ccccc1";
        let config = ProposalConfig::default();
        let pool_a = propose_one_step("rxn-example-001", target, &rules, &config).unwrap();
        let pool_b = propose_one_step("rxn-example-002", target, &rules, &config).unwrap();

        assert_eq!(pool_a.target_id, pool_b.target_id);
        assert_ne!(pool_a.group_id, pool_b.group_id);
        assert_eq!(pool_a.group_id, "rxn-example-001");
        assert_eq!(pool_b.group_id, "rxn-example-002");
    }

    // ---- template transformation features ----

    #[test]
    fn graph_based_rule_reaction_center_is_missing() {
        let graph_rule = rule("cbz_deprotection_retro", "");
        let f = template_transformation_features(&graph_rule);
        assert!(!f.extractable);
        assert_eq!(f.reaction_center_atom_count, 0);
    }

    #[test]
    fn mapped_smirks_reaction_center_is_deterministic() {
        // Ester hydrolysis: the ester O:3 keeps its mapping (becomes the
        // acid's -OH), the alkyl C:4 keeps its mapping (becomes an alcohol),
        // and a fresh (unmapped) OH is introduced on the alcohol side --
        // C:4's bond to O:3 is broken (deleted_bond_count == 1).
        let mapped_rule = rule(
            "ester_hydrolysis_retro",
            "[C:1](=[O:2])-[O:3]-[C:4]>>[C:1](=[O:2])-[OH:3].[OH]-[C:4]",
        );
        let a = template_transformation_features(&mapped_rule);
        let b = template_transformation_features(&mapped_rule);
        assert_eq!(a.mapped_atom_count, b.mapped_atom_count);
        assert_eq!(a.deleted_bond_count, b.deleted_bond_count);
        assert_eq!(a.extractable, b.extractable);
        assert!(
            a.extractable,
            "a properly atom-mapped SMIRKS must be extractable"
        );
        assert_eq!(a.mapped_atom_count, 4, "C:1, O:2, O:3, C:4");
        assert_eq!(a.deleted_bond_count, 1, "the O:3-C:4 ester bond is broken");
        assert!(a.reaction_center_atom_count > 0);
    }

    #[test]
    fn unmapped_smirks_reaction_center_is_missing_not_guessed() {
        // No atom-map annotations at all -- must not fabricate a value.
        let unmapped_rule = rule("fake_unmapped", "CC>>C.C");
        let f = template_transformation_features(&unmapped_rule);
        assert!(!f.extractable);
    }

    #[test]
    fn partially_mapped_smirks_is_extractable_over_the_mapped_atoms_only() {
        // Only atom map 1 is annotated; the rest of both sides are
        // unmapped. This is NOT the same code path as fully-unmapped
        // (`by_map.is_empty()`): `by_map` is non-empty and has no
        // duplicates, so this must be `extractable: true`, with the
        // reaction-center diff computed only over the mapped
        // intersection -- never guessed for the unmapped atoms, and never
        // downgraded to "missing" just because most atoms lack a map.
        let partial_rule = rule("partial_map", "[C:1]CC>>[C:1]C.C");
        let f = template_transformation_features(&partial_rule);
        assert!(
            f.extractable,
            "a partially-mapped SMIRKS (>=1 mapped atom, no duplicates) must still be extractable"
        );
        assert_eq!(
            f.mapped_atom_count, 1,
            "only atom map 1 is annotated on either side"
        );
    }

    #[test]
    fn changed_bond_order_is_detected_without_adding_or_deleting_a_bond() {
        // The C:1-O:2 bond survives (same two mapped atoms bonded on both
        // sides) but its order changes (double -> single) -- this must hit
        // the `changed_bond_order_count` branch specifically, not be
        // miscounted as a deleted+added bond pair.
        let bond_order_change_rule = rule("retro_reduction", "[C:1]=[O:2]>>[C:1][O:2]");
        let f = template_transformation_features(&bond_order_change_rule);
        assert!(f.extractable);
        assert_eq!(f.deleted_bond_count, 0, "the C:1-O:2 bond is never deleted");
        assert_eq!(
            f.added_bond_count, 0,
            "the C:1-O:2 bond is never newly formed"
        );
        assert_eq!(
            f.changed_bond_order_count, 1,
            "the C:1-O:2 bond order changes from double to single"
        );
        assert!(f.reaction_center_atom_count > 0);
    }

    #[test]
    fn multi_component_reaction_center_does_not_collide_on_local_atom_idx() {
        // Two reactant molecules (map1-map2 bonded, map3-map4 bonded) become
        // two DIFFERENT product molecules (map1-map3 newly bonded,
        // map2-map4 newly bonded). Each product molecule's newly-formed
        // bond sits at the same LOCAL AtomIdx pair (0, 1) as the other --
        // pooling raw AtomIdx values across molecules (the previous
        // implementation) would collapse these 4 distinct atoms down to 2.
        // Keying by atom_map number must keep all 4 distinct.
        let cross_rule = rule(
            "synthetic_cross_metathesis",
            "[C:1][C:2].[C:3][C:4]>>[C:1][C:3].[C:2][C:4]",
        );
        let f = template_transformation_features(&cross_rule);
        assert!(f.extractable);
        assert_eq!(f.deleted_bond_count, 2, "both original bonds are broken");
        assert_eq!(f.added_bond_count, 2, "both new cross-bonds are formed");
        assert_eq!(
            f.reaction_center_atom_count, 4,
            "all four atoms (map1..map4) participate in the reaction center -- \
             a raw-AtomIdx collision would undercount this to 2"
        );
    }

    #[test]
    fn duplicate_atom_map_within_one_side_is_not_extractable() {
        // The same atom_map number (1) appears on two different atoms on
        // the reactant side -- an ambiguous mapping that must never be
        // silently resolved to whichever occurrence was inserted last.
        let ambiguous_rule = rule("ambiguous_duplicate_map", "[C:1][C:1]>>[C:1].[C:1]");
        let f = template_transformation_features(&ambiguous_rule);
        assert!(
            !f.extractable,
            "a duplicate atom_map number on one side must not be extractable"
        );
    }

    #[test]
    fn transformation_cache_does_not_collide_on_reused_template_id_with_different_smirks() {
        // Two RetroRules sharing the same template_id but with different
        // SMIRKS must never read back each other's cached features -- the
        // cache key includes a SMIRKS hash precisely to prevent this.
        let mut a = rule("shared_id", "[C:1][C:2]>>[C:1].[C:2]");
        a.template_id = "rule:shared_id".to_string();
        let mut b = rule("shared_id", "[C:1][C:2][C:3]>>[C:1].[C:2].[C:3]");
        b.template_id = "rule:shared_id".to_string();

        let fa = template_transformation_features(&a);
        let fb = template_transformation_features(&b);
        assert!(fa.extractable);
        assert!(fb.extractable);
        assert_ne!(
            fa.mapped_atom_count, fb.mapped_atom_count,
            "these two SMIRKS have a different mapped atom count -- if the cache \
             collided on template_id alone, one of these would incorrectly read \
             back the other's cached result"
        );
    }

    #[test]
    fn index_rules_by_template_id_rejects_conflicting_duplicate() {
        let mut a = rule("dup", "[C:1]>>[C:1]");
        a.template_id = "rule:dup".to_string();
        let mut b = rule("dup", "[N:1]>>[N:1]"); // different smirks, same id
        b.template_id = "rule:dup".to_string();
        assert!(index_rules_by_template_id(&[a, b]).is_err());
    }

    #[test]
    fn index_rules_by_template_id_tolerates_exact_duplicate() {
        let mut a = rule("dup", "[C:1]>>[C:1]");
        a.template_id = "rule:dup".to_string();
        let b = a.clone();
        let rules = [a, b];
        let index = index_rules_by_template_id(&rules).unwrap();
        assert_eq!(index.len(), 1);
    }

    #[test]
    fn aggregate_transformation_features_no_nan_or_inf() {
        let features = vec![
            TemplateTransformationFeatures {
                extractable: true,
                reaction_center_atom_count: 4,
                ..Default::default()
            },
            TemplateTransformationFeatures {
                extractable: false,
                ..Default::default()
            },
        ];
        let agg = aggregate_transformation_features(&features);
        assert!(agg.reaction_center_atom_count_mean.is_finite());
        assert!(agg.reaction_center_extractable_fraction.is_finite());
        assert!(!agg.reaction_center_atom_count_mean.is_nan());

        let empty_agg = aggregate_transformation_features(&[]);
        assert!(!empty_agg.reaction_center_atom_count_mean.is_nan());
    }

    // ---- feature schema v1 / extract_features ----

    #[test]
    fn feature_schema_v1_names_and_group_boundary_are_consistent() {
        assert_eq!(FEATURE_NAMES_V1.len(), 18);
        assert!(FEATURE_GROUP1_LEN < FEATURE_NAMES_V1.len());
        assert_eq!(
            FEATURE_NAMES_V1[FEATURE_GROUP1_LEN],
            "fraction_precursors_in_stock"
        );
        assert_eq!(
            feature_index("num_precursors"),
            Some(0),
            "feature_index must find a real schema-v1 name"
        );
        assert_eq!(
            feature_index("not_a_real_feature"),
            None,
            "feature_index must return None for an unknown name"
        );
    }

    #[test]
    fn feature_schema_hash_is_stable_and_pinned_for_cross_language_verification() {
        let a = feature_schema_hash();
        let b = feature_schema_hash();
        assert_eq!(a, b, "the hash must be deterministic across calls");
        assert!(a.starts_with("sha256:"));
        // Pinned literal: `scripts/train_reranker.py` mirrors FEATURE_NAMES_V1
        // and this exact hashing algorithm in Python (it has no way to
        // import this crate). This fixed value was computed from the
        // current FEATURE_NAMES_V1/FEATURE_SCHEMA_VERSION and cross-checked
        // against the Python mirror at commit time -- if this assertion
        // ever fails after an intentional feature-schema change, the
        // pinned literal in scripts/tests/test_reranker_schema.py's
        // FeatureSchemaHashPinTests must be updated to match, not just
        // this one.
        assert_eq!(
            a,
            "sha256:756404c59bbee9a65e194f92df3530e1b801028f333e01c67214917977061df1"
        );
    }

    fn candidate_for(target: &str, rules: &[RetroRule], mode: ProposalMode) -> ReactionCandidate {
        let pool = propose_one_step("group:1", target, rules, &ProposalConfig { mode }).unwrap();
        pool.candidates
            .into_iter()
            .next()
            .expect("expected at least one candidate for this fixture")
    }

    #[test]
    fn extract_features_group2_missing_without_stock() {
        let rules = default_rules();
        let target = "CC(=O)c1ccccc1";
        let target_mol = mol_from_smiles(target).unwrap();
        let candidate = candidate_for(target, &rules, ProposalMode::Exhaustive);
        let templates_by_id = index_rules_by_template_id(&rules).unwrap();

        let features = extract_features(&candidate, &target_mol, &templates_by_id, None);
        assert_eq!(features.values.len(), FEATURE_NAMES_V1.len());
        assert_eq!(features.missing.len(), FEATURE_NAMES_V1.len());

        for name in ["fraction_precursors_in_stock", "all_precursors_in_stock"] {
            let i = feature_index(name).unwrap();
            assert!(
                features.missing[i],
                "{name} must be missing without a stock"
            );
        }
        for name in ["max_template_log_frequency", "mean_template_log_frequency"] {
            let i = feature_index(name).unwrap();
            assert!(
                features.missing[i],
                "{name} must always be missing until split-aware recomputation lands"
            );
        }
        let best_upstream_i = feature_index("best_upstream_score").unwrap();
        for (i, name) in FEATURE_NAMES_V1.iter().enumerate().take(FEATURE_GROUP1_LEN) {
            if i == best_upstream_i {
                // Exhaustive mode never attaches an upstream score (no
                // scorer is used at all -- see UpstreamScoreStatus::
                // NotApplicable), so this is genuinely absent, not a
                // computation failure. Still group 1: its missingness has
                // nothing to do with corpus/train-split leakage, unlike the
                // stock/frequency features below.
                assert!(
                    features.missing[i],
                    "best_upstream_score must be missing under Exhaustive mode (no scorer involved)"
                );
                continue;
            }
            assert!(
                !features.missing[i],
                "group-1 feature {i} ({name}) must be computed for a normal candidate"
            );
        }
    }

    #[test]
    fn extract_features_availability_reflects_stock_membership() {
        use crate::chem_env::ChemEnv;

        let rules = default_rules();
        let target = "CC(=O)c1ccccc1";
        let target_mol = mol_from_smiles(target).unwrap();
        let candidate = candidate_for(target, &rules, ProposalMode::Exhaustive);
        let templates_by_id = index_rules_by_template_id(&rules).unwrap();

        // Every precursor of this candidate is in the stock.
        let full_stock = ChemEnv::in_memory(
            &candidate
                .precursor_smiles
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>(),
        );
        let f_full = extract_features(&candidate, &target_mol, &templates_by_id, Some(&full_stock));
        let all_i = feature_index("all_precursors_in_stock").unwrap();
        let frac_i = feature_index("fraction_precursors_in_stock").unwrap();
        assert!(!f_full.missing[all_i]);
        assert_eq!(f_full.values[all_i], 1.0);
        assert_eq!(f_full.values[frac_i], 1.0);

        // An empty stock: nothing is available.
        let empty_stock = ChemEnv::in_memory(&[]);
        let f_empty = extract_features(
            &candidate,
            &target_mol,
            &templates_by_id,
            Some(&empty_stock),
        );
        assert!(!f_empty.missing[all_i]);
        assert_eq!(f_empty.values[all_i], 0.0);
        assert_eq!(f_empty.values[frac_i], 0.0);
    }

    #[test]
    fn extract_features_no_heavy_atom_gain_and_charge_balance_hold_for_real_reaction() {
        let rules = default_rules();
        let target = "CC(=O)c1ccccc1";
        let target_mol = mol_from_smiles(target).unwrap();
        let candidate = candidate_for(target, &rules, ProposalMode::Exhaustive);
        let templates_by_id = index_rules_by_template_id(&rules).unwrap();

        let features = extract_features(&candidate, &target_mol, &templates_by_id, None);
        let no_gain_i = feature_index("no_heavy_atom_gain").unwrap();
        let charge_i = feature_index("net_charge_balanced").unwrap();
        assert_eq!(
            features.values[no_gain_i], 1.0,
            "a real retro disconnection must never gain heavy atoms in the target"
        );
        assert_eq!(
            features.values[charge_i], 1.0,
            "a real retro disconnection on a neutral target/precursors must be charge-balanced"
        );
    }

    #[test]
    fn extract_features_num_precursors_survives_reparse_failure() {
        // A hand-built candidate with an unparseable precursor SMILES:
        // num_precursors is computable from the string list alone, but every
        // other structural/chemistry-integrity feature must be missing
        // rather than silently computed over only the precursors that
        // happened to re-parse.
        let target = "CCO";
        let target_mol = mol_from_smiles(target).unwrap();
        let candidate = ReactionCandidate {
            candidate_id: "sha256:fake".to_string(),
            target_smiles: target.to_string(),
            precursor_smiles: vec!["CC".to_string(), "not-a-valid-smiles(((".to_string()],
            sources: vec![],
            source_template_count: 0,
            best_upstream_score: None,
            best_upstream_rank: 0,
            min_base_step_cost: 0.0,
            max_template_frequency: None,
            mean_template_frequency: None,
            features: CandidateFeatures::default(),
            reranker_score: None,
        };
        let templates_by_id: HashMap<String, &RetroRule> = HashMap::new();
        let features = extract_features(&candidate, &target_mol, &templates_by_id, None);

        let num_i = feature_index("num_precursors").unwrap();
        assert!(!features.missing[num_i]);
        assert_eq!(features.values[num_i], 2.0);

        for name in [
            "target_heavy_atom_count",
            "precursor_heavy_atom_count_sum",
            "precursor_heavy_atom_count_max",
            "heavy_atom_retention_ratio",
            "net_charge_balanced",
            "no_heavy_atom_gain",
        ] {
            let i = feature_index(name).unwrap();
            assert!(
                features.missing[i],
                "{name} must be missing on reparse failure"
            );
        }
    }

    #[test]
    fn extract_features_is_deterministic_across_two_calls() {
        let rules = default_rules();
        let target = "CC(=O)c1ccccc1";
        let target_mol = mol_from_smiles(target).unwrap();
        let candidate = candidate_for(target, &rules, ProposalMode::Exhaustive);
        let templates_by_id = index_rules_by_template_id(&rules).unwrap();

        let a = extract_features(&candidate, &target_mol, &templates_by_id, None);
        let b = extract_features(&candidate, &target_mol, &templates_by_id, None);
        assert_eq!(a.values, b.values);
        assert_eq!(a.missing, b.missing);
    }
}
