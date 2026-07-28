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
