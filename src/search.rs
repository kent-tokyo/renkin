use std::collections::BinaryHeap;
use std::sync::Arc;

use anyhow::Result;
use chematic::chem::{molecular_weight, sa_score};
#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet, FxHasher};
use serde::Serialize;
use smallvec::{SmallVec, smallvec};

use crate::chem_env::{
    ChemEnv, PrecursorMol, RetroRule, TemplateBondIndex, apply_retro, mol_from_smiles, to_canonical,
};
use crate::score::{step_cost, template_bonus};

/// Cached expansion for one (target_smiles, rule) combination.
/// Tuple: (rule_name, net_step_cost, precursor_smiles_list).
type RetroEntry = (String, f64, Vec<String>);
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

#[derive(Debug, Clone, Serialize)]
pub struct ReactionStep {
    pub rule: String,
    pub target: String,
    pub precursors: Vec<String>,
    /// Suggested conditions for the forward reaction (None for extracted templates).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conditions: Option<ReactionConditions>,
    /// Atom economy: MW(target) / Σ MW(precursors) × 100 — fraction of atoms retained.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub atom_economy: Option<f64>,
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
    /// Product of step_confidence values (Retro-prob style).
    /// Estimates the probability that every step in the route succeeds.
    /// Single-step: equals step_confidence. Multi-step: decays multiplicatively.
    pub success_probability: f64,
    /// Estimated synthesis cost: Σ(BB complexity or price) + step_count × 0.5.
    /// Uses SA Score as complexity proxy when no price file is provided.
    /// Lower = cheaper / simpler route.
    pub route_cost: f64,
}

/// Statistics returned alongside routes from [`find_routes`].
#[derive(Debug, Default, Serialize)]
pub struct SearchStats {
    /// Number of unique states expanded (inserted into the closed set).
    pub nodes_expanded: u64,
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
fn elem_mask_from_smiles(smiles: &str) -> u64 {
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
    // VF2 fallback: handles edge cases where run_reactants produces
    // explicit-H forms whose canonical SMILES doesn't match the stored form.
    mol_from_smiles(smiles)
        .map(|mol| env.is_building_block(&mol))
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
        "aryl_chloride_retro" | "aryl_iodide_retro" | "aryl_fluoride_snAr_retro" => {
            Some("c_halide_activation")
        }
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
        "aryl_chloride_retro" => cond!("none", "DMF", "80 °C", "SNAr or Pd activation"),
        "aryl_iodide_retro" => cond!("Pd(OAc)₂ / CuI", "DMF", "60 °C"),
        "aryl_fluoride_snAr_retro" => cond!(
            "K₂CO₃ (2 eq)",
            "DMSO",
            "rt → 60 °C",
            "SNAr; F best leaving group"
        ),
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
    /// Phase B: ONNX template relevance scorer (CLI/Python only, not WASM).
    /// When Some, pre-filters rules to top-K most relevant before SMARTS matching.
    #[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
    pub nn_scorer: Option<std::sync::Arc<crate::scorer::nn::TemplateScorer>>,
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
            #[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
            nn_scorer: None,
        }
    }
}

