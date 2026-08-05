//! Corpus-wide audit of Issue #88's `[#N]` hash-atom compatibility fix:
//! reports, per extracted-template file, how many templates are directly
//! usable, how many need hash-atom variant expansion (and how many
//! variants), and how many are still unsupported and why. Run with no
//! arguments to check the two checked-in/locally-generated template
//! files this project uses (`data/templates_extracted.smi`, the frozen
//! benchmark corpus, and `data/templates_extracted_5000.smi`, the
//! production default -- gitignored/locally generated, so silently
//! skipped if not present); pass explicit paths to check others.
use renkin::chem_env::{
    ConcreteApplicationStatus, HashAtomUnsupportedReason, concrete_application_status,
    load_rules_from_file,
};
use std::collections::BTreeMap;

fn analyze(path: &str) {
    let Ok(content) = std::fs::read_to_string(path) else {
        println!("=== {path} === (skipped: not found)\n");
        return;
    };
    let lines: Vec<&str> = content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();

    let mut direct = 0usize;
    let mut hash_variants = 0usize;
    let mut variant_count_hist: BTreeMap<usize, usize> = BTreeMap::new();
    let mut unsupported_by_reason: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut variant_limit_exceeded_examples: Vec<(usize, String)> = Vec::new();

    for line in &lines {
        let smirks = line.split('\t').next().unwrap_or(line);
        match concrete_application_status(smirks) {
            ConcreteApplicationStatus::Direct => direct += 1,
            ConcreteApplicationStatus::HashAtomVariants { variant_count } => {
                hash_variants += 1;
                *variant_count_hist.entry(variant_count).or_insert(0) += 1;
            }
            ConcreteApplicationStatus::Unsupported { reason } => {
                let key = match reason {
                    HashAtomUnsupportedReason::UnhandledSyntax => "unhandled_syntax",
                    HashAtomUnsupportedReason::InconsistentElement => "inconsistent_element",
                    HashAtomUnsupportedReason::VariantLimitExceeded { total_combinations } => {
                        variant_limit_exceeded_examples
                            .push((total_combinations, smirks.to_string()));
                        "variant_limit_exceeded"
                    }
                    HashAtomUnsupportedReason::NoValidVariant => "no_valid_variant",
                };
                *unsupported_by_reason.entry(key).or_insert(0) += 1;
            }
        }
    }

    let logical_rule_count = load_rules_from_file(path).len();
    let supported = direct + hash_variants;

    println!("=== {path} ===");
    println!("raw lines: {}", lines.len());
    println!(
        "logical RetroRule count (load_rules_from_file, unchanged by #88): {logical_rule_count}"
    );
    println!("direct (no hash atom): {direct}");
    println!("hash-atom-variant-supported: {hash_variants}");
    println!("  variant_count distribution: {variant_count_hist:?}");
    println!("unsupported by reason: {unsupported_by_reason:?}");
    if !variant_limit_exceeded_examples.is_empty() {
        variant_limit_exceeded_examples.sort_by_key(|(n, _)| *n);
        println!(
            "  variant_limit_exceeded total_combinations range: {}..={}",
            variant_limit_exceeded_examples.first().unwrap().0,
            variant_limit_exceeded_examples.last().unwrap().0
        );
    }
    println!(
        "concrete-application-supported: {supported}/{} ({:.1}%)\n",
        lines.len(),
        100.0 * supported as f64 / lines.len() as f64
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let paths: Vec<String> = if args.is_empty() {
        vec![
            "data/templates_extracted.smi".to_string(),
            "data/templates_extracted_5000.smi".to_string(),
        ]
    } else {
        args
    };
    for path in paths {
        analyze(&path);
    }
}
