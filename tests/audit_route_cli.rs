//! Process-level tests for `renkin audit-route` (RENKIN Bridge PR5): spawn
//! the real `renkin` binary twice per test -- once to generate a real route
//! JSON file via the standard search path, once to audit it -- so this
//! exercises the actual round-trip through the CLI's real `--format json`
//! output shape, not a hand-authored fixture that could silently drift from
//! what the CLI really emits. Mirrors `tests/coverage_mode_cli.rs`'s
//! spawn-the-real-binary convention.

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_renkin")
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(bin())
        .args(args)
        .output()
        .expect("failed to spawn renkin")
}

/// A path under the system temp dir unique to this call -- `cargo test`
/// runs tests in parallel *threads within one process*, so `std::process::id()`
/// alone collides across concurrently-running tests; combine it with a
/// per-call atomic counter instead.
fn unique_temp_path(label: &str) -> std::path::PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "renkin_audit_route_{label}_{}_{n}.json",
        std::process::id()
    ))
}

/// Generates a real route-search JSON output file for `target` and returns
/// its path. `CCOC(=O)c1ccccc1` at `--depth 1` reliably solves via
/// `co_aliphatic_cleavage` (a SMIRKS-based hand-crafted rule, so its steps
/// are forward-replayable) against the repo's own `data/building_blocks.smi`
/// default stock -- same target/depth already established as fast and
/// reliable elsewhere in this Bridge program's own tests.
fn generate_route_fixture() -> std::path::PathBuf {
    let out = run(&["--target", "CCOC(=O)c1ccccc1", "--depth", "1"]);
    assert!(
        out.status.success(),
        "route generation must succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let path = unique_temp_path("fixture");
    std::fs::write(&path, &out.stdout).expect("failed to write route fixture");
    path
}

#[test]
fn passes_when_stock_and_forward_replay_succeed() {
    let route_path = generate_route_fixture();
    let out = run(&[
        "audit-route",
        route_path.to_str().unwrap(),
        "--stock",
        "data/building_blocks.smi",
        "--output",
        "json",
    ]);
    assert!(
        out.status.success(),
        "a Pass/Fail/Partial verdict must never be a nonzero exit: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["source_format"], "renkin");
    assert!(report["summary"]["routes_total"].as_u64().unwrap() > 0);
    assert!(
        report["routes"][0]["stock_validation"]["status"] == "pass",
        "{report}"
    );
    std::fs::remove_file(&route_path).ok();
}

/// v0.27.0 "Reproducible Route Audit": `audit_manifest` must record real,
/// verifiable provenance -- not just be present. Recomputes the expected
/// `input_sha256` independently (same recipe as `input_content_sha256` in
/// `main.rs`, over the exact bytes on disk that were audited) rather than
/// only checking the field exists, so a manifest hashing the wrong thing
/// (or a hardcoded placeholder) would fail this test.
#[test]
fn audit_manifest_records_verifiable_reproducibility_metadata() {
    let route_path = generate_route_fixture();
    let out = run(&[
        "audit-route",
        route_path.to_str().unwrap(),
        "--stock",
        "data/building_blocks.smi",
        "--output",
        "json",
    ]);
    assert!(out.status.success());
    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
    let manifest = &report["audit_manifest"];

    assert_eq!(manifest["report_schema_version"], report["schema_version"]);
    assert_eq!(manifest["source_format"], report["source_format"]);
    assert_eq!(manifest["source_version"], serde_json::Value::Null);
    assert_eq!(manifest["policy"], "standard");
    assert!(
        !manifest["renkin_version"].as_str().unwrap_or("").is_empty(),
        "{report}"
    );

    let raw_bytes = std::fs::read(&route_path).unwrap();
    let expected_input_sha256 = format!("sha256:{}", sha256_hex(&raw_bytes));
    assert_eq!(manifest["input_sha256"], expected_input_sha256, "{report}");

    let stock_sha256 = manifest["stock_sha256"]
        .as_str()
        .expect("stock_sha256 must be present when --stock was given");
    assert!(stock_sha256.starts_with("sha256:"), "{report}");

    std::fs::remove_file(&route_path).ok();
}

#[test]
fn stock_sha256_is_null_without_stock_flag() {
    let route_path = generate_route_fixture();
    let out = run(&[
        "audit-route",
        route_path.to_str().unwrap(),
        "--output",
        "json",
    ]);
    assert!(out.status.success());
    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
    assert_eq!(
        report["audit_manifest"]["stock_sha256"],
        serde_json::Value::Null,
        "{report}"
    );
    std::fs::remove_file(&route_path).ok();
}

/// v0.27.0 P0 item 4: same input audited twice must produce identical
/// output. Audits the SAME already-generated route file twice (rather than
/// regenerating the route each time), so this isolates `audit-route`'s own
/// determinism from the separate question of search determinism.
#[test]
fn auditing_the_same_input_twice_is_byte_identical() {
    let route_path = generate_route_fixture();
    let args = [
        "audit-route",
        route_path.to_str().unwrap(),
        "--stock",
        "data/building_blocks.smi",
        "--output",
        "json",
    ];
    let out1 = run(&args);
    let out2 = run(&args);
    assert!(out1.status.success() && out2.status.success());
    assert_eq!(
        out1.stdout, out2.stdout,
        "auditing identical input twice must be byte-identical"
    );
    std::fs::remove_file(&route_path).ok();
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn partial_without_stock_reports_stock_not_provided_not_a_silent_pass() {
    let route_path = generate_route_fixture();
    let out = run(&[
        "audit-route",
        route_path.to_str().unwrap(),
        "--output",
        "json",
    ]);
    assert!(out.status.success());
    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
    assert_eq!(
        report["routes"][0]["stock_validation"]["status"],
        "not_evaluable"
    );
    assert_eq!(
        report["routes"][0]["stock_validation"]["reason"],
        "stock_not_provided"
    );
    assert_ne!(report["routes"][0]["status"], "pass");
    std::fs::remove_file(&route_path).ok();
}

#[test]
fn human_output_is_readable_and_stdout_only_carries_the_report() {
    let route_path = generate_route_fixture();
    let out = run(&[
        "audit-route",
        route_path.to_str().unwrap(),
        "--stock",
        "data/building_blocks.smi",
    ]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("routes audited"), "{stdout}");
    assert!(stdout.contains("route 1/"), "{stdout}");
    // stdout must never contain raw JSON braces in human mode -- confirms
    // the human/json output paths are genuinely distinct, not one always
    // printing JSON regardless of --output.
    assert!(!stdout.trim_start().starts_with('{'), "{stdout}");
    std::fs::remove_file(&route_path).ok();
}

#[test]
fn empty_routes_input_is_handled_gracefully() {
    // A real CLI "no routes found" JSON shape, not hand-authored: a
    // deliberately absurd target with a tiny depth guarantees zero routes.
    let out = run(&[
        "--target",
        "c1ccccc1CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC",
        "--depth",
        "1",
    ]);
    assert!(out.status.success());
    let path = unique_temp_path("empty");
    std::fs::write(&path, &out.stdout).unwrap();

    let out = run(&["audit-route", path.to_str().unwrap(), "--output", "json"]);
    assert!(out.status.success());
    let report: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON");
    assert_eq!(report["summary"]["routes_total"], 0);
    assert_eq!(report["routes"].as_array().unwrap().len(), 0);
    std::fs::remove_file(&path).ok();
}

#[test]
fn rejects_unsupported_format() {
    let route_path = generate_route_fixture();
    let out = run(&[
        "audit-route",
        route_path.to_str().unwrap(),
        "--format",
        "aizynthfinder",
    ]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("--format"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::remove_file(&route_path).ok();
}

#[test]
fn rejects_unreadable_path() {
    let out = run(&["audit-route", "/nonexistent/path/route.json"]);
    assert!(!out.status.success());
}

#[test]
fn rejects_malformed_json_with_context() {
    // RENKIN Bridge PR6: format auto-detection parses a generic
    // `serde_json::Value` first, so syntactically invalid JSON now fails at
    // that stage with "not valid JSON" -- before any format-specific
    // (RENKIN/AiZynthFinder) deserialize ever runs.
    let path = unique_temp_path("malformed");
    std::fs::write(&path, "not json").unwrap();
    let out = run(&["audit-route", path.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("not valid JSON"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::remove_file(&path).ok();
}

#[test]
fn rejects_valid_json_that_fails_the_renkin_schema_with_context() {
    // Valid JSON with the top-level shape `detect_audit_route_format` reads
    // as RENKIN (`target` + `routes` keys present), but `routes` isn't the
    // expected array-of-route-entries shape -- must fail at the
    // RENKIN-specific deserialize step with schema-specific context, not a
    // generic JSON-syntax error.
    let path = unique_temp_path("bad_renkin_schema");
    std::fs::write(&path, r#"{"target": "CCO", "routes": "not an array"}"#).unwrap();
    let out = run(&["audit-route", path.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("not a recognized RENKIN route JSON"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::remove_file(&path).ok();
}
