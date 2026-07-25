//! Track D measurement (Phase 32): does root-only NN template-scorer ranking
//! (computed once for the root target and reused for every deeper intermediate,
//! see `search.rs`'s "Phase B" comment) hurt top-K recall of the *actually-used*
//! rule at deep intermediates, compared to re-running the scorer fresh on each
//! intermediate ("per-node" ranking)?
//!
//! Method: reconstruct real routes with a full (unfiltered) search — same config
//! as `examples/inspect_validation.rs` — to get ground-truth (intermediate SMILES,
//! rule actually used) pairs from RENKIN's own solved routes. For every pair where
//! the rule is an extracted template (name = "extracted_<i>", i.e. scoreable by
//! the NN), compute the rank of template `i` under:
//!   (a) root-only ranking: scorer run once on the route's root target SMILES
//!   (b) per-node ranking: scorer run fresh on that step's intermediate SMILES
//! Reports top-1/10/50/100 recall for both, stratified by depth (depth 0 = the
//! rule applied directly to the root, where (a) and (b) are identical by
//! construction — the informative comparison is depth >= 1).
//!
//! Not part of any measured binary. Reads root target SMILES from stdin (one per
//! line). Usage:
//!   cargo run --release --features nn-scoring --example measure_rank_recall \
//!       -- [max_targets] < targets.smi > report.txt
use renkin::chem_env::{ChemEnv, default_rules, load_rules_from_file};
use renkin::scorer::nn::TemplateScorer;
use renkin::search::{SearchConfig, find_routes};
use std::io::Read;

/// Full descending-score rank of every file template for `smiles`, as a
/// 0-indexed-template -> 1-indexed-rank map. `scorer.top_k` must be >= the
/// number of file templates (construct with `top_k = rules.len()`) so
/// `top_k_indices` returns the complete order, not a truncated one.
fn file_template_ranks(
    scorer: &TemplateScorer,
    smiles: &str,
    offset: usize,
    n_rules: usize,
) -> Vec<usize> {
    let order = scorer.top_k_indices(smiles, n_rules);
    // order[..offset] are the always-included hand-crafted rules (unranked
    // prefix); order[offset..] are file-template global indices in descending
    // score order. Invert into rank_by_template_idx[template_idx] = rank (1-based).
    let mut rank_by_template_idx = vec![0usize; n_rules - offset];
    for (rank0, &global_idx) in order[offset..].iter().enumerate() {
        rank_by_template_idx[global_idx - offset] = rank0 + 1;
    }
    rank_by_template_idx
}

fn recall_at(ranks: &[usize], k: usize) -> f64 {
    if ranks.is_empty() {
        return f64::NAN;
    }
    ranks.iter().filter(|&&r| r <= k).count() as f64 / ranks.len() as f64
}

fn main() {
    let max_targets: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(250);

    let env = ChemEnv::load("data/building_blocks.smi").expect("load building blocks");
    let mut rules = default_rules();
    let offset = rules.len();
    rules.extend(load_rules_from_file("data/templates_extracted_5000.smi"));
    let n_rules = rules.len();
    eprintln!(
        "Loaded {offset} hand-crafted rules + {} extracted templates ({n_rules} total)",
        n_rules - offset
    );

    let scorer = TemplateScorer::from_path("data/template_scorer.onnx", n_rules, offset)
        .expect("load ONNX scorer");

    let search_config = SearchConfig {
        max_depth: 5,
        max_routes: 1,
        beam_width: 100,
        ..Default::default()
    };

    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    let targets: Vec<&str> = input
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.split('\t').next().unwrap_or(l))
        .take(max_targets)
        .collect();

    // (depth, root_rank, node_rank) for every ground-truth extracted-template step.
    let mut samples: Vec<(u32, usize, usize)> = Vec::new();
    let mut n_solved = 0usize;
    let mut n_default_rule_steps = 0usize;

    for (i, &target) in targets.iter().enumerate() {
        eprintln!("[{}/{}] {target}", i + 1, targets.len());
        let Ok((routes, _stats)) = find_routes(target, &env, &rules, &search_config) else {
            continue;
        };
        let Some(route) = routes.first() else {
            continue;
        };
        n_solved += 1;

        // Root-only ranking is computed once per route, against the ORIGINAL
        // root target SMILES — exactly matching what `find_routes` itself does
        // (search.rs computes `ranked_rules` once before the A* loop).
        let root_ranks = file_template_ranks(&scorer, target, offset, n_rules);

        for (depth, step) in route.steps.iter().enumerate() {
            let Some(idx_str) = step.rule.strip_prefix("extracted_") else {
                n_default_rule_steps += 1;
                continue;
            };
            let Ok(tmpl_idx) = idx_str.parse::<usize>() else {
                continue;
            };
            if tmpl_idx >= root_ranks.len() {
                continue;
            }
            let root_rank = root_ranks[tmpl_idx];
            let node_ranks = file_template_ranks(&scorer, &step.target, offset, n_rules);
            let node_rank = node_ranks[tmpl_idx];
            samples.push((depth as u32, root_rank, node_rank));
            println!(
                "SAMPLE\tdepth={depth}\trule={}\troot_rank={root_rank}\tnode_rank={node_rank}\troot={target}\tintermediate={}",
                step.rule, step.target
            );
        }
    }

    eprintln!(
        "\n{n_solved}/{} targets solved; {} extracted-template steps; {n_default_rule_steps} hand-crafted-rule steps (excluded, always top-{offset})",
        targets.len(),
        samples.len()
    );

    type Filter = fn(&(u32, usize, usize)) -> bool;
    let strata: [(&str, Filter); 3] = [
        ("ALL depths", |_| true),
        (
            "depth == 0 (root step; sanity check, must be ~identical)",
            |s| s.0 == 0,
        ),
        (
            "depth >= 1 (deep intermediates; the interesting case)",
            |s| s.0 >= 1,
        ),
    ];
    for (label, filt) in strata {
        let root_ranks: Vec<usize> = samples.iter().filter(|s| filt(s)).map(|s| s.1).collect();
        let node_ranks: Vec<usize> = samples.iter().filter(|s| filt(s)).map(|s| s.2).collect();
        println!("\n=== {label} (n={}) ===", root_ranks.len());
        for k in [1usize, 10, 50, 100] {
            println!(
                "  top-{k:<3} recall:  root-only={:>6.1}%   per-node={:>6.1}%   delta={:>+6.1}pp",
                100.0 * recall_at(&root_ranks, k),
                100.0 * recall_at(&node_ranks, k),
                100.0 * (recall_at(&node_ranks, k) - recall_at(&root_ranks, k)),
            );
        }
        if !root_ranks.is_empty() {
            let mut rr = root_ranks.clone();
            let mut nr = node_ranks.clone();
            rr.sort_unstable();
            nr.sort_unstable();
            println!(
                "  median rank:      root-only={}   per-node={}",
                rr[rr.len() / 2],
                nr[nr.len() / 2]
            );
        }
    }
}
