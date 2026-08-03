use std::collections::BinaryHeap;
use std::sync::Arc;

use anyhow::Result;
use chematic::chem::{molecular_weight, sa_score};
use rustc_hash::{FxHashMap, FxHashSet, FxHasher};
use serde::Serialize;
use smallvec::{SmallVec, smallvec};

use crate::chem_env::{
    ChemEnv, RetroRule, TemplateBondIndex, canonical_stock_identity_from_smiles, mol_from_smiles,
    to_canonical,
};
use crate::evidence::{EvidenceScope, MetadataSource, StepEvidence, TemplateMetadataEntry};
use crate::score::{step_cost, template_bonus};

/// Cached expansion for one (target_smiles, rule) combination.
struct RetroEntry {
    rule_name: String,
    template_id: String,
    step_cost: f64,
    precursor_smiles: Vec<String>,
}
type RetroCache = FxHashMap<String, Arc<Vec<RetroEntry>>>;

/// Suggested reaction conditions for a synthesis step (rule-based, hand-crafted rules only).
#[derive(Debug, Clone, Serialize)]
pub struct ReactionConditions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalyst: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub solvent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// Why `ReactionStep::atom_economy` is (or isn't) populated. `AboveExpectedRange`
/// means MW(target) / Σ MW(precursors) computed to more than 100% -- the
/// precursor set RENKIN represents supplies less mass than the target
/// needs. This is **not** proof of target-atom loss on its own: the
/// denominator is only the precursors a template names, not every reactant
/// or reagent the real reaction would use, so an omitted reagent (a
/// deprotection's H2, a leaving-group source, a catalyst -- none of which
/// carry target atoms) can push this ratio over 100% for a perfectly valid
/// route. Whether atoms were genuinely lost is a question for the
/// independent directional element-accounting check
/// (`synthesizability::element_accounting::compute_element_accounting`),
/// not this MW ratio (Issue #79). Earlier behaviour silently clamped this
/// ratio down to 100.0, which looked identical to a genuinely
/// perfect-economy route.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AtomEconomyStatus {
    Normal,
    AboveExpectedRange,
    NotEvaluable,
}

/// Computes the raw (unclamped) MW(target)/Σ MW(precursors)×100 ratio for a
/// step, or `None` if either side isn't cleanly evaluable. All-or-nothing on
/// the precursor side: a single unparseable precursor must not silently
/// shrink the denominator and inflate the ratio (a `filter_map`-based sum
/// would do exactly that). Never returns a non-finite value.
fn compute_atom_economy_raw(target_smiles: &str, precursors: &[String]) -> Option<f64> {
    let target_weight = mol_from_smiles(target_smiles)
        .ok()
        .map(|m| molecular_weight(&m))?;
    let precursor_weights: Vec<f64> = precursors
        .iter()
        .map(|s| mol_from_smiles(s).ok().map(|m| molecular_weight(&m)))
        .collect::<Option<Vec<f64>>>()?;
    let precursor_weight: f64 = precursor_weights.iter().sum();
    if !target_weight.is_finite()
        || !precursor_weight.is_finite()
        || target_weight < 0.0
        || precursor_weight <= 0.0
    {
        return None;
    }
    let ratio = target_weight / precursor_weight * 100.0;
    ratio.is_finite().then_some(ratio)
}

/// Classifies a raw (unclamped) MW(target)/Σ MW(precursors)×100 ratio into
/// (status, display value). `display` is `Some(raw)` only for `Normal` --
/// never a clamped substitute, so a caller can't mistake "not evaluable in
/// the normal sense" for a genuinely perfect route. A non-finite `raw`
/// (NaN/±Infinity) is defensively treated as `NotEvaluable` -- callers are
/// expected to already guard against this (see `find_routes`'s
/// post-processing step), but this pure function must never trust that.
fn classify_atom_economy(raw: Option<f64>) -> (AtomEconomyStatus, Option<f64>) {
    let status = match raw {
        Some(r) if r.is_finite() && r > 100.0 + 1e-6 => AtomEconomyStatus::AboveExpectedRange,
        Some(r) if r.is_finite() => AtomEconomyStatus::Normal,
        _ => AtomEconomyStatus::NotEvaluable,
    };
    let display = match status {
        AtomEconomyStatus::Normal => raw,
        AtomEconomyStatus::AboveExpectedRange | AtomEconomyStatus::NotEvaluable => None,
    };
    (status, display)
}

#[derive(Debug, Clone, Serialize)]
pub struct ReactionStep {
    pub rule: String,
    /// Stable identity of the template used (see `RetroRule::template_id`).
    /// Always populated -- `rule:<name>` for hand-crafted rules,
    /// `smirks-sha256:<hex>` for extracted templates.
    pub template_id: String,
    pub target: String,
    pub precursors: Vec<String>,
    /// Suggested conditions for the forward reaction (None for extracted templates).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conditions: Option<ReactionConditions>,
    /// Atom economy: MW(target) / Σ MW(precursors) × 100 — fraction of atoms
    /// retained. `None` whenever `atom_economy_status` isn't `Normal`: never
    /// clamped down to fit an expected range (see `atom_economy_raw_percent`
    /// for the unclamped ratio, and `atom_economy_status` for why).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub atom_economy: Option<f64>,
    /// The unclamped MW(target) / Σ MW(precursors) × 100 ratio, populated
    /// whenever both molecular weights are computable regardless of
    /// `atom_economy_status` -- the honest number `atom_economy` is derived
    /// from, kept even when that ratio exceeds the physically-expected
    /// [0, 100] range.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub atom_economy_raw_percent: Option<f64>,
    /// See `AtomEconomyStatus`.
    pub atom_economy_status: AtomEconomyStatus,
    /// Per-step template confidence: rule_weight / max_rule_weight ∈ [0, 1].
    /// Hand-crafted rules (weight=1.0) yield lower values when high-frequency extracted
    /// templates are present; all weights equal → all step_confidence = 1.0.
    pub step_confidence: f64,
    /// Suggested experimental procedure hint for the forward reaction.
    /// Populated for hand-crafted rules; None for extracted templates.
    /// Placeholder for QFANG-style structured procedure generation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub procedure_hint: Option<String>,
    /// Reaction family for this step (e.g. "suzuki_coupling", "esterification").
    /// None for extracted templates that have no manual assignment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reaction_family: Option<String>,
    /// Provenance of `conditions`/`reaction_family`. `None` for extracted templates --
    /// nothing is fabricated for them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_source: Option<MetadataSource>,
    /// Scope at which `metadata_source` was assigned. `None` for extracted templates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_scope: Option<EvidenceScope>,
    /// Curated external evidence (conditions/yields/warnings/references) matched
    /// by `template_id` from an optional metadata sidecar. `None` unless a
    /// sidecar was supplied and matched -- nothing is fabricated for templates
    /// without an entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<StepEvidence>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Route {
    pub steps: Vec<ReactionStep>,
    pub depth: u32,
    /// Cumulative A* step cost (lower = better). Included in JSON output.
    pub score: f64,
    /// Leaf building blocks for this route (precursors not expanded further).
    pub building_blocks: Vec<String>,
    /// Template confidence: min(step template frequency) / max frequency in rule set.
    /// 0 = route uses very rare templates; 1 = all templates are maximally common.
    pub confidence: f64,
    /// Convergency score: 1.0 = all branches same depth (parallel synthesis possible);
    /// 0.0 = purely linear route.
    pub convergency: f64,
    /// Product of step_confidence values (Retro-prob style): a
    /// frequency-derived route ranking score, not a calibrated experimental
    /// success probability -- decays with route length purely because rarer
    /// templates compound, not because of any measured or predicted failure
    /// rate. Single-step: equals step_confidence. Multi-step: decays
    /// multiplicatively.
    pub success_probability: f64,
    /// Estimated synthesis cost: Σ(BB complexity or price) + step_count × 0.5.
    /// Uses SA Score as complexity proxy when no price file is provided.
    /// Lower = cheaper / simpler route.
    pub route_cost: f64,
}

/// Statistics returned alongside routes from [`find_routes`].
#[derive(Debug, Default, Serialize)]
pub struct SearchStats {
    pub nodes_expanded: u64,
    pub max_depth_reached: bool,
    pub beam_limit_hit: bool,
    /// Total template-molecule matches across all expansions.
    pub matched_templates: u64,
    /// Total building-block hits seen in node frontiers.
    pub stock_hits: u64,
    /// retro_cache hits (same intermediate seen before → O(1) reuse).
    pub retro_cache_hits: u64,
    /// retro_cache misses (new intermediate → full apply_retro run).
    pub retro_cache_misses: u64,
    /// Ring-context safety guard counters (Issue #72), accumulated across
    /// every extracted-template application in this search. All-zero
    /// unless `SearchConfig::ring_context_policy` is not `Disabled`.
    pub ring_context_diagnostics: crate::ring_context::RingContextDiagnostics,
}

fn extract_building_blocks(steps: &[ReactionStep]) -> Vec<String> {
    let targets: std::collections::HashSet<&str> =
        steps.iter().map(|s| s.target.as_str()).collect();
    let mut bbs: Vec<String> = steps
        .iter()
        .flat_map(|s| s.precursors.iter())
        .filter(|p| !targets.contains(p.as_str()))
        .cloned()
        .collect();
    bbs.sort_unstable();
    bbs.dedup();
    bbs
}

