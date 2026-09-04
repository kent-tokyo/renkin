//! Phase 3A ground-truth validation: do real labels actually land in the
//! same coordinate system as `propose_one_step`'s candidate output? Not a
//! permanent test -- a one-off check kept here for reproducibility across
//! all three label corpora (formal test, train, val -- see Round 2's
//! Section I "training corpus preflight"). Usage:
//!
//!   cargo run --release --example label_spotcheck
//!   cargo run --release --example label_spotcheck -- \
//!       --labels data/reranker_labels_uspto50k_train.jsonl \
//!       --targets data/reranker_targets_uspto50k_train.jsonl \
//!       --sample-size 30

use renkin::candidate::{ProposalConfig, propose_one_step};
use renkin::chem_env::load_rules_from_file;
use std::collections::{BTreeSet, HashMap};
use std::fs;

fn arg_value(flag: &str, default: &str) -> String {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| default.to_string())
}

fn main() {
    let labels_path = arg_value("--labels", "data/reranker_labels_uspto50k_test.jsonl");
    let targets_path = arg_value("--targets", "data/comparison/sample_full_sorted.jsonl");
    let sample_size: usize = arg_value("--sample-size", "40")
        .parse()
        .expect("--sample-size must be an integer");

    let rules = load_rules_from_file("data/templates_extracted_500.smi");
    println!("loaded {} rules", rules.len());

    let target_smiles_by_id = load_target_lookup(&targets_path);

    let content = fs::read_to_string(&labels_path).expect("read labels");
    let sample: Vec<serde_json::Value> = content
        .lines()
        .take(sample_size)
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();

    let mut n_checked = 0;
    let mut n_any_candidate_match = 0;
    let mut n_zero_candidates = 0;

    for row in &sample {
        let target_id = row["target_id"].as_str().unwrap();
        let target_smiles = target_smiles_by_id
            .get(target_id)
            .unwrap_or_else(|| panic!("target_id {target_id} not found in {targets_path}"));
        let correct_sets: Vec<BTreeSet<String>> = row["correct_precursor_sets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| {
                s.as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_string())
                    .collect()
            })
            .collect();

        let config = ProposalConfig::default();
        let pool = match propose_one_step(target_id, target_smiles, &rules, &config) {
            Ok(p) => p,
            Err(e) => {
                println!("{target_id}: propose_one_step ERROR: {e}");
                continue;
            }
        };
        n_checked += 1;
        if pool.candidates.is_empty() {
            n_zero_candidates += 1;
        }
        let matched = pool.candidates.iter().any(|c| {
            let cset: BTreeSet<String> = c.precursor_smiles.iter().cloned().collect();
            correct_sets.contains(&cset)
        });
        if matched {
            n_any_candidate_match += 1;
        }
        println!(
            "{target_id}: {} candidates, ground_truth_reachable={}",
            pool.candidates.len(),
            matched
        );
    }

    println!(
        "\nsummary ({labels_path}): checked={n_checked} zero_candidates={n_zero_candidates} \
         ground_truth_reachable={n_any_candidate_match}/{n_checked}"
    );
}

/// Loads a target_id -> canonical_smiles lookup from either
/// `sample_full_sorted.jsonl` (the formal test corpus, field name
/// `canonical_smiles`) or `generate_train_val_labels.py`'s own targets
/// output (same field name) -- both share the same JSONL shape.
fn load_target_lookup(path: &str) -> HashMap<String, String> {
    let content = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    content
        .lines()
        .map(|line| {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            (
                v["target_id"].as_str().unwrap().to_string(),
                v["canonical_smiles"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}
