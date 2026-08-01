//! Ring-context safety guard for extracted templates (Issue #72).
//!
//! Extracted templates (`data/templates_extracted_*.smi`) carry no
//! ring-membership information at all -- confirmed by both reading
//! `rdchiral`'s own `convert_atom_to_wildcard()` source and re-running the
//! full 40,008-reaction USPTO-50k extraction (see Issue #72's posted
//! comment): the gap is `absent_in_rdchiral_output` for all 500 checked-in
//! templates, not something RENKIN's own simplification strips. So nothing
//! in a bare SMARTS match can tell, at template-application time, whether a
//! given disconnection is breaking a ring open -- a template whose training
//! occurrences were overwhelmingly non-ring can still pattern-match a ring
//! bond in an unrelated target molecule and silently produce a structurally
//! wrong precursor (`extracted_9`'s original failure).
//!
//! This module adds an opt-in, match-level filter: for each match of an
//! extracted template against a real target, look up whether historical
//! source reactions (`scripts/generate_ring_context_metadata.py`,
//! `data/ring_context_metadata_500.json`) predominantly broke that mapped
//! bond inside a ring or outside one, then check the SAME bond's real
//! ring-membership in the actual target being decomposed. A mismatch
//! rejects that one match -- never the whole template, since the same
//! template can have both a safe and an unsafe match on the same molecule.
//!
//! Filtering is match-level, not template-level, and requires chematic
//! 0.10.0's [`chematic::rxn::find_reaction_matches`]/
//! [`chematic::rxn::apply_reaction_match`] (chematic#225) to enumerate
//! matches and apply only the accepted ones.

use std::cell::RefCell;

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use chematic::core::{AtomIdx, Element};
use chematic::rxn::{ReactionMatch, apply_reaction_match, find_reaction_matches, parse_reaction};

use crate::chem_env::{
    Molecule, PrecursorMol, RetroRule, apply_retro, is_bridge_bond, split_fragments,
};
use crate::sha256_hex;

/// Whether historical source reactions broke a template's mapped bond
/// (map_a, map_b) inside a ring, outside a ring, both, or with no evidence
/// either way. Derived purely from presence/absence of observations --
/// never from whether the raw or simplified SMIRKS *string* contains a
/// ring-membership primitive, since Issue #72 established those primitives
/// are absent from rdchiral's output entirely (not a signal to read).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RingBondIntent {
    Ring,
    NonRing,
    Either,
    Unknown,
}

/// How strictly the ring-context guard enforces its findings. Governs both
/// the ring-context check and the independent element-accounting gate
/// (§ below) -- one knob, not two, so `AuditOnly`'s "observe, never filter"
/// guarantee covers both checks uniformly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RingContextPolicy {
    /// No guard. `apply_retro_with_policy` delegates directly to the
    /// unmodified `apply_retro` -- the exact legacy `run_reactants` path,
    /// untouched. No sidecar is loaded or required.
    #[default]
    Disabled,
    /// Every match is enumerated and classified (ring-context AND
    /// element-accounting), diagnostics are recorded, but the *returned*
    /// precursor sets are always identical to `Disabled`'s -- literally
    /// produced by the same `apply_retro` call, not reconstructed via
    /// find+apply and hoped to match. Byte-identical to `Disabled` by
    /// construction, not by measurement.
    AuditOnly,
    /// Matches that fail either gate are excluded before
    /// `apply_reaction_match`; the template itself is never wholly
    /// rejected.
    Conservative,
}

/// Why one match (not necessarily the whole template) was excluded under
/// `Conservative`. Ring-context and element-accounting are independent
/// gates with independent reasons, so a caller can tell which mechanism is
/// responsible for a given rejection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractedTemplateRejectReason {
    /// A changed bond's real ring-membership in the target contradicts its
    /// historical intent (`NonRing`-intent-on-a-ring-bond or
    /// `Ring`-intent-on-a-non-ring-bond).
    RingContextMismatch,
    /// A changed bond has `Unknown` intent (no historical evidence either
    /// way) and is a real ring bond in the target -- fails closed.
    UnknownRingIntentOnRingBond,
    /// The accepted match's precursor set doesn't supply enough of some
    /// heavy element the target needs (independent of ring-context).
    UnaccountedTargetElement,
    /// The sidecar loaded successfully but has no entry for this specific
    /// template (should not happen if the sidecar was generated from the
    /// same `.smi` file the loaded rules came from; possible if the two
    /// have drifted).
    MissingTopologyMetadata,
    /// A changed bond's mapped atoms didn't resolve to a real bond in the
    /// target for this specific match (e.g. the pattern matched but the
    /// two mapped atoms aren't directly bonded -- shouldn't happen for a
    /// bond that's actually part of the LHS query, but checked and failed
    /// closed rather than assumed).
    InvalidMappedBond,
    /// `find_reaction_matches`/`apply_reaction_match` returned a parse
    /// error for this template's SMIRKS.
    ReactionApplicationFailed,
}

// ── Sidecar deserialization ────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct SidecarChangedBond {
    map_a: u16,
    map_b: u16,
    intent: RingBondIntent,
}

#[derive(Debug, Deserialize)]
struct SidecarTemplate {
    #[allow(dead_code)]
    simplified_smirks: String,
    changed_bonds: Vec<SidecarChangedBond>,
}

#[derive(Debug, Deserialize)]
struct SidecarFile {
    schema_version: u32,
    template_file_sha256: String,
    templates: FxHashMap<String, SidecarTemplate>,
}

const SUPPORTED_SIDECAR_SCHEMA_VERSION: u32 = 1;

/// One template's compiled guard data: its changed-bond intents (by atom
/// map pair) plus the query-atom-index → atom-map-number table for its
/// SMIRKS LHS, parsed once so a match's real `AtomIdx` positions can be
/// resolved without re-parsing the SMIRKS per match (chematic's own
/// `ReactionMatch::atom_map_positions` would re-parse every call).
struct CompiledTemplate {
    /// (map_a, map_b) -> intent, `map_a < map_b`.
    changed_bond_intents: FxHashMap<(u16, u16), RingBondIntent>,
    /// LHS reactant-template query-atom-index -> atom-map number.
    atom_map_table: Vec<Option<u16>>,
}

