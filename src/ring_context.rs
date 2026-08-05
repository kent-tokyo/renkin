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

use rustc_hash::{FxHashMap, FxHashSet};
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

/// Whether one axis of the safety guard (ring-context, or element-accounting)
/// actually filters, or merely classifies and diagnoses. The two axes are
/// independent: a caller can enforce ring-context while only diagnosing
/// element-accounting (or vice versa) to measure each gate's individual
/// contribution, without disabling the other's classification/diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Enforcement {
    /// Classify and diagnose, but never exclude a match/outcome on this
    /// axis's account.
    AuditOnly,
    /// Actually filter on this axis's verdict.
    Enforce,
}

/// The two independent safety axes for extracted-template application.
/// `Conservative` (both `Enforce`) is the real safety policy; `AuditOnly`
/// (both `AuditOnly`) is diagnose-only; `RingOnly`/`ElementOnly` (one axis
/// `Enforce`, the other `AuditOnly`) isolate each gate's individual
/// contribution for ablation measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtractedTemplateSafetyPolicy {
    pub ring_context: Enforcement,
    pub element_accounting: Enforcement,
}

impl ExtractedTemplateSafetyPolicy {
    pub const AUDIT_ONLY: Self = Self {
        ring_context: Enforcement::AuditOnly,
        element_accounting: Enforcement::AuditOnly,
    };
    pub const CONSERVATIVE: Self = Self {
        ring_context: Enforcement::Enforce,
        element_accounting: Enforcement::Enforce,
    };
    pub const RING_ONLY: Self = Self {
        ring_context: Enforcement::Enforce,
        element_accounting: Enforcement::AuditOnly,
    };
    pub const ELEMENT_ONLY: Self = Self {
        ring_context: Enforcement::AuditOnly,
        element_accounting: Enforcement::Enforce,
    };
}

/// Ring-context guard configuration. `Disabled` carries no guard at all and
/// reproduces pre-existing behaviour exactly (delegates straight to
/// `apply_retro`); `Guarded` always carries a loaded
/// [`RingContextGuard`] alongside its policy, so "enforce/audit without a
/// guard" is not a state this type can represent -- unlike the previous
/// `(RingContextPolicy, Option<&RingContextGuard>)` pair, which allowed
/// exactly that combination and silently fell back to legacy behaviour for
/// it.
#[derive(Clone, Default)]
pub enum RingContextConfig {
    #[default]
    Disabled,
    Guarded {
        guard: std::sync::Arc<RingContextGuard>,
        policy: ExtractedTemplateSafetyPolicy,
    },
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
    ring_observations: u64,
    non_ring_observations: u64,
    ambiguous_observations: u64,
    unknown_observations: u64,
}

#[derive(Debug, Deserialize)]
struct SidecarTemplate {
    simplified_smirks: String,
    changed_bonds: Vec<SidecarChangedBond>,
}

#[derive(Debug, Deserialize)]
struct SidecarFile {
    schema_version: u32,
    template_file_sha256: String,
    templates: FxHashMap<String, SidecarTemplate>,
}

const SUPPORTED_SIDECAR_SCHEMA_VERSION: u32 = 2;

/// Recomputes the `RingBondIntent` a changed bond's own observation counts
/// imply, mirroring `generate_ring_context_metadata.py`'s `classify_intent`
/// exactly (an ambiguous single-occurrence disagreement folds into
/// `Either`, same as ring/non-ring disagreeing across occurrences). Used at
/// load time to catch a sidecar whose declared `intent` doesn't match its
/// own declared counts (hand-edited, corrupted, or generated by a diverged
/// script) -- `unknown_observations` never affects the result, exactly as
/// in the generator.
fn recompute_intent(cb: &SidecarChangedBond) -> RingBondIntent {
    if cb.ambiguous_observations > 0 || (cb.ring_observations > 0 && cb.non_ring_observations > 0) {
        RingBondIntent::Either
    } else if cb.ring_observations > 0 {
        RingBondIntent::Ring
    } else if cb.non_ring_observations > 0 {
        RingBondIntent::NonRing
    } else {
        RingBondIntent::Unknown
    }
}

/// SMIRKS strings from a raw `.smi` file's content (tab-separated
/// `SMIRKS<TAB>count` lines, `#`-comments and blank lines skipped) --
/// mirrors `chem_env::load_rules_from_file`'s line convention, but only
/// extracts the SMIRKS column (no weight/validation), since the sidecar
/// loader only needs the expected template-id set here.
fn parse_smirks_lines(content: &str) -> Vec<String> {
    content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|line| line.split('\t').next().unwrap_or(line).trim().to_string())
        .collect()
}

/// (map_a, map_b) pairs (a < b) for every bond between two mapped atoms in
/// an already-parsed `chematic` molecule -- the Rust-side counterpart of
/// `generate_ring_context_metadata.py`'s `mapped_bonds`/`real_mapped_bonds`,
/// used here to independently re-derive a template's changed bonds from its
/// own `simplified_smirks` at load time (never trusting the sidecar's
/// declared `changed_bonds` list without cross-checking it).
fn mapped_bond_pairs(mol: &chematic::core::Molecule) -> std::collections::HashSet<(u16, u16)> {
    let mut pairs = std::collections::HashSet::new();
    for (_, bond) in mol.bonds() {
        let a = mol.atom(bond.atom1).atom_map;
        let b = mol.atom(bond.atom2).atom_map;
        if let (Some(a), Some(b)) = (a, b) {
            pairs.insert(if a < b { (a, b) } else { (b, a) });
        }
    }
    pairs
}