#[derive(Debug, Clone)]
struct FEntry {
    smiles: String,
}

/// Persistent linked-list node for synthesis path sharing.
/// Children share the parent's prefix via Arc::clone (pointer copy only).
#[derive(Debug, Clone)]
struct PathNode {
    step: ReactionStep,
    prev: Option<Arc<PathNode>>,
}

fn collect_path(mut cur: Option<&Arc<PathNode>>) -> Vec<ReactionStep> {
    let mut steps = Vec::new();
    while let Some(node) = cur {
        steps.push(node.step.clone());
        cur = node.prev.as_ref();
    }
    steps.reverse();
    steps
}

#[derive(Debug, Clone)]
struct Node {
    frontier: SmallVec<[FEntry; 6]>,
    path: Option<Arc<PathNode>>,
    depth: u32,
    g: f64,
    h: f64,
}

impl Node {
    fn f(&self) -> f64 {
        self.g + self.h
    }
}

impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        self.f().to_bits() == other.f().to_bits()
    }
}
impl Eq for Node {}
impl PartialOrd for Node {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Node {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Min-heap by f = g + h (best = lowest cost first).
        other
            .f()
            .partial_cmp(&self.f())
            .unwrap_or(std::cmp::Ordering::Equal)
    }
}

/// Build a bitmask of atomic numbers present in a canonical SMILES string.
/// Conservative: may over-report (false positives) but never under-reports (no false negatives).
/// Used to skip rules whose required elements are absent from the target molecule.
pub(crate) fn elem_mask_from_smiles(smiles: &str) -> u64 {
    const TWO_CHAR: &[(&str, u64)] = &[
        ("Cl", 17),
        ("Br", 35),
        ("Si", 14),
        ("Se", 34),
        ("Te", 52),
        ("Sn", 50),
        ("Zn", 30),
        ("Pd", 46),
        ("Cu", 29),
        ("Fe", 26),
    ];
    const ONE_CHAR: &[(char, u64)] = &[
        ('B', 5),
        ('C', 6),
        ('N', 7),
        ('O', 8),
        ('F', 9),
        ('P', 15),
        ('S', 16),
        ('I', 53),
    ];
    let mut mask: u64 = 0;
    for (sym, an) in TWO_CHAR {
        if smiles.contains(*sym) {
            mask |= 1u64 << an;
        }
    }
    for (ch, an) in ONE_CHAR {
        let lo = ch.to_ascii_lowercase();
        if smiles.chars().any(|c| c == *ch || c == lo) {
            mask |= 1u64 << an;
        }
    }
    mask
}

/// Hash the sorted frontier SMILES into a u64 for closed-set deduplication.
/// Avoids String allocation per node vs. the former join-based state_key.
/// Collision probability is 2^-64 per node pair — negligible in practice.
fn state_hash(frontier: &[FEntry]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut keys: Vec<&str> = frontier.iter().map(|e| e.smiles.as_str()).collect();
    keys.sort_unstable();
    let mut h = FxHasher::default();
    for k in &keys {
        k.hash(&mut h);
    }
    h.finish()
}

fn is_bb(smiles: &str, env: &ChemEnv) -> bool {
    // Fast path: direct HashSet lookup (FEntry.smiles is always canonical SMILES).
    if env.is_building_block_smiles(smiles) {
        return true;
    }
    // Slow path: re-parse and re-standardize under the same shared
    // stock-identity policy `ChemEnv` itself uses. Exact identity only (no
    // subgraph matching) — see `chem_env::canonical_stock_identity`.
    canonical_stock_identity_from_smiles(smiles)
        .map(|canon| env.is_building_block_smiles(&canon))
        .unwrap_or(false)
}

/// Pluggable molecule value estimator for the A* heuristic (Retro*-style).
///
/// Returns the estimated synthesis cost for a SMILES string (≥ 0.0; higher = harder).
/// The default implementation uses SA Score. Implement this trait to plug in a neural
/// value function without changing the search algorithm.
pub trait MoleculeValueEstimator: Send + Sync {
    fn estimate_cost(&self, smiles: &str) -> f64;
}

/// Default estimator: SA Score-based heuristic (h ∈ [1.0, 1.5] per unsolved molecule).
/// Admissible because step_cost ≥ 1.0 per step, so h ≤ 1.5 < true cost.
pub struct SaScoreEstimator;

impl MoleculeValueEstimator for SaScoreEstimator {
    fn estimate_cost(&self, smiles: &str) -> f64 {
        let v = mol_from_smiles(smiles)
            .map(|m| sa_score(&m).clamp(1.0, 10.0))
            .unwrap_or(5.5);
        1.0 + 0.5 * (v - 1.0) / 9.0
    }
}

/// Pluggable template prior for A* expansion scoring (Retro*-style).
///
/// Returns a bonus ≥ 0.0: how relevant `template_name` is for expanding `target_smiles`.
/// Higher bonus → smaller effective step cost → template is tried earlier in A\* search.
/// The default implementation (`FrequencyPrior`) uses log-frequency from training data.
pub trait ReactionPrior: Send + Sync {
    fn prior(&self, template_name: &str, target_smiles: &str) -> f64;
}

/// Default prior: log-frequency weight from USPTO training data (same as pre-v0.9 behavior).
///
/// `weight = ln(count + 1)` for extracted templates; hand-crafted rules use `weight = 1.0`.
/// The bonus is `template_bonus(weight, max_weight)` ∈ [0.0, 0.2].
pub struct FrequencyPrior {
    pub rule_weights: std::collections::HashMap<String, f64>,
    pub max_weight: f64,
}

impl FrequencyPrior {
    pub fn from_rules(rules: &[RetroRule]) -> Self {
        let max_weight = rules.iter().map(|r| r.weight).fold(1.0_f64, f64::max);
        let rule_weights = rules.iter().map(|r| (r.name.clone(), r.weight)).collect();
        Self {
            rule_weights,
            max_weight,
        }
    }
}

impl ReactionPrior for FrequencyPrior {
    fn prior(&self, template_name: &str, _target_smiles: &str) -> f64 {
        let w = self.rule_weights.get(template_name).copied().unwrap_or(1.0);
        template_bonus(w, self.max_weight)
    }
}

fn compute_h(
    frontier: &[FEntry],
    env: &ChemEnv,
    sa_cache: &mut FxHashMap<String, f64>,
    estimator: Option<&std::sync::Arc<dyn MoleculeValueEstimator>>,
) -> f64 {
    frontier
        .iter()
        .filter(|e| !is_bb(&e.smiles, env))
        .map(|e| {
            if let Some(est) = estimator {
                return est.estimate_cost(&e.smiles);
            }
            // Default: SA Score (cached)
            if let Some(&v) = sa_cache.get(&e.smiles) {
                return 1.0 + 0.5 * (v - 1.0) / 9.0;
            }
            let v = mol_from_smiles(&e.smiles)
                .map(|m| sa_score(&m).clamp(1.0, 10.0))
                .unwrap_or(5.5);
            sa_cache.insert(e.smiles.clone(), v);
            1.0 + 0.5 * (v - 1.0) / 9.0
        })
        .sum()
}

/// Classify a rule name into a human-readable reaction family.
/// Hand-crafted rules only; extracted templates return None.
fn reaction_family_for_rule(rule: &str) -> Option<&'static str> {
    match rule {
        "ester_cleavage" => Some("esterification"),
        "amide_cleavage" => Some("amide_coupling"),
        "friedel_crafts_acylation_retro" => Some("friedel_crafts_acylation"),
        "aryl_carboxylation_retro" => Some("decarboxylation"),
        "buchwald_hartwig_retro" => Some("buchwald_hartwig"),
        "aryl_amine_retro" => Some("chan_lam_coupling"),
        "aryl_ether_retro" => Some("ullmann_ether"),
        // aryl_chloride_retro / aryl_iodide_retro / aryl_fluoride_snAr_retro
        // removed from default_rules() (31.11, chem_env.rs) — atom-loss bug,
        // no tracked reagent. Arms deleted so this stays dead-code-free.
        "aryl_chloride_to_bromide" => Some("halogen_exchange"),
        "suzuki_retro" => Some("suzuki_coupling"),
        "heck_retro" | "heck_retro_terminal" => Some("heck_reaction"),
        "negishi_retro" => Some("negishi_coupling"),
        "wittig_retro" => Some("wittig_reaction"),
        "reductive_amination_retro" => Some("reductive_amination"),
        "sonogashira_retro" => Some("sonogashira_coupling"),
        "sulfonamide_retro" => Some("sulfonamide_formation"),
        "diaryl_sulfone_retro" => Some("friedel_crafts_sulfonylation"),
        "boc_deprotection_retro" => Some("boc_deprotection"),
        "cbz_deprotection_retro" => Some("cbz_deprotection"),
        "n_benzylation_retro" => Some("n_benzylation"),
        "grignard_addition_retro" => Some("grignard_addition"),
        "claisen_retro" => Some("claisen_condensation"),
        "michael_retro" => Some("michael_addition"),
        "acyl_chloride_from_acid" => Some("acyl_chloride_formation"),
        "alcohol_oxidation_retro" => Some("carbonyl_reduction"),
        _ => None,
    }
}

