use std::collections::{BinaryHeap, HashSet};

use anyhow::Result;
#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;
use serde::Serialize;

use crate::chem_env::{ChemEnv, PrecursorMol, RetroRule, apply_retro, mol_from_smiles, to_canonical};
use crate::score::{heuristic, step_cost};

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
        other.f().partial_cmp(&self.f()).unwrap_or(std::cmp::Ordering::Equal)
    }
}

fn state_key(frontier: &[FEntry]) -> String {
    let mut keys: Vec<&str> = frontier.iter().map(|e| e.smiles.as_str()).collect();
    keys.sort();
    keys.join("|")
}

fn is_bb(smiles: &str, env: &ChemEnv) -> bool {
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
    nodes.sort_by(|a, b| a.f().partial_cmp(&b.f()).unwrap_or(std::cmp::Ordering::Equal));
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
        Self { max_depth: 5, max_routes: 5, beam_width: 0 }
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

    let mut routes: Vec<Route> = Vec::new();
    let mut closed: HashSet<String> = HashSet::new();
    let mut heap: BinaryHeap<Node> = BinaryHeap::new();

    let initial = vec![FEntry { smiles: target_canonical }];
    let h0 = compute_h(&initial, env);
    heap.push(Node { frontier: initial, path: vec![], depth: 0, g: 0.0, h: h0 });

    while let Some(node) = heap.pop() {
        if routes.len() >= config.max_routes {
            break;
        }

        if count_unsolved(&node.frontier, env) == 0 {
            routes.push(Route { steps: node.path.clone(), depth: node.depth });
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

        // Parallel rule application via rayon (native); sequential on WASM.
        #[cfg(not(target_arch = "wasm32"))]
        let expanded: Vec<(String, Vec<PrecursorMol>)> = rules
            .par_iter()
            .flat_map(|rule| {
                apply_retro(&target_mol, rule)
                    .into_iter()
                    .map(|precs| (rule.name.to_string(), precs))
                    .collect::<Vec<_>>()
            })
            .collect();
        #[cfg(target_arch = "wasm32")]
        let expanded: Vec<(String, Vec<PrecursorMol>)> = rules
            .iter()
            .flat_map(|rule| {
                apply_retro(&target_mol, rule)
                    .into_iter()
                    .map(|precs| (rule.name.to_string(), precs))
                    .collect::<Vec<_>>()
            })
            .collect();

        for (rule_name, precursors) in expanded {
            if precursors.is_empty() {
                continue;
            }
            if precursors.iter().any(|p| p.smiles == target_smi) {
                continue;
            }

            let precursor_smiles: Vec<String> = precursors.iter().map(|p| p.smiles.clone()).collect();

            let new_frontier: Vec<FEntry> = node
                .frontier
                .iter()
                .filter(|e| e.smiles != target_smi)
                .cloned()
                .chain(precursors.iter().map(|p| FEntry { smiles: p.smiles.clone() }))
                .collect();

            let step_c = step_cost(&precursors.iter().map(|p| &p.mol).collect::<Vec<_>>());
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
