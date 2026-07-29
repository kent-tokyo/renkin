//! Process-level tests: spawn the actual `renkin-forward` binary and check
//! its real stdout/stderr/exit-code behavior, not just library internals.

use std::io::Write;
use std::process::{Command, Stdio};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_renkin-forward")
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(bin())
        .args(args)
        .output()
        .expect("failed to spawn renkin-forward")
}

fn run_stdin(args: &[&str], stdin_data: &str) -> std::process::Output {
    let mut child = Command::new(bin())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn renkin-forward");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(stdin_data.as_bytes())
        .unwrap();
    child.wait_with_output().expect("failed to wait on child")
}

#[test]
fn help_flag_succeeds() {
    let out = run(&["--help"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("renkin-forward"));
    assert!(stdout.contains("predict"));
    assert!(stdout.contains("validate"));
}

#[test]
fn version_flag_succeeds_and_disambiguates() {
    let out = run(&["--version"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("renkin-forward"));
    // Must not be misread as the root renkin package's version.
    assert!(stdout.to_lowercase().contains("not"));
}

#[test]
fn predict_help_succeeds() {
    let out = run(&["predict", "--help"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("--reactants"));
    assert!(stdout.contains("--report"));
}

#[test]
fn validate_help_succeeds() {
    let out = run(&["validate", "--help"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("--route-json"));
}

#[test]
fn predict_legacy_json_is_valid_and_stdout_only() {
    let out = run(&[
        "predict",
        "--reactants",
        "CC(=O)O",
        "CCO",
        "--max-results",
        "5",
    ]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    assert!(parsed.is_array());
}

#[test]
fn predict_report_is_valid_versioned_json() {
    let out = run(&[
        "predict",
        "--reactants",
        "CC(=O)O",
        "CCO",
        "--max-results",
        "5",
        "--report",
    ]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    assert_eq!(parsed["schema_version"], 1);
    assert!(parsed["candidates"].is_array());
    assert!(parsed["stats"].is_object());
    assert!(parsed["warnings"].is_array());
}

#[test]
fn validate_via_route_json_flag_succeeds() {
    let out = run(&[
        "validate",
        "--route-json",
        r#"{"steps":[{"target":"CC(=O)OCC","precursors":["CC(=O)O","CCO"]}]}"#,
    ]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    assert!(parsed.is_array());
    assert_eq!(parsed[0]["step_index"], 0);
}

#[test]
fn validate_via_stdin_matches_flag_form() {
    let route = r#"{"steps":[{"target":"CC(=O)OCC","precursors":["CC(=O)O","CCO"]}]}"#;
    let out_flag = run(&["validate", "--route-json", route]);
    let out_stdin = run_stdin(&["validate"], route);
    assert!(out_flag.status.success());
    assert!(out_stdin.status.success());
    assert_eq!(out_flag.stdout, out_stdin.stdout);
}

#[test]
fn unknown_option_is_hard_error() {
    let out = run(&["predict", "--reactants", "CCO", "--bogus-flag"]);
    assert!(!out.status.success());
    assert!(!out.stderr.is_empty());
}

#[test]
fn missing_option_value_is_hard_error() {
    let out = run(&["predict", "--max-results"]);
    assert!(!out.status.success());
}

#[test]
fn invalid_max_results_integer_is_hard_error() {
    let out = run(&[
        "predict",
        "--reactants",
        "CCO",
        "--max-results",
        "not-a-number",
    ]);
    assert!(!out.status.success());
}

#[test]
fn max_results_zero_is_hard_error() {
    let out = run(&["predict", "--reactants", "CCO", "--max-results", "0"]);
    assert!(!out.status.success());
}

#[test]
fn missing_template_file_is_hard_error() {
    let out = run(&[
        "predict",
        "--reactants",
        "CCO",
        "--templates",
        "/nonexistent/path/does-not-exist.smi",
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("does not exist") || stderr.contains("not accessible"));
}

#[test]
fn stdout_is_json_only_stderr_has_diagnostics() {
    let out = run(&[
        "predict",
        "--reactants",
        "CC(=O)O",
        "CCO",
        "--max-results",
        "5",
        "--report",
    ]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    // The whole of stdout must parse as one JSON document -- nothing else interleaved.
    serde_json::from_str::<serde_json::Value>(&stdout).expect("stdout must be JSON-only");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Loaded"),
        "expected template-load summary on stderr"
    );
}

#[test]
fn two_identical_runs_are_byte_identical() {
    let args = [
        "predict",
        "--reactants",
        "CC(=O)O",
        "CCO",
        "--max-results",
        "5",
        "--report",
    ];
    let out1 = run(&args);
    let out2 = run(&args);
    assert_eq!(out1.stdout, out2.stdout);
}

#[test]
fn clean_eof_on_empty_stdin_validate_is_hard_error_not_hang() {
    let out = run_stdin(&["validate"], "");
    assert!(!out.status.success());
}

#[test]
fn unknown_subcommand_is_hard_error() {
    let out = run(&["frobnicate"]);
    assert!(!out.status.success());
}

#[test]
fn predict_rejects_validate_only_route_json_option() {
    let out = run(&["predict", "--reactants", "CCO", "--route-json", "{}"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--route-json") || stderr.contains("unknown option"));
}

#[test]
fn validate_rejects_predict_only_reactants_option() {
    let out = run(&["validate", "--reactants", "CCO"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--reactants") || stderr.contains("unknown option"));
}

#[test]
fn validate_rejects_predict_only_report_option() {
    let out = run(&[
        "validate",
        "--route-json",
        r#"{"steps":[{"target":"CC(=O)OCC","precursors":["CC(=O)O","CCO"]}]}"#,
        "--report",
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--report") || stderr.contains("unknown option"));
}

#[test]
fn predict_legacy_mode_surfaces_warnings_on_stderr() {
    // A default rule that requires exactly 1 reactant, applied to 2 here,
    // fails inside run_reactants (ReactantCountMismatch) and is reported as
    // a warning -- this must reach stderr even without --report, since
    // predict_products_detailed's Rust return type can't carry warnings
    // back through the legacy array.
    let out = run(&["predict", "--reactants", "c1ccccc1Cl", "CCO"]);
    assert!(out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("warning["),
        "expected at least one warning on stderr in legacy (non --report) mode, got: {stderr}"
    );
}

#[test]
fn validate_step_that_is_not_an_object_is_hard_error() {
    let out = run(&["validate", "--route-json", r#"{"steps":["not-an-object"]}"#]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("step 0"));
}

#[test]
fn validate_step_missing_target_is_hard_error() {
    let out = run(&[
        "validate",
        "--route-json",
        r#"{"steps":[{"precursors":["CCO"]}]}"#,
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("step 0"));
    assert!(stderr.contains("target"));
}

#[test]
fn validate_step_empty_precursors_array_is_hard_error() {
    let out = run(&[
        "validate",
        "--route-json",
        r#"{"steps":[{"target":"CCO","precursors":[]}]}"#,
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("step 0"));
    assert!(stderr.contains("precursors"));
}

#[test]
fn predict_calls_prediction_engine_exactly_once_regardless_of_report() {
    // `aryl_chloride_to_bromide` requires exactly 1 reactant; applied to 2,
    // it fails inside run_reactants for every reactant ordering the fix
    // tries, and the warning is deduped to one entry per rule -- but
    // several other default rules also require exactly 1 reactant and fail
    // the same way, so the proxy must count occurrences of this ONE rule's
    // name, not the shared `template_application_failed` code (which
    // legitimately appears once per distinct failing rule). If `predict`
    // (in legacy mode) ran the engine twice, per the old bug where it
    // called `predict_products_detailed` once for `--report`'s report and
    // `predict_products` again for the legacy array, this rule's warning
    // would be primed to appear twice as well (it doesn't).
    let legacy = run(&["predict", "--reactants", "c1ccccc1Cl", "CCO"]);
    assert!(legacy.status.success());
    let legacy_stderr = String::from_utf8_lossy(&legacy.stderr);
    let legacy_count = legacy_stderr
        .matches("rule:aryl_chloride_to_bromide")
        .count();
    assert_eq!(
        legacy_count, 1,
        "expected exactly 1 aryl_chloride_to_bromide warning in legacy mode, got {legacy_count}: {legacy_stderr}"
    );

    let report = run(&["predict", "--reactants", "c1ccccc1Cl", "CCO", "--report"]);
    assert!(report.status.success());
    let report_stderr = String::from_utf8_lossy(&report.stderr);
    let report_count = report_stderr
        .matches("rule:aryl_chloride_to_bromide")
        .count();
    assert_eq!(
        report_count, 1,
        "expected exactly 1 aryl_chloride_to_bromide warning with --report, got {report_count}: {report_stderr}"
    );
}

#[test]
fn validate_calls_prediction_engine_exactly_once_per_step() {
    // Same proxy as predict_calls_prediction_engine_exactly_once_regardless_of_report,
    // but through `validate`: if `verified` and `top_predictions` were each
    // computed by a separate prediction pass (the original bug), this
    // rule's warning would appear twice for the one step.
    let out = run(&[
        "validate",
        "--route-json",
        r#"{"steps":[{"target":"CCO","precursors":["c1ccccc1Cl","CCO"]}]}"#,
    ]);
    assert!(out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    let count = stderr.matches("rule:aryl_chloride_to_bromide").count();
    assert_eq!(
        count, 1,
        "expected exactly 1 aryl_chloride_to_bromide warning for 1 step, got {count}: {stderr}"
    );
}

#[test]
fn validate_step_non_string_precursor_is_hard_error_not_silently_dropped() {
    let out = run(&[
        "validate",
        "--route-json",
        r#"{"steps":[{"target":"CC(=O)OCC","precursors":["CC(=O)O", 42]}]}"#,
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("step 0"));
    assert!(stderr.contains("precursors"));
}

// -- enumerate ------------------------------------------------------------

fn write_temp_partners(label: &str, content: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "renkin_forward_cli_test_{label}_{}_{:?}.smi",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::write(&path, content).expect("failed to write temp partners file");
    path
}

#[test]
fn enumerate_help_succeeds() {
    let out = run(&["enumerate", "--help"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("--reactant"));
    assert!(stdout.contains("--partners"));
}

#[test]
fn enumerate_requires_reactant_flag() {
    let out = run(&["enumerate"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--reactant"));
}

#[test]
fn enumerate_unary_only_discovery_without_partners_succeeds() {
    let out = run(&["enumerate", "--reactant", "Brc1ccccc1"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    assert_eq!(parsed["schema_version"], 1);
    assert!(parsed["known_reactant"].is_object());
    assert!(parsed["candidates"].is_array());
    assert!(
        parsed["stats"]["templates_binary_skipped_no_partners"]
            .as_u64()
            .unwrap()
            > 0
    );
}

#[test]
fn enumerate_missing_partners_file_is_hard_error() {
    let out = run(&[
        "enumerate",
        "--reactant",
        "CCCCCl",
        "--partners",
        "/nonexistent/path/does-not-exist.smi",
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("does not exist") || stderr.contains("not accessible"));
}

#[test]
fn enumerate_with_partners_file_end_to_end() {
    let path = write_temp_partners("e2e", "CCBr\nCCCBr\n");
    let out = run(&[
        "enumerate",
        "--reactant",
        "CCCCCl",
        "--partners",
        path.to_str().unwrap(),
    ]);
    std::fs::remove_file(&path).ok();

    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout must be valid JSON");
    assert_eq!(parsed["schema_version"], 1);
    assert_eq!(parsed["stats"]["partners_scanned"], 2);
    assert!(parsed["stats"]["partners_file_sha256"].is_string());
    let candidates = parsed["candidates"].as_array().unwrap();
    assert!(!candidates.is_empty());
    assert!(candidates[0]["sources"][0]["partner"].is_object());
}

#[test]
fn enumerate_two_identical_runs_are_byte_identical() {
    let path = write_temp_partners("identical", "CCBr\nCCCBr\n");
    let args = [
        "enumerate",
        "--reactant",
        "CCCCCl",
        "--partners",
        path.to_str().unwrap(),
    ];
    let out1 = run(&args);
    let out2 = run(&args);
    std::fs::remove_file(&path).ok();
    assert!(out1.status.success());
    assert_eq!(out1.stdout, out2.stdout);
}

#[test]
fn enumerate_rejects_predict_only_reactants_option() {
    let out = run(&["enumerate", "--reactant", "CCO", "--reactants", "CCO"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--reactants") || stderr.contains("unknown option"));
}

#[test]
fn predict_rejects_enumerate_only_reactant_option() {
    let out = run(&["predict", "--reactant", "CCO"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--reactant") || stderr.contains("unknown option"));
}

#[test]
fn enumerate_max_combinations_zero_is_hard_error() {
    let out = run(&[
        "enumerate",
        "--reactant",
        "CCCCCl",
        "--max-combinations",
        "0",
    ]);
    assert!(!out.status.success());
}
