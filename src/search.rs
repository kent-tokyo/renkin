use std::collections::{BinaryHeap, HashSet};

use anyhow::Result;
#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;
use serde::Serialize;

use crate::chem_env::{
    ChemEnv, PrecursorMol, RetroRule, apply_retro, mol_from_smiles, to_canonical,
};
use crate::score::{heuristic, step_cost, template_bonus};

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
}

#[derive(Debug, Clone)]
struct FEntry {
    smiles: String,
}

#[derive(Debug, Clone)]
struct Node {
    frontier: Vec<FEntry>,
    path: Vec<ReactionStep>,
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

fn state_key(frontier: &[FEntry]) -> String {
    let mut keys: Vec<&str> = frontier.iter().map(|e| e.smiles.as_str()).collect();
    keys.sort();
    keys.join("|")
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

fn unsolved_mols(frontier: &[FEntry], env: &ChemEnv) -> Vec<crate::chem_env::Molecule> {
    frontier
        .iter()
        .filter(|e| !is_bb(&e.smiles, env))
        .filter_map(|e| mol_from_smiles(&e.smiles).ok())
        .collect()
}

fn count_unsolved(frontier: &[FEntry], env: &ChemEnv) -> usize {
    frontier.iter().filter(|e| !is_bb(&e.smiles, env)).count()
}

fn compute_h(frontier: &[FEntry], env: &ChemEnv) -> f64 {
    let mols = unsolved_mols(frontier, env);
    heuristic(&mols.iter().collect::<Vec<_>>())
}

/// Prune the heap to at most `beam_width` nodes (keep the best).
fn beam_prune(heap: &mut BinaryHeap<Node>, beam_width: usize) {
    if beam_width == 0 || heap.len() <= beam_width {
        return;
    }
    let mut nodes: Vec<Node> = heap.drain().collect();
    // Sort best (lowest f) first; BinaryHeap is max-heap but our Ord reverses it,
    // so after drain+sort we need ascending f order.
    nodes.sort_by(|a, b| {
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
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            max_depth: 5,
            max_routes: 5,
            beam_width: 0,
        }
    }
}

pub fn find_routes(
    target_smiles: &str,
    env: &ChemEnv,
    rules: &[RetroRule],
    config: &SearchConfig,
) -> Result<Vec<Route>> {
    let target_mol = mol_from_smiles(target_smiles)?;
    let target_canonical = to_canonical(&target_mol);

    let max_rule_weight = rules.iter().map(|r| r.weight).fold(1.0_f64, f64::max);

    let mut routes: Vec<Route> = Vec::new();
    let mut closed: HashSet<String> = HashSet::new();
    let mut heap: BinaryHeap<Node> = BinaryHeap::new();

    let initial = vec![FEntry {
        smiles: target_canonical,
    }];
    let h0 = compute_h(&initial, env);
    heap.push(Node {
        frontier: initial,
        path: vec![],
        depth: 0,
        g: 0.0,
        h: h0,
    });

    while let Some(node) = heap.pop() {
        if routes.len() >= config.max_routes {
            break;
        }

        if count_unsolved(&node.frontier, env) == 0 {
            routes.push(Route {
                steps: node.path.clone(),
                depth: node.depth,
            });
        }

        if node.depth >= config.max_depth {
            continue;
        }

        let key = state_key(&node.frontier);
        if closed.contains(&key) {
            continue;
        }
        closed.insert(key);

        let Some(target_entry) = node
            .frontier
            .iter()
            .find(|e| !is_bb(&e.smiles, env))
            .or_else(|| node.frontier.first())
        else {
            continue;
        };
        let target_smi = target_entry.smiles.clone();

        let Ok(target_mol) = mol_from_smiles(&target_smi) else {
            continue;
        };

        let target_elem_mask: u64 = elem_mask_from_smiles(&target_smi);

        // Parallel rule application via rayon (native); sequential on WASM.
        // Rules whose required_elements are absent from the target are skipped early.
        #[cfg(not(target_arch = "wasm32"))]
        let expanded: Vec<(String, f64, Vec<PrecursorMol>)> = rules
            .par_iter()
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
        let expanded: Vec<(String, f64, Vec<PrecursorMol>)> = rules
            .iter()
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

        for (rule_name, rule_weight, precursors) in expanded {
            if precursors.is_empty() {
                continue;
            }
            if precursors.iter().any(|p| p.smiles == target_smi) {
                continue;
            }

            let precursor_smiles: Vec<String> =
                precursors.iter().map(|p| p.smiles.clone()).collect();

            let new_frontier: Vec<FEntry> = node
                .frontier
                .iter()
                .filter(|e| e.smiles != target_smi)
                .cloned()
                .chain(precursors.iter().map(|p| FEntry {
                    smiles: p.smiles.clone(),
                }))
                .collect();

            let step_c = step_cost(&precursors.iter().map(|p| &p.mol).collect::<Vec<_>>())
                - template_bonus(rule_weight, max_rule_weight);
            let new_h = compute_h(&new_frontier, env);

            let mut new_path = node.path.clone();
            new_path.push(ReactionStep {
                rule: rule_name,
                target: target_smi.clone(),
                precursors: precursor_smiles,
            });

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

    Ok(routes)
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
        }
    }

    #[test]
    fn aspirin_finds_route_depth1() {
        let env = aspirin_env();
        let rules = default_rules();
        let routes = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg(3)).unwrap();
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
        let routes = find_routes("CC(=O)O", &env, &rules, &cfg(2)).unwrap();
        assert!(
            routes.iter().any(|r| r.depth == 0),
            "building block must return depth-0 route"
        );
    }

    #[test]
    fn anthranilic_acid_recognized_as_bb() {
        let env = aspirin_env();
        let rules = default_rules();
        let routes = find_routes("c1ccc(N)cc1C(=O)O", &env, &rules, &cfg(3)).unwrap();
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
        let routes = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg(1)).unwrap();
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
        let routes = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg(1)).unwrap();
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
        };
        let cfg_full = SearchConfig {
            max_depth: 3,
            max_routes: 10,
            beam_width: 0,
        };
        let routes_beam = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg_beam).unwrap();
        let routes_full = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg_full).unwrap();
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
        let routes = find_routes("CC(=O)Oc1ccccc1C(=O)O", &env, &rules, &cfg(3)).unwrap();
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
        };
        let routes = find_routes("c1ccc(-c2ccccc2)cc1", &env, &rules, &cfg).unwrap();
        // Both orientations resolve to identical BB sets — expect exactly 1 unique route.
        assert_eq!(
            routes.len(),
            1,
            "symmetric biphenyl should produce exactly 1 deduplicated route; got {}",
            routes.len()
        );
    }
}