/// Loaded sidecar + per-template compiled lookup data, ready for repeated
/// use across many `apply_retro_with_policy` calls (one `RingContextGuard`
/// built once per process/search run, not per call).
pub struct RingContextGuard {
    compiled: FxHashMap<String, CompiledTemplate>,
}

impl RingContextGuard {
    /// Load and validate the sidecar at `sidecar_path` against the exact
    /// `.smi` template file content the caller loaded rules from
    /// (`templates_smi_content`). Fails closed: any parse error, schema
    /// version mismatch, or `template_file_sha256` mismatch is a hard
    /// error -- never a silent fall-back to the legacy path. Per-template
    /// absence *within* an otherwise-valid sidecar is not an error here;
    /// it surfaces later as `MissingTopologyMetadata` per-rule.
    pub fn load(sidecar_path: &str, templates_smi_content: &str) -> anyhow::Result<Self> {
        let raw = std::fs::read_to_string(sidecar_path).map_err(|e| {
            anyhow::anyhow!("ring-context sidecar {sidecar_path} could not be read: {e}")
        })?;
        let sidecar: SidecarFile = serde_json::from_str(&raw).map_err(|e| {
            anyhow::anyhow!("ring-context sidecar {sidecar_path} failed to parse: {e}")
        })?;
        if sidecar.schema_version != SUPPORTED_SIDECAR_SCHEMA_VERSION {
            anyhow::bail!(
                "ring-context sidecar {sidecar_path} has schema_version {} but this build \
                 supports {SUPPORTED_SIDECAR_SCHEMA_VERSION}",
                sidecar.schema_version
            );
        }
        // Matches scripts/generate_ring_context_metadata.py's
        // `sha256_hex(open(args.templates).read())` exactly: the RAW file
        // content, no trim (unlike `template_id_for_smirks`, which trims
        // the individual SMIRKS string).
        let actual_hash = sha256_hex(Sha256::digest(templates_smi_content.as_bytes()));
        if sidecar.template_file_sha256 != actual_hash {
            anyhow::bail!(
                "ring-context sidecar {sidecar_path} was generated from a template file with \
                 sha256 {}, but the template file currently loaded hashes to {actual_hash} -- \
                 refusing to apply stale ring-context metadata to a different template set",
                sidecar.template_file_sha256
            );
        }

        let mut compiled = FxHashMap::default();
        for (template_id, tmpl) in sidecar.templates {
            let mut changed_bond_intents = FxHashMap::default();
            for cb in &tmpl.changed_bonds {
                let key = if cb.map_a < cb.map_b {
                    (cb.map_a, cb.map_b)
                } else {
                    (cb.map_b, cb.map_a)
                };
                changed_bond_intents.insert(key, cb.intent);
            }
            let atom_map_table = if changed_bond_intents.is_empty() {
                Vec::new()
            } else {
                lhs_atom_map_table(&tmpl.simplified_smirks).ok_or_else(|| {
                    anyhow::anyhow!(
                        "ring-context sidecar entry {template_id} has changed bonds but its \
                         SMIRKS LHS failed to parse: {}",
                        tmpl.simplified_smirks
                    )
                })?
            };
            compiled.insert(
                template_id,
                CompiledTemplate {
                    changed_bond_intents,
                    atom_map_table,
                },
            );
        }
        Ok(Self { compiled })
    }
}

/// Query-atom-index -> atom-map-number table for a retro-direction SMIRKS's
/// LHS (reactant/target-matching) component. Parsed once per template via
/// the same public `parse_reaction` chematic's own `find_reaction_matches`
/// uses internally, relying on the same atom-index-order correspondence
/// between a parsed `Molecule` and its derived `QueryMolecule` that
/// chematic's own `ReactionMatch::atom_map_positions` depends on.
fn lhs_atom_map_table(smirks: &str) -> Option<Vec<Option<u16>>> {
    let rxn = parse_reaction(smirks).ok()?;
    let reactant = rxn.reactants.first()?;
    Some(
        (0..reactant.atom_count())
            .map(|i| reactant.atom(AtomIdx(i as u32)).atom_map)
            .collect(),
    )
}

// ── Per-target ring-bond cache ─────────────────────────────────────────

/// Memoizes `is_bridge_bond` results for one target molecule across every
/// match of every extracted template evaluated against it within one
/// `apply_retro_with_policy` call. `is_bridge_bond` is a BFS; without this,
/// a promiscuous template with hundreds of matches would re-walk the same
/// target graph per match.
///
/// Scoped per-rule (fresh per `apply_retro_with_policy` call), not shared
/// across the many rules `candidate::raw_propose` applies to the same
/// target -- a further per-target-across-all-rules cache is a possible
/// future optimization if profiling shows this rule-scoped cache isn't
/// enough, not implemented here.
struct RingBondCache<'a> {
    mol: &'a Molecule,
    cache: RefCell<FxHashMap<(u32, u32), bool>>,
}

impl<'a> RingBondCache<'a> {
    fn new(mol: &'a Molecule) -> Self {
        Self {
            mol,
            cache: RefCell::new(FxHashMap::default()),
        }
    }

    /// `Some(true)` if the bond is a ring bond, `Some(false)` if not,
    /// `None` if `a`/`b` aren't directly bonded in this molecule.
    fn is_ring_bond(&self, a: AtomIdx, b: AtomIdx) -> Option<bool> {
        self.mol.bond_between(a, b)?;
        let key = if a.0 <= b.0 { (a.0, b.0) } else { (b.0, a.0) };
        if let Some(&v) = self.cache.borrow().get(&key) {
            return Some(v);
        }
        let v = !is_bridge_bond(self.mol, a, b);
        self.cache.borrow_mut().insert(key, v);
        Some(v)
    }
}

// ── Diagnostics ─────────────────────────────────────────────────────────

