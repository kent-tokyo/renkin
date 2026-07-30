//! Static, partner-free forward-template analysis: which template slots a
//! known reactant can occupy, and what the remaining (missing) slots
//! require -- without inventing partner molecules or calling
//! `chematic::rxn::run_reactants`. Backs `renkin-forward hints`, a
//! retrieval-hint mode distinct from `predict` (all reactants known,
//! concrete products) and `enumerate` (one missing partner, filled from an
//! explicit library, concrete products).
//!
//! Unlike `enumerate`'s `contributing_lhs_slots` (which operates on
//! `chematic::rxn::Reaction`'s plain `Molecule`s from `parse_reaction`),
//! every type here is built from `chematic::smarts::parse_smarts`, so a
//! slot's `QueryAtom::atom_map` is always available -- `hints` never
//! depends on `parse_reaction` for slot splitting or matching.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};

use anyhow::Context;
use chematic::core::{AtomIdx, Element, Molecule};
use chematic::smarts::{
    self, AtomPrimitive, AtomQuery, BondPrimitive, BondQuery, MatchConfig, QueryMolecule,
};
use chematic::smiles::canonical_smiles;
use renkin::chem_env::{RetroRule, mol_from_smiles};
use rustc_hash::FxHashMap;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::hash_string_sequence;

