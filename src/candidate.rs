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
//! evaluation (see the offline-evaluation docs, added alongside the
//! training pipeline).
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
use sha2::{Digest, Sha256};

use crate::chem_env::{
    Molecule, PrecursorMol, RetroRule, TemplateBondIndex, apply_retro, mol_from_smiles,
    to_canonical,
};
use crate::score::step_cost;
use crate::search::is_extracted_template;

#[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
pub use crate::scorer::nn::{TemplateScore, TemplateScoreOutput, TemplateScoreStatus};

/// Why a `ScoredRuleRef`'s `upstream_score` is `Some`/`None`. Distinct from
/// `TemplateScoreStatus` (a whole-scoring-call status): this is attached to
/// *each rule*, since `Exhaustive`/`BondIndexed` modes and hand-crafted rules
/// within `ScorerConditioned` mode never go through a scorer at all --
/// that's a different situation from a scorer being configured and failing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// Mirrors an active NN template scorer. `scores` must be pre-computed
    /// by the caller (e.g. via `TemplateScorer::score_templates`) -- this
    /// module never owns a scorer, so it can't silently fail to configure
    /// one. Hand-crafted rules (`!is_extracted_template`) are always
    /// included regardless of `scores`, matching `TemplateScorer`'s own
    /// `rules_offset` convention exactly.
    ScorerConditioned {
        #[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
        scores: Vec<TemplateScore>,
        #[cfg(not(all(not(target_arch = "wasm32"), feature = "nn-scoring")))]
        scores: Vec<(usize, f32, usize)>, // (rule_index, raw_logit, rank) -- scorer-independent shape
        top_k: usize,
    },
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
    /// `RetroRule.weight` (`ln(count+1)` for extracted templates, 1.0 for
    /// hand-crafted rules). NOT train-split-frozen yet -- that requires
    /// split-aware recomputation, added with the full feature schema/
    /// training-pipeline commit. Do not treat this as a leakage-safe
    /// feature on its own.
    pub template_log_frequency: Option<f32>,
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

