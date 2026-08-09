//! Corpus-wide audit of Issue #88's `[#N]` hash-atom compatibility fix:
//! reports, per extracted-template file, how many raw template lines exist,
//! how many `load_rules_from_file` actually accepts as logical `RetroRule`s
//! (a small, unrelated fraction of lines can fail `parse_smarts` for reasons
//! that have nothing to do with `[#N]` -- see Issue #88), and, among the
//! *loaded* rules, how many are directly usable, how many need hash-atom
//! variant expansion (and how many variants), and how many are still
//! unsupported and why. Run with no arguments to check the two
//! checked-in/locally-generated template files this project uses
//! (`data/templates_extracted.smi`, the frozen benchmark corpus, and
//! `data/templates_extracted_5000.smi`, an optional, locally generated
//! 5,000-template corpus that RENKIN only loads when explicitly passed via
//! `--templates` -- gitignored, so silently skipped if not present); pass
//! explicit paths to check others.
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
    let raw_template_lines = content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .count();

    // The one source of truth for "does this line become a usable
    // RetroRule at all": load_rules_from_file itself. A raw line can fail
    // to load for reasons unrelated to [#N] (Issue #88 found one such line
    // in the 5,000-template file -- a disconnected-fragment reactant SMARTS
    // that parse_smarts rejects outright). Iterating the *loaded* rules
    // here, instead of re-deriving load-acceptance from the raw lines,
    // guarantees this tool can never count a load-rejected line as
    // "supported" by mistake.
    let rules = load_rules_from_file(path);
    let logical_rules_loaded = rules.len();
    let load_rejected = raw_template_lines.saturating_sub(logical_rules_loaded);

    let mut direct_loaded = 0usize;
    let mut hash_atom_supported_loaded = 0usize;
    let mut hash_atom_unsupported_loaded = 0usize;
    let mut variant_count_hist: BTreeMap<usize, usize> = BTreeMap::new();
    let mut unsupported_by_reason: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut variant_limit_exceeded_examples: Vec<(usize, String)> = Vec::new();

    for rule in &rules {
        match concrete_application_status(&rule.smirks) {
            ConcreteApplicationStatus::Direct => direct_loaded += 1,
            ConcreteApplicationStatus::HashAtomVariants { variant_count } => {
                hash_atom_supported_loaded += 1;
                *variant_count_hist.entry(variant_count).or_insert(0) += 1;
            }
            ConcreteApplicationStatus::Unsupported { reason } => {
                hash_atom_unsupported_loaded += 1;
                let key = match reason {
                    HashAtomUnsupportedReason::UnhandledSyntax => "unhandled_syntax",
                    HashAtomUnsupportedReason::InconsistentElement => "inconsistent_element",
                    HashAtomUnsupportedReason::VariantLimitExceeded { total_combinations } => {
                        variant_limit_exceeded_examples
                            .push((total_combinations, rule.smirks.clone()));
                        "variant_limit_exceeded"
                    }
                    HashAtomUnsupportedReason::NoValidVariant => "no_valid_variant",
                };
                *unsupported_by_reason.entry(key).or_insert(0) += 1;
            }
        }
    }

    let concrete_supported_among_loaded = direct_loaded + hash_atom_supported_loaded;
    // "raw" here means: out of every line in the file, including the ones
    // load_rules_from_file itself already rejects for unrelated reasons.
    // This is the honest end-to-end denominator -- do not report supported
    // counts against raw_template_lines without subtracting load_rejected
    // first, or a load-time rejection silently gets counted as a win.
    let end_to_end_usable_raw = concrete_supported_among_loaded;

    println!("=== {path} ===");
    println!("raw_template_lines: {raw_template_lines}");
    println!(
        "logical_rules_loaded (load_rules_from_file): {logical_rules_loaded}  \
         [load_rejected: {load_rejected}, for reasons unrelated to Issue #88 unless noted]"
    );
    println!("  among loaded rules:");
    println!("    direct_loaded (no hash atom): {direct_loaded}");
    println!("    hash_atom_supported_loaded: {hash_atom_supported_loaded}");
    println!("      variant_count distribution: {variant_count_hist:?}");
    println!("    hash_atom_unsupported_loaded: {hash_atom_unsupported_loaded}");
    println!("      by reason: {unsupported_by_reason:?}");
    if !variant_limit_exceeded_examples.is_empty() {
        variant_limit_exceeded_examples.sort_by_key(|(n, _)| *n);
        println!(
            "      variant_limit_exceeded total_combinations range: {}..={}",
            variant_limit_exceeded_examples.first().unwrap().0,
            variant_limit_exceeded_examples.last().unwrap().0
        );
    }
    println!(
        "concrete_supported_among_loaded: {concrete_supported_among_loaded}/{logical_rules_loaded} ({:.1}%)",
        100.0 * concrete_supported_among_loaded as f64 / logical_rules_loaded.max(1) as f64
    );
    println!(
        "end_to_end_usable_raw: {end_to_end_usable_raw}/{raw_template_lines} ({:.1}%)\n",
        100.0 * end_to_end_usable_raw as f64 / raw_template_lines.max(1) as f64
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