/// Split a SMIRKS side into top-level (`.`-separated) components, treating
/// `[`/`]` and `(`/`)` as nesting delimiters so a `.` inside a bracket atom
/// or a parenthesized branch (including recursive SMARTS `$(...)`) is never
/// mistaken for a component separator.
///
/// Empirically checked against `chematic-smarts` 0.8.1: a literal `.` inside
/// `$(...)` is rejected outright by its parser (`parse_smarts("[$(C.C)]C")`
/// -> `UnexpectedChar('.', 1)`), so this scanner is defensive rather than
/// guarding an observed silent-misparse case -- but tracking depth costs
/// nothing and removes any doubt.
pub(crate) fn split_top_level_components(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            '.' if depth == 0 => {
                parts.push(s[start..i].to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(s[start..].to_string());
    parts
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

/// One top-level component of a forward SMIRKS side, parsed as a SMARTS
/// query via [`split_top_level_components`] + `chematic::smarts::parse_smarts`.
#[derive(Debug, Clone)]
pub(crate) struct QuerySlot {
    pub(crate) index: usize,
    pub(crate) source_smarts: String,
    pub(crate) query: QueryMolecule,
}

/// Why a template could not be prepared for static hint analysis -- always
/// counted (`stats.template_parse_failed`), never silently dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TemplateParseError {
    /// `reverse_smirks_validated` itself rejected the SMIRKS (wrong `>>`
    /// count, an empty side, or its own internal reaction-parse sanity
    /// check failed).
    ReversalFailed(String),
    /// A top-level LHS (forward reactant slot) component failed
    /// `chematic::smarts::parse_smarts`.
    LhsComponentUnparseable { component: String, message: String },
    /// A top-level RHS (forward product) component failed
    /// `chematic::smarts::parse_smarts`.
    RhsComponentUnparseable { component: String, message: String },
}

/// A reversed, fully-parsed template ready for static slot matching.
#[derive(Debug, Clone)]
pub(crate) struct ParsedHintTemplate {
    pub(crate) template_id: String,
    pub(crate) rule_name: String,
    pub(crate) template_weight: f64,
    pub(crate) lhs_slots: Vec<QuerySlot>,
    pub(crate) rhs_components: Vec<QuerySlot>,
}

fn parse_components(
    side: &str,
    on_error: impl Fn(String, String) -> TemplateParseError,
) -> Result<Vec<QuerySlot>, TemplateParseError> {
    split_top_level_components(side)
        .into_iter()
        .enumerate()
        .map(|(index, source_smarts)| {
            smarts::parse_smarts(&source_smarts)
                .map(|query| QuerySlot {
                    index,
                    source_smarts: source_smarts.clone(),
                    query,
                })
                .map_err(|e| on_error(source_smarts, format!("{e:?}")))
        })
        .collect()
}

/// Reverse a retro SMIRKS string's `>>` direction (`product >> precursors`
/// becomes `precursors >> product`), validating only the shape (exactly one
/// `>>`, neither side empty). Shared with `crate::reverse_smirks_validated`,
/// which layers its own additional `chematic::rxn::parse_reaction` sanity
/// check on top for its callers (`predict`/`enumerate`).
///
/// `hints` deliberately uses only this shape-only core, not the full
/// `reverse_smirks_validated`: `parse_reaction` parses each side with
/// `parse_smiles` and empirically rejects legitimate multi-condition SMARTS
/// a reactant slot may need (e.g. `[N;H1,H2:2]` fails with "invalid bracket
/// atom... missing ']'", since `;`/`,` logical operators aren't SMILES
/// syntax). `hints` validates every component with
/// `chematic::smarts::parse_smarts` immediately below instead, which is the
/// correct grammar for this feature and makes that gate both redundant and
/// wrong here.
pub(crate) fn reverse_smirks_shape_only(smirks: &str) -> Result<String, String> {
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
    Ok(format!("{rhs}>>{lhs}"))
}

/// Reverse a retro [`RetroRule`] into a fully-parsed forward template for
/// static hint analysis, or a counted [`TemplateParseError`].
pub(crate) fn parse_hint_template(
    rule: &RetroRule,
) -> Result<ParsedHintTemplate, TemplateParseError> {
    let fwd =
        reverse_smirks_shape_only(&rule.smirks).map_err(TemplateParseError::ReversalFailed)?;
    let parts: Vec<&str> = fwd.split(">>").collect();
    // reverse_smirks_shape_only guarantees exactly one '>>' and non-empty sides.
    let (fwd_lhs, fwd_rhs) = (parts[0], parts[1]);

    let lhs_slots = parse_components(fwd_lhs, |component, message| {
        TemplateParseError::LhsComponentUnparseable { component, message }
    })?;
    let rhs_components = parse_components(fwd_rhs, |component, message| {
        TemplateParseError::RhsComponentUnparseable { component, message }
    })?;

    Ok(ParsedHintTemplate {
        template_id: rule.template_id.clone(),
        rule_name: rule.name.clone(),
        template_weight: rule.weight,
        lhs_slots,
        rhs_components,
    })
}

/// Atom-map numbers appearing anywhere among the product-side components --
/// the set an LHS slot's mapped atoms must intersect to be "contributing"
/// (not a structural spectator).
pub(crate) fn product_side_atom_maps(rhs_components: &[QuerySlot]) -> BTreeSet<u16> {
    rhs_components
        .iter()
        .flat_map(|c| c.query.atoms.iter().filter_map(|a| a.atom_map))
        .collect()
}

/// Whether an LHS slot's query has at least one atom-map number also
/// present on the product side. A slot with zero atom-mapped atoms at all
/// has an empty intersection with anything and is therefore also treated as
/// a spectator -- consistent with `enumerate`'s existing convention (a
/// slot's contribution is established purely by shared atom-map numbers,
/// not by the mere presence of `:N` syntax).
pub(crate) fn slot_is_contributing(slot: &QuerySlot, product_side_maps: &BTreeSet<u16>) -> bool {
    slot.query
        .atoms
        .iter()
        .filter_map(|a| a.atom_map)
        .any(|m| product_side_maps.contains(&m))
}

/// Indices (into `template.lhs_slots`) of slots that are not statically
/// proven spectators.
pub(crate) fn contributing_slot_indices(template: &ParsedHintTemplate) -> Vec<usize> {
    let product_maps = product_side_atom_maps(&template.rhs_components);
    template
        .lhs_slots
        .iter()
        .filter(|s| slot_is_contributing(s, &product_maps))
        .map(|s| s.index)
        .collect()
}

/// One slot's raw VF2 embeddings (query atom index -> target `AtomIdx`) for
/// one known reactant.
type SlotMatchSites = Vec<FxHashMap<usize, AtomIdx>>;
/// Every contributing slot index a known reactant matches at least once,
/// with its (possibly capped) match sites and whether the cap was hit.
type ReactantSlotMatches = FxHashMap<usize, (SlotMatchSites, bool)>;

/// Every embedding of one slot's query into one known reactant, capped at
/// `max_matches_per_slot`, with an explicit flag when more embeddings
/// existed beyond the cap (requests one extra match internally to detect
/// this deterministically, rather than treating a result count equal to the
/// cap as ambiguous).
pub(crate) fn match_slot(
    slot: &QuerySlot,
    known_reactant: &Molecule,
    max_matches_per_slot: usize,
) -> (SlotMatchSites, bool) {
    let probe_cap = max_matches_per_slot.saturating_add(1);
    let config = MatchConfig {
        max_matches: Some(probe_cap),
        ..Default::default()
    };
    let mut matches = smarts::find_matches_with_config(&slot.query, known_reactant, &config);
    let truncated = matches.len() > max_matches_per_slot;
    matches.truncate(max_matches_per_slot);
    (matches, truncated)
}

/// One (known reactant, slot) pairing within a [`SlotAssignment`].
#[derive(Debug, Clone)]
pub(crate) struct SlotMatch {
    pub(crate) known_reactant_index: usize,
    pub(crate) slot_index: usize,
    pub(crate) match_sites: Vec<FxHashMap<usize, AtomIdx>>,
    pub(crate) match_sites_truncated: bool,
}

/// One complete injective assignment of every known reactant to a distinct
/// contributing slot of a [`ParsedHintTemplate`]; contributing slots left
/// over become `missing_slot_indices`.
#[derive(Debug, Clone)]
pub(crate) struct SlotAssignment {
    /// One entry per known reactant, ordered by `known_reactant_index`.
    pub(crate) matches: Vec<SlotMatch>,
    pub(crate) missing_slot_indices: Vec<usize>,
}

/// Caps for slot matching and assignment enumeration.
#[derive(Debug, Clone, Copy)]
pub(crate) struct HintMatchConfig {
    pub(crate) max_matches_per_slot: usize,
    pub(crate) max_assignments_per_template: usize,
}

/// Enumerate every injective assignment of `known_reactants` to distinct
/// contributing slots of `template` (every known reactant must be placed;
/// if there are more known reactants than contributing slots, the template
/// simply does not apply and no assignments are produced). Returns the
/// assignments plus whether the full permutation space was capped by
/// `max_assignments_per_template`.
///
/// This performs no reaction application: only `find_matches_with_config`
/// (static SMARTS matching) is ever called.
pub(crate) fn enumerate_slot_assignments(
    template: &ParsedHintTemplate,
    known_reactants: &[Molecule],
    config: &HintMatchConfig,
) -> (Vec<SlotAssignment>, bool) {
    let contributing = contributing_slot_indices(template);
    if known_reactants.is_empty() || known_reactants.len() > contributing.len() {
        return (vec![], false);
    }

    // matches_by_reactant[r] = slot_index -> (match_sites, truncated) for
    // every contributing slot the reactant matches at least once.
    let matches_by_reactant: Vec<ReactantSlotMatches> = known_reactants
        .iter()
        .map(|mol| {
            contributing
                .iter()
                .filter_map(|&slot_idx| {
                    let (sites, truncated) = match_slot(
                        &template.lhs_slots[slot_idx],
                        mol,
                        config.max_matches_per_slot,
                    );
                    if sites.is_empty() {
                        None
                    } else {
                        Some((slot_idx, (sites, truncated)))
                    }
                })
                .collect()
        })
        .collect();

    if matches_by_reactant.iter().any(FxHashMap::is_empty) {
        // At least one known reactant matches no contributing slot at all:
        // no valid assignment can place every known reactant.
        return (vec![], false);
    }

    let mut assignments = Vec::new();
    let mut truncated = false;
    let mut used_slots: Vec<usize> = Vec::with_capacity(known_reactants.len());
    assign_recursive(
        &matches_by_reactant,
        &contributing,
        &mut used_slots,
        config.max_assignments_per_template,
        &mut assignments,
        &mut truncated,
    );

    for assignment in &mut assignments {
        assignment.missing_slot_indices = contributing
            .iter()
            .filter(|slot_idx| {
                !assignment
                    .matches
                    .iter()
                    .any(|m| m.slot_index == **slot_idx)
            })
            .copied()
            .collect();
    }

    (assignments, truncated)
}

fn assign_recursive(
    matches_by_reactant: &[ReactantSlotMatches],
    contributing: &[usize],
    used_slots: &mut Vec<usize>,
    max_assignments: usize,
    out: &mut Vec<SlotAssignment>,
    truncated: &mut bool,
) {
    if out.len() >= max_assignments {
        *truncated = true;
        return;
    }
    if used_slots.len() == matches_by_reactant.len() {
        let matches = used_slots
            .iter()
            .enumerate()
            .map(|(reactant_idx, &slot_idx)| {
                let (sites, site_truncated) = &matches_by_reactant[reactant_idx][&slot_idx];
                SlotMatch {
                    known_reactant_index: reactant_idx,
                    slot_index: slot_idx,
                    match_sites: sites.clone(),
                    match_sites_truncated: *site_truncated,
                }
            })
            .collect();
        out.push(SlotAssignment {
            matches,
            missing_slot_indices: Vec::new(), // filled in by the caller
        });
        return;
    }
    let reactant_idx = used_slots.len();
    for &slot_idx in contributing {
        if used_slots.contains(&slot_idx) {
            continue;
        }
        if !matches_by_reactant[reactant_idx].contains_key(&slot_idx) {
            continue;
        }
        used_slots.push(slot_idx);
        assign_recursive(
            matches_by_reactant,
            contributing,
            used_slots,
            max_assignments,
            out,
            truncated,
        );
        used_slots.pop();
        if out.len() >= max_assignments {
            *truncated = true;
            return;
        }
    }
}

// ---------------------------------------------------------------------------
// Report schema
// ---------------------------------------------------------------------------

/// Schema version of [`ForwardRetrievalHintReport`]. Bump whenever a field
/// is added, removed, or its meaning changes.
pub const FORWARD_RETRIEVAL_HINT_REPORT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize)]
pub struct HintKnownReactantRef {
    pub input_index: usize,
    pub canonical_smiles: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HintMappedAtom {
    pub template_map: u16,
    pub target_atom_index: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct HintMatchSite {
    pub target_atom_indices: Vec<u32>,
    pub mapped_atoms: Vec<HintMappedAtom>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HintKnownAssignment {
    pub input_index: usize,
    pub slot_index: usize,
    pub match_sites: Vec<HintMatchSite>,
    /// `true` if `--max-matches-per-slot` capped the number of `match_sites`
    /// below the true count -- a capped assignment is not the complete
    /// picture of where this known reactant matches.
    pub match_sites_truncated: bool,
}

/// Conservative, best-effort summary of a missing partner slot's atom
/// constraints. `query_smarts` (on the enclosing [`HintMissingPartner`]) is
/// always authoritative; every field here is auxiliary and may be
/// incomplete -- see `summary_complete`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct HintRequiredFeatures {
    pub required_elements: Vec<String>,
    /// Elements excluded by a `NOT` over a single element primitive (e.g.
    /// `[!#6]` -> `["C"]`) -- an explicit negative constraint, not folded
    /// into `required_elements`.
    pub excluded_elements: Vec<String>,
    pub aromatic: Option<bool>,
    pub charge: Option<i8>,
    pub hydrogen_constraints: Vec<String>,
    pub degree: Option<u8>,
    pub ring_membership: Option<bool>,
    /// `[RN]` -- number of SSSR rings containing the atom (`R0` = acyclic).
    /// Distinct from `ring_membership` (`[R]`/`[!R]`, "in any ring or not").
    pub ring_count: Option<u8>,
    pub valence: Option<u8>,
    pub hybridization: Option<u8>,
    pub isotope: Option<u16>,
    pub chirality: Option<u8>,
    /// `false` when any atom in this slot's query contains: a recursive
    /// `$(...)` condition; a `NOT` over anything other than a single
    /// element (a bare `[!#6]`-style exclusion is fully captured by
    /// `excluded_elements` and does NOT make this `false`); or an `OR`
    /// combining primitives of genuinely different kinds (e.g. `[c,C]` --
    /// aromatic vs. aliphatic, or `[N,+1]` -- element vs. charge), since
    /// folding such alternatives into these fields would misrepresent a
    /// choice as a simultaneous requirement. An `OR` across values of the
    /// *same* primitive kind (e.g. `[N;H1,H2]`, `[N,O]`) is still
    /// considered completely summarized -- multiple `required_elements`/
    /// `hydrogen_constraints` entries already mean "any of these". In every
    /// case, `query_smarts` on the enclosing type remains the authoritative
    /// representation; this struct is always auxiliary.
    pub summary_complete: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct HintMissingPartner {
    pub slot_index: usize,
    pub query_smarts: String,
    pub required_features: HintRequiredFeatures,
}

#[derive(Debug, Clone, Serialize)]
pub struct HintBondChange {
    pub left_map: u16,
    pub right_map: u16,
    /// `"single"`/`"double"`/`"triple"`/`"aromatic"`/`"any"`/`"ring"`/
    /// `"up"`/`"down"`, or `"complex"` for a compound AND/OR/NOT bond query
    /// this describer does not attempt to flatten.
    pub order: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct HintTransformation {
    pub bonds_formed: Vec<HintBondChange>,
    pub bonds_broken: Vec<HintBondChange>,
    pub bonds_order_changed: Vec<HintBondChange>,
}

/// `basis` is always one of `"rule_name"` or `"derived_bond_delta"` --
/// never a numeric confidence, and never a named-reaction guess (e.g.
/// "Buchwald-Hartwig") unless trusted template metadata provides it, which
/// `RetroRule` does not currently carry.
#[derive(Debug, Clone, Serialize)]
pub struct HintReactionFamily {
    pub label: String,
    pub basis: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HintSource {
    pub template_id: String,
    pub rule_name: String,
    pub template_weight: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RetrievalHint {
    pub hint_id: String,
    pub rank: usize,
    pub reaction_family: HintReactionFamily,
    pub known_assignments: Vec<HintKnownAssignment>,
    pub missing_partners: Vec<HintMissingPartner>,
    pub transformation: HintTransformation,
    /// One query pattern per top-level forward-product component. Plural
    /// (unlike a single concrete product SMILES) because a template's
    /// product side may itself have multiple disconnected components (e.g.
    /// a metathesis-style swap) -- every one is a query pattern, never a
    /// concrete predicted structure.
    pub product_query_smarts: Vec<String>,
    /// Auxiliary only -- always derivable from `reaction_family`/
    /// `missing_partners`/`sources`, never overclaiming a named reaction or
    /// literal patent presence.
    pub search_terms: Vec<String>,
    /// Ranking signal only (maximum contributing source's template
    /// weight) -- not a probability of reaction success.
    pub proposal_score: f64,
    pub sources: Vec<HintSource>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ForwardRetrievalHintStats {
    pub rules_loaded: usize,
    pub templates_inspected: usize,
    /// A rule with an empty `smirks` field -- a graph-based/hard-coded
    /// transformation (e.g. Boc deprotection) rather than a reversible
    /// SMIRKS template, so it has no retro-SMIRKS to reverse and analyze
    /// statically. Same convention as `predict`/`enumerate`'s own
    /// `graph_rules_skipped`; never counted as a parse failure.
    pub graph_rules_skipped: usize,
    /// A non-empty retro SMIRKS that failed reversal or per-component
    /// SMARTS parsing -- always counted, the template is simply skipped,
    /// never a hard error for the whole report.
    pub template_parse_failed: usize,
    /// Sum of top-level forward-LHS + forward-RHS SMARTS components
    /// (`chematic::smarts::parse_smarts` calls) across every successfully
    /// parsed template -- the real per-template cost driver for the
    /// matching work below.
    pub smarts_components_parsed: usize,
    pub assignments_generated: usize,
    /// Templates for which `--max-assignments-per-template` cut off the
    /// full permutation space.
    pub templates_with_assignments_truncated: usize,
    pub hints_before_merge: usize,
    pub duplicate_hints_merged: usize,
    pub hints_returned: usize,
    pub hints_capped: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ForwardRetrievalHintReport {
    pub schema_version: u32,
    pub known_reactants: Vec<HintKnownReactantRef>,
    pub hints: Vec<RetrievalHint>,
    pub stats: ForwardRetrievalHintStats,
}

// ---------------------------------------------------------------------------
// Atom/bond feature extraction
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct AtomFeatureAccumulator {
    required_elements: BTreeSet<String>,
    excluded_elements: BTreeSet<String>,
    aromatic: Option<bool>,
    charge: Option<i8>,
    hcounts: BTreeSet<u8>,
    implicit_hcounts: BTreeSet<u8>,
    degree: Option<u8>,
    ring_membership: Option<bool>,
    ring_count: Option<u8>,
    valence: Option<u8>,
    hybridization: Option<u8>,
    isotope: Option<u16>,
    chirality: Option<u8>,
    has_not: bool,
    has_recursive: bool,
    /// A primitive kind this walker doesn't yet fold into a dedicated field
    /// (ring size/bond-count/total-connectivity queries) was present.
    has_unsummarized_primitive: bool,
    /// An `OR`'s branches disagreed on some scalar dimension (e.g. `[c,C]`:
    /// one branch is aromatic, the other isn't), or a branch contained
    /// NOT/recursive content -- either way that dimension represents
    /// alternative interpretations of the atom, not a simultaneous
    /// requirement, so it's left unset rather than arbitrarily committing
    /// to one branch's value.
    has_mixed_family_or: bool,
}

/// Whether `q` contains a `NOT` or recursive `$(...)` anywhere -- used to
/// decide if an OR branch is safe to walk normally even when
/// same-family, since a nested NOT/recursive still makes that branch
/// incompletely summarizable.
fn contains_not_or_recursive(q: &AtomQuery) -> bool {
    match q {
        AtomQuery::Primitive(AtomPrimitive::Recursive(_)) => true,
        AtomQuery::Primitive(_) => false,
        AtomQuery::And(a, b) | AtomQuery::Or(a, b) => {
            contains_not_or_recursive(a) || contains_not_or_recursive(b)
        }
        AtomQuery::Not(_) => true,
    }
}

/// Sets `has_not`/`has_recursive` flags for everything under `q` without
/// touching any value field -- used for the branches of a mixed-family OR,
/// which must not contribute to `required_elements`/etc. but whose
/// incompleteness (NOT/recursive content) must still be reflected in
/// `summary_complete`.
fn mark_incompleteness_flags_only(q: &AtomQuery, acc: &mut AtomFeatureAccumulator) {
    match q {
        AtomQuery::Primitive(AtomPrimitive::Recursive(_)) => acc.has_recursive = true,
        AtomQuery::Primitive(_) => {}
        AtomQuery::And(a, b) | AtomQuery::Or(a, b) => {
            mark_incompleteness_flags_only(a, acc);
            mark_incompleteness_flags_only(b, acc);
        }
        AtomQuery::Not(inner) => {
            acc.has_not = true;
            mark_incompleteness_flags_only(inner, acc);
        }
    }
}

fn walk_atom_query(q: &AtomQuery, acc: &mut AtomFeatureAccumulator) {
    match q {
        AtomQuery::Primitive(p) => apply_atom_primitive(p, acc),
        AtomQuery::And(a, b) => {
            walk_atom_query(a, acc);
            walk_atom_query(b, acc);
        }
        AtomQuery::Or(a, b) => walk_or(a, b, acc),
        AtomQuery::Not(inner) => walk_not(inner, acc),
    }
}

/// An `OR`'s two branches are each walked independently into their own
/// accumulator, then merged into the shared one:
///
/// - Multi-valued (set) fields (`required_elements`, `excluded_elements`,
///   `hcounts`, `implicit_hcounts`) are always safe to union -- a set
///   already means "any of these", exactly what OR expresses.
/// - Single-valued fields (`aromatic`, `charge`, `degree`, ...) are only
///   kept when *both* branches agree on the same value (e.g. `[N,O]`:
///   both branches happen to be non-aromatic, so `aromatic = Some(false)`
///   is genuinely true regardless of which branch matches). When branches
///   disagree (e.g. `[c,C]` -- aromatic vs. aliphatic carbon, both
///   equally valid interpretations), that field is left unset rather than
///   arbitrarily committing to one branch's value, and `has_mixed_family_or`
///   marks the atom incomplete.
///
/// A branch containing `NOT`/recursive `$(...)` is never walked for values
/// at all (only its incompleteness flags propagate) -- a negated or
/// recursive alternative can't be safely reduced to a positive value to
/// compare for agreement.
fn walk_or(a: &AtomQuery, b: &AtomQuery, acc: &mut AtomFeatureAccumulator) {
    if contains_not_or_recursive(a) || contains_not_or_recursive(b) {
        acc.has_mixed_family_or = true;
        mark_incompleteness_flags_only(a, acc);
        mark_incompleteness_flags_only(b, acc);
        return;
    }

    let mut acc_a = AtomFeatureAccumulator::default();
    walk_atom_query(a, &mut acc_a);
    let mut acc_b = AtomFeatureAccumulator::default();
    walk_atom_query(b, &mut acc_b);

    acc.required_elements
        .extend(acc_a.required_elements.iter().cloned());
    acc.required_elements
        .extend(acc_b.required_elements.iter().cloned());
    acc.excluded_elements
        .extend(acc_a.excluded_elements.iter().cloned());
    acc.excluded_elements
        .extend(acc_b.excluded_elements.iter().cloned());
    acc.hcounts.extend(acc_a.hcounts.iter().copied());
    acc.hcounts.extend(acc_b.hcounts.iter().copied());
    acc.implicit_hcounts
        .extend(acc_a.implicit_hcounts.iter().copied());
    acc.implicit_hcounts
        .extend(acc_b.implicit_hcounts.iter().copied());

    macro_rules! merge_scalar {
        ($field:ident) => {
            match (acc_a.$field, acc_b.$field) {
                (Some(x), Some(y)) if x == y => acc.$field = acc.$field.or(Some(x)),
                (None, None) => {}
                _ => acc.has_mixed_family_or = true,
            }
        };
    }
    merge_scalar!(aromatic);
    merge_scalar!(charge);
    merge_scalar!(degree);
    merge_scalar!(ring_membership);
    merge_scalar!(ring_count);
    merge_scalar!(valence);
    merge_scalar!(hybridization);
    merge_scalar!(isotope);
    merge_scalar!(chirality);

    acc.has_not = acc.has_not || acc_a.has_not || acc_b.has_not;
    acc.has_recursive = acc.has_recursive || acc_a.has_recursive || acc_b.has_recursive;
    acc.has_unsummarized_primitive = acc.has_unsummarized_primitive
        || acc_a.has_unsummarized_primitive
        || acc_b.has_unsummarized_primitive;
    acc.has_mixed_family_or =
        acc.has_mixed_family_or || acc_a.has_mixed_family_or || acc_b.has_mixed_family_or;
}

/// A `NOT` over a single element primitive is represented explicitly as an
/// exclusion (`excluded_elements`); any other NOT content (compound
/// expressions, non-element primitives) can't be safely reduced to a
/// positive field value, so it's left out of every field and flags the
/// atom incomplete instead.
fn walk_not(inner: &AtomQuery, acc: &mut AtomFeatureAccumulator) {
    acc.has_not = true;
    match inner {
        AtomQuery::Primitive(AtomPrimitive::AtomicNum(n)) => {
            if let Some(sym) = Element::from_atomic_number(*n) {
                acc.excluded_elements.insert(sym.symbol().to_string());
                return;
            }
        }
        AtomQuery::Primitive(AtomPrimitive::Symbol(s)) => {
            acc.excluded_elements.insert(s.clone());
            return;
        }
        _ => {}
    }
    mark_incompleteness_flags_only(inner, acc);
    acc.has_unsummarized_primitive = true;
}

fn apply_atom_primitive(p: &AtomPrimitive, acc: &mut AtomFeatureAccumulator) {
    match p {
        AtomPrimitive::AtomicNum(n) => {
            if let Some(sym) = Element::from_atomic_number(*n) {
                acc.required_elements.insert(sym.symbol().to_string());
            }
        }
        AtomPrimitive::Symbol(s) => {
            acc.required_elements.insert(s.clone());
        }
        AtomPrimitive::Aromatic(b) => acc.aromatic = Some(*b),
        AtomPrimitive::Charge(c) => acc.charge = Some(*c),
        AtomPrimitive::HCount(h) => {
            acc.hcounts.insert(*h);
        }
        AtomPrimitive::ImplicitHCount(h) => {
            acc.implicit_hcounts.insert(*h);
        }
        AtomPrimitive::Degree(d) => acc.degree = Some(*d),
        AtomPrimitive::RingMembership(b) => acc.ring_membership = Some(*b),
        AtomPrimitive::Valence(v) => acc.valence = Some(*v),
        AtomPrimitive::Hybridization(h) => acc.hybridization = Some(*h),
        AtomPrimitive::Isotope(i) => acc.isotope = Some(*i),
        AtomPrimitive::Chirality(c) => acc.chirality = Some(*c),
        AtomPrimitive::Wildcard => {}
        AtomPrimitive::Recursive(_) => acc.has_recursive = true,
        // Ring-size/bond-count/connectivity primitives don't yet have a
        // dedicated report field; recorded as "not fully summarized" rather
        // than silently dropped.
        AtomPrimitive::RingSize(_)
        | AtomPrimitive::MinRingSize(_)
        | AtomPrimitive::RingBondCount(_)
        | AtomPrimitive::TotalConnectivity(_) => acc.has_unsummarized_primitive = true,
        AtomPrimitive::RingCount(n) => acc.ring_count = Some(*n),
    }
}

fn format_hcount_constraint(prefix: &str, values: &BTreeSet<u8>) -> Option<String> {
    if values.is_empty() {
        return None;
    }
    let joined = values
        .iter()
        .map(|h| format!("{prefix}{h}"))
        .collect::<Vec<_>>()
        .join(" or ");
    Some(joined)
}

/// Merge feature constraints across every atom in a (missing-partner) slot's
/// query into one best-effort summary. `query_smarts` on the caller's
/// [`HintMissingPartner`] remains the authoritative representation; this is
/// auxiliary.
fn extract_required_features(query: &QueryMolecule) -> HintRequiredFeatures {
    let mut required_elements = BTreeSet::new();
    let mut excluded_elements = BTreeSet::new();
    let mut aromatic = None;
    let mut charge = None;
    let mut hcounts = BTreeSet::new();
    let mut implicit_hcounts = BTreeSet::new();
    let mut degree = None;
    let mut ring_membership = None;
    let mut ring_count = None;
    let mut valence = None;
    let mut hybridization = None;
    let mut isotope = None;
    let mut chirality = None;
    let mut summary_complete = true;

    for atom in &query.atoms {
        let mut acc = AtomFeatureAccumulator::default();
        walk_atom_query(&atom.query, &mut acc);
        required_elements.extend(acc.required_elements);
        excluded_elements.extend(acc.excluded_elements);
        aromatic = aromatic.or(acc.aromatic);
        charge = charge.or(acc.charge);
        hcounts.extend(acc.hcounts);
        implicit_hcounts.extend(acc.implicit_hcounts);
        degree = degree.or(acc.degree);
        ring_membership = ring_membership.or(acc.ring_membership);
        ring_count = ring_count.or(acc.ring_count);
        valence = valence.or(acc.valence);
        hybridization = hybridization.or(acc.hybridization);
        isotope = isotope.or(acc.isotope);
        chirality = chirality.or(acc.chirality);
        // `has_not` alone does NOT make an atom incomplete: a NOT over a
        // single element (`[!#6]`) is fully captured by
        // `excluded_elements`. Only a NOT/OR/recursive combination this
        // walker couldn't safely reduce to a field value does.
        if acc.has_recursive || acc.has_unsummarized_primitive || acc.has_mixed_family_or {
            summary_complete = false;
        }
    }

    let mut hydrogen_constraints = Vec::new();
    if let Some(s) = format_hcount_constraint("H", &hcounts) {
        hydrogen_constraints.push(s);
    }
    if let Some(s) = format_hcount_constraint("implicit H", &implicit_hcounts) {
        hydrogen_constraints.push(s);
    }

    HintRequiredFeatures {
        required_elements: required_elements.into_iter().collect(),
        excluded_elements: excluded_elements.into_iter().collect(),
        aromatic,
        charge,
        hydrogen_constraints,
        degree,
        ring_membership,
        ring_count,
        valence,
        hybridization,
        isotope,
        chirality,
        summary_complete,
    }
}

fn describe_bond_order(q: &BondQuery) -> String {
    match q {
        BondQuery::Primitive(BondPrimitive::Single) => "single".to_string(),
        BondQuery::Primitive(BondPrimitive::Double) => "double".to_string(),
        BondQuery::Primitive(BondPrimitive::Triple) => "triple".to_string(),
        BondQuery::Primitive(BondPrimitive::Aromatic) => "aromatic".to_string(),
        BondQuery::Primitive(BondPrimitive::Any) => "any".to_string(),
        BondQuery::Primitive(BondPrimitive::Ring) => "ring".to_string(),
        BondQuery::Primitive(BondPrimitive::Up) => "up".to_string(),
        BondQuery::Primitive(BondPrimitive::Down) => "down".to_string(),
        BondQuery::Any => "any".to_string(),
        BondQuery::And(..) | BondQuery::Or(..) | BondQuery::Not(..) => "complex".to_string(),
    }
}

/// `(atom_map_a, atom_map_b)` (sorted low-to-high) -> bond query, over every
/// mapped bond across all components on one side of a reversed template.
fn mapped_bond_signature(components: &[QuerySlot]) -> BTreeMap<(u16, u16), BondQuery> {
    let mut sig = BTreeMap::new();
    for comp in components {
        for bond in &comp.query.bonds {
            let a = comp.query.atoms[bond.atom1].atom_map;
            let b = comp.query.atoms[bond.atom2].atom_map;
            if let (Some(a), Some(b)) = (a, b) {
                let key = if a <= b { (a, b) } else { (b, a) };
                sig.insert(key, bond.query.clone());
            }
        }
    }
    sig
}

struct BondDelta {
    formed: Vec<(u16, u16, BondQuery)>,
    broken: Vec<(u16, u16, BondQuery)>,
    order_changed: Vec<(u16, u16, BondQuery)>,
}

/// Compare mapped bonds present on the forward-LHS (reactant) side against
/// the forward-RHS (product) side, keyed purely by atom-map number -- never
/// by calling `run_reactants`.
fn compute_bond_delta(lhs_components: &[QuerySlot], rhs_components: &[QuerySlot]) -> BondDelta {
    let lhs = mapped_bond_signature(lhs_components);
    let rhs = mapped_bond_signature(rhs_components);
    let mut formed = Vec::new();
    let mut broken = Vec::new();
    let mut order_changed = Vec::new();
    for (k, v) in &rhs {
        match lhs.get(k) {
            None => formed.push((k.0, k.1, v.clone())),
            Some(old) if old != v => order_changed.push((k.0, k.1, v.clone())),
            _ => {}
        }
    }
    for (k, v) in &lhs {
        if !rhs.contains_key(k) {
            broken.push((k.0, k.1, v.clone()));
        }
    }
    BondDelta {
        formed,
        broken,
        order_changed,
    }
}

fn element_for_atom_map(components: &[QuerySlot], atom_map: u16) -> Option<String> {
    for comp in components {
        for atom in &comp.query.atoms {
            if atom.atom_map == Some(atom_map) {
                let mut acc = AtomFeatureAccumulator::default();
                walk_atom_query(&atom.query, &mut acc);
                if let Some(sym) = acc.required_elements.into_iter().next() {
                    return Some(sym);
                }
            }
        }
    }
    None
}

/// Conservative reaction-family label derived only from the bond-delta
/// signature or the rule's own name -- never an inferred named reaction.
fn derive_reaction_family(
    rule_name: &str,
    lhs_components: &[QuerySlot],
    delta: &BondDelta,
) -> HintReactionFamily {
    if delta.formed.len() == 1 && delta.broken.is_empty() {
        let (a, b, _) = &delta.formed[0];
        let elem_a = element_for_atom_map(lhs_components, *a);
        let elem_b = element_for_atom_map(lhs_components, *b);
        if let (Some(elem_a), Some(elem_b)) = (elem_a, elem_b) {
            let mut pair = [elem_a, elem_b];
            pair.sort();
            return HintReactionFamily {
                label: format!("{}-{} bond formation", pair[0], pair[1]),
                basis: "derived_bond_delta".to_string(),
            };
        }
    }
    if delta.broken.len() == 1 && delta.formed.is_empty() {
        let (a, b, _) = &delta.broken[0];
        let elem_a = element_for_atom_map(lhs_components, *a);
        let elem_b = element_for_atom_map(lhs_components, *b);
        if let (Some(elem_a), Some(elem_b)) = (elem_a, elem_b) {
            let mut pair = [elem_a, elem_b];
            pair.sort();
            return HintReactionFamily {
                label: format!("{}-{} bond cleavage", pair[0], pair[1]),
                basis: "derived_bond_delta".to_string(),
            };
        }
    }
    HintReactionFamily {
        label: rule_name.replace(['_', '-'], " "),
        basis: "rule_name".to_string(),
    }
}

fn hint_merge_key(
    known_slot_roles: &[String],
    missing_partner_smarts: &[String],
    delta: &BondDelta,
    product_query_smarts: &[String],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"renkin-forward-hint-v1\0");

    let mut roles = known_slot_roles.to_vec();
    roles.sort();
    hash_string_sequence(&mut hasher, &roles);

    let mut missing = missing_partner_smarts.to_vec();
    missing.sort();
    hasher.update(b"\0missing\0");
    hash_string_sequence(&mut hasher, &missing);

    let mut bond_strs: Vec<String> = delta
        .formed
        .iter()
        .map(|(a, b, q)| format!("+{a}-{b}:{}", describe_bond_order(q)))
        .chain(
            delta
                .broken
                .iter()
                .map(|(a, b, q)| format!("-{a}-{b}:{}", describe_bond_order(q))),
        )
        .chain(
            delta
                .order_changed
                .iter()
                .map(|(a, b, q)| format!("~{a}-{b}:{}", describe_bond_order(q))),
        )
        .collect();
    bond_strs.sort();
    hasher.update(b"\0bonds\0");
    hash_string_sequence(&mut hasher, &bond_strs);

    let mut prods = product_query_smarts.to_vec();
    prods.sort();
    hasher.update(b"\0products\0");
    hash_string_sequence(&mut hasher, &prods);

    format!("sha256:{:x}", hasher.finalize())
}

// ---------------------------------------------------------------------------
// Hint assembly
// ---------------------------------------------------------------------------

/// Caps for [`generate_retrieval_hints`], mirroring `ForwardEnumerationConfig`'s
/// shape.
#[derive(Debug, Clone, Copy)]
pub struct HintGenerationConfig {
    pub max_hints: usize,
    pub max_matches_per_slot: usize,
    pub max_assignments_per_template: usize,
}

impl Default for HintGenerationConfig {
    fn default() -> Self {
        Self {
            max_hints: 50,
            max_matches_per_slot: 20,
            max_assignments_per_template: 200,
        }
    }
}

fn build_hint(template: &ParsedHintTemplate, assignment: &SlotAssignment) -> RetrievalHint {
    let known_assignments: Vec<HintKnownAssignment> = assignment
        .matches
        .iter()
        .map(|m| {
            let match_sites = m
                .match_sites
                .iter()
                .map(|site| {
                    let mut target_atom_indices: Vec<u32> =
                        site.values().map(|idx| idx.0).collect();
                    target_atom_indices.sort_unstable();
                    let slot = &template.lhs_slots[m.slot_index];
                    let mut mapped_atoms: Vec<HintMappedAtom> = site
                        .iter()
                        .filter_map(|(query_idx, target_idx)| {
                            slot.query.atoms[*query_idx].atom_map.map(|template_map| {
                                HintMappedAtom {
                                    template_map,
                                    target_atom_index: target_idx.0,
                                }
                            })
                        })
                        .collect();
                    mapped_atoms.sort_by_key(|a| (a.template_map, a.target_atom_index));
                    HintMatchSite {
                        target_atom_indices,
                        mapped_atoms,
                    }
                })
                .collect();
            HintKnownAssignment {
                input_index: m.known_reactant_index,
                slot_index: m.slot_index,
                match_sites,
                match_sites_truncated: m.match_sites_truncated,
            }
        })
        .collect();

    let missing_partners: Vec<HintMissingPartner> = assignment
        .missing_slot_indices
        .iter()
        .map(|&slot_idx| {
            let slot = &template.lhs_slots[slot_idx];
            HintMissingPartner {
                slot_index: slot_idx,
                query_smarts: slot.source_smarts.clone(),
                required_features: extract_required_features(&slot.query),
            }
        })
        .collect();

    let delta = compute_bond_delta(&template.lhs_slots, &template.rhs_components);
    let transformation = HintTransformation {
        bonds_formed: delta
            .formed
            .iter()
            .map(|(a, b, q)| HintBondChange {
                left_map: *a,
                right_map: *b,
                order: describe_bond_order(q),
            })
            .collect(),
        bonds_broken: delta
            .broken
            .iter()
            .map(|(a, b, q)| HintBondChange {
                left_map: *a,
                right_map: *b,
                order: describe_bond_order(q),
            })
            .collect(),
        bonds_order_changed: delta
            .order_changed
            .iter()
            .map(|(a, b, q)| HintBondChange {
                left_map: *a,
                right_map: *b,
                order: describe_bond_order(q),
            })
            .collect(),
    };

    let reaction_family = derive_reaction_family(&template.rule_name, &template.lhs_slots, &delta);
    let product_query_smarts: Vec<String> = template
        .rhs_components
        .iter()
        .map(|c| c.source_smarts.clone())
        .collect();

    let mut search_terms = vec![reaction_family.label.clone()];
    search_terms.push(template.rule_name.replace(['_', '-'], " "));
    for mp in &missing_partners {
        for elem in &mp.required_features.required_elements {
            search_terms.push(format!("{elem} partner"));
        }
    }
    search_terms.sort();
    search_terms.dedup();

    let known_slot_roles: Vec<String> = assignment
        .matches
        .iter()
        .map(|m| template.lhs_slots[m.slot_index].source_smarts.clone())
        .collect();
    let missing_partner_smarts: Vec<String> = missing_partners
        .iter()
        .map(|m| m.query_smarts.clone())
        .collect();
    let hint_id = hint_merge_key(
        &known_slot_roles,
        &missing_partner_smarts,
        &delta,
        &product_query_smarts,
    );

    RetrievalHint {
        hint_id,
        rank: 0, // assigned after sort/merge
        reaction_family,
        known_assignments,
        missing_partners,
        transformation,
        product_query_smarts,
        search_terms,
        proposal_score: template.template_weight,
        sources: vec![HintSource {
            template_id: template.template_id.clone(),
            rule_name: template.rule_name.clone(),
            template_weight: template.template_weight,
        }],
    }
}

/// Generate partner-free forward retrieval hints for `known_reactant_smiles`
/// against `rules`. Never calls `chematic::rxn::run_reactants` and never
/// invents partner molecules -- purely static SMARTS matching plus
/// atom-map-based bond-delta analysis.
pub fn generate_retrieval_hints(
    known_reactant_smiles: &[&str],
    rules: &[RetroRule],
    config: &HintGenerationConfig,
) -> anyhow::Result<ForwardRetrievalHintReport> {
    let known_reactants: Vec<Molecule> = known_reactant_smiles
        .iter()
        .map(|s| {
            mol_from_smiles(s).with_context(|| format!("invalid known reactant SMILES: {s:?}"))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let known_reactant_refs: Vec<HintKnownReactantRef> = known_reactants
        .iter()
        .enumerate()
        .map(|(input_index, mol)| HintKnownReactantRef {
            input_index,
            canonical_smiles: canonical_smiles(mol),
        })
        .collect();

    let mut stats = ForwardRetrievalHintStats {
        rules_loaded: rules.len(),
        ..Default::default()
    };
    let match_config = HintMatchConfig {
        max_matches_per_slot: config.max_matches_per_slot,
        max_assignments_per_template: config.max_assignments_per_template,
    };

    // hint_id -> merged hint (first occurrence's fields, extra sources appended)
    let mut merged: BTreeMap<String, RetrievalHint> = BTreeMap::new();

    for rule in rules {
        stats.templates_inspected += 1;
        if rule.smirks.is_empty() {
            stats.graph_rules_skipped += 1;
            continue;
        }
        let template = match parse_hint_template(rule) {
            Ok(t) => t,
            Err(_) => {
                stats.template_parse_failed += 1;
                continue;
            }
        };
        stats.smarts_components_parsed += template.lhs_slots.len() + template.rhs_components.len();

        let (assignments, truncated) =
            enumerate_slot_assignments(&template, &known_reactants, &match_config);
        if truncated {
            stats.templates_with_assignments_truncated += 1;
        }
        stats.assignments_generated += assignments.len();

        for assignment in &assignments {
            let hint = build_hint(&template, assignment);
            stats.hints_before_merge += 1;
            match merged.entry(hint.hint_id.clone()) {
                Entry::Occupied(mut existing) => {
                    stats.duplicate_hints_merged += 1;
                    let existing = existing.get_mut();
                    existing.sources.extend(hint.sources);
                    existing
                        .sources
                        .sort_by(|a, b| a.template_id.cmp(&b.template_id));
                    existing
                        .sources
                        .dedup_by(|a, b| a.template_id == b.template_id);
                    existing.proposal_score = existing.proposal_score.max(hint.proposal_score);
                }
                Entry::Vacant(slot) => {
                    slot.insert(hint);
                }
            }
        }
    }

    let mut hints: Vec<RetrievalHint> = merged.into_values().collect();
    hints.sort_by(|a, b| {
        b.proposal_score
            .partial_cmp(&a.proposal_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.hint_id.cmp(&b.hint_id))
    });
    let hints_capped = hints.len() > config.max_hints;
    hints.truncate(config.max_hints);
    for (rank, hint) in hints.iter_mut().enumerate() {
        hint.rank = rank;
    }

    stats.hints_returned = hints.len();
    stats.hints_capped = hints_capped;

    Ok(ForwardRetrievalHintReport {
        schema_version: FORWARD_RETRIEVAL_HINT_REPORT_SCHEMA_VERSION,
        known_reactants: known_reactant_refs,
        hints,
        stats,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use renkin::chem_env::mol_from_smiles;

    fn default_config() -> HintMatchConfig {
        HintMatchConfig {
            max_matches_per_slot: 50,
            max_assignments_per_template: 100,
        }
    }

    fn rule(name: &str, smirks: &str) -> RetroRule {
        RetroRule {
            name: name.to_string(),
            template_id: format!("rule:{name}"),
            smirks: smirks.to_string(),
            weight: 1.0,
            required_elements: 0,
        }
    }

    #[test]
    fn split_top_level_components_respects_brackets_and_parens() {
        assert_eq!(
            split_top_level_components("[c:1][Br].[N:2]"),
            vec!["[c:1][Br]".to_string(), "[N:2]".to_string()]
        );
        // A '.' can never legally appear inside `[...]` bracket-atom syntax,
        // but the scanner must not be fooled by brackets containing other
        // punctuation-like characters either.
        assert_eq!(
            split_top_level_components("[c:1]1ccccc1.[NH2:2]CC"),
            vec!["[c:1]1ccccc1".to_string(), "[NH2:2]CC".to_string()]
        );
    }

    // --- Test 1: known aryl electrophile + missing nitrogen partner ---
    #[test]
    fn known_aryl_electrophile_matches_one_slot_leaves_nitrogen_slot_missing() {
        // Retro: aryl amine decomposes into an aryl bromide + an amine.
        let r = rule("aryl_amination", "[c:1][N;H1,H2:2]>>[c:1][Br].[N;H1,H2:2]");
        let template = parse_hint_template(&r).unwrap();
        assert_eq!(template.lhs_slots.len(), 2);

        let bromobenzene = mol_from_smiles("Brc1ccccc1").unwrap();
        let (assignments, truncated) =
            enumerate_slot_assignments(&template, &[bromobenzene], &default_config());
        assert!(!truncated);
        assert_eq!(assignments.len(), 1, "aryl-Br slot must match exactly once");
        let a = &assignments[0];
        assert_eq!(a.matches.len(), 1);
        assert_eq!(a.matches[0].known_reactant_index, 0);
        assert_eq!(
            a.missing_slot_indices.len(),
            1,
            "the amine slot must be missing"
        );
    }

    // --- Test 2: known amine + missing carbon electrophile ---
    #[test]
    fn known_amine_matches_nitrogen_slot_leaves_carbon_slot_missing() {
        let r = rule("aryl_amination", "[c:1][N;H1,H2:2]>>[c:1][Br].[N;H1,H2:2]");
        let template = parse_hint_template(&r).unwrap();

        let amine = mol_from_smiles("NCC").unwrap();
        let (assignments, _) = enumerate_slot_assignments(&template, &[amine], &default_config());
        assert_eq!(assignments.len(), 1);
        assert_eq!(assignments[0].matches[0].slot_index, 1);
        assert_eq!(assignments[0].missing_slot_indices, vec![0]);
    }

    // --- Test 3: unary transformation with zero missing partners ---
    #[test]
    fn unary_transformation_has_zero_missing_partners() {
        let r = rule("halide_swap", "[c:1][Cl]>>[c:1][Br]");
        let template = parse_hint_template(&r).unwrap();
        assert_eq!(template.lhs_slots.len(), 1);

        let bromobenzene = mol_from_smiles("Brc1ccccc1").unwrap();
        let (assignments, _) =
            enumerate_slot_assignments(&template, &[bromobenzene], &default_config());
        assert_eq!(assignments.len(), 1);
        assert!(assignments[0].missing_slot_indices.is_empty());
    }

    // --- Test 4: arity-3 template with two missing partners ---
    #[test]
    fn arity_3_template_reports_two_missing_partners() {
        let r = rule(
            "triple_coupling",
            "[C:1][C:2][C:3]>>[C:1][Br].[C:2][Cl].[C:3][I]",
        );
        let template = parse_hint_template(&r).unwrap();
        assert_eq!(template.lhs_slots.len(), 3);

        let bromomethane = mol_from_smiles("CBr").unwrap();
        let (assignments, _) =
            enumerate_slot_assignments(&template, &[bromomethane], &default_config());
        assert_eq!(assignments.len(), 1);
        assert_eq!(assignments[0].missing_slot_indices.len(), 2);
    }

    // --- Test 5: spectator-only slot rejection ---
    #[test]
    fn spectator_only_slot_never_produces_an_assignment() {
        // Forward product atom-maps {8,9} share zero overlap with the
        // single forward reactant slot's atom-maps {1,2} -- same shape as
        // enumerate's synthetic_disconnected_rule fixture.
        let r = rule("disconnected", "[C:9]=[O:8]>>[C:1][Cl:2]");
        let template = parse_hint_template(&r).unwrap();
        assert_eq!(contributing_slot_indices(&template), Vec::<usize>::new());

        let known = mol_from_smiles("CCl").unwrap();
        let (assignments, _) = enumerate_slot_assignments(&template, &[known], &default_config());
        assert!(
            assignments.is_empty(),
            "a structural spectator must never be assigned"
        );
    }

    // --- Test 6: two functional-group match sites on one known molecule ---
    #[test]
    fn two_match_sites_on_one_known_molecule_are_both_reported() {
        let r = rule("halide_swap", "[c:1][Cl]>>[c:1][Br]");
        let template = parse_hint_template(&r).unwrap();
        // Two bromines on the same ring -> two distinct embeddings of the
        // single-atom aromatic-Br slot query.
        let dibromobenzene = mol_from_smiles("Brc1ccc(Br)cc1").unwrap();
        let (assignments, _) =
            enumerate_slot_assignments(&template, &[dibromobenzene], &default_config());
        assert_eq!(assignments.len(), 1);
        assert_eq!(
            assignments[0].matches[0].match_sites.len(),
            2,
            "both aromatic bromine sites must be reported"
        );
    }

    // --- Test 7: two known reactants assigned to distinct slots ---
    #[test]
    fn two_known_reactants_assign_to_distinct_slots() {
        let r = rule("aryl_amination", "[c:1][N;H1,H2:2]>>[c:1][Br].[N;H1,H2:2]");
        let template = parse_hint_template(&r).unwrap();
        let bromobenzene = mol_from_smiles("Brc1ccccc1").unwrap();
        let amine = mol_from_smiles("NCC").unwrap();
        let (assignments, truncated) =
            enumerate_slot_assignments(&template, &[bromobenzene, amine], &default_config());
        assert!(!truncated);
        assert_eq!(assignments.len(), 1);
        let a = &assignments[0];
        assert!(a.missing_slot_indices.is_empty());
        assert_eq!(a.matches.len(), 2);
        let slot_for = |reactant_idx: usize| {
            a.matches
                .iter()
                .find(|m| m.known_reactant_index == reactant_idx)
                .unwrap()
                .slot_index
        };
        assert_eq!(
            slot_for(0),
            0,
            "the bromobenzene must land in the aryl slot"
        );
        assert_eq!(slot_for(1), 1, "the amine must land in the amine slot");
    }

    #[test]
    fn more_known_reactants_than_contributing_slots_yields_no_assignment() {
        let r = rule("halide_swap", "[c:1][Cl]>>[c:1][Br]");
        let template = parse_hint_template(&r).unwrap();
        let a = mol_from_smiles("Brc1ccccc1").unwrap();
        let b = mol_from_smiles("Brc1ccc(Br)cc1").unwrap();
        let (assignments, _) = enumerate_slot_assignments(&template, &[a, b], &default_config());
        assert!(assignments.is_empty());
    }

    #[test]
    fn max_matches_per_slot_caps_and_flags_truncation() {
        let r = rule("halide_swap", "[c:1][Cl]>>[c:1][Br]");
        let template = parse_hint_template(&r).unwrap();
        let tribromobenzene = mol_from_smiles("Brc1cc(Br)cc(Br)c1").unwrap();
        let capped = HintMatchConfig {
            max_matches_per_slot: 2,
            max_assignments_per_template: 100,
        };
        let (assignments, _) = enumerate_slot_assignments(&template, &[tribromobenzene], &capped);
        assert_eq!(assignments.len(), 1);
        assert_eq!(assignments[0].matches[0].match_sites.len(), 2);
        assert!(assignments[0].matches[0].match_sites_truncated);
    }

    #[test]
    fn parse_hint_template_reports_reversal_failure_not_a_panic() {
        let r = rule("bad", "not a valid smirks");
        let err = parse_hint_template(&r).unwrap_err();
        assert!(matches!(err, TemplateParseError::ReversalFailed(_)));
    }

    fn default_report_config() -> HintGenerationConfig {
        HintGenerationConfig {
            max_hints: 50,
            max_matches_per_slot: 50,
            max_assignments_per_template: 100,
        }
    }

    #[test]
    fn end_to_end_aryl_amination_reports_one_hint_with_missing_nitrogen_partner() {
        let r = rule("aryl_amination", "[c:1][N;H1,H2:2]>>[c:1][Br].[N;H1,H2:2]");
        let report =
            generate_retrieval_hints(&["Brc1ccccc1"], &[r], &default_report_config()).unwrap();

        assert_eq!(
            report.schema_version,
            FORWARD_RETRIEVAL_HINT_REPORT_SCHEMA_VERSION
        );
        assert_eq!(report.known_reactants.len(), 1);
        assert_eq!(report.known_reactants[0].input_index, 0);
        assert_eq!(report.hints.len(), 1);

        let hint = &report.hints[0];
        assert_eq!(hint.known_assignments.len(), 1);
        assert_eq!(hint.known_assignments[0].slot_index, 0);
        assert_eq!(hint.missing_partners.len(), 1);
        let missing = &hint.missing_partners[0];
        assert_eq!(missing.slot_index, 1);
        assert_eq!(missing.query_smarts, "[N;H1,H2:2]");
        assert_eq!(
            missing.required_features.required_elements,
            vec!["N".to_string()]
        );
        assert_eq!(
            missing.required_features.hydrogen_constraints,
            vec!["H1 or H2".to_string()]
        );
        assert!(missing.required_features.summary_complete);
        assert_eq!(
            hint.product_query_smarts,
            vec!["[c:1][N;H1,H2:2]".to_string()]
        );
        assert_eq!(hint.sources.len(), 1);
        assert_eq!(hint.sources[0].template_id, "rule:aryl_amination");
        assert_eq!(report.stats.hints_returned, 1);
        assert_eq!(report.stats.hints_before_merge, 1);
        assert_eq!(report.stats.duplicate_hints_merged, 0);
        assert!(!report.stats.hints_capped);
    }

    #[test]
    fn two_templates_with_the_same_retrieval_signature_merge_into_one_hint() {
        // Two differently-named rules that reverse to the exact same
        // forward slots/bond-delta/product must merge, keeping both as
        // distinct provenance sources.
        let r1 = rule("halide_swap_v1", "[c:1][Cl]>>[c:1][Br]");
        let r2 = rule("halide_swap_v2", "[c:1][Cl]>>[c:1][Br]");
        let report =
            generate_retrieval_hints(&["Brc1ccccc1"], &[r1, r2], &default_report_config()).unwrap();

        assert_eq!(report.hints.len(), 1);
        assert_eq!(report.stats.hints_before_merge, 2);
        assert_eq!(report.stats.duplicate_hints_merged, 1);
        let mut template_ids: Vec<&str> = report.hints[0]
            .sources
            .iter()
            .map(|s| s.template_id.as_str())
            .collect();
        template_ids.sort_unstable();
        assert_eq!(
            template_ids,
            vec!["rule:halide_swap_v1", "rule:halide_swap_v2"]
        );
    }

    #[test]
    fn no_matching_template_produces_an_empty_but_valid_report() {
        let r = rule("aryl_amination", "[c:1][N;H1,H2:2]>>[c:1][Br].[N;H1,H2:2]");
        let report = generate_retrieval_hints(&["CCO"], &[r], &default_report_config()).unwrap();
        assert!(report.hints.is_empty());
        assert_eq!(report.stats.hints_before_merge, 0);
        assert_eq!(report.stats.templates_inspected, 1);
        assert_eq!(report.stats.template_parse_failed, 0);
    }

    #[test]
    fn malformed_template_is_counted_and_skipped_not_a_hard_error() {
        let good = rule("halide_swap", "[c:1][Cl]>>[c:1][Br]");
        let bad = rule("bad", "not a valid smirks");
        let report =
            generate_retrieval_hints(&["Brc1ccccc1"], &[good, bad], &default_report_config())
                .unwrap();
        assert_eq!(report.stats.templates_inspected, 2);
        assert_eq!(report.stats.template_parse_failed, 1);
        assert_eq!(
            report.hints.len(),
            1,
            "the good template must still produce a hint"
        );
    }

    #[test]
    fn graph_based_rule_is_skipped_and_counted_not_as_a_parse_failure() {
        let graph_rule = rule("graph_rule", "");
        let good = rule("halide_swap", "[c:1][Cl]>>[c:1][Br]");
        let report = generate_retrieval_hints(
            &["Brc1ccccc1"],
            &[graph_rule, good],
            &default_report_config(),
        )
        .unwrap();
        assert_eq!(report.stats.templates_inspected, 2);
        assert_eq!(report.stats.graph_rules_skipped, 1);
        assert_eq!(report.stats.template_parse_failed, 0);
        assert_eq!(report.hints.len(), 1);
    }

    // Regression audit companion to lib.rs's
    // `reverse_smirks_validated_still_rejects_*` fixtures: `hints` deliberately
    // ACCEPTS what `predict`/`enumerate` reject, via its own shape-only
    // reversal + `parse_smarts`-per-component validation. `reverse_smirks_shape_only`
    // itself only checks `>>` structure, so it also accepts things that fail
    // later at the per-component `parse_smarts` stage (unbalanced bracket,
    // invalid atom-map token) -- both stages are exercised here.
    #[test]
    fn reverse_smirks_shape_only_accepts_multi_condition_smarts() {
        assert!(reverse_smirks_shape_only("[c:1][N;H1,H2:2]>>[c:1][Br].[N;H1,H2:2]").is_ok());
    }

    #[test]
    fn parse_hint_template_accepts_multi_condition_smarts() {
        let r = rule("aryl_amination", "[c:1][N;H1,H2:2]>>[c:1][Br].[N;H1,H2:2]");
        assert!(parse_hint_template(&r).is_ok());
    }

    #[test]
    fn parse_hint_template_accepts_recursive_smarts() {
        let r = rule("recursive_probe", "[c:1][C:2]>>[c:1][Br].[C;$(C=O):2]");
        assert!(parse_hint_template(&r).is_ok());
    }

    #[test]
    fn parse_hint_template_rejects_unbalanced_bracket_at_component_stage() {
        // reverse_smirks_shape_only accepts the `>>` shape; the failure
        // surfaces one stage later, at per-component parse_smarts.
        let r = rule("bad_bracket", "[c:1][N:2>>[c:1].[N:2]");
        assert!(reverse_smirks_shape_only(&r.smirks).is_ok());
        let err = parse_hint_template(&r).unwrap_err();
        assert!(matches!(
            err,
            TemplateParseError::RhsComponentUnparseable { .. }
        ));
    }

    #[test]
    fn parse_hint_template_rejects_invalid_atom_map_token_at_component_stage() {
        let r = rule("bad_atom_map", "[c:1][N:xyz]>>[c:1].[N:xyz]");
        assert!(reverse_smirks_shape_only(&r.smirks).is_ok());
        let err = parse_hint_template(&r).unwrap_err();
        assert!(matches!(
            err,
            TemplateParseError::LhsComponentUnparseable { .. }
        ));
    }

    #[test]
    fn parse_hint_template_rejects_malformed_arrow_and_empty_sides() {
        assert!(matches!(
            parse_hint_template(&rule("zero_arrows", "[C:1][C:2]")).unwrap_err(),
            TemplateParseError::ReversalFailed(_)
        ));
        assert!(matches!(
            parse_hint_template(&rule("multi_arrows", "[C:1]>>[C:2]>>[C:3]")).unwrap_err(),
            TemplateParseError::ReversalFailed(_)
        ));
        assert!(matches!(
            parse_hint_template(&rule("empty_lhs", ">>[C:1]")).unwrap_err(),
            TemplateParseError::ReversalFailed(_)
        ));
        assert!(matches!(
            parse_hint_template(&rule("empty_rhs", "[C:1]>>")).unwrap_err(),
            TemplateParseError::ReversalFailed(_)
        ));
    }

    /// Snapshot-style regression guard mirroring lib.rs's
    /// `reverse_smirks_validated_default_rules_accept_reject_partition_is_stable`:
    /// locks in hints' own (more permissive) accept/reject partition over
    /// the real embedded default-rule corpus, so a future change to either
    /// validator's strictness is caught rather than silently drifting.
    #[test]
    fn parse_hint_template_default_rules_accept_reject_partition_is_stable() {
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
            match parse_hint_template(rule) {
                Ok(_) => accepted += 1,
                Err(_) => rejected += 1,
            }
        }
        assert_eq!(graph_based, 7);
        assert_eq!(smirks_based, rules.len() - 7);
        assert_eq!(
            rejected, 0,
            "every SMIRKS-backed default rule must be statically parseable by hints \
             (it accepts everything predict/enumerate does, plus more)"
        );
        assert_eq!(accepted, smirks_based);
    }

    /// Extracted-corpus companion to
    /// `parse_hint_template_default_rules_accept_reject_partition_is_stable`
    /// (and lib.rs's `reverse_smirks_validated_extracted_templates_...`
    /// audit): every one of the ~500 USPTO-derived extracted templates must
    /// be statically analyzable by `hints`, since it validates via
    /// `parse_smarts` (the correct grammar), a strict superset of what
    /// `predict`/`enumerate`'s `parse_reaction`-based check accepts.
    #[test]
    fn parse_hint_template_extracted_templates_accept_reject_partition_is_stable() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../data/templates_extracted.smi"
        );
        let rules = renkin::chem_env::load_rules_from_file(path);
        assert!(
            rules.len() > 400,
            "sanity check that the real extracted corpus loaded, got {} rules",
            rules.len()
        );
        let mut rejected = 0usize;
        for rule in &rules {
            if parse_hint_template(rule).is_err() {
                rejected += 1;
            }
        }
        assert_eq!(
            rejected, 0,
            "every extracted template must be statically parseable by hints"
        );
    }

    #[test]
    fn repeated_run_produces_byte_identical_report_json() {
        let r = rule("aryl_amination", "[c:1][N;H1,H2:2]>>[c:1][Br].[N;H1,H2:2]");
        let rules = std::slice::from_ref(&r);
        let a = generate_retrieval_hints(&["Brc1ccccc1"], rules, &default_report_config()).unwrap();
        let b = generate_retrieval_hints(&["Brc1ccccc1"], rules, &default_report_config()).unwrap();
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
    }

    #[test]
    fn reactant_input_order_converges_to_the_same_merged_hint() {
        let r = rule("aryl_amination", "[c:1][N;H1,H2:2]>>[c:1][Br].[N;H1,H2:2]");
        let a = generate_retrieval_hints(
            &["Brc1ccccc1", "NCC"],
            std::slice::from_ref(&r),
            &default_report_config(),
        )
        .unwrap();
        let b = generate_retrieval_hints(
            &["NCC", "Brc1ccccc1"],
            std::slice::from_ref(&r),
            &default_report_config(),
        )
        .unwrap();
        assert_eq!(a.hints.len(), 1);
        assert_eq!(b.hints.len(), 1);
        assert_eq!(
            a.hints[0].hint_id, b.hints[0].hint_id,
            "the same underlying assignment must converge to the same hint_id \
             regardless of which order the known reactants were supplied in"
        );
    }

    #[test]
    fn input_index_provenance_tracks_this_calls_own_argument_order() {
        let r = rule("aryl_amination", "[c:1][N;H1,H2:2]>>[c:1][Br].[N;H1,H2:2]");
        let forward = generate_retrieval_hints(
            &["Brc1ccccc1", "NCC"],
            std::slice::from_ref(&r),
            &default_report_config(),
        )
        .unwrap();
        let known_assignments = &forward.hints[0].known_assignments;
        let aryl_input_index = known_assignments
            .iter()
            .find(|ka| ka.slot_index == 0)
            .unwrap()
            .input_index;
        let amine_input_index = known_assignments
            .iter()
            .find(|ka| ka.slot_index == 1)
            .unwrap()
            .input_index;
        assert_eq!(
            aryl_input_index, 0,
            "Brc1ccccc1 was argument 0 in this call"
        );
        assert_eq!(amine_input_index, 1, "NCC was argument 1 in this call");

        let reversed =
            generate_retrieval_hints(&["NCC", "Brc1ccccc1"], &[r], &default_report_config())
                .unwrap();
        let known_assignments_reversed = &reversed.hints[0].known_assignments;
        let amine_input_index_reversed = known_assignments_reversed
            .iter()
            .find(|ka| ka.slot_index == 1)
            .unwrap()
            .input_index;
        assert_eq!(
            amine_input_index_reversed, 0,
            "NCC was argument 0 in this second call -- input_index reflects THIS call's order"
        );
    }

    #[test]
    fn duplicate_smiles_known_reactants_are_assigned_injectively_to_distinct_slots() {
        // Two rows of the identical SMILES are still two distinct known
        // reactants (distinct input_index) and must land on two distinct
        // slots, never both on the same one.
        let r = rule("symmetric_binary", "[C:1][C:2]>>[C:1][Br].[C:2][Br]");
        let report =
            generate_retrieval_hints(&["CCBr", "CCBr"], &[r], &default_report_config()).unwrap();
        assert_eq!(report.hints.len(), 1);
        let assignments = &report.hints[0].known_assignments;
        assert_eq!(assignments.len(), 2);
        let slots: BTreeSet<usize> = assignments.iter().map(|a| a.slot_index).collect();
        assert_eq!(
            slots.len(),
            2,
            "both duplicate-SMILES known reactants must occupy distinct slots, got {assignments:?}"
        );
        let input_indices: BTreeSet<usize> = assignments.iter().map(|a| a.input_index).collect();
        assert_eq!(input_indices, BTreeSet::from([0, 1]));
    }

    #[test]
    fn assignment_cap_is_deterministic_across_repeated_runs() {
        let r = rule("symmetric_binary", "[C:1][C:2]>>[C:1][Br].[C:2][Br]");
        let capped_config = HintGenerationConfig {
            max_hints: 50,
            max_matches_per_slot: 50,
            max_assignments_per_template: 1,
        };
        let a =
            generate_retrieval_hints(&["CCBr", "CBr"], std::slice::from_ref(&r), &capped_config)
                .unwrap();
        let b =
            generate_retrieval_hints(&["CCBr", "CBr"], std::slice::from_ref(&r), &capped_config)
                .unwrap();
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap(),
            "an assignment-capped run must still be byte-identical across repeats"
        );
    }

    #[test]
    fn hitting_max_assignments_per_template_is_flagged_not_silently_treated_as_no_match() {
        let r = rule("symmetric_binary", "[C:1][C:2]>>[C:1][Br].[C:2][Br]");
        let capped_config = HintGenerationConfig {
            max_hints: 50,
            max_matches_per_slot: 50,
            max_assignments_per_template: 1,
        };
        let report = generate_retrieval_hints(&["CCBr", "CBr"], &[r], &capped_config).unwrap();
        assert!(
            !report.hints.is_empty(),
            "a capped assignment enumeration must still return the assignments found \
             before the cap, never silently zero"
        );
        assert!(
            report.stats.templates_with_assignments_truncated >= 1,
            "the cap must be explicitly flagged in stats, not silently absorbed"
        );
    }

    #[test]
    fn match_sites_order_is_stable_across_repeated_runs() {
        let r = rule("halide_swap", "[c:1][Cl]>>[c:1][Br]");
        let known = "Brc1cc(Br)cc(Br)c1"; // three symmetric bromine sites
        let a =
            generate_retrieval_hints(&[known], std::slice::from_ref(&r), &default_report_config())
                .unwrap();
        let b =
            generate_retrieval_hints(&[known], std::slice::from_ref(&r), &default_report_config())
                .unwrap();
        let sites_a = &a.hints[0].known_assignments[0].match_sites;
        let sites_b = &b.hints[0].known_assignments[0].match_sites;
        assert_eq!(sites_a.len(), 3);
        for (site_a, site_b) in sites_a.iter().zip(sites_b.iter()) {
            assert_eq!(
                site_a.target_atom_indices, site_b.target_atom_indices,
                "match_sites must appear in the same order across repeated runs, \
                 not merely contain the same set"
            );
        }
    }

    #[test]
    fn max_hints_is_applied_after_merge_not_before() {
        // Two differently-named templates converge on the same retrieval
        // signature (same merged hint). With max_hints=1, this must NOT be
        // reported as capped -- there is exactly one hint after merging,
        // even though there were two hints before merging.
        let r1 = rule("halide_swap_v1", "[c:1][Cl]>>[c:1][Br]");
        let r2 = rule("halide_swap_v2", "[c:1][Cl]>>[c:1][Br]");
        let config = HintGenerationConfig {
            max_hints: 1,
            max_matches_per_slot: 50,
            max_assignments_per_template: 100,
        };
        let report = generate_retrieval_hints(&["Brc1ccccc1"], &[r1, r2], &config).unwrap();
        assert_eq!(report.stats.hints_before_merge, 2);
        assert_eq!(report.stats.duplicate_hints_merged, 1);
        assert_eq!(report.hints.len(), 1);
        assert!(
            !report.stats.hints_capped,
            "max_hints must be applied to the post-merge count, not the pre-merge count"
        );
    }

    #[test]
    fn isotope_and_charge_constraints_are_retained_in_required_features() {
        let r = rule("isotope_charge_probe", "[c:1][C:2]>>[c:1][Br].[13C;+1:2]");
        let template = parse_hint_template(&r).unwrap();
        let missing_slot = &template.lhs_slots[1];
        let features = extract_required_features(&missing_slot.query);
        assert_eq!(features.isotope, Some(13));
        assert_eq!(features.charge, Some(1));
        assert!(features.summary_complete);
    }

    /// Parses a single-atom SMARTS fragment and extracts features from its
    /// one atom, for concise feature-extraction fixture tests.
    fn features_of(atom_smarts: &str) -> HintRequiredFeatures {
        let query = smarts::parse_smarts(atom_smarts).unwrap();
        assert_eq!(
            query.atoms.len(),
            1,
            "fixture must be a single-atom SMARTS: {atom_smarts:?}"
        );
        extract_required_features(&query)
    }

    #[test]
    fn same_family_element_or_flattens_to_required_elements() {
        let f = features_of("[N,O]");
        assert_eq!(f.required_elements, vec!["N".to_string(), "O".to_string()]);
        assert!(f.summary_complete);
    }

    #[test]
    fn same_family_hcount_or_under_and_flattens_cleanly() {
        let f = features_of("[N;H1,H2]");
        assert_eq!(f.required_elements, vec!["N".to_string()]);
        assert_eq!(f.hydrogen_constraints, vec!["H1 or H2".to_string()]);
        assert!(f.summary_complete);
    }

    #[test]
    fn not_over_single_element_becomes_explicit_exclusion() {
        let f = features_of("[!#6]");
        assert!(f.required_elements.is_empty());
        assert_eq!(f.excluded_elements, vec!["C".to_string()]);
        assert!(
            f.summary_complete,
            "a clean single-element NOT is fully captured by excluded_elements"
        );
    }

    #[test]
    fn mixed_family_recursive_or_is_not_flattened() {
        // Two entirely different candidate interpretations of the atom
        // (N-with-H1 vs. negatively-charged-O) wrapped in recursive SMARTS
        // on each OR branch -- must not fabricate a combined
        // required_elements/hydrogen_constraints/charge summary.
        let f = features_of("[$([N;H1]),$([O-])]");
        assert!(f.required_elements.is_empty());
        assert!(f.excluded_elements.is_empty());
        assert!(f.hydrogen_constraints.is_empty());
        assert_eq!(f.charge, None);
        assert!(!f.summary_complete);
    }

    #[test]
    fn and_of_element_or_and_ring_constraint_flattens_cleanly() {
        // AND(OR(C, N), R0): "(C or N) and belongs to zero SSSR rings" --
        // R0 parses as RingCount(0) (empirically verified), not
        // RingMembership -- element OR is same-family (safe), combined via
        // AND with an unrelated ring-count constraint (also safe); both
        // fields can be populated together.
        let f = features_of("[C,N;R0]");
        assert_eq!(f.required_elements, vec!["C".to_string(), "N".to_string()]);
        assert_eq!(f.ring_count, Some(0));
        assert!(f.summary_complete);
    }

    #[test]
    fn aromatic_aliphatic_or_is_not_flattened_into_a_single_aromatic_value() {
        // [c,C]: aromatic-C OR aliphatic-C. Both branches agree on element
        // (C) but disagree on aromaticity -- a mixed-family OR (element +
        // aromatic families both present). Must not arbitrarily commit to
        // aromatic=true or aromatic=false, since either is a valid
        // interpretation depending on which OR-branch matches.
        let f = features_of("[c,C]");
        assert_eq!(
            f.aromatic, None,
            "aromaticity must stay unconstrained, not arbitrarily pick one OR-branch's value"
        );
        assert!(!f.summary_complete);
    }

    #[test]
    fn nested_not_plus_recursive_smarts_stays_incomplete() {
        let f = features_of("[!$(C=O)]");
        assert!(f.required_elements.is_empty());
        assert!(f.excluded_elements.is_empty());
        assert!(!f.summary_complete);
    }

    #[test]
    fn bond_or_and_any_bond_are_never_misrepresented_as_a_single_type() {
        // Any-bond (`~`) and a compound bond OR/AND/NOT must never be
        // described as one of the concrete single/double/triple/aromatic
        // labels -- that would silently narrow an ambiguous constraint.
        assert_eq!(describe_bond_order(&BondQuery::Any), "any");
        let compound = BondQuery::Or(
            Box::new(BondQuery::Primitive(BondPrimitive::Single)),
            Box::new(BondQuery::Primitive(BondPrimitive::Double)),
        );
        assert_eq!(describe_bond_order(&compound), "complex");
    }

    #[test]
    fn recursive_smarts_sets_summary_incomplete() {
        let r = rule("recursive_probe", "[c:1][C:2]>>[c:1][Br].[C;$(C=O):2]");
        let template = parse_hint_template(&r).unwrap();
        let missing_slot = &template.lhs_slots[1];
        let features = extract_required_features(&missing_slot.query);
        assert!(
            !features.summary_complete,
            "a recursive $(...) condition must not be silently summarized as complete"
        );
    }
}