/// Structured counters for one `apply_retro_with_policy` call (accumulate
/// these across a search to characterize a policy's effect at scale --
/// see the 100-target `AuditOnly`/`Conservative` gate).
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct RingContextDiagnostics {
    pub matches_enumerated: u64,
    pub matches_ring_checked: u64,
    /// `NonRing`-intent matched against a real ring bond.
    pub ring_rejects_nonring_intent_on_ring_bond: u64,
    /// `Ring`-intent matched against a real non-ring bond.
    pub ring_rejects_ring_intent_on_nonring_bond: u64,
    pub ring_rejects_unknown_intent_on_ring_bond: u64,
    pub matches_unknown_intent: u64,
    pub matches_applied: u64,
    pub valence_filtered: u64,
    pub outcomes_element_rejected: u64,
    pub outcomes_accepted: u64,
    pub reaction_parse_calls: u64,
    pub templates_missing_metadata: u64,
    pub invalid_mapped_bond: u64,
    pub reaction_application_failed: u64,
}

impl RingContextDiagnostics {
    pub fn merge(&mut self, other: &RingContextDiagnostics) {
        self.matches_enumerated += other.matches_enumerated;
        self.matches_ring_checked += other.matches_ring_checked;
        self.ring_rejects_nonring_intent_on_ring_bond +=
            other.ring_rejects_nonring_intent_on_ring_bond;
        self.ring_rejects_ring_intent_on_nonring_bond +=
            other.ring_rejects_ring_intent_on_nonring_bond;
        self.ring_rejects_unknown_intent_on_ring_bond +=
            other.ring_rejects_unknown_intent_on_ring_bond;
        self.matches_unknown_intent += other.matches_unknown_intent;
        self.matches_applied += other.matches_applied;
        self.valence_filtered += other.valence_filtered;
        self.outcomes_element_rejected += other.outcomes_element_rejected;
        self.outcomes_accepted += other.outcomes_accepted;
        self.reaction_parse_calls += other.reaction_parse_calls;
        self.templates_missing_metadata += other.templates_missing_metadata;
        self.invalid_mapped_bond += other.invalid_mapped_bond;
        self.reaction_application_failed += other.reaction_application_failed;
    }
}

// ── Match classification ───────────────────────────────────────────────

enum MatchVerdict {
    Accept,
    Reject(ExtractedTemplateRejectReason),
}

/// Single chokepoint every reject path routes through: records the
/// bond-level or match-level counter that corresponds to `reason`. Keeping
/// this in one place (rather than incrementing ad hoc at each call site)
/// means every `ExtractedTemplateRejectReason` variant this module can
/// produce has exactly one, auditable place that turns it into a counter --
/// and is the one real (non-test) reader of `MatchVerdict::Reject`'s
/// payload, so the reason a caller gets back is never just a label nobody
/// looks at.
fn record_reject(diagnostics: &mut RingContextDiagnostics, reason: ExtractedTemplateRejectReason) {
    match reason {
        // Ring-context rejects are already counted with full directional
        // detail (which side was wrong) at the point of detection in
        // `classify_match`, below -- this arm exists so the match is
        // exhaustive and so a future caller reading `reason` off a
        // `MatchVerdict` has a documented no-op to point at, not a silent
        // gap.
        ExtractedTemplateRejectReason::RingContextMismatch
        | ExtractedTemplateRejectReason::UnknownRingIntentOnRingBond => {}
        ExtractedTemplateRejectReason::UnaccountedTargetElement => {
            diagnostics.outcomes_element_rejected += 1;
        }
        ExtractedTemplateRejectReason::MissingTopologyMetadata => {
            diagnostics.templates_missing_metadata += 1;
        }
        ExtractedTemplateRejectReason::InvalidMappedBond => {
            diagnostics.invalid_mapped_bond += 1;
        }
        ExtractedTemplateRejectReason::ReactionApplicationFailed => {
            diagnostics.reaction_application_failed += 1;
        }
    }
}

fn classify_match(
    m: &ReactionMatch,
    compiled: &CompiledTemplate,
    ring_cache: &RingBondCache<'_>,
    diagnostics: &mut RingContextDiagnostics,
) -> MatchVerdict {
    let per_reactant = match m.per_reactant.first() {
        Some(p) => p,
        None => return MatchVerdict::Reject(ExtractedTemplateRejectReason::InvalidMappedBond),
    };
    let real_idx_of =
        |query_idx: usize| -> Option<AtomIdx> { per_reactant.get(&query_idx).copied() };

    for (&(map_a, map_b), &intent) in &compiled.changed_bond_intents {
        let qidx_a = compiled
            .atom_map_table
            .iter()
            .position(|m| *m == Some(map_a));
        let qidx_b = compiled
            .atom_map_table
            .iter()
            .position(|m| *m == Some(map_b));
        let (Some(qidx_a), Some(qidx_b)) = (qidx_a, qidx_b) else {
            return MatchVerdict::Reject(ExtractedTemplateRejectReason::InvalidMappedBond);
        };
        let (Some(real_a), Some(real_b)) = (real_idx_of(qidx_a), real_idx_of(qidx_b)) else {
            return MatchVerdict::Reject(ExtractedTemplateRejectReason::InvalidMappedBond);
        };
        let Some(actual_ring) = ring_cache.is_ring_bond(real_a, real_b) else {
            return MatchVerdict::Reject(ExtractedTemplateRejectReason::InvalidMappedBond);
        };
        diagnostics.matches_ring_checked += 1;

        match (intent, actual_ring) {
            (RingBondIntent::Either, _) => {}
            (RingBondIntent::NonRing, false) | (RingBondIntent::Ring, true) => {}
            (RingBondIntent::NonRing, true) => {
                diagnostics.ring_rejects_nonring_intent_on_ring_bond += 1;
                return MatchVerdict::Reject(ExtractedTemplateRejectReason::RingContextMismatch);
            }
            (RingBondIntent::Ring, false) => {
                diagnostics.ring_rejects_ring_intent_on_nonring_bond += 1;
                return MatchVerdict::Reject(ExtractedTemplateRejectReason::RingContextMismatch);
            }
            (RingBondIntent::Unknown, true) => {
                diagnostics.ring_rejects_unknown_intent_on_ring_bond += 1;
                return MatchVerdict::Reject(
                    ExtractedTemplateRejectReason::UnknownRingIntentOnRingBond,
                );
            }
            (RingBondIntent::Unknown, false) => {
                diagnostics.matches_unknown_intent += 1;
            }
        }
    }
    MatchVerdict::Accept
}