/// Bonds present on the LHS (target-matching) side of `smirks` but absent
/// from every RHS (precursor) component -- the retro disconnection point,
/// re-derived directly from `chematic::rxn::parse_reaction` independently
/// of anything the sidecar declares. `None` if `parse_reaction` can't parse
/// this SMIRKS at all (the same pre-existing, unrelated gap
/// `RingContextGuard::load`'s per-template metadata already degrades
/// gracefully for elsewhere) -- callers must treat that as "can't
/// cross-check", not "no changed bonds".
fn recompute_changed_bonds(smirks: &str) -> Option<std::collections::HashSet<(u16, u16)>> {
    let rxn = parse_reaction(smirks).ok()?;
    let lhs = rxn.reactants.first()?;
    let lhs_bonds = mapped_bond_pairs(lhs);
    let mut rhs_bonds = std::collections::HashSet::new();
    for product in &rxn.products {
        rhs_bonds.extend(mapped_bond_pairs(product));
    }
    Some(lhs_bonds.difference(&rhs_bonds).copied().collect())
}

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

        // Every checked-in template must have exactly one sidecar entry --
        // no missing coverage, no stale extras left over from a template
        // set the sidecar was generated against before templates were
        // added/removed. `template_file_sha256` matching only proves the
        // *file bytes* match; it says nothing about whether the sidecar's
        // `templates` map actually covers every line in it.
        let expected_ids: std::collections::HashSet<String> =
            parse_smirks_lines(templates_smi_content)
                .iter()
                .map(|s| crate::chem_env::template_id_for_smirks(s))
                .collect();
        let actual_ids: std::collections::HashSet<String> =
            sidecar.templates.keys().cloned().collect();
        let missing: Vec<&String> = expected_ids.difference(&actual_ids).collect();
        if !missing.is_empty() {
            anyhow::bail!(
                "ring-context sidecar {sidecar_path} is missing {} of {} checked-in templates \
                 (e.g. {:?}) -- refusing to load a sidecar with incomplete coverage",
                missing.len(),
                expected_ids.len(),
                missing.iter().take(3).collect::<Vec<_>>()
            );
        }
        let extra: Vec<&String> = actual_ids.difference(&expected_ids).collect();
        if !extra.is_empty() {
            anyhow::bail!(
                "ring-context sidecar {sidecar_path} has {} entries not present in the loaded \
                 template file (e.g. {:?}) -- refusing to load a sidecar generated against a \
                 different template set",
                extra.len(),
                extra.iter().take(3).collect::<Vec<_>>()
            );
        }

        let mut compiled = FxHashMap::default();
        for (template_id, tmpl) in sidecar.templates {
            let recomputed_id = crate::chem_env::template_id_for_smirks(&tmpl.simplified_smirks);
            if recomputed_id != template_id {
                anyhow::bail!(
                    "ring-context sidecar {sidecar_path} entry key {template_id} does not match \
                     template_id_for_smirks(simplified_smirks) = {recomputed_id} -- sidecar is \
                     corrupt or was hand-edited"
                );
            }

            let mut changed_bond_intents = FxHashMap::default();
            for cb in &tmpl.changed_bonds {
                if cb.map_a == cb.map_b {
                    anyhow::bail!(
                        "ring-context sidecar {sidecar_path} template {template_id} has a \
                         changed bond with map_a == map_b == {} -- not a real bond",
                        cb.map_a
                    );
                }
                let key = if cb.map_a < cb.map_b {
                    (cb.map_a, cb.map_b)
                } else {
                    (cb.map_b, cb.map_a)
                };
                if changed_bond_intents.contains_key(&key) {
                    anyhow::bail!(
                        "ring-context sidecar {sidecar_path} template {template_id} declares \
                         changed bond {key:?} more than once"
                    );
                }
                let recomputed_intent = recompute_intent(cb);
                if recomputed_intent != cb.intent {
                    anyhow::bail!(
                        "ring-context sidecar {sidecar_path} template {template_id} bond {key:?} \
                         declares intent {:?} but its own observation counts \
                         (ring={}, non_ring={}, ambiguous={}, unknown={}) recompute to {:?}",
                        cb.intent,
                        cb.ring_observations,
                        cb.non_ring_observations,
                        cb.ambiguous_observations,
                        cb.unknown_observations,
                        recomputed_intent
                    );
                }
                changed_bond_intents.insert(key, cb.intent);
            }

            // Independently re-derive this template's changed bonds from its
            // own SMIRKS and cross-check against what the sidecar declares.
            // `None` means `parse_reaction` can't parse this SMIRKS at all
            // (the same pre-existing, unrelated `#7`-style-atomic-number gap
            // handled below) -- there is nothing to cross-check against in
            // that case, not a validation failure.
            if let Some(actual_bonds) = recompute_changed_bonds(&tmpl.simplified_smirks) {
                let declared_bonds: std::collections::HashSet<(u16, u16)> =
                    changed_bond_intents.keys().copied().collect();
                if declared_bonds != actual_bonds {
                    anyhow::bail!(
                        "ring-context sidecar {sidecar_path} template {template_id} declares \
                         changed bonds {declared_bonds:?} but re-deriving LHS-minus-RHS from its \
                         own simplified_smirks gives {actual_bonds:?}"
                    );
                }
            }
            // A handful of extracted templates parse fine under
            // `chematic::smarts::parse_smarts` (chem_env.rs's own
            // `load_rules_from_file` validation, hence still present in the
            // .smi corpus) but fail under `chematic::rxn::parse_reaction`
            // (e.g. `#7`-style atomic-number SMARTS primitives, which
            // `parse_reaction`'s stricter SMILES-shaped grammar rejects).
            // Issue #88 fixed this by trying every independently-validated
            // concrete-element reading (`application_smirks_variants`) at
            // both `find_reaction_matches`/`apply_reaction_match` call
            // sites in this file (`run_diagnostics_pass`/`run_gated_pass`),
            // so such a template genuinely does match/apply here now, not
            // just at the plain `apply_retro` path -- an empty
            // `atom_map_table` here is a real correctness gap (every match
            // fails closed as `InvalidMappedBond`, before ever reaching
            // ring-context classification), not unreachable dead data.
            // Every `[#N]` reading of one template shares the identical
            // atom-map layout/connectivity by construction (only the
            // element/aromaticity annotation differs), so trying variants
            // in order and using the first one that parses is sufficient
            // -- there is no "which variant's atom_map_table" ambiguity.
            let atom_map_table = if changed_bond_intents.is_empty() {
                Vec::new()
            } else {
                smirks_variants_to_try(&tmpl.simplified_smirks)
                    .iter()
                    .find_map(|variant| lhs_atom_map_table(variant))
                    .unwrap_or_default()
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

/// Bundles the ring-context config for threading through
/// `candidate::raw_propose` without changing its signature on every future
/// addition. `Default` (`RingContextConfig::Disabled`) reproduces
/// pre-existing search behaviour exactly -- the only caller that needs
/// anything else is `search::find_routes`, driven by
/// `SearchConfig::ring_context`. Owns a clone of the config (cheap: an Arc
/// bump or a unit variant) rather than borrowing, so no lifetime parameter
/// is needed here.
#[derive(Clone, Default)]
pub struct RingContextArgs {
    pub config: RingContextConfig,
}

// ── Public entry point ─────────────────────────────────────────────────

/// Sibling to [`apply_retro`] that additionally gates extracted templates
/// through the ring-context and element-accounting checks according to
/// `config`. At [`RingContextConfig::Disabled`] (or for any non-extracted
/// rule), delegates directly to the untouched [`apply_retro`] -- the exact
/// legacy path, not a reimplementation. Unlike the policy/guard pair this
/// replaced, `Guarded` always carries a real guard -- "enforce/audit
/// without a guard" is not representable, so there is no fallback path to
/// document here.
pub fn apply_retro_with_policy(
    mol: &Molecule,
    rule: &RetroRule,
    config: &RingContextConfig,
    diagnostics: &mut RingContextDiagnostics,
) -> Vec<Vec<PrecursorMol>> {
    let (guard, policy) = match config {
        RingContextConfig::Disabled => return apply_retro(mol, rule),
        RingContextConfig::Guarded { guard, policy } => (guard.as_ref(), *policy),
    };
    if !crate::search::is_extracted_template(&rule.name) {
        return apply_retro(mol, rule);
    }

    let Some(compiled) = guard.compiled.get(&rule.template_id) else {
        diagnostics.templates_missing_metadata += 1;
        return if policy.ring_context == Enforcement::AuditOnly {
            // ponytail: can't run the match-level pipeline at all without
            // per-template metadata (no CompiledTemplate to classify
            // against), so this falls back to fully unfiltered legacy
            // output even under ElementOnly ablation -- element-accounting
            // isn't independently enforceable here either without one.
            // Unreachable on the real 500-template corpus today (sidecar
            // coverage is verified template-by-template at load time, see
            // `RingContextGuard::load`); upgrade path if it ever fires:
            // synthesize a permissive all-Either `CompiledTemplate` instead
            // of bypassing element-accounting too.
            apply_retro(mol, rule)
        } else {
            vec![]
        };
    };

    if policy.ring_context == Enforcement::AuditOnly
        && policy.element_accounting == Enforcement::AuditOnly
    {
        run_diagnostics_pass(mol, rule, compiled, diagnostics);
        apply_retro(mol, rule)
    } else {
        run_gated_pass(mol, rule, compiled, policy, diagnostics)
    }
}

/// The concrete-element SMIRKS to actually attempt for `smirks` (Issue
/// #88): `find_reaction_matches`/`apply_reaction_match` are, like
/// `chematic::rxn::run_reactants`, SMILES-grammar-based and cannot parse a
/// bare `[#N]` atomic-number SMARTS primitive -- this match-level API is
/// a *separate* code path from `apply_retro` (chematic 0.10.0 added it
/// specifically for this guard) and does not automatically inherit
/// `chem_env::apply_retro`'s hash-atom fix, so it needs the same
/// treatment independently. A single-element `Vec` for an ordinary
/// SMIRKS, matching today's direct-call behavior exactly; every
/// independently-validated concrete-element reading for a `[#N]`-bearing
/// one. `crate::chem_env::application_smirks_variants` is cached by
/// SMIRKS string, so repeated calls for the same rule are cheap.
fn smirks_variants_to_try(smirks: &str) -> Vec<String> {
    if !smirks.contains('#') {
        return vec![smirks.to_string()];
    }
    crate::chem_env::application_smirks_variants(smirks)
        .as_ref()
        .clone()
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
    let ring_cache = RingBondCache::new(mol);
    for variant in smirks_variants_to_try(&rule.smirks) {
        diagnostics.reaction_parse_calls += 1;
        let matches = match find_reaction_matches(&variant, &[mol]) {
            Ok(m) => m,
            Err(_) => {
                diagnostics.reaction_application_failed += 1;
                continue;
            }
        };
        diagnostics.matches_enumerated += matches.len() as u64;
        for m in &matches {
            match classify_match(m, compiled, &ring_cache, diagnostics) {
                MatchVerdict::Accept => {}
                MatchVerdict::Reject(reason) => {
                    record_reject(diagnostics, reason);
                    continue;
                }
            }
            // Element-accounting is diagnosed too, on the actual applied
            // outcome, gated behind the same ring-context accept that
            // Conservative uses -- so AuditOnly's counters reflect exactly
            // what Conservative would accept/reject, over the same
            // denominator, but never filter the returned outcomes here.
            diagnostics.matches_applied += 1;
            diagnostics.reaction_parse_calls += 1;
            if let Ok(Some(products)) = apply_reaction_match(&variant, &[mol], m, true) {
                let precs: Vec<PrecursorMol> = products.iter().flat_map(split_fragments).collect();
                if !element_accounting_ok(mol, &precs) {
                    diagnostics.outcomes_element_rejected += 1;
                } else {
                    diagnostics.outcomes_accepted += 1;
                }
            } else {
                diagnostics.valence_filtered += 1;
            }
        }
    }
}

/// Builds the real, filtered `Vec<Vec<PrecursorMol>>` for `Conservative`:
/// enumerate matches, keep only those passing ring-context, apply each via
/// `apply_reaction_match`, then keeps outcomes passing element-accounting
/// (or all outcomes, if `policy.element_accounting` is only `AuditOnly`).
/// Handles `Conservative` (both axes `Enforce`) and the `RingOnly`/
/// `ElementOnly` ablation policies (one axis `Enforce`, the other
/// `AuditOnly`) uniformly -- both axes are always classified/diagnosed
/// regardless of enforcement, only the decision to actually exclude a
/// match/outcome differs. Never called for the pure-`AuditOnly` policy
/// (both axes `AuditOnly`); that case goes through `run_diagnostics_pass`
/// instead, which guarantees byte-identical-to-`Disabled` output by never
/// reconstructing it via find+apply at all. Accepted-match order follows
/// `find_reaction_matches`'s own returned order.
fn run_gated_pass(
    mol: &Molecule,
    rule: &RetroRule,
    compiled: &CompiledTemplate,
    policy: ExtractedTemplateSafetyPolicy,
    diagnostics: &mut RingContextDiagnostics,
) -> Vec<Vec<PrecursorMol>> {
    let ring_cache = RingBondCache::new(mol);
    let mut outcomes: Vec<Vec<PrecursorMol>> = Vec::new();
    // A [#N]-bearing rule (Issue #88) may have multiple validated
    // readings; two readings producing the same real precursor set for
    // this molecule must not appear as separate outcomes.
    let mut seen_signatures: FxHashSet<Vec<String>> = FxHashSet::default();

    for variant in smirks_variants_to_try(&rule.smirks) {
        diagnostics.reaction_parse_calls += 1;
        let matches = match find_reaction_matches(&variant, &[mol]) {
            Ok(m) => m,
            Err(_) => {
                diagnostics.reaction_application_failed += 1;
                continue;
            }
        };
        diagnostics.matches_enumerated += matches.len() as u64;

        for m in &matches {
            if let MatchVerdict::Reject(reason) =
                classify_match(m, compiled, &ring_cache, diagnostics)
            {
                record_reject(diagnostics, reason);
                if policy.ring_context == Enforcement::Enforce {
                    continue;
                }
            }
            diagnostics.matches_applied += 1;
            diagnostics.reaction_parse_calls += 1;
            match apply_reaction_match(&variant, &[mol], m, true) {
                Ok(Some(products)) => {
                    let precs: Vec<PrecursorMol> =
                        products.iter().flat_map(split_fragments).collect();
                    let accept_for_element_accounting = if element_accounting_ok(mol, &precs) {
                        diagnostics.outcomes_accepted += 1;
                        true
                    } else {
                        diagnostics.outcomes_element_rejected += 1;
                        policy.element_accounting != Enforcement::Enforce
                    };
                    if accept_for_element_accounting {
                        let mut signature: Vec<String> =
                            precs.iter().map(|p| p.smiles.clone()).collect();
                        signature.sort_unstable();
                        if seen_signatures.insert(signature) {
                            outcomes.push(precs);
                        }
                    }
                }
                Ok(None) => diagnostics.valence_filtered += 1,
                Err(_) => diagnostics.reaction_application_failed += 1,
            }
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

    /// Wraps a loaded guard + policy into a `RingContextConfig::Guarded`,
    /// the only way to construct a non-`Disabled` config -- there is no
    /// `Enforce`/`AuditOnly` state reachable without a real guard.
    fn guarded(
        guard: RingContextGuard,
        policy: ExtractedTemplateSafetyPolicy,
    ) -> RingContextConfig {
        RingContextConfig::Guarded {
            guard: std::sync::Arc::new(guard),
            policy,
        }
    }

    fn smiles_of(precs: &[Vec<PrecursorMol>]) -> Vec<Vec<String>> {
        precs
            .iter()
            .map(|p| p.iter().map(|x| x.smiles.clone()).collect())
            .collect()
    }

    /// Sidecar with exactly one template (extracted_9), NonRing intent on
    /// (map 1, map 5) -- matching the real generated corpus's actual
    /// (post-attribution-fix) classification (231 non-ring observations, 0
    /// ring, 0 ambiguous).
    fn nonring_sidecar_json(template_file_sha256: &str) -> String {
        format!(
            r#"{{
                "schema_version": 2,
                "template_file_sha256": "{template_file_sha256}",
                "templates": {{
                    "{tid}": {{
                        "simplified_smirks": "{smirks}",
                        "changed_bonds": [
                            {{"map_a": 1, "map_b": 5, "operation": "delete", "intent": "non_ring",
                              "ring_observations": 0, "non_ring_observations": 231,
                              "ambiguous_observations": 0, "unknown_observations": 0}}
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

    // ── Hash-atom ([#N]) x ring-context guard interaction (Issue #88) ───
    //
    // Confirms the Issue #88 fix's central compatibility claim: a
    // `[#N]`-bearing template's expanded application-time variants still
    // resolve against a sidecar generated for the *original* (unexpanded)
    // template, because `load_rules_from_file` never changes
    // `RetroRule::template_id`/`name`/`smirks` -- there is only ever one
    // real `RetroRule` per line for the guard to look up by template_id,
    // exactly as before this fix.

    /// 2-anilinopyrimidine-class retro template with two `[#7]` ring
    /// nitrogens (map 2, map 3) that are pure spectators, and one real
    /// changed bond: the aryl C(map 1)-N(map 4) bond formed by the SNAr
    /// amination this template represents (present in the target pattern,
    /// absent from the precursor pattern -- Cl takes map 1's other slot,
    /// map 4 becomes a free amine).
    const HASH_ATOM_SMIRKS: &str =
        "[#7:2]:[c:1](-[NH:4]-[c:5]):[#7:3]>>Cl-[c:1](:[#7:2]):[#7:3].[NH2:4]-[c:5]";

    fn hash_atom_rule() -> RetroRule {
        RetroRule {
            name: "extracted_hashtest".to_string(),
            template_id: template_id_for_smirks(HASH_ATOM_SMIRKS),
            smirks: HASH_ATOM_SMIRKS.to_string(),
            weight: 1.0,
            required_elements: 0,
        }
    }

    /// Sidecar for `HASH_ATOM_SMIRKS`, declaring its one real changed bond
    /// (map 1 - map 4, the formed aryl C-N bond) as `non_ring` -- correct
    /// for this chemistry (an intermolecular amination), independent of
    /// which `[#7]` aromatic/aliphatic reading a given application-time
    /// variant happens to use, since the guard's classification runs on
    /// atom-map identity, not element spelling.
    fn hash_atom_nonring_sidecar_json(template_file_sha256: &str) -> String {
        format!(
            r#"{{
                "schema_version": 2,
                "template_file_sha256": "{template_file_sha256}",
                "templates": {{
                    "{tid}": {{
                        "simplified_smirks": "{smirks}",
                        "changed_bonds": [
                            {{"map_a": 1, "map_b": 4, "operation": "delete", "intent": "non_ring",
                              "ring_observations": 0, "non_ring_observations": 167,
                              "ambiguous_observations": 0, "unknown_observations": 0}}
                        ]
                    }}
                }}
            }}"#,
            tid = template_id_for_smirks(HASH_ATOM_SMIRKS),
            smirks = HASH_ATOM_SMIRKS,
        )
    }

    fn hash_atom_templates_smi_fixture() -> String {
        format!("{HASH_ATOM_SMIRKS}\t167\n")
    }

    #[test]
    fn hash_atom_expanded_template_resolves_sidecar_by_original_template_id_across_all_policies() {
        let smi = hash_atom_templates_smi_fixture();
        let sidecar = hash_atom_nonring_sidecar_json("__HASH__");
        let rule = hash_atom_rule();
        let target = mol_from_smiles("c1ccc(Nc2ncccn2)cc1").unwrap(); // 2-anilinopyrimidine

        let policies = [
            ("AuditOnly", ExtractedTemplateSafetyPolicy::AUDIT_ONLY),
            ("Conservative", ExtractedTemplateSafetyPolicy::CONSERVATIVE),
            ("RingOnly", ExtractedTemplateSafetyPolicy::RING_ONLY),
            ("ElementOnly", ExtractedTemplateSafetyPolicy::ELEMENT_ONLY),
        ];

        // Disabled: delegates straight to `apply_retro`, no guard involved
        // at all -- included as the baseline every guarded policy is
        // compared against.
        let mut disabled_diagnostics = RingContextDiagnostics::default();
        let disabled_outcomes = apply_retro_with_policy(
            &target,
            &rule,
            &RingContextConfig::Disabled,
            &mut disabled_diagnostics,
        );
        assert!(
            !disabled_outcomes.is_empty(),
            "Disabled: must still decompose the real target via the internal hash-atom \
             variant path"
        );

        for (label, policy) in policies {
            let guard = load_guard_with_intent(&sidecar, &smi);
            let config = guarded(guard, policy);
            let mut diagnostics = RingContextDiagnostics::default();
            let outcomes = apply_retro_with_policy(&target, &rule, &config, &mut diagnostics);
            assert_eq!(
                diagnostics.templates_missing_metadata, 0,
                "{label}: expanded hash-atom rule's template_id must still resolve against \
                 the sidecar generated for the original (unexpanded) template"
            );
            assert_eq!(
                diagnostics.ring_rejects_nonring_intent_on_ring_bond, 0,
                "{label}: the real C-N bond is genuinely non-ring, matching the declared \
                 intent, so a mismatch must never fire here"
            );
            assert!(
                !outcomes.is_empty(),
                "{label}: a correctly-classified match must be accepted, not silently \
                 zero-resulted merely because the applied rule went through hash-atom \
                 expansion"
            );
        }
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

    /// Writes `sidecar_json` (with `__HASH__` substituted for the real hash
    /// of `templates_smi`) to a fresh temp path and returns
    /// `RingContextGuard::load`'s raw result, without unwrapping -- for
    /// tampered-sidecar tests that expect load to fail.
    fn try_load_with_intent(
        sidecar_json: &str,
        templates_smi: &str,
    ) -> anyhow::Result<RingContextGuard> {
        let hash = sha256_hex(Sha256::digest(templates_smi.as_bytes()));
        let sidecar = sidecar_json.replace("__HASH__", &hash);
        let dir = std::env::temp_dir().join(format!(
            "renkin_ring_context_tamper_{}_{}",
            std::process::id(),
            next_temp_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sidecar.json");
        std::fs::write(&path, sidecar).unwrap();
        RingContextGuard::load(path.to_str().unwrap(), templates_smi)
    }

    // ── Guard loading: tampered/corrupt sidecar rejection ──────────────

    #[test]
    fn guard_load_rejects_incomplete_template_coverage() {
        // Hash-matching (the raw .smi bytes are untouched) but declares
        // ZERO templates -- extracted_9's entry is simply missing.
        let smi = templates_smi_fixture();
        let sidecar =
            r#"{"schema_version": 2, "template_file_sha256": "__HASH__", "templates": {}}"#;
        let result = try_load_with_intent(sidecar, &smi);
        assert!(
            result.is_err(),
            "sidecar missing a checked-in template's entry must fail closed"
        );
    }

    #[test]
    fn guard_load_rejects_unknown_extra_template_entry() {
        let smi = templates_smi_fixture();
        let extra_smirks = "[C:1]-[O:2]>>[C:1]=[O:2]";
        let sidecar = format!(
            r#"{{"schema_version": 2, "template_file_sha256": "__HASH__", "templates": {{
                "{tid}": {{"simplified_smirks": "{smirks}", "changed_bonds": [
                    {{"map_a": 1, "map_b": 5, "intent": "non_ring", "ring_observations": 0,
                      "non_ring_observations": 231, "ambiguous_observations": 0, "unknown_observations": 0}}
                ]}},
                "{extra_tid}": {{"simplified_smirks": "{extra_smirks}", "changed_bonds": []}}
            }}}}"#,
            tid = template_id_for_smirks(EXTRACTED_9_SMIRKS),
            smirks = EXTRACTED_9_SMIRKS,
            extra_tid = template_id_for_smirks(extra_smirks),
            extra_smirks = extra_smirks,
        );
        let result = try_load_with_intent(&sidecar, &smi);
        assert!(
            result.is_err(),
            "sidecar entry not present in the loaded template file must fail closed"
        );
    }

    #[test]
    fn guard_load_rejects_key_smirks_mismatch() {
        let smi = templates_smi_fixture();
        let sidecar = format!(
            r#"{{"schema_version": 2, "template_file_sha256": "__HASH__", "templates": {{
                "smirks-sha256:0000000000000000000000000000000000000000000000000000000000000000": {{
                    "simplified_smirks": "{smirks}",
                    "changed_bonds": [
                        {{"map_a": 1, "map_b": 5, "intent": "non_ring", "ring_observations": 0,
                          "non_ring_observations": 231, "ambiguous_observations": 0, "unknown_observations": 0}}
                    ]
                }}
            }}}}"#,
            smirks = EXTRACTED_9_SMIRKS,
        );
        let result = try_load_with_intent(&sidecar, &smi);
        assert!(
            result.is_err(),
            "sidecar entry keyed under the wrong template_id must fail closed"
        );
    }

    #[test]
    fn guard_load_rejects_duplicate_changed_bond() {
        let smi = templates_smi_fixture();
        let sidecar = format!(
            r#"{{"schema_version": 2, "template_file_sha256": "__HASH__", "templates": {{
                "{tid}": {{"simplified_smirks": "{smirks}", "changed_bonds": [
                    {{"map_a": 1, "map_b": 5, "intent": "non_ring", "ring_observations": 0,
                      "non_ring_observations": 100, "ambiguous_observations": 0, "unknown_observations": 0}},
                    {{"map_a": 5, "map_b": 1, "intent": "non_ring", "ring_observations": 0,
                      "non_ring_observations": 131, "ambiguous_observations": 0, "unknown_observations": 0}}
                ]}}
            }}}}"#,
            tid = template_id_for_smirks(EXTRACTED_9_SMIRKS),
            smirks = EXTRACTED_9_SMIRKS,
        );
        let result = try_load_with_intent(&sidecar, &smi);
        assert!(
            result.is_err(),
            "declaring the same changed bond twice (regardless of map_a/map_b order) must fail closed"
        );
    }

    #[test]
    fn guard_load_rejects_self_loop_changed_bond() {
        let smi = templates_smi_fixture();
        let sidecar = format!(
            r#"{{"schema_version": 2, "template_file_sha256": "__HASH__", "templates": {{
                "{tid}": {{"simplified_smirks": "{smirks}", "changed_bonds": [
                    {{"map_a": 3, "map_b": 3, "intent": "non_ring", "ring_observations": 0,
                      "non_ring_observations": 231, "ambiguous_observations": 0, "unknown_observations": 0}}
                ]}}
            }}}}"#,
            tid = template_id_for_smirks(EXTRACTED_9_SMIRKS),
            smirks = EXTRACTED_9_SMIRKS,
        );
        let result = try_load_with_intent(&sidecar, &smi);
        assert!(
            result.is_err(),
            "map_a == map_b is not a real bond and must fail closed"
        );
    }

    #[test]
    fn guard_load_rejects_intent_not_matching_observation_counts() {
        let smi = templates_smi_fixture();
        // Declares "ring" while every observation count says non_ring.
        let sidecar = format!(
            r#"{{"schema_version": 2, "template_file_sha256": "__HASH__", "templates": {{
                "{tid}": {{"simplified_smirks": "{smirks}", "changed_bonds": [
                    {{"map_a": 1, "map_b": 5, "intent": "ring", "ring_observations": 0,
                      "non_ring_observations": 231, "ambiguous_observations": 0, "unknown_observations": 0}}
                ]}}
            }}}}"#,
            tid = template_id_for_smirks(EXTRACTED_9_SMIRKS),
            smirks = EXTRACTED_9_SMIRKS,
        );
        let result = try_load_with_intent(&sidecar, &smi);
        assert!(
            result.is_err(),
            "declared intent must match what its own observation counts recompute to"
        );
    }

    #[test]
    fn guard_load_rejects_changed_bond_not_matching_recomputed_lhs_minus_rhs() {
        let smi = templates_smi_fixture();
        // (map 4, map 5) is a bond present on BOTH sides of extracted_9's
        // SMIRKS (C4-N5 survives into the RHS fragment) -- not a real
        // changed bond. The real one is (1, 5).
        let sidecar = format!(
            r#"{{"schema_version": 2, "template_file_sha256": "__HASH__", "templates": {{
                "{tid}": {{"simplified_smirks": "{smirks}", "changed_bonds": [
                    {{"map_a": 4, "map_b": 5, "intent": "non_ring", "ring_observations": 0,
                      "non_ring_observations": 231, "ambiguous_observations": 0, "unknown_observations": 0}}
                ]}}
            }}}}"#,
            tid = template_id_for_smirks(EXTRACTED_9_SMIRKS),
            smirks = EXTRACTED_9_SMIRKS,
        );
        let result = try_load_with_intent(&sidecar, &smi);
        assert!(
            result.is_err(),
            "a changed bond that isn't actually LHS-minus-RHS on its own SMIRKS must fail closed"
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
        let config = guarded(guard, ExtractedTemplateSafetyPolicy::CONSERVATIVE);
        let mut diag = RingContextDiagnostics::default();

        let legacy = apply_retro(&mol, &rule);
        assert!(
            !legacy.is_empty(),
            "legacy path must still misapply extracted_9 here (that's the bug)"
        );

        let conservative = apply_retro_with_policy(&mol, &rule, &config, &mut diag);
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
        let config = guarded(guard, ExtractedTemplateSafetyPolicy::CONSERVATIVE);
        let mut diag = RingContextDiagnostics::default();

        let legacy = apply_retro(&mol, &rule);
        let conservative = apply_retro_with_policy(&mol, &rule, &config, &mut diag);
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
                apply_retro_with_policy(&mol, &rule, &RingContextConfig::Disabled, &mut diag);
            assert_eq!(smiles_of(&legacy), smiles_of(&disabled));
            assert_eq!(
                diag.matches_enumerated, 0,
                "Disabled must never enumerate matches"
            );
        }
    }

    #[test]
    fn auditonly_returns_legacy_output_even_though_isoindolinone_match_is_unsafe() {
        let rule = extracted_9_rule();
        let mol = mol_from_smiles(ISOINDOLINONE_RING_CASE).unwrap();
        let smi = format!("{EXTRACTED_9_SMIRKS}\t231\n");
        let guard = load_guard_with_intent(&nonring_sidecar_json("__HASH__"), &smi);
        let config = guarded(guard, ExtractedTemplateSafetyPolicy::AUDIT_ONLY);
        let mut diag = RingContextDiagnostics::default();

        let legacy = apply_retro(&mol, &rule);
        let audit = apply_retro_with_policy(&mol, &rule, &config, &mut diag);

        assert_eq!(
            smiles_of(&legacy),
            smiles_of(&audit),
            "AuditOnly must be byte-identical to legacy by construction"
        );
        assert_eq!(
            diag.ring_rejects_nonring_intent_on_ring_bond, 1,
            "AuditOnly must still record what Conservative would have rejected"
        );
    }

    // ── RingOnly / ElementOnly ablation: axes filter independently ─────

    #[test]
    fn ring_only_ablation_still_rejects_ring_unsafe_match() {
        let rule = extracted_9_rule();
        let mol = mol_from_smiles(ISOINDOLINONE_RING_CASE).unwrap();
        let smi = format!("{EXTRACTED_9_SMIRKS}\t231\n");
        let guard = load_guard_with_intent(&nonring_sidecar_json("__HASH__"), &smi);
        let config = guarded(guard, ExtractedTemplateSafetyPolicy::RING_ONLY);
        let mut diag = RingContextDiagnostics::default();

        let result = apply_retro_with_policy(&mol, &rule, &config, &mut diag);
        assert!(
            result.is_empty(),
            "RingOnly enforces the ring-context axis regardless of the element-accounting axis"
        );
        assert_eq!(diag.ring_rejects_nonring_intent_on_ring_bond, 1);
    }

    #[test]
    fn element_only_ablation_still_attempts_ring_unsafe_match() {
        // Note this molecule's ring-fused topology happens to *also* fail
        // element-accounting independently (opening a fused ring via a
        // 2-product-split template can't cleanly separate into two
        // fragments, since a ring bond's removal never disconnects a
        // graph -- unlike RingOnly/Conservative, this isn't proven by
        // "empty output", since element-accounting alone would empty it
        // too. What ElementOnly's ring axis being AuditOnly actually
        // guarantees is that the ring-flagged match still reaches
        // `apply_reaction_match` at all (`matches_applied` increments)
        // rather than being skipped before ever getting there.
        let rule = extracted_9_rule();
        let mol = mol_from_smiles(ISOINDOLINONE_RING_CASE).unwrap();
        let smi = format!("{EXTRACTED_9_SMIRKS}\t231\n");
        let guard = load_guard_with_intent(&nonring_sidecar_json("__HASH__"), &smi);
        let config = guarded(guard, ExtractedTemplateSafetyPolicy::ELEMENT_ONLY);
        let mut diag = RingContextDiagnostics::default();

        apply_retro_with_policy(&mol, &rule, &config, &mut diag);
        assert_eq!(
            diag.matches_applied, 1,
            "ElementOnly's ring axis is AuditOnly -- the ring-flagged match must still reach \
             apply_reaction_match rather than being skipped"
        );
        assert_eq!(
            diag.ring_rejects_nonring_intent_on_ring_bond, 1,
            "the ring-context axis is still classified/diagnosed even though not enforced"
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
        // A REAL guard is loaded and policy IS Conservative here -- proving
        // the bypass is keyed on `is_extracted_template(&rule.name)`, not
        // merely on the weaker "no guard was even loaded" case.
        let smi = templates_smi_fixture();
        let guard = load_guard_with_intent(&nonring_sidecar_json("__HASH__"), &smi);
        let config = guarded(guard, ExtractedTemplateSafetyPolicy::CONSERVATIVE);
        let gated = apply_retro_with_policy(&mol, &rule, &config, &mut diag);
        assert_eq!(smiles_of(&legacy), smiles_of(&gated));
        assert_eq!(diag.matches_enumerated, 0);
    }

    // ── `MissingTopologyMetadata`: defensive path, unreachable via the real
    // loader (which now hard-errors on incomplete coverage -- see
    // `guard_load_rejects_incomplete_template_coverage` above) but still
    // real code, exercised here by constructing a `RingContextGuard`
    // directly rather than through `RingContextGuard::load`. ─────────────

    #[test]
    fn missing_topology_metadata_conservative_rejects_fail_closed() {
        let guard = RingContextGuard {
            compiled: FxHashMap::default(),
        };
        let config = guarded(guard, ExtractedTemplateSafetyPolicy::CONSERVATIVE);

        let rule = extracted_9_rule();
        let mol = mol_from_smiles(ACYCLIC_NONRING_CASE).unwrap();
        let mut diag = RingContextDiagnostics::default();
        let result = apply_retro_with_policy(&mol, &rule, &config, &mut diag);
        assert!(
            result.is_empty(),
            "missing per-template metadata must fail closed under Conservative"
        );
        assert_eq!(diag.templates_missing_metadata, 1);
    }

    #[test]
    fn missing_topology_metadata_auditonly_still_returns_legacy() {
        let guard = RingContextGuard {
            compiled: FxHashMap::default(),
        };
        let config = guarded(guard, ExtractedTemplateSafetyPolicy::AUDIT_ONLY);

        let rule = extracted_9_rule();
        let mol = mol_from_smiles(ACYCLIC_NONRING_CASE).unwrap();
        let legacy = apply_retro(&mol, &rule);
        let mut diag = RingContextDiagnostics::default();
        let result = apply_retro_with_policy(&mol, &rule, &config, &mut diag);
        assert_eq!(smiles_of(&legacy), smiles_of(&result));
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
    fn ring_context_args_default_is_disabled() {
        let args = RingContextArgs::default();
        assert!(matches!(args.config, RingContextConfig::Disabled));
    }

    /// Sanity check that the real checked-in extracted-500 corpus loads
    /// via `load_rules_from_file` cleanly enough to build a `RetroRule`
    /// list containing `extracted_9` under its expected name -- a
    /// regression guard for the test fixtures above staying in sync with
    /// the real template file's line position.
    #[test]
    fn extracted_9_name_matches_checked_in_corpus_position() {
        let rules = load_rules_from_file("data/templates_extracted_500.smi");
        let rule9 = rules.iter().find(|r| r.name == "extracted_9").expect(
            "extracted_9 must exist in the checked-in corpus -- if this fails, the corpus \
                 was re-extracted/reordered and every fixture above keyed on EXTRACTED_9_SMIRKS \
                 needs to be revisited, not silently skipped",
        );
        assert_eq!(
            rule9.smirks, EXTRACTED_9_SMIRKS,
            "extracted_9's real position in the checked-in corpus no longer matches this test's fixture SMIRKS -- update EXTRACTED_9_SMIRKS"
        );
    }
}