/// True iff `rule` came from `chem_env::load_rules_from_file` (always named
/// `extracted_{i}`) rather than `default_rules()`. Used for `metadata_source`/
/// `metadata_scope` tagging (unchanged since PR #48): `conditions_for_rule`/
/// `reaction_family_for_rule` both return `None` for 3 legitimately-hand-crafted
/// generic-cleavage rules, so `.is_some()` on either would mis-tag those 3 as
/// extracted. (`RetroRule::template_id`'s `rule:`/`smirks-sha256:` prefix is
/// also a reliable discriminator now, but this function's name-prefix check
/// is kept as-is to avoid changing existing tagging behavior.)
pub(crate) fn is_extracted_template(rule: &str) -> bool {
    rule.starts_with("extracted_")
}

/// Rule-based reaction conditions for hand-crafted retro rules.
/// Returns None for extracted templates (conditions unknown without ML).
fn conditions_for_rule(rule: &str) -> Option<ReactionConditions> {
    macro_rules! cond {
        ($cat:expr, $sol:expr, $tmp:expr) => {
            Some(ReactionConditions {
                catalyst: Some($cat.into()),
                solvent: Some($sol.into()),
                temperature: Some($tmp.into()),
                notes: None,
            })
        };
        ($cat:expr, $sol:expr, $tmp:expr, $note:expr) => {
            Some(ReactionConditions {
                catalyst: Some($cat.into()),
                solvent: Some($sol.into()),
                temperature: Some($tmp.into()),
                notes: Some($note.into()),
            })
        };
    }
    match rule {
        "ester_cleavage" => cond!("NaOH or LiOH (2 eq)", "THF/H₂O (2:1)", "rt → 60 °C"),
        "amide_cleavage" => cond!("LiOH (3 eq)", "THF/H₂O (3:1)", "60 °C"),
        "friedel_crafts_acylation_retro" => cond!("AlCl₃ (1.2 eq)", "DCM", "0 °C → rt"),
        "aryl_carboxylation_retro" => {
            cond!("none", "water", "150 °C", "Kolbe-Schmitt / decarboxylation")
        }
        "buchwald_hartwig_retro" => cond!("Pd₂(dba)₃ / XPhos (5 mol%)", "toluene", "100 °C"),
        "aryl_amine_retro" => cond!("Cu(OAc)₂ / pyridine", "DCM", "rt", "Chan-Lam retro"),
        "aryl_ether_retro" => cond!("Cs₂CO₃ (2 eq)", "DMF", "110 °C", "Ullmann ether retro"),
        "aryl_chloride_to_bromide" => cond!("NaBr (excess)", "DMF", "120 °C", "halogen exchange"),
        "suzuki_retro" => cond!("Pd(PPh₃)₄ (5 mol%)", "EtOH/H₂O (3:1)", "80 °C"),
        "heck_retro" => cond!("Pd(OAc)₂ / PPh₃ (5 mol%)", "DMF", "100 °C"),
        "heck_retro_terminal" => cond!("Pd(OAc)₂ / PPh₃ (5 mol%)", "DMF", "100 °C"),
        "negishi_retro" => cond!("Pd(PPh₃)₄ / ZnCl₂", "THF", "65 °C"),
        "cc_single_cleavage" => None, // retrosynthetic disconnection only
        "wittig_retro" => cond!("Ph₃P (1.2 eq)", "toluene", "0 °C → rt"),
        "reductive_amination_retro" => cond!("NaBH₃CN (1.5 eq)", "MeOH", "rt"),
        "cn_aliphatic_cleavage" => None,
        "co_aliphatic_cleavage" => None,
        "alcohol_oxidation_retro" => {
            cond!("NaBH₄ (1.2 eq)", "EtOH", "0 °C → rt", "retro = reduction")
        }
        "sonogashira_retro" => cond!("Pd(PPh₃)₂Cl₂ / CuI (5 mol%)", "Et₃N", "60 °C"),
        "sulfonamide_retro" => cond!("Et₃N (2 eq)", "DCM", "0 °C → rt"),
        "diaryl_sulfone_retro" => cond!(
            "AlCl₃ (1.2 eq)",
            "DCM",
            "0 °C → rt",
            "Friedel-Crafts sulfonylation"
        ),
        "boc_deprotection_retro" => cond!("TFA (20 % in DCM)", "DCM", "rt"),
        "n_benzylation_retro" => cond!("K₂CO₃ (2 eq)", "DMF", "60 °C"),
        "grignard_addition_retro" => cond!("Mg (1.1 eq)", "THF (dry)", "0 °C → rt"),
        "claisen_retro" => cond!("LDA (2.0 eq)", "THF (dry)", "−78 °C"),
        "michael_retro" => cond!("DBU or K₂CO₃ (1.2 eq)", "THF", "rt"),
        "acyl_chloride_from_acid" => cond!("(COCl)₂ (1.2 eq) + cat. DMF", "DCM", "0 °C → rt"),
        "cbz_deprotection_retro" => cond!("H₂ (1 atm), Pd/C (10 %)", "EtOH", "rt"),
        _ => None,
    }
}

/// One-line experimental procedure hint for hand-crafted retro rules (forward direction).
/// Placeholder infrastructure for QFANG-style structured procedure generation.
fn procedure_hint_for_rule(rule: &str) -> Option<&'static str> {
    match rule {
        "ester_cleavage" => {
            Some("Dissolve in THF/H₂O, add NaOH (2 eq), stir at 60 °C, acidify to pH 2.")
        }
        "amide_cleavage" => Some("Reflux in 6M HCl or add LiOH (3 eq) in THF/H₂O at 60 °C."),
        "friedel_crafts_acylation_retro" => {
            Some("Add acid chloride to arene + AlCl₃ (1.2 eq) in DCM at 0 °C, warm to rt.")
        }
        "buchwald_hartwig_retro" => {
            Some("Combine aryl halide + amine + Pd₂(dba)₃/XPhos in toluene, heat at 100 °C.")
        }
        "aryl_ether_retro" => {
            Some("Mix aryl halide + phenol + Cs₂CO₃ (2 eq) in DMF, heat at 110 °C.")
        }
        "suzuki_retro" => {
            Some("Combine aryl boronate + aryl halide + Pd(PPh₃)₄ in EtOH/H₂O, reflux at 80 °C.")
        }
        "heck_retro" | "heck_retro_terminal" => {
            Some("Add alkene + aryl halide + Pd(OAc)₂/PPh₃ in DMF with Et₃N at 100 °C.")
        }
        "wittig_retro" => {
            Some("Add aldehyde to Ph₃P=CHR (Wittig ylide) in toluene at 0 °C, warm to rt.")
        }
        "reductive_amination_retro" => {
            Some("Mix aldehyde + amine in MeOH, add NaBH₃CN (1.5 eq), stir at rt.")
        }
        "sonogashira_retro" => {
            Some("Combine terminal alkyne + aryl halide + Pd/CuI in Et₃N at 60 °C.")
        }
        "sulfonamide_retro" => Some("Add sulfonyl chloride to amine + Et₃N (2 eq) in DCM at 0 °C."),
        "boc_deprotection_retro" => {
            Some("Treat with TFA (20% in DCM) at rt for 1 h, then evaporate.")
        }
        "cbz_deprotection_retro" => Some("Hydrogenate (H₂, 1 atm) over Pd/C (10%) in EtOH at rt."),
        "grignard_addition_retro" => {
            Some("Add carbonyl to Grignard reagent in dry THF at 0 °C, then rt; quench with NH₄Cl.")
        }
        "acyl_chloride_from_acid" => {
            Some("Add oxalyl chloride (1.2 eq) + cat. DMF to carboxylic acid in DCM at 0 °C.")
        }
        "alcohol_oxidation_retro" => {
            Some("Reduce ketone/aldehyde with NaBH₄ (1.2 eq) in EtOH at 0 °C → rt.")
        }
        "claisen_retro" => Some(
            "Deprotonate ester α-position with LDA (2 eq) in dry THF at −78 °C, add electrophile.",
        ),
        "michael_retro" => {
            Some("Combine Michael donor + acceptor + K₂CO₃ or DBU (1.2 eq) in THF at rt.")
        }
        "n_benzylation_retro" => {
            Some("React amine + benzyl halide + K₂CO₃ (2 eq) in DMF at 60 °C.")
        }
        _ => None,
    }
}

/// Convergency score for a route: 1.0 = all leaf branches same depth (ideal parallel
/// synthesis); 0.0 = purely linear. Computed from depth of each leaf in the step tree.
fn convergency_score(steps: &[ReactionStep]) -> f64 {
    if steps.is_empty() {
        return 1.0;
    }
    // BFS: assign depth to every molecule in the tree.
    let mut depth_map: rustc_hash::FxHashMap<&str, u32> = rustc_hash::FxHashMap::default();
    if let Some(first) = steps.first() {
        depth_map.insert(first.target.as_str(), 0);
    }
    for step in steps {
        let d = depth_map.get(step.target.as_str()).copied().unwrap_or(0);
        for prec in &step.precursors {
            depth_map.entry(prec.as_str()).or_insert(d + 1);
        }
    }
    let targets: rustc_hash::FxHashSet<&str> = steps.iter().map(|s| s.target.as_str()).collect();
    let leaf_depths: Vec<u32> = depth_map
        .iter()
        .filter(|(k, _)| !targets.contains(*k))
        .map(|(_, &v)| v)
        .collect();
    if leaf_depths.len() <= 1 {
        return 1.0;
    }
    let max = leaf_depths.iter().copied().max().unwrap_or(0) as f64;
    let min = leaf_depths.iter().copied().min().unwrap_or(0) as f64;
    if max == 0.0 {
        1.0
    } else {
        1.0 - (max - min) / max
    }
}

