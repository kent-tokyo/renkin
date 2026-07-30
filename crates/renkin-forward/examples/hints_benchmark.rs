//! Deterministic performance report for `renkin-forward hints` across
//! increasingly large template corpora and reactant sizes.
//!
//! Usage:
//!   cargo run --release -p renkin-forward --example hints_benchmark
//!
//! Not a pass/fail CI gate (wall-clock varies run to run and machine to
//! machine) -- a deterministic data generator for a PR-body comparison,
//! matching this repo's own `apply_retro_perf_gate.rs` convention: SHA-256
//! template-file provenance, git HEAD SHA, explicit warmup/measured counts,
//! and full per-case stats alongside timing.
//!
//! `slot_match_sites_reported` is computed post-hoc from the public report
//! (summed `match_sites.len()` across every hint's `known_assignments`) --
//! it is a real count of *reported* match sites, not an internal call
//! count of every `find_matches` invocation attempted (that finer
//! instrumentation was judged not worth threading a stats accumulator
//! through the internal matching path for this round; `smarts_components_parsed`
//! is tracked exactly, and is the more direct cost driver of that path).

use std::time::Instant;

use renkin::chem_env::{RetroRule, default_rules, load_rules_from_file};
use renkin_forward::hints::{HintGenerationConfig, generate_retrieval_hints};
use sha2::{Digest, Sha256};

fn sha256_file(path: &str) -> String {
    let bytes = std::fs::read(path).expect("template file must exist");
    format!("{:x}", Sha256::digest(&bytes))
}

fn git_head_sha() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown (not run inside a git checkout)".to_string())
}

struct Case {
    label: &'static str,
    rules: Vec<RetroRule>,
    reactants: Vec<&'static str>,
}

const WARMUP_RUNS: usize = 1;
const MEASURED_RUNS: usize = 7;

fn percentile(sorted_ms: &[f64], p: f64) -> f64 {
    let idx = ((sorted_ms.len() as f64 - 1.0) * p).round() as usize;
    sorted_ms[idx.min(sorted_ms.len() - 1)]
}

fn main() {
    let extracted_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../data/templates_extracted.smi"
    );
    let template_file_sha256 = sha256_file(extracted_path);
    let all_extracted = load_rules_from_file(extracted_path);
    let extracted_100: Vec<RetroRule> = all_extracted.iter().take(100).cloned().collect();
    let extracted_full = all_extracted.clone();

    // Small: a single aromatic ring with one substituent.
    let small = "Brc1ccccc1";
    // Medium: several functional groups, two rings, an amide linkage.
    let medium = "COc1ccc(Br)cc1C(=O)Nc1ccccc1";
    // Large: multiple rings, a carbamate-protected amine, an ether linkage,
    // an amide -- representative of a moderately complex drug-like molecule.
    let large = "CC(C)(C)OC(=O)N1CCC(Oc2ccc(-c3ccc(C(=O)Nc4ccc(Br)cc4)cc3)cc2)CC1";

    let cases = vec![
        Case {
            label: "embedded_28__small",
            rules: default_rules(),
            reactants: vec![small],
        },
        Case {
            label: "embedded_28__medium",
            rules: default_rules(),
            reactants: vec![medium],
        },
        Case {
            label: "embedded_28__large",
            rules: default_rules(),
            reactants: vec![large],
        },
        Case {
            label: "extracted_100__small",
            rules: extracted_100.clone(),
            reactants: vec![small],
        },
        Case {
            label: "extracted_100__medium",
            rules: extracted_100.clone(),
            reactants: vec![medium],
        },
        Case {
            label: "extracted_100__large",
            rules: extracted_100.clone(),
            reactants: vec![large],
        },
        Case {
            label: "extracted_full__small",
            rules: extracted_full.clone(),
            reactants: vec![small],
        },
        Case {
            label: "extracted_full__medium",
            rules: extracted_full.clone(),
            reactants: vec![medium],
        },
        Case {
            label: "extracted_full__large",
            rules: extracted_full.clone(),
            reactants: vec![large],
        },
        Case {
            label: "extracted_full__two_known_reactants",
            rules: extracted_full,
            reactants: vec![small, medium],
        },
    ];

    println!("git_head_sha: {}", git_head_sha());
    println!("templates_extracted_file: {extracted_path}");
    println!("templates_extracted_sha256: {template_file_sha256}");
    println!("templates_extracted_total_rules: {}", all_extracted.len());
    println!("warmup_runs: {WARMUP_RUNS}");
    println!("measured_runs: {MEASURED_RUNS}");
    println!();

    let config = HintGenerationConfig::default();

    for case in &cases {
        for _ in 0..WARMUP_RUNS {
            let _ = generate_retrieval_hints(&case.reactants, &case.rules, &config);
        }

        let mut elapsed_ms = Vec::with_capacity(MEASURED_RUNS);
        let mut last_report = None;
        for _ in 0..MEASURED_RUNS {
            let start = Instant::now();
            let report = generate_retrieval_hints(&case.reactants, &case.rules, &config).unwrap();
            elapsed_ms.push(start.elapsed().as_secs_f64() * 1000.0);
            last_report = Some(report);
        }
        elapsed_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let report = last_report.unwrap();
        let slot_match_sites_reported: usize = report
            .hints
            .iter()
            .flat_map(|h| h.known_assignments.iter())
            .map(|ka| ka.match_sites.len())
            .sum();

        println!("case: {}", case.label);
        println!("  reactants: {:?}", case.reactants);
        println!("  rules_loaded: {}", report.stats.rules_loaded);
        println!(
            "  templates_inspected: {}",
            report.stats.templates_inspected
        );
        println!(
            "  graph_rules_skipped: {}",
            report.stats.graph_rules_skipped
        );
        println!(
            "  template_parse_failed: {}",
            report.stats.template_parse_failed
        );
        println!(
            "  smarts_components_parsed: {}",
            report.stats.smarts_components_parsed
        );
        println!(
            "  assignments_generated: {}",
            report.stats.assignments_generated
        );
        println!(
            "  templates_with_assignments_truncated: {}",
            report.stats.templates_with_assignments_truncated
        );
        println!("  slot_match_sites_reported: {slot_match_sites_reported}");
        println!("  hints_before_merge: {}", report.stats.hints_before_merge);
        println!(
            "  duplicate_hints_merged: {}",
            report.stats.duplicate_hints_merged
        );
        println!("  hints_returned: {}", report.stats.hints_returned);
        println!("  hints_capped: {}", report.stats.hints_capped);
        println!("  elapsed_ms_p50: {:.3}", percentile(&elapsed_ms, 0.50));
        println!("  elapsed_ms_p95: {:.3}", percentile(&elapsed_ms, 0.95));
        println!("  elapsed_ms_max: {:.3}", elapsed_ms.last().unwrap());
        println!();
    }
}
