//! Process-level tests for `--search-mode coverage` (Phase 41.18B): spawn
//! the real `renkin` binary and check actual stdout/stderr/exit code, not
//! just library internals -- mirrors `tests/search_diagnostics_cli.rs`'s
//! convention.
//!
//! Deliberately does not use `data/phase_a5_template_scaling/templates/
//! templates_2000.smi` (the real Phase B.2 coverage template set) here --
//! that run takes ~10-20s per invocation even in release mode, too slow to
//! run twice per test across a dozen tests in CI. Manual verification
//! against the real 2,000-template set and a real Phase B.1/B.2 target
//! (`uspto50k_val#L2628`, the fastest known newly-solved target in the
//! committed decision-run data) is recorded in the PR description instead.
//! These tests use `data/templates_extracted_500.smi` (already small,
//! already committed, already used throughout the reranker/coverage-mode
//! research program) as a fast, real "coverage" set, paired with a target
//! hand-picked to be unsolved by `default_rules()` alone but solved once
//! `--templates`/`--coverage-templates` adds the 500-template file, at a
//! shallow depth -- exercises the real Stage-1/Stage-2 control flow in
//! well under a second per run, no synthetic/fake chemistry needed.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_renkin")
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(bin())
        .args(args)
        .output()
        .expect("failed to spawn renkin")
}

fn run_json(args: &[&str]) -> serde_json::Value {
    let out = run(args);
    assert!(
        out.status.success(),
        "renkin exited non-zero: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("stdout must be valid JSON ({e}): {out:?}"))
}

const BUILDING_BLOCK: &str = "CC(=O)O"; // acetic acid: depth-0 building block
// Unsolved by default_rules() alone at depth=2, solved once the 500-template
// file is loaded (either as --templates or --coverage-templates) -- see
// module doc for how this was chosen.
const STAGE1_UNSOLVED_AT_500: &str = "O=C1CCC(=O)N1c1ccccc1";
const TEMPLATES_500: &str = "data/templates_extracted_500.smi";

const COVERAGE_MODE_KEYS: &[&str] = &[
    "search_mode",
    "selected_stage",
    "stage2_invoked",
    "stage1_timeout",
    "stage2_timeout",
    "stage1_elapsed_ms",
    "stage2_elapsed_ms",
    "total_elapsed_ms",
];

#[test]
fn standard_mode_output_has_no_coverage_mode_keys() {
    let v = run_json(&[
        "--target",
        BUILDING_BLOCK,
        "--depth",
        "2",
        "--max-routes",
        "1",
    ]);
    for key in COVERAGE_MODE_KEYS {
        assert!(
            v.get(key).is_none(),
            "standard mode must never emit {key}: {v}"
        );
    }
    // Exact pre-existing key set, unchanged.
    let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "joint_success_probability",
            "routes",
            "routes_found",
            "target"
        ]
    );
}

#[test]
fn invalid_search_mode_fails() {
    let out = run(&["--target", BUILDING_BLOCK, "--search-mode", "bogus"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--search-mode"), "stderr: {stderr}");
}

#[test]
fn missing_coverage_templates_path_fails_before_search() {
    let out = run(&["--target", BUILDING_BLOCK, "--search-mode", "coverage"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--coverage-templates"), "stderr: {stderr}");
}

#[test]
fn unreadable_coverage_templates_path_fails_before_search() {
    let out = run(&[
        "--target",
        BUILDING_BLOCK,
        "--search-mode",
        "coverage",
        "--coverage-templates",
        "/nonexistent/renkin_coverage_mode_test_path.smi",
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--coverage-templates"), "stderr: {stderr}");
}

#[test]
fn coverage_flags_in_standard_mode_fail() {
    let out = run(&[
        "--target",
        BUILDING_BLOCK,
        "--coverage-templates",
        TEMPLATES_500,
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--search-mode coverage"),
        "stderr: {stderr}"
    );

    let out = run(&["--target", BUILDING_BLOCK, "--coverage-timeout-secs", "10"]);
    assert!(!out.status.success());
}

#[test]
fn coverage_timeout_zero_fails() {
    let out = run(&[
        "--target",
        BUILDING_BLOCK,
        "--search-mode",
        "coverage",
        "--coverage-templates",
        TEMPLATES_500,
        "--coverage-timeout-secs",
        "0",
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--coverage-timeout-secs"),
        "stderr: {stderr}"
    );
}

#[test]
fn coverage_timeout_non_integer_fails() {
    for bad in ["abc", "12.5", "-3"] {
        let out = run(&[
            "--target",
            BUILDING_BLOCK,
            "--search-mode",
            "coverage",
            "--coverage-templates",
            TEMPLATES_500,
            "--coverage-timeout-secs",
            bad,
        ]);
        assert!(!out.status.success(), "expected failure for {bad:?}");
    }
}

#[test]
fn bond_index_with_coverage_mode_fails_loud() {
    let out = run(&[
        "--target",
        BUILDING_BLOCK,
        "--search-mode",
        "coverage",
        "--coverage-templates",
        TEMPLATES_500,
        "--bond-index",
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("bond-index") || stderr.contains("bond_index"),
        "stderr: {stderr}"
    );
}

#[test]
fn stage1_solved_reports_stage2_invoked_false() {
    let v = run_json(&[
        "--target",
        BUILDING_BLOCK,
        "--depth",
        "2",
        "--max-routes",
        "1",
        "--search-mode",
        "coverage",
        "--coverage-templates",
        TEMPLATES_500,
    ]);
    assert_eq!(v["routes_found"], 1);
    assert_eq!(v["search_mode"], "coverage");
    assert_eq!(v["selected_stage"], "stage1");
    assert_eq!(v["stage2_invoked"], false);
    assert!(v.get("stage2_elapsed_ms").is_none() || v["stage2_elapsed_ms"].is_null());
}

#[test]
fn stage1_unsolved_reports_stage2_invoked_true() {
    let v = run_json(&[
        "--target",
        STAGE1_UNSOLVED_AT_500,
        "--depth",
        "2",
        "--max-routes",
        "1",
        "--beam-width",
        "100",
        "--search-mode",
        "coverage",
        "--coverage-templates",
        TEMPLATES_500,
    ]);
    assert_eq!(v["selected_stage"], "stage2");
    assert_eq!(v["stage2_invoked"], true);
    assert_eq!(v["routes_found"], 1);
    assert!(v["stage2_elapsed_ms"].as_f64().unwrap() >= 0.0);
    assert!(v["total_elapsed_ms"].as_f64().unwrap() >= v["stage1_elapsed_ms"].as_f64().unwrap());
}

#[test]
fn stage1_and_stage2_both_unsolved_reports_no_routes_with_coverage_fields() {
    // A target unsolved even with the 500-template file -- exercises the
    // routes.is_empty() JSON branch (raw json!(), not the Output struct)
    // with coverage-mode fields attached.
    let v = run_json(&[
        "--target",
        "c1ccc2c(c1)oc1ccccc12", // dibenzofuran -- confirmed unsolved by default_rules() and by +500 templates at shallow depth
        "--depth",
        "2",
        "--max-routes",
        "1",
        "--beam-width",
        "100",
        "--search-mode",
        "coverage",
        "--coverage-templates",
        TEMPLATES_500,
    ]);
    assert_eq!(v["routes_found"], 0);
    assert_eq!(v["search_mode"], "coverage");
    assert_eq!(v["selected_stage"], "stage2");
    assert_eq!(v["stage2_invoked"], true);
    assert!(
        v.get("diagnostics").is_some(),
        "pre-existing diagnostics block must still be present: {v}"
    );
}