/// Estimate synthesis cost for a route.
///
/// `Σ(BB complexity or price) + step_count × 0.5`
///
/// BB cost: price from `prices` map if available; otherwise SA Score (1–10 scale).
/// Lower values indicate cheaper / simpler routes.
fn compute_route_cost(
    route: &Route,
    prices: Option<&std::collections::HashMap<String, f64>>,
) -> f64 {
    use chematic::chem::sa_score;

    let bb_cost: f64 = route
        .building_blocks
        .iter()
        .map(|smiles| {
            if let Some(map) = prices
                && let Some(&p) = map.get(smiles.as_str())
            {
                return p;
            }
            mol_from_smiles(smiles)
                .ok()
                .map(|m| sa_score(&m))
                .unwrap_or(5.0)
        })
        .sum();
    bb_cost + route.steps.len() as f64 * 0.5
}

/// Prune the heap to at most `beam_width` nodes (keep the best).
/// Uses sort_unstable_by (lower constant than sort_by) for deterministic ordering.
fn beam_prune(heap: &mut BinaryHeap<Node>, beam_width: usize) {
    if beam_width == 0 || heap.len() <= beam_width {
        return;
    }
    let mut nodes: Vec<Node> = heap.drain().collect();
    nodes.sort_unstable_by(|a, b| {
        a.f()
            .partial_cmp(&b.f())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    nodes.truncate(beam_width);
    *heap = nodes.into_iter().collect();
}

pub struct SearchConfig {
    pub max_depth: u32,
    pub max_routes: usize,
    /// 0 = unlimited (pure A*). N > 0 = beam search, keep top-N nodes.
    pub beam_width: usize,
    /// Element bitmask (same format as `RetroRule::required_elements`).
    /// Routes whose leaf building blocks contain any forbidden element are dropped.
    /// 0 = no constraint.
    pub forbidden_elements: u64,
    /// Routes are kept only when the union of all leaf BB element masks covers this mask.
    /// 0 = no constraint.
    pub required_element_present: u64,
    /// Print search statistics (nodes expanded, elapsed time) to stderr after search.
    pub verbose: bool,
    /// Bond-center template index (RetroKNN-inspired).
    /// When true, only templates whose SMIRKS bond pairs match bonds present in
    /// the target molecule are tried. Graph-based and fallback rules are always included.
    /// Typically gives ~24% speedup over the full template set with no accuracy loss.
    pub bond_index: bool,
    /// Optional building block price map: canonical SMILES → price per gram.
    /// When Some, route_cost uses these prices; unmatched BBs fall back to SA Score.
    /// When None, route_cost uses SA Score for all BBs.
    pub bb_price_map: Option<std::collections::HashMap<String, f64>>,
    /// Custom molecule value estimator for the A* heuristic.
    /// None = use `SaScoreEstimator` (default SA Score-based behaviour).
    pub value_estimator: Option<std::sync::Arc<dyn MoleculeValueEstimator>>,
    /// Custom reaction prior for template scoring.
    /// None = use `FrequencyPrior` (log-frequency weighting, same as pre-v0.9 behaviour).
    pub reaction_prior: Option<std::sync::Arc<dyn ReactionPrior>>,
    /// Optional template metadata sidecar (`--template-metadata` / Python
    /// `template_metadata_path`), keyed by `RetroRule::template_id`. When Some,
    /// matching steps get `evidence` populated in post-processing; unmatched
    /// templates are left as `None` -- nothing is fabricated. `None` (the
    /// default) reproduces pre-existing search behaviour exactly.
    pub template_metadata: Option<std::collections::HashMap<String, TemplateMetadataEntry>>,
    /// Phase B: ONNX template relevance scorer (CLI/Python only, not WASM).
    /// When Some, pre-filters rules to top-K most relevant before SMARTS matching.
    #[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
    pub nn_scorer: Option<std::sync::Arc<crate::scorer::nn::TemplateScorer>>,
    /// Ring-context safety guard configuration (Issue #72). `Disabled` (the
    /// default) reproduces pre-existing behaviour exactly -- extracted
    /// templates are applied via the unmodified legacy `apply_retro` path,
    /// no sidecar is required. `Guarded` always carries a loaded guard
    /// alongside its enforcement policy; "enforce without a guard" is not a
    /// state this type can represent.
    pub ring_context: crate::ring_context::RingContextConfig,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            max_depth: 5,
            max_routes: 5,
            beam_width: 0,
            forbidden_elements: 0,
            required_element_present: 0,
            verbose: false,
            bond_index: false,
            bb_price_map: None,
            value_estimator: None,
            reaction_prior: None,
            template_metadata: None,
            #[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
            nn_scorer: None,
            ring_context: crate::ring_context::RingContextConfig::Disabled,
        }
    }
}

/// Per-node NN template ranking (Phase D). `None` when no scorer is configured,
/// and always `None` on WASM / without the `nn-scoring` feature — callers must
/// fall back to `ranked_rules`/`bond_idx`, preserving the existing WASM
/// frequency/bond-index-only retrieval path unchanged.
#[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
fn nn_rank<'a>(
    config: &SearchConfig,
    rules: &'a [RetroRule],
    smiles: &str,
) -> Option<Vec<&'a RetroRule>> {
    config.nn_scorer.as_ref().map(|sc| {
        sc.top_k_indices(smiles, rules.len())
            .into_iter()
            .filter_map(|i| rules.get(i))
            .collect()
    })
}
#[cfg(not(all(not(target_arch = "wasm32"), feature = "nn-scoring")))]
fn nn_rank<'a>(
    _config: &SearchConfig,
    _rules: &'a [RetroRule],
    _smiles: &str,
) -> Option<Vec<&'a RetroRule>> {
    None
}

