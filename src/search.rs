use std::collections::BinaryHeap;
use std::sync::Arc;

use anyhow::Result;
use chematic::chem::sa_score;
#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet, FxHasher};
use serde::Serialize;
use smallvec::{SmallVec, smallvec};

use crate::chem_env::{
    ChemEnv, PrecursorMol, RetroRule, apply_retro, mol_from_smiles, to_canonical,
};
use crate::score::{step_cost, template_bonus};

/// Cached expansion for one (target_smiles, rule) combination.
/// Tuple: (rule_name, net_step_cost, precursor_smiles_list).
type RetroEntry = (String, f64, Vec<String>);
type RetroCache = FxHashMap<String, Arc<Vec<RetroEntry>>>;

#[derive(Debug, Clone, Serialize)]
pub struct ReactionStep {
    pub rule: String,
    pub target: String,
    pub precursors: Vec<String>,
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

fn compute_h(frontier: &[FEntry], env: &ChemEnv, sa_cache: &mut FxHashMap<String, f64>) -> f64 {
    frontier
        .iter()
        .filter(|e| !is_bb(&e.smiles, env))
        .map(|e| {
            let sa = if let Some(&v) = sa_cache.get(&e.smiles) {
                v
            } else {
                let v = mol_from_smiles(&e.smiles)
                    .map(|m| sa_score(&m).clamp(1.0, 10.0))
                    .unwrap_or(5.5);
                sa_cache.insert(e.smiles.clone(), v);
                v
            };
            1.0 + 0.5 * (sa - 1.0) / 9.0
        })
        .sum()
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
    let h0 = compute_h(&initial, env, &mut sa_cache);
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
                confidence: 0.0, // computed below after all routes collected
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
            #[cfg(not(target_arch = "wasm32"))]
            let raw: Vec<(String, f64, Vec<PrecursorMol>)> = ranked_rules
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
            let raw: Vec<(String, f64, Vec<PrecursorMol>)> = ranked_rules
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
                    let step_c = step_cost(&precs.iter().map(|p| &p.mol).collect::<Vec<_>>())
                        - template_bonus(rule_weight, max_rule_weight);
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

            let new_h = compute_h(&new_frontier, env, &mut sa_cache);

            // O(1) Arc::clone — shares the parent prefix without copying.
            let new_path = Some(Arc::new(PathNode {
                step: ReactionStep {
                    rule: rule_name.clone(),
                    target: target_smi.clone(),
                    precursors: precursor_smiles.clone(),
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

    // Compute confidence for each route: min template weight / max weight in rule set.
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