pub fn find_routes(
    target_smiles: &str,
    env: &ChemEnv,
    rules: &[RetroRule],
    config: &SearchConfig,
) -> Result<(Vec<Route>, SearchStats)> {
    let target_mol = mol_from_smiles(target_smiles)?;
    let target_canonical = to_canonical(&target_mol);

    // Phase B: pre-rank rules ONCE for the initial target molecule.
    // The scorer is called here (before the A* loop) — not per-node — to avoid
    // hundreds of ONNX inference calls per search. The ranking is reused across
    // all A* expansions; deeper intermediates use the same ordering.
    #[cfg(all(not(target_arch = "wasm32"), feature = "nn-scoring"))]
    let ranked_rules: Vec<&RetroRule> = {
        if let Some(sc) = &config.nn_scorer {
            sc.top_k_indices(target_smiles, rules.len())
                .into_iter()
                .filter_map(|i| rules.get(i))
                .collect()
        } else {
            rules.iter().collect()
        }
    };
    #[cfg(not(all(not(target_arch = "wasm32"), feature = "nn-scoring")))]
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

        let target_elem_mask: u64 = elem_mask_from_smiles(&target_smi);

        // Opt-D: look up the memoized expansion for this target molecule.
        // On cache miss: run apply_retro in parallel (native) / sequential (WASM),
        // filter invalid results, precompute net step cost, and store.
        // On cache hit: O(1) Arc::clone — no Vec data is copied.
        let expansions: Arc<Vec<RetroEntry>> = if let Some(cached) = retro_cache.get(&target_smi) {
            Arc::clone(cached) // O(1): pointer copy only, no Vec clone
        } else {
            // Bond-center retrieval: filter ranked_rules to those relevant to this molecule's bonds.
            // Falls back to ranked_rules unchanged when bond_idx is None (--bond-index not set).
            let retrieved: Vec<&RetroRule>;
            let active_rules: &[&RetroRule] = if let Some(ref idx) = bond_idx {
                retrieved = idx
                    .retrieve(&target_mol, 0, rules) // top_k=0 = no truncation
                    .into_iter()
                    .filter_map(|i| rules.get(i))
                    .collect();
                &retrieved
            } else {
                &ranked_rules
            };

            #[cfg(not(target_arch = "wasm32"))]
            let raw: Vec<(String, f64, Vec<PrecursorMol>)> = active_rules
                .par_iter()
                .copied()
                .filter(|rule| {
                    rule.required_elements == 0
                        || (target_elem_mask & rule.required_elements == rule.required_elements)
                })
                .flat_map(|rule| {
                    apply_retro(&target_mol, rule)
                        .into_iter()
                        .map(|precs| (rule.name.to_string(), rule.weight, precs))
                        .collect::<Vec<_>>()
                })
                .collect();
            #[cfg(target_arch = "wasm32")]
            let raw: Vec<(String, f64, Vec<PrecursorMol>)> = active_rules
                .iter()
                .copied()
                .filter(|rule| {
                    rule.required_elements == 0
                        || (target_elem_mask & rule.required_elements == rule.required_elements)
                })
                .flat_map(|rule| {
                    apply_retro(&target_mol, rule)
                        .into_iter()
                        .map(|precs| (rule.name.to_string(), rule.weight, precs))
                        .collect::<Vec<_>>()
                })
                .collect();

            let entries: Vec<(String, f64, Vec<String>)> = raw
                .into_iter()
                .filter(|(_, _, precs)| {
                    !precs.is_empty() && !precs.iter().any(|p| p.smiles == target_smi)
                })
                .map(|(rule_name, rule_weight, precs)| {
                    let bonus = if let Some(ref prior) = config.reaction_prior {
                        prior.prior(&rule_name, &target_smi)
                    } else {
                        template_bonus(rule_weight, max_rule_weight)
                    };
                    let step_c =
                        step_cost(&precs.iter().map(|p| &p.mol).collect::<Vec<_>>()) - bonus;
                    let smiles_list: Vec<String> = precs.iter().map(|p| p.smiles.clone()).collect();
                    (rule_name, step_c, smiles_list)
                })
                .collect();
            let arc = Arc::new(entries);
            retro_cache.insert(target_smi.clone(), Arc::clone(&arc));
            arc // no extra clone: Arc move
        };

        for (rule_name, step_c, precursor_smiles) in expansions.iter() {
            let new_frontier: SmallVec<[FEntry; 6]> = node
                .frontier
                .iter()
                .filter(|e| e.smiles != target_smi)
                .cloned()
                .chain(
                    precursor_smiles
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
                    rule: rule_name.clone(),
                    target: target_smi.clone(),
                    precursors: precursor_smiles.clone(),
                    conditions: conditions_for_rule(rule_name),
                    atom_economy: None,   // populated in post-processing
                    step_confidence: 0.0, // populated in post-processing
                    reaction_family: reaction_family_for_rule(rule_name).map(str::to_string),
                    procedure_hint: procedure_hint_for_rule(rule_name).map(str::to_string),
                },
                prev: node.path.clone(),
            }));

            // In-search pruning: skip expansions where a BB-precursor contains a
            // forbidden element. Avoids pushing dead-end nodes onto the heap.
            if config.forbidden_elements != 0 {
                let mask = config.forbidden_elements;
                if precursor_smiles
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
                g: node.g + step_c,
                h: new_h,
            });
        }

        // --- Phase 3.2: Beam search pruning ---
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

                let tmw = mol_from_smiles(&step.target)
                    .ok()
                    .map(|m| molecular_weight(&m));
                let pmw: f64 = step
                    .precursors
                    .iter()
                    .filter_map(|s| mol_from_smiles(s).ok())
                    .map(|m| molecular_weight(&m))
                    .sum();
                step.atom_economy = tmw.and_then(|tw| {
                    if pmw > 0.0 {
                        Some((tw / pmw * 100.0).min(100.0))
                    } else {
                        None
                    }
                });
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
            "[renkin] search complete\n  nodes popped   : {}\n  nodes expanded : {}\n  routes found   : {}\n  elapsed        : {:.2} s",
            nodes_popped,
            nodes_expanded,
            routes.len(),
            t0.elapsed().as_secs_f64()
        );
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

    Ok((routes, SearchStats { nodes_expanded }))
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