pub fn find_routes(
    target_smiles: &str,
    env: &ChemEnv,
    rules: &[RetroRule],
    config: &SearchConfig,
) -> Result<(Vec<Route>, SearchStats)> {
    let target_mol = mol_from_smiles(target_smiles)?;
    let target_canonical = to_canonical(&target_mol);

    // Default rule order when no scorer/bond-index retrieval narrows it down.
    // Phase D (2026-07): the NN scorer used to rank ONCE against the root target
    // and reuse that order for every deeper intermediate. Measurement (986 solved
    // targets, 994 depth>=1 ground-truth steps) showed that's a poor proxy for
    // what's actually applicable at an intermediate: top-100 recall of the
    // ground-truth rule was 37.1% under root-only ranking vs 64.1% re-ranked
    // fresh on the intermediate (median rank 304 -> 27). So scoring now happens
    // per-node, right below in the retro_cache-miss branch — which already keys
    // on canonical intermediate SMILES, so each unique intermediate still gets
    // exactly one ONNX call for the whole search (cache hits skip it entirely).
    let ranked_rules: Vec<&RetroRule> = rules.iter().collect();

    let max_rule_weight = rules.iter().map(|r| r.weight).fold(1.0_f64, f64::max);

    // Bond-center template index — built once, queried per-expansion (O(bonds) per node).
    let bond_idx: Option<TemplateBondIndex> = if config.bond_index {
        Some(TemplateBondIndex::build(rules))
    } else {
        None
    };

    #[cfg(not(target_arch = "wasm32"))]
    let t0 = std::time::Instant::now();
    #[cfg(not(target_arch = "wasm32"))]
    let mut nodes_popped: u64 = 0;
    let mut nodes_expanded: u64 = 0;
    let mut max_depth_reached = false;
    let mut beam_limit_hit = false;
    let mut matched_templates: u64 = 0;
    let mut stock_hits: u64 = 0;
    let mut retro_cache_hits: u64 = 0;
    let mut ring_context_diagnostics = crate::ring_context::RingContextDiagnostics::default();
    let mut retro_cache_misses: u64 = 0;

    let mut routes: Vec<Route> = Vec::new();
    let mut closed: FxHashSet<u64> = FxHashSet::default();
    let mut heap: BinaryHeap<Node> = BinaryHeap::new();
    let mut sa_cache: FxHashMap<String, f64> = FxHashMap::default();
    // Opt-D: per-search memoization of apply_retro results.
    // Key: canonical target SMILES. Value: Arc-wrapped filtered expansions.
    // Arc avoids full-Vec cloning on both hit (O(1) Arc::clone) and miss (no extra clone).
    let mut retro_cache: RetroCache = FxHashMap::default();

    let initial: SmallVec<[FEntry; 6]> = smallvec![FEntry {
        smiles: target_canonical,
    }];
    let h0 = compute_h(
        &initial,
        env,
        &mut sa_cache,
        config.value_estimator.as_ref(),
    );
    heap.push(Node {
        frontier: initial,
        path: None,
        depth: 0,
        g: 0.0,
        h: h0,
    });

    while let Some(node) = heap.pop() {
        #[cfg(not(target_arch = "wasm32"))]
        {
            nodes_popped += 1;
        }
        if routes.len() >= config.max_routes {
            break;
        }

        // Single pass: count unsolved + find first unsolved entry simultaneously.
        let mut n_unsolved = 0usize;
        let mut first_unsolved: Option<&FEntry> = None;
        for e in node.frontier.iter() {
            if !is_bb(&e.smiles, env) {
                n_unsolved += 1;
                if first_unsolved.is_none() {
                    first_unsolved = Some(e);
                }
            } else {
                stock_hits += 1;
            }
        }

        if n_unsolved == 0 {
            let steps = collect_path(node.path.as_ref());
            let building_blocks = extract_building_blocks(&steps);
            routes.push(Route {
                steps,
                depth: node.depth,
                score: node.g,
                building_blocks,
                confidence: 0.0,          // computed below
                convergency: 0.0,         // computed below
                success_probability: 0.0, // computed below
                route_cost: 0.0,          // computed below
            });
        }

        if node.depth >= config.max_depth {
            max_depth_reached = true;
            continue;
        }

        let key = state_hash(&node.frontier);
        if closed.contains(&key) {
            continue;
        }
        closed.insert(key);
        #[cfg(not(target_arch = "wasm32"))]
        {
            nodes_expanded += 1;
        }

        let Some(target_entry) = first_unsolved.or_else(|| node.frontier.first()) else {
            continue;
        };
        let target_smi = target_entry.smiles.clone();

        let Ok(target_mol) = mol_from_smiles(&target_smi) else {
            continue;
        };

        // Opt-D: look up the memoized expansion for this target molecule.
        // On cache miss: run apply_retro in parallel (native) / sequential (WASM),
        // filter invalid results, precompute net step cost, and store.
        // On cache hit: O(1) Arc::clone — no Vec data is copied.
        let expansions: Arc<Vec<RetroEntry>> = if let Some(cached) = retro_cache.get(&target_smi) {
            retro_cache_hits += 1;
            Arc::clone(cached) // O(1): pointer copy only, no Vec clone
        } else {
            retro_cache_misses += 1;
            // Bond-center retrieval: filter ranked_rules to those relevant to this molecule's bonds.
            // Else, per-node NN ranking (Phase D) — scored fresh against THIS intermediate,
            // not the root; this whole branch only runs once per unique canonical
            // `target_smi` (retro_cache dedupes repeat visits), so it's exactly one ONNX
            // inference call per unique intermediate for the whole search, same as a
            // dedicated SMILES-keyed cache would give, with no extra cache to maintain.
            // Falls back to ranked_rules unchanged when neither is configured.
            let retrieved: Vec<&RetroRule>;
            let per_node: Vec<&RetroRule>;
            let active_rules: &[&RetroRule] = if let Some(ref idx) = bond_idx {
                retrieved = idx
                    .retrieve(&target_mol, 0, rules) // top_k=0 = no truncation
                    .into_iter()
                    .filter_map(|i| rules.get(i))
                    .collect();
                &retrieved
            } else if let Some(v) = nn_rank(config, rules, &target_smi) {
                per_node = v;
                &per_node
            } else {
                &ranked_rules
            };

            // Shared with the standalone `propose_one_step` candidate-pool API
            // (`crate::candidate::raw_propose`) so route search and offline
            // candidate generation apply the exact same rule-application
            // logic -- this must stay a call, not a re-inlined copy.
            // find_routes' own active-rule selection (above) is *not* a
            // ProposalMode -- it has its own bond_idx/nn_rank/ranked_rules
            // fallback chain, including per-node NN re-ranking that
            // ProposalMode::ScorerConditioned deliberately does not
            // reproduce (see candidate module doc) -- so these scores are
            // marked NotApplicable rather than reusing UpstreamScoreStatus's
            // Available variant, which is reserved for candidate-pool
            // generation going through an explicit ProposalMode.
            let scored_active_rules: Vec<crate::candidate::ScoredRuleRef<'_>> = active_rules
                .iter()
                .enumerate()
                .map(|(rank, &rule)| crate::candidate::ScoredRuleRef {
                    rule,
                    source_rank: rank,
                    upstream_score: None,
                    upstream_score_status: crate::candidate::UpstreamScoreStatus::NotApplicable,
                })
                .collect();
            let (raw_proposals, step_ring_diag) = crate::candidate::raw_propose(
                &target_mol,
                &target_smi,
                &scored_active_rules,
                crate::ring_context::RingContextArgs {
                    config: config.ring_context.clone(),
                },
            );
            ring_context_diagnostics.merge(&step_ring_diag);

            let entries: Vec<RetroEntry> = raw_proposals
                .into_iter()
                .map(|p| {
                    let bonus = if let Some(ref prior) = config.reaction_prior {
                        prior.prior(&p.rule_name, &target_smi)
                    } else {
                        template_bonus(p.rule_weight, max_rule_weight)
                    };
                    let step_c =
                        step_cost(&p.precursors.iter().map(|pm| &pm.mol).collect::<Vec<_>>())
                            - bonus;
                    let smiles_list: Vec<String> =
                        p.precursors.iter().map(|pm| pm.smiles.clone()).collect();
                    RetroEntry {
                        rule_name: p.rule_name,
                        template_id: p.template_id,
                        step_cost: step_c,
                        precursor_smiles: smiles_list,
                    }
                })
                .collect();
            let arc = Arc::new(entries);
            retro_cache.insert(target_smi.clone(), Arc::clone(&arc));
            arc // no extra clone: Arc move
        };

        matched_templates += expansions.len() as u64;

        for entry in expansions.iter() {
            let new_frontier: SmallVec<[FEntry; 6]> = node
                .frontier
                .iter()
                .filter(|e| e.smiles != target_smi)
                .cloned()
                .chain(
                    entry
                        .precursor_smiles
                        .iter()
                        .map(|s| FEntry { smiles: s.clone() }),
                )
                .collect();

            let new_h = compute_h(
                &new_frontier,
                env,
                &mut sa_cache,
                config.value_estimator.as_ref(),
            );

            // O(1) Arc::clone — shares the parent prefix without copying.
            let new_path = Some(Arc::new(PathNode {
                step: ReactionStep {
                    rule: entry.rule_name.clone(),
                    template_id: entry.template_id.clone(),
                    target: target_smi.clone(),
                    precursors: entry.precursor_smiles.clone(),
                    conditions: conditions_for_rule(&entry.rule_name),
                    atom_economy: None,             // populated in post-processing
                    atom_economy_raw_percent: None, // populated in post-processing
                    atom_economy_status: AtomEconomyStatus::NotEvaluable, // populated in post-processing
                    step_confidence: 0.0, // populated in post-processing
                    reaction_family: reaction_family_for_rule(&entry.rule_name).map(str::to_string),
                    procedure_hint: procedure_hint_for_rule(&entry.rule_name).map(str::to_string),
                    metadata_source: (!is_extracted_template(&entry.rule_name))
                        .then_some(MetadataSource::HandcraftedDefault),
                    metadata_scope: (!is_extracted_template(&entry.rule_name))
                        .then_some(EvidenceScope::ReactionFamily),
                    evidence: None, // populated in post-processing, routes actually returned only
                },
                prev: node.path.clone(),
            }));

            // In-search pruning: skip expansions where a BB-precursor contains a
            // forbidden element. Avoids pushing dead-end nodes onto the heap.
            if config.forbidden_elements != 0 {
                let mask = config.forbidden_elements;
                if entry
                    .precursor_smiles
                    .iter()
                    .filter(|p| is_bb(p, env))
                    .any(|p| (elem_mask_from_smiles(p) & mask) != 0)
                {
                    continue;
                }
            }

            heap.push(Node {
                frontier: new_frontier,
                path: new_path,
                depth: node.depth + 1,
                g: node.g + entry.step_cost,
                h: new_h,
            });
        }

        // --- Phase 3.2: Beam search pruning ---
        if config.beam_width > 0 && heap.len() > config.beam_width {
            beam_limit_hit = true;
        }
        beam_prune(&mut heap, config.beam_width);
    }

    // Post-processing: confidence, atom economy, convergency.
    {
        let rule_weights: FxHashMap<&str, f64> =
            rules.iter().map(|r| (r.name.as_str(), r.weight)).collect();
        for route in &mut routes {
            let min_w = route
                .steps
                .iter()
                .map(|s| rule_weights.get(s.rule.as_str()).copied().unwrap_or(1.0))
                .fold(f64::INFINITY, f64::min);
            route.confidence = if min_w.is_infinite() {
                1.0
            } else {
                (min_w / max_rule_weight).clamp(0.0, 1.0)
            };

            for step in &mut route.steps {
                let w = rule_weights.get(step.rule.as_str()).copied().unwrap_or(1.0);
                step.step_confidence = (w / max_rule_weight).clamp(0.0, 1.0);

                let raw = compute_atom_economy_raw(&step.target, &step.precursors);
                let (status, display) = classify_atom_economy(raw);
                step.atom_economy_raw_percent = raw;
                step.atom_economy_status = status;
                step.atom_economy = display;

                step.evidence = config
                    .template_metadata
                    .as_ref()
                    .and_then(|m| m.get(&step.template_id))
                    .and_then(|e| e.to_step_evidence(&step.target, &step.precursors));
            }

            route.success_probability = route
                .steps
                .iter()
                .map(|s| s.step_confidence)
                .product::<f64>()
                .clamp(0.0, 1.0);

            route.convergency = convergency_score(&route.steps);
            route.route_cost = compute_route_cost(route, config.bb_price_map.as_ref());
        }
    }

    if config.forbidden_elements != 0 {
        let mask = config.forbidden_elements;
        routes.retain(|route| {
            let all_targets: std::collections::HashSet<&str> =
                route.steps.iter().map(|s| s.target.as_str()).collect();
            route.steps.iter().all(|step| {
                step.precursors.iter().all(|prec| {
                    all_targets.contains(prec.as_str()) || (elem_mask_from_smiles(prec) & mask) == 0
                })
            })
        });
    }

    #[cfg(not(target_arch = "wasm32"))]
    if config.verbose {
        eprintln!(
            "[renkin] search complete\n  nodes popped   : {}\n  nodes expanded : {}\n  routes found   : {}\n  retro cache    : {}/{} hits ({:.0}%)\n  elapsed        : {:.2} s",
            nodes_popped,
            nodes_expanded,
            routes.len(),
            retro_cache_hits,
            retro_cache_hits + retro_cache_misses,
            if retro_cache_hits + retro_cache_misses > 0 {
                retro_cache_hits as f64 / (retro_cache_hits + retro_cache_misses) as f64 * 100.0
            } else {
                0.0
            },
            t0.elapsed().as_secs_f64()
        );
        if !matches!(
            config.ring_context,
            crate::ring_context::RingContextConfig::Disabled
        ) {
            eprintln!(
                "[renkin] ring_context_diagnostics: {}",
                serde_json::to_string(&ring_context_diagnostics).unwrap_or_default()
            );
        }
    }

    if config.required_element_present != 0 {
        let need = config.required_element_present;
        routes.retain(|route| {
            let all_targets: std::collections::HashSet<&str> =
                route.steps.iter().map(|s| s.target.as_str()).collect();
            let leaf_union: u64 = route
                .steps
                .iter()
                .flat_map(|s| s.precursors.iter())
                .filter(|p| !all_targets.contains(p.as_str()))
                .fold(0u64, |acc, p| acc | elem_mask_from_smiles(p));
            (leaf_union & need) == need
        });
    }

    Ok((
        routes,
        SearchStats {
            nodes_expanded,
            max_depth_reached,
            beam_limit_hit,
            matched_templates,
            stock_hits,
            retro_cache_hits,
            retro_cache_misses,
            ring_context_diagnostics,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chem_env::apply_retro;
    use crate::chem_env::{ChemEnv, default_rules};

    fn aspirin_env() -> ChemEnv {
        ChemEnv::load("data/building_blocks.smi").unwrap_or_else(|_| {
            ChemEnv::in_memory(&["CC(=O)O", "Oc1ccccc1C(=O)O", "c1ccccc1C(=O)O", "C", "O"])
        })
    }

    fn cfg(depth: u32) -> SearchConfig {
        SearchConfig {
            max_depth: depth,
            max_routes: 5,
            beam_width: 0,
            ..Default::default()
        }
    }

    #[test]
    fn aspirin_finds_route_depth1() {
        let env = aspirin_env();
        let rules = default_rules();
        let routes = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg(3))
            .unwrap()
            .0;
        assert!(
            !routes.is_empty(),
            "must find at least one route for aspirin"
        );
        assert!(
            routes.iter().any(|r| r.depth <= 2),
            "must find a route with depth ≤ 2"
        );
    }

    #[test]
    fn building_block_target_returns_depth0() {
        let env = aspirin_env();
        let rules = default_rules();
        // Acetic acid is a building block → expect a depth-0 route (empty steps).
        let routes = find_routes("CC(=O)O", &env, &rules, &cfg(2)).unwrap().0;
        assert!(
            routes.iter().any(|r| r.depth == 0),
            "building block must return depth-0 route"
        );
    }

    #[test]
    fn anthranilic_acid_recognized_as_bb() {
        let env = aspirin_env();
        let rules = default_rules();
        let routes = find_routes("c1ccc(N)cc1C(=O)O", &env, &rules, &cfg(3))
            .unwrap()
            .0;
        assert!(
            routes.iter().any(|r| r.depth == 0),
            "anthranilic acid is in building blocks"
        );
    }

    #[test]
    fn beam_width_limits_does_not_panic() {
        let env = aspirin_env();
        let rules = default_rules();
        let cfg_beam = SearchConfig {
            max_depth: 3,
            max_routes: 3,
            beam_width: 10,
            ..Default::default()
        };
        // With a very tight beam, search may find fewer routes but must not panic.
        let routes = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg_beam);
        assert!(routes.is_ok());
    }

    #[test]
    fn no_routes_for_unknown_target_within_depth() {
        let env = ChemEnv::in_memory(&["O"]); // only water as BB
        let rules = default_rules();
        // Aspirin with depth=1 and only water as BB: unlikely to fully solve.
        // At minimum should return the trivially solved (depth=0) only if aspirin IS water (it isn't).
        let routes = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg(1))
            .unwrap()
            .0;
        // depth=0 not possible (aspirin ≠ water); we just check it doesn't panic.
        let _ = routes;
    }

    // ── Layer 3: search behaviour tests ──────────────────────────────────────

    #[test]
    fn invalid_smiles_returns_err() {
        let env = aspirin_env();
        let rules = default_rules();
        // Unclosed bracket is guaranteed to be rejected by SMILES parsers.
        let result = find_routes("[C(", &env, &rules, &cfg(3));
        assert!(result.is_err(), "invalid SMILES must return Err");
    }

    #[test]
    fn max_depth_one_caps_all_routes() {
        let env = aspirin_env();
        let rules = default_rules();
        let routes = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg(1))
            .unwrap()
            .0;
        // No route should exceed depth=1 when max_depth=1.
        for r in &routes {
            assert!(
                r.depth <= 1,
                "route with depth {} exceeds max_depth=1",
                r.depth
            );
        }
    }

    #[test]
    fn beam_width_one_does_not_exceed_unrestricted() {
        let env = aspirin_env();
        let rules = default_rules();
        let cfg_beam = SearchConfig {
            max_depth: 3,
            max_routes: 10,
            beam_width: 1,
            ..Default::default()
        };
        let cfg_full = SearchConfig {
            max_depth: 3,
            max_routes: 10,
            beam_width: 0,
            ..Default::default()
        };
        let routes_beam = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg_beam)
            .unwrap()
            .0;
        let routes_full = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg_full)
            .unwrap()
            .0;
        assert!(
            routes_beam.len() <= routes_full.len(),
            "beam=1 ({}) should find ≤ routes than beam=0 ({})",
            routes_beam.len(),
            routes_full.len()
        );
    }

    #[test]
    fn route_steps_are_populated() {
        // Non-BB target must produce routes whose steps are non-empty.
        let env = aspirin_env();
        let rules = default_rules();
        let routes = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg(3))
            .unwrap()
            .0;
        let non_zero: Vec<_> = routes.iter().filter(|r| r.depth > 0).collect();
        assert!(
            !non_zero.is_empty(),
            "must find at least one multi-step route"
        );
        for r in non_zero {
            assert!(
                !r.steps.is_empty(),
                "route with depth>0 must have non-empty steps"
            );
            for step in &r.steps {
                assert!(!step.rule.is_empty(), "step.rule must be non-empty");
                assert!(!step.target.is_empty(), "step.target must be non-empty");
                assert!(
                    !step.precursors.is_empty(),
                    "step.precursors must be non-empty"
                );
            }
        }
    }

    #[test]
    fn is_extracted_template_detects_name_prefix_only() {
        assert!(is_extracted_template("extracted_0"));
        assert!(is_extracted_template("extracted_1234"));
        assert!(!is_extracted_template("suzuki_retro"));
        assert!(!is_extracted_template("cc_single_cleavage"));
    }

    #[test]
    fn absent_metadata_fields_are_omitted_from_json() {
        // An extracted-template-shaped step (metadata_source/scope both None) must
        // serialize with neither key present, so pre-existing JSON consumers see no
        // change from before these fields were added.
        let step = ReactionStep {
            rule: "extracted_0".to_string(),
            template_id: "smirks-sha256:deadbeef".to_string(),
            target: "CC(=O)O".to_string(),
            precursors: vec!["C".to_string(), "O=C=O".to_string()],
            conditions: None,
            atom_economy: None,
            atom_economy_raw_percent: None,
            atom_economy_status: AtomEconomyStatus::NotEvaluable,
            step_confidence: 0.5,
            procedure_hint: None,
            reaction_family: None,
            metadata_source: None,
            metadata_scope: None,
            evidence: None,
        };
        let json = serde_json::to_string(&step).unwrap();
        assert!(
            !json.contains("metadata_source")
                && !json.contains("metadata_scope")
                && !json.contains("evidence"),
            "absent metadata fields must be omitted from JSON, got: {json}"
        );
    }

    // ── Issue #79: atom_economy must never silently clamp ──────────────

    #[test]
    fn classify_atom_economy_normal_case_unchanged() {
        let (status, display) = classify_atom_economy(Some(87.5));
        assert_eq!(status, AtomEconomyStatus::Normal);
        assert_eq!(display, Some(87.5));
    }

    #[test]
    fn classify_atom_economy_exactly_100_is_normal() {
        let (status, display) = classify_atom_economy(Some(100.0));
        assert_eq!(status, AtomEconomyStatus::Normal);
        assert_eq!(display, Some(100.0));
    }

    #[test]
    fn classify_atom_economy_above_range_is_never_clamped_into_display() {
        // The historical bug: (raw).min(100.0) silently turned 183.4 into
        // 100.0, making a route with an unrepresented mass gap look like a
        // perfect one.
        let (status, display) = classify_atom_economy(Some(183.4));
        assert_eq!(status, AtomEconomyStatus::AboveExpectedRange);
        assert_eq!(
            display, None,
            "a ratio above the expected range must never be reported as a display value, clamped or otherwise"
        );
    }

    #[test]
    fn classify_atom_economy_not_evaluable_when_no_raw_ratio() {
        let (status, display) = classify_atom_economy(None);
        assert_eq!(status, AtomEconomyStatus::NotEvaluable);
        assert_eq!(display, None);
    }

    #[test]
    fn classify_atom_economy_nan_is_not_evaluable() {
        let (status, display) = classify_atom_economy(Some(f64::NAN));
        assert_eq!(status, AtomEconomyStatus::NotEvaluable);
        assert_eq!(display, None);
    }

    #[test]
    fn classify_atom_economy_positive_infinity_is_not_evaluable() {
        let (status, display) = classify_atom_economy(Some(f64::INFINITY));
        assert_eq!(status, AtomEconomyStatus::NotEvaluable);
        assert_eq!(display, None);
    }

    #[test]
    fn classify_atom_economy_negative_infinity_is_not_evaluable() {
        let (status, display) = classify_atom_economy(Some(f64::NEG_INFINITY));
        assert_eq!(status, AtomEconomyStatus::NotEvaluable);
        assert_eq!(display, None);
    }

    // ── compute_atom_economy_raw: the all-or-nothing denominator ────────

    #[test]
    fn compute_raw_one_unparseable_precursor_is_not_evaluable() {
        // Must not silently drop the malformed entry and compute a ratio
        // over just the remaining (valid) precursor -- that would inflate
        // the ratio exactly the way the historical clamp did.
        let raw =
            compute_atom_economy_raw("CCO", &["not_a_smiles(((".to_string(), "C".to_string()]);
        assert_eq!(raw, None);
    }

    #[test]
    fn compute_raw_unparseable_target_is_not_evaluable() {
        let raw = compute_atom_economy_raw("not_a_smiles(((", &["CCO".to_string()]);
        assert_eq!(raw, None);
    }

    #[test]
    fn compute_raw_empty_precursors_is_not_evaluable() {
        // Also exercises the zero-denominator path: an empty precursor list
        // sums to a weight of exactly 0.0.
        let raw = compute_atom_economy_raw("CCO", &[]);
        assert_eq!(raw, None);
    }

    #[test]
    fn compute_raw_normal_case_matches_direct_molecular_weight_ratio() {
        let target_w = molecular_weight(&mol_from_smiles("CCO").unwrap());
        let precursor_w = molecular_weight(&mol_from_smiles("CC=O").unwrap());
        let raw = compute_atom_economy_raw("CCO", &["CC=O".to_string()]).unwrap();
        assert!((raw - target_w / precursor_w * 100.0).abs() < 1e-9);
    }

    #[test]
    fn compute_raw_reagent_omission_lands_above_expected_range() {
        // Retro step: target = cyclohexane, precursor = benzene (H2 omitted
        // from the precursor list, as a common reagent never is tracked).
        // Every heavy (carbon) atom the target needs is supplied, but the
        // precursor list is lighter than the target -- this must classify
        // as AboveExpectedRange, not silently pass as Normal or crash.
        let raw = compute_atom_economy_raw("C1CCCCC1", &["c1ccccc1".to_string()]).unwrap();
        assert!(raw > 100.0, "expected > 100%, got {raw}");
        let (status, display) = classify_atom_economy(Some(raw));
        assert_eq!(status, AtomEconomyStatus::AboveExpectedRange);
        assert_eq!(display, None);
    }

    #[test]
    fn above_range_step_omits_atom_economy_but_keeps_raw_and_status_in_json() {
        let raw = 183.4;
        let (status, display) = classify_atom_economy(Some(raw));
        let step = ReactionStep {
            rule: "extracted_0".to_string(),
            template_id: "smirks-sha256:deadbeef".to_string(),
            target: "CC(=O)O".to_string(),
            precursors: vec!["C".to_string()],
            conditions: None,
            atom_economy: display,
            atom_economy_raw_percent: Some(raw),
            atom_economy_status: status,
            step_confidence: 0.5,
            procedure_hint: None,
            reaction_family: None,
            metadata_source: None,
            metadata_scope: None,
            evidence: None,
        };
        let json = serde_json::to_string(&step).unwrap();
        assert!(
            !json.contains("\"atom_economy\":"),
            "atom_economy must be absent (never a clamped 100.0), got: {json}"
        );
        assert!(
            json.contains("\"atom_economy_raw_percent\":183.4"),
            "the honest raw ratio must still be reported, got: {json}"
        );
        assert!(
            json.contains("\"atom_economy_status\":\"above_expected_range\""),
            "got: {json}"
        );
    }

    #[test]
    fn compute_raw_precursor_excess_is_normal_well_under_100() {
        // Opposite direction: precursors heavier than the target (a leaving
        // group is dropped) is the ordinary, expected case, not a status of
        // its own.
        let raw = compute_atom_economy_raw("c1ccccc1", &["C1CCCCC1".to_string()]).unwrap();
        assert!(raw < 100.0, "expected < 100%, got {raw}");
        let (status, _) = classify_atom_economy(Some(raw));
        assert_eq!(status, AtomEconomyStatus::Normal);
    }

    #[test]
    fn atom_economy_fields_json_round_trip_by_status() {
        fn step_with(
            status: AtomEconomyStatus,
            display: Option<f64>,
            raw: Option<f64>,
        ) -> ReactionStep {
            ReactionStep {
                rule: "extracted_0".to_string(),
                template_id: "smirks-sha256:deadbeef".to_string(),
                target: "CC(=O)O".to_string(),
                precursors: vec!["C".to_string()],
                conditions: None,
                atom_economy: display,
                atom_economy_raw_percent: raw,
                atom_economy_status: status,
                step_confidence: 0.5,
                procedure_hint: None,
                reaction_family: None,
                metadata_source: None,
                metadata_scope: None,
                evidence: None,
            }
        }

        let normal = step_with(AtomEconomyStatus::Normal, Some(87.5), Some(87.5));
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&normal).unwrap()).unwrap();
        assert_eq!(v["atom_economy"], serde_json::json!(87.5));
        assert_eq!(v["atom_economy_raw_percent"], serde_json::json!(87.5));
        assert_eq!(v["atom_economy_status"], serde_json::json!("normal"));

        let above = step_with(AtomEconomyStatus::AboveExpectedRange, None, Some(183.4));
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&above).unwrap()).unwrap();
        assert!(v.get("atom_economy").is_none());
        assert_eq!(v["atom_economy_raw_percent"], serde_json::json!(183.4));
        assert_eq!(
            v["atom_economy_status"],
            serde_json::json!("above_expected_range")
        );

        let not_evaluable = step_with(AtomEconomyStatus::NotEvaluable, None, None);
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&not_evaluable).unwrap()).unwrap();
        assert!(v.get("atom_economy").is_none());
        assert!(v.get("atom_economy_raw_percent").is_none());
        assert_eq!(v["atom_economy_status"], serde_json::json!("not_evaluable"));
    }

    #[test]
    fn handcrafted_rule_step_is_tagged() {
        // default_rules() contains only hand-crafted rules (no extracted templates
        // loaded), so every step of every route found here must be hand-crafted.
        let env = aspirin_env();
        let rules = default_rules();
        let routes = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg(3))
            .unwrap()
            .0;
        let non_zero: Vec<_> = routes.iter().filter(|r| r.depth > 0).collect();
        assert!(
            !non_zero.is_empty(),
            "must find at least one multi-step route"
        );
        for r in non_zero {
            for step in &r.steps {
                assert_eq!(
                    step.metadata_source,
                    Some(MetadataSource::HandcraftedDefault),
                    "step using hand-crafted rule {:?} must be tagged HandcraftedDefault",
                    step.rule
                );
                assert_eq!(
                    step.metadata_scope,
                    Some(EvidenceScope::ReactionFamily),
                    "step using hand-crafted rule {:?} must be scoped ReactionFamily",
                    step.rule
                );
            }
        }
    }

    #[test]
    fn no_metadata_configured_means_no_evidence() {
        // config.template_metadata defaults to None -- every step.evidence must be
        // None, reproducing pre-existing (pre-evidence-sidecar) behavior exactly.
        let env = aspirin_env();
        let rules = default_rules();
        let routes = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg(3))
            .unwrap()
            .0;
        assert!(!routes.is_empty());
        for route in &routes {
            for step in &route.steps {
                assert!(
                    step.evidence.is_none(),
                    "no metadata sidecar configured -- step.evidence must stay None"
                );
                assert!(
                    !step.template_id.is_empty(),
                    "template_id must always be populated"
                );
            }
        }
    }

    #[test]
    fn evidence_attached_only_to_matching_template_id() {
        let env = aspirin_env();
        let rules = default_rules();
        let target_template_id = rules
            .iter()
            .find(|r| r.name == "ester_cleavage")
            .unwrap()
            .template_id
            .clone();

        let mut templates = std::collections::HashMap::new();
        templates.insert(
            target_template_id.clone(),
            crate::evidence::TemplateMetadataEntry {
                warnings: vec![crate::evidence::ReactionWarning {
                    code: "test_code".to_string(),
                    severity: crate::evidence::WarningSeverity::Low,
                    message: "test warning".to_string(),
                    source: MetadataSource::Literature,
                    scope: EvidenceScope::Template,
                    reference_ids: vec![],
                }],
                ..Default::default()
            },
        );
        let config = SearchConfig {
            template_metadata: Some(templates),
            ..cfg(3)
        };
        let routes = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &config)
            .unwrap()
            .0;

        let mut saw_match = false;
        let mut saw_non_match = false;
        for route in &routes {
            for step in &route.steps {
                if step.template_id == target_template_id {
                    assert!(
                        step.evidence.is_some(),
                        "step using the metadata-matched template must get evidence"
                    );
                    saw_match = true;
                } else {
                    assert!(
                        step.evidence.is_none(),
                        "step using a non-matched template must not get evidence"
                    );
                    saw_non_match = true;
                }
            }
        }
        assert!(saw_match, "expected at least one step using ester_cleavage");
        assert!(
            saw_non_match,
            "expected at least one step using a different rule"
        );
    }

    #[test]
    fn symmetric_biaryl_routes_deduplicated() {
        // Biphenyl is symmetric: both orientations of Suzuki retro yield the same
        // precursor set {Brc1ccccc1, c1ccccc1}. The search must dedup to ≤ 1 route.
        let env = ChemEnv::in_memory(&["Brc1ccccc1", "c1ccccc1"]);
        let rules = default_rules();
        let cfg = SearchConfig {
            max_depth: 2,
            max_routes: 10,
            beam_width: 0,
            ..Default::default()
        };
        let routes = find_routes("c1ccc(-c2ccccc2)cc1", &env, &rules, &cfg)
            .unwrap()
            .0;
        // Both orientations resolve to identical BB sets — expect exactly 1 unique route.
        assert_eq!(
            routes.len(),
            1,
            "symmetric biphenyl should produce exactly 1 deduplicated route; got {}",
            routes.len()
        );
    }

    #[test]
    fn confidence_is_between_zero_and_one() {
        let env = aspirin_env();
        let rules = default_rules();
        let (routes, _) = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg(3)).unwrap();
        assert!(!routes.is_empty(), "must find at least one route");
        for route in &routes {
            assert!(
                (0.0..=1.0).contains(&route.confidence),
                "confidence {} out of [0,1]",
                route.confidence
            );
        }
    }

    #[test]
    fn search_stats_nodes_expanded_nonzero() {
        let env = ChemEnv::in_memory(&["O"]); // only water — aspirin unsolvable
        let rules = default_rules();
        let (routes, stats) = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg(2)).unwrap();
        assert!(
            routes.is_empty(),
            "aspirin should be unsolvable with only water as BB"
        );
        assert!(
            stats.nodes_expanded > 0,
            "nodes_expanded must be > 0 even for failed search"
        );
    }

    #[test]
    fn avoid_elements_removes_forbidden_bbs() {
        let env = aspirin_env();
        let rules = default_rules();
        let config = SearchConfig {
            forbidden_elements: crate::chem_env::elem_symbols_to_mask("Cl"),
            ..cfg(3)
        };
        let (routes, _) = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &config).unwrap();
        for route in &routes {
            for bb in &route.building_blocks {
                assert!(!bb.contains("Cl"), "BB {bb} contains forbidden element Cl");
            }
        }
    }

    #[test]
    fn find_routes_returns_stats_tuple() {
        let env = aspirin_env();
        let rules = default_rules();
        // Just verify the return type is a tuple and stats has a reasonable value.
        let (routes, stats) = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg(3)).unwrap();
        assert!(!routes.is_empty());
        assert!(stats.nodes_expanded >= routes.len() as u64);
    }

    // ── E2 closed-set correctness: proven LATENT bug reproduction ───────────
    //
    // `closed: FxHashSet<u64>` is a boolean "already visited" set keyed only
    // by frontier molecule content (`state_hash`) — no `g` is stored, so
    // there is no reopen-on-lower-g. For a *consistent* heuristic, A* graph
    // search guarantees the first pop of a state has optimal `g`, so a plain
    // closed set is safe.
    //
    // IMPORTANT — this test is a LATENT bug demonstration, not a live one:
    // it requires an injected `ReactionPrior`/`MoleculeValueEstimator` (bonus
    // 5.0, h 100.0) to force the pop order needed. Every production entry
    // point (CLI, renkin-bench, Python, WASM) passes `reaction_prior: None`
    // / `value_estimator: None` today (grep confirms), so this exact
    // mechanism does not fire in current production runs. Separately, E4
    // (below) shows the *default* cost formula is already inadmissible
    // (net step cost can be 0.8 < the heuristic's assumed 1.0 floor) — but
    // algebraically, that bounded 0.2 gap can never make a longer path to
    // the same single-molecule state cheaper than a direct one (extra hop
    // costs >=0.8, max bonus saving is 0.2), so today's default config
    // cannot trigger *this specific* construction either. The risk is real
    // but currently dormant: `ReactionPrior`/`MoleculeValueEstimator` are
    // unbounded public hooks meant for future NN-based scoring (Track D/E3)
    // — the day one is wired up without a floor clamp, this closed set will
    // silently drop better paths in production.
    //
    // Minimal deterministic reproduction using the real `find_routes` (real
    // heap, real closed set, real chematic SMIRKS chemistry), with an
    // injected prior/estimator to force the exact pop order needed to prove
    // the mechanism:
    //
    //   T (ClCCI) --r_direct (bonus 0)--------------> M (BrCCBr)  [g≈1.09]
    //   T (ClCCI) --r_step1  (bonus 5)--> Y (BrCCI)
    //                 Y      --r_step2  (bonus 5)--> M (BrCCBr)  [g≈-7.79]
    //   M --r_final--------------------------------> Z (FCCF, the only BB)
    //
    // h(Y) is set artificially high (100) so the direct T->M arrival (g≈1.09)
    // pops and closes state {M} *before* the much cheaper T->Y->M arrival
    // (g≈-7.79) is even generated. When the cheaper arrival is later popped,
    // it finds {M} already closed and is discarded without expansion — the
    // true-optimal route (T->Y->M->Z, g≈-6.76) is never found; only the
    // worse route (T->M->Z, g≈2.13) is returned.
    //
    // NOTE for whoever implements the E2 fix: this test asserts the CURRENT
    // (buggy) behavior and will start FAILING once the closed set reopens on
    // a lower g (verified experimentally: swapping `closed` for a
    // `FxHashMap<u64, f64>` best-g map with a reopen check makes
    // `best_score` land at -6.755613, matching the hand-derived optimum). At
    // that point, invert the assertions below to pin the fixed behavior.
    #[test]
    fn closed_set_discards_better_path_reaching_same_state() {
        fn rr(name: &str, smirks: &str) -> RetroRule {
            RetroRule {
                name: name.to_string(),
                template_id: format!("rule:{name}"),
                smirks: smirks.to_string(),
                weight: 1.0,
                required_elements: 0,
            }
        }

        let rules = vec![
            rr("r_direct", "[Cl][C:1][C:2][I]>>[Br][C:1][C:2][Br]"),
            rr("r_step1", "[Cl][C:1][C:2][I]>>[Br][C:1][C:2][I]"),
            rr("r_step2", "[Br][C:1][C:2][I]>>[Br][C:1][C:2][Br]"),
            rr("r_final", "[Br][C:1][C:2][Br]>>[F][C:1][C:2][F]"),
        ];

        // Discover Y's canonical SMILES dynamically — don't hardcode a
        // chematic-version-dependent canonical string (chematic has already
        // moved 0.4.25 -> 0.4.30 once in this repo's history).
        let t_mol = mol_from_smiles("ClCCI").unwrap();
        let y_smiles = apply_retro(&t_mol, &rules[1])[0][0].smiles.clone();

        let env = ChemEnv::in_memory(&["FCCF"]); // the only building block

        struct FixedPrior;
        impl ReactionPrior for FixedPrior {
            fn prior(&self, template_name: &str, _target_smiles: &str) -> f64 {
                match template_name {
                    "r_step1" | "r_step2" => 5.0,
                    _ => 0.0,
                }
            }
        }

        struct FixedEstimator {
            y_smiles: String,
        }
        impl MoleculeValueEstimator for FixedEstimator {
            fn estimate_cost(&self, smiles: &str) -> f64 {
                if smiles == self.y_smiles { 100.0 } else { 0.0 }
            }
        }

        let config = SearchConfig {
            max_depth: 5,
            max_routes: 10,
            beam_width: 0,
            reaction_prior: Some(std::sync::Arc::new(FixedPrior)),
            value_estimator: Some(std::sync::Arc::new(FixedEstimator {
                y_smiles: y_smiles.clone(),
            })),
            ..Default::default()
        };

        let (routes, _stats) = find_routes("ClCCI", &env, &rules, &config).unwrap();

        assert!(!routes.is_empty(), "must find at least the direct route");
        let best_score = routes.iter().map(|r| r.score).fold(f64::INFINITY, f64::min);

        // The true optimum (T->Y->M->Z) has g ≈ -6.76. If the closed set
        // reopened on a better g, `best_score` would be deeply negative.
        // Instead only the worse direct route (g ≈ 2.13) is ever recorded —
        // proving the cheaper re-arrival at {M} was discarded unexpanded.
        assert!(
            best_score > -1.0,
            "expected the boolean closed-set bug to discard the better \
             (g≈-6.76) route, leaving only the worse (g≈2.13) route — but \
             best_score={best_score} suggests the optimal route WAS found \
             (bug fixed, or test assumptions stale)"
        );
        assert!(
            (best_score - 2.127).abs() < 0.05,
            "expected the only recorded route to be the direct-path route \
             (g≈2.13), got best_score={best_score}"
        );
    }
}
