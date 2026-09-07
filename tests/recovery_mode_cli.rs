//! Process-level contract tests for Issue #239 staged recovery mode.

use std::process::Command;

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_renkin"))
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
    serde_json::from_slice(&out.stdout).expect("stdout must be JSON")
}

const STOCK: &str = "data/building_blocks.smi";
const COVERAGE: &str = "tests/fixtures/coverage_mode_templates.smi";
const COVERAGE_ONLY_TARGET: &str = "O=C1CCC(=O)N1c1ccccc1";
const BEAM_WIDTH_TARGET: &str = "c4cc(ccc4Br)-c3c1CN(CCc1n(n3)CC2OC2)C(=O)C";

#[test]
fn baseline_success_short_circuits_and_is_audited() {
    let value = run_json(&[
        "--target",
        "CC(=O)O",
        "--building-blocks",
        STOCK,
        "--depth",
        "2",
        "--beam-width",
        "100",
        "--max-routes",
        "1",
        "--search-mode",
        "recovery",
        "--beam-diversity-slots",
        "20",
    ]);
    assert_eq!(value["routes_found"], 1);
    assert_eq!(value["search_mode"], "recovery");
    assert_eq!(value["recovery"]["selected_stage"], "baseline");
    assert_eq!(value["recovery"]["recovered_route"], false);
    assert_eq!(value["recovery"]["attempts"].as_array().unwrap().len(), 1);
}

#[test]
fn depth_recovery_short_circuits_before_coverage() {
    let value = run_json(&[
        "--target",
        COVERAGE_ONLY_TARGET,
        "--building-blocks",
        STOCK,
        "--depth",
        "2",
        "--recovery-depth",
        "3",
        "--beam-width",
        "100",
        "--max-routes",
        "1",
        "--search-mode",
        "recovery",
        "--beam-diversity-slots",
        "20",
        "--recovery-coverage-tier",
        COVERAGE,
        "--coverage-templates",
        COVERAGE,
        "--coverage-timeout-secs",
        "5",
        "--coverage-beam-width",
        "100",
    ]);
    assert_eq!(value["routes_found"], 1);
    assert_eq!(value["recovery"]["selected_stage"], "depth");
    assert_eq!(value["recovery"]["recovered_route"], true);
    let attempts = value["recovery"]["attempts"].as_array().unwrap();
    assert_eq!(attempts.first().unwrap()["stage"], "baseline");
    assert_eq!(attempts.last().unwrap()["stage"], "depth");
    assert!(
        attempts
            .iter()
            .all(|attempt| attempt["stage"] != "coverage")
    );
    assert!(attempts.iter().all(|attempt| {
        attempt["termination"] == "completed"
            && attempt["rule_count"].as_u64().unwrap() > 0
            && attempt["rules_sha256"]
                .as_str()
                .is_some_and(|value| value.starts_with("sha256:"))
    }));
}

#[test]
fn wider_beam_recovery_short_circuits_before_depth() {
    let value = run_json(&[
        "--target",
        BEAM_WIDTH_TARGET,
        "--building-blocks",
        "data/comparison/shared_stock/shared_stock.smi",
        "--templates",
        "data/templates_extracted_500.smi",
        "--depth",
        "5",
        "--recovery-depth",
        "6",
        "--beam-width",
        "100",
        "--recovery-beam-width",
        "200",
        "--max-routes",
        "1",
        "--search-mode",
        "recovery",
    ]);
    assert_eq!(
        value["routes_found"], 0,
        "fixture remains unresolved: {value}"
    );
    assert_eq!(value["recovery"]["selected_stage"], "depth");
    assert_eq!(value["recovery"]["recovered_route"], false);
    let attempts = value["recovery"]["attempts"].as_array().unwrap();
    assert_eq!(attempts.len(), 3);
    assert_eq!(attempts[0]["beam_width"], 100);
    assert_eq!(attempts[1]["beam_width"], 200);
    assert_eq!(attempts[1]["trigger"], "baseline_beam_exhaustion");
    assert_eq!(attempts[2]["stage"], "depth");
}

#[test]
fn coverage_is_the_last_attempt_when_every_stage_fails() {
    let value = run_json(&[
        "--target",
        "c1ccc2c(c1)c1ccccc1c1ccccc21",
        "--building-blocks",
        STOCK,
        "--depth",
        "1",
        "--recovery-depth",
        "2",
        "--beam-width",
        "100",
        "--max-routes",
        "1",
        "--search-mode",
        "recovery",
        "--beam-diversity-slots",
        "20",
        "--recovery-coverage-tier",
        COVERAGE,
        "--coverage-templates",
        COVERAGE,
        "--coverage-timeout-secs",
        "5",
        "--coverage-beam-width",
        "100",
    ]);
    assert_eq!(value["routes_found"], 0);
    assert_eq!(value["search_mode"], "recovery");
    assert_eq!(value["recovery"]["selected_stage"], "coverage");
    assert_eq!(value["recovery"]["recovered_route"], false);
    let attempts = value["recovery"]["attempts"].as_array().unwrap();
    assert_eq!(attempts.first().unwrap()["stage"], "baseline");
    assert_eq!(attempts.last().unwrap()["stage"], "coverage");
    let coverage_attempts: Vec<_> = attempts
        .iter()
        .filter(|attempt| attempt["stage"] == "coverage")
        .collect();
    assert_eq!(coverage_attempts.len(), 2);
    assert_eq!(coverage_attempts[0]["coverage_tier"], 1);
    assert_eq!(coverage_attempts[1]["coverage_tier"], 2);
}

#[test]
fn non_increasing_recovery_depth_fails_before_search() {
    let out = run(&[
        "--target",
        "CC(=O)O",
        "--depth",
        "5",
        "--search-mode",
        "recovery",
        "--recovery-depth",
        "5",
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("must be greater"), "stderr: {stderr}");
}

#[test]
fn explicit_retry_policy_cannot_be_nested_in_recovery_mode() {
    let out = run(&[
        "--target",
        "CC(=O)O",
        "--search-mode",
        "recovery",
        "--beam-diversity-policy",
        "retry-on-beam-exhaustion",
        "--beam-diversity-slots",
        "20",
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("orchestrates beam diversity"),
        "stderr: {stderr}"
    );
}