pub struct CandidatePool {
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

fn transformation_cache() -> &'static Mutex<HashMap<String, TemplateTransformationFeatures>> {
    static CACHE: OnceLock<Mutex<HashMap<String, TemplateTransformationFeatures>>> =
        OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Bonds present with the same mapped-atom pair on both sides, but whose
/// `BondOrder` differs -- invisible to `chematic::rxn::find_reaction_center`
/// (it only compares bond *presence*, not order), so computed here as a
/// small complementary pass over the same public `atom_map`/`bond_between`
/// API `find_reaction_center` itself uses.
fn count_changed_bond_orders(rxn: &chematic::rxn::Reaction) -> u32 {
    use rustc_hash::FxHashMap;

    let mut reactant_map: FxHashMap<u16, (usize, chematic::core::AtomIdx)> = FxHashMap::default();
    for (mol_idx, mol) in rxn.reactants.iter().enumerate() {
        for (atom_idx, atom) in mol.atoms() {
            if let Some(map_num) = atom.atom_map {
                reactant_map.insert(map_num, (mol_idx, atom_idx));
            }
        }
    }
    let mut product_map: FxHashMap<u16, (usize, chematic::core::AtomIdx)> = FxHashMap::default();
    for (mol_idx, mol) in rxn.products.iter().enumerate() {
        for (atom_idx, atom) in mol.atoms() {
            if let Some(map_num) = atom.atom_map {
                product_map.insert(map_num, (mol_idx, atom_idx));
            }
        }
    }

    let mut changed = 0u32;
    let mut seen_pairs: std::collections::HashSet<(u16, u16)> = std::collections::HashSet::new();
    for (&map_a, &(r_mol_idx, r_atom_idx)) in &reactant_map {
        let r_mol = &rxn.reactants[r_mol_idx];
        for (r_neighbor, r_bond_idx) in r_mol.neighbors(r_atom_idx) {
            let Some(map_b) = r_mol.atom(r_neighbor).atom_map else {
                continue;
            };
            if map_b <= map_a {
                continue;
            }
            let pair = (map_a, map_b);
            if !seen_pairs.insert(pair) {
                continue;
            }
            let Some((p_mol_idx_a, p_atom_idx_a)) = product_map.get(&map_a).copied() else {
                continue;
            };
            let Some((p_mol_idx_b, p_atom_idx_b)) = product_map.get(&map_b).copied() else {
                continue;
            };
            if p_mol_idx_a != p_mol_idx_b {
                continue; // split apart -- that's a broken bond, not an order change
            }
            let p_mol = &rxn.products[p_mol_idx_a];
            let Some((_, p_bond)) = p_mol.bond_between(p_atom_idx_a, p_atom_idx_b) else {
                continue; // no longer bonded -- broken bond, handled by find_reaction_center
            };
            let r_bond = r_mol.bond(r_bond_idx);
            if r_bond.order != p_bond.order {
                changed += 1;
            }
        }
    }
    changed
}

/// Compute (and cache) template-level transformation features for one rule.
/// Keyed by `template_id` (stable, content-derived for extracted templates
/// per `template_id_for_smirks`).
pub fn template_transformation_features(rule: &RetroRule) -> TemplateTransformationFeatures {
    if let Some(cached) = transformation_cache()
        .lock()
        .unwrap()
        .get(&rule.template_id)
    {
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
                    let center = chematic::rxn::find_reaction_center(&rxn);
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
                    let changed_bond_order_count = count_changed_bond_orders(&rxn);
                    let mut center_atoms: std::collections::HashSet<chematic::core::AtomIdx> =
                        std::collections::HashSet::new();
                    for &(a, b) in center.broken_bonds.iter().chain(center.formed_bonds.iter()) {
                        center_atoms.insert(a);
                        center_atoms.insert(b);
                    }
                    for &a in &center.changed_atoms {
                        center_atoms.insert(a);
                    }
                    TemplateTransformationFeatures {
                        mapped_atom_count,
                        unmapped_atom_count,
                        deleted_bond_count: center.broken_bonds.len() as u32,
                        added_bond_count: center.formed_bonds.len() as u32,
                        changed_bond_order_count,
                        reaction_center_atom_count: center_atoms.len() as u32,
                        extractable: true,
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
        .insert(rule.template_id.clone(), features);
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
///   [`CandidateSource::template_log_frequency`]'s doc). These stay
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
    "atom_economy",
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
    // `template_log_frequency` is populated from `RetroRule.weight` as
    // computed over WHATEVER rule set the caller passed to
    // `propose_one_step`, not necessarily a train-split-frozen recomputation
    // -- always missing until split-aware recomputation lands, per
    // `CandidateSource::template_log_frequency`'s doc.
    missing[16] = true;
    missing[17] = true;

    CandidateFeatures { values, missing }
}

/// Build a `template_id -> &RetroRule` index once per pool-export call, for
/// [`extract_features`] to look up each candidate source's rule by id
/// without an O(rules) scan per source.
pub fn index_rules_by_template_id(rules: &[RetroRule]) -> HashMap<String, &RetroRule> {
    rules.iter().map(|r| (r.template_id.clone(), r)).collect()
}

/// Select the active rule set for one target under `mode`, mirroring
/// `find_routes`' bond_idx / scorer fallback chains exactly for the modes
/// that have a `find_routes` analog.
fn select_active_rules<'a>(
    target_mol: &Molecule,
    rules: &'a [RetroRule],
    mode: &ProposalMode,
) -> Vec<ScoredRuleRef<'a>> {
    match mode {
        ProposalMode::Exhaustive => rules
            .iter()
            .enumerate()
            .map(|(i, rule)| ScoredRuleRef {
                rule,
                source_rank: i,
                upstream_score: None,
                upstream_score_status: UpstreamScoreStatus::NotApplicable,
            })
            .collect(),
        ProposalMode::BondIndexed { top_k } => {
            let idx = TemplateBondIndex::build(rules);
            idx.retrieve(target_mol, *top_k, rules)
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
                .collect()
        }
        ProposalMode::ScorerConditioned { scores, top_k } => {
            let mut result = Vec::new();
            let mut rank = 0usize;
            for rule in rules.iter().filter(|r| !is_extracted_template(&r.name)) {
                result.push(ScoredRuleRef {
                    rule,
                    source_rank: rank,
                    upstream_score: None,
                    upstream_score_status: UpstreamScoreStatus::NotApplicable,
                });
                rank += 1;
            }
            #[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
            let mut by_rank: Vec<&TemplateScore> = scores.iter().collect();
            #[cfg(not(all(not(target_arch = "wasm32"), feature = "nn-scoring")))]
            let mut by_rank: Vec<&(usize, f32, usize)> = scores.iter().collect();
            #[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
            by_rank.sort_by_key(|s| s.rank);
            #[cfg(not(all(not(target_arch = "wasm32"), feature = "nn-scoring")))]
            by_rank.sort_by_key(|s| s.2);
            for entry in by_rank.into_iter().take(*top_k) {
                #[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
                let (rule_index, raw_logit) = (entry.rule_index, entry.raw_logit);
                #[cfg(not(all(not(target_arch = "wasm32"), feature = "nn-scoring")))]
                let (rule_index, raw_logit) = (entry.0, entry.1);
                if let Some(rule) = rules.get(rule_index) {
                    result.push(ScoredRuleRef {
                        rule,
                        source_rank: rank,
                        upstream_score: Some(raw_logit),
                        upstream_score_status: UpstreamScoreStatus::Available,
                    });
                    rank += 1;
                }
            }
            result
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

/// Canonicalize and merge raw proposals into candidate-pool entries.
///
/// Candidate ID: see [`candidate_id_for`]. Proposals whose sorted precursor
/// list hashes the same are merged: every source's full provenance is kept
/// in `sources`, `sources` is sorted deterministically (see
/// `ReactionCandidate` doc) so the "representative" source is never
/// ambiguous, and `best_*`/`min_*`/`max_*`/`mean_*` aggregates are computed
/// over all sources -- no provenance is dropped in favor of "just the best
/// one".
fn merge_into_candidates(canonical_target: &str, raw: Vec<RawCandidate>) -> Vec<ReactionCandidate> {
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
            template_log_frequency: Some(proposal.rule_weight as f32),
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
        .filter_map(|id| {
            let mut sources = sources_by_id.remove(&id)?;
            let precursor_smiles = precursors_by_id.remove(&id)?;

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
            });

            let best_upstream_score = sources
                .iter()
                .filter_map(|s| s.upstream_score)
                .fold(None, |acc: Option<f32>, v| {
                    Some(acc.map_or(v, |a| a.max(v)))
                });
            let best_upstream_rank = sources.iter().map(|s| s.original_rank).min().unwrap_or(0);
            let min_base_step_cost = sources
                .iter()
                .map(|s| s.base_step_cost)
                .fold(f64::INFINITY, f64::min);
            let freqs: Vec<f32> = sources
                .iter()
                .filter_map(|s| s.template_log_frequency)
                .collect();
            let max_template_frequency = freqs.iter().copied().fold(None, |acc: Option<f32>, v| {
                Some(acc.map_or(v, |a| a.max(v)))
            });
            let mean_template_frequency = if freqs.is_empty() {
                None
            } else {
                Some(freqs.iter().sum::<f32>() / freqs.len() as f32)
            };

            Some(ReactionCandidate {
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
pub fn propose_one_step(
    target_smiles: &str,
    rules: &[RetroRule],
    config: &ProposalConfig,
) -> anyhow::Result<CandidatePool> {
    let target_mol = mol_from_smiles(target_smiles)?;
    let canonical_target = to_canonical(&target_mol);

    let active_rules = select_active_rules(&target_mol, rules, &config.mode);
    let raw = raw_propose(&target_mol, &canonical_target, &active_rules);
    let candidates = merge_into_candidates(&canonical_target, raw);

    Ok(CandidatePool {
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

    #[test]
    fn exhaustive_mode_tries_all_rules() {
        let rules = default_rules();
        let target = "CC(=O)c1ccccc1";
        let target_mol = mol_from_smiles(target).unwrap();
        let active = select_active_rules(&target_mol, &rules, &ProposalMode::Exhaustive);
        assert_eq!(active.len(), rules.len());
        for r in &active {
            assert_eq!(r.upstream_score_status, UpstreamScoreStatus::NotApplicable);
            assert!(r.upstream_score.is_none());
        }
    }

    #[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
    fn make_score(rule_index: usize, raw_logit: f32, rank: usize) -> TemplateScore {
        TemplateScore {
            rule_index,
            raw_logit,
            rank,
        }
    }
    #[cfg(not(all(not(target_arch = "wasm32"), feature = "nn-scoring")))]
    fn make_score(rule_index: usize, raw_logit: f32, rank: usize) -> (usize, f32, usize) {
        (rule_index, raw_logit, rank)
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
            make_score(n_handcrafted, 0.9, 0),
            make_score(n_handcrafted + 1, 0.5, 1),
            make_score(n_handcrafted + 2, 0.1, 2),
        ];
        let mode = ProposalMode::ScorerConditioned { scores, top_k: 1 };
        let target = "CCCC";
        let target_mol = mol_from_smiles(target).unwrap();
        let active = select_active_rules(&target_mol, &rules, &mode);

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
            scores: vec![make_score(n_handcrafted, 0.9, 0)],
            top_k: 0, // no file templates selected at all
        };
        let target_mol = mol_from_smiles("CCCC").unwrap();
        let active = select_active_rules(&target_mol, &rules, &mode_zero_k);
        assert_eq!(
            active.len(),
            n_handcrafted,
            "hand-crafted rules must all still be present"
        );
        assert!(active.iter().all(|r| !is_extracted_template(&r.rule.name)));
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

        let exhaustive = select_active_rules(&target_mol, &rules, &ProposalMode::Exhaustive);
        let conditioned = select_active_rules(
            &target_mol,
            &rules,
            &ProposalMode::ScorerConditioned {
                scores: vec![
                    make_score(n_handcrafted, 0.9, 0),
                    make_score(n_handcrafted + 1, 0.1, 1),
                ],
                top_k: 1,
            },
        );
        assert!(
            conditioned.len() < exhaustive.len(),
            "scorer-conditioned selection must be a strict subset here, not just reordered"
        );
    }

    #[test]
    fn original_rank_matches_within_mode_rank() {
        let rules = default_rules();
        let target_mol = mol_from_smiles("CC(=O)c1ccccc1").unwrap();
        let active = select_active_rules(&target_mol, &rules, &ProposalMode::Exhaustive);
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
        let active = select_active_rules(&target_mol, &rules, &ProposalMode::Exhaustive);
        for r in &active {
            assert!(r.upstream_score.is_none());
            assert_eq!(r.upstream_score_status, UpstreamScoreStatus::NotApplicable);
        }
    }

    #[test]
    fn scorer_conditioned_with_no_scores_selects_only_handcrafted_no_frequency_substitution() {
        // Simulates the caller receiving TemplateScoreOutput { scores: vec![],
        // status: <failure> } from TemplateScorer::score_templates and
        // passing that empty `scores` straight through -- ScorerConditioned
        // must select exactly zero file templates in that case, never a
        // frequency-ranked top-K (that would be a silent, undocumented
        // fallback disguised as a successful narrowing), and never fall back
        // to Exhaustive on its own initiative.
        let mut rules = default_rules();
        let n_handcrafted = rules.len();
        rules.push(extracted_rule(0, "[C:1][C:2]>>[C:1].[C:2]", 3.0));
        rules.push(extracted_rule(1, "[C:1][C:2]>>[C:1].[C:2]", 2.0));

        let mode = ProposalMode::ScorerConditioned {
            scores: Vec::new(),
            top_k: 10,
        };
        let target_mol = mol_from_smiles("CCCC").unwrap();
        let active = select_active_rules(&target_mol, &rules, &mode);

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

        let active_rules = select_active_rules(&target_mol, &rules, &ProposalMode::Exhaustive);
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
        let merged = merge_into_candidates("target", raw);
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
        let merged = merge_into_candidates("target", raw);
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

        let merged = merge_into_candidates("target", vec![a, b]);
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
        let merged = merge_into_candidates("target", raw);
        assert_eq!(merged.len(), 1);
        // Tied on upstream_score and base_step_cost and original_rank -->
        // tie-break by template_id lexicographic: "a" before "z".
        assert_eq!(merged[0].sources[0].template_id, "a");
    }

    #[test]
    fn no_precursors_produces_no_self_loop_candidate() {
        let target = "CCO";
        let rules = vec![rule("noop", "[C:1]>>[C:1]")];
        let config = ProposalConfig::default();
        let pool = propose_one_step(target, &rules, &config).unwrap();
        for c in &pool.candidates {
            assert_ne!(c.precursor_smiles, vec![pool.target_smiles.clone()]);
        }
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

    fn candidate_for(target: &str, rules: &[RetroRule], mode: ProposalMode) -> ReactionCandidate {
        let pool = propose_one_step(target, rules, &ProposalConfig { mode }).unwrap();
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
        let templates_by_id = index_rules_by_template_id(&rules);

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
        let templates_by_id = index_rules_by_template_id(&rules);

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
        let templates_by_id = index_rules_by_template_id(&rules);

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
            "atom_economy",
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
        let templates_by_id = index_rules_by_template_id(&rules);

        let a = extract_features(&candidate, &target_mol, &templates_by_id, None);
        let b = extract_features(&candidate, &target_mol, &templates_by_id, None);
        assert_eq!(a.values, b.values);
        assert_eq!(a.missing, b.missing);
    }
}