// ── Element-accounting gate (independent of ring-context) ─────────────

/// Heavy-atom (hydrogen-excluded) per-element counts, computed directly on
/// a `Molecule` -- reuses the shape of
/// `synthesizability::element_accounting::heavy_atom_counts` (SMILES ->
/// counts) but takes a `Molecule` directly since match-time already holds
/// one, avoiding a wasteful re-serialize/re-parse round trip.
fn heavy_atom_counts(mol: &Molecule) -> FxHashMap<Element, usize> {
    let mut counts = FxHashMap::default();
    for (_, atom) in mol.atoms() {
        if atom.element != Element::H {
            *counts.entry(atom.element).or_insert(0) += 1;
        }
    }
    counts
}

/// Same direction as `element_accounting::compute_element_accounting`:
/// fails only when the target needs *more* of some heavy element than the
/// accepted precursor set supplies. Excess in precursors (leaving groups,
/// protecting groups, reagents) is never a failure. Never uses
/// `atom_economy` (Issue #79's clamp-masked metric) -- this is a separate,
/// unclamped, per-element check.
fn element_accounting_ok(target_mol: &Molecule, precursors: &[PrecursorMol]) -> bool {
    let target_counts = heavy_atom_counts(target_mol);
    let mut precursor_counts: FxHashMap<Element, usize> = FxHashMap::default();
    for p in precursors {
        for (element, n) in heavy_atom_counts(&p.mol) {
            *precursor_counts.entry(element).or_insert(0) += n;
        }
    }
    target_counts
        .iter()
        .all(|(element, n)| *n <= precursor_counts.get(element).copied().unwrap_or(0))
}

/// Bundles the ring-context policy + guard for threading through
/// `candidate::raw_propose` without changing its signature on every future
/// addition. `Default` (`Disabled`/`None`) reproduces pre-existing search
/// behaviour exactly -- the only caller that needs anything else is
/// `search::find_routes`, driven by `SearchConfig::ring_context_policy`/
/// `SearchConfig::ring_context_guard`.
#[derive(Clone, Copy, Default)]
pub struct RingContextArgs<'a> {
    pub policy: RingContextPolicy,
    pub guard: Option<&'a RingContextGuard>,
}

// ── Public entry point ─────────────────────────────────────────────────

/// Sibling to [`apply_retro`] that additionally gates extracted templates
/// through the ring-context and element-accounting checks according to
/// `policy`. At [`RingContextPolicy::Disabled`] (or for any non-extracted
/// rule, or when `guard` is `None`), delegates directly to the untouched
/// [`apply_retro`] -- the exact legacy path, not a reimplementation.
pub fn apply_retro_with_policy(
    mol: &Molecule,
    rule: &RetroRule,
    policy: RingContextPolicy,
    guard: Option<&RingContextGuard>,
    diagnostics: &mut RingContextDiagnostics,
) -> Vec<Vec<PrecursorMol>> {
    let Some(guard) = guard else {
        return apply_retro(mol, rule);
    };
    if policy == RingContextPolicy::Disabled || !crate::search::is_extracted_template(&rule.name) {
        return apply_retro(mol, rule);
    }

    let Some(compiled) = guard.compiled.get(&rule.template_id) else {
        diagnostics.templates_missing_metadata += 1;
        return match policy {
            RingContextPolicy::AuditOnly => apply_retro(mol, rule),
            _ => vec![],
        };
    };

    if compiled.changed_bond_intents.is_empty() {
        // No changed bonds -- ring-context is inapplicable (e.g. pure
        // functional-group interconversion). Element-accounting still
        // applies below via the shared match-application loop.
    }

    match policy {
        RingContextPolicy::Disabled => unreachable!(),
        RingContextPolicy::AuditOnly => {
            run_diagnostics_pass(mol, rule, compiled, diagnostics);
            apply_retro(mol, rule)
        }
        RingContextPolicy::Conservative => run_conservative_pass(mol, rule, compiled, diagnostics),
    }
}

/// Enumerates and classifies every match purely for `diagnostics`; the
/// return value is discarded by the caller (`AuditOnly` always returns
/// `apply_retro(mol, rule)`'s own result instead), so this can never affect
/// what's returned to the search.
fn run_diagnostics_pass(
    mol: &Molecule,
    rule: &RetroRule,
    compiled: &CompiledTemplate,
    diagnostics: &mut RingContextDiagnostics,
) {
    diagnostics.reaction_parse_calls += 1;
    let matches = match find_reaction_matches(&rule.smirks, &[mol]) {
        Ok(m) => m,
        Err(_) => {
            diagnostics.reaction_application_failed += 1;
            return;
        }
    };
    diagnostics.matches_enumerated += matches.len() as u64;
    let ring_cache = RingBondCache::new(mol);
    for m in &matches {
        let verdict = classify_match(m, compiled, &ring_cache, diagnostics);
        // Element-accounting is diagnosed too, on the actual applied
        // outcome, so AuditOnly's counters reflect what Conservative would
        // reject -- but never filters here.
        diagnostics.reaction_parse_calls += 1;
        if let Ok(Some(products)) = apply_reaction_match(&rule.smirks, &[mol], m, true) {
            let precs: Vec<PrecursorMol> = products.iter().flat_map(split_fragments).collect();
            if !element_accounting_ok(mol, &precs) {
                diagnostics.outcomes_element_rejected += 1;
            } else {
                diagnostics.outcomes_accepted += 1;
            }
        } else {
            diagnostics.valence_filtered += 1;
        }
        match verdict {
            MatchVerdict::Accept => diagnostics.matches_applied += 1,
            MatchVerdict::Reject(reason) => record_reject(diagnostics, reason),
        }
    }
}

