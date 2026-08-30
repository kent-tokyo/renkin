//! Validator-accuracy measurement (v0.37.0, `docs/design/validator-
//! accuracy-measurement-v0.md`): measures `validation::validate_step`
//! only (never `bridge::forward::validate_step_forward`, the two are
//! deliberately not unified -- see that design doc §1.1).
//!
//! **True-accept side** (design doc §2(b), the "attribution-free probe"):
//! for each of `data/reranker_labels_uspto50k_test.jsonl`'s 4,903 real,
//! human/USPTO-derived `(target, correct_precursor_set)` pairs, construct
//! a candidate `ReactionStep` under every one of `default_rules()`'s 21
//! hand-crafted rule names in turn and call `validate_step` directly --
//! pure in-process function calls, no search, no subprocess. If **any**
//! attribution makes `validate_step` return `Valid`, the validator
//! recognizes this genuine disconnection as valid under *some* labeling.
//!
//! Reported split by validation branch (`validate_step`'s own two
//! internal mechanisms have very different discriminative power, so
//! blending them into one number would repeat exactly the mistake the
//! design doc forbids between `validate_step`/`validate_step_forward`):
//! `Valid` via a SMIRKS rule (`forward::rule_reproduces` reverses *that
//! rule's own* SMIRKS -- genuinely rule-specific) vs. `Valid` only via a
//! graph rule (`validate_graph_step`'s small delta table -- see the
//! true-reject section below for why this branch is structurally weaker).
//!
//! **True-reject side**: the design doc's own premise ("reusing the
//! already-existing confirmed-wrong cases") does not hold -- see
//! `data/validator_accuracy_probe_2026-08-30/findings.md` for the full
//! writeup. In short: `validate_graph_step` (`src/validation/
//! graph_rules.rs:121-129`) deliberately maps `ester_cleavage`/
//! `amide_cleavage`/`aryl_ether_retro` to one shared `ESTER_AMIDE_DELTA`
//! (documented: "formally the same hydrolysis-shaped delta... confirmed
//! by direct atom counting"), so a wrong attribution *within* that bucket
//! is structurally unrejectable by this check, not merely untested --
//! this project's own existing `aryl_ether_retro_skips_*` fixtures
//! (`chem_env.rs`) test a different layer (`apply_retro`'s own generation
//! guard) and don't serve as `validate_step` negatives at all. The one
//! genuinely confusable graph-rule pair with *distinguishable* deltas is
//! `boc_deprotection_retro` (`-C5H8O2`) vs. `cbz_deprotection_retro`
//! (`-C8H6O2`) -- both directions constructed and verified here.

use renkin::chem_env::default_rules;
use renkin::search::{AtomEconomyStatus, ReactionStep, reaction_family_for_rule};
use renkin::validation::{StepValidationStatus, validate_step};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::io::Write;

const LABEL_PATH: &str = "data/reranker_labels_uspto50k_test.jsonl";
const OUTPUT_DIR: &str = "data/validator_accuracy_probe_2026-08-30";

#[derive(Deserialize)]
struct LabelRow {
    group_id: String,
    /// Canonical target SMILES -- this project's own established
    /// convention (`candidate::CandidatePool`'s own doc comment), not an
    /// opaque id despite the field name.
    target_id: String,
    correct_precursor_sets: Vec<Vec<String>>,
}

fn step(rule: &str, template_id: &str, target: &str, precursors: &[String]) -> ReactionStep {
    ReactionStep {
        rule: rule.to_string(),
        template_id: template_id.to_string(),
        target: target.to_string(),
        precursors: precursors.to_vec(),
        conditions: None,
        atom_economy: None,
        atom_economy_raw_percent: None,
        atom_economy_status: AtomEconomyStatus::NotEvaluable,
        step_confidence: 1.0,
        procedure_hint: None,
        reaction_family: None,
        metadata_source: None,
        metadata_scope: None,
        evidence: None,
    }
}