/// Builds the real, filtered `Vec<Vec<PrecursorMol>>` for `Conservative`:
/// enumerate matches, keep only those passing ring-context, apply each via
/// `apply_reaction_match`, then keep only outcomes passing
/// element-accounting. Accepted-match order follows `find_reaction_matches`'s
/// own returned order.
fn run_conservative_pass(
    mol: &Molecule,
    rule: &RetroRule,
    compiled: &CompiledTemplate,
    diagnostics: &mut RingContextDiagnostics,
) -> Vec<Vec<PrecursorMol>> {
    diagnostics.reaction_parse_calls += 1;
    let matches = match find_reaction_matches(&rule.smirks, &[mol]) {
        Ok(m) => m,
        Err(_) => {
            diagnostics.reaction_application_failed += 1;
            return vec![];
        }
    };
    diagnostics.matches_enumerated += matches.len() as u64;
    let ring_cache = RingBondCache::new(mol);

    let mut outcomes = Vec::new();
    for m in &matches {
        match classify_match(m, compiled, &ring_cache, diagnostics) {
            MatchVerdict::Accept => {}
            MatchVerdict::Reject(reason) => {
                record_reject(diagnostics, reason);
                continue;
            }
        }
        diagnostics.matches_applied += 1;
        diagnostics.reaction_parse_calls += 1;
        match apply_reaction_match(&rule.smirks, &[mol], m, true) {
            Ok(Some(products)) => {
                let precs: Vec<PrecursorMol> = products.iter().flat_map(split_fragments).collect();
                if element_accounting_ok(mol, &precs) {
                    diagnostics.outcomes_accepted += 1;
                    outcomes.push(precs);
                } else {
                    diagnostics.outcomes_element_rejected += 1;
                }
            }
            Ok(None) => diagnostics.valence_filtered += 1,
            Err(_) => diagnostics.reaction_application_failed += 1,
        }
    }
    outcomes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chem_env::{load_rules_from_file, mol_from_smiles, template_id_for_smirks};

    const EXTRACTED_9_SMIRKS: &str =
        "[C:4]-[N:5](-[C:1](=[O:2])-[c:3])-[C:6]>>O-[C:1](=[O:2])-[c:3].[C:4]-[NH:5]-[C:6]";

    fn extracted_9_rule() -> RetroRule {
        RetroRule {
            name: "extracted_9".to_string(),
            template_id: template_id_for_smirks(EXTRACTED_9_SMIRKS),
            smirks: EXTRACTED_9_SMIRKS.to_string(),
            weight: 1.0,
            required_elements: 0,
        }
    }

    /// Sidecar with exactly one template (extracted_9), NonRing intent on
    /// (map 1, map 5) -- matching the real generated corpus's actual
    /// classification (235 non-ring observations, 0 ring observations).
    fn nonring_sidecar_json(template_file_sha256: &str) -> String {
        format!(
            r#"{{
                "schema_version": 1,
                "template_file_sha256": "{template_file_sha256}",
                "templates": {{
                    "{tid}": {{
                        "simplified_smirks": "{smirks}",
                        "changed_bonds": [
                            {{"map_a": 1, "map_b": 5, "operation": "delete", "intent": "non_ring",
                              "ring_observations": 0, "non_ring_observations": 235, "unknown_observations": 0}}
                        ]
                    }}
                }}
            }}"#,
            tid = template_id_for_smirks(EXTRACTED_9_SMIRKS),
            smirks = EXTRACTED_9_SMIRKS,
        )
    }

    fn templates_smi_fixture() -> String {
        format!("{EXTRACTED_9_SMIRKS}\t231\n")
    }

    /// Monotonic counter guaranteeing every test's temp sidecar file gets a
    /// unique path -- tests run in parallel by default, and several tests
    /// here call this helper with byte-identical sidecar content (same
    /// length), so keying the path on content/length alone collided under
    /// `cargo test`'s thread pool (one test's write racing another's read).
    fn next_temp_id() -> u64 {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    fn load_guard_with_intent(intent_json: &str, templates_smi: &str) -> RingContextGuard {
        let digest = Sha256::digest(templates_smi.as_bytes());
        let hash = sha256_hex(digest);
        let sidecar_json = intent_json.replace("__HASH__", &hash);
        let dir = std::env::temp_dir().join(format!(
            "renkin_ring_context_test_{}_{}",
            std::process::id(),
            next_temp_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sidecar.json");
        std::fs::write(&path, sidecar_json).unwrap();
        RingContextGuard::load(path.to_str().unwrap(), templates_smi).unwrap()
    }

    // ── Guard loading: fail-closed ─────────────────────────────────────

    #[test]
    fn guard_load_rejects_hash_mismatch() {
        let smi = templates_smi_fixture();
        let sidecar = nonring_sidecar_json(
            "0000000000000000000000000000000000000000000000000000000000000000",
        );
        let dir = std::env::temp_dir().join(format!(
            "renkin_ring_context_hashfail_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sidecar.json");
        std::fs::write(&path, sidecar).unwrap();
        let result = RingContextGuard::load(path.to_str().unwrap(), &smi);
        assert!(
            result.is_err(),
            "sidecar with wrong template_file_sha256 must fail closed"
        );
    }

    #[test]
    fn guard_load_rejects_unsupported_schema_version() {
        let smi = templates_smi_fixture();
        let digest = Sha256::digest(smi.as_bytes());
        let hash = sha256_hex(digest);
        let sidecar = format!(
            r#"{{"schema_version": 99, "template_file_sha256": "{hash}", "templates": {{}}}}"#
        );
        let dir = std::env::temp_dir().join(format!(
            "renkin_ring_context_schemafail_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sidecar.json");
        std::fs::write(&path, sidecar).unwrap();
        let result = RingContextGuard::load(path.to_str().unwrap(), &smi);
        assert!(
            result.is_err(),
            "unsupported schema_version must fail closed"
        );
    }

    #[test]
    fn guard_load_rejects_missing_file() {
        let result = RingContextGuard::load("/nonexistent/path/sidecar.json", "irrelevant");
        assert!(result.is_err());
    }

    #[test]
    fn guard_load_succeeds_on_matching_hash() {
        let smi = templates_smi_fixture();
        let sidecar = nonring_sidecar_json("__HASH__");
        let guard = load_guard_with_intent(&sidecar, &smi);
        assert!(
            guard
                .compiled
                .contains_key(&template_id_for_smirks(EXTRACTED_9_SMIRKS))
        );
    }

    // ── classify_match: all RingBondIntent x actual-ring combinations ──

    fn compiled_with_intent(intent: RingBondIntent) -> CompiledTemplate {
        let mut changed_bond_intents = FxHashMap::default();
        changed_bond_intents.insert((1u16, 5u16), intent);
        let atom_map_table = lhs_atom_map_table(EXTRACTED_9_SMIRKS).unwrap();
        CompiledTemplate {
            changed_bond_intents,
            atom_map_table,
        }
    }

    /// Runs `find_reaction_matches` for extracted_9 against `target_smiles`
    /// and returns the first match, panicking if there is none -- these
    /// fixtures are constructed to match exactly once.
    fn single_match(target_smiles: &str) -> (Molecule, ReactionMatch) {
        let mol = mol_from_smiles(target_smiles).unwrap();
        let matches = find_reaction_matches(EXTRACTED_9_SMIRKS, &[&mol]).unwrap();
        assert_eq!(
            matches.len(),
            1,
            "fixture must match extracted_9 exactly once: {target_smiles}"
        );
        let m = matches.into_iter().next().unwrap();
        (mol, m)
    }

    /// N-methylisoindolin-1-one: the amide N-C(=O) bond IS a ring bond
    /// (5-membered lactam fused to benzene). Real regression case for
    /// Issue #72 / extracted_9: L984's actual failure disconnected an
    /// isoindolinone's ring N-C(=O) bond because extracted_9's training
    /// data (235 non-ring observations, 0 ring) never saw this bond as a
    /// ring bond -- verified via RDKit that atoms (1, 2) = (carbonyl C, N)
    /// is IsInRing()==True in this exact molecule.
    const ISOINDOLINONE_RING_CASE: &str = "O=C1N(C)Cc2ccccc21";

    /// N-methylacetanilide-shaped acyclic case: N bonded to a carbonyl
    /// (aroyl), a methyl, and an acyclic ethyl -- the disconnected bond is
    /// not part of any ring.
    const ACYCLIC_NONRING_CASE: &str = "CCN(C)C(=O)c1ccccc1";

    #[test]
    fn classify_match_nonring_intent_on_ring_bond_rejects() {
        let (mol, m) = single_match(ISOINDOLINONE_RING_CASE);
        let compiled = compiled_with_intent(RingBondIntent::NonRing);
        let cache = RingBondCache::new(&mol);
        let mut diag = RingContextDiagnostics::default();
        let verdict = classify_match(&m, &compiled, &cache, &mut diag);
        assert!(matches!(
            verdict,
            MatchVerdict::Reject(ExtractedTemplateRejectReason::RingContextMismatch)
        ));
        assert_eq!(diag.ring_rejects_nonring_intent_on_ring_bond, 1);
    }

    #[test]
    fn classify_match_ring_intent_on_nonring_bond_rejects() {
        let (mol, m) = single_match(ACYCLIC_NONRING_CASE);
        let compiled = compiled_with_intent(RingBondIntent::Ring);
        let cache = RingBondCache::new(&mol);
        let mut diag = RingContextDiagnostics::default();
        let verdict = classify_match(&m, &compiled, &cache, &mut diag);
        assert!(matches!(
            verdict,
            MatchVerdict::Reject(ExtractedTemplateRejectReason::RingContextMismatch)
        ));
        assert_eq!(diag.ring_rejects_ring_intent_on_nonring_bond, 1);
    }

    #[test]
    fn classify_match_nonring_intent_on_nonring_bond_accepts() {
        let (mol, m) = single_match(ACYCLIC_NONRING_CASE);
        let compiled = compiled_with_intent(RingBondIntent::NonRing);
        let cache = RingBondCache::new(&mol);
        let mut diag = RingContextDiagnostics::default();
        let verdict = classify_match(&m, &compiled, &cache, &mut diag);
        assert!(matches!(verdict, MatchVerdict::Accept));
    }

    #[test]
    fn classify_match_ring_intent_on_ring_bond_accepts() {
        let (mol, m) = single_match(ISOINDOLINONE_RING_CASE);
        let compiled = compiled_with_intent(RingBondIntent::Ring);
        let cache = RingBondCache::new(&mol);
        let mut diag = RingContextDiagnostics::default();
        let verdict = classify_match(&m, &compiled, &cache, &mut diag);
        assert!(matches!(verdict, MatchVerdict::Accept));
    }

    #[test]
    fn classify_match_either_intent_allows_ring_and_nonring() {
        let compiled = compiled_with_intent(RingBondIntent::Either);
        for target in [ISOINDOLINONE_RING_CASE, ACYCLIC_NONRING_CASE] {
            let (mol, m) = single_match(target);
            let cache = RingBondCache::new(&mol);
            let mut diag = RingContextDiagnostics::default();
            let verdict = classify_match(&m, &compiled, &cache, &mut diag);
            assert!(
                matches!(verdict, MatchVerdict::Accept),
                "Either must allow {target}"
            );
        }
    }

    #[test]
    fn classify_match_unknown_intent_on_ring_bond_rejects_fail_closed() {
        let (mol, m) = single_match(ISOINDOLINONE_RING_CASE);
        let compiled = compiled_with_intent(RingBondIntent::Unknown);
        let cache = RingBondCache::new(&mol);
        let mut diag = RingContextDiagnostics::default();
        let verdict = classify_match(&m, &compiled, &cache, &mut diag);
        assert!(matches!(
            verdict,
            MatchVerdict::Reject(ExtractedTemplateRejectReason::UnknownRingIntentOnRingBond)
        ));
        assert_eq!(diag.ring_rejects_unknown_intent_on_ring_bond, 1);
    }

    #[test]
    fn classify_match_unknown_intent_on_nonring_bond_allows_with_diagnostic() {
        let (mol, m) = single_match(ACYCLIC_NONRING_CASE);
        let compiled = compiled_with_intent(RingBondIntent::Unknown);
        let cache = RingBondCache::new(&mol);
        let mut diag = RingContextDiagnostics::default();
        let verdict = classify_match(&m, &compiled, &cache, &mut diag);
        assert!(matches!(verdict, MatchVerdict::Accept));
        assert_eq!(diag.matches_unknown_intent, 1);
    }

    // ── extracted_9 / Issue #72 end-to-end regression ──────────────────

    #[test]
    fn extracted_9_conservative_rejects_isoindolinone_ring_opening() {
        let rule = extracted_9_rule();
        let mol = mol_from_smiles(ISOINDOLINONE_RING_CASE).unwrap();
        let smi = format!("{EXTRACTED_9_SMIRKS}\t231\n");
        let guard = load_guard_with_intent(&nonring_sidecar_json("__HASH__"), &smi);
        let mut diag = RingContextDiagnostics::default();

        let legacy = apply_retro(&mol, &rule);
        assert!(
            !legacy.is_empty(),
            "legacy path must still misapply extracted_9 here (that's the bug)"
        );

        let conservative = apply_retro_with_policy(
            &mol,
            &rule,
            RingContextPolicy::Conservative,
            Some(&guard),
            &mut diag,
        );
        assert!(
            conservative.is_empty(),
            "Conservative must reject the ring-opening match extracted_9's training data never saw"
        );
        assert_eq!(diag.ring_rejects_nonring_intent_on_ring_bond, 1);
    }

    #[test]
    fn extracted_9_conservative_still_allows_genuine_acyclic_case() {
        let rule = extracted_9_rule();
        let mol = mol_from_smiles(ACYCLIC_NONRING_CASE).unwrap();
        let smi = format!("{EXTRACTED_9_SMIRKS}\t231\n");
        let guard = load_guard_with_intent(&nonring_sidecar_json("__HASH__"), &smi);
        let mut diag = RingContextDiagnostics::default();

        let legacy = apply_retro(&mol, &rule);
        let conservative = apply_retro_with_policy(
            &mol,
            &rule,
            RingContextPolicy::Conservative,
            Some(&guard),
            &mut diag,
        );
        assert_eq!(
            legacy.len(),
            conservative.len(),
            "the genuine (training-consistent) acyclic case must still be produced under Conservative"
        );
    }

    // ── Policy semantics: Disabled / AuditOnly must reproduce legacy ───

    #[test]
    fn disabled_policy_is_byte_identical_to_apply_retro() {
        let rule = extracted_9_rule();
        for target in [ISOINDOLINONE_RING_CASE, ACYCLIC_NONRING_CASE] {
            let mol = mol_from_smiles(target).unwrap();
            let legacy = apply_retro(&mol, &rule);
            let mut diag = RingContextDiagnostics::default();
            let disabled =
                apply_retro_with_policy(&mol, &rule, RingContextPolicy::Disabled, None, &mut diag);
            let legacy_smiles: Vec<Vec<String>> = legacy
                .iter()
                .map(|precs| precs.iter().map(|p| p.smiles.clone()).collect())
                .collect();
            let disabled_smiles: Vec<Vec<String>> = disabled
                .iter()
                .map(|precs| precs.iter().map(|p| p.smiles.clone()).collect())
                .collect();
            assert_eq!(legacy_smiles, disabled_smiles);
            assert_eq!(
                diag.matches_enumerated, 0,
                "Disabled must never enumerate matches"
            );
        }
    }

    #[test]
    fn no_guard_falls_back_to_disabled_even_if_policy_requests_more() {
        let rule = extracted_9_rule();
        let mol = mol_from_smiles(ISOINDOLINONE_RING_CASE).unwrap();
        let legacy = apply_retro(&mol, &rule);
        let mut diag = RingContextDiagnostics::default();
        let result = apply_retro_with_policy(
            &mol,
            &rule,
            RingContextPolicy::Conservative,
            None,
            &mut diag,
        );
        let legacy_smiles: Vec<Vec<String>> = legacy
            .iter()
            .map(|precs| precs.iter().map(|p| p.smiles.clone()).collect())
            .collect();
        let result_smiles: Vec<Vec<String>> = result
            .iter()
            .map(|precs| precs.iter().map(|p| p.smiles.clone()).collect())
            .collect();
        assert_eq!(
            legacy_smiles, result_smiles,
            "guard: None must behave exactly like Disabled"
        );
    }

    #[test]
    fn auditonly_returns_legacy_output_even_though_isoindolinone_match_is_unsafe() {
        let rule = extracted_9_rule();
        let mol = mol_from_smiles(ISOINDOLINONE_RING_CASE).unwrap();
        let smi = format!("{EXTRACTED_9_SMIRKS}\t231\n");
        let guard = load_guard_with_intent(&nonring_sidecar_json("__HASH__"), &smi);
        let mut diag = RingContextDiagnostics::default();

        let legacy = apply_retro(&mol, &rule);
        let audit = apply_retro_with_policy(
            &mol,
            &rule,
            RingContextPolicy::AuditOnly,
            Some(&guard),
            &mut diag,
        );

        let legacy_smiles: Vec<Vec<String>> = legacy
            .iter()
            .map(|precs| precs.iter().map(|p| p.smiles.clone()).collect())
            .collect();
        let audit_smiles: Vec<Vec<String>> = audit
            .iter()
            .map(|precs| precs.iter().map(|p| p.smiles.clone()).collect())
            .collect();
        assert_eq!(
            legacy_smiles, audit_smiles,
            "AuditOnly must be byte-identical to legacy by construction"
        );
        assert_eq!(
            diag.ring_rejects_nonring_intent_on_ring_bond, 1,
            "AuditOnly must still record what Conservative would have rejected"
        );
    }

    #[test]
    fn handcrafted_rules_are_never_gated_regardless_of_policy() {
        let rule = crate::chem_env::default_rules()
            .into_iter()
            .find(|r| r.name == "amide_cleavage")
            .expect("amide_cleavage must exist in default_rules");
        let mol = mol_from_smiles("CC(=O)Nc1ccccc1").unwrap();
        let legacy = apply_retro(&mol, &rule);
        let mut diag = RingContextDiagnostics::default();
        // No guard loaded at all -- if this rule were mistakenly routed
        // through the extracted-template path it would panic/differ.
        let gated = apply_retro_with_policy(
            &mol,
            &rule,
            RingContextPolicy::Conservative,
            None,
            &mut diag,
        );
        let legacy_smiles: Vec<Vec<String>> = legacy
            .iter()
            .map(|precs| precs.iter().map(|p| p.smiles.clone()).collect())
            .collect();
        let gated_smiles: Vec<Vec<String>> = gated
            .iter()
            .map(|precs| precs.iter().map(|p| p.smiles.clone()).collect())
            .collect();
        assert_eq!(legacy_smiles, gated_smiles);
        assert_eq!(diag.matches_enumerated, 0);
    }

    #[test]
    fn missing_topology_metadata_conservative_rejects_fail_closed() {
        // Sidecar valid and hash-matching, but with ZERO templates -- so
        // extracted_9 is absent from an otherwise-valid sidecar.
        let smi = format!("{EXTRACTED_9_SMIRKS}\t231\n");
        let digest = Sha256::digest(smi.as_bytes());
        let hash = sha256_hex(digest);
        let sidecar = format!(
            r#"{{"schema_version": 1, "template_file_sha256": "{hash}", "templates": {{}}}}"#
        );
        let guard = load_guard_with_intent(&sidecar, &smi);

        let rule = extracted_9_rule();
        let mol = mol_from_smiles(ACYCLIC_NONRING_CASE).unwrap();
        let mut diag = RingContextDiagnostics::default();
        let result = apply_retro_with_policy(
            &mol,
            &rule,
            RingContextPolicy::Conservative,
            Some(&guard),
            &mut diag,
        );
        assert!(
            result.is_empty(),
            "missing per-template metadata must fail closed under Conservative"
        );
        assert_eq!(diag.templates_missing_metadata, 1);
    }

    #[test]
    fn missing_topology_metadata_auditonly_still_returns_legacy() {
        let smi = format!("{EXTRACTED_9_SMIRKS}\t231\n");
        let digest = Sha256::digest(smi.as_bytes());
        let hash = sha256_hex(digest);
        let sidecar = format!(
            r#"{{"schema_version": 1, "template_file_sha256": "{hash}", "templates": {{}}}}"#
        );
        let guard = load_guard_with_intent(&sidecar, &smi);

        let rule = extracted_9_rule();
        let mol = mol_from_smiles(ACYCLIC_NONRING_CASE).unwrap();
        let legacy = apply_retro(&mol, &rule);
        let mut diag = RingContextDiagnostics::default();
        let result = apply_retro_with_policy(
            &mol,
            &rule,
            RingContextPolicy::AuditOnly,
            Some(&guard),
            &mut diag,
        );
        let legacy_smiles: Vec<Vec<String>> = legacy
            .iter()
            .map(|precs| precs.iter().map(|p| p.smiles.clone()).collect())
            .collect();
        let result_smiles: Vec<Vec<String>> = result
            .iter()
            .map(|precs| precs.iter().map(|p| p.smiles.clone()).collect())
            .collect();
        assert_eq!(legacy_smiles, result_smiles);
        assert_eq!(diag.templates_missing_metadata, 1);
    }

    // ── Element-accounting gate ─────────────────────────────────────────

    #[test]
    fn element_accounting_ok_when_precursors_cover_target() {
        let target = mol_from_smiles("CC(=O)Nc1ccccc1").unwrap();
        let precs = vec![
            crate::chem_env::PrecursorMol {
                smiles: "CC(=O)O".to_string(),
                mol: mol_from_smiles("CC(=O)O").unwrap(),
            },
            crate::chem_env::PrecursorMol {
                smiles: "Nc1ccccc1".to_string(),
                mol: mol_from_smiles("Nc1ccccc1").unwrap(),
            },
        ];
        assert!(element_accounting_ok(&target, &precs));
    }

    #[test]
    fn element_accounting_allows_precursor_excess() {
        let target = mol_from_smiles("CC(=O)O").unwrap();
        // Precursors bring far more carbon than the target needs -- allowed.
        let precs = vec![crate::chem_env::PrecursorMol {
            smiles: "CCCCCCCC(=O)O".to_string(),
            mol: mol_from_smiles("CCCCCCCC(=O)O").unwrap(),
        }];
        assert!(element_accounting_ok(&target, &precs));
    }

    #[test]
    fn element_accounting_rejects_target_atom_loss() {
        let target = mol_from_smiles("c1ccc2c(c1)C(=O)N(C)C2")
            .or_else(|_| mol_from_smiles(ISOINDOLINONE_RING_CASE))
            .unwrap();
        // Precursor set missing the whole aromatic ring the target needs.
        let precs = vec![crate::chem_env::PrecursorMol {
            smiles: "CN".to_string(),
            mol: mol_from_smiles("CN").unwrap(),
        }];
        assert!(!element_accounting_ok(&target, &precs));
    }

    #[test]
    fn diagnostics_merge_sums_all_fields() {
        let mut a = RingContextDiagnostics {
            matches_enumerated: 1,
            outcomes_accepted: 2,
            ..Default::default()
        };
        let b = RingContextDiagnostics {
            matches_enumerated: 3,
            outcomes_accepted: 4,
            ..Default::default()
        };
        a.merge(&b);
        assert_eq!(a.matches_enumerated, 4);
        assert_eq!(a.outcomes_accepted, 6);
    }

    #[test]
    fn ring_context_args_default_is_disabled_and_none() {
        let args = RingContextArgs::default();
        assert_eq!(args.policy, RingContextPolicy::Disabled);
        assert!(args.guard.is_none());
    }

    /// Sanity check that the real checked-in extracted-500 corpus loads
    /// via `load_rules_from_file` cleanly enough to build a `RetroRule`
    /// list containing `extracted_9` under its expected name -- a
    /// regression guard for the test fixtures above staying in sync with
    /// the real template file's line position.
    #[test]
    fn extracted_9_name_matches_checked_in_corpus_position() {
        let rules = load_rules_from_file("data/templates_extracted_500.smi");
        let rule9 = rules.iter().find(|r| r.name == "extracted_9");
        if let Some(rule9) = rule9 {
            assert_eq!(
                rule9.smirks, EXTRACTED_9_SMIRKS,
                "extracted_9's real position in the checked-in corpus no longer matches this test's fixture SMIRKS -- update EXTRACTED_9_SMIRKS"
            );
        }
    }
}