fn true_reject_check() {
    let rules = default_rules();
    println!("\n=== true-reject: boc/cbz cross-attribution (the one distinguishable pair) ===");

    let boc_target = "CC(C)(C)OC(=O)Nc1ccccc1"; // Boc-protected aniline
    let cbz_target = "O=C(OCc1ccccc1)Nc1ccccc1"; // Cbz-protected aniline
    let amine = vec!["Nc1ccccc1".to_string()];

    let cases = [
        (
            "boc_deprotection_retro (correct)",
            boc_target,
            "boc_deprotection_retro",
        ),
        (
            "boc target x cbz rule (wrong)",
            boc_target,
            "cbz_deprotection_retro",
        ),
        (
            "cbz_deprotection_retro (correct)",
            cbz_target,
            "cbz_deprotection_retro",
        ),
        (
            "cbz target x boc rule (wrong)",
            cbz_target,
            "boc_deprotection_retro",
        ),
    ];
    for (label, target, rule_name) in cases {
        let s = step(rule_name, &format!("rule:{rule_name}"), target, &amine);
        let status = validate_step(&s, &rules);
        println!("{label}: {status:?}");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let limit: Option<usize> = args.get(1).and_then(|s| s.parse().ok());

    true_reject_check();

    let rules = default_rules();
    let content = std::fs::read_to_string(LABEL_PATH)
        .unwrap_or_else(|e| panic!("failed to read {LABEL_PATH}: {e} -- run from the crate root"));
    let mut rows: Vec<LabelRow> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("valid label row JSON"))
        .collect();
    if let Some(n) = limit {
        rows.truncate(n);
    }

    std::fs::create_dir_all(OUTPUT_DIR).expect("create output dir");
    let mut jsonl =
        std::fs::File::create(format!("{OUTPUT_DIR}/rows.jsonl")).expect("create rows.jsonl");

    let mut n_any_valid = 0usize;
    let mut n_valid_via_smirks = 0usize;
    let mut n_valid_via_graph_only = 0usize;
    let mut n_all_not_evaluable = 0usize;
    let mut family_hits: BTreeMap<&'static str, usize> = BTreeMap::new();

    let t0 = std::time::Instant::now();
    for (i, row) in rows.iter().enumerate() {
        let mut any_valid = false;
        let mut valid_via_smirks = false;
        let mut valid_via_graph = false;
        let mut any_evaluated = false;

        for rule in &rules {
            let mut rule_status = StepValidationStatus::NotEvaluable;
            for precursors in &row.correct_precursor_sets {
                let s = step(&rule.name, &rule.template_id, &row.target_id, precursors);
                let status = validate_step(&s, &rules);
                if status != StepValidationStatus::NotEvaluable {
                    any_evaluated = true;
                }
                if status == StepValidationStatus::Valid {
                    rule_status = StepValidationStatus::Valid;
                    break;
                }
                if rule_status != StepValidationStatus::Valid {
                    rule_status = status;
                }
            }
            if rule_status == StepValidationStatus::Valid {
                any_valid = true;
                if rule.smirks.is_empty() {
                    valid_via_graph = true;
                } else {
                    valid_via_smirks = true;
                }
                if let Some(family) = reaction_family_for_rule(&rule.name) {
                    *family_hits.entry(family).or_insert(0) += 1;
                }
            }
            writeln!(
                jsonl,
                "{}",
                serde_json::json!({
                    "target_id": row.group_id,
                    "rule_attribution_tried": rule.name,
                    "validate_step_result": format!("{rule_status:?}").to_lowercase(),
                    "matches_any_correct_set": rule_status == StepValidationStatus::Valid,
                })
            )
            .expect("write jsonl row");
        }

        if any_valid {
            n_any_valid += 1;
            if valid_via_smirks {
                n_valid_via_smirks += 1;
            } else if valid_via_graph {
                n_valid_via_graph_only += 1;
            }
        } else if !any_evaluated {
            n_all_not_evaluable += 1;
        }

        if (i + 1) % 500 == 0 {
            eprintln!(
                "{}/{} targets, {:.1}s elapsed",
                i + 1,
                rows.len(),
                t0.elapsed().as_secs_f64()
            );
        }
    }
    let elapsed = t0.elapsed().as_secs_f64();

    let n = rows.len();
    let summary = serde_json::json!({
        "n_targets": n,
        "elapsed_secs": elapsed,
        "true_accept_rate": {
            "denominator_kind": "all_labeled_targets",
            "n_denominator": n,
            "n_numerator": n_any_valid,
            "value": n_any_valid as f64 / n as f64,
        },
        "true_accept_via_smirks_rule": {
            "denominator_kind": "all_labeled_targets",
            "n_denominator": n,
            "n_numerator": n_valid_via_smirks,
            "value": n_valid_via_smirks as f64 / n as f64,
        },
        "true_accept_via_graph_rule_only": {
            "denominator_kind": "all_labeled_targets",
            "n_denominator": n,
            "n_numerator": n_valid_via_graph_only,
            "value": n_valid_via_graph_only as f64 / n as f64,
        },
        "all_not_evaluable": {
            "denominator_kind": "all_labeled_targets",
            "n_denominator": n,
            "n_numerator": n_all_not_evaluable,
            "value": n_all_not_evaluable as f64 / n as f64,
        },
        "reaction_family_hit_counts_caveat": "counts are grouped by RENKIN's own asserted reaction_family_for_rule, not an independent corpus-native class label (design doc §1.3) -- 'broken down by what RENKIN itself claims the reaction is', not validated against a third-party taxonomy",
        "reaction_family_hit_counts": family_hits,
    });

    std::fs::write(
        format!("{OUTPUT_DIR}/summary.json"),
        serde_json::to_string_pretty(&summary).unwrap(),
    )
    .expect("write summary.json");

    println!("\n=== true-accept summary ===");
    println!("{}", serde_json::to_string_pretty(&summary).unwrap());
}
